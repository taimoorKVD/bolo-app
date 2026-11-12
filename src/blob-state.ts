import type { DictationPhase } from "./types";

/** What the character is doing. `hidden` draws nothing. */
export type CharacterState = "hidden" | "listening" | "thinking" | "done" | "error" | "fix";

export type Quality = "full" | "low";

export type Voice = {
  lv: number;
  bands: [number, number, number, number];
  /** Vowel openness, from the 300–1100 Hz band. */
  vo: number;
  /** Mouth width, from the 1100–3200 Hz band. */
  vw: number;
  /** Lip rounding, when the low band leads the high band. */
  vr: number;
  /** 1 while the mic is quiet enough that the mouth should close. */
  cl: number;
};

export type PillModel = {
  line: string;
  status: string;
  detail: string;
  showLine: boolean;
  showStatus: boolean;
};

const ZERO_BANDS: [number, number, number, number] = [0, 0, 0, 0];

export function characterState(phase: DictationPhase | string): CharacterState {
  switch (phase) {
    case "arming":
    case "recording":
      return "listening";
    case "transcribing":
    case "inserting":
      return "thinking";
    case "done":
    case "typed":
      return "done";
    case "error":
      return "error";
    case "fix":
      return "fix";
    default:
      return "hidden";
  }
}

/** The overlay renders only while a take is on screen. */
export function wantsFrames(state: CharacterState): boolean {
  return state !== "hidden" && state !== "fix";
}

export function clamp(value: number, min = 0, max = 1): number {
  if (!Number.isFinite(value)) return min;
  return Math.max(min, Math.min(max, value));
}

/** Failure copy for the overlay. The engine name is never shown here. */
export function publicProblem(message: string | null | undefined): string {
  const text = (message ?? "").trim();
  if (!text) return "";
  return text
    .replace(/a Groq API key/g, "an API key")
    .replace(/Groq API key/g, "API key")
    .replace(/Groq/g, "the service");
}

/**
 * Mouth shape from the real meter.
 * `bands` is [80–300, 300–1100, 1100–3200, 3200–8000], each 0..1.
 * When the app has only loudness, the mouth still opens, with a neutral vowel.
 */
export function shapeVoice(level: number, bands?: number[] | null): Voice {
  const lv = clamp(level);
  const live = bands && bands.length >= 4 ? bands : null;
  const hasSpectrum = !!live && live.some((band) => clamp(band) > 0.02);
  let shaped: [number, number, number, number];
  if (live && hasSpectrum) {
    shaped = [clamp(live[0] ?? 0), clamp(live[1] ?? 0), clamp(live[2] ?? 0), clamp(live[3] ?? 0)];
  } else if (lv > 0.02) {
    shaped = [lv * 0.45, lv * 0.85, lv * 0.4, lv * 0.12];
  } else {
    shaped = ZERO_BANDS;
  }
  const tot = shaped[0] + shaped[1] + shaped[2] + 0.05;
  return {
    lv,
    bands: shaped,
    vo: clamp((shaped[1] / tot) * 2.2),
    vw: 0.7 + 0.9 * clamp((shaped[2] / tot) * 2),
    vr: clamp(((shaped[0] - shaped[2]) / tot) * 1.6),
    cl: lv < 0.05 ? 1 : 0,
  };
}

export function pillModel(phase: DictationPhase | string, message: string | null | undefined): PillModel {
  const state = characterState(phase);
  const text = publicProblem(message);
  if (state === "listening") {
    return { line: text || "Listening", status: "", detail: "", showLine: true, showStatus: false };
  }
  if (state === "thinking") {
    if (text) return { line: text, status: "", detail: "", showLine: true, showStatus: false };
    const status = phase === "inserting" ? "Inserting…" : "Writing…";
    return { line: "", status, detail: "", showLine: false, showStatus: true };
  }
  if (state === "done") {
    const status = text || (phase === "typed" ? "Typed" : "Inserted");
    return { line: "", status, detail: "", showLine: false, showStatus: true };
  }
  if (state === "error") return errorPill(text);
  return { line: "", status: "", detail: "", showLine: false, showStatus: false };
}

function errorPill(text: string): PillModel {
  if (!text || /didn.?t catch|louder|no speech/i.test(text)) {
    return {
      line: "",
      status: "Didn't catch that",
      detail: "Hold the key and try again",
      showLine: false,
      showStatus: true,
    };
  }
  if (/hold the hotkey/i.test(text)) {
    return {
      line: "",
      status: "Hold the key",
      detail: "Keep it down while you talk",
      showLine: false,
      showStatus: true,
    };
  }
  return { line: "", status: "Not inserted", detail: text, showLine: false, showStatus: true };
}

/** Decorative motion that reduced-motion turns off. The mouth still follows audio. */
export function poseImpulse(state: CharacterState, reduced: boolean): { hop: boolean; shrug: boolean } {
  if (reduced) return { hop: false, shrug: false };
  if (state === "done") return { hop: true, shrug: false };
  if (state === "error") return { hop: false, shrug: true };
  return { hop: false, shrug: false };
}

/**
 * Drop to the cheap path after several slow frames, and only climb back
 * after a long run of fast ones so the resolution does not flicker.
 */
export function nextQuality(
  quality: Quality,
  frameMs: number,
  slow: number,
  fast: number,
): { quality: Quality; slow: number; fast: number } {
  let nextSlow = slow;
  let nextFast = fast;
  if (frameMs > 14) {
    nextSlow += 1;
    nextFast = 0;
  } else if (frameMs < 7) {
    nextFast += 1;
    nextSlow = 0;
  } else {
    nextSlow = 0;
    nextFast = 0;
  }
  let next = quality;
  if (next === "full" && nextSlow >= 8) {
    next = "low";
    nextSlow = 0;
    nextFast = 0;
  } else if (next === "low" && nextFast >= 45) {
    next = "full";
    nextSlow = 0;
    nextFast = 0;
  }
  return { quality: next, slow: nextSlow, fast: nextFast };
}

/** Full quality stays at or below 1.25. The cheap path is 1x. */
export function dprFor(quality: Quality, deviceDpr: number): number {
  const cap = quality === "low" ? 1 : 1.25;
  const dpr = Number.isFinite(deviceDpr) && deviceDpr > 0 ? deviceDpr : 1;
  return Math.max(1, Math.min(dpr, cap));
}

export function prefersReducedMotion(): boolean {
  if (typeof matchMedia !== "function") return false;
  return matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** How long the done pose stays up before the window hides, so the tuck can finish. */
export const DONE_TUCK_AT_S = 0.95;
