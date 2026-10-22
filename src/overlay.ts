import { inTauri, onDictation } from "./api";
import { formatElapsed } from "./format";
import type { DictationEvent, DictationPhase } from "./types";

export function mountOverlay(root: HTMLElement): void {
  root.replaceChildren();
  const pill = el("div", "pill");
  const lamp = el("span", "lamp");
  lamp.setAttribute("aria-hidden", "true");
  const copy = el("div", "pill-copy");
  const label = el("strong", "pill-label", "Listening");
  const detail = el("span", "pill-detail");
  copy.append(label, detail);
  const bars = el("div", "bars");
  bars.setAttribute("aria-hidden", "true");
  const barEls = Array.from({ length: 5 }, () => el("span"));
  bars.append(...barEls);
  const timer = el("span", "timer", "0:00");
  const engine = el("span", "engine");
  engine.hidden = true;
  pill.append(lamp, copy, bars, engine, timer);
  pill.setAttribute("role", "status");
  pill.setAttribute("aria-live", "polite");
  root.append(pill);

  const apply = (event: DictationEvent) => {
    const phase = event.phase;
    pill.dataset.phase = phase;
    label.textContent = phaseTitle(phase);
    detail.textContent = event.message ?? "";
    detail.hidden = !event.message;
    timer.textContent = phase === "recording" ? formatElapsed(event.elapsedMs) : "";
    timer.hidden = phase !== "recording";
    bars.hidden = phase !== "recording" && phase !== "arming";
    const engineName = event.engine?.trim() ?? "";
    engine.hidden = engineName.length === 0;
    engine.textContent = engineName;
    const level = Math.max(0, Math.min(1, event.level));
    barEls.forEach((bar, index) => {
      const shape = [0.45, 0.75, 1, 0.7, 0.4][index] ?? 0.5;
      const height = 4 + Math.round(level * shape * 16);
      bar.style.height = `${height}px`;
    });
    root.hidden = phase === "idle";
  };

  void onDictation(apply);

  const params = new URLSearchParams(location.search);
  const demo = params.get("demo");
  if (!inTauri()) {
    apply(demoEvent(demo));
    root.append(demoSwitcher(apply));
  } else {
    apply({ phase: "idle", message: null, elapsedMs: 0, level: 0 });
  }
}

function phaseTitle(phase: DictationPhase): string {
  switch (phase) {
    case "arming":
      return "Starting";
    case "recording":
      return "Listening";
    case "transcribing":
      return "Transcribing";
    case "inserting":
      return "Inserting";
    case "done":
      return "Inserted";
    case "typed":
      return "Typed";
    case "error":
      return "Not inserted";
    default:
      return "Ready";
  }
}

function demoEvent(demo: string | null): DictationEvent {
  if (demo === "transcribing") {
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
  if (demo === "done") {
    return { phase: "done", message: "Inserted", elapsedMs: 1800, level: 0, engine: "Groq" };
  }
  if (demo === "arming") {
    return { phase: "arming", message: "Opening the microphone", elapsedMs: 0, level: 0 };
  }
  if (demo === "typed") {
    return { phase: "typed", message: "Typed", elapsedMs: 1800, level: 0, engine: "On this PC" };
  }
  return {
    phase: "recording",
    message: "Microphone Array · system default",
    elapsedMs: 4200,
    level: 0.62,
  };
}

function demoSwitcher(apply: (event: DictationEvent) => void): HTMLElement {
  const row = el("div", "demo-row");
  const phases: DictationPhase[] = ["arming", "recording", "transcribing", "inserting", "done", "typed", "error"];
  for (const phase of phases) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = phase;
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
