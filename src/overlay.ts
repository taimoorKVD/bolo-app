import { api, inTauri, onDictation } from "./api";
import {
  DEFAULT_CHARACTER,
  activeCharacter,
  clipUrls,
  resolvePack,
  soundUrl,
  type SoundCue,
  type SoundPackId,
} from "./characters";
import type { DictationEvent, DictationPhase, Settings } from "./types";

type Pose = "listen" | "write" | "leave" | "error" | "fix" | "hidden";

export function mountOverlay(root: HTMLElement): void {
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
  const fixCard = buildFixCard();
  stage.append(listen, write, hide, note, fixCard);
  root.append(stage);

  const cues = new CuePlayer();
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
    cues.configure(resolvePack(settings.soundPack, id), settings.soundVolume ?? 0.6);
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
        // Swap only while hidden so a take in progress never flashes.
        if (pose === "hidden") void refreshPrefs();
        else pendingRefresh = true;
      }),
    );
  }

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
    root.hidden = next === "hidden";
    if (next === "hidden" && pendingRefresh) {
      pendingRefresh = false;
      void refreshPrefs();
    }
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

/** Plays the start, done, and dismiss cues for the chosen pack. */
class CuePlayer {
  private pack: SoundPackId | null = null;
  private volume = 0.6;
  private audio = new Map<SoundCue, HTMLAudioElement>();
  private tokens = new WeakMap<HTMLAudioElement, number>();

  configure(pack: SoundPackId, volume: number): void {
    this.volume = Math.max(0, Math.min(1, Number.isFinite(volume) ? volume : 0.6));
    if (pack === this.pack) return;
    this.pack = pack;
    this.audio.clear();
    for (const cue of ["start", "done", "dismiss"] as SoundCue[]) {
      const url = soundUrl(pack, cue);
      if (!url) continue;
      const node = new Audio(url);
      node.preload = "auto";
      node.load();
      this.audio.set(cue, node);
      this.warm(node);
    }
  }

  play(cue: SoundCue): void {
    const node = this.audio.get(cue);
    if (!node || this.volume <= 0) return;
    this.tokens.set(node, (this.tokens.get(node) ?? 0) + 1);
    // The mic is already open for the start cue, so keep it soft.
    node.volume = cue === "start" ? this.volume * 0.45 : this.volume;
    try {
      node.pause();
      node.currentTime = 0;
    } catch {
      // The element may not be seekable until the first bytes arrive.
    }
    const pending = node.play();
    if (pending) void pending.catch(() => {});
  }

  /** Decode once silently so the first real cue is not late. */
  private warm(node: HTMLAudioElement): void {
    const mine = (this.tokens.get(node) ?? 0) + 1;
    this.tokens.set(node, mine);
    node.volume = 0;
    const reset = () => {
      if (mine !== this.tokens.get(node)) return;
      node.pause();
      try {
        node.currentTime = 0;
      } catch {
        // Ignore a seek that races the first play.
      }
    };
    const pending = node.play();
    if (pending) void pending.then(reset).catch(reset);
    else reset();
  }
}

function demoEvent(demo: string | null): DictationEvent {
  if (demo === "transcribing" || demo === "inserting") {
    return { phase: "transcribing", message: null, elapsedMs: 2400, level: 0 };
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
    return { phase: "recording", message: null, elapsedMs: 0, level: 0.4 };
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

/** Small card offered for a few seconds after a paste. */
function buildFixCard(): HTMLElement {
  const card = el("div", "fix-card");
  card.hidden = true;
  const first = document.createElement("button");
  first.type = "button";
  first.className = "fix-main";
  first.textContent = "↻ Fix last";
  const choices = el("div", "fix-choices");
  choices.hidden = true;
  const again = document.createElement("button");
  again.type = "button";
  again.textContent = "Listen again";
  const type = document.createElement("button");
  type.type = "button";
  type.textContent = "Type it";
  choices.append(again, type);
  const close = document.createElement("button");
  close.type = "button";
  close.className = "fix-close";
  close.textContent = "✕";
  close.setAttribute("aria-label", "No thanks");
  first.addEventListener("click", () => {
    first.hidden = true;
    choices.hidden = false;
  });
  const send = (action: "relisten" | "type" | "dismiss") => {
    void api.overlayFix(action).catch((error) => console.warn("fix", error));
  };
  again.addEventListener("click", () => send("relisten"));
  type.addEventListener("click", () => send("type"));
  close.addEventListener("click", () => send("dismiss"));
  card.append(first, choices, close);
  return card;
}

function resetFixCard(card: HTMLElement): void {
  const first = card.querySelector<HTMLElement>(".fix-main");
  const choices = card.querySelector<HTMLElement>(".fix-choices");
  if (first) first.hidden = false;
  if (choices) choices.hidden = true;
}
