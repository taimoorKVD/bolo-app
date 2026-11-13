import { emitLocalDictation } from "./api";
import type {
  ApiKeyStatus,
  AppInfo,
  HistoryEntry,
  MicInfo,
  MemoryView,
  ProviderKeyRow,
  ProviderKeyStatus,
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
  model: "whisper-large-v3",
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
  listener: previewParam("listener") === "classic" ? "classic" : "blob",
  soundPack: "match",
  soundVolume: 0.6,
  smartMood: false,
  smartTimeOfDay: false,
  smartHoliday: false,
  keepLearning: true,
  keepRecordings: false,
  debugCapture: false,
};

let apiKey: string | null = null;
let recording = false;
let appInfoReads = 0;
let history: HistoryEntry[] = [
  {
    id: "sample-1",
    text: "Move the standup to Thursday and send the notes to Priya.",
    createdUnixMs: Date.now() - 1000 * 60 * 42,
    model: "whisper-large-v3",
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

const ORDINARY = new Set(["the", "a", "to", "there", "their", "easy", "invoice", "deadline", "roadmap", "that"]);

const backupKeys: ProviderKeyRow[] = [];

function providerStatus(): ProviderKeyStatus {
  const keys: ProviderKeyRow[] = [];
  if (apiKey) {
    keys.push({ id: "default", hint: `····${apiKey.slice(-4)}`, state: "ok", kind: "user", preferred: true, lastError: null });
  }
  keys.push(...backupKeys);
  if (builtinPreview()) {
    keys.push({ id: "builtin", hint: null, state: "ok", kind: "builtin", preferred: !apiKey, lastError: null });
  }
  return { keys, note: null };
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
    case "provider_keys":
      return providerStatus() as T;
    case "save_provider_key": {
      const key = String(args?.key ?? "").trim();
      const id = String(args?.id ?? "new");
      if (key.length < 20 || /\s/.test(key)) throw new Error("That doesn't look like a Groq API key.");
      if (key.includes("reject")) throw new Error("Groq did not accept that key. Check it and paste it again.");
      if (id === "default") apiKey = key;
      else {
        const slot = id === "new" ? `backup-${backupKeys.length + 1}` : id;
        if (backupKeys.length >= 3 && id === "new") throw new Error("You already have three backup keys. Remove one to add another.");
        const row: ProviderKeyRow = { id: slot, hint: `····${key.slice(-4)}`, state: "ok", kind: "user", preferred: false, lastError: null };
        const index = backupKeys.findIndex((item) => item.id === slot);
        if (index >= 0) backupKeys[index] = row;
        else backupKeys.push(row);
      }
      return { ...providerStatus(), note: null } as T;
    }
    case "revoke_provider_key": {
      const id = String(args?.id ?? "");
      if (id === "builtin") throw new Error("That one belongs to Bolo. Remove a key you added.");
      if (id === "default") apiKey = null;
      else {
        const index = backupKeys.findIndex((item) => item.id === id);
        if (index >= 0) backupKeys.splice(index, 1);
      }
      return providerStatus() as T;
    }
    case "teach_spelling": {
      const from = String(args?.from ?? "").trim();
      const to = String(args?.to ?? "").trim();
      if (!from || !to) throw new Error("Type what Bolo heard, and what you meant.");
      if (from.toLowerCase() === to.toLowerCase()) {
        throw new Error("Those are the same word. A capital letter does not make an ordinary word into a name.");
      }
      const fromWord = from.split(/\s+/);
      if (fromWord.length === 1 && ORDINARY.has(fromWord[0].toLowerCase())) {
        throw new Error("That would change an ordinary word in every sentence. Teach a short phrase instead, such as “easy ssh” to EasySSH.");
      }
      memory.fixes = [{ from, to, createdUnixMs: Date.now() }, ...memory.fixes.filter((fix) => fix.from.toLowerCase() !== from.toLowerCase())];
      return memoryView() as T;
    }
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
        version: "0.1.20",
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
    case "open_debug_folder":
    case "clear_debug_folder":
    case "accuracy_record_discard":
    case "accuracy_open_folder":
    case "accuracy_clear_folder":
      return undefined as T;
    case "accuracy_script": {
      const set = String(args?.set ?? "original");
      if (set === "languages") {
        return [
          {
            id: "sa-01-weather",
            kind: "urdu",
            expectedLanguage: "ur",
            bucket: "ur",
            cue: "Everyday Urdu.",
            say: "آج موسم بہت اچھا ہے۔",
          },
        ] as T;
      }
      return [
        {
          id: "01-bolo",
          kind: "names",
          expectedLanguage: "en",
          bucket: "en",
          cue: "Say this sentence.",
          say: "Open Bolo and start dictating.",
        },
      ] as T;
    }
    case "accuracy_record_start":
      if (!String(args?.id ?? "").trim()) throw new Error("Choose a sentence first.");
      return undefined as T;
    case "accuracy_record_stop":
      return {
        deviceName: "Preview mic",
        sampleRate: 48000,
        channels: 1,
        holdMs: 1200,
        peak: 0.2,
        clippingPct: 0,
        silent: false,
      } as T;
    case "accuracy_save":
      return "preview" as T;
    case "accuracy_saved":
      return [] as T;
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
