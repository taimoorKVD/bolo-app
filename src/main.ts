import { api, inTauri } from "./api";
import { mountOverlay } from "./overlay";
import { mountSettings } from "./settings";
import { mountUpdatePrompt } from "./update";

const root = document.querySelector<HTMLElement>("#app");

declare global {
  interface Window {
    __CADENCE_SMOKE__?: boolean;
  }
}

async function currentView(): Promise<"settings" | "overlay"> {
  const params = new URLSearchParams(location.search);
  if (params.get("view") === "overlay") return "overlay";
  if (inTauri()) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return getCurrentWindow().label === "overlay" ? "overlay" : "settings";
  }
  return "settings";
}

async function boot(): Promise<void> {
  if (!root) return;
  const smoke = window.__CADENCE_SMOKE__ === true;
  const view = await currentView();
  document.body.classList.add(view === "overlay" ? "view-overlay" : "view-settings");
  if (!inTauri()) document.body.classList.add("browser-preview");
  try {
    if (view === "overlay") {
      mountOverlay(root);
      return;
    }
    const loaded = await mountSettings(root);
    if (!smoke) void mountUpdatePrompt();
    if (!smoke) return;
    if (loaded.ok) await api.smokeFinished(true, loaded.detail);
    else await api.smokeFinished(false, loaded.error);
  } catch (error) {
    if (!smoke) return;
    const message = error instanceof Error ? error.message : "Settings didn't load.";
    try {
      await api.smokeFinished(false, message);
    } catch {
      // The smoke harness times out if this invoke cannot be sent.
    }
  }
}

void boot();
