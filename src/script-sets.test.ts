import { BUCKET_LABEL, SCRIPT_SETS, scriptSetId, sentenceCue } from "./script-sets.ts";

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

assert(SCRIPT_SETS[0]?.id === "original", "original set stays first");
assert(SCRIPT_SETS[1]?.id === "languages", "second set is selectable");
assert(scriptSetId("languages") === "languages", "languages id");
assert(scriptSetId("nope") === "original", "unknown set stays on the original 30");
assert(sentenceCue("ur", "Everyday Urdu.") === "Urdu · Everyday Urdu.", "bucket tag");
assert(sentenceCue("hi", "") === "Hindi", "hindi bucket");
assert(sentenceCue("mixed", "") === "Mixed", "mixed bucket");
assert(BUCKET_LABEL.en === "English", "english bucket");

console.log("script-sets ok");
