/** Keep this sentence identical to `offline::URDU_OFFLINE_NOTICE`. */
export const URDU_OFFLINE_NOTICE =
  "Urdu needs the online speech service, or download the multilingual offline model.";

export type OfflineChoice = {
  id: string;
  label: string;
  /** English-only whisper.cpp weights. They cannot write Urdu or Hindi. */
  englishOnly: boolean;
};

/** Sizes match the published ggml files (148 MB and 488 MB, decimal). */
export const OFFLINE_MODELS: OfflineChoice[] = [
  { id: "base.en", label: "base.en — smaller English model", englishOnly: true },
  { id: "small.en", label: "small.en — larger English model", englishOnly: true },
  { id: "ggml-base", label: "ggml-base — multilingual, 148 MB", englishOnly: false },
  { id: "ggml-small", label: "ggml-small — multilingual, 488 MB", englishOnly: false },
  { id: "off", label: "Off — Groq only", englishOnly: false },
];

export function knownOfflineModel(model: string | null | undefined): string {
  if (model && OFFLINE_MODELS.some((item) => item.id === model)) return model;
  return "base.en";
}

/** Auto is the mixed Urdu/English setting. Pin English to keep an English-only model. */
export function needsMultilingualOffline(language: string | null | undefined): boolean {
  const code = (language ?? "").trim().toLowerCase();
  return code === "ur" || code === "hi" || code === "auto" || code === "mixed";
}

export function offlineModelNote(_language: string, model: string): string {
  const choice = OFFLINE_MODELS.find((item) => item.id === knownOfflineModel(model));
  if (choice?.id === "off") {
    return "Bolo uses the online speech service for every take.";
  }
  if (choice && !choice.englishOnly) {
    return `${choice.label}. Bolo downloads it when needed.`;
  }
  return "If the online service is busy, Bolo transcribes on this PC and downloads what it needs in the background.";
}
