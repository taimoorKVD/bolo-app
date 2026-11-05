#!/usr/bin/env node
/**
 * Generate ADMIN_OWNER_PASSWORD_HASH. Reads the password from stdin (no echo in shells
 * that support it). Prints only the scrypt$ hash. Never logs the password.
 *
 *   printf '%s' "$PASSWORD" | node node/hash-owner-password.mjs
 */
import { randomBytes, scrypt as scryptCb } from "node:crypto";

const N = 16384, R = 8, P = 1, KEYLEN = 64, SALT_LEN = 16;
const chunks = [];
for await (const c of process.stdin) chunks.push(c);
const password = Buffer.concat(chunks).toString("utf8").replace(/\r?\n$/, "");
if (password.length < 14) {
  console.error("Password must be at least 14 characters.");
  process.exit(1);
}
const salt = randomBytes(SALT_LEN);
const hash = await new Promise((resolve, reject) => {
  scryptCb(password, salt, KEYLEN, { N, r: R, p: P }, (err, buf) => (err ? reject(err) : resolve(buf)));
});
process.stdout.write(`scrypt$${N}$${R}$${P}$${salt.toString("hex")}$${hash.toString("hex")}\n`);
