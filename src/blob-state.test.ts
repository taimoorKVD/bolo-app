import {
  characterState,
  dprFor,
  nextQuality,
  liveCaption,
  pillModel,
  poseImpulse,
  tailWords,
  publicProblem,
  shapeVoice,
  wantsFrames,
} from "./blob-state.ts";
import type { DictationPhase } from "./types.ts";

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

const phases: DictationPhase[] = [
  "idle",
  "arming",
  "recording",
  "transcribing",
  "inserting",
  "done",
  "typed",
  "fix",
  "error",
];

const expected: Record<DictationPhase, string> = {
  idle: "hidden",
  arming: "listening",
  recording: "listening",
  transcribing: "thinking",
  inserting: "thinking",
  done: "done",
  typed: "done",
  error: "error",
  fix: "fix",
};

for (const phase of phases) {
  assert(characterState(phase) === expected[phase], `${phase} maps to ${expected[phase]}`);
}
assert(characterState("nope") === "hidden", "unknown phases stay hidden");
assert(wantsFrames("listening") && wantsFrames("thinking") && wantsFrames("done") && wantsFrames("error"), "visible poses render");
assert(!wantsFrames("hidden"), "hidden draws no frames");
assert(!wantsFrames("fix"), "the fix card is DOM, not a shader frame");

const open = shapeVoice(0.7, [0.1, 0.9, 0.2, 0.05]);
const round = shapeVoice(0.7, [0.9, 0.25, 0.05, 0.02]);
assert(open.lv === 0.7, "loudness is kept");
assert(open.vo > round.vo, "the mid band opens the vowel");
assert(round.vr > open.vr, "the low band rounds the mouth");
assert(open.cl === 0, "a loud frame does not close the mouth");

const quiet = shapeVoice(0.02, [0.4, 0.4, 0.4, 0.4]);
assert(quiet.cl === 1, "a quiet mic closes the mouth");

const levelOnly = shapeVoice(0.55, undefined);
assert(levelOnly.lv === 0.55, "loudness still drives the mouth without a spectrum");
assert(levelOnly.bands[1] > levelOnly.bands[3], "a level-only mouth is a neutral vowel, not a hiss");
assert(shapeVoice(Number.NaN, [2, -1, 0, 0]).lv === 0, "bad levels clamp");

const listening = pillModel("recording", null);
assert(listening.line === "Listening" && listening.showLine, "listening names its state");
const hands = pillModel("recording", "Hands-free — tap the hotkey to stop");
assert(hands.line.includes("Hands-free"), "a live caption replaces the generic label");
const heard = pillModel("recording", null, "ship the notes to Sam");
assert(heard.line === "ship the notes to Sam" && heard.showLine, "spoken words replace Listening");
const heardOverHands = pillModel("recording", "Hands-free — tap the hotkey to stop", "ship the notes");
assert(heardOverHands.line === "ship the notes", "words win over the hands-free caption");
assert(pillModel("recording", null, "   ").line === "Listening", "blank preview stays Listening");
const writing = pillModel("transcribing", null);
assert(writing.status === "Writing…" && writing.showStatus, "transcribing is the thinking pose");
const writingWords = pillModel("transcribing", null, "ship the notes to Sam");
assert(writingWords.line === "ship the notes to Sam" && writingWords.showLine, "words stay up while the take is written");
assert(liveCaption("recording", null, "ship the notes to Sam") === "ship the notes to Sam", "classic shows the same words");
assert(liveCaption("recording", null, null) === "", "classic stays quiet until words arrive");
assert(liveCaption("idle", null, "leftover") === "", "a hidden overlay does not keep the caption");
const long =
  "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen";
assert(tailWords(long).startsWith("…") && tailWords(long).endsWith("sixteen"), "a long caption keeps the newest words");
for (const phase of ["recording", "arming", "transcribing", "inserting"]) {
  assert(liveCaption(phase, null, "ship it").includes("ship it"), `${phase} shows the words`);
}
assert(pillModel("inserting", null).status === "Inserting…", "inserting stays distinct");
assert(pillModel("done", "Inserted").status === "Inserted", "done uses the app's word");
assert(pillModel("typed", null).status === "Inserted", "typed delivery still shows Inserted with the check");
assert(pillModel("done", null).status === "Inserted", "a bare done still says Inserted");

const missed = pillModel("error", "Didn't catch any speech. Speak a little louder and try again.");
assert(missed.status === "Didn't catch that", "no speech uses the error pose");
assert(missed.detail.includes("Hold the key"), "no speech tells the user what to do");
const groq = pillModel("error", "Add a Groq API key in Settings.");
assert(!groq.status.includes("Groq") && !groq.detail.includes("Groq"), "the engine name stays off the overlay");
assert(publicProblem("Groq failed") === "the service failed", "Groq is rewritten");
assert(pillModel("error", "Hold the hotkey while you talk, or double-tap it for hands-free.").status === "Hold the key", "a short press is a hint, not a crash");
assert(pillModel("idle", "leftover").showLine === false, "idle has no pill");

assert(poseImpulse("done", false).hop, "done hops");
assert(!poseImpulse("done", true).hop, "reduced motion does not hop");
assert(poseImpulse("error", false).shrug && !poseImpulse("error", true).shrug, "reduced motion does not shrug");

let budget = nextQuality("full", 4, 0, 0);
assert(budget.quality === "full", "a fast frame stays on the full path");
budget = { quality: "full", slow: 0, fast: 0 };
for (let i = 0; i < 8; i++) budget = nextQuality(budget.quality, 20, budget.slow, budget.fast);
assert(budget.quality === "low", "eight slow frames drop quality");
budget = { quality: "low", slow: 0, fast: 0 };
for (let i = 0; i < 44; i++) budget = nextQuality(budget.quality, 4, budget.slow, budget.fast);
assert(budget.quality === "low", "quality does not bounce back immediately");
budget = nextQuality(budget.quality, 4, budget.slow, budget.fast);
assert(budget.quality === "full", "a long fast run restores quality");

assert(dprFor("full", 2) === 1.25, "full quality caps pixel ratio");
assert(dprFor("low", 2) === 1, "the cheap path is 1x");
assert(dprFor("full", 1) === 1, "a 1x display stays 1x");
assert(dprFor("full", Number.NaN) === 1, "a missing ratio is 1x");

console.log("blob-state ok");
