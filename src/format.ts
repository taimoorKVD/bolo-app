import type { Hotkey } from "./types";

export function formatHotkey(hotkey: Hotkey): string {
  const parts: string[] = [];
  if (hotkey.ctrl) parts.push("Ctrl");
  if (hotkey.alt) parts.push("Alt");
  if (hotkey.shift) parts.push("Shift");
  if (hotkey.win) parts.push("Win");
  if (hotkey.key) parts.push(prettyKey(hotkey.key));
  return parts.length > 0 ? parts.join(" + ") : "Not set";
}

export function prettyKey(key: string): string {
  const names: Record<string, string> = {
    space: "Space",
    tab: "Tab",
    enter: "Enter",
    backspace: "Backspace",
    delete: "Delete",
    insert: "Insert",
    home: "Home",
    end: "End",
    pageup: "Page Up",
    pagedown: "Page Down",
    left: "Left",
    right: "Right",
    up: "Up",
    down: "Down",
    esc: "Esc",
  };
  if (names[key]) return names[key];
  if (/^f\d{1,2}$/.test(key)) return key.toUpperCase();
  if (key.length === 1) return key.toUpperCase();
  return key;
}

export function formatWhen(ms: number): string {
  const date = new Date(ms);
  const sameDay = date.toDateString() === new Date().toDateString();
  if (sameDay) {
    return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  }
  return date.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function formatDuration(ms: number): string {
  const seconds = Math.max(0.1, Math.round(ms / 100) / 10);
  return `${seconds.toFixed(1)}s`;
}

export function formatElapsed(ms: number): string {
  const total = Math.floor(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function modelLabel(model: string): string {
  if (model === "whisper-large-v3") return "Whisper Large v3";
  if (model === "whisper-large-v3-turbo") return "Whisper Large v3 Turbo";
  return model;
}
