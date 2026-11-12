/// Which listening overlay is on screen.
/// The four animated characters share one rig. `classic` is the clip overlay.
export type ListenerId = "blob" | "buddy" | "spirit" | "pebble" | "classic";

export const LISTENER_BLOB: ListenerId = "blob";
export const LISTENER_CLASSIC: ListenerId = "classic";

export const LISTENER_CHOICES: Array<{ id: ListenerId; name: string; animated: boolean }> = [
  { id: "blob", name: "Bolo Blob", animated: true },
  { id: "buddy", name: "Mic Buddy", animated: true },
  { id: "spirit", name: "Glow Spirit", animated: true },
  { id: "pebble", name: "Pebble", animated: true },
  { id: "classic", name: "Classic", animated: false },
];

const ANIMATED: ListenerId[] = ["blob", "buddy", "spirit", "pebble"];

/** Unknown and missing values use Bolo Blob. Classic is an explicit choice. */
export function normalizeListener(value: string | null | undefined): ListenerId {
  const id = (value ?? "").trim().toLowerCase();
  if (id === LISTENER_CLASSIC) return LISTENER_CLASSIC;
  if (ANIMATED.includes(id as ListenerId)) return id as ListenerId;
  return LISTENER_BLOB;
}

export function isAnimatedListener(value: string | null | undefined): value is Exclude<ListenerId, "classic"> {
  const id = normalizeListener(value);
  return id !== LISTENER_CLASSIC;
}

export function listenerIsAnimated(value: string | null | undefined): boolean {
  return normalizeListener(value) !== LISTENER_CLASSIC;
}
