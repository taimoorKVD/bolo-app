import { inTauri, onDictation } from "./api";
import type { DictationEvent, DictationPhase } from "./types";

const dismissUrl = new URL("./assets/dismiss.wav", import.meta.url).href;
const listenUrl = new URL("./assets/listen.webm", import.meta.url).href;
const writeUrl = new URL("./assets/write.webm", import.meta.url).href;

type Pose = "listen" | "write" | "leave" | "error" | "hidden";

export function mountOverlay(root: HTMLElement): void {
  root.replaceChildren();
  const stage = el("div", "listener");
  stage.dataset.pose = "hidden";
  stage.setAttribute("role", "status");
  stage.setAttribute("aria-live", "polite");

  const listen = clip("clip-listen", listenUrl);
  const write = clip("clip-write", writeUrl);
  const note = el("p", "listener-note");
  note.hidden = true;
  stage.append(listen, write, note);
  root.append(stage);

  const dismiss = new Audio(dismissUrl);
  dismiss.preload = "auto";
  dismiss.load();
  let pose: Pose = "hidden";
  let warmToken = 0;
  let warmed = false;

  const apply = (event: DictationEvent) => {
    const next = poseFor(event.phase);
    const gain = Math.max(0, Math.min(1, event.level));
    stage.style.setProperty("--gain", gain.toFixed(3));
    // Keep listen playback steady; gain-tied rate changes made the loop look jerky.
    if (!listen.paused && listen.playbackRate !== 1) listen.playbackRate = 1;
    stage.setAttribute("aria-label", announce(event.phase, event.message));

    if (next !== pose) {
      if (next === "leave") {
        // Start the sound before the hide motion so it is not late.
        playDismissNow(dismiss, () => {
          warmToken += 1;
        });
        stage.classList.add("is-leaving");
      } else {
        stage.classList.remove("is-leaving");
        for (const node of [listen, write]) {
          node.style.animation = "none";
          void node.offsetWidth;
          node.style.animation = "";
        }
      }
      pose = next;
      stage.dataset.pose = next;
      syncClips(next, listen, write);
      if (next === "listen" && !warmed) {
        warmed = true;
        warmDismiss(dismiss, () => warmToken, () => {
          warmToken += 1;
          return warmToken;
        });
      }
    }

    const problem = next === "error" ? overlayProblem(event.message) : "";
    note.textContent = problem;
    note.hidden = problem.length === 0;
    root.hidden = next === "hidden";
  };

  void onDictation(apply);

  const params = new URLSearchParams(location.search);
  const demo = params.get("demo");
  if (!inTauri()) {
    root.append(demoSwitcher(apply));
    if (demo === "cycle") runCycle(apply);
    else apply(demoEvent(demo));
  } else {
    apply({ phase: "idle", message: null, elapsedMs: 0, level: 0 });
  }
}

function clip(className: string, src: string): HTMLVideoElement {
  const node = document.createElement("video");
  node.className = `clip ${className}`;
  node.src = src;
  node.muted = true;
  node.loop = true;
  node.autoplay = true;
  node.playsInline = true;
  node.preload = "auto";
  node.setAttribute("aria-hidden", "true");
  node.hidden = true;
  return node;
}

function syncClips(pose: Pose, listen: HTMLVideoElement, write: HTMLVideoElement): void {
  if (pose === "hidden") {
    listen.pause();
    write.pause();
    return;
  }
  // Leave keeps whichever clip is already on screen and animates it away.
  if (pose === "leave") return;
  const active = pose === "write" ? write : listen;
  const idle = active === listen ? write : listen;
  active.hidden = false;
  idle.hidden = true;
  idle.pause();
  if (pose === "error") {
    active.pause();
    return;
  }
  void active.play().catch(() => {
    // The loop still shows its first frame if autoplay is blocked.
  });
}

function poseFor(phase: DictationPhase): Pose {
  switch (phase) {
    case "arming":
    case "recording":
      return "listen";
    case "transcribing":
    case "inserting":
      return "write";
    case "done":
    case "typed":
      return "leave";
    case "error":
      return "error";
    default:
      return "hidden";
  }
}

function announce(phase: DictationPhase, message: string | null): string {
  switch (phase) {
    case "arming":
    case "recording":
      return "Taking dictation";
    case "transcribing":
    case "inserting":
      return "Writing the transcript";
    case "done":
    case "typed":
      return "Inserted";
    case "error":
      return overlayProblem(message) || "Not inserted";
    default:
      return "Ready";
  }
}

/** Failure copy for the overlay. The engine name is never shown here. */
function overlayProblem(message: string | null): string {
  const text = (message ?? "").trim();
  if (!text) return "";
  return text
    .replace(/a Groq API key/g, "an API key")
    .replace(/Groq API key/g, "API key")
    .replace(/Groq/g, "the service");
}

function playDismissNow(audio: HTMLAudioElement, bump: () => void): void {
  bump();
  audio.volume = 1;
  try {
    audio.pause();
    audio.currentTime = 0;
  } catch {
    // The element may not be seekable until the first bytes arrive.
  }
  const pending = audio.play();
  if (pending) void pending.catch(() => {});
}

function warmDismiss(audio: HTMLAudioElement, token: () => number, bump: () => number): void {
  const mine = bump();
  audio.volume = 0;
  const pending = audio.play();
  const reset = () => {
    if (mine !== token()) return;
    audio.pause();
    try {
      audio.currentTime = 0;
    } catch {
      // Ignore a seek that races the first play.
    }
    audio.volume = 1;
  };
  if (pending) void pending.then(reset).catch(reset);
  else reset();
}

function demoEvent(demo: string | null): DictationEvent {
  if (demo === "transcribing" || demo === "inserting") {
    return { phase: "transcribing", message: null, elapsedMs: 2400, level: 0 };
  }
  if (demo === "error") {
    return {
      phase: "error",
      message: "Groq rejected the API key. Check it in Settings.",
      elapsedMs: 0,
      level: 0,
    };
  }
  if (demo === "done" || demo === "typed") {
    return { phase: "done", message: "Inserted", elapsedMs: 1800, level: 0, engine: "Groq" };
  }
  if (demo === "arming") {
    return { phase: "recording", message: null, elapsedMs: 0, level: 0.4 };
  }
  if (demo === "idle") {
    return { phase: "idle", message: null, elapsedMs: 0, level: 0 };
  }
  return {
    phase: "recording",
    message: null,
    elapsedMs: 4200,
    level: 0.62,
    engine: "Groq",
  };
}

function runCycle(apply: (event: DictationEvent) => void): void {
  const sequence: Array<{ phase: DictationPhase; wait: number }> = [
    { phase: "recording", wait: 2400 },
    { phase: "transcribing", wait: 2800 },
    { phase: "done", wait: 0 },
  ];
  let index = 0;
  const step = () => {
    const item = sequence[index];
    if (!item) return;
    apply(demoEvent(item.phase));
    index += 1;
    if (item.wait > 0) window.setTimeout(step, item.wait);
  };
  step();
}

function demoSwitcher(apply: (event: DictationEvent) => void): HTMLElement {
  const row = el("div", "demo-row");
  const phases: Array<[DictationPhase, string]> = [
    ["recording", "listen"],
    ["transcribing", "write"],
    ["done", "hide"],
    ["error", "problem"],
  ];
  for (const [phase, label] of phases) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.addEventListener("click", () => apply(demoEvent(phase)));
    row.append(button);
  }
  return row;
}

function el(tag: string, className?: string, text?: string): HTMLElement {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}
