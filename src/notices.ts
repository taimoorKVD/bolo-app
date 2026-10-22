/** Blank and whitespace-only strings are not messages. */
export function visibleMessage(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const text = value.trim();
  return text.length > 0 ? text : null;
}

/** Copy for the hotkey banner. Null when the hook is active and the banner should not render. */
export function listenerNotice(running: boolean, error: string | null | undefined): string | null {
  if (running) return null;
  const detail = visibleMessage(error);
  if (detail) return `Global hotkey isn't active. ${detail}`;
  return "Global hotkey isn't active yet. You can still record a test below.";
}
