import { describe, expect, it } from "vitest";
import { issueTokenPair, isAccessValid, rotateRefresh } from "../src/tokens";
import { sha256Hex } from "../src/crypto";

describe("install token pair", () => {
  it("issues hashed access/refresh tokens without storing plaintext", async () => {
    const issued = await issueTokenPair("install-1", 900);
    expect(issued.accessToken).toHaveLength(64);
    expect(issued.refreshToken).toHaveLength(64);
    expect(issued.record.tokenPublicId.startsWith("tok_")).toBe(true);
    expect(issued.record.accessTokenHash).toBe(await sha256Hex(issued.accessToken));
    expect(issued.record.refreshTokenHash).toBe(await sha256Hex(issued.refreshToken));
    expect(issued.record.accessLast4).toBe(issued.accessToken.slice(-4));
    expect(JSON.stringify(issued.record)).not.toContain(issued.accessToken);
    expect(JSON.stringify(issued.record)).not.toContain(issued.refreshToken);
    expect(isAccessValid(issued.record)).toBe(true);
  });

  it("rotates refresh and keeps the family id", async () => {
    const first = await issueTokenPair("install-1", 900);
    const rotated = await rotateRefresh(first.record, first.refreshToken, 900);
    expect(rotated.ok).toBe(true);
    if (!rotated.ok) return;
    expect(rotated.issued.record.refreshFamilyId).toBe(first.record.refreshFamilyId);
    expect(rotated.issued.refreshToken).not.toBe(first.refreshToken);
  });

  it("rejects a mismatched refresh token", async () => {
    const first = await issueTokenPair("install-1", 900);
    const rotated = await rotateRefresh(first.record, "deadbeef", 900);
    expect(rotated).toEqual({ ok: false, reason: "mismatch" });
  });
});
