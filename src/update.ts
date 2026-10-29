import { inTauri } from "./api";

type UpdateInfo = {
  currentVersion: string;
  version: string;
  notes: string;
  date: string | null;
};

async function invoke<T>(command: string): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command);
}

let open = false;

/** Mounts the "Update available" prompt in the settings window. */
export async function mountUpdatePrompt(): Promise<void> {
  if (!inTauri()) return;
  if (!(await invoke<boolean>("updates_enabled").catch(() => false))) return;
  const { listen } = await import("@tauri-apps/api/event");
  await listen<UpdateInfo>("update-available", (event) => show(event.payload));
  const pending = await invoke<UpdateInfo | null>("pending_update").catch(() => null);
  if (pending) show(pending);
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

function formatDate(value: string | null): string {
  if (!value) return "";
  const date = new Date(value.replace(" +00:00:00", "Z").replace(" ", "T"));
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

function show(info: UpdateInfo): void {
  if (open) return;
  open = true;
  const backdrop = el("div", "update-backdrop");
  const card = el("section", "update-card");
  card.setAttribute("role", "dialog");
  card.setAttribute("aria-modal", "true");
  card.setAttribute("aria-labelledby", "update-title");

  const badge = el("span", "update-badge", "Update available");
  const title = el("h2", "update-title", `Bolo ${info.version} is ready`);
  title.id = "update-title";
  const released = formatDate(info.date);
  const meta = el(
    "p",
    "update-meta",
    `You have ${info.currentVersion}${released ? ` · released ${released}` : ""}`,
  );
  const notesHead = el("h3", "update-notes-head", "What's new");
  const notes = el("div", "update-notes");
  const lines = (info.notes || "Bug fixes and improvements.").split(/\r?\n/).filter((l) => l.trim());
  const list = el("ul");
  for (const line of lines) list.append(el("li", undefined, line.replace(/^[-*•]\s*/, "")));
  notes.append(list);

  const progress = el("div", "update-progress");
  const bar = el("div", "update-progress-bar");
  progress.append(bar);
  progress.hidden = true;
  const status = el("p", "update-status");
  status.setAttribute("aria-live", "polite");

  const actions = el("div", "update-actions");
  const later = el("button", "update-later", "Later");
  later.type = "button";
  const now = el("button", "update-now", "Update now");
  now.type = "button";
  actions.append(later, now);

  card.append(badge, title, meta, notesHead, notes, progress, status, actions);
  backdrop.append(card);
  document.body.append(backdrop);
  now.focus();

  const close = () => {
    backdrop.remove();
    open = false;
  };
  later.addEventListener("click", close);
  backdrop.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !now.disabled) close();
  });

  now.addEventListener("click", async () => {
    now.disabled = true;
    later.disabled = true;
    now.textContent = "Updating…";
    progress.hidden = false;
    status.textContent = "Downloading the update…";
    const { listen } = await import("@tauri-apps/api/event");
    const stop = await listen<{ received: number; total: number | null }>("update-progress", (e) => {
      const { received, total } = e.payload;
      if (total) {
        const pct = Math.min(100, Math.round((received / total) * 100));
        bar.style.width = `${pct}%`;
        status.textContent = `Downloading… ${pct}%`;
      } else {
        bar.classList.add("indeterminate");
      }
    });
    try {
      await invoke<void>("install_update");
      status.textContent = "Installing. Bolo will restart in a moment.";
    } catch (error) {
      stop();
      status.textContent = `The update didn't install: ${String(error)}`;
      now.disabled = false;
      later.disabled = false;
      now.textContent = "Try again";
    }
  });
}
