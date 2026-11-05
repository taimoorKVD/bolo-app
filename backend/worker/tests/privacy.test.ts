import { describe, expect, it } from "vitest";
import { createLocalStack } from "../src/app";

describe("privacy defaults", () => {
  it("stores usage metadata only — no audio or transcript fields on events", async () => {
    const { backend } = createLocalStack();
    const email = "privacy@example.com";
    await backend.requestOtp(email);
    const verified = await backend.verifyOtp(email, backend.lastOtpByEmail.get(email)!);
    if (!verified.ok) return;
    const reg = await backend.registerInstall({
      userId: verified.profile.userId,
      deviceName: "PC",
      publicKey: "pk-privacy",
    });
    if (!reg.ok) return;
    await backend.dictate({
      accessToken: reg.session.accessToken,
      kind: "transcribe",
      audioMinutes: 0.5,
    });
    expect(backend.events).toHaveLength(1);
    const raw = JSON.stringify(backend.events[0]);
    expect(raw).not.toMatch(/audioBase64|transcript|dictated|clipboard/i);
    expect(backend.events[0]).toMatchObject({
      provider: "workers_ai",
      kind: "transcribe",
      statusClass: "ok",
      audioMinutes: 0.5,
    });
  });

  it("keeps OpenRouter and paid usage disabled by default", () => {
    const { backend } = createLocalStack();
    expect(backend.flags.openRouterEnabled).toBe(false);
    expect(backend.flags.paidUsageEnabled).toBe(false);
  });
});
