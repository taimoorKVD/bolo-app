// Quiet managed-Bolo connection state. Listens to the `managed-status` event
// from the desktop core and shows a small black/gold pill only when the user
// should know something. Copy is fixed here; raw codes are never displayed.

export type ManagedStateName =
  | "not_configured"
  | "connected"
  | "reconnecting"
  | "sign_in_required"
  | "paused"
  | "fair_use"
  | "pool_full"
  | "offline";

export interface ManagedSnapshot {
  state: ManagedStateName | string;
  message?: string | null;
}

export interface ManagedStatusView {
  tone: "info" | "backup" | "action";
  label: string;
  detail: string;
}

export function managedStatusView(snapshot: ManagedSnapshot | null | undefined): ManagedStatusView | null {
  switch (snapshot?.state) {
    case "reconnecting":
      return { tone: "info", label: "Reconnecting…", detail: "Bolo is reconnecting. Dictation keeps working." };
    case "pool_full":
    case "fair_use":
    case "paused":
    case "offline":
      return { tone: "backup", label: "Using backup", detail: "Bolo is busy right now, so your backup is in use." };
    case "sign_in_required":
      return { tone: "action", label: "Sign in again", detail: "Sign in again to keep using Bolo. Your backup is in use meanwhile." };
    default:
      return null;
  }
}

export function renderManagedStatus(pill: HTMLElement, snapshot: ManagedSnapshot | null | undefined): void {
  const view = managedStatusView(snapshot);
  if (!view) {
    pill.hidden = true;
    pill.textContent = "";
    return;
  }
  pill.hidden = false;
  pill.dataset.tone = view.tone;
  pill.textContent = view.label;
  pill.title = view.detail;
  pill.setAttribute("aria-label", view.detail);
}

/** Mount the pill and subscribe. Safe outside Tauri (does nothing). */
export async function mountManagedStatus(host: HTMLElement): Promise<() => void> {
  const pill = document.createElement("div");
  pill.className = "managed-status";
  pill.setAttribute("role", "status");
  pill.setAttribute("aria-live", "polite");
  pill.hidden = true;
  host.appendChild(pill);
  try {
    const { listen } = await import("@tauri-apps/api/event");
    const stop = await listen<ManagedSnapshot>("managed-status", (event) => renderManagedStatus(pill, event.payload));
    return () => {
      stop();
      pill.remove();
    };
  } catch {
    return () => pill.remove();
  }
}
