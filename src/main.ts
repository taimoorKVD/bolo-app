import { inTauri } from "./api";
import { mountOverlay } from "./overlay";
import { mountSettings } from "./settings";

const root = document.querySelector<HTMLElement>("#app");

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
  const view = await currentView();
  document.body.classList.add(view === "overlay" ? "view-overlay" : "view-settings");
  if (!inTauri()) document.body.classList.add("browser-preview");
  if (view === "overlay") mountOverlay(root);
  else await mountSettings(root);
}

void boot();
