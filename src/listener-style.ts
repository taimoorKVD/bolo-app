/// Which listening overlay is on screen.
/// `blob` is the animated Bolo Blob. `classic` is the existing clip overlay.
export type ListenerId = "blob" | "classic";

export const LISTENER_BLOB: ListenerId = "blob";
export const LISTENER_CLASSIC: ListenerId = "classic";

/** Unknown and missing values use Bolo Blob. Classic is an explicit choice. */
export function normalizeListener(value: string | null | undefined): ListenerId {
  const id = (value ?? "").trim().toLowerCase();
  return id === LISTENER_CLASSIC ? LISTENER_CLASSIC : LISTENER_BLOB;
}

export function listenerIsAnimated(value: string | null | undefined): boolean {
  return normalizeListener(value) !== LISTENER_CLASSIC;
}
