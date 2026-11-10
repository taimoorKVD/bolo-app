import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createApp, createMockEnv } from "../worker/src/app";
import { MockBackend } from "../worker/mock/store";
import { AdminService } from "../worker/src/admin";
import type { WorkerEnv } from "../worker/src/types";
import { GroqGateway } from "./groq";
import { Smtp2GoMailer, SmtpMailer, smtpConfigFromEnv, type Mailer } from "./mailer";
import { TotpVerifier } from "./totp";
import { FileStateStore, SupabaseStateStore, autosave, restore, snapshot } from "./persist";
import { loadDotEnv } from "./load-env";
import { boloProxyContactPhp as proxyContactPhp, tryStatic } from "./static";

// Named exactly as the website deploy's route patch searches, so a later
// site publish leaves this file alone instead of inserting a second proxy.
async function boloProxyContactPhp(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
  return proxyContactPhp(req, res);
}

/**
 * Self-hosted Bolo API + admin portal + static site for Plesk Passenger / PM2.
 * Same /v1 and /office contracts. Single process. Docroot = <app>/public.
 */
const appRoot = process.env.BOLO_APP_ROOT
  || (existsSync(join(process.cwd(), "dist", "server.mjs")) || existsSync(join(process.cwd(), "package.json"))
      ? process.cwd()
      : dirname(fileURLToPath(import.meta.url)));
const envFile = loadDotEnv(appRoot);
const E = process.env;
if (envFile) console.log(`env: loaded ${envFile}`);
const need = (k: string) => { const v = E[k]; if (!v) { console.error(`Missing env ${k}`); process.exit(1); } return v; };

const env: WorkerEnv = {
  ...createMockEnv(),
  REQUIRE_DEVICE_PROOF: "true",
  WHISPER_MODEL: E.WHISPER_MODEL || "whisper-large-v3",
  CLEANUP_MODEL: E.CLEANUP_MODEL || "llama-3.3-70b-versatile",
  ...Object.fromEntries(["FAIR_USE_DAILY_MINUTES", "FAIR_USE_MONTHLY_MINUTES", "FAIR_USE_NEW_INSTALL_DAILY_MINUTES", "MAX_INSTALLS_PER_USER", "ACCESS_TOKEN_TTL_SECONDS", "GLOBAL_DAILY_AUDIO_MINUTES", "KILL_SWITCH"].filter((k) => E[k]).map((k) => [k, E[k]!])),
};

class ProdBackend extends MockBackend {
  constructor(e: WorkerEnv, private mailer: Mailer) { super(e); }
  override async requestOtp(email: string) {
    const r = await super.requestOtp(email);
    if (!r.ok) return r;
    const n = email.trim().toLowerCase();
    const code = this.lastOtpByEmail.get(n)!; this.lastOtpByEmail.delete(n);
    if (!(await this.mailer.sendOtp(n, code))) { this.otps.delete(n); return { ok: false as const, code: "otp_send_failed" }; }
    return r;
  }
}

const smtp = smtpConfigFromEnv(E);
const mailer: Mailer = smtp ? new SmtpMailer(smtp, need("SMTP2GO_SENDER")) : new Smtp2GoMailer(need("SMTP2GO_API_KEY"), need("SMTP2GO_SENDER"));
console.log(`mail: ${smtp ? `smtp ${smtp.host}:${smtp.port}` : "smtp2go api"}`);
const backend = new ProdBackend(env, mailer);
backend.requireDeviceProof = true;
backend.statusInfo = { provider: "groq", models: { whisper: env.WHISPER_MODEL, cleanup: env.CLEANUP_MODEL } };
const admin = new AdminService(backend, "supabase");
// ADMIN_ACCOUNTS = "owner:you@x.com:Name;support:a@x.com:Name"  ADMIN_TOTP_SECRETS = "you@x.com:BASE32;a@x.com:BASE32"
for (const part of need("ADMIN_ACCOUNTS").split(";").filter(Boolean)) {
  const [role, email, label] = part.split(":");
  if (!["owner", "support", "viewer"].includes(role)) throw new Error(`bad admin role ${role}`);
  admin.addAdmin({
    email: email.toLowerCase(),
    role: role as "owner",
    label: label || email,
    bootstrap: true,
    totpEnrolled: true,
    enabled: true,
  });
}
const secrets = new Map(need("ADMIN_TOTP_SECRETS").split(";").filter(Boolean).map((p) => { const [e, s] = p.split(":"); return [e.toLowerCase(), s] as [string, string]; }));
for (const [email, secret] of secrets) admin.setTotpSecret(email, secret, true);
const ownerEmail = [...admin.exportStaff()].find((a) => a.bootstrap && a.role === "owner")?.email;
if (!ownerEmail) throw new Error("ADMIN_ACCOUNTS must include an owner");
const ownerEmailFixed = ownerEmail;
// Optional env hash — never required when using one-time /office/setup.
const ownerHashEnv = (E.ADMIN_OWNER_PASSWORD_HASH || "").trim();
if (ownerHashEnv) admin.setOwnerPasswordHash(ownerEmailFixed, ownerHashEnv);
admin.configureOwnerSetup(E.OFFICE_SETUP_TOKEN);
const totp = new TotpVerifier(secrets);
admin.codeVerifier = (e, c) => totp.verify(e, c);

const keys = need("GROQ_API_KEYS").split(",").map((s) => s.trim()).filter(Boolean);
const ai = new GroqGateway(keys);
const store = E.SUPABASE_URL ? new SupabaseStateStore(E.SUPABASE_URL, need("SUPABASE_SERVICE_ROLE_KEY")) : new FileStateStore(E.STATE_FILE || "./bolo-state.json");
const saver = autosave(store, () => snapshot(backend, admin));
const allowIps = (E.ADMIN_ALLOWED_IPS || "").split(",").map((s) => s.trim()).filter(Boolean);
const trustProxy = E.TRUST_PROXY !== "false";
const OFFICE = "/office";
const PUBLIC = E.PUBLIC_DIR || join(appRoot, "public");
const MAX_BODY = 36 * 1024 * 1024;

async function main() {
  restore(backend, admin, (await store.load()) as never);
  // Persisted snapshot hash wins; env hash only fills a gap (setup page is preferred).
  if (ownerHashEnv) admin.setOwnerPasswordHash(ownerEmailFixed, ownerHashEnv, { onlyIfMissing: true });
  admin.configureOwnerSetup(E.OFFICE_SETUP_TOKEN);
  if (!admin.ownerHasPassword() && !admin.setupAvailable()) {
    console.error("Owner has no password hash and OFFICE_SETUP_TOKEN is missing/too short (need >=24 chars).");
    process.exit(1);
  }
  for (const [email, secret] of secrets) admin.setTotpSecret(email, secret, true);
  for (const [i, k] of keys.entries()) await admin.addPool({ id: `pool_groq_${i + 1}`, label: `Groq · Pool ${i + 1}`, provider: "groq", status: "active" }, k);
  const handle = createApp(backend, ai, env, admin);
  const port = Number(E.PORT || 8787);

  createServer(async (req, res) => {
    try {
      const ip = (trustProxy ? String(req.headers["x-forwarded-for"] || "").split(",")[0].trim() : "") || req.socket.remoteAddress || "";
      const rawPath = (req.url || "/").split("?")[0];
      // Set this before any /office return so errors and API routes stay out of indexes too.
      if (rawPath === OFFICE || rawPath.startsWith(OFFICE + "/")) res.setHeader("x-robots-tag", "noindex, nofollow");
      // Public admin lives at /office; /admin only redirects. Internally the tested app still routes /admin.
      if (rawPath === "/admin" || rawPath.startsWith("/admin/")) { res.statusCode = 301; res.setHeader("location", OFFICE + rawPath.slice(6)); return res.end(); }
      const isApi = rawPath === "/v1" || rawPath.startsWith("/v1/") || rawPath === "/health" || rawPath === OFFICE || rawPath.startsWith(OFFICE + "/");
      if (!isApi && (req.method === "GET" || req.method === "HEAD")) {
        // Apache rewrites /contact to PHP. Proxy index.php when this process still sees the path.
        if ((rawPath === "/contact" || rawPath === "/contact/") && await boloProxyContactPhp(req, res)) return;
        if (await tryStatic(req, res, PUBLIC)) return;
      }
      if (rawPath === "/v1/health") req.url = "/health";
      if (rawPath === OFFICE || rawPath.startsWith(OFFICE + "/")) req.url = "/admin" + (req.url || "").slice(OFFICE.length);
      const path = (req.url || "/").split("?")[0];
      if (path.startsWith("/admin") && allowIps.length && !allowIps.includes(ip.replace(/^::ffff:/, ""))) { res.statusCode = 404; return res.end(); }
      if (path === "/admin/preview") { res.statusCode = 404; return res.end(); }
      const chunks: Buffer[] = []; let size = 0;
      for await (const c of req) { size += c.length; if (size > MAX_BODY) { res.statusCode = 413; return res.end('{"error":"Audio too large.","code":"audio_too_large"}'); } chunks.push(Buffer.from(c)); }
      const method = req.method || "GET";
      const request = new Request(`https://${req.headers.host || "localhost"}${req.url || "/"}`, { method, headers: req.headers as HeadersInit, body: ["GET", "HEAD"].includes(method) ? undefined : Buffer.concat(chunks) });
      let response: Response;
      try { response = await handle(request); }
      catch (e) { const st = (e as { status?: number }).status; response = Response.json(st === 429 ? { error: "Pool busy.", code: "pool_full", retryAfterMs: 900000 } : { error: "Upstream error.", code: "upstream_error" }, { status: st === 429 ? 429 : 502 }); console.error("request failed:", (e as Error).message); }
      res.statusCode = response.status;
      response.headers.forEach((v, k) => res.setHeader(k, v));
      res.setHeader("strict-transport-security", "max-age=31536000");
      res.setHeader("x-content-type-options", "nosniff");
      if (path.startsWith("/admin")) { res.setHeader("x-frame-options", "DENY"); res.setHeader("cache-control", "no-store"); }
      let out = Buffer.from(await response.arrayBuffer());
      if (path.startsWith("/admin") && (response.headers.get("content-type") || "").includes("text/html")) {
        out = Buffer.from(out.toString("utf8").replaceAll('"/admin/api"', `"${OFFICE}/api"`));
        res.setHeader("content-length", out.length);
      }
      res.end(out);
      if (method === "POST" && (path === "/admin/api/setup" || path.endsWith("/api/setup"))) await saver.flush();
      else if (method !== "GET" && method !== "OPTIONS" || path.startsWith("/v1/status")) saver.touch();
    } catch (e) { console.error("server error:", (e as Error).name); res.statusCode = 500; res.end('{"error":"Internal error.","code":"internal"}'); }
  }).listen(port, E.HOST || "127.0.0.1", () => console.log(`Bolo listening on ${E.HOST || "127.0.0.1"}:${port} (state: ${E.SUPABASE_URL ? "supabase" : "file"}; public: ${existsSync(PUBLIC) ? PUBLIC : "none"})`));

  for (const sig of ["SIGTERM", "SIGINT"]) process.on(sig, async () => { await saver.flush(); process.exit(0); });
  setInterval(() => saver.touch(), 60_000).unref();
}
main().catch((e) => { console.error("boot failed:", (e as Error).message); process.exit(1); });
