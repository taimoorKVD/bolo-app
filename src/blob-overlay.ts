import { api, inTauri, onDictation } from "./api";
import { BlobActor } from "./blob-actor";
import { createBlobGl, drawBlob, releaseBlobGl, resizeCanvas } from "./blob-gl";
import {
  DONE_TUCK_AT_S,
  characterState,
  dprFor,
  nextQuality,
  pillModel,
  liveCaption,
  prefersReducedMotion,
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

export function mountBlobOverlay(root: HTMLElement, listener: ListenerId = "blob"): (() => void) | null {
  const character = isAnimatedListener(listener) ? listener : "blob";
  root.replaceChildren();
  const stage = document.createElement("div");
  stage.className = "blob-stage";
  stage.dataset.pose = "hidden";
  stage.dataset.character = character;
  stage.setAttribute("role", "status");
  stage.setAttribute("aria-live", "polite");

  const canvas = document.createElement("canvas");
  canvas.className = "blob-canvas";
  canvas.setAttribute("aria-hidden", "true");
  const bundle = createBlobGl(canvas, character);
  if (!bundle) {
    root.replaceChildren();
    return null;
  }

  const pill = document.createElement("div");
  pill.className = "blob-pill";
  pill.hidden = true;
  const icon = document.createElement("div");
  icon.className = "blob-picon";
  const dot = document.createElement("div");
  dot.className = "blob-pdot";
  const spin = svgSpin();
  const check = svgCheck();
  const errDot = document.createElement("div");
  errDot.className = "blob-perr";
  icon.append(dot, spin, check.svg, errDot);
  const cap = document.createElement("div");
  cap.className = "blob-pcap";
  const line = document.createElement("div");
  line.className = "blob-line";
  const status = document.createElement("div");
  status.className = "blob-status";
  cap.append(line, status);
  pill.append(icon, cap);

  const qm = document.createElement("div");
  qm.className = "blob-qm";
  qm.textContent = "?";

  const fixCard = buildFixCard();
  stage.append(canvas, pill, qm, fixCard);
  root.append(stage);

  const actor = new BlobActor(prefersReducedMotion(), character);
  const cues = new CuePlayer();
  let prefs: Partial<Settings> = {};
  let state: CharacterState = "hidden";
  let phase: DictationPhase = "idle";
  let message: string | null = null;
  let partial: string | null = null;
  let level = 0;
  let bands: number[] | null = null;
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

  const paintPill = () => {
    const model = pillModel(phase, message, partial);
    const show = state !== "hidden" && state !== "fix" && actor.pill.x > 0.04 && !actor.hidden;
    pill.hidden = !show;
    if (!show) return;
    if (line.textContent !== model.line) line.textContent = model.line;
    const statusText = model.detail ? `${model.status}` : model.status;
    if (status.textContent !== statusText || status.dataset.detail !== model.detail) {
      status.textContent = "";
      status.append(document.createTextNode(model.status));
      if (model.detail) {
        const small = document.createElement("small");
        small.textContent = model.detail;
        status.append(small);
      }
      status.dataset.detail = model.detail;
    }
    line.hidden = !model.showLine;
    status.hidden = !model.showStatus;
    const scale = Math.max(0, Math.min(1.05, actor.pill.x));
    const opacity = sm(scale * 1.6);
    pill.style.opacity = opacity.toFixed(3);
    pill.style.transform = `scale(${(0.92 + 0.08 * scale).toFixed(3)})`;
    dot.style.opacity = state === "listening" ? "1" : "0";
    dot.style.transform = `scale(${(0.8 + actor.lv * 0.55).toFixed(3)})`;
    spin.style.opacity = state === "thinking" && !reduced ? "1" : "0";
    if (state === "thinking" && !reduced) spin.style.transform = `rotate(${((actor.t * 420) % 360).toFixed(1)}deg)`;
    check.svg.style.opacity = state === "done" ? "1" : "0";
    errDot.style.opacity = state === "error" ? "1" : "0";
    const drawn = state === "done" ? ease(Math.max(0, actor.t - actor.since - 0.35) / 0.4) : 0;
    check.path.setAttribute("stroke-dashoffset", (1 - drawn).toFixed(3));
    const q = Math.max(0, actor.qm.x);
    qm.style.opacity = String(Math.min(1, q * 2) * actor.alpha);
    qm.style.transform = `scale(${q.toFixed(3)}) rotate(${(-12 + (reduced ? 0 : 6 * Math.sin(actor.t * 3))).toFixed(1)}deg)`;
  };

  const render = () => {
    const dpr = dprFor(quality, window.devicePixelRatio || 1);
    resizeCanvas(canvas, dpr);
    const voice = state === "listening" ? shapeVoice(demoLevel(level, actor.t, state), bands) : shapeVoice(0, null);
    const uniforms = actor.step(1 / 60, voice, quality === "low");
    drawBlob(bundle.gl, bundle.program, canvas, uniforms, actor.rig.unit, character === "spirit" && !reduced);
    paintPill();
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
    const dpr = dprFor(quality, window.devicePixelRatio || 1);
    resizeCanvas(canvas, dpr);
    const voice = state === "listening" ? shapeVoice(demoLevel(level, actor.t, state), bands) : null;
    const uniforms = actor.step(dt, voice, quality === "low");
    if (uniforms.uAlpha > 0.001) {
      drawBlob(bundle.gl, bundle.program, canvas, uniforms, actor.rig.unit, character === "spirit" && !reduced);
    }
    else {
      bundle.gl.viewport(0, 0, canvas.width, canvas.height);
      bundle.gl.clearColor(0, 0, 0, 0);
      bundle.gl.clear(bundle.gl.COLOR_BUFFER_BIT);
    }
    paintPill();
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
    partial = event.partial ?? null;
    level = Number.isFinite(event.level) ? event.level : 0;
    bands = event.bands && event.bands.length >= 4 ? event.bands : null;
    stage.dataset.pose = next;
    const words = liveCaption(event.phase, event.message, event.partial);
    stage.setAttribute("aria-label", words || announce(event.phase, event.message));
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
    qm.hidden = next !== "error";
    root.hidden = next === "hidden";
    if (!wantsFrames(next)) {
      pill.hidden = true;
      if (next === "hidden") actor.hide();
      stop();
      publishStats();
      return;
    }
    if (reduced && next !== "listening") {
      render();
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
    root.append(demoSwitcher(apply));
    if (demo === "cycle") stopDemo = runCycle(apply, () => disposed);
    else apply(previewEvent(demo));
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

function announce(phase: DictationPhase, message: string | null): string {
  const model = pillModel(phase, message);
  if (model.detail) return `${model.status}. ${model.detail}`;
  return model.status || model.line || "Ready";
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
    return { phase: demo === "typed" ? "typed" : "done", message: demo === "typed" ? null : "Inserted", elapsedMs: 1800, level: 0 };
  }
  if (demo === "fix") return { phase: "fix", message: null, elapsedMs: 0, level: 0 };
  if (demo === "idle") return { phase: "idle", message: null, elapsedMs: 0, level: 0 };
  return {
    phase: "recording",
    message: null,
    elapsedMs: 4200,
    level: 0.72,
    bands: [0.35, 0.82, 0.28, 0.08],
    partial: "ship the notes to Sam by Friday",
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
  svg.classList.add("blob-pspin");
  svg.innerHTML =
    '<circle cx="9" cy="9" r="6.5" fill="none" stroke="rgba(217,180,74,.25)" stroke-width="2"/><path d="M9 2.5a6.5 6.5 0 0 1 6.5 6.5" fill="none" stroke="#e6c46a" stroke-width="2" stroke-linecap="round"/>';
  return svg;
}

function svgCheck(): { svg: SVGSVGElement; path: SVGPathElement } {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 18 18");
  svg.classList.add("blob-pchk");
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
  }
}
