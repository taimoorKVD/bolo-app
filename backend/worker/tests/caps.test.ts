import { describe, expect, it } from "vitest";
import { evaluateCaps, flagsFromEnv } from "../src/caps";
import type { Install, Profile, UsageEvent } from "../src/types";

const profile: Profile = {
  userId: "u1",
  email: "a@example.com",
  plan: "free",
  status: "active",
  fairUseDailyMinutes: 20,
  fairUseMonthlyMinutes: 300,
  createdAt: new Date().toISOString(),
};

const install: Install = {
  id: "i1",
  userId: "u1",
  deviceName: "PC",
  publicKey: "pk",
  publicKeyFingerprint: "abcd",
  status: "active",
  reconnectRequired: false,
  createdAt: new Date().toISOString(),
  isNew: false,
};

describe("fair-use caps", () => {
  it("allows dictation under the daily cap", () => {
    const flags = flagsFromEnv({});
    const decision = evaluateCaps({
      profile,
      install,
      flags,
      events: [],
      requestedAudioMinutes: 1,
    });
    expect(decision.allowed).toBe(true);
    expect(decision.remainingDailyMinutes).toBe(20);
  });

  it("blocks when kill switch is on", () => {
    const flags = flagsFromEnv({ KILL_SWITCH: "true" });
    const decision = evaluateCaps({
      profile,
      install,
      flags,
      events: [],
      requestedAudioMinutes: 0.1,
    });
    expect(decision).toMatchObject({ allowed: false, code: "kill_switch" });
  });

  it("blocks when the user daily fair-use is exhausted", () => {
    const flags = flagsFromEnv({ FAIR_USE_DAILY_MINUTES: "20" });
    const now = Date.now();
    const events: UsageEvent[] = [
      {
        userId: "u1",
        installId: "i1",
        provider: "workers_ai",
        kind: "transcribe",
        audioMinutes: 20,
        statusClass: "ok",
        createdAt: now,
      },
    ];
    const decision = evaluateCaps({
      profile,
      install,
      flags,
      events,
      requestedAudioMinutes: 0.2,
      now,
    });
    expect(decision).toMatchObject({ allowed: false, code: "fair_use_daily" });
  });

  it("blocks when the global pool is full", () => {
    const flags = flagsFromEnv({ GLOBAL_DAILY_AUDIO_MINUTES: "5" });
    const now = Date.now();
    const events: UsageEvent[] = [
      {
        userId: "other",
        installId: "other",
        provider: "workers_ai",
        kind: "transcribe",
        audioMinutes: 5,
        statusClass: "ok",
        createdAt: now,
      },
    ];
    const decision = evaluateCaps({
      profile,
      install,
      flags,
      events,
      requestedAudioMinutes: 0.1,
      now,
    });
    expect(decision).toMatchObject({ allowed: false, code: "pool_full" });
  });
});
