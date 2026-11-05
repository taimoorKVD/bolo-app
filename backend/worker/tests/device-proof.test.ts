import { describe, expect, it } from "vitest";
import { createApp, createLocalStack, createMockEnv } from "../src/app";
import { MockBackend, registrationMessage } from "../mock/store";
import { AdminService } from "../src/admin";
import { MockAiGateway } from "../src/ai";
import { generateEd25519KeyPair, signEd25519 } from "../src/crypto";

function production() {
  const env = createMockEnv({ REQUIRE_DEVICE_PROOF: "true" });
  const backend = new MockBackend(env);
  const admin = new AdminService(backend, "supabase");
  const handle = createApp(backend, new MockAiGateway(), env, admin);
  return { backend, admin, handle };
}

const post = (path: string, body: unknown, token?: string) =>
  new Request(`http://local${path}`, {
    method: "POST",
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: JSON.stringify(body),
  });

async function signIn(handle: (r: Request) => Promise<Response>, backend: MockBackend, email: string) {
  await handle(post("/v1/auth/request-otp", { email }));
  const code = backend.lastOtpByEmail.get(email)!;
  const res = await handle(post("/v1/auth/verify-otp", { email, code }));
  expect(res.status).toBe(200);
  return (await res.json()) as { userId: string; registrationTicket: string };
}

async function signed(userId: string, keys: { publicKeySpkiBase64: string; privateKeyPkcs8Base64: string }, at = Date.now()) {
  const publicKey = keys.publicKeySpkiBase64;
  return { publicKey, signedAtMs: at, signature: await signEd25519(keys.privateKeyPkcs8Base64, registrationMessage(userId, publicKey, at)) };
}

async function register(handle: (r: Request) => Promise<Response>, body: Record<string, unknown>) {
  const res = await handle(post("/v1/installs/register", { deviceName: "Test PC", ...body }));
  return { status: res.status, body: (await res.json()) as any };
}

describe("production device registration", () => {
  it("rejects unsigned registration and accepts signed + OTP ticket", async () => {
    const { handle, backend } = production();
    const v = await signIn(handle, backend, "p1@example.com");
    expect(v.registrationTicket).toMatch(/^rt_/);
    let r = await register(handle, { userId: v.userId, publicKey: "UNSIGNED", registrationTicket: v.registrationTicket });
    expect(r.status).toBe(401);
    expect(r.body.code).toBe("device_proof_required");

    const keys = await generateEd25519KeyPair();
    r = await register(handle, { userId: v.userId, ...(await signed(v.userId, keys)) });
    expect(r.body.code).toBe("registration_ticket_required");

    r = await register(handle, { userId: v.userId, registrationTicket: v.registrationTicket, ...(await signed(v.userId, keys)) });
    expect(r.status).toBe(200);
    expect(JSON.stringify(r.body)).not.toContain(keys.privateKeyPkcs8Base64);
  });

  it("rejects bad, stale and replayed signatures and reused tickets", async () => {
    const { handle, backend } = production();
    const v = await signIn(handle, backend, "p2@example.com");
    const keys = await generateEd25519KeyPair();
    const other = await generateEd25519KeyPair();
    const forged = { ...(await signed(v.userId, other)), publicKey: keys.publicKeySpkiBase64 };
    let r = await register(handle, { userId: v.userId, registrationTicket: v.registrationTicket, ...forged });
    expect(r.body.code).toBe("device_proof_invalid");

    const stale = await signed(v.userId, keys, Date.now() - 10 * 60 * 1000);
    r = await register(handle, { userId: v.userId, registrationTicket: v.registrationTicket, ...stale });
    expect(r.body.code).toBe("device_proof_stale");

    const good = await signed(v.userId, keys);
    r = await register(handle, { userId: v.userId, registrationTicket: v.registrationTicket, ...good });
    expect(r.status).toBe(200);
    r = await register(handle, { userId: v.userId, ...good });
    expect(r.body.code).toBe("device_proof_replayed");

    // Ticket is single use: a second new device can't reuse it.
    const third = await generateEd25519KeyPair();
    r = await register(handle, { userId: v.userId, registrationTicket: v.registrationTicket, ...(await signed(v.userId, third)) });
    expect(r.body.code).toBe("registration_ticket_invalid");
  });

  it("after admin reissue, refresh says reissued (no theft lockout) and the signed device reconnects", async () => {
    const { handle, backend } = production();
    const v = await signIn(handle, backend, "p3@example.com");
    const keys = await generateEd25519KeyPair();
    const first = await register(handle, { userId: v.userId, registrationTicket: v.registrationTicket, ...(await signed(v.userId, keys)) });
    const installId = first.body.installId as string;
    backend.adminReissue(installId, "admin_reissue");

    const refreshed = await handle(post("/v1/auth/refresh", { refreshToken: first.body.session.refreshToken }));
    expect(refreshed.status).toBe(401);
    expect(((await refreshed.json()) as { code: string }).code).toBe("reissued");
    expect(backend.installs.get(installId)!.status).toBe("reissue_pending");

    // Unsigned silent reconnect is refused in production.
    let r = await register(handle, { userId: v.userId, publicKey: keys.publicKeySpkiBase64 });
    expect(r.body.code).toBe("device_proof_required");

    // Signed reconnect needs no new OTP and keeps the same install.
    r = await register(handle, { userId: v.userId, ...(await signed(v.userId, keys)) });
    expect(r.status).toBe(200);
    expect(r.body.installId).toBe(installId);
    expect(backend.installs.get(installId)!.status).toBe("active");
  });

  it("a revoked device key can never register again, even with a fresh OTP", async () => {
    const { handle, backend } = production();
    const v = await signIn(handle, backend, "p4@example.com");
    const keys = await generateEd25519KeyPair();
    const first = await register(handle, { userId: v.userId, registrationTicket: v.registrationTicket, ...(await signed(v.userId, keys)) });
    const install = backend.installs.get(first.body.installId)!;
    install.status = "revoked";
    backend.revokeFamily(backend.tokens.find((t) => t.installId === install.id)!.refreshFamilyId, "admin_revoke");

    const again = await signIn(handle, backend, "p4@example.com");
    const r = await register(handle, { userId: again.userId, registrationTicket: again.registrationTicket, ...(await signed(again.userId, keys)) });
    expect(r.status).toBe(403);
    expect(r.body.code).toBe("device_revoked");

    const refreshed = await handle(post("/v1/auth/refresh", { refreshToken: first.body.session.refreshToken }));
    expect(refreshed.status).toBe(401);
  });

  it("local mock mode still allows unsigned registration but never a revoked key", async () => {
    const { backend } = createLocalStack();
    expect(backend.requireDeviceProof).toBe(false);
    await backend.requestOtp("m@example.com");
    const v = await backend.verifyOtp("m@example.com", backend.lastOtpByEmail.get("m@example.com")!);
    if (!v.ok) throw new Error("otp");
    const reg = await backend.registerInstall({ userId: v.profile.userId, deviceName: "PC", publicKey: "pk-local" });
    expect(reg.ok).toBe(true);
    if (!reg.ok) return;
    reg.install.status = "revoked";
    const again = await backend.registerInstall({ userId: v.profile.userId, deviceName: "PC", publicKey: "pk-local" });
    expect(again.ok).toBe(false);
    if (!again.ok) expect(again.code).toBe("device_revoked");
  });

  it("a backend built without explicit env defaults to requiring proof", () => {
    expect(new MockBackend({}).requireDeviceProof).toBe(true);
  });
});
