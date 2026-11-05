import { createHmac, randomBytes, timingSafeEqual } from "node:crypto";

const A = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

function base32Decode(s: string): Buffer {
  let bits = 0, val = 0;
  const out: number[] = [];
  for (const c of s.replace(/[\s=]/g, "").toUpperCase()) {
    const i = A.indexOf(c);
    if (i < 0) throw new Error("bad base32");
    val = (val << 5) | i;
    bits += 5;
    if (bits >= 8) {
      out.push((val >>> (bits - 8)) & 255);
      bits -= 8;
    }
  }
  return Buffer.from(out);
}

function base32Encode(buf: Buffer): string {
  let bits = 0, val = 0, out = "";
  for (const b of buf) {
    val = (val << 8) | b;
    bits += 8;
    while (bits >= 5) {
      out += A[(val >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) out += A[(val << (5 - bits)) & 31];
  return out;
}

export function generateTotpSecret(bytes = 20): string {
  return base32Encode(randomBytes(bytes));
}

export function totpCode(secretB32: string, step: number, digits = 6): string {
  const msg = Buffer.alloc(8);
  msg.writeBigUInt64BE(BigInt(step));
  const h = createHmac("sha1", base32Decode(secretB32)).update(msg).digest();
  const o = h[h.length - 1] & 15;
  const n = ((h[o] & 127) << 24) | (h[o + 1] << 16) | (h[o + 2] << 8) | h[o + 3];
  return String(n % 10 ** digits).padStart(digits, "0");
}

export function verifyTotpCode(
  secretB32: string,
  code: string,
  nowMs: number,
  lastUsedStep?: number,
): { ok: true; step: number } | { ok: false } {
  if (!secretB32 || !/^\d{6}$/.test(code)) return { ok: false };
  const cur = Math.floor(nowMs / 30000);
  for (const st of [cur - 1, cur, cur + 1]) {
    if (lastUsedStep !== undefined && st <= lastUsedStep) continue;
    const c = Buffer.from(totpCode(secretB32, st));
    if (timingSafeEqual(c, Buffer.from(code))) return { ok: true, step: st };
  }
  return { ok: false };
}

export function otpauthUri(email: string, secretB32: string, issuer = "Bolo Office"): string {
  const label = encodeURIComponent(`${issuer}:${email}`);
  const iss = encodeURIComponent(issuer);
  return `otpauth://totp/${label}?secret=${secretB32}&issuer=${iss}&algorithm=SHA1&digits=6&period=30`;
}
