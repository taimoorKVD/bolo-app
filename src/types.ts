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
};

export type ApiKeyStatus = {
  configured: boolean;
  hint: string | null;
  error: string | null;
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

export type DictationPhase = "idle" | "recording" | "transcribing" | "done" | "error";

export type DictationEvent = {
  phase: DictationPhase;
  message: string | null;
  elapsedMs: number;
  level: number;
};
