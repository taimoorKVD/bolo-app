import { describe, expect, it } from "vitest";
import { createLocalStack } from "../src/app";
import { AdminService, MOCK_TOTP_CODE, can } from "../src/admin";
import { MockBackend } from "../mock/store";
import { createMockEnv } from "../src/app";
import { hashPassword } from "../src/passwords";
import { snapshot, restore } from "../../node/persist";
import { totpCode } from "../src/totp-util";

async function mockStack() {
  const s = createLocalStack(undefined, { seed: true });
  await s.ready;
  const call = async (path: string, init: { method?: string; token?: string; body?: unknown } = {}) => {
    const res = await s.handle(new Request("http://local" + path, {
      method: init.method ?? "GET",
      headers: { "content-type": "application/json", ...(init.token ? { authorization: `Bearer ${init.token}` } : {}) },
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
    }));
    const text = await res.text();
    return { status: res.status, text, json: JSON.parse(text || "{}") as Record<string, any> };
  };
  const login = async (email: string, password = "") =>
    (await call("/admin/api/login", { method: "POST", body: { email, password, code: MOCK_TOTP_CODE } })).json.token as string;
  return { ...s, call, login };
}

describe("production password auth", () => {
  it("requires email + password + TOTP and returns a generic failure", async () => {
    const env = createMockEnv();
    const backend = new MockBackend(env);
    const admin = new AdminService(backend, "supabase");
    const password = "correct-horse-battery-staple";
    const hash = await hashPassword(password);
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, totpEnrolled: true });
    admin.setOwnerPasswordHash("info@kingdomvision.com", hash);
    admin.setTotpSecret("info@kingdomvision.com", "JBSWY3DPEHPK3PXP", true);
    const secret = admin.getTotpSecret("info@kingdomvision.com")!;
    const code = totpCode(secret, Math.floor(Date.now() / 30000));

    const bad = await admin.login("info@kingdomvision.com", "wrong-password-xx", code);
    expect(bad.ok).toBe(false);
    if (!bad.ok) {
      expect(bad.message).toBe("Sign-in failed.");
      expect(bad.code).toBe("admin_login_failed");
    }

    const ok = await admin.login("info@kingdomvision.com", password, code);
    expect(ok.ok).toBe(true);
    if (ok.ok) expect(ok.value.email).toBe("info@kingdomvision.com");
  });

  it("rate-limits repeated failures per account", async () => {
    const env = createMockEnv();
    const backend = new MockBackend(env);
    let now = Date.now();
    const admin = new AdminService(backend, "supabase", () => now);
    const password = "correct-horse-battery-staple";
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, totpEnrolled: true });
    admin.setOwnerPasswordHash("info@kingdomvision.com", await hashPassword(password));
    admin.setTotpSecret("info@kingdomvision.com", "JBSWY3DPEHPK3PXP", true);
    for (let i = 0; i < 5; i++) {
      const r = await admin.login("info@kingdomvision.com", "nope-nope-nope-1", "000000");
      expect(r.ok).toBe(false);
    }
    const locked = await admin.login("info@kingdomvision.com", password, totpCode("JBSWY3DPEHPK3PXP", Math.floor(now / 30000)));
    expect(locked.ok).toBe(false);
    if (!locked.ok) expect(locked.status).toBe(429);
  });
});

describe("team permissions and persistence", () => {
  it("owner can create staff; support cannot; last owner protected", async () => {
    const s = await mockStack();
    const owner = await s.login("owner@example.com");
    const support = await s.login("support@example.com");

    expect((await s.call("/admin/api/team", { token: support })).status).toBe(403);

    const created = await s.call("/admin/api/team", {
      method: "POST",
      token: owner,
      body: {
        email: "new.support@example.com",
        label: "New Support",
        role: "support",
        password: "fourteen-chars!!",
        reason: "hire support agent",
      },
    });
    expect(created.status).toBe(200);
    expect(created.json.staff.email).toBe("new.support@example.com");
    expect(created.json.enrollment.secret).toBeTruthy();
    expect(created.json.enrollment.otpauthUrl).toContain("otpauth://totp/");
    expect(created.text).not.toContain("passwordHash");
    expect(created.text).not.toMatch(/scrypt\$/);

    // Cannot demote last remaining owner when only one owner exists among enabled... there is only owner@example.com as owner
    const demote = await s.call("/admin/api/team/" + encodeURIComponent("owner@example.com") + "/role", {
      method: "POST",
      token: owner,
      body: { role: "viewer", reason: "should fail last owner" },
    });
    expect(demote.status).toBe(400);
    expect(demote.json.code).toBe("last_owner");

    const selfOff = await s.call("/admin/api/team/" + encodeURIComponent("owner@example.com") + "/enabled", {
      method: "POST",
      token: owner,
      body: { enabled: false, reason: "should fail self disable" },
    });
    expect(selfOff.status).toBe(400);
    expect(selfOff.json.code).toBe("self_disable");
  });

  it("persists staff password hashes and totp secrets through snapshot/restore", async () => {
    const env = createMockEnv();
    const backend = new MockBackend(env);
    const admin = new AdminService(backend, "supabase");
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, totpEnrolled: true });
    const hash = await hashPassword("owner-password-14");
    admin.setOwnerPasswordHash("info@kingdomvision.com", hash);
    admin.setTotpSecret("info@kingdomvision.com", "JBSWY3DPEHPK3PXP", true);

    // simulate createStaff
    let clock = Date.now();
    admin.setClock(() => clock);
    const tokenRes = await admin.login("info@kingdomvision.com", "owner-password-14", totpCode("JBSWY3DPEHPK3PXP", Math.floor(clock / 30000)));
    expect(tokenRes.ok).toBe(true);
    if (!tokenRes.ok) return;
    const authz = await admin.authorize(tokenRes.value.token, "manage_team");
    expect(authz.ok).toBe(true);
    if (!authz.ok) return;
    const sess = authz.value;
    // refresh recent auth already set
    const created = await admin.createStaff(sess, {
      email: "staff@example.com",
      label: "Staff",
      role: "viewer",
      password: "staff-password-14",
    }, "add viewer");
    expect(created.ok).toBe(true);

    const snap = snapshot(backend, admin) as any;
    expect(snap.staff.some((x: any) => x.email === "staff@example.com" && x.passwordHash?.startsWith("scrypt$"))).toBe(true);
    expect(snap.staff.find((x: any) => x.email === "staff@example.com").totpSecret).toBeTruthy();

    const backend2 = new MockBackend(env);
    const admin2 = new AdminService(backend2, "supabase");
    admin2.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, totpEnrolled: true });
    admin2.setOwnerPasswordHash("info@kingdomvision.com", hash);
    admin2.setTotpSecret("info@kingdomvision.com", "JBSWY3DPEHPK3PXP", true);
    restore(backend2, admin2, snap);

    const staff = admin2.exportStaff().find((a) => a.email === "staff@example.com");
    expect(staff?.passwordHash?.startsWith("scrypt$")).toBe(true);
    expect(staff?.totpSecret).toBeTruthy();
    expect(staff?.role).toBe("viewer");
  });

  it("production login HTML has no mock wording", async () => {
    const env = createMockEnv();
    const backend = new MockBackend(env);
    const { createApp } = await import("../src/app");
    const { MockAiGateway } = await import("../src/ai");
    const handle = createApp(backend, new MockAiGateway(), env, new AdminService(backend, "supabase"));
    const res = await handle(new Request("http://local/admin"));
    const html = await res.text();
    expect(html).toContain("Office sign-in");
    expect(html).toContain('id="shell" hidden');
    expect(html).toContain('"mock":false');
    expect(html).not.toContain("password, and authenticator");
    expect(html).not.toContain("Local mock");
    expect(html).not.toContain("local mock");
    expect(html).not.toContain("123456");
    expect(html).not.toContain("owner@example.com");
    expect(html).not.toContain("example.com ·");
  });
});

describe("email sign-in codes", () => {
  function harness() {
    const env = createMockEnv();
    const backend = new MockBackend(env);
    let now = Date.now();
    const admin = new AdminService(backend, "supabase", () => now);
    const sent: { email: string; code: string }[] = [];
    admin.officeMailer = async (email, code) => { sent.push({ email, code }); return true; };
    return { env, backend, admin, sent, setNow(n: number) { now = n; } };
  }

  it("turns email codes on for the existing owner and keeps an explicit off", async () => {
    const { admin } = harness();
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, totpEnrolled: true });
    admin.applyEmail2faMigration();
    expect(admin.exportStaff().find((a) => a.email === "info@kingdomvision.com")?.email2fa).toBe(true);
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, email2fa: false });
    admin.applyEmail2faMigration();
    expect(admin.exportStaff().find((a) => a.email === "info@kingdomvision.com")?.email2fa).toBe(false);
  });

  it("signs in with password only when email codes are off, and with a one-time email code when on", async () => {
    const { admin, backend, sent } = harness();
    const password = "correct-horse-battery-staple";
    const hash = await hashPassword(password);
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, totpEnrolled: true, email2fa: true });
    admin.setOwnerPasswordHash("info@kingdomvision.com", hash);
    admin.addAdmin({ email: "helper@kingdomvision.com", role: "support", label: "Helper", passwordHash: hash, email2fa: false, totpEnrolled: false });

    const open = await admin.login("helper@kingdomvision.com", password, "");
    expect(open.ok).toBe(true);
    if (open.ok) {
      expect(open.value.token).toMatch(/^adm_/);
      expect(open.value.needsEmailCode).toBeUndefined();
    }

    const step = await admin.login("info@kingdomvision.com", password, "");
    expect(step.ok).toBe(true);
    if (!step.ok) return;
    expect(step.value.token).toBeUndefined();
    expect(step.value.needsEmailCode).toBe(true);
    expect(step.value.challenge).toBeTruthy();
    expect(sent).toHaveLength(1);
    expect(sent[0].email).toBe("info@kingdomvision.com");
    expect(sent[0].code).toMatch(/^\d{6}$/);
    expect(admin.storesPlaintextCode(sent[0].code)).toBe(false);
    expect(JSON.stringify(snapshot(backend, admin))).not.toContain(sent[0].code);

    const wrong = await admin.completeEmailLogin(step.value.challenge!, "000000");
    expect(wrong.ok).toBe(false);
    if (!wrong.ok) {
      expect(wrong.message).toBe("Sign-in failed.");
      expect(wrong.message).not.toContain(password);
      expect(JSON.stringify(wrong)).not.toContain(sent[0].code);
    }

    const ok = await admin.completeEmailLogin(step.value.challenge!, sent[0].code);
    expect(ok.ok).toBe(true);
    if (ok.ok) expect(ok.value.token).toMatch(/^adm_/);
    const reuse = await admin.completeEmailLogin(step.value.challenge!, sent[0].code);
    expect(reuse.ok).toBe(false);
  });

  it("expires codes, stops after 5 attempts, and rate-limits resend", async () => {
    const { admin, sent, setNow } = harness();
    let now = Date.now();
    setNow(now);
    const password = "correct-horse-battery-staple";
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, email2fa: true, totpEnrolled: false });
    admin.setOwnerPasswordHash("info@kingdomvision.com", await hashPassword(password));
    const step = await admin.login("info@kingdomvision.com", password, "");
    expect(step.ok).toBe(true);
    if (!step.ok || !step.value.challenge) return;
    const challenge = step.value.challenge;
    const early = await admin.resendEmailLogin(challenge);
    expect(early.ok).toBe(false);
    if (!early.ok) expect(early.status).toBe(429);

    now += 61_000;
    setNow(now);
    const again = await admin.resendEmailLogin(challenge);
    expect(again.ok).toBe(true);
    expect(sent).toHaveLength(2);
    const stale = await admin.completeEmailLogin(challenge, sent[0].code);
    expect(stale.ok).toBe(false);

    now += 11 * 60 * 1000;
    setNow(now);
    const expired = await admin.completeEmailLogin(challenge, sent[1].code);
    expect(expired.ok).toBe(false);

    now += 1000;
    setNow(now);
    const fresh = await admin.login("info@kingdomvision.com", password, "");
    expect(fresh.ok && fresh.value.needsEmailCode).toBe(true);
    if (!fresh.ok || !fresh.value.challenge) return;
    for (let i = 0; i < 5; i++) {
      const bad = await admin.completeEmailLogin(fresh.value.challenge, "111111");
      expect(bad.ok).toBe(false);
    }
    const locked = await admin.completeEmailLogin(fresh.value.challenge, sent[sent.length - 1].code);
    expect(locked.ok).toBe(false);
    if (!locked.ok) expect([401, 429]).toContain(locked.status);
  });

  it("lets the owner manage email codes and lets a user turn their own on", async () => {
    const { admin, sent } = harness();
    const password = "correct-horse-battery-staple";
    const hash = await hashPassword(password);
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, email2fa: true, totpEnrolled: true });
    admin.setOwnerPasswordHash("info@kingdomvision.com", hash);
    admin.setTotpSecret("info@kingdomvision.com", "JBSWY3DPEHPK3PXP", true);
    const ownerLogin = await admin.login("info@kingdomvision.com", password, totpCode("JBSWY3DPEHPK3PXP", Math.floor(Date.now() / 30000)));
    expect(ownerLogin.ok).toBe(true);
    if (!ownerLogin.ok || !ownerLogin.value.token) return;
    const authz = await admin.authorize(ownerLogin.value.token, "manage_team");
    expect(authz.ok).toBe(true);
    if (!authz.ok) return;
    const created = await admin.createStaff(authz.value, {
      email: "staff@kingdomvision.com", label: "Staff", role: "viewer", password,
    }, "add viewer");
    expect(created.ok).toBe(true);
    if (!created.ok) return;
    expect(created.value.staff.email2fa).toBe(false);

    const off = await admin.login("staff@kingdomvision.com", password, "");
    expect(off.ok && Boolean(off.value.token)).toBe(true);
    if (!off.ok || !off.value.token) return;
    const asked = await admin.requestOwnEmail2fa(off.value.token);
    expect(asked.ok).toBe(true);
    const mine = sent[sent.length - 1];
    expect(mine.email).toBe("staff@kingdomvision.com");
    const confirmed = await admin.confirmOwnEmail2fa(off.value.token, mine.code);
    expect(confirmed.ok).toBe(true);
    const next = await admin.login("staff@kingdomvision.com", password, "");
    expect(next.ok && next.value.needsEmailCode).toBe(true);

    const toggled = admin.setStaffEmail2fa(authz.value, "staff@kingdomvision.com", false, "owner turns codes off");
    expect(toggled.ok && toggled.value.email2fa).toBe(false);
    const plain = await admin.login("staff@kingdomvision.com", password, "");
    expect(plain.ok && Boolean(plain.value.token)).toBe(true);
  });
});

describe("manage_team permission matrix", () => {
  it("only owner has manage_team", () => {
    expect(can("owner", "manage_team")).toBe(true);
    expect(can("support", "manage_team")).toBe(false);
    expect(can("viewer", "manage_team")).toBe(false);
  });
});
