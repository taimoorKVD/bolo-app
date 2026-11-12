export type Hotkey = {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  win: boolean;
  key: string | null;
};

export type WhisperModel = "whisper-large-v3-turbo" | "whisper-large-v3";

export type Settings = {
  hotkey: Hotkey;
  microphone: string | null;
  model: WhisperModel;
  cleanupEnabled: boolean;
  language: string;
  dictionary: string[];
  launchAtStartup: boolean;
  historyEnabled: boolean;
  pasteHotkey: Hotkey;
  offlineModel: "base.en" | "small.en" | "off" | string;
  /** `english` translates into English. `same` keeps the spoken language. */
  output: "english" | "same";
  /** Paste exactly what was said. Cleanup does not rewrite. */
  verbatim: boolean;
  /** Groq chat model for cleanup and mixed-language translation. */
  cleanupModel: string;
  /** Overlay character id. `classic` is the plush listener. Used by the classic overlay. */
  character: string;
  /** `blob` is the animated Bolo Blob. `classic` keeps the clip overlay. */
  listener: string;
  /** Sound pack id, `match` (follow the character) or `off`. */
  soundPack: string;
  /** 0..1 */
  soundVolume: number;
  smartMood: boolean;
  smartTimeOfDay: boolean;
  smartHoliday: boolean;
  /** Learn names, fixes and style on this PC. */
  keepLearning: boolean;
  /** Keep the last few recordings on this PC so Listen again can play them. */
  keepRecordings: boolean;
  /** Save each take's audio and stage text on this PC. Off unless chosen. */
  debugCapture: boolean;
};

export type ScriptLine = {
  id: string;
  kind: string;
  expectedLanguage: string;
  /** `en`, `ur`, `mixed`, or `hi`. */
  bucket?: string;
  cue: string;
  say: string;
};

export type HeldInfo = {
  deviceName: string;
  sampleRate: number;
  channels: number;
  holdMs: number;
  peak: number;
  clippingPct: number;
  silent: boolean;
};

export type ProviderKeyRow = {
  id: string;
  hint: string | null;
  state: string;
  kind: string;
  preferred: boolean;
  lastError: string | null;
};

export type ProviderKeyStatus = {
  keys: ProviderKeyRow[];
  note: string | null;
};

export type ApiKeyStatus = {
  configured: boolean;
  hint: string | null;
  error: string | null;
  /** This build has a compiled-in key. The secret is never included. */
  builtin: boolean;
};

export type MicInfo = {
  name: string;
  isDefault: boolean;
};

export type HistoryEntry = {
  id: string;
  text: string;
  createdUnixMs: number;
  model: string;
  cleaned: boolean;
  durationMs: number;
  /** Spool id of the saved audio, when it is still kept. */
  audioId?: string;
};

export type ListenerStatus = {
  running: boolean;
  error: string | null;
};

export type HomeCard =
  | {
      kind: "rating";
      title: string;
      body: string;
      rateLabel: string;
      laterLabel: string;
      feedbackLabel: string;
      rtl: boolean;
    }
  | {
      kind: "milestone";
      count: number;
      title: string;
      body: string;
    };

export type RatingCard = {
  title: string;
  body: string;
  rateLabel: string;
  laterLabel: string;
  feedbackLabel: string;
  rtl: boolean;
};

export type RateResponse = {
  /** rated, later, opened, retry, or failed */
  status: string;
  message: string;
};

export type AppInfo = {
  version: string;
  listenerRunning: boolean;
  listenerError: string | null;
  platform: string;
  lastError: string | null;
};

export type MicStatus = {
  preference: string | null;
  activeName: string | null;
  usingFallback: boolean;
  message: string | null;
  devices: MicInfo[];
};

export type DictationPhase =
  | "idle"
  | "arming"
  | "recording"
  | "transcribing"
  | "inserting"
  | "done"
  | "typed"
  | "fix"
  | "error";

export type DictationEvent = {
  phase: DictationPhase;
  message: string | null;
  elapsedMs: number;
  level: number;
  engine?: string | null;
  /** 80–300, 300–1100, 1100–3200, 3200–8000 Hz, each 0..1. Present while recording. */
  bands?: number[];
};

export interface AudioHealth {
  micLevel: number | null;
  micMuted: boolean;
  micQuiet: boolean;
  outputLevel: number | null;
  outputMuted: boolean;
  outputQuiet: boolean;
}

export type Correction = { from: string; to: string; createdUnixMs: number };

export type MyWordEntry = {
  word: string;
  uses: number;
  seed: boolean;
  tag: string;
};

export type MemoryView = {
  words: string[];
  fixes: Correction[];
  style: string[];
  keepLearning: boolean;
  myWords?: string[];
  entries?: MyWordEntry[];
};

export type FixTarget = { id: string; text: string; canRelisten: boolean; isLast: boolean };

export type FixResult = { text: string; message: string; learned: string[] };
