// Local smoke: boots server.ts deps with fakes. Run: node --import tsx node/smoke.ts
import assert from "node:assert";
import { totpCode, TotpVerifier } from "./totp";
import { MockBackend } from "../worker/mock/store";
import { AdminService } from "../worker/src/admin";
import { createMockEnv } from "../worker/src/app";
import { snapshot, restore } from "./persist";
import { hashPassword } from "../worker/src/passwords";
// RFC 6238 SHA1 vector (secret "12345678901234567890"), T=59 -> 94287082
assert.equal(totpCode("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 1, 8), "94287082");
const v = new TotpVerifier(new Map([["a@x.com", "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"]]), () => 59000);
assert.equal(await v.verify("a@x.com", totpCode("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 1)), true);
assert.equal(await v.verify("a@x.com", totpCode("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 1)), false, "replay");
const b = new MockBackend(createMockEnv()); const a = new AdminService(b, "supabase");
const pw = "smoke-password-14";
a.addAdmin({ email: "a@x.com", role: "owner", label: "A", totpEnrolled: true, enabled: true });
a.setOwnerPasswordHash("a@x.com", await hashPassword(pw));
a.setTotpSecret("a@x.com", "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", true);
assert.equal((await a.login("a@x.com", "wrong-password-x", "654321")).ok, false, "bad password refused");
a.codeVerifier = async (_e, c) => c === "654321";
assert.equal((await a.login("a@x.com", pw, "123456")).ok, false, "bad totp refused");
assert.equal((await a.login("a@x.com", pw, "654321")).ok, true);
await b.requestOtp("u@x.com"); const r = await b.verifyOtp("u@x.com", b.lastOtpByEmail.get("u@x.com")!);
assert.ok(r.ok);
const s = JSON.parse(JSON.stringify(snapshot(b, a)));
const b2 = new MockBackend(createMockEnv()); const a2 = new AdminService(b2, "supabase"); restore(b2, a2, s);
assert.equal(b2.profiles.size, 1); assert.equal(a2.auditLog().length, a.auditLog().length);
assert.ok(a2.exportStaff().some((x) => x.email === "a@x.com" && x.passwordHash?.startsWith("scrypt$")));
console.log("node adapter smoke: OK");
