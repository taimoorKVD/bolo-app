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

export function offlineModelNote(language: string, model: string): string {
  const choice = OFFLINE_MODELS.find((item) => item.id === knownOfflineModel(model));
  if (choice && !choice.englishOnly && choice.id !== "off") {
    return `${choice.label}. Bolo downloads it the first time Groq can't take a request, and checks the file hash before saving it.`;
  }
  if (needsMultilingualOffline(language) && choice?.englishOnly) {
    return `${URDU_OFFLINE_NOTICE} The English on-device model is not used for that speech. Pick ggml-base or ggml-small to download one. If you only dictate in English, set I speak to English.`;
  }
  return "If Groq is rate limiting, down, or unreachable, Bolo transcribes this take on the PC with whisper.cpp and tries Groq again on the next one. The model downloads the first time it's needed.";
}
