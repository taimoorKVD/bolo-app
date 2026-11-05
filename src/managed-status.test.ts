import { managedStatusView } from "./managed-status.ts";

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

assert(managedStatusView(null) === null, "nothing before first event");
assert(managedStatusView({ state: "connected" }) === null, "connected is quiet");
assert(managedStatusView({ state: "not_configured" }) === null, "not configured is quiet");
assert(managedStatusView({ state: "reconnecting" })?.label === "Reconnecting…", "reconnecting");
for (const state of ["pool_full", "fair_use", "paused", "offline"]) {
  assert(managedStatusView({ state })?.label === "Using backup", `${state} uses backup`);
}
assert(managedStatusView({ state: "sign_in_required" })?.label === "Sign in again", "sign in");
assert(managedStatusView({ state: "sign_in_required" })?.tone === "action", "sign in is the only action");

const raw = { state: "pool_full", message: "pool_full http 503 refresh_reuse" };
const view = managedStatusView(raw)!;
for (const text of [view.label, view.detail]) {
  assert(!/[a-z]+_[a-z]+|http \d|\d{3}/.test(text), `no raw codes in "${text}"`);
}
assert(managedStatusView({ state: "something_new" }) === null, "unknown states stay quiet");
console.log("managed-status tests passed");
