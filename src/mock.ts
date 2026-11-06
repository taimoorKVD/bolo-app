import { emitLocalDictation } from "./api";
import type {
  ApiKeyStatus,
  AppInfo,
  HistoryEntry,
  MicInfo,
  MemoryView,
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
  dictionary: ["Bolo", "Groq"],
  launchAtStartup: false,
  historyEnabled: true,
  pasteHotkey: { ctrl: true, alt: false, shift: false, win: true, key: "v" },
  offlineModel: "base.en",
  output: "english",
  verbatim: false,
  cleanupModel: "openai/gpt-oss-20b",
  character: "classic",
  soundPack: "match",
  soundVolume: 0.6,
  smartMood: false,
  smartTimeOfDay: false,
  smartHoliday: false,
  keepLearning: true,
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
    audioId: "spool-1",
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

const SEEDS: Array<{ word: string; uses: number }> = [
  { word: "Bolo", uses: 12 },
  { word: "taimoorhussain.com", uses: 8 },
  { word: "Kingdom Vision", uses: 0 },
  { word: "Bolo.com", uses: 0 },
  { word: "Taimoor Hussain", uses: 1 },
  { word: "thebetawebsite.com", uses: 0 },
];

const memory: MemoryView = {
  words: ["Taimoor", "Kingdom Vision", "Lahore", "Groq"],
  fixes: [
    { from: "tie more", to: "Taimoor", createdUnixMs: Date.now() - 3600_000 },
    { from: "bolo dot com", to: "Bolo.com", createdUnixMs: Date.now() - 86_400_000 },
  ],
  style: [
    "Send the invoice to Ahmed by Friday, thanks.",
    "Kal meeting 3 baje rakh lo, theek hai?",
  ],
  keepLearning: true,
  myWords: ["Priya"],
};

function isSeed(word: string): boolean {
  return SEEDS.some((seed) => seed.word.toLowerCase() === word.toLowerCase());
}

function memoryView(): MemoryView {
  memory.keepLearning = settings.keepLearning;
  memory.words = settings.dictionary.length ? [...settings.dictionary] : memory.words;
  const mine = (memory.myWords ?? []).filter((word) => !isSeed(word));
  const rows = [
    ...SEEDS.map((seed) => ({ word: seed.word, uses: seed.uses, seed: true })),
    ...mine.map((word) => ({ word, uses: 1, seed: false })),
  ].sort((a, b) => b.uses - a.uses || a.word.localeCompare(b.word));
  memory.entries = rows.map((row) => ({
    ...row,
    tag: row.uses >= 3 ? "Used a lot" : "Ready",
  }));
  return structuredClone(memory);
}

const mics: MicInfo[] = [
  { name: "Microphone Array", isDefault: true },
  { name: "Jack Mic (2- Realtek(R) Audio)", isDefault: false },
];

function builtinPreview(): boolean {
  const flag = previewParam("builtin");
  return flag !== "0" && flag !== "false";
}

function keyStatus(): ApiKeyStatus {
  const builtin = builtinPreview();
  if (!apiKey) return { configured: false, hint: null, error: null, builtin };
  return { configured: true, hint: `····${apiKey.slice(-4)}`, error: null, builtin };
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
    case "home_card":
    case "rating_card":
      return null as T;
    case "dismiss_home_milestone":
    case "copy_share_message":
    case "open_share_whatsapp":
    case "save_character_card":
      return false as T;
    case "rate_bolo":
      return { status: "opened", message: "" } as T;
    case "rating_not_now":
    case "rating_feedback":
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
        version: "0.1.9",
        listenerRunning,
        listenerError,
        platform: "linux",
        lastError: previewParam("error"),
      } satisfies AppInfo as T;
    }
    case "audio_health":
    case "fix_mic_level":
    case "fix_output_volume":
      return { micLevel: 0.8, micMuted: false, micQuiet: false, outputLevel: 0.5, outputMuted: false, outputQuiet: false } as T;
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
    case "get_memory":
      return memoryView() as T;
    case "forget_word": {
      const word = String(args?.word ?? "").trim();
      if (isSeed(word)) throw new Error("Bolo always spells that one.");
      memory.myWords = (memory.myWords ?? []).filter((w) => w.toLowerCase() !== word.toLowerCase());
      settings.dictionary = memoryView().words.filter((w) => w.toLowerCase() !== word.toLowerCase());
      memory.words = settings.dictionary;
      return memoryView() as T;
    }
    case "add_word": {
      const w = String(args?.word ?? "").trim();
      if (!w) throw new Error("Type a word first.");
      if (isSeed(w)) throw new Error("Bolo already spells that one.");
      if (w.split(/\s+/).length > 4) throw new Error("Use a name or a site.");
      memory.myWords = [w, ...(memory.myWords ?? []).filter((have) => have.toLowerCase() !== w.toLowerCase())];
      settings.dictionary = [...memoryView().words.filter((x) => x.toLowerCase() !== w.toLowerCase()), w];
      return memoryView() as T;
    }
    case "forget_fix":
      memory.fixes = memory.fixes.filter((f) => f.from !== args?.from);
      return memoryView() as T;
    case "forget_style":
      memory.style.splice(Number(args?.index), 1);
      return memoryView() as T;
    case "set_keep_learning":
      settings.keepLearning = Boolean(args?.on);
      return memoryView() as T;
    case "forget_everything":
      settings.dictionary = [];
      memory.words = [];
      memory.myWords = [];
      memory.fixes = [];
      memory.style = [];
      return memoryView() as T;
    case "fix_target": {
      const id = (args?.id as string | null) ?? history[0]?.id ?? "";
      const entry = history.find((h) => h.id === id);
      if (!entry) throw new Error("There's no take to fix yet.");
      return { id, text: entry.text, canRelisten: Boolean(entry.audioId), isLast: id === history[0]?.id } as T;
    }
    case "take_fix_request":
      return previewParam("fix") as T;
    case "fix_requested":
      return undefined as T;
    case "fix_relisten":
      await delay(900);
      return { text: "Please ask Taimoor about it.", message: "Listened again and replaced the text.", learned: [] } as T;
    case "fix_with_text": {
      const text = String(args?.text ?? "");
      const entry = history.find((h) => h.id === args?.id);
      if (entry) entry.text = text;
      return { text, message: "Fixed. Bolo will remember this.", learned: ["tie more → Taimoor"] } as T;
    }
    case "overlay_fix":
      return undefined as T;
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
      engine: "Groq",
    });
    window.dispatchEvent(new CustomEvent("bolo-history"));
    await delay(900);
    emitLocalDictation({ phase: "idle", message: null, elapsedMs: 0, level: 0 });
    return;
  }
  recording = true;
  const started = Date.now();
  emitLocalDictation({ phase: "recording", message: null, elapsedMs: 0, level: 0.35 });
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
