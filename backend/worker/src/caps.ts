import type { GlobalFlags, Install, Profile, UsageEvent } from "./types";

export interface CapDecision {
  allowed: boolean;
  code?: "kill_switch" | "pool_full" | "fair_use_daily" | "fair_use_monthly" | "suspended";
  remainingDailyMinutes: number;
  remainingMonthlyMinutes: number;
}

function sumMinutes(events: UsageEvent[], predicate: (e: UsageEvent) => boolean): number {
  return events.filter(predicate).reduce((n, e) => n + e.audioMinutes, 0);
}

export function evaluateCaps(input: {
  profile: Profile;
  install: Install;
  flags: GlobalFlags;
  events: UsageEvent[];
  requestedAudioMinutes: number;
  now?: number;
}): CapDecision {
  const now = input.now ?? Date.now();
  if (input.flags.killSwitch) {
    return { allowed: false, code: "kill_switch", remainingDailyMinutes: 0, remainingMonthlyMinutes: 0 };
  }
  if (input.profile.status !== "active" || input.install.status === "suspended" || input.install.status === "revoked") {
    return { allowed: false, code: "suspended", remainingDailyMinutes: 0, remainingMonthlyMinutes: 0 };
  }

  const startOfUtcDay = Date.UTC(
    new Date(now).getUTCFullYear(),
    new Date(now).getUTCMonth(),
    new Date(now).getUTCDate(),
  );
  const startOfUtcMonth = Date.UTC(new Date(now).getUTCFullYear(), new Date(now).getUTCMonth(), 1);

  const dailyUsed = sumMinutes(
    input.events,
    (e) => e.userId === input.profile.userId && e.createdAt >= startOfUtcDay && e.statusClass === "ok",
  );
  const monthlyUsed = sumMinutes(
    input.events,
    (e) => e.userId === input.profile.userId && e.createdAt >= startOfUtcMonth && e.statusClass === "ok",
  );
  const globalDailyUsed = sumMinutes(
    input.events,
    (e) => e.createdAt >= startOfUtcDay && e.statusClass === "ok",
  );

  const dailyCap = input.install.isNew
    ? Math.min(input.profile.fairUseDailyMinutes, input.flags.fairUseDailyMinutes)
    : input.profile.fairUseDailyMinutes;
  // New installs get a tighter default from flags when profile still has the default.
  const effectiveDaily = input.install.isNew
    ? Math.min(dailyCap, input.flags.fairUseDailyMinutes)
    : dailyCap;

  const remainingDaily = Math.max(0, effectiveDaily - dailyUsed);
  const remainingMonthly = Math.max(0, input.profile.fairUseMonthlyMinutes - monthlyUsed);
  const remainingGlobal = Math.max(0, input.flags.globalDailyAudioMinutes - globalDailyUsed);

  if (remainingGlobal < input.requestedAudioMinutes) {
    return {
      allowed: false,
      code: "pool_full",
      remainingDailyMinutes: remainingDaily,
      remainingMonthlyMinutes: remainingMonthly,
    };
  }
  if (remainingDaily < input.requestedAudioMinutes) {
    return {
      allowed: false,
      code: "fair_use_daily",
      remainingDailyMinutes: remainingDaily,
      remainingMonthlyMinutes: remainingMonthly,
    };
  }
  if (remainingMonthly < input.requestedAudioMinutes) {
    return {
      allowed: false,
      code: "fair_use_monthly",
      remainingDailyMinutes: remainingDaily,
      remainingMonthlyMinutes: remainingMonthly,
    };
  }
  return {
    allowed: true,
    remainingDailyMinutes: remainingDaily,
    remainingMonthlyMinutes: remainingMonthly,
  };
}

export function parseBool(value: string | undefined, fallback = false): boolean {
  if (value == null || value === "") return fallback;
  return ["1", "true", "yes", "on"].includes(value.toLowerCase());
}

export function flagsFromEnv(env: Partial<Record<string, string | undefined>>): GlobalFlags {
  return {
    killSwitch: parseBool(env.KILL_SWITCH),
    paidUsageEnabled: parseBool(env.PAID_USAGE_ENABLED),
    openRouterEnabled: parseBool(env.OPENROUTER_ENABLED),
    globalDailyAudioMinutes: Number(env.GLOBAL_DAILY_AUDIO_MINUTES ?? 180),
    fairUseDailyMinutes: Number(env.FAIR_USE_DAILY_MINUTES ?? 20),
    fairUseMonthlyMinutes: Number(env.FAIR_USE_MONTHLY_MINUTES ?? 300),
  };
}
