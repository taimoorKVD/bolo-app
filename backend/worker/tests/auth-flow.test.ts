import { describe, expect, it } from "vitest";
import { createLocalStack } from "../src/app";

describe("auth + install HTTP flow", () => {
  it("signs up with OTP, registers an install, and dictates", async () => {
    const { handle, backend } = createLocalStack();
    const email = "layman@example.com";

    let res = await handle(
      new Request("http://local/v1/auth/request-otp", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ email }),
      }),
    );
    expect(res.status).toBe(200);
    const code = backend.lastOtpByEmail.get(email);
    expect(code).toMatch(/^\d{6}$/);

    res = await handle(
      new Request("http://local/v1/auth/verify-otp", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ email, code }),
      }),
    );
    expect(res.status).toBe(200);
    const verified = (await res.json()) as { userId: string };
    expect(verified.userId).toBeTruthy();

    res = await handle(
      new Request("http://local/v1/installs/register", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          userId: verified.userId,
          deviceName: "Backup PC",
          appVersion: "0.1.8",
          publicKey: "TEST_PUBLIC_KEY_MATERIAL",
        }),
      }),
    );
    expect(res.status).toBe(200);
    const registered = (await res.json()) as {
      installId: string;
      session: { accessToken: string; refreshToken: string; tokenPublicId: string };
    };
    expect(registered.session.tokenPublicId.startsWith("tok_")).toBe(true);

    res = await handle(
      new Request("http://local/v1/dictation/transcribe", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          authorization: `Bearer ${registered.session.accessToken}`,
        },
        body: JSON.stringify({ audioMinutes: 0.2, audioBase64: btoa("fake-audio") }),
      }),
    );
    expect(res.status).toBe(200);
    const dictated = (await res.json()) as { text: string; provider: string };
    expect(dictated.provider).toBe("workers_ai");
    expect(dictated.text.length).toBeGreaterThan(0);

    res = await handle(
      new Request("http://local/v1/status", {
        headers: { authorization: `Bearer ${registered.session.accessToken}` },
      }),
    );
    const status = (await res.json()) as { authenticated: boolean; remainingDailyMinutes: number };
    expect(status.authenticated).toBe(true);
    expect(status.remainingDailyMinutes).toBeLessThan(20);
  });

  it("returns kill_switch without calling AI", async () => {
    const { handle, backend } = createLocalStack();
    backend.setKillSwitch(true);
    const email = "kill@example.com";
    await backend.requestOtp(email);
    const code = backend.lastOtpByEmail.get(email)!;
    const verified = await backend.verifyOtp(email, code);
    expect(verified.ok).toBe(true);
    if (!verified.ok) return;
    const reg = await backend.registerInstall({
      userId: verified.profile.userId,
      deviceName: "PC",
      publicKey: "pk-kill",
    });
    expect(reg.ok).toBe(true);
    if (!reg.ok) return;
    const res = await handle(
      new Request("http://local/v1/dictation/transcribe", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          authorization: `Bearer ${reg.session.accessToken}`,
        },
        body: JSON.stringify({ audioMinutes: 0.1 }),
      }),
    );
    expect(res.status).toBe(429);
    const body = (await res.json()) as { code: string };
    expect(body.code).toBe("kill_switch");
  });
});
