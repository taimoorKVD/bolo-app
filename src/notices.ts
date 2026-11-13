/** Blank and whitespace-only strings are not messages. */
export function visibleMessage(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const text = value.trim();
  return text.length > 0 ? text : null;
}

const STATUS_TITLES: Record<string, string> = {
  arming: "Listening",
  recording: "Listening",
  transcribing: "Transcribing",
  inserting: "Inserting",
  done: "Inserted",
  // Legacy phase from older builds; treat as a successful insert.
  typed: "Inserted",
  error: "Dictation failed",
};

/**
 * Settings status line. Null when it would only repeat itself
 * ("Inserted — Inserted"), which is not a useful row.
 */
export function statusBanner(phase: string, message: string | null | undefined): string | null {
  const title = STATUS_TITLES[phase] ?? "Bolo";
  const detail = visibleMessage(message);
  if (!detail) {
    // Bare legacy "typed" with no note: hide the empty row.
    return phase === "typed" ? null : title;
  }
  const same =
    detail.toLowerCase() === title.toLowerCase() || detail.toLowerCase() === "typed";
  if (same) return title;
  return `${title} — ${detail}`;
}

/** Copy for the hotkey banner. Null when the hook is active and the banner should not render. */
export function listenerNotice(running: boolean, error: string | null | undefined): string | null {
  if (running) return null;
  const detail = visibleMessage(error);
  if (detail) return `Global hotkey isn't active. ${detail}`;
  return "Global hotkey isn't active yet. You can still record a test below.";
}
