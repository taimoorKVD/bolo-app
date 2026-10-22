import { emitLocalDictation } from "./api";
import type {
  ApiKeyStatus,
  AppInfo,
  HistoryEntry,
  MicInfo,
  Settings,
} from "./types";

function previewParam(name: string): string | null {
  if (typeof location === "undefined") return null;
  const value = new URLSearchParams(location.search).get(name);
  return value && value.trim() ? value : null;
}

const settings: Settings = {
  hotkey: { ctrl: true, alt: false, shift: false, win: true, key: null },
  microphone: previewParam("mic"),
  model: "whisper-large-v3-turbo",
  cleanupEnabled: true,
  language: "auto",
  dictionary: ["Bolo.com", "Groq"],
  launchAtStartup: false,
  historyEnabled: true,
};

let apiKey: string | null = null;
let recording = false;
let appInfoReads = 0;
let history: HistoryEntry[] = [
  {
    id: "sample-1",
    text: "Move the standup to Thursday and send the notes to Priya.",
    createdUnixMs: Date.now() - 1000 * 60 * 42,
    model: "whisper-large-v3-turbo",
    cleaned: false,
    durationMs: 3100,
  },
  {
    id: "sample-2",
    text: "The installer should land in the user's local app data, not Program Files.",
    createdUnixMs: Date.now() - 1000 * 60 * 60 * 5,
    model: "whisper-large-v3",
    cleaned: true,
    durationMs: 4800,
  },
];

const mics: MicInfo[] = [
  { name: "Microphone Array", isDefault: true },
  { name: "Jack Mic (2- Realtek(R) Audio)", isDefault: false },
];

function keyStatus(): ApiKeyStatus {
  if (!apiKey) return { configured: false, hint: null, error: null };
  return { configured: true, hint: `····${apiKey.slice(-4)}`, error: null };
}

export async function mockInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  await delay(command === "get_settings" ? 180 : 40);
  switch (command) {
    case "get_settings":
      return structuredClone(settings) as T;
    case "save_settings": {
      const next = args?.settings as Settings;
      Object.assign(settings, structuredClone(next));
      return undefined as T;
    }
    case "api_key_status":
      return keyStatus() as T;
    case "set_api_key": {
      const key = String(args?.key ?? "").trim();
      if (key.length < 20 || /\s/.test(key)) {
        throw new Error("That doesn't look like a Groq API key.");
      }
      apiKey = key;
      return keyStatus() as T;
    }
    case "clear_api_key":
      apiKey = null;
      return undefined as T;
    case "list_microphones":
      return structuredClone(mics) as T;
    case "get_history":
      return structuredClone(history) as T;
    case "delete_history_entry":
      history = history.filter((entry) => entry.id !== args?.id);
      return undefined as T;
    case "clear_history":
      history = [];
      return undefined as T;
    case "set_hotkey_capture":
      return undefined as T;
    case "toggle_test_recording":
      void runDemoTake();
      return undefined as T;
    case "get_app_info": {
      appInfoReads += 1;
      const late =
        typeof location !== "undefined" &&
        new URLSearchParams(location.search).get("listener") === "late";
      const listenerRunning = late && appInfoReads >= 8;
      const listenerError =
        late || listenerRunning
          ? null
          : "Global hook preview is Windows-only in this browser session.";
      return {
        version: "0.1.0",
        listenerRunning,
        listenerError,
        platform: "linux",
        lastError: previewParam("error"),
      } satisfies AppInfo as T;
    }
    case "mic_level":
      return (0.12 + 0.5 * Math.abs(Math.sin(Date.now() / 260))) as T;
    case "get_mic_status":
      return {
        preference: settings.microphone,
        activeName: settings.microphone ?? "Microphone Array",
        usingFallback: settings.microphone != null && !mics.some((mic) => mic.name === settings.microphone),
        message: null,
        devices: structuredClone(mics),
      } as T;
    case "dismiss_error":
      return undefined as T;
    case "copy_text":
      await navigator.clipboard.writeText(String(args?.text ?? ""));
      return undefined as T;
    case "open_groq_keys":
      window.open("https://console.groq.com/keys", "_blank", "noopener");
      return undefined as T;
    default:
      throw new Error(`Unknown command ${command}`);
  }
}

async function runDemoTake(): Promise<void> {
  if (recording) {
    recording = false;
    emitLocalDictation({
      phase: "transcribing",
      message: null,
      elapsedMs: 1800,
      level: 0,
    });
    await delay(700);
    const entry: HistoryEntry = {
      id: `demo-${Date.now()}`,
      text: "This is a preview transcript. On Windows it would be pasted at the cursor.",
      createdUnixMs: Date.now(),
      model: settings.model,
      cleaned: settings.cleanupEnabled,
      durationMs: 1800,
    };
    if (settings.historyEnabled) history = [entry, ...history].slice(0, 50);
    emitLocalDictation({
      phase: "done",
      message: "Inserted",
      elapsedMs: 1800,
      level: 0,
    });
    window.dispatchEvent(new CustomEvent("bolo-history"));
    await delay(900);
    emitLocalDictation({ phase: "idle", message: null, elapsedMs: 0, level: 0 });
    return;
  }
  recording = true;
  const started = Date.now();
  emitLocalDictation({ phase: "recording", message: null, elapsedMs: 0, level: 0.2 });
  const timer = window.setInterval(() => {
    if (!recording) {
      window.clearInterval(timer);
      return;
    }
    emitLocalDictation({
      phase: "recording",
      message: null,
      elapsedMs: Date.now() - started,
      level: 0.25 + Math.random() * 0.75,
    });
  }, 80);
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}
