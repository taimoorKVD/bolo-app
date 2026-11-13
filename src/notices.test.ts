import { listenerNotice, statusBanner, visibleMessage } from "./notices.ts";

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

assert(visibleMessage(null) === null, "null is not a message");
assert(visibleMessage(undefined) === null, "undefined is not a message");
assert(visibleMessage("") === null, "empty string is not a message");
assert(visibleMessage("  \n") === null, "whitespace is not a message");
assert(visibleMessage(" hook failed ") === "hook failed", "message is trimmed");

assert(listenerNotice(true, null) === null, "a running hook hides the banner");
assert(listenerNotice(true, "stale") === null, "a running hook hides a leftover error");
assert(
  listenerNotice(false, null) ===
    "Global hotkey isn't active yet. You can still record a test below.",
  "waiting copy while the hook has not reported in",
);
assert(
  listenerNotice(false, "   ") ===
    "Global hotkey isn't active yet. You can still record a test below.",
  "a blank error is still the waiting copy",
);
assert(
  listenerNotice(false, "couldn't install the keyboard hook") ===
    "Global hotkey isn't active. couldn't install the keyboard hook",
  "a real hook error is shown",
);

assert(statusBanner("typed", "Typed") === "Inserted", "legacy Typed becomes Inserted");
assert(statusBanner("typed", " typed ") === "Inserted", "a repeated Typed status collapses");
assert(statusBanner("typed", null) === null, "a bare Typed phase is not a row");
assert(
  statusBanner("typed", "Part of this take didn't transcribe.") ===
    "Inserted — Part of this take didn't transcribe.",
  "a typed take with a real note still explains it",
);
assert(statusBanner("recording", null) === "Listening", "listening stays a single label");
assert(statusBanner("done", "Inserted") === "Inserted", "Inserted — Inserted collapses");

console.log("notices ok");
