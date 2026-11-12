import { api, inTauri, onDictation } from "./api";
import { BlobActor } from "./blob-actor";
import { createBlobGl, drawBlob, releaseBlobGl, resizeCanvas } from "./blob-gl";
import {
  DONE_TUCK_AT_S,
  characterState,
  dprFor,
  nextQuality,
  prefersReducedMotion,
  publicProblem,
  shapeVoice,
  wantsFrames,
  type CharacterState,
  type Quality,
} from "./blob-state";
import { CuePlayer } from "./cues";
import { activeCharacter } from "./characters";
import { buildFixCard, resetFixCard } from "./fix-card";
import { isAnimatedListener, normalizeListener, type ListenerId } from "./listener-style";
import type { DictationEvent, DictationPhase, Settings } from "./types";

const LISTENER_CACHE = "bolo.listener";

/**
 * v4 overlay geometry, in CSS px. The stage is the overlay window.
 * The character stands at (CX, GY) like the mockup's (462, 742) on its 1280x800
 * desktop, with the same scissor box around it, and its pill starts at CX + 0.3U,
 * tucked behind the character, exactly as in the approved video.
 */
export const STAGE_W = 700;
export const STAGE_H = 230;
export const CX = 96;
export const GY = 196;
/** Longest caption before the line scrolls left under a fade, as in the mockup. */
const MAX_TEXT = 470;
/** Each newly heard word fades up 0.28 s after it is stamped. New words in one
 * preview arrive one after another, this far apart, so the sentence builds up. */
const WORD_STAGGER = 0.085;
/** Words kept on the line. Older ones are far off the faded left edge. */
const MAX_WORDS = 60;

type Word = { text: string; t: number; el: HTMLSpanElement };

export function mountBlobOverlay(root: HTMLElement, listener: ListenerId = "blob"): (() => void) | null {
  const character = isAnimatedListener(listener) ? listener : "blob";
  root.replaceChildren();
  const stage = document.createElement("div");
  stage.className = "v4-stage";
  stage.dataset.pose = "hidden";
  stage.dataset.character = character;
  stage.setAttribute("role", "status");
  stage.setAttribute("aria-live", "polite");

  const canvas = document.createElement("canvas");
  canvas.className = "v4-gl";
  canvas.setAttribute("aria-hidden", "true");
  const bundle = createBlobGl(canvas, character);
  if (!bundle) {
    root.replaceChildren();
    return null;
  }

  // Same DOM as the mockup: #pill > #picon (dot, spinner, check, error dot), #cap > #line, #status.
  const pill = document.createElement("div");
  pill.className = "v4-pill";
  const icon = document.createElement("div");
  icon.className = "v4-picon";
  const dot = document.createElement("div");
  dot.className = "v4-pdot";
  const spin = svgSpin();
  const check = svgCheck();
  const errDot = document.createElement("div");
  errDot.className = "v4-perr";
  icon.append(dot, spin, check.svg, errDot);
  const cap = document.createElement("div");
  cap.className = "v4-cap";
  const line = document.createElement("div");
  line.className = "v4-line";
  cap.append(line);
  const status = document.createElement("div");
  status.className = "v4-status";
  pill.append(icon, cap, status);

  const qm = document.createElement("div");
  qm.className = "v4-qm";
  qm.textContent = "?";

  const fixCard = buildFixCard();
  stage.append(pill, canvas, qm, fixCard);
  root.append(stage);

  const actor = new BlobActor(prefersReducedMotion(), character);
  const cues = new CuePlayer();
  const U = actor.rig.unit;
  let prefs: Partial<Settings> = {};
  let state: CharacterState = "hidden";
  let phase: DictationPhase = "idle";
  let message: string | null = null;
  let level = 0;
  let bands: number[] | null = null;
  let words: Word[] = [];
  let doneText = "";
  let statusKey = "";
  let quality: Quality = "full";
  let slow = 0;
  let fast = 0;
  let frames = 0;
  let running = false;
  let raf = 0;
  let last = 0;
  let tucked = false;
  let disposed = false;
  let unlisten: (() => void) | null = null;
  let unlistenSettings: (() => void) | null = null;
  let reduced = prefersReducedMotion();
  const frameSamples: number[] = [];
  const params = new URLSearchParams(location.search);
  const record = !inTauri() && params.has("record");
  const holdPose = !inTauri() && params.has("still");
  const motionQuery = typeof matchMedia === "function" ? matchMedia("(prefers-reduced-motion: reduce)") : null;
  const onMotion = () => {
    reduced = prefersReducedMotion();
    actor.setReduced(reduced);
  };
  motionQuery?.addEventListener?.("change", onMotion);

  const applyPrefs = (settings: Partial<Settings>) => {
    prefs = settings;
    cues.configureSettings(settings, activeCharacter(settings));
    try {
      localStorage.setItem(LISTENER_CACHE, normalizeListener(settings.listener));
    } catch {
      // Private mode can reject the cache. The next settings read still works.
    }
  };
  void api.getSettings().then(applyPrefs).catch((error) => console.warn("overlay could not read settings", error));

  /** Keep the words that did not change (and their reveal times); stamp new ones one after another. */
  const setWords = (text: string) => {
    const next = text.replace(/\s+/g, " ").trim().split(" ").filter(Boolean).slice(-MAX_WORDS);
    const offset = Math.max(0, findOffset(words.map((w) => w.text), next));
    const kept = words.slice(offset);
    let same = 0;
    while (same < kept.length && same < next.length && kept[same]?.text === next[same]) same += 1;
    for (const w of words.slice(0, offset)) w.el.remove();
    for (const w of kept.slice(same)) w.el.remove();
    const out = kept.slice(0, same);
    // New words start now (or just after the last word still fading in) and follow one another.
    let stamp = actor.t - WORD_STAGGER;
    for (const w of out) stamp = Math.max(stamp, w.t);
    for (const text of next.slice(same)) {
      const el = document.createElement("span");
      el.textContent = text;
      el.style.display = "none";
      line.appendChild(el);
      stamp = Math.max(actor.t, stamp + WORD_STAGGER);
      out.push({ text, t: stamp, el });
    }
    words = out;
  };
  const clearWords = () => {
    for (const w of words) w.el.remove();
    words = [];
  };

  /** The mockup's updateDOM(), driven by the app's state instead of its demo. */
  const paintPill = (dt: number) => {
    const t = actor.t;
    const ts = t - actor.since;
    const s = state;
    let vcount = 0;
    for (const [i, w] of words.entries()) {
      const p = (t - w.t) / 0.28;
      if (p < 0) {
        w.el.style.display = "none";
        continue;
      }
      vcount += 1;
      w.el.style.display = "inline-block";
      const e = reduced ? 1 : ease(p);
      let op = e;
      if (s === "thinking" && !reduced) {
        const wp = ((ts * 9) % (words.length + 6)) - 3;
        op *= 0.45 + 0.55 * Math.exp(-Math.pow(i - wp, 2) / 3);
      }
      w.el.style.opacity = op.toFixed(3);
      w.el.style.transform = `translateY(${((1 - e) * 5).toFixed(2)}px)`;
      w.el.style.filter = p < 1 && !reduced ? `blur(${((1 - e) * 3).toFixed(2)}px)` : "none";
    }
    const hasWords = vcount > 0 || words.length > 0;
    const showWords = (s === "listening" || s === "thinking") && hasWords;
    const statusModel = pillStatus(phase, s, message, doneText, hasWords);
    const showStat = !!statusModel && ((s === "done" && ts > 0.25) || (s === "error" && ts > 0.12) || ((s === "listening" || s === "thinking") && !hasWords));
    const key = statusModel ? `${statusModel.main}|${statusModel.small}|${statusModel.tone}` : "";
    if (key !== statusKey) {
      statusKey = key;
      status.replaceChildren();
      if (statusModel) {
        const main = document.createElement("span");
        main.textContent = statusModel.main;
        if (statusModel.tone === "error") main.className = "v4-status-err";
        if (statusModel.tone === "dim") main.className = "v4-status-dim";
        status.append(main);
        if (statusModel.small) {
          const small = document.createElement("small");
          small.textContent = statusModel.small;
          status.append(small);
        }
      }
    }
    const lw = line.offsetWidth;
    const sw = status.offsetWidth;
    const padL = U * (character === "buddy" ? 1.32 : 0.8) + 14;
    const iconW = 26;
    const padR = 18;
    const inner = showWords ? Math.min(Math.max(lw, 90), MAX_TEXT) : showStat ? Math.min(sw, MAX_TEXT) : 90;
    actor.pillW.t = padL + iconW + inner + padR;
    actor.pillW.step(dt, reduced);
    const pv = clamp(actor.pill.x, 0, 1.05);
    const left = CX + U * 0.3;
    const pw = Math.max(0, (actor.pillW.x - U * 0.6) * pv + U * 0.6);
    const hidden = s === "hidden" || s === "fix";
    pill.style.cssText = `left:${left.toFixed(1)}px;top:${(GY - U * 0.95 - 22).toFixed(1)}px;width:${pw.toFixed(1)}px;opacity:${hidden ? 0 : sm(pv * 1.6).toFixed(3)};transform:scale(${(0.92 + 0.08 * pv).toFixed(3)});transform-origin:0 50%`;
    icon.style.left = `${padL}px`;
    cap.style.left = `${padL + iconW}px`;
    status.style.left = `${padL + iconW}px`;
    cap.style.width = `${inner.toFixed(1)}px`;
    cap.style.opacity = showWords ? "1" : "0";
    const ov = lw > inner;
    const mask = ov ? "linear-gradient(90deg,transparent 0,#000 36px)" : "none";
    cap.style.maskImage = mask;
    cap.style.setProperty("-webkit-mask-image", mask);
    line.style.transform = `translateX(${Math.min(0, inner - lw).toFixed(2)}px)`;
    status.style.opacity = showStat ? String(s === "done" || s === "error" ? sm((ts - 0.25) / 0.25) : 1) : "0";
    dot.style.opacity = s === "listening" ? "1" : "0";
    dot.style.transform = `scale(${(0.8 + actor.lv * 0.55).toFixed(3)})`;
    spin.style.opacity = s === "thinking" ? "1" : "0";
    spin.style.transform = reduced ? "none" : `rotate(${((t * 420) % 360).toFixed(1)}deg)`;
    check.svg.style.opacity = s === "done" ? "1" : "0";
    errDot.style.opacity = s === "error" ? "1" : "0";
    check.path.setAttribute("stroke-dashoffset", (1 - (s === "done" ? ease((ts - 0.35) / 0.4) : 0)).toFixed(3));
    const q = Math.max(0, actor.qm.x);
    const hover = actor.lastHover;
    qm.style.cssText = `left:${(CX - 0.95 * U).toFixed(1)}px;top:${(GY - (2.4 + hover) * U).toFixed(1)}px;opacity:${Math.min(1, q * 2) * actor.alpha};transform:scale(${q.toFixed(3)}) rotate(${(-12 + (reduced ? 0 : 6 * Math.sin(t * 3))).toFixed(1)}deg)`;
  };

  const draw = (dt: number) => {
    const dpr = dprFor(quality, window.devicePixelRatio || 1);
    resizeCanvas(canvas, dpr);
    const voice = state === "listening" ? shapeVoice(demoLevel(level, actor.t, state), bands) : null;
    const uniforms = actor.step(dt, voice, quality === "low");
    drawBlob(bundle.gl, bundle.program, canvas, uniforms, U, character === "spirit" && !reduced, { x: CX, y: GY });
    paintPill(dt);
    return uniforms;
  };

  const stop = () => {
    running = false;
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
  };

  const frame = (now: number) => {
    if (!running || disposed) return;
    const elapsed = now - last;
    if (elapsed < 15.5) {
      raf = requestAnimationFrame(frame);
      return;
    }
    const dt = Math.min(0.05, Math.max(0.001, elapsed / 1000));
    last = now;
    const t0 = performance.now();
    const uniforms = draw(dt);
    const ms = performance.now() - t0;
    frameSamples.push(ms);
    if (frameSamples.length > 120) frameSamples.shift();
    const budget = nextQuality(quality, ms, slow, fast);
    quality = budget.quality;
    slow = budget.slow;
    fast = budget.fast;
    frames += 1;
    publishStats();
    if (state === "done" && !tucked && !reduced && !holdPose && actor.t - actor.since > DONE_TUCK_AT_S) {
      tucked = true;
      actor.tuck();
    }
    if (tucked && uniforms.uAlpha < 0.02) {
      stop();
      publishStats();
      return;
    }
    if (reduced && state !== "listening") {
      stop();
      return;
    }
    raf = requestAnimationFrame(frame);
  };

  const start = () => {
    if (record) return;
    if (running || disposed || !wantsFrames(state)) return;
    running = true;
    last = performance.now();
    raf = requestAnimationFrame(frame);
  };

  const publishStats = () => {
    const sum = frameSamples.reduce((total, sample) => total + sample, 0);
    window.__boloBlob = {
      state,
      phase,
      quality,
      frames,
      running,
      dpr: dprFor(quality, window.devicePixelRatio || 1),
      avgFrameMs: frameSamples.length ? sum / frameSamples.length : 0,
      maxFrameMs: frameSamples.length ? Math.max(...frameSamples) : 0,
      mouth: actor.lv,
      reduced,
    };
  };

  const apply = (event: DictationEvent) => {
    if (disposed) return;
    const next = characterState(event.phase);
    const prev = state;
    phase = event.phase;
    message = event.message;
    level = Number.isFinite(event.level) ? event.level : 0;
    bands = event.bands && event.bands.length >= 4 ? event.bands : null;
    stage.dataset.pose = next;
    if (next === "listening" && (prev === "hidden" || prev === "done" || prev === "error" || prev === "fix")) {
      clearWords();
      doneText = "";
    }
    const heard = (event.partial ?? "").trim();
    if (next === "listening" || next === "thinking") {
      if (heard) setWords(heard);
    } else if (next === "done") {
      doneText = heard;
    }
    stage.setAttribute("aria-label", announce(next, event.message, words.map((w) => w.text).join(" "), doneText));
    if (next !== prev) {
      if (next === "listening") {
        tucked = false;
        cues.play("start");
        const id = activeCharacter(prefs);
        cues.configureSettings(prefs, id);
      } else if (next === "done") {
        tucked = false;
        cues.play("done");
      } else if (next === "hidden" && prev !== "hidden") {
        cues.play("dismiss");
      }
      state = next;
      actor.setState(next);
      if (next === "fix") resetFixCard(fixCard);
    } else {
      state = next;
    }
    fixCard.hidden = next !== "fix";
    canvas.hidden = next === "fix" || next === "hidden";
    pill.hidden = next === "fix" || next === "hidden";
    qm.hidden = next !== "error";
    root.hidden = next === "hidden";
    if (!wantsFrames(next)) {
      if (next === "hidden") {
        actor.hide();
        clearWords();
      }
      stop();
      publishStats();
      return;
    }
    if (reduced && next !== "listening") {
      draw(1 / 60);
      publishStats();
      stop();
      return;
    }
    start();
  };

  const onVis = () => {
    if (document.hidden) stop();
    else if (wantsFrames(state)) start();
  };
  document.addEventListener("visibilitychange", onVis);

  void onDictation(apply).then((fn) => {
    if (disposed) fn();
    else unlisten = fn;
  });
  if (inTauri()) {
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen("bolo-settings-changed", () => {
        void api.getSettings().then(applyPrefs).catch(() => {});
      }).then((fn) => {
        if (disposed) fn();
        else unlistenSettings = fn;
      }),
    );
  }

  const demo = params.get("demo");
  let stopDemo = () => {};
  if (!inTauri()) {
    if (record) {
      // Deterministic frames for the parity screenshots (browser only).
      window.__boloRecord = {
        apply,
        frame: (n: number) => {
          for (let i = 0; i < n; i++) draw(1 / 60);
          return state;
        },
      };
    } else {
      root.append(demoSwitcher(apply));
      if (demo === "cycle") stopDemo = runCycle(apply, () => disposed);
      else if (demo === "stream") stopDemo = runStream(apply, () => disposed);
      else apply(previewEvent(demo));
    }
  } else {
    apply({ phase: "idle", message: null, elapsedMs: 0, level: 0 });
  }
  publishStats();

  return () => {
    disposed = true;
    stop();
    stopDemo();
    unlisten?.();
    unlistenSettings?.();
    document.removeEventListener("visibilitychange", onVis);
    motionQuery?.removeEventListener?.("change", onMotion);
    cues.stop();
    releaseBlobGl(bundle.gl);
    root.replaceChildren();
  };
}

/**
 * Where `next` continues `prev` when the caption window has slid
 * (older words dropped from the front). 0 when they line up from the start.
 */
export function findOffset(prev: string[], next: string[]): number {
  if (!prev.length || !next.length) return 0;
  if (prev[0] === next[0]) return 0;
  for (let i = 1; i < prev.length; i++) {
    if (prev[i] === next[0] && prev[i + 1] === next[1]) return i;
  }
  return 0;
}

type PillStatus = { main: string; small: string; tone: "plain" | "error" | "dim" };

/**
 * Status text, as in the mockup: "Inserted into Notepad · 12 words",
 * "Didn't catch that · Hold the key and try again". The word "Listening" is never shown.
 */
export function pillStatus(
  phase: DictationPhase | string,
  state: CharacterState,
  message: string | null | undefined,
  doneText: string,
  hasWords: boolean,
): PillStatus | null {
  const text = publicProblem(message);
  if (state === "listening") {
    if (hasWords) return null;
    // Hands-free and similar hints. A plain "Listening" caption stays hidden: the pill waits for words.
    if (text && !/^listening\b/i.test(text)) return { main: text, small: "", tone: "dim" };
    return null;
  }
  if (state === "thinking") {
    if (hasWords) return null;
    if (text) return { main: text, small: "", tone: "dim" };
    return { main: phase === "inserting" ? "Inserting…" : "Writing…", small: "", tone: "dim" };
  }
  if (state === "done") {
    const main = text || (phase === "typed" ? "Typed" : "Inserted");
    const count = doneText ? doneText.split(/\s+/).filter(Boolean).length : 0;
    return { main, small: count ? `${count} ${count === 1 ? "word" : "words"}` : "", tone: "plain" };
  }
  if (state === "error") {
    if (!text || /didn.?t catch|louder|no speech/i.test(text)) {
      return { main: "Didn't catch that", small: "Hold the key and try again", tone: "error" };
    }
    if (/hold the hotkey/i.test(text)) return { main: "Hold the key", small: "Keep it down while you talk", tone: "error" };
    return { main: "Not inserted", small: text, tone: "error" };
  }
  return null;
}

function announce(state: CharacterState, message: string | null, heard: string, doneText: string): string {
  if (state === "listening" || state === "thinking") return heard || "Listening";
  const model = pillStatus(state === "done" ? "done" : state, state, message, doneText, false);
  if (!model) return "Ready";
  return model.small ? `${model.main}. ${model.small}` : model.main;
}

function clamp(x: number, a = 0, b = 1): number {
  return Math.max(a, Math.min(b, x));
}

export function cachedListener(): string | null {
  try {
    return localStorage.getItem(LISTENER_CACHE);
  } catch {
    return null;
  }
}

function demoLevel(level: number, time: number, state: CharacterState): number {
  if (inTauri() || state !== "listening") return level;
  if (new URLSearchParams(location.search).has("still")) return level;
  const wobble = 0.55 + 0.45 * Math.sin(time * 7.2);
  return Math.max(0, Math.min(1, level * (0.25 + wobble)));
}

function previewEvent(demo: string | null): DictationEvent {
  if (demo === "transcribing" || demo === "inserting" || demo === "thinking") {
    return { phase: demo === "inserting" ? "inserting" : "transcribing", message: null, elapsedMs: 2400, level: 0 };
  }
  if (demo === "error") {
    return {
      phase: "error",
      message: "Didn't catch any speech. Speak a little louder and try again.",
      elapsedMs: 0,
      level: 0,
    };
  }
  if (demo === "done" || demo === "typed") {
    return {
      phase: demo === "typed" ? "typed" : "done",
      message: demo === "typed" ? null : "Inserted into Notepad",
      elapsedMs: 1800,
      level: 0,
      partial: "Let's move the design review to Thursday at three, and send the deck to Sara before lunch.",
    };
  }
  if (demo === "fix") return { phase: "fix", message: null, elapsedMs: 0, level: 0 };
  if (demo === "idle") return { phase: "idle", message: null, elapsedMs: 0, level: 0 };
  return {
    phase: "recording",
    message: null,
    elapsedMs: 4200,
    level: 0.72,
    bands: [0.35, 0.82, 0.28, 0.08],
    partial: "Let's move the design review to Thursday at three, and send the deck to Sara before lunch.",
  };
}

function runCycle(apply: (event: DictationEvent) => void, dead: () => boolean): () => void {
  const sequence: Array<{ demo: string; wait: number }> = [
    { demo: "recording", wait: 1800 },
    { demo: "transcribing", wait: 1500 },
    { demo: "done", wait: 2200 },
    { demo: "error", wait: 1600 },
  ];
  let index = 0;
  let timer = 0;
  let stopped = false;
  const step = () => {
    if (stopped || dead()) return;
    const item = sequence[index];
    if (!item) return;
    apply(previewEvent(item.demo));
    index += 1;
    if (index >= sequence.length) index = 0;
    timer = window.setTimeout(step, item.wait);
  };
  step();
  return () => {
    stopped = true;
    window.clearTimeout(timer);
  };
}

/** Browser preview: a take whose live words grow the way the caption thread delivers them. */
function runStream(apply: (event: DictationEvent) => void, dead: () => boolean): () => void {
  const sentence = "Let's move the design review to Thursday at three, and send the deck to Sara before lunch.".split(" ");
  let n = 0;
  let stopped = false;
  let t = 0;
  const meter = window.setInterval(() => {
    if (stopped || dead()) return;
    t += 0.08;
    const level = 0.35 + 0.35 * Math.abs(Math.sin(t * 5.3));
    apply({ phase: "recording", message: null, elapsedMs: t * 1000, level, bands: [level * 0.4, level * 0.9, level * 0.35, level * 0.1], partial: sentence.slice(0, n).join(" ") || null });
  }, 80);
  const grow = window.setInterval(() => {
    if (stopped || dead()) return;
    n = Math.min(sentence.length, n + 2 + (n % 2));
    if (n >= sentence.length) {
      window.clearInterval(grow);
    }
  }, 700);
  return () => {
    stopped = true;
    window.clearInterval(meter);
    window.clearInterval(grow);
  };
}

function demoSwitcher(apply: (event: DictationEvent) => void): HTMLElement {
  const row = document.createElement("div");
  row.className = "demo-row";
  const phases: Array<[string, string]> = [
    ["recording", "listen"],
    ["transcribing", "think"],
    ["done", "done"],
    ["error", "error"],
    ["fix", "fix"],
    ["idle", "hide"],
  ];
  for (const [demo, label] of phases) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.addEventListener("click", () => apply(previewEvent(demo)));
    row.append(button);
  }
  return row;
}

function svgSpin(): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 18 18");
  svg.classList.add("v4-pspin");
  svg.innerHTML =
    '<circle cx="9" cy="9" r="6.5" fill="none" stroke="rgba(217,180,74,.25)" stroke-width="2"/><path d="M9 2.5a6.5 6.5 0 0 1 6.5 6.5" fill="none" stroke="#e6c46a" stroke-width="2" stroke-linecap="round"/>';
  return svg;
}

function svgCheck(): { svg: SVGSVGElement; path: SVGPathElement } {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 18 18");
  svg.classList.add("v4-pchk");
  svg.innerHTML =
    '<defs><radialGradient id="blobChk" cx="35%" cy="30%" r="80%"><stop offset="0" stop-color="#ffe7a3"/><stop offset=".55" stop-color="#D9B44A"/><stop offset="1" stop-color="#94701f"/></radialGradient></defs><circle cx="9" cy="9" r="8.5" fill="url(#blobChk)"/><path d="M5.3 9.3l2.4 2.4 5-5.2" fill="none" stroke="#1b1407" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" pathLength="1" stroke-dasharray="1" stroke-dashoffset="1"/>';
  const path = svg.querySelector("path");
  if (!path) throw new Error("check mark");
  return { svg, path };
}

function sm(x: number): number {
  const c = Math.max(0, Math.min(1, x));
  return c * c * (3 - 2 * c);
}

function ease(x: number): number {
  const c = Math.max(0, Math.min(1, x));
  return 1 - (1 - c) ** 3;
}

declare global {
  interface Window {
    __boloBlob?: {
      state: string;
      phase: string;
      quality: string;
      frames: number;
      running: boolean;
      dpr: number;
      avgFrameMs: number;
      maxFrameMs: number;
      mouth: number;
      reduced: boolean;
    };
    __boloRecord?: {
      apply: (event: DictationEvent) => void;
      frame: (n: number) => CharacterState;
    };
  }
}
