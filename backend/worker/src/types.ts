export type Plan = "free" | "paid";
export type ProfileStatus = "active" | "suspended" | "deleted";
export type InstallStatus = "active" | "revoked" | "reissue_pending" | "suspended";
export type StatusClass = "ok" | "quota" | "rate" | "revoked" | "outage" | "rejected" | "pool_full" | "kill_switch";

export interface EnvConfig {
  OPENROUTER_ENABLED: string;
  PAID_USAGE_ENABLED: string;
  KILL_SWITCH: string;
  FAIR_USE_DAILY_MINUTES: string;
  FAIR_USE_MONTHLY_MINUTES: string;
  FAIR_USE_NEW_INSTALL_DAILY_MINUTES: string;
  MAX_INSTALLS_PER_USER: string;
  ACCESS_TOKEN_TTL_SECONDS: string;
  GLOBAL_DAILY_AUDIO_MINUTES: string;
  WHISPER_MODEL: string;
  CLEANUP_MODEL: string;
  SUPABASE_URL?: string;
  SUPABASE_SERVICE_ROLE_KEY?: string;
  SUPABASE_JWT_SECRET?: string;
  ADMIN_BOOTSTRAP_EMAIL?: string;
  /** "supabase" (default, MFA required) or "mock" for local dev only. */
  ADMIN_AUTH_MODE?: string;
  /** "false" only for local mock dev; production always requires device proof. */
  REQUIRE_DEVICE_PROOF?: string;
}

export interface WorkerEnv extends EnvConfig {
  AI?: Ai;
}

export interface Profile {
  userId: string;
  email: string;
  plan: Plan;
  status: ProfileStatus;
  fairUseDailyMinutes: number;
  fairUseMonthlyMinutes: number;
  createdAt: string;
}

export interface Install {
  id: string;
  userId: string;
  deviceName: string;
  appVersion?: string;
  publicKey: string;
  publicKeyFingerprint: string;
  status: InstallStatus;
  reconnectRequired: boolean;
  lastSeenAt?: string;
  createdAt: string;
  isNew: boolean;
}

export interface TokenRecord {
  id: string;
  installId: string;
  tokenPublicId: string;
  accessTokenHash: string;
  accessLast4: string;
  accessExpiresAt: number;
  refreshTokenHash: string;
  refreshFamilyId: string;
  issuedAt: number;
  lastUsedAt?: number;
  revokedAt?: number;
  revokeReason?: string;
}

export interface UsageEvent {
  userId: string;
  installId: string;
  provider: string;
  kind: "transcribe" | "cleanup";
  audioMinutes: number;
  statusClass: StatusClass;
  httpStatus?: number;
  latencyMs?: number;
  createdAt: number;
}

export interface GlobalFlags {
  killSwitch: boolean;
  paidUsageEnabled: boolean;
  openRouterEnabled: boolean;
  globalDailyAudioMinutes: number;
  fairUseDailyMinutes: number;
  fairUseMonthlyMinutes: number;
}

export interface AuthSession {
  userId: string;
  email: string;
  installId: string;
  tokenPublicId: string;
  accessToken: string;
  refreshToken: string;
  expiresIn: number;
}

export interface ApiErrorBody {
  error: string;
  code: string;
  retryAfterMs?: number;
}
