import { describe, expect, it } from "vitest";
import { createLocalStack } from "../src/app";
import { MOCK_TOTP_CODE, RECENT_AUTH_MS, can } from "../src/admin";

async function stack() {
  const s = createLocalStack(undefined, { seed: true });
  await s.ready;
  const call = async (path: string, init: { method?: string; token?: string; body?: unknown } = {}) => {
    const res = await s.handle(new Request("http://local" + path, {
      method: init.method ?? "GET",
      headers: { "content-type": "application/json", ...(init.token ? { authorization: `Bearer ${init.token}` } : {}) },
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
    }));
    const text = await res.text();
    return { status: res.status, text, json: (text.startsWith("{") ? JSON.parse(text) : {}) as Record<string, any> };
  };
  const login = async (email: string, password = "") => (await call("/admin/api/login", { method: "POST", body: { email, password, code: MOCK_TOTP_CODE } })).json.token as string;
  return { ...s, call, login };
}

describe("admin role permissions", () => {
  it("matrix: viewer read-only, support access ops, owner everything", () => {
    expect(can("viewer", "view")).toBe(true);
    for (const a of ["notice", "revoke", "reissue", "move_pool", "set_plan", "kill_switch", "view_full_email", "manage_team"] as const) expect(can("viewer", a)).toBe(false);
    for (const a of ["revoke", "reissue", "notice"] as const) expect(can("support", a)).toBe(true);
    for (const a of ["move_pool", "set_plan", "suspend", "kill_switch", "manage_team"] as const) expect(can("support", a)).toBe(false);
    for (const a of ["move_pool", "set_plan", "suspend", "kill_switch", "revoke", "manage_team"] as const) expect(can("owner", a)).toBe(true);
  });

  it("rejects unauthenticated and wrong-code sign-in", async () => {
    const s = await stack();
    expect((await s.call("/admin/api/users")).status).toBe(401);
    expect((await s.call("/admin/api/login", { method: "POST", body: { email: "owner@example.com", code: "000000" } })).status).toBe(401);
    expect((await s.call("/admin/api/login", { method: "POST", body: { email: "stranger@example.com", code: MOCK_TOTP_CODE } })).status).toBe(401);
  });

  it("viewer can list (masked emails) but cannot mutate", async () => {
    const s = await stack();
    const t = await s.login("viewer@example.com");
    const users = await s.call("/admin/api/users", { token: t });
    expect(users.status).toBe(200);
    expect(users.json.users.length).toBe(4);
    expect(users.text).not.toContain("ayesha.example@");
    const installId = [...s.backend.installs.keys()][0];
    const r = await s.call(`/admin/api/installs/${installId}/revoke`, { method: "POST", token: t, body: { reason: "testing viewer" } });
    expect(r.status).toBe(403);
    expect((await s.call("/admin/api/kill-switch", { method: "POST", token: t, body: { on: true, reason: "testing viewer" } })).status).toBe(403);
    expect(s.backend.flags.killSwitch).toBe(false);
  });

  it("support can reissue/revoke but not kill switch, plan or pool moves", async () => {
    const s = await stack();
    const t = await s.login("support@example.com");
    const installId = [...s.backend.installs.keys()][0];
    const userId = s.backend.installs.get(installId)!.userId;
    expect((await s.call(`/admin/api/installs/${installId}/reissue`, { method: "POST", token: t, body: { reason: "user lost access" } })).status).toBe(200);
    expect(s.backend.installs.get(installId)!.status).toBe("reissue_pending");
    expect((await s.call("/admin/api/kill-switch", { method: "POST", token: t, body: { on: true, reason: "support try" } })).status).toBe(403);
    expect((await s.call(`/admin/api/users/${userId}/plan`, { method: "POST", token: t, body: { plan: "paid", reason: "support try" } })).status).toBe(403);
    expect((await s.call(`/admin/api/installs/${installId}/move-pool`, { method: "POST", token: t, body: { poolId: "pool_wai_b", reason: "support try" } })).status).toBe(403);
  });

  it("owner needs a reason and recent auth for sensitive actions", async () => {
    let clock = Date.now();
    const s = await stack();
    s.admin.setClock(() => clock);
    const t = await s.login("owner@example.com");
    expect((await s.call("/admin/api/kill-switch", { method: "POST", token: t, body: { on: true, reason: "" } })).json.code).toBe("reason_required");
    clock += RECENT_AUTH_MS + 1000;
    expect((await s.call("/admin/api/kill-switch", { method: "POST", token: t, body: { on: true, reason: "budget spike" } })).json.code).toBe("reauth_required");
    expect((await s.call("/admin/api/reauth", { method: "POST", token: t, body: { code: MOCK_TOTP_CODE } })).status).toBe(200);
    expect((await s.call("/admin/api/kill-switch", { method: "POST", token: t, body: { on: true, reason: "budget spike" } })).status).toBe(200);
    expect(s.backend.flags.killSwitch).toBe(true);
    const status = await s.call("/v1/status");
    expect(status.json.killSwitch).toBe(true);
  });
});

describe("audit log", () => {
  it("records successes and denials, append-only and immutable", async () => {
    const s = await stack();
    const v = await s.login("viewer@example.com");
    const o = await s.login("owner@example.com");
    const installId = [...s.backend.installs.keys()][1];
    await s.call(`/admin/api/installs/${installId}/revoke`, { method: "POST", token: v, body: { reason: "viewer attempt" } });
    await s.call(`/admin/api/installs/${installId}/revoke`, { method: "POST", token: o, body: { reason: "device stolen" } });
    const log = (await s.call("/admin/api/audit", { token: v })).json.entries as any[];
    expect(log.find((e) => e.action === "revoke" && e.result === "denied" && e.role === "viewer")).toBeTruthy();
    expect(log.find((e) => e.action === "revoke" && e.result === "ok" && e.reason === "device stolen")).toBeTruthy();
    const entry = s.admin.auditLog()[0] as any;
    expect(() => { entry.reason = "tampered"; }).toThrow();
    expect((await s.call("/admin/api/audit", { method: "DELETE", token: o })).status).toBe(404);
  });
});

describe("revoke vs reissue semantics", () => {
  it("reissue kills old token with 'reissued' and the device silently reconnects; revoke blocks reconnect", async () => {
    const s = await stack();
    const o = await s.login("owner@example.com");
    // a fresh install we hold tokens for
    await s.backend.requestOtp("e2e.example@example.com");
    const v = await s.backend.verifyOtp("e2e.example@example.com", s.backend.lastOtpByEmail.get("e2e.example@example.com")!);
    if (!v.ok) throw new Error("otp");
    const reg = await s.backend.registerInstall({ userId: v.profile.userId, deviceName: "Test PC", publicKey: "EXAMPLE-E2E-KEY" });
    if (!reg.ok) throw new Error("reg");
    const old = reg.session.accessToken;

    await s.call(`/admin/api/installs/${reg.install.id}/reissue`, { method: "POST", token: o, body: { reason: "support request" } });
    const st = await s.call("/v1/status", { token: old });
    expect(st.json.code).toBe("reissued");
    expect(st.json.reconnectRequired).toBe(true);
    const d = await s.call("/v1/dictation/transcribe", { method: "POST", token: old, body: { audioMinutes: 0.1 } });
    expect(d.status).toBe(401);
    expect(d.json.code).toBe("reissued");
    const again = await s.call("/v1/installs/register", { method: "POST", body: { userId: v.profile.userId, deviceName: "Test PC", publicKey: "EXAMPLE-E2E-KEY" } });
    expect(again.status).toBe(200);
    expect(again.json.installId).toBe(reg.install.id);
    const ok = await s.call("/v1/dictation/transcribe", { method: "POST", token: again.json.session.accessToken, body: { audioMinutes: 0.1 } });
    expect(ok.status).toBe(200);

    await s.call(`/admin/api/installs/${reg.install.id}/revoke`, { method: "POST", token: o, body: { reason: "abuse report" } });
    const after = await s.call("/v1/dictation/transcribe", { method: "POST", token: again.json.session.accessToken, body: { audioMinutes: 0.1 } });
    expect(after.status).toBe(401);
    expect(after.json.code).toBe("revoked");
    expect((await s.call(`/admin/api/installs/${reg.install.id}/reissue`, { method: "POST", token: o, body: { reason: "try reissue" } })).status).toBe(409);
  });

  it("pool full is reported by status and blocks managed dictation with 429", async () => {
    const s = await stack();
    await s.backend.requestOtp("pool.example@example.com");
    const v = await s.backend.verifyOtp("pool.example@example.com", s.backend.lastOtpByEmail.get("pool.example@example.com")!);
    if (!v.ok) throw new Error("otp");
    const reg = await s.backend.registerInstall({ userId: v.profile.userId, deviceName: "P", publicKey: "EXAMPLE-POOL-KEY" });
    if (!reg.ok) throw new Error("reg");
    s.admin.poolFullIds.add(s.admin.poolFor(reg.install.id));
    const st = await s.call("/v1/status", { token: reg.session.accessToken });
    expect(st.json.poolFull).toBe(true);
    expect(st.json.statusCheckIntervalSeconds).toBe(900);
    const d = await s.call("/v1/dictation/transcribe", { method: "POST", token: reg.session.accessToken, body: { audioMinutes: 0.1 } });
    expect(d.status).toBe(429);
    expect(d.json.code).toBe("pool_full");
  });
});

describe("no secret leakage through admin API/UI", () => {
  it("never returns tokens, hashes, raw device keys or pool secrets", async () => {
    const s = await stack();
    const o = await s.login("owner@example.com");
    const secrets: string[] = ["EXAMPLE-PLACEHOLDER-A", "EXAMPLE-DEVICE-KEY-0-0", o];
    for (const t of s.backend.tokens) secrets.push(t.accessTokenHash, t.refreshTokenHash);
    const bodies: string[] = [];
    for (const p of ["/admin/api/me", "/admin/api/dashboard", "/admin/api/users", "/admin/api/audit"]) bodies.push((await s.call(p, { token: o })).text);
    for (const id of s.backend.profiles.keys()) bodies.push((await s.call(`/admin/api/users/${id}`, { token: o })).text);
    bodies.push((await s.call("/admin")).text);
    const all = bodies.join("\n");
    for (const secret of secrets) expect(all).not.toContain(secret);
    expect(all).not.toMatch(/accessTokenHash|refreshTokenHash|refreshToken"|accessToken"|publicKey"/);
    expect(all).toContain("tok_");
  });
});
