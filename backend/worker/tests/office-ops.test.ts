import { describe, expect, it } from "vitest";
import { createApp, createMockEnv } from "../src/app";
import { AdminService, can } from "../src/admin";
import { MockBackend } from "../mock/store";
import { MockAiGateway } from "../src/ai";
import { hashPassword } from "../src/passwords";
import { codeRecipient, OfficeOps, sanitizeReportLine } from "../src/office-ops";
import { snapshot, restore } from "../../node/persist";

describe("office roles", () => {
  it("keeps owner, admin, support, and viewer apart", () => {
    expect(can("owner", "billing")).toBe(true);
    expect(can("owner", "owner_settings")).toBe(true);
    expect(can("admin", "manage_keys")).toBe(true);
    expect(can("admin", "manage_settings")).toBe(true);
    expect(can("admin", "manage_team")).toBe(false);
    expect(can("admin", "billing")).toBe(false);
    expect(can("admin", "owner_settings")).toBe(false);
    expect(can("support", "manage_feedback")).toBe(true);
    expect(can("support", "view_keys")).toBe(true);
    expect(can("support", "manage_keys")).toBe(false);
    expect(can("viewer", "view")).toBe(true);
    expect(can("viewer", "view_keys")).toBe(false);
    expect(can("viewer", "manage_feedback")).toBe(false);
  });
});

describe("office modules", () => {
  it("reports health only from recorded samples and alerts on a real spike", () => {
    const ops = new OfficeOps();
    expect(ops.health().samples).toBe(0);
    expect(ops.health().alert).toBe(false);
    expect(ops.health().failureRate).toBeNull();
    for (let i = 0; i < 8; i++) ops.recordSpeech({ kind: "transcribe", outcome: "rate_limit", latencyMs: 100 + i });
    expect(ops.health().alert).toBe(false);
    ops.recordSpeech({ kind: "transcribe", outcome: "ok", latencyMs: 80, version: "0.1.14" });
    ops.recordSpeech({ kind: "transcribe", outcome: "unavailable", latencyMs: 90, version: "0.1.14" });
    const h = ops.health();
    expect(h.samples).toBe(10);
    expect(h.onlineFail).toBe(9);
    expect(h.alert).toBe(true);
    expect(h.p50).toBe(80);
    expect(h.byVersion["0.1.14"]).toBe(2);
    expect(ops.consumeAlert()).toBe(true);
    expect(ops.consumeAlert()).toBe(false);
  });

  it("stores only fingerprints in the key list and fails over at the cap", async () => {
    const ops = new OfficeOps();
    const secret = "gsk_test_secret_value_1234";
    const added = await ops.addKey("pool_a", secret, { dailyMinuteCap: 1 });
    expect(added?.fingerprint).toHaveLength(12);
    expect(JSON.stringify(ops.listKeys())).not.toContain(secret);
    await ops.addKey("pool_a", "gsk_backup_secret_value_999", { backup: true, dailyMinuteCap: 5 });
    expect(ops.selectSecret()).toBe(secret);
    ops.noteMinutes(added!.fingerprint, 1);
    expect(ops.selectSecret()).toBe("gsk_backup_secret_value_999");
    expect(ops.retireKey(added!.id)).toBe(true);
    expect(ops.listKeys().some((k) => k.id === added!.id)).toBe(false);
  });

  it("rejects report lines that carry a key", () => {
    expect(sanitizeReportLine("hook failed")).toBe("hook failed");
    expect(sanitizeReportLine("saw gsk_abcDEF123")).toBe("saw [key]");
    expect(sanitizeReportLine("data:audio/wav;base64,AAAA")).toBeNull();
  });

  it("keeps limits, feedback, and a pinned updater file", () => {
    const ops = new OfficeOps();
    expect(ops.pinnedLatest()).toBeNull();
    expect(ops.warnAt(1)).toBeNull();
    ops.setLimits(10, 0.8);
    expect(ops.warnAt(8)?.cap).toBe(10);
    const fb = ops.addFeedback({ source: "website", issueType: "contact", text: "The contact form asked about dictation." });
    expect(fb?.issueType).toBe("contact");
    ops.setChannel("store", "0.1.14");
    ops.setPinnedLatest({ version: "0.1.13" }, true);
    expect(ops.pinnedLatest()).toEqual({ version: "0.1.13" });
    expect(ops.state.settings.channels.store).toBe("0.1.14");
  });

  it("restores ops state and redirects the owner code by default", async () => {
    expect(codeRecipient("info@kingdomvision.com", undefined)).toBe("taimoor.kingdomvision@gmail.com");
    expect(codeRecipient("other@kingdomvision.com", undefined)).toBe("other@kingdomvision.com");
    expect(codeRecipient("info@kingdomvision.com", "info@kingdomvision.com=owner@kingdomvision.com")).toBe("owner@kingdomvision.com");

    const env = createMockEnv();
    const backend = new MockBackend(env);
    const admin = new AdminService(backend, "supabase");
    const sent: string[] = [];
    admin.officeMailer = async (email) => { sent.push(email); return true; };
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, email2fa: true, totpEnrolled: false });
    admin.setOwnerPasswordHash("info@kingdomvision.com", await hashPassword("correct-horse-battery"));
    const step = await admin.login("info@kingdomvision.com", "correct-horse-battery", "");
    expect(step.ok && step.value.needsEmailCode).toBe(true);
    expect(sent[0]).toBe("taimoor.kingdomvision@gmail.com");

    await admin.ops.addKey("pool_a", "gsk_persist_secret_value");
    const snap = snapshot(backend, admin) as { ops: { keys: { secret: string }[] } };
    expect(snap.ops.keys[0].secret).toBe("gsk_persist_secret_value");
    const next = new AdminService(new MockBackend(env), "supabase");
    next.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true });
    restore(new MockBackend(env), next, snap as never);
    expect(next.ops.selectSecret()).toBe("gsk_persist_secret_value");
    expect(JSON.stringify(next.ops.listKeys())).not.toContain("gsk_persist_secret_value");
  });
});

describe("office module routes", () => {
  async function ownerApp() {
    const env = createMockEnv();
    const backend = new MockBackend(env);
    const admin = new AdminService(backend, "supabase");
    const password = "correct-horse-battery";
    admin.addAdmin({ email: "info@kingdomvision.com", role: "owner", label: "Owner", bootstrap: true, email2fa: false, totpEnrolled: false });
    admin.addAdmin({ email: "view@kingdomvision.com", role: "viewer", label: "View", email2fa: false, totpEnrolled: false, passwordHash: await hashPassword(password) });
    admin.addAdmin({ email: "ops@kingdomvision.com", role: "admin", label: "Admin", email2fa: false, totpEnrolled: false, passwordHash: await hashPassword(password) });
    admin.setOwnerPasswordHash("info@kingdomvision.com", await hashPassword(password));
    const handle = createApp(backend, new MockAiGateway(), env, admin);
    const token = async (email: string) => {
      const res = await handle(new Request("http://local/admin/api/login", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ email, password }) }));
      return ((await res.json()) as { token: string }).token;
    };
    return { handle, admin, owner: await token("info@kingdomvision.com"), viewer: await token("view@kingdomvision.com"), adminToken: await token("ops@kingdomvision.com") };
  }
  const call = (handle: (r: Request) => Promise<Response>, path: string, token: string, body?: unknown) =>
    handle(new Request("http://local" + path, { method: body ? "POST" : "GET", headers: { authorization: "Bearer " + token, "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined }));

  it("enforces key and owner-settings routes", async () => {
    const s = await ownerApp();
    expect((await call(s.handle, "/admin/api/ops/keys", s.viewer)).status).toBe(403);
    expect((await call(s.handle, "/admin/api/ops/health", s.viewer)).status).toBe(200);
    const added = await call(s.handle, "/admin/api/ops/keys", s.owner, { poolId: "pool_a", secret: "gsk_route_secret_value", reason: "add pool key" });
    const body = await added.json() as { key: { fingerprint: string } };
    expect(added.status).toBe(200);
    expect(JSON.stringify(body)).not.toContain("gsk_route_secret_value");
    expect(body.key.fingerprint).toHaveLength(12);
    const denied = await call(s.handle, "/admin/api/ops/settings", s.adminToken, { requireEmail2fa: true, reason: "turn on policy" });
    expect(denied.status).toBe(403);
    const ok = await call(s.handle, "/admin/api/ops/settings", s.owner, { requireEmail2fa: true, reason: "turn on policy" });
    expect(ok.status).toBe(200);
    expect(s.admin.ops.state.settings.requireEmail2fa).toBe(true);
  });

  it("owner can reset usage, mark a key primary, and resolve feedback", async () => {
    const s = await ownerApp();
    const backend = (s.admin as unknown as { backend: MockBackend }).backend;
    backend.profiles.set("u1", { userId: "u1", email: "a@kingdomvision.com", plan: "free", status: "active", fairUseDailyMinutes: 20, fairUseMonthlyMinutes: 300, createdAt: new Date().toISOString() });
    backend.events.push({ userId: "u1", installId: "i1", provider: "groq", kind: "transcribe", audioMinutes: 2, statusClass: "ok", createdAt: Date.now() });
    expect((await call(s.handle, "/admin/api/users/u1/reset-usage", s.viewer, { reason: "viewer try" })).status).toBe(403);
    expect((await call(s.handle, "/admin/api/users/u1/reset-usage", s.owner, { reason: "clear minutes" })).status).toBe(200);
    expect(backend.events).toHaveLength(0);
    const added = await call(s.handle, "/admin/api/ops/keys", s.owner, { poolId: "pool_b", secret: "gsk_primary_secret_value", reason: "add pool key" });
    const id = ((await added.json()) as { key: { id: string } }).key.id;
    expect((await call(s.handle, "/admin/api/ops/keys", s.owner, { op: "primary", id, reason: "make primary" })).status).toBe(200);
    expect((await call(s.handle, "/admin/api/ops/keys", s.viewer, { op: "retire", id, reason: "viewer try" })).status).toBe(403);
    const fb = s.admin.ops.addFeedback({ source: "app", issueType: "bug", text: "The shortcut missed a word." });
    expect((await call(s.handle, "/admin/api/ops/feedback", s.owner, { id: fb!.id, resolved: true, reason: "fixed in office" })).status).toBe(200);
    expect(s.admin.ops.state.feedback.find((f) => f.id === fb!.id)?.resolved).toBe(true);
    expect(s.admin.auditLog().some((e) => e.action === "reset_usage")).toBe(true);
  });
});
