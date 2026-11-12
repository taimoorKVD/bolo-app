import { listenerIsAnimated, normalizeListener } from "./listener-style.ts";

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

assert(normalizeListener(undefined) === "blob", "missing setting is Bolo Blob");
assert(normalizeListener(null) === "blob", "null setting is Bolo Blob");
assert(normalizeListener("") === "blob", "blank setting is Bolo Blob");
assert(normalizeListener("BLOB") === "blob", "blob is accepted in any case");
assert(normalizeListener(" classic ") === "classic", "classic is the explicit fallback");
assert(normalizeListener("buddy") === "blob", "characters not shipped yet stay on Bolo Blob");
assert(listenerIsAnimated("blob") === true, "blob is the animated overlay");
assert(listenerIsAnimated("classic") === false, "classic keeps the clip overlay");
assert(listenerIsAnimated("nope") === true, "an unknown id still animates, as Bolo Blob");

console.log("listener-style ok");
