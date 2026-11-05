import {
  last4,
  publicTokenId,
  randomToken,
  sha256Hex,
} from "./crypto";
import type { TokenRecord } from "./types";

export interface IssuedTokens {
  record: TokenRecord;
  accessToken: string;
  refreshToken: string;
}

export async function issueTokenPair(installId: string, ttlSeconds: number, familyId?: string): Promise<IssuedTokens> {
  const accessToken = randomToken(32);
  const refreshToken = randomToken(32);
  const now = Date.now();
  const record: TokenRecord = {
    id: crypto.randomUUID(),
    installId,
    tokenPublicId: publicTokenId(),
    accessTokenHash: await sha256Hex(accessToken),
    accessLast4: last4(accessToken),
    accessExpiresAt: now + ttlSeconds * 1000,
    refreshTokenHash: await sha256Hex(refreshToken),
    refreshFamilyId: familyId ?? crypto.randomUUID(),
    issuedAt: now,
  };
  return { record, accessToken, refreshToken };
}

export async function rotateRefresh(
  previous: TokenRecord,
  presentedRefreshToken: string,
  ttlSeconds: number,
): Promise<{ ok: true; issued: IssuedTokens } | { ok: false; reason: "reuse" | "mismatch" | "revoked" | "expired_family" }> {
  if (previous.revokedAt) {
    return { ok: false, reason: "revoked" };
  }
  const presentedHash = await sha256Hex(presentedRefreshToken);
  if (presentedHash !== previous.refreshTokenHash) {
    // Reuse of an already-rotated refresh token is a theft signal when the
    // family still has a newer active token; callers revoke the family.
    return { ok: false, reason: "mismatch" };
  }
  const issued = await issueTokenPair(previous.installId, ttlSeconds, previous.refreshFamilyId);
  issued.record.refreshFamilyId = previous.refreshFamilyId;
  return { ok: true, issued };
}

export function isAccessValid(record: TokenRecord, now = Date.now()): boolean {
  return !record.revokedAt && record.accessExpiresAt > now;
}
