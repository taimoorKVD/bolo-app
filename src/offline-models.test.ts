import {
  URDU_OFFLINE_NOTICE,
  knownOfflineModel,
  needsMultilingualOffline,
  offlineModelNote,
  OFFLINE_MODELS,
} from "./offline-models.ts";

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

assert(
  URDU_OFFLINE_NOTICE ===
    "Urdu needs the online speech service, or download the multilingual offline model.",
  "notice text",
);
assert(needsMultilingualOffline("ur"), "urdu");
assert(needsMultilingualOffline("hi"), "hindi");
assert(needsMultilingualOffline("auto"), "mixed auto");
assert(needsMultilingualOffline("mixed"), "mixed");
assert(!needsMultilingualOffline("en"), "english keeps the english model");
assert(knownOfflineModel("ggml-base") === "ggml-base", "multilingual id is kept");
assert(knownOfflineModel("nope") === "base.en", "unknown falls back");

const base = OFFLINE_MODELS.find((item) => item.id === "ggml-base");
const small = OFFLINE_MODELS.find((item) => item.id === "ggml-small");
assert(base?.label.includes("148 MB") === true, "base size");
assert(small?.label.includes("488 MB") === true, "small size");
assert(base?.englishOnly === false && small?.englishOnly === false, "multilingual");

const urduNote = offlineModelNote("ur", "base.en");
assert(urduNote.includes(URDU_OFFLINE_NOTICE) === false, "settings do not tell the user to download a model");
assert(!urduNote.toLowerCase().includes("whisper"), "no engine names in the note");
assert(offlineModelNote("en", "base.en").includes("downloads what it needs"), "english fallback stays plain");
assert(offlineModelNote("hi", "ggml-small").includes("488 MB"), "chosen model shows its size");
assert(offlineModelNote("auto", "ggml-base").includes("downloads it when needed"), "multilingual downloads itself");

console.log("offline-models ok");
