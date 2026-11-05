import { createHmac, timingSafeEqual } from "node:crypto";

function base32(s: string): Buffer {
  const A = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let bits = 0, val = 0; const out: number[] = [];
  for (const c of s.replace(/[\s=]/g, "").toUpperCase()) {
    const i = A.indexOf(c); if (i < 0) throw new Error("bad base32");
    val = (val << 5) | i; bits += 5;
    if (bits >= 8) { out.push((val >>> (bits - 8)) & 255); bits -= 8; }
  }
  return Buffer.from(out);
}

export function totpCode(secretB32: string, step: number, digits = 6): string {
  const msg = Buffer.alloc(8); msg.writeBigUInt64BE(BigInt(step));
  const h = createHmac("sha1", base32(secretB32)).update(msg).digest();
  const o = h[h.length - 1] & 15;
  const n = ((h[o] & 127) << 24) | (h[o + 1] << 16) | (h[o + 2] << 8) | h[o + 3];
  return String(n % 10 ** digits).padStart(digits, "0");
}

/** RFC 6238 check, +/-1 step, each step usable once per account. */
export class TotpVerifier {
  private lastStep = new Map<string, number>();
  constructor(private secrets: Map<string, string>, private now: () => number = () => Date.now()) {}
  async verify(email: string, code: string): Promise<boolean> {
    const secret = this.secrets.get(email.toLowerCase());
    if (!secret || !/^\d{6}$/.test(code)) return false;
    const cur = Math.floor(this.now() / 30000);
    for (const st of [cur - 1, cur, cur + 1]) {
      const c = Buffer.from(totpCode(secret, st));
      if (timingSafeEqual(c, Buffer.from(code)) && st > (this.lastStep.get(email.toLowerCase()) ?? -1)) {
        this.lastStep.set(email.toLowerCase(), st); return true;
      }
    }
    return false;
  }
}
