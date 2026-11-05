import { randomBytes, scrypt as scryptCb, timingSafeEqual } from "node:crypto";

/** scrypt params: N=16384, r=8, p=1, keylen=64 — strong salted hashes only. */
const N = 16384;
const R = 8;
const P = 1;
const KEYLEN = 64;
const SALT_LEN = 16;

export const MIN_PASSWORD_LENGTH = 14;

export type PasswordHashParts = {
  n: number;
  r: number;
  p: number;
  salt: Buffer;
  hash: Buffer;
};

function scryptAsync(password: string, salt: Buffer, keylen: number, opts: { N: number; r: number; p: number }): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    scryptCb(password, salt, keylen, opts, (err, derived) => {
      if (err) reject(err);
      else resolve(derived as Buffer);
    });
  });
}

/** Encode as scrypt$N$r$p$saltHex$hashHex */
export function encodePasswordHash(parts: PasswordHashParts): string {
  return `scrypt$${parts.n}$${parts.r}$${parts.p}$${parts.salt.toString("hex")}$${parts.hash.toString("hex")}`;
}

export function parsePasswordHash(encoded: string): PasswordHashParts | null {
  const m = /^scrypt\$(\d+)\$(\d+)\$(\d+)\$([0-9a-f]+)\$([0-9a-f]+)$/i.exec(encoded.trim());
  if (!m) return null;
  const n = Number(m[1]);
  const r = Number(m[2]);
  const p = Number(m[3]);
  const salt = Buffer.from(m[4], "hex");
  const hash = Buffer.from(m[5], "hex");
  if (!n || !r || !p || salt.length < 8 || hash.length < 16) return null;
  return { n, r, p, salt, hash };
}

export async function hashPassword(password: string): Promise<string> {
  if (typeof password !== "string" || password.length < MIN_PASSWORD_LENGTH) {
    throw new Error(`password must be at least ${MIN_PASSWORD_LENGTH} characters`);
  }
  const salt = randomBytes(SALT_LEN);
  const hash = await scryptAsync(password, salt, KEYLEN, { N, r: R, p: P });
  return encodePasswordHash({ n: N, r: R, p: P, salt, hash });
}

export async function verifyPassword(password: string, encoded: string | undefined | null): Promise<boolean> {
  if (!encoded || typeof password !== "string" || password.length === 0) return false;
  const parts = parsePasswordHash(encoded);
  if (!parts) return false;
  try {
    const got = await scryptAsync(password, parts.salt, parts.hash.length, {
      N: parts.n,
      r: parts.r,
      p: parts.p,
    });
    if (got.length !== parts.hash.length) return false;
    return timingSafeEqual(got, parts.hash);
  } catch {
    return false;
  }
}

export function isPasswordStrongEnough(password: string): boolean {
  return typeof password === "string" && password.length >= MIN_PASSWORD_LENGTH;
}

/** Constant-time string compare (pads to equal length; also requires equal original lengths). */
export function timingSafeEqualString(a: string, b: string): boolean {
  const ba = Buffer.from(a, "utf8");
  const bb = Buffer.from(b, "utf8");
  const len = Math.max(ba.length, bb.length, 1);
  const pa = Buffer.alloc(len);
  const pb = Buffer.alloc(len);
  ba.copy(pa);
  bb.copy(pb);
  const same = timingSafeEqual(pa, pb);
  return same && ba.length === bb.length;
}
