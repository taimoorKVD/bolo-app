import { describe, expect, it } from "vitest";
import { createApp, createMockEnv } from "../src/app";
import { AdminService } from "../src/admin";
import { MockBackend } from "../mock/store";
import { MockAiGateway } from "../src/ai";
import { hashPassword } from "../src/passwords";
import { snapshot, restore } from "../../node/persist";
import { totpCode } from "../src/totp-util";

function prodAdmin() {
  const env = createMockEnv();
  const backend = new MockBackend(env);
  const admin = new AdminService(backend, "supabase");
  admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, totpEnrolled: true, enabled: true });
  admin.setTotpSecret("info@kingdomvision.com", "JBSWY3DPEHPK3PXP", true);
  return { env, backend, admin, handle: createApp(backend, new MockAiGateway(), env, admin) };
}

async function call(handle: (req: Request) => Promise<Response>, path: string, init: { method?: string; body?: unknown } = {}) {
  const res = await handle(new Request("http://local" + path, {
    method: init.method ?? "GET",
    headers: { "content-type": "application/json" },
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
  }));
  const text = await res.text();
  return { status: res.status, text, json: JSON.parse(text || "{}") as Record<string, any> };
}

describe("one-time owner setup", () => {
  it("shows setup page only when owner has no password and token is configured", async () => {
    const { admin, handle } = prodAdmin();
    expect((await call(handle, "/admin/setup")).status).toBe(404);
    admin.configureOwnerSetup("a".repeat(24));
    const page = await handle(new Request("http://local/admin/setup"));
    expect(page.status).toBe(200);
    const html = await page.text();
    expect(html).toContain("Create the owner password");
    expect(html).toContain("Bolo");
    expect(html).not.toContain("a".repeat(24));
    expect(html).not.toContain("scrypt$");
    expect(html).not.toContain("Local mock");
  });

  it("accepts setup token + password, then disables setup and allows login", async () => {
    const { admin, handle } = prodAdmin();
    const token = "setup-token-high-entropy-0001";
    admin.configureOwnerSetup(token);

    const bad = await call(handle, "/admin/api/setup", {
      method: "POST",
      body: { token: "wrong-token-high-entropy-xxx", password: "owner-password-14", confirm: "owner-password-14" },
    });
    expect(bad.status).toBe(401);
    expect(bad.json.error).toBe("Setup failed.");
    expect(bad.text).not.toContain(token);
    expect(bad.text).not.toContain("owner-password-14");

    const mismatch = await call(handle, "/admin/api/setup", {
      method: "POST",
      body: { token, password: "owner-password-14", confirm: "owner-password-15" },
    });
    expect(mismatch.status).toBe(400);

    const ok = await call(handle, "/admin/api/setup", {
      method: "POST",
      body: { token, password: "owner-password-14", confirm: "owner-password-14" },
    });
    expect(ok.status).toBe(200);
    expect(ok.json.done).toBe(true);
    expect(ok.text).not.toContain("scrypt$");
    expect(ok.text).not.toContain(token);
    expect(admin.ownerHasPassword()).toBe(true);
    expect(admin.setupAvailable()).toBe(false);
    expect((await call(handle, "/admin/setup")).status).toBe(404);
    expect((await call(handle, "/admin/api/setup", {
      method: "POST",
      body: { token, password: "owner-password-14", confirm: "owner-password-14" },
    })).status).toBe(404);

    // Re-configure env token as if process still has it — still unavailable because password exists
    admin.configureOwnerSetup(token);
    expect(admin.setupAvailable()).toBe(false);

    const code = totpCode("JBSWY3DPEHPK3PXP", Math.floor(Date.now() / 30000));
    const login = await admin.login("info@kingdomvision.com", "owner-password-14", code);
    expect(login.ok).toBe(true);
  });

  it("persisted hash wins over env hash on restore", async () => {
    const { backend, admin } = prodAdmin();
    const persisted = await hashPassword("persisted-password1");
    const envHash = await hashPassword("env-password-xxxx");
    admin.setOwnerPasswordHash("info@kingdomvision.com", persisted);
    const snap = snapshot(backend, admin) as any;

    const env = createMockEnv();
    const backend2 = new MockBackend(env);
    const admin2 = new AdminService(backend2, "supabase");
    admin2.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, totpEnrolled: true });
    admin2.setOwnerPasswordHash("info@kingdomvision.com", envHash); // pre-restore env apply
    restore(backend2, admin2, snap);
    admin2.setOwnerPasswordHash("info@kingdomvision.com", envHash, { onlyIfMissing: true });
    const staff = admin2.exportStaff().find((a) => a.email === "info@kingdomvision.com");
    expect(staff?.passwordHash).toBe(persisted);
    expect(staff?.passwordHash).not.toBe(envHash);
  });

  it("rate-limits setup attempts", async () => {
    const { admin } = prodAdmin();
    let now = Date.now();
    admin.setClock(() => now);
    admin.configureOwnerSetup("setup-token-high-entropy-0002");
    for (let i = 0; i < 5; i++) {
      const r = await admin.completeOwnerSetup("wrong-token-high-entropy-yyy", "owner-password-14", "owner-password-14");
      expect(r.ok).toBe(false);
    }
    const locked = await admin.completeOwnerSetup("setup-token-high-entropy-0002", "owner-password-14", "owner-password-14");
    expect(locked.ok).toBe(false);
    if (!locked.ok) expect(locked.status).toBe(429);
  });

  it("invalidates owner sessions on setup", async () => {
    const { admin } = prodAdmin();
    // Temporarily set a password to create a session, then clear hash to allow setup — simulate edge case:
    // Instead: create session via mock path is hard in supabase mode without password.
    // Set password, login, then manually clear hash and run setup which should drop sessions.
    const hash = await hashPassword("temp-password-14x");
    admin.setOwnerPasswordHash("info@kingdomvision.com", hash);
    const code = totpCode("JBSWY3DPEHPK3PXP", Math.floor(Date.now() / 30000));
    const login = await admin.login("info@kingdomvision.com", "temp-password-14x", code);
    expect(login.ok).toBe(true);
    if (!login.ok) return;
    // Force setup path: clear hash, configure token, complete setup
    const acct = admin.exportStaff().find((a) => a.email === "info@kingdomvision.com")!;
    (admin as any).admins.get("info@kingdomvision.com").passwordHash = undefined;
    admin.configureOwnerSetup("setup-token-high-entropy-0003");
    expect(admin.setupAvailable()).toBe(true);
    const done = await admin.completeOwnerSetup("setup-token-high-entropy-0003", "fresh-password-14", "fresh-password-14");
    expect(done.ok).toBe(true);
    const me = await admin.authorize(login.value.token, "view");
    expect(me.ok).toBe(false);
  });
});
