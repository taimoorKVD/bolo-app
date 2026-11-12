import { listenerIsAnimated, normalizeListener } from "./listener-style.ts";

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

assert(normalizeListener(undefined) === "blob", "missing setting is Bolo Blob");
assert(normalizeListener(null) === "blob", "null setting is Bolo Blob");
assert(normalizeListener("") === "blob", "blank setting is Bolo Blob");
assert(normalizeListener("BLOB") === "blob", "blob is accepted in any case");
assert(normalizeListener(" classic ") === "classic", "classic is the explicit fallback");
assert(normalizeListener("buddy") === "buddy", "Mic Buddy is a listener");
assert(normalizeListener("SPIRIT") === "spirit", "Glow Spirit is a listener");
assert(normalizeListener(" pebble ") === "pebble", "Pebble is a listener");
assert(listenerIsAnimated("blob") === true, "blob is the animated overlay");
assert(listenerIsAnimated("buddy") && listenerIsAnimated("spirit") && listenerIsAnimated("pebble"), "the other characters animate");
assert(listenerIsAnimated("classic") === false, "classic keeps the clip overlay");
assert(listenerIsAnimated("nope") === true, "an unknown id still animates, as Bolo Blob");

console.log("listener-style ok");
