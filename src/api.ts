import type {
  ApiKeyStatus,
  ProviderKeyStatus,
  AppInfo,
  DictationEvent,
  HistoryEntry,
  ListenerStatus,
  MicInfo,
  MicStatus,
  AudioHealth,
  Settings,
  MemoryView,
  FixTarget,
  FixResult,
  HomeCard,
  RatingCard,
  RateResponse,
  ScriptLine,
  HeldInfo,
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

export async function onMicStatus(handler: (status: MicStatus) => void): Promise<() => void> {
  if (!inTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<MicStatus>("mic-status", (event) => handler(event.payload));
}

export async function onListenerStatus(
  handler: (status: ListenerStatus) => void,
): Promise<() => void> {
  if (!inTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<ListenerStatus>("listener-status", (event) => handler(event.payload));
}

export async function onDictation(handler: Handler): Promise<() => void> {
  if (!inTauri()) {
    localHandlers.add(handler);
    return () => localHandlers.delete(handler);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<DictationEvent>("dictation-state", (event) => handler(event.payload));
}

export async function onOpenFix(handler: (id: string) => void): Promise<() => void> {
  if (!inTauri()) {
    const fn = (e: Event) => handler(String((e as CustomEvent).detail ?? ""));
    window.addEventListener("bolo-open-fix", fn);
    return () => window.removeEventListener("bolo-open-fix", fn);
  }
  const { listen } = await import("@tauri-apps/api/event");
  return listen<string>("bolo-open-fix", (event) => handler(event.payload ?? ""));
}

export async function onHistoryChanged(handler: () => void): Promise<() => void> {
  if (!inTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen("bolo-history", () => handler());
}

export const api = {
  getSettings: () => call<Settings>("get_settings"),
  saveSettings: (settings: Settings) => call<void>("save_settings", { settings }),
  apiKeyStatus: () => call<ApiKeyStatus>("api_key_status"),
  setApiKey: (key: string) => call<ApiKeyStatus>("set_api_key", { key }),
  clearApiKey: () => call<void>("clear_api_key"),
  providerKeys: () => call<ProviderKeyStatus>("provider_keys"),
  saveProviderKey: (id: string, key: string) => call<ProviderKeyStatus>("save_provider_key", { id, key }),
  revokeProviderKey: (id: string) => call<ProviderKeyStatus>("revoke_provider_key", { id }),
  teachSpelling: (from: string, to: string) => call<MemoryView>("teach_spelling", { from, to }),
  listMicrophones: () => call<MicInfo[]>("list_microphones"),
  getMicStatus: () => call<MicStatus>("get_mic_status"),
  micLevel: () => call<number>("mic_level"),
  audioHealth: () => call<AudioHealth>("audio_health"),
  fixMicLevel: () => call<AudioHealth>("fix_mic_level"),
  fixOutputVolume: () => call<AudioHealth>("fix_output_volume"),
  dismissError: () => call<void>("dismiss_error"),
  getHistory: () => call<HistoryEntry[]>("get_history"),
  deleteHistoryEntry: (id: string) => call<void>("delete_history_entry", { id }),
  clearHistory: () => call<void>("clear_history"),
  setHotkeyCapture: (paused: boolean) => call<void>("set_hotkey_capture", { paused }),
  toggleTestRecording: () => call<void>("toggle_test_recording"),
  getAppInfo: () => call<AppInfo>("get_app_info"),
  copyText: (text: string) => call<void>("copy_text", { text }),
  openGroqKeys: () => call<void>("open_groq_keys"),
  getMemory: () => call<MemoryView>("get_memory"),
  forgetWord: (word: string) => call<MemoryView>("forget_word", { word }),
  addWord: (word: string) => call<MemoryView>("add_word", { word }),
  forgetFix: (from: string) => call<MemoryView>("forget_fix", { from }),
  forgetStyle: (index: number) => call<MemoryView>("forget_style", { index }),
  setKeepLearning: (on: boolean) => call<MemoryView>("set_keep_learning", { on }),
  forgetEverything: () => call<MemoryView>("forget_everything"),
  fixTarget: (id: string | null) => call<FixTarget>("fix_target", { id }),
  fixRequested: () => call<void>("fix_requested"),
  takeFixRequest: () => call<string | null>("take_fix_request"),
  fixRelisten: (id: string | null) => call<FixResult>("fix_relisten", { id }),
  fixWithText: (id: string | null, text: string) => call<FixResult>("fix_with_text", { id, text }),
  overlayFix: (action: "relisten" | "type" | "dismiss") => call<void>("overlay_fix", { action }),
  homeCard: () => call<HomeCard | null>("home_card"),
  dismissMilestone: (count: number) => call<void>("dismiss_home_milestone", { count }),
  copyShareMessage: () => call<void>("copy_share_message"),
  openShareWhatsapp: () => call<void>("open_share_whatsapp"),
  saveCharacterCard: (bytes: number[]) => call<boolean>("save_character_card", { bytes }),
  ratingCard: () => call<RatingCard | null>("rating_card"),
  rateBolo: () => call<RateResponse>("rate_bolo"),
  ratingNotNow: () => call<void>("rating_not_now"),
  ratingFeedback: () => call<void>("rating_feedback"),
  smokeFinished: (ok: boolean, detail: string | null) =>
    call<void>("smoke_finished", { ok, detail }),
  openDebugFolder: () => call<void>("open_debug_folder"),
  clearDebugFolder: () => call<void>("clear_debug_folder"),
  accuracyScript: (set?: string) => call<ScriptLine[]>("accuracy_script", { set: set ?? "original" }),
  accuracyRecordStart: () => call<void>("accuracy_record_start"),
  accuracyRecordStop: () => call<HeldInfo>("accuracy_record_stop"),
  accuracyDiscard: () => call<void>("accuracy_record_discard"),
  accuracySave: (id: string) => call<string>("accuracy_save", { id }),
  accuracySaved: () => call<string[]>("accuracy_saved"),
  accuracyOpenFolder: () => call<void>("accuracy_open_folder"),
  accuracyClearFolder: () => call<void>("accuracy_clear_folder"),
};
