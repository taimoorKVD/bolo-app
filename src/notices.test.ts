import { listenerNotice, visibleMessage } from "./notices.ts";

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

console.log("notices ok");
