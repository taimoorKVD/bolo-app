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
  /** Overlay character id. `classic` is the plush listener. */
  character: string;
  /** Sound pack id, `match` (follow the character) or `off`. */
  soundPack: string;
  /** 0..1 */
  soundVolume: number;
  smartMood: boolean;
  smartTimeOfDay: boolean;
  smartHoliday: boolean;
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
};

export type ListenerStatus = {
  running: boolean;
  error: string | null;
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
  | "error";

export type DictationEvent = {
  phase: DictationPhase;
  message: string | null;
  elapsedMs: number;
  level: number;
  engine?: string | null;
};

export interface AudioHealth {
  micLevel: number | null;
  micMuted: boolean;
  micQuiet: boolean;
  outputLevel: number | null;
  outputMuted: boolean;
  outputQuiet: boolean;
}
