import { api, inTauri, onDictation } from "./api";
import { mountBlobOverlay, cachedListener } from "./blob-overlay";
import {
  DEFAULT_CHARACTER,
  activeCharacter,
  clipUrls,
} from "./characters";
import { CuePlayer } from "./cues";
import { buildFixCard, resetFixCard } from "./fix-card";
import { liveCaption } from "./blob-state";
import { normalizeListener, type ListenerId } from "./listener-style";
import type { DictationEvent, DictationPhase, Settings } from "./types";

type Pose = "listen" | "write" | "leave" | "error" | "fix" | "hidden";

export function mountOverlay(root: HTMLElement): void {
  let mode: ListenerId | null = null;
  let dispose: (() => void) | null = null;

  const mount = (id: ListenerId) => {
    if (id === mode) return;
    dispose?.();
    mode = id;
    const started = id === "classic" ? mountClassicOverlay(root) : mountBlobOverlay(root, id);
    if (started) {
      dispose = started;
      return;
    }
    mode = "classic";
    dispose = mountClassicOverlay(root);
  };

  const fromSettings = (settings: Partial<Settings>) => {
    const forced = previewListener();
    mount(forced ?? normalizeListener(settings.listener ?? cachedListener()));
  };

  fromSettings({});
  void api.getSettings().then(fromSettings).catch((error) => {
    console.warn("overlay could not read settings", error);
  });
  if (inTauri()) {
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen("bolo-settings-changed", () => {
        void api.getSettings().then(fromSettings).catch(() => {});
      }),
    );
  }
}

function previewListener(): ListenerId | null {
  if (inTauri()) return null;
  const value = new URLSearchParams(location.search).get("listener");
  if (!value) return null;
  return normalizeListener(value);
}

function mountClassicOverlay(root: HTMLElement): () => void {
  root.replaceChildren();
  const stage = el("div", "listener");
  stage.dataset.pose = "hidden";
  stage.setAttribute("role", "status");
  stage.setAttribute("aria-live", "polite");

  const initial = clipUrls(DEFAULT_CHARACTER);
  const listen = clip("clip-listen", initial.listen);
  const write = clip("clip-write", initial.write);
  const hide = clip("clip-hide", initial.hide);
  hide.loop = false;
  hide.autoplay = false;
  stage.classList.toggle("alpha", initial.alpha);
  const note = el("p", "listener-note");
  note.hidden = true;
  const live = el("p", "listener-live");
  live.hidden = true;
  const fixCard = buildFixCard();
  stage.append(listen, write, hide, live, note, fixCard);
  root.append(stage);

  const cues = new CuePlayer();
  let disposed = false;
  let unlistenDictation: (() => void) | null = null;
  let unlistenSettings: (() => void) | null = null;
  let characterId = DEFAULT_CHARACTER;
  let alpha = initial.alpha;
  let prefs: Partial<Settings> = {};
  let pose: Pose = "hidden";
  let pendingRefresh = false;

  const useCharacter = (id: string) => {
    if (id === characterId) return;
    characterId = id;
    const urls = clipUrls(id);
    listen.src = urls.listen;
    write.src = urls.write;
    alpha = urls.alpha;
    if (urls.hide) hide.src = urls.hide;
    else hide.removeAttribute("src");
    stage.classList.toggle("alpha", alpha);
    syncClips(pose, listen, write);
  };
  const applyPrefs = (settings: Partial<Settings>) => {
    prefs = settings;
    const id = activeCharacter(settings);
    cues.configureSettings(settings, id);
    useCharacter(id);
  };
  const refreshPrefs = async () => {
    try {
      applyPrefs(await api.getSettings());
    } catch (error) {
      console.warn("overlay could not read settings", error);
    }
  };
  void refreshPrefs();
  if (inTauri()) {
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen("bolo-settings-changed", () => {
        if (disposed) return;
        // Swap only while hidden so a take in progress never flashes.
        if (pose === "hidden") void refreshPrefs();
        else pendingRefresh = true;
      }).then((un) => {
        if (disposed) un();
        else unlistenSettings = un;
      }),
    );
  }

  const apply = (event: DictationEvent) => {
    const next = poseFor(event.phase);
    const gain = Math.max(0, Math.min(1, event.level));
    stage.style.setProperty("--gain", gain.toFixed(3));
    // Keep listen playback steady; gain-tied rate changes made the loop look jerky.
    if (!listen.paused && listen.playbackRate !== 1) listen.playbackRate = 1;
    const words = liveCaption(event.phase, event.message, event.partial);
    stage.setAttribute("aria-label", words || announce(event.phase, event.message));

    if (next !== pose) {
      if (next === "leave") {
        // Start the sound before the hide motion so it is not late.
        cues.play("dismiss");
        if (alpha) {
          // Re-made characters bring their own hide animation.
          listen.hidden = true;
          write.hidden = true;
          listen.pause();
          write.pause();
          hide.hidden = false;
          try {
            hide.currentTime = 0;
          } catch {
            // Not seekable yet; it still plays from the start.
          }
          void hide.play().catch(() => {});
        } else {
          stage.classList.add("is-leaving");
        }
      } else {
        stage.classList.remove("is-leaving");
        hide.hidden = true;
        hide.pause();
        for (const node of [listen, write]) {
          node.style.animation = "none";
          void node.offsetWidth;
          node.style.animation = "";
        }
      }
      pose = next;
      stage.dataset.pose = next;
      if (next === "listen") {
        // Smart rules (night, holiday) can change between takes.
        const id = activeCharacter(prefs);
        if (id !== characterId) useCharacter(id);
        cues.play("start");
        void refreshPrefs();
      } else if (next === "write") {
        cues.play("done");
      }
      syncClips(next, listen, write);
    }

    const problem = next === "error" ? overlayProblem(event.message) : "";
    fixCard.hidden = next !== "fix";
    if (next === "fix") resetFixCard(fixCard);
    note.textContent = problem;
    note.hidden = problem.length === 0;
    live.textContent = words;
    live.hidden = words.length === 0 || next === "error" || next === "fix";
    root.hidden = next === "hidden";
    if (next === "hidden" && pendingRefresh) {
      pendingRefresh = false;
      void refreshPrefs();
    }
  };

  void onDictation((event) => {
    if (!disposed) apply(event);
  }).then((un) => {
    if (disposed) un();
    else unlistenDictation = un;
  });

  const params = new URLSearchParams(location.search);
  const demo = params.get("demo");
  if (!inTauri()) {
    root.append(demoSwitcher(apply));
    if (demo === "cycle") runCycle(apply);
    else apply(demoEvent(demo));
  } else {
    apply({ phase: "idle", message: null, elapsedMs: 0, level: 0 });
  }

  return () => {
    disposed = true;
    unlistenDictation?.();
    unlistenSettings?.();
    listen.pause();
    write.pause();
    hide.pause();
    cues.stop();
    root.replaceChildren();
  };
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
  if (pose === "fix") {
    listen.hidden = true;
    write.hidden = true;
    listen.pause();
    write.pause();
    return;
  }
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
    case "fix":
      return "fix";
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
    case "fix":
      return "Fix last dictation";
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

function demoEvent(demo: string | null): DictationEvent {
  if (demo === "transcribing" || demo === "inserting") {
    return { phase: "transcribing", message: null, elapsedMs: 2400, level: 0, partial: "ship the notes to Sam by Friday" };
  }
  if (demo === "error") {
    return {
      phase: "error",
      message: "Dictation had a brief problem. Bolo will try again on its own.",
      elapsedMs: 0,
      level: 0,
    };
  }
  if (demo === "done" || demo === "typed") {
    return { phase: "done", message: "Inserted", elapsedMs: 1800, level: 0, engine: "Groq" };
  }
  if (demo === "arming") {
    return { phase: "recording", message: null, elapsedMs: 0, level: 0.4, partial: "ship the notes to Sam by Friday" };
  }
  if (demo === "fix") {
    return { phase: "fix", message: null, elapsedMs: 0, level: 0 };
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
    partial: "ship the notes to Sam by Friday",
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
    ["fix", "fix"],
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
