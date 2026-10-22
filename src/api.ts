import type {
  ApiKeyStatus,
  AppInfo,
  DictationEvent,
  HistoryEntry,
  MicInfo,
  Settings,
} from "./types";

type Handler = (event: DictationEvent) => void;
const localHandlers = new Set<Handler>();

export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri()) {
    const { mockInvoke } = await import("./mock");
    return mockInvoke<T>(command, args);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

export function emitLocalDictation(event: DictationEvent): void {
  for (const handler of localHandlers) handler(event);
}

export async function onDictation(handler: Handler): Promise<() => void> {
  if (!inTauri()) {
    localHandlers.add(handler);
    return () => localHandlers.delete(handler);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<DictationEvent>("dictation-state", (event) => handler(event.payload));
}

export const api = {
  getSettings: () => call<Settings>("get_settings"),
  saveSettings: (settings: Settings) => call<void>("save_settings", { settings }),
  apiKeyStatus: () => call<ApiKeyStatus>("api_key_status"),
  setApiKey: (key: string) => call<ApiKeyStatus>("set_api_key", { key }),
  clearApiKey: () => call<void>("clear_api_key"),
  listMicrophones: () => call<MicInfo[]>("list_microphones"),
  getHistory: () => call<HistoryEntry[]>("get_history"),
  deleteHistoryEntry: (id: string) => call<void>("delete_history_entry", { id }),
  clearHistory: () => call<void>("clear_history"),
  setHotkeyCapture: (paused: boolean) => call<void>("set_hotkey_capture", { paused }),
  toggleTestRecording: () => call<void>("toggle_test_recording"),
  getAppInfo: () => call<AppInfo>("get_app_info"),
  copyText: (text: string) => call<void>("copy_text", { text }),
  openGroqKeys: () => call<void>("open_groq_keys"),
};
