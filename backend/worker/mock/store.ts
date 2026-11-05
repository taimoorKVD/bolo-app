import { fingerprintPublicKey, randomToken, sha256Hex, verifyEd25519 } from "../src/crypto";
import { issueTokenPair, isAccessValid, rotateRefresh } from "../src/tokens";
import { evaluateCaps, flagsFromEnv } from "../src/caps";
import type {
  AuthSession,
  GlobalFlags,
  Install,
  Profile,
  StatusClass,
  TokenRecord,
  UsageEvent,
} from "../src/types";

/** Device signatures are valid for 5 minutes and single-use. */
export const PROOF_WINDOW_MS = 5 * 60 * 1000;

/** Exact bytes the device signs to register or re-register. */
export function registrationMessage(userId: string, publicKey: string, signedAtMs: number): string {
  return `bolo-register-v1\n${userId}\n${publicKey}\n${signedAtMs}`;
}

export interface OtpChallenge {
  email: string;
  code: string;
  expiresAt: number;
  attempts: number;
}

/** In-memory stand-in for Supabase Postgres + Auth while developing locally. */
export class MockBackend {
  profiles = new Map<string, Profile>();
  installs = new Map<string, Install>();
  tokens: TokenRecord[] = [];
  events: UsageEvent[] = [];
  otps = new Map<string, OtpChallenge>();
  flags: GlobalFlags;
  maxInstalls: number;
  accessTtlSeconds: number;
  newInstallDailyMinutes: number;
  /** Codes returned by requestOtp for tests (never log in production). */
  lastOtpByEmail = new Map<string, string>();
  /** Production rejects unsigned / unticketed registrations. Default on. */
  requireDeviceProof: boolean;
  /** Hashes of OTP-bound registration tickets (single use). */
  tickets = new Map<string, { userId: string; expiresAt: number }>();
  /** Signature hashes already used, to stop replays. */
  usedProofs = new Map<string, number>();

  /** What /v1/status advertises; the self-hosted Node service sets Groq. */
  statusInfo = { provider: "workers_ai", models: { whisper: "@cf/openai/whisper-large-v3-turbo", cleanup: "@cf/openai/gpt-oss-20b" } };

  constructor(env: object = {}) {
    this.flags = flagsFromEnv(env as Partial<Record<string, string | undefined>>);
    this.maxInstalls = Number((env as Record<string, unknown>).MAX_INSTALLS_PER_USER ?? 3);
    this.accessTtlSeconds = Number((env as Record<string, unknown>).ACCESS_TOKEN_TTL_SECONDS ?? 900);
    this.newInstallDailyMinutes = Number((env as Record<string, unknown>).FAIR_USE_NEW_INSTALL_DAILY_MINUTES ?? 10);
    this.requireDeviceProof = String((env as Record<string, unknown>).REQUIRE_DEVICE_PROOF ?? "true").toLowerCase() !== "false";
  }

  setKillSwitch(on: boolean) {
    this.flags.killSwitch = on;
  }

  async requestOtp(email: string): Promise<{ ok: true } | { ok: false; code: string }> {
    const normalized = email.trim().toLowerCase();
    if (!normalized.includes("@")) return { ok: false, code: "invalid_email" };
    const code = String(Math.floor(100000 + Math.random() * 900000));
    this.otps.set(normalized, {
      email: normalized,
      code,
      expiresAt: Date.now() + 10 * 60 * 1000,
      attempts: 0,
    });
    this.lastOtpByEmail.set(normalized, code);
    return { ok: true };
  }

  async verifyOtp(email: string, code: string): Promise<{ ok: true; profile: Profile } | { ok: false; code: string }> {
    const normalized = email.trim().toLowerCase();
    const challenge = this.otps.get(normalized);
    if (!challenge) return { ok: false, code: "otp_not_found" };
    challenge.attempts += 1;
    if (challenge.attempts > 5) return { ok: false, code: "otp_locked" };
    if (Date.now() > challenge.expiresAt) return { ok: false, code: "otp_expired" };
    if (challenge.code !== code.trim()) return { ok: false, code: "otp_mismatch" };
    this.otps.delete(normalized);

    let profile = [...this.profiles.values()].find((p) => p.email === normalized);
    if (!profile) {
      profile = {
        userId: crypto.randomUUID(),
        email: normalized,
        plan: "free",
        status: "active",
        fairUseDailyMinutes: this.flags.fairUseDailyMinutes,
        fairUseMonthlyMinutes: this.flags.fairUseMonthlyMinutes,
        createdAt: new Date().toISOString(),
      };
      this.profiles.set(profile.userId, profile);
    }
    return { ok: true, profile };
  }

  /**
   * Register (or silently re-register) a device.
   *
   * Production (`requireDeviceProof`): every call must carry an Ed25519
   * signature by the device key over `registrationMessage(...)`, signed in the
   * last 5 minutes and never replayed. A brand-new install additionally needs
   * the single-use registration ticket from a fresh OTP sign-in. A device key
   * that was ever revoked is refused for good, even with a new OTP.
   * Local mock mode keeps unsigned registration for convenience.
   */
  async registerInstall(input: {
    userId: string;
    deviceName: string;
    appVersion?: string;
    publicKey: string;
    registrationTicket?: string;
    signature?: string;
    signedAtMs?: number;
  }): Promise<{ ok: true; session: AuthSession; install: Install } | { ok: false; code: string }> {
    const profile = this.profiles.get(input.userId);
    if (!profile || profile.status !== "active") return { ok: false, code: "user_inactive" };

    const fingerprint = await fingerprintPublicKey(input.publicKey);
    if ([...this.installs.values()].some((i) => i.publicKeyFingerprint === fingerprint && i.status === "revoked")) {
      return { ok: false, code: "device_revoked" };
    }

    const signed = input.signature !== undefined || input.signedAtMs !== undefined;
    if (this.requireDeviceProof || signed) {
      const proof = await this.checkDeviceProof(input);
      if (proof !== "ok") return { ok: false, code: proof };
    }

    const existing = [...this.installs.values()].find(
      (i) => i.userId === input.userId && i.publicKeyFingerprint === fingerprint && i.status !== "revoked",
    );
    if (existing) {
      existing.reconnectRequired = false;
      existing.status = "active";
      existing.appVersion = input.appVersion;
      existing.deviceName = input.deviceName;
      existing.lastSeenAt = new Date().toISOString();
      const issued = await issueTokenPair(existing.id, this.accessTtlSeconds);
      this.tokens.push(issued.record);
      return {
        ok: true,
        install: existing,
        session: {
          userId: profile.userId,
          email: profile.email,
          installId: existing.id,
          tokenPublicId: issued.record.tokenPublicId,
          accessToken: issued.accessToken,
          refreshToken: issued.refreshToken,
          expiresIn: this.accessTtlSeconds,
        },
      };
    }

    // New install: production requires a fresh OTP-bound ticket.
    if (this.requireDeviceProof || input.registrationTicket !== undefined) {
      const ticket = await this.consumeTicket(input.userId, input.registrationTicket);
      if (ticket !== "ok") return { ok: false, code: ticket };
    }

    const activeCount = [...this.installs.values()].filter(
      (i) => i.userId === input.userId && i.status === "active",
    ).length;
    if (activeCount >= this.maxInstalls) return { ok: false, code: "install_limit" };

    const install: Install = {
      id: crypto.randomUUID(),
      userId: input.userId,
      deviceName: input.deviceName,
      appVersion: input.appVersion,
      publicKey: input.publicKey,
      publicKeyFingerprint: fingerprint,
      status: "active",
      reconnectRequired: false,
      createdAt: new Date().toISOString(),
      lastSeenAt: new Date().toISOString(),
      isNew: true,
    };
    // Tighten daily cap for brand-new installs via profile override only when still default.
    if (install.isNew) {
      profile.fairUseDailyMinutes = Math.min(profile.fairUseDailyMinutes, this.newInstallDailyMinutes);
    }
    this.installs.set(install.id, install);
    const issued = await issueTokenPair(install.id, this.accessTtlSeconds);
    this.tokens.push(issued.record);
    return {
      ok: true,
      install,
      session: {
        userId: profile.userId,
        email: profile.email,
        installId: install.id,
        tokenPublicId: issued.record.tokenPublicId,
        accessToken: issued.accessToken,
        refreshToken: issued.refreshToken,
        expiresIn: this.accessTtlSeconds,
      },
    };
  }

  private async checkDeviceProof(input: {
    userId: string;
    publicKey: string;
    signature?: string;
    signedAtMs?: number;
  }): Promise<"ok" | string> {
    if (!input.signature || typeof input.signedAtMs !== "number" || !Number.isFinite(input.signedAtMs)) {
      return "device_proof_required";
    }
    if (Math.abs(Date.now() - input.signedAtMs) > PROOF_WINDOW_MS) return "device_proof_stale";
    const replayKey = await sha256Hex(input.signature);
    this.pruneProofs();
    if (this.usedProofs.has(replayKey)) return "device_proof_replayed";
    const ok = await verifyEd25519(
      input.publicKey,
      registrationMessage(input.userId, input.publicKey, input.signedAtMs),
      input.signature,
    );
    if (!ok) return "device_proof_invalid";
    this.usedProofs.set(replayKey, Date.now() + PROOF_WINDOW_MS * 2);
    return "ok";
  }

  private pruneProofs() {
    const now = Date.now();
    for (const [key, until] of this.usedProofs) if (until < now) this.usedProofs.delete(key);
  }

  private async consumeTicket(userId: string, ticket?: string): Promise<"ok" | string> {
    if (!ticket) return "registration_ticket_required";
    const hash = await sha256Hex(ticket);
    const entry = this.tickets.get(hash);
    if (!entry) return "registration_ticket_invalid";
    this.tickets.delete(hash); // single use, even on mismatch
    if (entry.userId !== userId) return "registration_ticket_invalid";
    if (Date.now() > entry.expiresAt) return "registration_ticket_expired";
    return "ok";
  }

  /** Issued by verifyOtp; only the hash is stored. */
  async issueRegistrationTicket(userId: string): Promise<string> {
    const ticket = `rt_${randomToken(24)}`;
    this.tickets.set(await sha256Hex(ticket), { userId, expiresAt: Date.now() + 10 * 60 * 1000 });
    return ticket;
  }

  private activeTokenForInstall(installId: string): TokenRecord | undefined {
    return this.tokens
      .filter((t) => t.installId === installId && !t.revokedAt)
      .sort((a, b) => b.issuedAt - a.issuedAt)[0];
  }

  async authenticateAccess(accessToken: string): Promise<
    | { ok: true; profile: Profile; install: Install; token: TokenRecord }
    | { ok: false; code: string }
  > {
    const hash = await sha256Hex(accessToken);
    const token = this.tokens.find((t) => t.accessTokenHash === hash);
    if (!token) return { ok: false, code: "invalid_token" };
    const install = this.installs.get(token.installId);
    if (!install) return { ok: false, code: "install_missing" };
    // Admin reissue / reconnect takes priority over generic expiry/revoke codes.
    if (install.reconnectRequired || install.status === "reissue_pending") {
      return { ok: false, code: "reissued" };
    }
    if (install.status === "revoked") return { ok: false, code: "revoked" };
    if (token.revokedAt) return { ok: false, code: "revoked" };
    if (!isAccessValid(token)) return { ok: false, code: "token_expired" };
    const profile = this.profiles.get(install.userId);
    if (!profile) return { ok: false, code: "user_missing" };
    if (profile.status !== "active") return { ok: false, code: "suspended" };
    token.lastUsedAt = Date.now();
    install.lastSeenAt = new Date().toISOString();
    return { ok: true, profile, install, token };
  }

  async refresh(refreshToken: string): Promise<
    | { ok: true; session: AuthSession }
    | { ok: false; code: string }
  > {
    const hash = await sha256Hex(refreshToken);
    const matches = this.tokens.filter((t) => t.refreshTokenHash === hash);
    if (matches.length === 0) {
      return { ok: false, code: "invalid_refresh" };
    }
    // Prefer an active match; a revoked match with this refresh hash means reuse.
    const previous = matches.find((t) => !t.revokedAt) ?? matches[0];
    const owner = this.installs.get(previous.installId);
    if (previous.revokedAt && owner?.status === "revoked") {
      return { ok: false, code: "revoked" };
    }
    if (previous.revokedAt && owner?.status === "reissue_pending") {
      // Admin reissue/move-pool is not theft: tell the device to re-register.
      return { ok: false, code: "reissued" };
    }
    if (previous.revokedAt) {
      this.revokeFamily(previous.refreshFamilyId, "refresh_reuse");
      const install = this.installs.get(previous.installId);
      if (install) {
        install.status = "revoked";
        install.reconnectRequired = true;
      }
      return { ok: false, code: "refresh_reuse" };
    }

    const result = await rotateRefresh(previous, refreshToken, this.accessTtlSeconds);
    if (!result.ok) {
      if (result.reason === "mismatch" || result.reason === "reuse" || result.reason === "revoked") {
        this.revokeFamily(previous.refreshFamilyId, "refresh_reuse");
        const install = this.installs.get(previous.installId);
        if (install) {
          install.status = "revoked";
          install.reconnectRequired = true;
        }
        return { ok: false, code: "refresh_reuse" };
      }
      return { ok: false, code: result.reason };
    }

    previous.revokedAt = Date.now();
    previous.revokeReason = "rotated";
    this.tokens.push(result.issued.record);
    const install = this.installs.get(previous.installId)!;
    const profile = this.profiles.get(install.userId)!;
    return {
      ok: true,
      session: {
        userId: profile.userId,
        email: profile.email,
        installId: install.id,
        tokenPublicId: result.issued.record.tokenPublicId,
        accessToken: result.issued.accessToken,
        refreshToken: result.issued.refreshToken,
        expiresIn: this.accessTtlSeconds,
      },
    };
  }

  revokeFamily(familyId: string, reason: string) {
    const now = Date.now();
    for (const token of this.tokens) {
      if (token.refreshFamilyId === familyId && !token.revokedAt) {
        token.revokedAt = now;
        token.revokeReason = reason;
      }
    }
  }

  adminReissue(installId: string, reason: string) {
    const install = this.installs.get(installId);
    if (!install) return false;
    install.status = "reissue_pending";
    install.reconnectRequired = true;
    const now = Date.now();
    for (const token of this.tokens) {
      if (token.installId === installId && !token.revokedAt) {
        token.revokedAt = now;
        token.revokeReason = reason;
      }
    }
    return true;
  }

  statusFor(accessToken: string | null) {
    const base = {
      killSwitch: this.flags.killSwitch,
      openRouterEnabled: this.flags.openRouterEnabled,
      paidUsageEnabled: this.flags.paidUsageEnabled,
      provider: this.statusInfo.provider,
      models: { ...this.statusInfo.models },
    };
    if (!accessToken) return { ...base, authenticated: false as const };
    return this.authenticateAccess(accessToken).then((auth) => {
      if (!auth.ok) {
        return { ...base, authenticated: false as const, code: auth.code };
      }
      const decision = evaluateCaps({
        profile: auth.profile,
        install: auth.install,
        flags: this.flags,
        events: this.events,
        requestedAudioMinutes: 0.01,
      });
      return {
        ...base,
        authenticated: true as const,
        email: auth.profile.email,
        plan: auth.profile.plan,
        installId: auth.install.id,
        tokenPublicId: auth.token.tokenPublicId,
        accessLast4: auth.token.accessLast4,
        reconnectRequired: auth.install.reconnectRequired,
        remainingDailyMinutes: decision.remainingDailyMinutes,
        remainingMonthlyMinutes: decision.remainingMonthlyMinutes,
      };
    });
  }

  async dictate(input: {
    accessToken: string;
    kind: "transcribe" | "cleanup";
    audioMinutes: number;
  }): Promise<
    | { ok: true; text: string; provider: string; remainingDailyMinutes: number }
    | { ok: false; code: string; statusClass: StatusClass }
  > {
    const auth = await this.authenticateAccess(input.accessToken);
    if (!auth.ok) {
      const statusClass: StatusClass =
        auth.code === "reissued" || auth.code === "revoked" ? "revoked" : "rejected";
      return { ok: false, code: auth.code, statusClass };
    }
    const decision = evaluateCaps({
      profile: auth.profile,
      install: auth.install,
      flags: this.flags,
      events: this.events,
      requestedAudioMinutes: input.audioMinutes,
    });
    if (!decision.allowed) {
      const statusClass: StatusClass =
        decision.code === "kill_switch"
          ? "kill_switch"
          : decision.code === "pool_full"
            ? "pool_full"
            : "quota";
      this.events.push({
        userId: auth.profile.userId,
        installId: auth.install.id,
        provider: "workers_ai",
        kind: input.kind,
        audioMinutes: 0,
        statusClass,
        createdAt: Date.now(),
      });
      return { ok: false, code: decision.code ?? "quota", statusClass };
    }

    // Mock Workers AI: never store audio/text server-side beyond the response.
    const text =
      input.kind === "transcribe"
        ? "hello from bolo managed whisper"
        : "Hello from Bolo managed cleanup.";
    this.events.push({
      userId: auth.profile.userId,
      installId: auth.install.id,
      provider: "workers_ai",
      kind: input.kind,
      audioMinutes: input.audioMinutes,
      statusClass: "ok",
      httpStatus: 200,
      latencyMs: 12,
      createdAt: Date.now(),
    });
    // Mark install no longer new after first successful dictation day boundary logic is separate.
    auth.install.isNew = false;
    return {
      ok: true,
      text,
      provider: "workers_ai",
      remainingDailyMinutes: decision.remainingDailyMinutes - input.audioMinutes,
    };
  }
}
