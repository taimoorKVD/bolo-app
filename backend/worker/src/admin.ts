import { randomToken, sha256Hex } from "./crypto";
import { evaluateCaps } from "./caps";
import type { MockBackend } from "../mock/store";
import type { Install, Plan, Profile, UsageEvent } from "./types";
import { hashPassword, isPasswordStrongEnough, MIN_PASSWORD_LENGTH, verifyPassword, timingSafeEqualString } from "./passwords";
import { generateTotpSecret, otpauthUri, verifyTotpCode } from "./totp-util";
import { qrSvg } from "./qr-svg";
import { codeRecipient, OfficeOps } from "./office-ops";

/**
 * Phase 4 admin service. Real authorization (roles, recent-auth, reasons,
 * append-only audit) enforced here; only the *identity provider* is mocked
 * locally. Production: Supabase Auth admin sign-in with TOTP MFA (aal2 JWT).
 */
export type AdminRole = "owner" | "admin" | "support" | "viewer";
export type AdminAction =
  | "view"
  | "view_full_email"
  | "notice"
  | "revoke"
  | "reissue"
  | "move_pool"
  | "set_plan"
  | "suspend"
  | "kill_switch"
  | "manage_team"
  | "view_keys"
  | "manage_keys"
  | "manage_limits"
  | "manage_releases"
  | "manage_feedback"
  | "manage_settings"
  | "owner_settings"
  | "billing";

const STAFF: AdminAction[] = [
  "view", "view_full_email", "notice", "revoke", "reissue", "move_pool", "set_plan", "suspend", "kill_switch",
  "view_keys", "manage_keys", "manage_limits", "manage_releases", "manage_feedback", "manage_settings",
];
const PERMISSIONS: Record<AdminRole, ReadonlySet<AdminAction>> = {
  viewer: new Set<AdminAction>(["view"]),
  support: new Set<AdminAction>(["view", "view_full_email", "notice", "revoke", "reissue", "view_keys", "manage_feedback"]),
  admin: new Set<AdminAction>(STAFF),
  owner: new Set<AdminAction>([...STAFF, "manage_team", "owner_settings", "billing"]),
};
/** Actions that change access or money and need a reason + fresh MFA. */
const SENSITIVE: ReadonlySet<AdminAction> = new Set<AdminAction>([
  "revoke", "reissue", "move_pool", "set_plan", "suspend", "kill_switch",
]);

export function can(role: AdminRole, action: AdminAction): boolean {
  return PERMISSIONS[role].has(action);
}

export interface AdminAccount {
  email: string;
  role: AdminRole;
  label: string;
  passwordHash?: string;
  totpSecret?: string;
  enabled: boolean;
  totpEnrolled: boolean;
  /** Email sign-in code. Undefined in old snapshots until migration. */
  email2fa?: boolean;
  createdAt: string;
  /** Env-bootstrap account (owner/support/viewer from ADMIN_ACCOUNTS). */
  bootstrap?: boolean;
}

/** Safe staff row for API/UI — never includes password hashes or TOTP secrets. */
export interface StaffPublic {
  email: string;
  role: AdminRole;
  label: string;
  enabled: boolean;
  totpEnrolled: boolean;
  email2fa: boolean;
  createdAt: string;
  bootstrap: boolean;
}

export interface StaffEnrollment {
  email: string;
  secret: string;
  otpauthUrl: string;
  /** Self-contained SVG QR for otpauthUrl (no external services). */
  qrSvg?: string;
}

interface AdminSession {
  id: string;
  tokenHash: string;
  email: string;
  role: AdminRole;
  expiresAt: number;
  recentAuthAt: number;
  needsEnrollment: boolean;
}

interface LoginAttempt { fails: number; windowStart: number; lockedUntil: number }

export interface AuditEntry {
  readonly id: number;
  readonly at: string;
  readonly adminEmail: string;
  readonly role: AdminRole;
  readonly action: string;
  readonly target: string;
  readonly reason: string;
  readonly result: "ok" | "denied" | "error";
  readonly detail?: string;
}

export interface PoolKey {
  id: string;
  label: string;
  provider: string;
  fingerprint: string;
  status: "active" | "retired";
}

export interface Notice { id: string; userId: string; message: string; createdAt: string; createdBy: string }

export const RECENT_AUTH_MS = 5 * 60 * 1000;
const SESSION_TTL_MS = 8 * 60 * 60 * 1000;
/** LOCAL MOCK ONLY: TOTP stand-in so the dev portal works without Supabase. */
export const MOCK_TOTP_CODE = "123456";

export type AdminResult<T> = { ok: true; value: T } | { ok: false; status: number; code: string; message: string };
const fail = (status: number, code: string, message: string) => ({ ok: false as const, status, code, message });

export function maskEmail(email: string): string {
  const [local, domain] = email.split("@");
  if (!domain) return "***";
  return `${local.slice(0, 1)}***@${domain}`;
}

function startOfDay(now: number) { const d = new Date(now); d.setUTCHours(0, 0, 0, 0); return d.getTime(); }
function startOfMonth(now: number) { const d = new Date(now); d.setUTCDate(1); d.setUTCHours(0, 0, 0, 0); return d.getTime(); }
const round = (n: number) => Math.round(n * 100) / 100;

const LOGIN_FAIL_LIMIT = 5;
const LOGIN_WINDOW_MS = 15 * 60 * 1000;
const LOGIN_LOCK_MS = 15 * 60 * 1000;
const GENERIC_LOGIN_ERROR = "Sign-in failed.";
const EMAIL_CODE_TTL_MS = 10 * 60 * 1000;
const EMAIL_CODE_MAX_ATTEMPTS = 5;
const EMAIL_RESEND_MS = 60 * 1000;
const EMAIL_MAX_SENDS = 5;
/** Existing owner gets email 2FA when a snapshot has no explicit flag. */
const OWNER_EMAIL_2FA = "info@kingdomvision.com";

interface EmailCodeRow {
  email: string;
  purpose: "login" | "reauth" | "enable";
  salt: string;
  codeHash: string;
  challengeId: string;
  expiresAt: number;
  attempts: number;
  lastSentAt: number;
  sends: number;
}

export interface LoginValue {
  token?: string;
  role: AdminRole;
  email: string;
  label: string;
  needsEnrollment: boolean;
  needsEmailCode?: boolean;
  challenge?: string;
  resendAfterSeconds?: number;
  totpAvailable?: boolean;
  enrollment?: StaffEnrollment;
}

export class AdminService {
  private admins = new Map<string, AdminAccount>();
  private sessions: AdminSession[] = [];
  private audit: AuditEntry[] = [];
  readonly pools = new Map<string, PoolKey>();
  private installPool = new Map<string, string>();
  readonly notices: Notice[] = [];
  readonly poolFullIds = new Set<string>();
  private loginAttempts = new Map<string, LoginAttempt>();
  private lastTotpStep = new Map<string, number>();
  /** One-time enrollment payloads waiting to be shown (create or first login). */
  private pendingEnrollment = new Map<string, StaffEnrollment>();

  readonly ops = new OfficeOps();
  /** OFFICE_CODE_REDIRECT. Unset keeps the built-in owner mapping. */
  officeCodeRedirect?: string;
  constructor(private backend: MockBackend, private mode: "mock" | "supabase" = "mock", private now: () => number = () => Date.now()) {
    this.ops.setClock(() => this.now());
  }
  /** Test hook for recent-auth expiry. */
  setClock(fn: () => number) { this.now = fn; this.ops.setClock(fn); }
  isMock() { return this.mode === "mock"; }
  /** Optional external verifier (e.g. env bootstrap secrets). Falls back to account.totpSecret. */
  codeVerifier?: (email: string, code: string) => Promise<boolean>;
  /** Sends a 6-digit Office code. Production wires this to SMTP. Never log the code. */
  officeMailer?: (email: string, code: string) => Promise<boolean>;
  private emailCodes = new Map<string, EmailCodeRow>();
  private async codeOk(email: string, code: string) {
    if (this.mode === "mock") return code === MOCK_TOTP_CODE;
    const key = email.toLowerCase();
    if (this.codeVerifier && (await this.codeVerifier(email, code))) return true;
    const acct = this.admins.get(key);
    if (!acct?.totpSecret) return false;
    const res = verifyTotpCode(acct.totpSecret, code, this.now(), this.lastTotpStep.get(key));
    if (!res.ok) return false;
    this.lastTotpStep.set(key, res.step);
    return true;
  }

  addAdmin(a: Omit<AdminAccount, "enabled" | "totpEnrolled" | "createdAt"> & Partial<Pick<AdminAccount, "enabled" | "totpEnrolled" | "createdAt" | "passwordHash" | "totpSecret" | "bootstrap" | "email2fa">>) {
    const email = a.email.toLowerCase();
    const existing = this.admins.get(email);
    const merged: AdminAccount = {
      email,
      role: a.role,
      label: a.label,
      passwordHash: a.passwordHash ?? existing?.passwordHash,
      totpSecret: a.totpSecret ?? existing?.totpSecret,
      enabled: a.enabled ?? existing?.enabled ?? true,
      totpEnrolled: a.totpEnrolled ?? existing?.totpEnrolled ?? (Boolean(a.totpSecret || existing?.totpSecret) || this.mode === "mock"),
      email2fa: a.email2fa ?? existing?.email2fa,
      createdAt: a.createdAt ?? existing?.createdAt ?? new Date(this.now()).toISOString(),
      bootstrap: a.bootstrap ?? existing?.bootstrap,
    };
    this.admins.set(email, merged);
  }

  /**
   * Apply OWNER password hash from env (never logs the hash).
   * When onlyIfMissing is true, a persisted snapshot hash wins over env.
   */
  setOwnerPasswordHash(email: string, passwordHash: string, opts: { onlyIfMissing?: boolean } = {}) {
    const acct = this.admins.get(email.toLowerCase());
    if (!acct) throw new Error("owner account missing");
    if (!passwordHash.startsWith("scrypt$")) throw new Error("ADMIN_OWNER_PASSWORD_HASH must be a scrypt$ hash");
    if (opts.onlyIfMissing && acct.passwordHash) return false;
    acct.passwordHash = passwordHash;
    return true;
  }

  ownerEmail(): string | null {
    const o = [...this.admins.values()].find((a) => a.bootstrap && a.role === "owner")
      ?? [...this.admins.values()].find((a) => a.role === "owner");
    return o?.email ?? null;
  }

  ownerHasPassword(): boolean {
    const email = this.ownerEmail();
    if (!email) return false;
    return Boolean(this.admins.get(email)?.passwordHash);
  }

  private setupToken: string | null = null;
  private setupAttempts: LoginAttempt = { fails: 0, windowStart: 0, lockedUntil: 0 };
  private setupUsed = false;

  /** High-entropy one-time Office setup token from env OFFICE_SETUP_TOKEN. */
  configureOwnerSetup(token: string | undefined | null) {
    const t = (token || "").trim();
    this.setupToken = t.length >= 24 ? t : null;
  }

  /** True when bootstrap owner still needs a password and a setup token is configured. */
  setupAvailable(): boolean {
    if (this.mode === "mock") return false;
    if (this.setupUsed) return false;
    if (!this.setupToken) return false;
    return !this.ownerHasPassword();
  }

  private noteSetupFail() {
    const t = this.now();
    let st = this.setupAttempts;
    if (!st.windowStart || t - st.windowStart > LOGIN_WINDOW_MS) {
      st = { fails: 0, windowStart: t, lockedUntil: 0 };
    }
    st.fails += 1;
    if (st.fails >= LOGIN_FAIL_LIMIT) {
      st.lockedUntil = t + LOGIN_LOCK_MS;
      st.fails = 0;
      st.windowStart = t;
    }
    this.setupAttempts = st;
  }

  private setupLocked(): boolean {
    const t = this.now();
    if (this.setupAttempts.lockedUntil > t) return true;
    if (this.setupAttempts.windowStart && t - this.setupAttempts.windowStart > LOGIN_WINDOW_MS) {
      this.setupAttempts = { fails: 0, windowStart: 0, lockedUntil: 0 };
    }
    return false;
  }

  /**
   * One-time owner password bootstrap. Constant-time token compare.
   * Never returns token/password/hash. Disables setup after success.
   */
  async completeOwnerSetup(token: string, password: string, confirm: string): Promise<AdminResult<{ done: true }>> {
    const GENERIC = "Setup failed.";
    const deny = (status = 401) => {
      this.noteSetupFail();
      this.record({ adminEmail: this.ownerEmail() || "(setup)", role: "owner", action: "owner_setup", target: "admin", reason: "", result: "denied" });
      return fail(status, "setup_failed", GENERIC);
    };

    if (!this.setupAvailable()) {
      // Still throttle blind probing once setup is closed.
      this.noteSetupFail();
      return fail(404, "setup_unavailable", "Not found.");
    }
    if (this.setupLocked()) {
      this.record({ adminEmail: this.ownerEmail() || "(setup)", role: "owner", action: "owner_setup", target: "admin", reason: "", result: "denied", detail: "rate_limited" });
      return fail(429, "setup_failed", GENERIC);
    }

    const expected = this.setupToken || "";
    if (!timingSafeEqualString(token, expected)) return deny();

    if (password !== confirm || !isPasswordStrongEnough(password)) return deny(400);

    const email = this.ownerEmail();
    if (!email) return deny(503);
    const acct = this.admins.get(email);
    if (!acct || acct.passwordHash) return deny(404);

    const passwordHash = await hashPassword(password);
    acct.passwordHash = passwordHash;
    this.setupUsed = true;
    this.setupToken = null; // drop from memory after use
    // Invalidate any existing owner sessions
    this.sessions = this.sessions.filter((s) => s.email.toLowerCase() !== email.toLowerCase());
    this.record({ adminEmail: email, role: "owner", action: "owner_setup", target: "admin", reason: "one-time office setup", result: "ok" });
    return { ok: true, value: { done: true } };
  }

  setTotpSecret(email: string, secret: string, enrolled = true) {
    const acct = this.admins.get(email.toLowerCase());
    if (!acct) throw new Error("admin account missing");
    acct.totpSecret = secret;
    acct.totpEnrolled = enrolled;
  }

  getTotpSecret(email: string): string | undefined {
    return this.admins.get(email.toLowerCase())?.totpSecret;
  }

  /** Serializable staff records for Supabase/file snapshot (includes secrets/hashes). */
  exportStaff(): AdminAccount[] {
    return [...this.admins.values()].map((a) => ({ ...a }));
  }

  importStaff(rows: AdminAccount[] | null | undefined) {
    if (!rows?.length) return;
    for (const raw of rows) {
      if (!raw?.email || !raw.role) continue;
      const email = String(raw.email).toLowerCase();
      const existing = this.admins.get(email);
      // Bootstrap env accounts keep role/label from env; restore secrets/hashes/flags from snapshot.
      if (existing?.bootstrap) {
        existing.passwordHash = raw.passwordHash || existing.passwordHash;
        existing.totpSecret = raw.totpSecret || existing.totpSecret;
        existing.enabled = raw.enabled !== false;
        existing.totpEnrolled = Boolean(raw.totpEnrolled || existing.totpEnrolled);
        if (raw.email2fa === true || raw.email2fa === false) existing.email2fa = raw.email2fa;
        if (raw.createdAt) existing.createdAt = raw.createdAt;
        continue;
      }
      this.admins.set(email, {
        email,
        role: raw.role,
        label: raw.label || email,
        passwordHash: raw.passwordHash,
        totpSecret: raw.totpSecret,
        enabled: raw.enabled !== false,
        totpEnrolled: Boolean(raw.totpEnrolled),
        email2fa: raw.email2fa === true ? true : raw.email2fa === false ? false : undefined,
        createdAt: raw.createdAt || new Date(this.now()).toISOString(),
        bootstrap: false,
      });
    }
    this.applyEmail2faMigration();
  }

  /**
   * Old snapshots have no email2fa flag. The existing owner
   * info@kingdomvision.com gets email codes on; an explicit false is kept.
   */
  applyEmail2faMigration() {
    for (const acct of this.admins.values()) {
      if (acct.email2fa === true || acct.email2fa === false) continue;
      acct.email2fa = acct.email === OWNER_EMAIL_2FA && acct.role === "owner";
    }
  }

  /** True when this account must enter an email code after the password. */
  email2faOn(acct: AdminAccount): boolean {
    if (acct.email2fa === false) return false;
    if (acct.email2fa === true) return true;
    return acct.email === OWNER_EMAIL_2FA && acct.role === "owner";
  }

  private toPublic(a: AdminAccount): StaffPublic {
    return {
      email: a.email,
      role: a.role,
      label: a.label,
      enabled: a.enabled,
      totpEnrolled: a.totpEnrolled,
      email2fa: this.email2faOn(a),
      createdAt: a.createdAt,
      bootstrap: Boolean(a.bootstrap),
    };
  }

  private loginKey(email: string) { return email.trim().toLowerCase() || "(blank)"; }

  private isLoginLocked(email: string): boolean {
    const key = this.loginKey(email);
    const t = this.now();
    const st = this.loginAttempts.get(key);
    if (!st) return false;
    if (st.lockedUntil > t) return true;
    if (t - st.windowStart > LOGIN_WINDOW_MS) {
      this.loginAttempts.delete(key);
      return false;
    }
    return false;
  }

  private noteLoginFail(email: string) {
    const key = this.loginKey(email);
    const t = this.now();
    let st = this.loginAttempts.get(key);
    if (!st || t - st.windowStart > LOGIN_WINDOW_MS) {
      st = { fails: 0, windowStart: t, lockedUntil: 0 };
    }
    st.fails += 1;
    if (st.fails >= LOGIN_FAIL_LIMIT) {
      st.lockedUntil = t + LOGIN_LOCK_MS;
      st.fails = 0;
      st.windowStart = t;
    }
    this.loginAttempts.set(key, st);
  }

  private clearLoginFails(email: string) {
    this.loginAttempts.delete(this.loginKey(email));
  }

  private owners(): AdminAccount[] {
    return [...this.admins.values()].filter((a) => a.role === "owner" && a.enabled);
  }
  async addPool(p: Omit<PoolKey, "fingerprint">, secretMaterialForFingerprint: string) {
    // Only a short fingerprint of the key is ever kept in admin memory.
    const fingerprint = (await sha256Hex(secretMaterialForFingerprint)).slice(0, 12);
    this.pools.set(p.id, { ...p, fingerprint });
  }
  poolFor(installId: string): string { return this.installPool.get(installId) ?? [...this.pools.keys()][0] ?? "workers_ai"; }
  setPool(installId: string, poolId: string) { this.installPool.set(installId, poolId); }

  /** Append-only: entries are frozen and there is no delete/update API. */
  private record(e: Omit<AuditEntry, "id" | "at">) {
    const entry: AuditEntry = Object.freeze({ id: this.audit.length + 1, at: new Date(this.now()).toISOString(), ...e });
    this.audit.push(entry);
    return entry;
  }
  auditLog(): readonly AuditEntry[] { return this.audit.slice().reverse(); }

  async login(email: string, password: string, totp = ""): Promise<AdminResult<LoginValue>> {
    const normalized = email.trim().toLowerCase();
    if (this.isLoginLocked(normalized)) {
      this.record({ adminEmail: normalized || "(blank)", role: "viewer", action: "login", target: "admin", reason: "", result: "denied", detail: "rate_limited" });
      return fail(429, "login_rate_limited", GENERIC_LOGIN_ERROR);
    }

    const deny = () => {
      this.noteLoginFail(normalized);
      this.record({ adminEmail: normalized || "(blank)", role: "viewer", action: "login", target: "admin", reason: "", result: "denied" });
      return fail(401, "admin_login_failed", GENERIC_LOGIN_ERROR);
    };

    if (this.mode !== "mock") {
      if (this.admins.size === 0) return fail(503, "admin_auth_not_configured", "Admin sign-in is not configured.");
    }

    const acct = this.admins.get(normalized);
    if (!acct || !acct.enabled) return deny();

    // Mock keeps email+TOTP for existing tests; password optional unless a hash is set.
    if (this.mode === "mock") {
      if (acct.passwordHash && !(await verifyPassword(password, acct.passwordHash))) return deny();
      if (!(await this.codeOk(acct.email, totp))) return deny();
      return this.issueSession(acct);
    }

    if (!acct.passwordHash || !(await verifyPassword(password, acct.passwordHash))) return deny();

    if (this.email2faOn(acct) || this.ops.state.settings.requireEmail2fa) {
      const given = totp.trim();
      if (given) {
        const emailHit = await this.takeEmailCode(given, "login", acct.email);
        if (emailHit === "ok" || (acct.totpEnrolled && await this.codeOk(acct.email, given))) {
          this.clearEmailCodes(acct.email, "login");
          return this.issueSession(acct);
        }
        return deny();
      }
      const sent = await this.issueEmailCode(acct.email, "login", false);
      if (!sent.ok) {
        this.record({ adminEmail: acct.email, role: acct.role, action: "login", target: "admin", reason: "", result: "denied", detail: "email_send" });
        return fail(sent.status, "admin_login_failed", GENERIC_LOGIN_ERROR);
      }
      this.clearLoginFails(normalized);
      return {
        ok: true,
        value: {
          role: acct.role, email: acct.email, label: acct.label, needsEnrollment: false,
          needsEmailCode: true, challenge: sent.challenge, resendAfterSeconds: sent.resendAfterSeconds,
          totpAvailable: acct.totpEnrolled,
        },
      };
    }

    return this.issueSession(acct);
  }

  async completeEmailLogin(challenge: string, code: string): Promise<AdminResult<LoginValue>> {
    const row = this.findCode(challenge, "login");
    const email = row?.email || "";
    if (email && this.isLoginLocked(email)) {
      return fail(429, "login_rate_limited", GENERIC_LOGIN_ERROR);
    }
    const deny = () => {
      if (email) this.noteLoginFail(email);
      this.record({ adminEmail: email || "(blank)", role: "viewer", action: "login", target: "admin", reason: "", result: "denied" });
      return fail(401, "admin_login_failed", GENERIC_LOGIN_ERROR);
    };
    if (!row) return deny();
    const acct = this.admins.get(row.email);
    if (!acct || !acct.enabled || !this.email2faOn(acct)) return deny();
    const hit = await this.takeEmailCode(code, "login", acct.email, challenge);
    if (hit === "ok" || (acct.totpEnrolled && await this.codeOk(acct.email, code.trim()))) {
      this.clearEmailCodes(acct.email, "login");
      return this.issueSession(acct);
    }
    return deny();
  }

  async resendEmailLogin(challenge: string): Promise<AdminResult<{ sent: true; resendAfterSeconds: number }>> {
    const row = this.findCode(challenge, "login");
    if (!row) return fail(401, "admin_login_failed", GENERIC_LOGIN_ERROR);
    if (this.isLoginLocked(row.email)) return fail(429, "login_rate_limited", GENERIC_LOGIN_ERROR);
    const sent = await this.issueEmailCode(row.email, "login", true);
    if (!sent.ok) return fail(sent.status, "admin_login_failed", GENERIC_LOGIN_ERROR);
    return { ok: true, value: { sent: true, resendAfterSeconds: sent.resendAfterSeconds } };
  }

  /** True when a stored code row contains this plaintext (tests; always false in production storage). */
  storesPlaintextCode(code: string): boolean {
    for (const row of this.emailCodes.values()) {
      const bag = row as EmailCodeRow & { code?: string };
      if (bag.code === code || bag.codeHash === code || bag.salt === code) return true;
    }
    return false;
  }

  private async issueSession(acct: AdminAccount): Promise<AdminResult<LoginValue>> {
    const bytes = crypto.getRandomValues(new Uint8Array(32));
    const token = "adm_" + [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
    const t = this.now();
    this.sessions.push({
      id: randomToken(8),
      tokenHash: await sha256Hex(token), email: acct.email, role: acct.role,
      expiresAt: t + SESSION_TTL_MS, recentAuthAt: t, needsEnrollment: false,
    });
    this.clearLoginFails(acct.email);
    this.record({ adminEmail: acct.email, role: acct.role, action: "login", target: "admin", reason: "", result: "ok" });
    return { ok: true, value: { token, role: acct.role, email: acct.email, label: acct.label, needsEnrollment: false } };
  }

  private randomDigits(): string {
    const limit = 0x1_0000_0000 - (0x1_0000_0000 % 1_000_000);
    const buf = new Uint32Array(1);
    let n = 0;
    do { crypto.getRandomValues(buf); n = buf[0]; } while (n >= limit);
    return String(n % 1_000_000).padStart(6, "0");
  }

  private codeKey(email: string, purpose: EmailCodeRow["purpose"]) {
    return `${purpose}:${email.toLowerCase()}`;
  }

  private findCode(challenge: string, purpose: EmailCodeRow["purpose"]): EmailCodeRow | undefined {
    for (const row of this.emailCodes.values()) {
      if (row.purpose === purpose && row.challengeId === challenge) return row;
    }
    return undefined;
  }

  private clearEmailCodes(email: string, purpose: EmailCodeRow["purpose"]) {
    this.emailCodes.delete(this.codeKey(email, purpose));
  }

  private async issueEmailCode(email: string, purpose: EmailCodeRow["purpose"], resend: boolean): Promise<
    { ok: true; challenge: string; resendAfterSeconds: number } | { ok: false; status: number }
  > {
    const key = this.codeKey(email, purpose);
    const t = this.now();
    const existing = this.emailCodes.get(key);
    const live = existing && existing.expiresAt > t ? existing : undefined;
    if (live && t - live.lastSentAt < EMAIL_RESEND_MS) {
      if (!resend) return { ok: true, challenge: live.challengeId, resendAfterSeconds: Math.max(1, Math.ceil((EMAIL_RESEND_MS - (t - live.lastSentAt)) / 1000)) };
      return { ok: false, status: 429 };
    }
    if (live && live.sends >= EMAIL_MAX_SENDS) return { ok: false, status: 429 };
    if (!this.officeMailer) return { ok: false, status: 503 };
    const code = this.randomDigits();
    let delivered = false;
    const to = codeRecipient(email, this.officeCodeRedirect);
    try { delivered = await this.officeMailer(to, code); } catch { delivered = false; }
    if (!delivered) return { ok: false, status: 503 };
    const salt = randomToken(8);
    const codeHash = await sha256Hex(`${salt}:${code}`);
    this.emailCodes.set(key, {
      email: email.toLowerCase(),
      purpose,
      salt,
      codeHash,
      challengeId: live?.challengeId || randomToken(16),
      expiresAt: t + EMAIL_CODE_TTL_MS,
      attempts: live?.attempts ?? 0,
      lastSentAt: t,
      sends: (live?.sends ?? 0) + 1,
    });
    const row = this.emailCodes.get(key)!;
    return { ok: true, challenge: row.challengeId, resendAfterSeconds: Math.ceil(EMAIL_RESEND_MS / 1000) };
  }

  /** ok, or fail. Deletes the row after max attempts or expiry. Does not accept TOTP. */
  private async takeEmailCode(code: string, purpose: EmailCodeRow["purpose"], email: string, challenge?: string): Promise<"ok" | "fail"> {
    const key = this.codeKey(email, purpose);
    const row = this.emailCodes.get(key);
    const t = this.now();
    if (!row || row.purpose !== purpose || (challenge && row.challengeId !== challenge)) return "fail";
    if (row.expiresAt <= t || row.attempts >= EMAIL_CODE_MAX_ATTEMPTS) {
      this.emailCodes.delete(key);
      return "fail";
    }
    const hash = await sha256Hex(`${row.salt}:${code.trim()}`);
    if (timingSafeEqualString(hash, row.codeHash)) {
      this.emailCodes.delete(key);
      return "ok";
    }
    row.attempts += 1;
    if (row.attempts >= EMAIL_CODE_MAX_ATTEMPTS) this.emailCodes.delete(key);
    return "fail";
  }

  private ensureEnrollment(acct: AdminAccount): StaffEnrollment {
    const key = acct.email.toLowerCase();
    const existing = this.pendingEnrollment.get(key);
    if (existing) return existing;
    if (!acct.totpSecret) acct.totpSecret = generateTotpSecret();
    let svg: string | undefined;
    try { svg = qrSvg(otpauthUri(acct.email, acct.totpSecret)); } catch { svg = undefined; }
    const enrollment: StaffEnrollment = {
      email: acct.email,
      secret: acct.totpSecret,
      otpauthUrl: otpauthUri(acct.email, acct.totpSecret),
      qrSvg: svg,
    };
    this.pendingEnrollment.set(key, enrollment);
    return enrollment;
  }

  private async session(token: string | null | undefined): Promise<AdminSession | null> {
    if (!token) return null;
    const h = await sha256Hex(token);
    const s = this.sessions.find((x) => x.tokenHash === h);
    if (!s || s.expiresAt < this.now()) return null;
    return s;
  }

  async reauth(token: string | null | undefined, totp: string, password = ""): Promise<AdminResult<{ recentAuthUntil: string }>> {
    const s = await this.session(token);
    if (!s) return fail(401, "admin_unauthenticated", "Please sign in.");
    const acct = this.admins.get(s.email.toLowerCase());
    if (!acct || !acct.enabled) return fail(401, "admin_unauthenticated", "Please sign in.");
    const code = totp.trim();
    const emailOk = code ? await this.takeEmailCode(code, "reauth", acct.email) === "ok" : false;
    const totpOk = code ? await this.codeOk(acct.email, code) : false;
    const passwordOk = !this.email2faOn(acct) && !acct.totpEnrolled && password
      ? await verifyPassword(password, acct.passwordHash || "")
      : false;
    if (!emailOk && !totpOk && !passwordOk) {
      this.record({ adminEmail: s.email, role: s.role, action: "reauth", target: "admin", reason: "", result: "denied" });
      return fail(401, "reauth_failed", "That code did not work.");
    }
    s.recentAuthAt = this.now();
    this.record({ adminEmail: s.email, role: s.role, action: "reauth", target: "admin", reason: "", result: "ok" });
    return { ok: true, value: { recentAuthUntil: new Date(s.recentAuthAt + RECENT_AUTH_MS).toISOString() } };
  }

  async sendReauthCode(token: string | null | undefined): Promise<AdminResult<{ sent: true; resendAfterSeconds: number }>> {
    const s = await this.session(token);
    if (!s) return fail(401, "admin_unauthenticated", "Please sign in.");
    const sent = await this.issueEmailCode(s.email, "reauth", true);
    if (!sent.ok) return fail(sent.status === 429 ? 429 : 400, "reauth_failed", "Could not email a code. Try again.");
    return { ok: true, value: { sent: true, resendAfterSeconds: sent.resendAfterSeconds } };
  }

  /** Central gate: role permission, reason and recent MFA for sensitive actions; audits denials. */
  async authorize(token: string | null | undefined, action: AdminAction, target = "", reason = ""): Promise<AdminResult<AdminSession>> {
    const s = await this.session(token);
    if (!s) return fail(401, "admin_unauthenticated", "Please sign in.");
    const acct = this.admins.get(s.email.toLowerCase());
    const deny = (status: number, code: string, message: string) => {
      if (action !== "view") this.record({ adminEmail: s.email, role: s.role, action, target, reason, result: "denied", detail: code });
      return fail(status, code, message);
    };
    if (!acct || !acct.enabled) return deny(401, "admin_unauthenticated", "Please sign in.");
    const needsEnrollment = Boolean(s.needsEnrollment);
    if (needsEnrollment && action !== "view") {
      return deny(403, "enrollment_required", "Finish authenticator setup before using Office.");
    }
    // During enrollment, only /me and enrollment endpoints are useful; still allow view for /me.
    if (!can(s.role, action)) return deny(403, "forbidden_role", `Your role (${s.role}) cannot do this.`);
    if (SENSITIVE.has(action)) {
      if (reason.trim().length < 5) return deny(400, "reason_required", "Please give a reason (at least 5 characters).");
      if (this.now() - s.recentAuthAt > RECENT_AUTH_MS) return deny(401, "reauth_required", "Confirm with a sign-in code first.");
    }
    return { ok: true, value: s };
  }

  /** Resolve a bearer token. A missing or expired session is not an error. */
  async peek(token: string | null | undefined): Promise<AdminSession | null> {
    const s = await this.session(token);
    if (!s) return null;
    const acct = this.admins.get(s.email.toLowerCase());
    if (!acct || !acct.enabled) return null;
    return s;
  }

  me(s: AdminSession) {
    const acct = this.admins.get(s.email.toLowerCase());
    const needsEnrollment = Boolean(s.needsEnrollment);
    return {
      email: s.email,
      role: s.role,
      label: acct?.label ?? s.email,
      recentAuthUntil: new Date(s.recentAuthAt + RECENT_AUTH_MS).toISOString(),
      permissions: needsEnrollment ? [] : [...PERMISSIONS[s.role]],
      needsEnrollment,
      email2fa: acct ? this.email2faOn(acct) : false,
      totpEnrolled: Boolean(acct?.totpEnrolled),
      production: this.mode !== "mock",
    };
  }

  private usage(events: UsageEvent[]) {
    const t = this.now(); const d = startOfDay(t); const m = startOfMonth(t);
    let daily = 0, monthly = 0, lifetime = 0, errors = 0;
    for (const e of events) {
      if (e.statusClass === "ok" && e.kind === "transcribe") {
        lifetime += e.audioMinutes;
        if (e.createdAt >= m) monthly += e.audioMinutes;
        if (e.createdAt >= d) daily += e.audioMinutes;
      } else if (e.statusClass !== "ok" && e.createdAt >= d) errors++;
    }
    return { dailyMinutes: round(daily), monthlyMinutes: round(monthly), lifetimeMinutes: round(lifetime), errorsToday: errors };
  }

  private email(p: Profile, s: AdminSession) { return can(s.role, "view_full_email") ? p.email : maskEmail(p.email); }

  listUsers(s: AdminSession, q = "") {
    const needle = q.trim().toLowerCase();
    return [...this.backend.profiles.values()]
      .filter((p) => !needle || p.email.includes(needle) || p.userId.startsWith(needle))
      .map((p) => {
        const installs = [...this.backend.installs.values()].filter((i) => i.userId === p.userId);
        return {
          userId: p.userId,
          email: this.email(p, s),
          plan: p.plan,
          status: p.status,
          installs: installs.length,
          activeInstalls: installs.filter((i) => i.status === "active").length,
          ...this.usage(this.backend.events.filter((e) => e.userId === p.userId)),
          createdAt: p.createdAt,
        };
      });
  }

  private token(installId: string) {
    const t = this.backend.tokens.filter((x) => x.installId === installId).sort((a, b) => b.issuedAt - a.issuedAt)[0];
    if (!t) return null;
    // Safe identifiers only: public id + last 4. Never hashes or secrets.
    return { tokenPublicId: t.tokenPublicId, last4: t.accessLast4, issuedAt: new Date(t.issuedAt).toISOString(), revoked: Boolean(t.revokedAt), revokeReason: t.revokeReason ?? null };
  }

  private safeInstall(i: Install) {
    const pool = this.pools.get(this.poolFor(i.id));
    const ev = this.backend.events.filter((e) => e.installId === i.id);
    const last = ev.sort((a, b) => b.createdAt - a.createdAt)[0];
    return {
      installId: i.id,
      deviceName: i.deviceName,
      appVersion: i.appVersion ?? "unknown",
      status: i.status,
      reconnectRequired: i.reconnectRequired,
      lastSeenAt: i.lastSeenAt ?? null,
      deviceKeyFingerprint: i.publicKeyFingerprint.slice(0, 12),
      token: this.token(i.id),
      pool: pool ? { id: pool.id, label: pool.label, provider: pool.provider, fingerprint: pool.fingerprint } : null,
      lastProvider: last?.provider ?? null,
      lastStatus: last?.statusClass ?? null,
      ...this.usage(ev),
    };
  }

  userDetail(s: AdminSession, userId: string) {
    const p = this.backend.profiles.get(userId);
    if (!p) return null;
    const events = this.backend.events.filter((e) => e.userId === userId);
    const byProvider: Record<string, number> = {};
    const errors: Record<string, number> = {};
    for (const e of events) {
      if (e.statusClass === "ok") byProvider[e.provider] = round((byProvider[e.provider] ?? 0) + e.audioMinutes);
      else errors[e.statusClass] = (errors[e.statusClass] ?? 0) + 1;
    }
    const installs = [...this.backend.installs.values()].filter((i) => i.userId === userId);
    const anyInstall = installs.find((i) => i.status === "active") ?? installs[0];
    const quota = anyInstall
      ? evaluateCaps({ profile: p, install: anyInstall, flags: this.backend.flags, events: this.backend.events, requestedAudioMinutes: 0.01 })
      : null;
    return {
      userId: p.userId,
      email: this.email(p, s),
      plan: p.plan,
      status: p.status,
      createdAt: p.createdAt,
      usage: this.usage(events),
      minutesByProvider: byProvider,
      errorsByClass: errors,
      quota: {
        dailyLimit: p.fairUseDailyMinutes,
        monthlyLimit: p.fairUseMonthlyMinutes,
        remainingDaily: quota ? round(quota.remainingDailyMinutes) : null,
        remainingMonthly: quota ? round(quota.remainingMonthlyMinutes) : null,
        blockedBy: quota && !quota.allowed ? quota.code ?? null : null,
      },
      devices: installs.map((i) => this.safeInstall(i)),
      notices: this.notices.filter((n) => n.userId === userId),
      recentAudit: this.audit.filter((a) => a.target.includes(userId) || installs.some((i) => a.target.includes(i.id))).slice(-10).reverse(),
    };
  }

  dashboard() {
    const t = this.now();
    const profiles = [...this.backend.profiles.values()];
    const u = this.usage(this.backend.events);
    const budget = this.backend.flags.globalDailyAudioMinutes;
    return {
      exampleData: this.mode === "mock",
      users: profiles.length,
      freeUsers: profiles.filter((p) => p.plan === "free").length,
      paidUsers: profiles.filter((p) => p.plan === "paid").length,
      activeInstalls: [...this.backend.installs.values()].filter((i) => i.status === "active").length,
      todayMinutes: u.dailyMinutes,
      monthMinutes: u.monthlyMinutes,
      lifetimeMinutes: u.lifetimeMinutes,
      errorsToday: u.errorsToday,
      dailyBudgetMinutes: budget,
      budgetUsedPct: budget > 0 ? Math.min(100, Math.round((u.dailyMinutes / budget) * 100)) : 0,
      killSwitch: this.backend.flags.killSwitch,
      paidUsageEnabled: this.backend.flags.paidUsageEnabled,
      openRouterEnabled: this.backend.flags.openRouterEnabled,
      pools: [...this.pools.values()].map((p) => ({ ...p, full: this.poolFullIds.has(p.id), installs: [...this.backend.installs.keys()].filter((id) => this.poolFor(id) === p.id).length })),
      healthAlert: this.ops.health().alert,
      generatedAt: new Date(t).toISOString(),
    };
  }

  // ---- mutating actions (caller must pass an authorized session) ----
  private install(id: string) { return this.backend.installs.get(id); }

  wrote(s: AdminSession, action: string, target: string, reason: string, detail?: string) {
    this.record({ adminEmail: s.email, role: s.role, action, target, reason, result: "ok", detail });
  }

  revoke(s: AdminSession, installId: string, reason: string): AdminResult<{ installId: string; status: string }> {
    const i = this.install(installId);
    if (!i) return fail(404, "install_missing", "That device was not found.");
    i.status = "revoked";
    // Revoked = full sign-in required (not a silent reconnect).
    i.reconnectRequired = false;
    const t = this.now();
    for (const tok of this.backend.tokens) if (tok.installId === installId && !tok.revokedAt) { tok.revokedAt = t; tok.revokeReason = "admin_revoke"; }
    this.record({ adminEmail: s.email, role: s.role, action: "revoke", target: `install:${installId}`, reason, result: "ok" });
    return { ok: true, value: { installId, status: i.status } };
  }

  /** Reissue: old tokens die now; the desktop re-registers silently with its device key on next 401 "reissued". */
  reissue(s: AdminSession, installId: string, reason: string): AdminResult<{ installId: string; status: string }> {
    const i = this.install(installId);
    if (!i) return fail(404, "install_missing", "That device was not found.");
    if (i.status === "revoked") return fail(409, "install_revoked", "Revoked devices must sign in again; reissue is not possible.");
    this.backend.adminReissue(installId, "admin_reissue");
    this.record({ adminEmail: s.email, role: s.role, action: "reissue", target: `install:${installId}`, reason, result: "ok" });
    return { ok: true, value: { installId, status: i.status } };
  }

  movePool(s: AdminSession, installId: string, poolId: string, reason: string): AdminResult<{ installId: string; poolId: string }> {
    const i = this.install(installId);
    if (!i) return fail(404, "install_missing", "That device was not found.");
    const p = this.pools.get(poolId);
    if (!p || p.status !== "active") return fail(400, "pool_invalid", "Pick an active pool key.");
    const from = this.poolFor(installId);
    this.setPool(installId, poolId);
    // Moving pools forces a silent reconnect so the device picks up the new route.
    if (i.status !== "revoked") this.backend.adminReissue(installId, "admin_move_pool");
    this.record({ adminEmail: s.email, role: s.role, action: "move_pool", target: `install:${installId}`, reason, result: "ok", detail: `${from} -> ${poolId}` });
    return { ok: true, value: { installId, poolId } };
  }

  setPlan(s: AdminSession, userId: string, plan: Plan, reason: string): AdminResult<{ userId: string; plan: Plan }> {
    const p = this.backend.profiles.get(userId);
    if (!p) return fail(404, "user_missing", "User not found.");
    if (plan !== "free" && plan !== "paid") return fail(400, "plan_invalid", "Plan must be free or paid.");
    const from = p.plan; p.plan = plan;
    this.record({ adminEmail: s.email, role: s.role, action: "set_plan", target: `user:${userId}`, reason, result: "ok", detail: `${from} -> ${plan}` });
    return { ok: true, value: { userId, plan } };
  }

  revokeAll(s: AdminSession, userId: string, reason: string): AdminResult<{ userId: string; devices: number }> {
    const p = this.backend.profiles.get(userId);
    if (!p) return fail(404, "user_missing", "User not found.");
    const ids = [...this.backend.installs.values()].filter((i) => i.userId === userId).map((i) => i.id);
    for (const id of ids) this.revoke(s, id, reason);
    this.wrote(s, "revoke_all", `user:${userId}`, reason, String(ids.length));
    return { ok: true, value: { userId, devices: ids.length } };
  }

  resetUsage(s: AdminSession, userId: string, reason: string): AdminResult<{ userId: string; removed: number }> {
    const p = this.backend.profiles.get(userId);
    if (!p) return fail(404, "user_missing", "User not found.");
    const before = this.backend.events.length;
    this.backend.events = this.backend.events.filter((e) => e.userId !== userId);
    const removed = before - this.backend.events.length;
    this.wrote(s, "reset_usage", `user:${userId}`, reason, String(removed));
    return { ok: true, value: { userId, removed } };
  }

  setDailyLimit(s: AdminSession, userId: string, minutes: number, reason: string): AdminResult<{ userId: string; dailyMinutes: number }> {
    const p = this.backend.profiles.get(userId);
    if (!p) return fail(404, "user_missing", "User not found.");
    if (!(minutes > 0) || minutes > 10000) return fail(400, "limit_invalid", "Enter a daily minute limit.");
    p.fairUseDailyMinutes = minutes;
    this.wrote(s, "set_limit", `user:${userId}`, reason, String(minutes));
    return { ok: true, value: { userId, dailyMinutes: minutes } };
  }

  setSuspended(s: AdminSession, userId: string, suspended: boolean, reason: string): AdminResult<{ userId: string; status: string }> {
    const p = this.backend.profiles.get(userId);
    if (!p) return fail(404, "user_missing", "User not found.");
    p.status = suspended ? "suspended" : "active";
    this.record({ adminEmail: s.email, role: s.role, action: "suspend", target: `user:${userId}`, reason, result: "ok", detail: p.status });
    return { ok: true, value: { userId, status: p.status } };
  }

  setKillSwitch(s: AdminSession, on: boolean, reason: string): AdminResult<{ killSwitch: boolean }> {
    this.backend.setKillSwitch(on);
    this.record({ adminEmail: s.email, role: s.role, action: "kill_switch", target: "global", reason, result: "ok", detail: on ? "on" : "off" });
    return { ok: true, value: { killSwitch: on } };
  }

  addNotice(s: AdminSession, userId: string, message: string): AdminResult<Notice> {
    if (!this.backend.profiles.get(userId)) return fail(404, "user_missing", "User not found.");
    const text = message.trim().slice(0, 280);
    if (!text) return fail(400, "message_required", "Write a short message.");
    const n: Notice = { id: crypto.randomUUID(), userId, message: text, createdAt: new Date(this.now()).toISOString(), createdBy: s.email };
    this.notices.push(n);
    this.record({ adminEmail: s.email, role: s.role, action: "notice", target: `user:${userId}`, reason: "user notice (placeholder)", result: "ok" });
    return { ok: true, value: n };
  }

  // ---- team (owner only; server-enforced) ----
  /** Owner team mutations: permission + reason + fresh MFA (same bar as other sensitive ops). */
  async authorizeTeamMutation(token: string | null | undefined, target = "", reason = ""): Promise<AdminResult<AdminSession>> {
    const base = await this.authorize(token, "manage_team", target, reason);
    if (!base.ok) return base;
    const s = base.value;
    const deny = (status: number, code: string, message: string) => {
      this.record({ adminEmail: s.email, role: s.role, action: "manage_team", target, reason, result: "denied", detail: code });
      return fail(status, code, message);
    };
    if (reason.trim().length < 5) return deny(400, "reason_required", "Please give a reason (at least 5 characters).");
    if (this.now() - s.recentAuthAt > RECENT_AUTH_MS) return deny(401, "reauth_required", "Confirm with a sign-in code first.");
    return base;
  }

  listTeam(_s: AdminSession): StaffPublic[] {
    return [...this.admins.values()].map((a) => this.toPublic(a)).sort((a, b) => a.email.localeCompare(b.email));
  }

  async createStaff(
    s: AdminSession,
    input: { email: string; label: string; role: AdminRole; password: string },
    reason: string,
  ): Promise<AdminResult<{ staff: StaffPublic; enrollment: StaffEnrollment }>> {
    const email = input.email.trim().toLowerCase();
    if (!email.includes("@") || email.length > 200) return fail(400, "email_invalid", "Enter a valid email.");
    if (!input.label.trim()) return fail(400, "name_required", "Enter a display name.");
    if (!["owner", "admin", "support", "viewer"].includes(input.role)) return fail(400, "role_invalid", "Role must be owner, admin, support, or viewer.");
    if (!isPasswordStrongEnough(input.password)) {
      return fail(400, "password_weak", `Password must be at least ${MIN_PASSWORD_LENGTH} characters.`);
    }
    if (this.admins.has(email)) return fail(409, "staff_exists", "That email already has an Office account.");
    const passwordHash = await hashPassword(input.password);
    const totpSecret = generateTotpSecret();
    const acct: AdminAccount = {
      email,
      role: input.role,
      label: input.label.trim().slice(0, 80),
      passwordHash,
      totpSecret,
      enabled: true,
      totpEnrolled: false,
      email2fa: false,
      createdAt: new Date(this.now()).toISOString(),
      bootstrap: false,
    };
    this.admins.set(email, acct);
    const enrollment = this.ensureEnrollment(acct);
    this.record({
      adminEmail: s.email, role: s.role, action: "team_create", target: `staff:${email}`,
      reason, result: "ok", detail: input.role,
    });
    return { ok: true, value: { staff: this.toPublic(acct), enrollment } };
  }

  setStaffRole(s: AdminSession, email: string, role: AdminRole, reason: string): AdminResult<StaffPublic> {
    const key = email.trim().toLowerCase();
    const acct = this.admins.get(key);
    if (!acct) return fail(404, "staff_missing", "Staff account not found.");
    if (!["owner", "admin", "support", "viewer"].includes(role)) return fail(400, "role_invalid", "Role must be owner, admin, support, or viewer.");
    if (acct.role === "owner" && role !== "owner" && this.owners().length <= 1) {
      this.record({ adminEmail: s.email, role: s.role, action: "team_role", target: `staff:${key}`, reason, result: "denied", detail: "last_owner" });
      return fail(400, "last_owner", "Cannot demote the last owner.");
    }
    const from = acct.role;
    acct.role = role;
    // Refresh live sessions for this account.
    for (const sess of this.sessions) if (sess.email === acct.email) sess.role = role;
    this.record({ adminEmail: s.email, role: s.role, action: "team_role", target: `staff:${key}`, reason, result: "ok", detail: `${from} -> ${role}` });
    return { ok: true, value: this.toPublic(acct) };
  }

  setStaffEnabled(s: AdminSession, email: string, enabled: boolean, reason: string): AdminResult<StaffPublic> {
    const key = email.trim().toLowerCase();
    const acct = this.admins.get(key);
    if (!acct) return fail(404, "staff_missing", "Staff account not found.");
    if (key === s.email.toLowerCase() && !enabled) {
      this.record({ adminEmail: s.email, role: s.role, action: "team_enable", target: `staff:${key}`, reason, result: "denied", detail: "self_disable" });
      return fail(400, "self_disable", "You cannot disable your own account.");
    }
    if (acct.role === "owner" && !enabled && this.owners().length <= 1) {
      this.record({ adminEmail: s.email, role: s.role, action: "team_enable", target: `staff:${key}`, reason, result: "denied", detail: "last_owner" });
      return fail(400, "last_owner", "Cannot disable the last owner.");
    }
    acct.enabled = enabled;
    if (!enabled) {
      this.sessions = this.sessions.filter((sess) => sess.email.toLowerCase() !== key);
    }
    this.record({
      adminEmail: s.email, role: s.role, action: "team_enable", target: `staff:${key}`,
      reason, result: "ok", detail: enabled ? "enabled" : "disabled",
    });
    return { ok: true, value: this.toPublic(acct) };
  }

  setStaffEmail2fa(s: AdminSession, email: string, enabled: boolean, reason: string): AdminResult<StaffPublic> {
    const key = email.trim().toLowerCase();
    const acct = this.admins.get(key);
    if (!acct) return fail(404, "staff_missing", "Staff account not found.");
    acct.email2fa = enabled;
    if (!enabled) this.clearEmailCodes(acct.email, "login");
    this.record({
      adminEmail: s.email, role: s.role, action: "team_email_2fa", target: `staff:${key}`,
      reason, result: "ok", detail: enabled ? "on" : "off",
    });
    return { ok: true, value: this.toPublic(acct) };
  }

  async requestOwnEmail2fa(token: string | null | undefined): Promise<AdminResult<{ challenge?: string; resendAfterSeconds?: number; email2fa?: true }>> {
    const s = await this.session(token);
    if (!s) return fail(401, "admin_unauthenticated", "Please sign in.");
    const acct = this.admins.get(s.email.toLowerCase());
    if (!acct || !acct.enabled) return fail(401, "admin_unauthenticated", "Please sign in.");
    if (this.email2faOn(acct)) return { ok: true, value: { email2fa: true } };
    const sent = await this.issueEmailCode(acct.email, "enable", true);
    if (!sent.ok) return fail(sent.status === 429 ? 429 : 400, "email_2fa_failed", "Could not email a code. Try again.");
    return { ok: true, value: { challenge: sent.challenge, resendAfterSeconds: sent.resendAfterSeconds } };
  }

  async confirmOwnEmail2fa(token: string | null | undefined, code: string): Promise<AdminResult<{ email2fa: true }>> {
    const s = await this.session(token);
    if (!s) return fail(401, "admin_unauthenticated", "Please sign in.");
    const acct = this.admins.get(s.email.toLowerCase());
    if (!acct || !acct.enabled) return fail(401, "admin_unauthenticated", "Please sign in.");
    if (this.email2faOn(acct)) return { ok: true, value: { email2fa: true } };
    if (await this.takeEmailCode(code, "enable", acct.email) !== "ok") {
      this.record({ adminEmail: s.email, role: s.role, action: "email_2fa", target: `staff:${acct.email}`, reason: "", result: "denied" });
      return fail(401, "email_2fa_failed", "That code did not work.");
    }
    acct.email2fa = true;
    this.record({ adminEmail: s.email, role: s.role, action: "email_2fa", target: `staff:${acct.email}`, reason: "turned on by the account", result: "ok" });
    return { ok: true, value: { email2fa: true } };
  }

  /** Confirm authenticator enrollment for the signed-in staff member. */
  async confirmEnrollment(token: string | null | undefined, code: string): Promise<AdminResult<{ enrolled: true }>> {
    const s = await this.session(token);
    if (!s) return fail(401, "admin_unauthenticated", "Please sign in.");
    const acct = this.admins.get(s.email.toLowerCase());
    if (!acct) return fail(401, "admin_unauthenticated", "Please sign in.");
    if (acct.totpEnrolled) return { ok: true, value: { enrolled: true } };
    const enrollment = this.ensureEnrollment(acct);
    const res = verifyTotpCode(enrollment.secret, code, this.now(), this.lastTotpStep.get(acct.email.toLowerCase()));
    if (!res.ok) {
      this.record({ adminEmail: s.email, role: s.role, action: "team_enroll", target: `staff:${acct.email}`, reason: "", result: "denied" });
      return fail(401, "enroll_failed", "Authenticator code is wrong.");
    }
    this.lastTotpStep.set(acct.email.toLowerCase(), res.step);
    acct.totpSecret = enrollment.secret;
    acct.totpEnrolled = true;
    s.needsEnrollment = false;
    s.recentAuthAt = this.now();
    this.pendingEnrollment.delete(acct.email.toLowerCase());
    this.record({ adminEmail: s.email, role: s.role, action: "team_enroll", target: `staff:${acct.email}`, reason: "", result: "ok" });
    return { ok: true, value: { enrolled: true } };
  }

  enrollmentFor(token: string | null | undefined): Promise<AdminResult<StaffEnrollment>> {
    return this.session(token).then((s) => {
      if (!s) return fail(401, "admin_unauthenticated", "Please sign in.");
      const acct = this.admins.get(s.email.toLowerCase());
      if (!acct) return fail(401, "admin_unauthenticated", "Please sign in.");
      if (acct.totpEnrolled) return fail(409, "already_enrolled", "Authenticator is already set up.");
      return { ok: true as const, value: this.ensureEnrollment(acct) };
    });
  }

  listSessions() {
    const t = this.now();
    return this.sessions.filter((s) => s.expiresAt > t).map((s) => ({
      id: s.id, email: s.email, role: s.role, expiresAt: new Date(s.expiresAt).toISOString(),
    }));
  }

  revokeSession(id: string): boolean {
    const before = this.sessions.length;
    this.sessions = this.sessions.filter((s) => s.id !== id);
    return this.sessions.length < before;
  }

  exportUser(userId: string) {
    const p = this.backend.profiles.get(userId);
    if (!p) return null;
    const installs = [...this.backend.installs.values()].filter((i) => i.userId === userId).map((i) => ({
      id: i.id, deviceName: i.deviceName, appVersion: i.appVersion, status: i.status, lastSeenAt: i.lastSeenAt, createdAt: i.createdAt,
    }));
    const audioMinutes = this.backend.events.filter((e) => e.userId === userId).reduce((n, e) => n + (e.audioMinutes || 0), 0);
    return {
      userId: p.userId, email: p.email, plan: p.plan, status: p.status, createdAt: p.createdAt,
      installs, audioMinutes,
    };
  }

  devicesView() {
    const devices = [...this.backend.installs.values()].map((i) => {
      const meta = this.ops.state.devices.find((d) => d.installId === i.id);
      return {
        id: i.id,
        version: i.appVersion || "",
        lastSeenAt: i.lastSeenAt || "",
        engine: meta?.engine || "",
        status: i.status,
        blocked: Boolean(meta?.blocked),
      };
    });
    const byVersion: Record<string, number> = {};
    for (const d of devices) if (d.version) byVersion[d.version] = (byVersion[d.version] ?? 0) + 1;
    return { devices, byVersion };
  }

  blockInstall(s: AdminSession, installId: string, blocked: boolean, reason: string): boolean {
    const inst = this.backend.installs.get(installId);
    if (!inst) return false;
    this.ops.setDevice(installId, { blocked });
    if (blocked) inst.status = "revoked";
    else if (inst.status === "revoked") inst.status = "active";
    this.wrote(s, blocked ? "block_device" : "unblock_device", `install:${installId}`, reason);
    return true;
  }

  deleteUserData(userId: string): boolean {
    const p = this.backend.profiles.get(userId);
    if (!p) return false;
    p.status = "deleted";
    p.email = "deleted";
    const installIds = new Set<string>();
    for (const inst of this.backend.installs.values()) {
      if (inst.userId !== userId) continue;
      inst.status = "revoked";
      installIds.add(inst.id);
      this.ops.forgetDevice(inst.id);
    }
    const t = this.now();
    for (const tok of this.backend.tokens) {
      if (installIds.has(tok.installId) && !tok.revokedAt) { tok.revokedAt = t; tok.revokeReason = "user_deleted"; }
    }
    this.backend.events = this.backend.events.filter((e) => e.userId !== userId);
    return true;
  }

  /** Extra flags for /v1/status so the desktop can react without user effort. */
  statusExtras(installId: string | undefined, userId: string | undefined) {
    const poolId = installId ? this.poolFor(installId) : undefined;
    return {
      poolFull: poolId ? this.poolFullIds.has(poolId) : false,
      notices: userId ? this.notices.filter((n) => n.userId === userId).map((n) => ({ id: n.id, message: n.message })) : [],
      statusCheckIntervalSeconds: 900,
    };
  }
}
