import { describe, expect, it } from "vitest";
import { createLocalStack } from "../src/app";

describe("refresh rotation and reuse protection", () => {
  it("rotates refresh tokens and revokes the install on reuse", async () => {
    const { backend } = createLocalStack();
    const email = "reuse@example.com";
    await backend.requestOtp(email);
    const verified = await backend.verifyOtp(email, backend.lastOtpByEmail.get(email)!);
    expect(verified.ok).toBe(true);
    if (!verified.ok) return;
    const reg = await backend.registerInstall({
      userId: verified.profile.userId,
      deviceName: "PC",
      publicKey: "pk-reuse",
    });
    expect(reg.ok).toBe(true);
    if (!reg.ok) return;

    const oldRefresh = reg.session.refreshToken;
    const rotated = await backend.refresh(oldRefresh);
    expect(rotated.ok).toBe(true);
    if (!rotated.ok) return;
    expect(rotated.session.refreshToken).not.toBe(oldRefresh);

    const reused = await backend.refresh(oldRefresh);
    expect(reused.ok).toBe(false);
    if (reused.ok) return;
    expect(reused.code).toBe("refresh_reuse");

    const install = backend.installs.get(reg.install.id)!;
    expect(install.status).toBe("revoked");
    expect(install.reconnectRequired).toBe(true);
  });

  it("admin reissue forces reconnect on next access", async () => {
    const { backend, handle } = createLocalStack();
    const email = "reissue@example.com";
    await backend.requestOtp(email);
    const verified = await backend.verifyOtp(email, backend.lastOtpByEmail.get(email)!);
    if (!verified.ok) return;
    const reg = await backend.registerInstall({
      userId: verified.profile.userId,
      deviceName: "PC",
      publicKey: "pk-reissue",
    });
    if (!reg.ok) return;

    expect(backend.adminReissue(reg.install.id, "user stuck")).toBe(true);
    const res = await handle(
      new Request("http://local/v1/dictation/cleanup", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          authorization: `Bearer ${reg.session.accessToken}`,
        },
        body: JSON.stringify({ text: "hello" }),
      }),
    );
    expect(res.status).toBe(401);
    const body = (await res.json()) as { code: string };
    expect(body.code).toBe("reissued");
  });
});
