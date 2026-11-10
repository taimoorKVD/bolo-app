import { mountManagedStatus } from "./managed-status";
import { api, onDictation, onHistoryChanged, onListenerStatus, onMicStatus, onOpenFix } from "./api";
import type { MemoryView } from "./types";
import {
  CHARACTERS,
  DEFAULT_CHARACTER,
  activeCharacter,
  GROUPS,
  SOUND_PACKS,
  clipUrls,
  findCharacter,
  resolvePack,
  soundUrl,
  type CharacterGroup,
} from "./characters";
import lockupUrl from "./brand-lockup.svg";
import { formatDuration, formatHotkey, formatWhen, modelLabel } from "./format";
import { listenerNotice, statusBanner, visibleMessage } from "./notices";
import { knownOfflineModel, OFFLINE_MODELS, offlineModelNote } from "./offline-models";
import { SCRIPT_SETS, scriptSetId, sentenceCue } from "./script-sets";
import type {
  ApiKeyStatus,
  ProviderKeyRow,
  AppInfo,
  DictationEvent,
  HistoryEntry,
  Hotkey,
  MicInfo,
  Settings,
  WhisperModel,
  ScriptLine,
  HeldInfo,
} from "./types";

const LANGUAGES: Array<[string, string]> = [
  ["auto", "Auto-detect"],
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["pt", "Portuguese"],
  ["it", "Italian"],
  ["nl", "Dutch"],
  ["pl", "Polish"],
  ["sv", "Swedish"],
  ["da", "Danish"],
  ["fi", "Finnish"],
  ["no", "Norwegian"],
  ["cs", "Czech"],
  ["tr", "Turkish"],
  ["ru", "Russian"],
  ["uk", "Ukrainian"],
  ["ar", "Arabic"],
  ["he", "Hebrew"],
  ["hi", "Hindi"],
  ["ja", "Japanese"],
  ["ko", "Korean"],
  ["zh", "Chinese"],
  ["id", "Indonesian"],
  ["vi", "Vietnamese"],
  ["th", "Thai"],
  ["ur", "Urdu"],
];

type SectionId =
  | "home"
  | "shortcut"
  | "character"
  | "languages"
  | "words"
  | "history"
  | "learned"
  | "advanced"
  | "key"
  | "engine"
  | "cleanup"
  | "dictionary"
  | "privacy"
  | "paste"
  | "diagnostics"
  | "accuracy";

/** Main pages, with an icon and a title. Advanced sits apart at the bottom. */
const NAV: Array<[SectionId, string, string]> = [
  ["home", "⌂", "Home"],
  ["shortcut", "⌨", "Shortcut"],
  ["character", "☺", "Character & sounds"],
  ["languages", "◍", "Languages"],
  ["words", "✎", "My words"],
  ["history", "↺", "History"],
  ["learned", "✦", "What Bolo has learned"],
];

/** Rows on the Advanced page. Each opens its own page. */
const ADVANCED: Array<[SectionId, string, string]> = [
  ["key", "Your own service key", "Use your own key instead of the built-in one"],
  ["engine", "Speech quality and offline mode", "Faster or more accurate, and what happens without internet"],
  ["cleanup", "Text clean-up style", "How much Bolo tidies your words"],
  ["dictionary", "Dictionary", "Extra spellings, one per line"],
  ["privacy", "Privacy", "What stays on this PC"],
  ["paste", "How text is pasted", "Paste the last take again"],
  ["diagnostics", "Help and diagnostics", "Version, status, and a report you can copy"],
  ["accuracy", "Accuracy", "Save debug traces on this PC, and record the test sentences"],
];

const FEATURED = ["classic", "c02", "c04", "c07", "c06"];

function normalizeSettings(settings: Settings): Settings {
  return {
    ...settings,
    historyEnabled: settings.historyEnabled !== false,
    offlineModel: knownOfflineModel(settings.offlineModel),
    pasteHotkey: settings.pasteHotkey ?? { ctrl: true, alt: false, shift: false, win: true, key: "v" },
    output: settings.output === "same" ? "same" : "english",
    verbatim: settings.verbatim === true,
    cleanupModel: settings.cleanupModel?.trim() || "openai/gpt-oss-20b",
    character: findCharacter(settings.character).id,
    soundPack: SOUND_PACKS.some(([id]) => id === settings.soundPack) ? settings.soundPack : "match",
    soundVolume:
      typeof settings.soundVolume === "number" && Number.isFinite(settings.soundVolume)
        ? Math.max(0, Math.min(1, settings.soundVolume))
        : 0.6,
    smartMood: settings.smartMood === true,
    smartTimeOfDay: settings.smartTimeOfDay === true,
    smartHoliday: settings.smartHoliday === true,
    keepLearning: settings.keepLearning !== false,
    keepRecordings: settings.keepRecordings === true,
    debugCapture: settings.debugCapture === true,
  };
}

type Draft = {
  settings: Settings;
  keyStatus: ApiKeyStatus;
  mics: MicInfo[];
  history: HistoryEntry[];
  info: AppInfo;
  dirty: boolean;
  saving: boolean;
  status: string;
  statusTone: "muted" | "good" | "bad";
  capturing: boolean;
  live: DictationEvent | null;
  lastEngine: string;
  lastError: string | null;
  section: SectionId;
};

export type SettingsLoad =
  | { ok: true; detail: string }
  | { ok: false; error: string };

export async function mountSettings(root: HTMLElement): Promise<SettingsLoad> {
  const splash = loadingShell();
  const splashAt = performance.now();
  root.replaceChildren(splash);
  // Microphone enumeration can take a second on Windows; don't hold the
  // whole window on it. The list fills in once it arrives.
  const micsPromise = api.listMicrophones().catch(() => [] as MicInfo[]);
  try {
    const [settings, keyStatus, history, info] = await Promise.all([
      api.getSettings(),
      api.apiKeyStatus(),
      api.getHistory(),
      api.getAppInfo(),
    ]);
    const quick = await Promise.race([micsPromise, new Promise<null>((r) => setTimeout(() => r(null), 120))]);
    const mics = quick ?? [];
    await leaveSplash(splash, splashAt);
    render(root, (currentDraft = {
      settings: normalizeSettings(settings),
      keyStatus,
      mics,
      history,
      info: {
        ...info,
        listenerError: visibleMessage(info.listenerError),
        lastError: visibleMessage(info.lastError),
      },
      dirty: false,
      saving: false,
      status: "",
      statusTone: "muted",
      capturing: false,
      live: null,
      lastEngine: "Groq",
      lastError: info.lastError,
      section: "home",
    }));
    root.firstElementChild?.classList.add("app-enter");
    if (quick === null) {
      const draft = currentDraft;
      void micsPromise.then((devices) => {
        if (draft && currentDraft === draft) applyDevices(draft, devices);
      });
    }
    return { ok: true, detail: JSON.stringify(settings) };
  } catch (error) {
    const message = messageOf(error);
    root.replaceChildren(errorShell(message, () => void mountSettings(root)));
    return { ok: false, error: message };
  }
}

function render(root: HTMLElement, draft: Draft): void {
  root.replaceChildren();
  const shell = el("div", "shell v2");
  const nav = buildNav(draft);
  const main = el("main", "stage");
  const form = document.createElement("form");
  form.className = "sheet";
  form.addEventListener("submit", (event) => event.preventDefault());

  const status = el("p", "save-status", "");
  status.dataset.saveStatus = "true";
  status.setAttribute("aria-live", "polite");
  const alert = alertBanner(draft);
  form.append(
    ...(alert ? [alert] : []),
    liveBanner(draft),
    listenerBanner(draft),
    homePanel(draft),
    hotkeyPanel(draft),
    micPanel(draft),
    characterPanel(draft),
    languagePanel(draft),
    historyPanel(draft),
    wordsPanel(draft),
    learnedPanel(draft),
    advancedPanel(draft),
    keyPanel(draft),
    enginePanel(draft),
    cleanupPanel(draft),
    dictionaryPanel(draft),
    privacyPanel(draft),
    pastePanel(draft),
    diagnosticsPanel(draft),
    accuracyPanel(draft),
  );
  main.append(status, form);
  shell.append(nav, main);
  root.append(shell);
  showSection(form, draft.section);
  // Names load after the window is on screen. The first paint does not wait.
  void refreshMemory(draft);

  void onDictation((event) => {
    draft.live = event.phase === "idle" ? null : event;
    if (event.phase === "error") {
      const message = visibleMessage(event.message);
      if (message) draft.lastError = message;
    }
    if (event.phase === "recording" || event.phase === "arming") draft.lastError = null;
    if (event.engine?.trim()) draft.lastEngine = event.engine.trim();
    const banner = form.querySelector<HTMLElement>("[data-live]");
    if (banner) banner.replaceWith(liveBanner(draft));
    placeAlert(form, draft);
    for (const test of document.querySelectorAll<HTMLButtonElement>("[data-try]")) {
      test.textContent = event.phase === "recording" ? "Stop" : test.dataset.try ?? "Try it";
      test.disabled = event.phase === "transcribing" || event.phase === "inserting";
    }
    if (event.phase === "recording" || event.phase === "transcribing" || event.phase === "arming") {
      document.querySelector("[data-rate-slot]")?.replaceChildren();
    }
    if (event.phase === "done" || event.phase === "idle") {
      void refreshHistory(draft);
      const slot = document.querySelector<HTMLElement>("[data-rate-slot]");
      if (slot && draft.section === "home") void fillHome(slot);
    }
    refreshPills(draft);
  });
  void onMicStatus((status) => applyDevices(draft, status.devices));
  void watchListener(draft, form);
  window.addEventListener("bolo-history", () => void refreshHistory(draft));
  void onHistoryChanged(() => void refreshHistory(draft));
  void onOpenFix((id) => {
    void api.fixRequested().catch(() => {});
    void openFixDialog(draft, id || null);
  });
  void api
    .takeFixRequest()
    .then((id) => {
      if (id !== null && id !== undefined) void openFixDialog(draft, id || null);
    })
    .catch(() => {});
  window.setInterval(() => void paintMeter(), 120);
}

function showSection(form: HTMLElement, id: SectionId): void {
  for (const panel of form.querySelectorAll<HTMLElement>("[data-panel]")) {
    panel.hidden = panel.dataset.panel !== id;
  }
  const navId = ADVANCED.some(([sub]) => sub === id) ? "advanced" : id;
  for (const button of document.querySelectorAll<HTMLButtonElement>("[data-nav]")) {
    const on = button.dataset.nav === navId;
    button.classList.toggle("is-on", on);
    if (on) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  }
  // Only the visible character clips play.
  for (const video of form.querySelectorAll<HTMLVideoElement>("video")) {
    const visible = !video.closest<HTMLElement>("[data-panel]")?.hidden;
    if (visible) void video.play().catch(() => {});
    else video.pause();
  }
  document.querySelector(".stage")?.scrollTo({ top: 0 });
}

function go(draft: Draft, id: SectionId): void {
  if (draft.section === "accuracy" && id !== "accuracy") {
    void api.accuracyDiscard();
    void api.setHotkeyCapture(false);
  }
  if (id === "accuracy" && draft.section !== "accuracy") {
    void api.setHotkeyCapture(true);
  }
  draft.section = id;
  const form = document.querySelector<HTMLElement>(".sheet");
  if (form) showSection(form, id);
}

function buildNav(draft: Draft): HTMLElement {
  const rail = el("aside", "rail");
  const brand = el("div", "brand");
  brand.append(logoHeading());
  const menu = el("nav", "sidenav");
  menu.setAttribute("aria-label", "Settings");
  const item = (id: SectionId, icon: string, title: string) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "nav-item";
    button.dataset.nav = id;
    const ic = el("span", "nav-icon", icon);
    ic.setAttribute("aria-hidden", "true");
    button.append(ic, el("span", "nav-title", title));
    button.addEventListener("click", () => go(draft, id));
    return button;
  };
  for (const [id, icon, title] of NAV) menu.append(item(id, icon, title));
  const advanced = item("advanced", "⚙", "Advanced");
  advanced.classList.add("nav-advanced");
  rail.append(brand, menu, el("div", "rail-spacer"), advanced);
  return rail;
}

function refreshPills(draft: Draft): void {
  refreshHome(draft);
  const key = document.querySelector<HTMLElement>('[data-pill="key"]');
  const hotkey = document.querySelector<HTMLElement>('[data-pill="hotkey"]');
  const mic = document.querySelector<HTMLElement>('[data-pill="mic"]');
  const keyText = keyPill(draft.keyStatus);
  setPill(key, keyText.value, keyText.tone);
  setPill(hotkey, formatHotkey(draft.settings.hotkey), "neutral");
  setPill(mic, micSummary(draft).value, "neutral");
  const engine = document.querySelector<HTMLElement>('[data-pill="engine"]');
  const engineName = draft.live?.engine?.trim() || draft.lastEngine || "Groq";
  setPill(engine, engineName, engineName === "Groq" ? "good" : "warn");
}

function setPill(node: HTMLElement | null, value: string, tone: string): void {
  if (!node) return;
  const text = node.querySelector(".pill-v");
  if (text) text.textContent = value;
  node.className = `status-pill tone-${tone}`;
}

function placeAlert(form: HTMLElement, draft: Draft): void {
  const current = form.querySelector<HTMLElement>("[data-alert]");
  const next = alertBanner(draft);
  if (next && current) current.replaceWith(next);
  else if (next) form.prepend(next);
  else current?.remove();
}

async function watchListener(draft: Draft, form: HTMLElement): Promise<void> {
  const apply = (running: boolean, error: string | null) => {
    draft.info.listenerRunning = running;
    draft.info.listenerError = visibleMessage(error);
    refreshHome(draft);
    const current = form.querySelector<HTMLElement>("[data-listener]");
    const next = listenerBanner(draft);
    if (current) current.replaceWith(next);
  };
  await onListenerStatus((status) => apply(status.running, status.error));
  void mountManagedStatus(document.body);
  let attempts = 0;
  const poll = async (): Promise<boolean> => {
    try {
      const info = await api.getAppInfo();
      apply(info.listenerRunning, info.listenerError);
      return info.listenerRunning || visibleMessage(info.listenerError) !== null;
    } catch {
      return false;
    }
  };
  if (await poll()) return;
  const timer = window.setInterval(() => {
    void (async () => {
      attempts += 1;
      const running = await poll();
      if (running || attempts >= 40) window.clearInterval(timer);
    })();
  }, 250);
}

function liveBanner(draft: Draft): HTMLElement {
  const banner = el("div", "banner live");
  banner.dataset.live = "true";
  if (!draft.live) {
    banner.hidden = true;
    return banner;
  }
  const text = statusBanner(draft.live.phase, draft.live.message);
  if (!text) {
    banner.hidden = true;
    return banner;
  }
  banner.textContent = text;
  if (draft.live.phase === "error") banner.classList.add("bad");
  return banner;
}

function alertBanner(draft: Draft): HTMLElement | null {
  const message = visibleMessage(draft.lastError);
  if (!message) return null;
  const banner = el("div", "banner bad alert");
  banner.dataset.alert = "true";
  banner.setAttribute("role", "alert");
  const copy = el("p", "", message);
  const dismiss = button("Dismiss", "text");
  dismiss.addEventListener("click", () => {
    draft.lastError = null;
    banner.remove();
    void api.dismissError();
  });
  banner.append(copy, dismiss);
  return banner;
}

function listenerBanner(draft: Draft): HTMLElement {
  const banner = el("div", "banner");
  banner.dataset.listener = "true";
  const notice = listenerNotice(draft.info.listenerRunning, draft.info.listenerError);
  if (!notice) {
    banner.hidden = true;
    return banner;
  }
  banner.classList.add("warn");
  banner.textContent = notice;
  return banner;
}

function keyPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "key",
    "Your own service key",
    "Bolo works without this. A key is optional. Paste one only if you want dictation to use your own Groq account when the built-in path is busy.",
  );
  const steps = el("ol", "key-steps");
  for (const step of [
    "Open Groq and create a key. You do not need to change any microphone setting for this.",
    "Paste it below. Bolo asks Groq whether the key is accepted, then stores it in Windows Credential Manager. It is not written into settings or the log.",
    "Optional: add a backup key. If one key hits its daily limit, Bolo uses the next key you saved. This does not turn on a paid fallback.",
  ]) {
    steps.append(el("li", "", step));
  }
  panel.append(steps);
  const ownLabel = el("p", "own-key-label", "Use my own Groq API key");
  ownLabel.dataset.ownKeyLabel = "true";
  ownLabel.hidden = !draft.keyStatus.builtin;
  const row = el("div", "key-row");
  const input = document.createElement("input");
  input.type = "password";
  input.autocomplete = "off";
  input.spellcheck = false;
  input.placeholder = draft.keyStatus.configured
    ? `Saved ${draft.keyStatus.hint ?? ""}. Paste a new key to replace it.`
    : "gsk_…";
  input.setAttribute("aria-label", "Groq API key");
  input.dataset.apiKey = "true";
  const saveKey = button("Save key", "primary");
  const remove = button(draft.keyStatus.builtin ? "Return to built-in key" : "Remove", "ghost");
  remove.dataset.keyClear = "true";
  remove.hidden = draft.keyStatus.builtin && !draft.keyStatus.configured;
  remove.disabled = !draft.keyStatus.configured;
  const note = el("p", "field-note");
  note.dataset.keyNote = "true";
  note.textContent = keyNote(draft.keyStatus);
  if (draft.keyStatus.error) note.classList.add("bad");
  saveKey.addEventListener("click", () => {
    void (async () => {
      saveKey.disabled = true;
      try {
        const saved = await api.saveProviderKey("default", input.value);
        input.value = "";
        draft.keyStatus = await api.apiKeyStatus();
        note.textContent = saved.note ?? keyNote(draft.keyStatus);
        note.className = saved.note ? "field-note" : "field-note";
        syncKeyControls(draft);
        paintProviderKeys(saved.keys, note);
      } catch (error) {
        note.textContent = messageOf(error);
        note.className = "field-note bad";
      } finally {
        saveKey.disabled = false;
      }
    })();
  });
  remove.addEventListener("click", () => {
    void (async () => {
      try {
        await api.revokeProviderKey("default");
        draft.keyStatus = await api.apiKeyStatus();
        input.value = "";
        syncKeyControls(draft);
        const listed = await api.providerKeys();
        paintProviderKeys(listed.keys, note);
      } catch (error) {
        note.textContent = messageOf(error);
        note.className = "field-note bad";
      }
    })();
  });
  const link = button("Open Groq console", "text");
  link.addEventListener("click", () =>
    void api.openGroqKeys().catch((error) => {
      note.textContent = messageOf(error);
      note.className = "field-note bad";
    }),
  );
  row.append(input, saveKey, remove);
  const backups = el("div", "provider-keys");
  backups.dataset.providerKeys = "true";
  const backupInput = document.createElement("input");
  backupInput.type = "password";
  backupInput.autocomplete = "off";
  backupInput.spellcheck = false;
  backupInput.placeholder = "Backup key";
  backupInput.setAttribute("aria-label", "Backup Groq API key");
  const addBackup = button("Add backup key", "ghost");
  addBackup.addEventListener("click", () => {
    void (async () => {
      addBackup.disabled = true;
      try {
        const saved = await api.saveProviderKey("new", backupInput.value);
        backupInput.value = "";
        note.textContent = saved.note ?? "Backup key saved on this PC.";
        note.className = "field-note";
        paintProviderKeys(saved.keys, note);
      } catch (error) {
        note.textContent = messageOf(error);
        note.className = "field-note bad";
      } finally {
        addBackup.disabled = false;
      }
    })();
  });
  const backupRow = el("div", "key-row");
  backupRow.append(backupInput, addBackup);
  panel.append(ownLabel, row, link, el("h3", "key-sub", "Backup keys"), backups, backupRow, note);
  void api.providerKeys().then((listed) => paintProviderKeys(listed.keys, note)).catch(() => {});
  return panel;
}

function providerStateLabel(row: ProviderKeyRow): string {
  if (row.kind === "builtin") return "Bolo's key";
  if (row.state === "cooling") return "Waiting for its daily limit to reset";
  if (row.state === "dead") return "Groq rejected it";
  if (row.id === "default") return "Your key";
  return "Backup";
}

function paintProviderKeys(rows: ProviderKeyRow[], note: HTMLElement): void {
  const host = document.querySelector<HTMLElement>("[data-provider-keys]");
  if (!host) return;
  host.replaceChildren();
  const shown = rows.filter((row) => row.kind !== "builtin");
  if (shown.length === 0) {
    host.append(el("p", "field-note", "No key of yours is saved. Bolo keeps working without one."));
    return;
  }
  for (const row of shown) {
    const line = el("div", "learned-row");
    const text = el("p", "");
    const hint = row.hint ?? "saved";
    text.append(el("strong", "", providerStateLabel(row)), el("span", "", ` ${hint}`));
    if (row.lastError) text.append(el("span", "field-note", ` ${row.lastError}`));
    const remove = button(row.id === "default" ? "Remove" : "Remove backup", "text");
    remove.addEventListener("click", () => {
      void (async () => {
        try {
          const saved = await api.revokeProviderKey(row.id);
          paintProviderKeys(saved.keys, note);
          if (row.id === "default" && currentDraft) {
            currentDraft.keyStatus = await api.apiKeyStatus();
            syncKeyControls(currentDraft);
          }
          note.textContent = "Removed from this PC.";
          note.className = "field-note";
        } catch (error) {
          note.textContent = messageOf(error);
          note.className = "field-note bad";
        }
      })();
    });
    line.append(text, remove);
    host.append(line);
  }
}

function hotkeyPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "shortcut",
    "Your shortcut",
    "Hold these keys and talk. Let go, and your words appear. Tap twice quickly to keep listening without holding, then tap once to stop.",
  );
  const display = el("div", "hotkey-display");
  const kbd = el("kbd", "hotkey-kbd", formatHotkey(draft.settings.hotkey));
  const change = button("Change shortcut", "ghost");
  const reset = button("Reset", "text");
  display.append(kbd, change, reset);
  panel.classList.add("shortcut-card");
  const presets = el("div", "presets");
  const presetHotkeys: Array<[string, Hotkey]> = [
    ["Ctrl + Win", { ctrl: true, alt: false, shift: false, win: true, key: null }],
    ["Ctrl + Space", { ctrl: true, alt: false, shift: false, win: false, key: "space" }],
    ["Ctrl + Alt", { ctrl: true, alt: true, shift: false, win: false, key: null }],
    ["Alt + Space", { ctrl: false, alt: true, shift: false, win: false, key: "space" }],
  ];
  for (const [label, hotkey] of presetHotkeys) {
    const preset = button(label, "text");
    preset.addEventListener("click", () => {
      draft.settings.hotkey = hotkey;
      kbd.textContent = formatHotkey(hotkey);
      markDirty(draft);
      refreshPills(draft);
    });
    presets.append(preset);
  }
  const hint = el(
    "p",
    "field-note",
    "Pick one of these, or click Change shortcut and press the keys you want. Press Esc to cancel.",
  );
  change.addEventListener("click", () => startCapture(draft, kbd, hint, change));
  reset.addEventListener("click", () => {
    draft.settings.hotkey = { ctrl: true, alt: false, shift: false, win: true, key: null };
    kbd.textContent = formatHotkey(draft.settings.hotkey);
    markDirty(draft);
    refreshPills(draft);
  });
  const test = tryButton(draft, "Try it", "ghost");
  panel.append(
    display,
    presets,
    hint,
    test,
    el("p", "field-note", "Try it starts listening. Press it again to stop, and the words go into whatever app is in front."),
  );
  return panel;
}

function pastePanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "paste",
    "How text is pasted",
    "Bolo pastes your words at the cursor. If a paste didn't land, or after Bolo restarts, this shortcut pastes the last take again.",
  );
  const pasteLabel = el("label", "stack");
  pasteLabel.append(el("span", "label", "Paste the last take again"));
  const pasteRow = el("div", "hotkey-display");
  const pasteKbd = el("kbd", "hotkey-kbd", formatHotkey(draft.settings.pasteHotkey));
  const pastePresets = el("div", "presets");
  const pasteChoices: Array<[string, Hotkey]> = [
    ["Ctrl + Win + V", { ctrl: true, alt: false, shift: false, win: true, key: "v" }],
    ["Ctrl + Alt + V", { ctrl: true, alt: true, shift: false, win: false, key: "v" }],
    ["Alt + V", { ctrl: false, alt: true, shift: false, win: false, key: "v" }],
  ];
  for (const [label, hotkey] of pasteChoices) {
    const preset = button(label, "text");
    preset.addEventListener("click", () => {
      draft.settings.pasteHotkey = hotkey;
      pasteKbd.textContent = formatHotkey(hotkey);
      markDirty(draft);
    });
    pastePresets.append(preset);
  }
  pasteRow.append(pasteKbd);
  pasteLabel.append(
    pasteRow,
    pastePresets,
    el(
      "p",
      "field-note",
      "Press this when a take didn't land, or after Bolo starts again. The audio stays on this PC until a paste is confirmed.",
    ),
  );
  panel.append(pasteLabel);
  return panel;
}

function tryButton(draft: Draft, label: string, kind: "primary" | "ghost"): HTMLButtonElement {
  const test = button(draft.live?.phase === "recording" ? "Stop" : label, kind);
  test.dataset.try = label;
  test.addEventListener("click", () => {
    (document.activeElement as HTMLElement | null)?.blur();
    void api.toggleTestRecording();
  });
  return test;
}

function startCapture(
  draft: Draft,
  kbd: HTMLElement,
  hint: HTMLElement,
  change: HTMLButtonElement,
): void {
  if (draft.capturing) return;
  draft.capturing = true;
  change.disabled = true;
  kbd.textContent = "Press a chord…";
  hint.textContent = "Modifiers plus a key, or two modifiers such as Ctrl + Win.";
  void api.setHotkeyCapture(true);

  const finish = (hotkey: Hotkey | null, error?: string) => {
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("keyup", onKeyUp, true);
    draft.capturing = false;
    change.disabled = false;
    void api.setHotkeyCapture(false);
    if (hotkey) {
      draft.settings.hotkey = hotkey;
      markDirty(draft);
      refreshPills(draft);
      hint.textContent = "Done. Your new shortcut is saved.";
      hint.className = "field-note";
    } else if (error) {
      hint.textContent = error;
      hint.className = "field-note bad";
    } else {
      hint.textContent = "Click Change, then press the chord. Escape cancels.";
      hint.className = "field-note";
    }
    kbd.textContent = formatHotkey(draft.settings.hotkey);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      finish(null);
      return;
    }
    if (event.repeat || isModifierKey(event.key)) return;
    const trigger = normalizeKey(event);
    if (!trigger) {
      finish(null, "That key can't be used as the trigger.");
      return;
    }
    const hotkey: Hotkey = {
      ctrl: event.ctrlKey,
      alt: event.altKey,
      shift: event.shiftKey,
      win: event.metaKey,
      key: trigger,
    };
    if (!hotkey.ctrl && !hotkey.alt && !hotkey.shift && !hotkey.win) {
      finish(null, "Add at least one modifier, for example Ctrl + Space.");
      return;
    }
    finish(hotkey);
  };

  const onKeyUp = (event: KeyboardEvent) => {
    if (!isModifierKey(event.key)) return;
    event.preventDefault();
    const hotkey: Hotkey = {
      ctrl: event.ctrlKey || event.key === "Control",
      alt: event.altKey || event.key === "Alt",
      shift: event.shiftKey || event.key === "Shift",
      win: event.metaKey || event.key === "Meta",
      key: null,
    };
    const count = [hotkey.ctrl, hotkey.alt, hotkey.shift, hotkey.win].filter(Boolean).length;
    if (count >= 2) finish(hotkey);
  };

  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("keyup", onKeyUp, true);
}

function micPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "shortcut",
    "Microphone",
    "The mic Bolo listens to. Leave it on Automatic to follow Windows.",
  );
  panel.dataset.micPanel = "true";
  const meter = el("div", "meter");
  meter.setAttribute("role", "meter");
  meter.setAttribute("aria-label", "Microphone level");
  meter.setAttribute("aria-valuemin", "0");
  meter.setAttribute("aria-valuemax", "100");
  const fill = el("span", "meter-fill");
  fill.dataset.meterFill = "true";
  meter.append(fill);
  const select = document.createElement("select");
  select.setAttribute("aria-label", "Microphone");
  select.dataset.micSelect = "true";
  fillMicOptions(select, draft);
  select.addEventListener("change", () => {
    draft.settings.microphone = select.value || null;
    markDirty(draft);
    const note = panel.querySelector<HTMLElement>("[data-mic-note]");
    if (note) note.textContent = micSummary(draft).note;
    refreshPills(draft);
  });
  const refresh = button("Refresh list", "text");
  const note = el("p", "field-note", micSummary(draft).note);
  note.dataset.micNote = "true";
  refresh.addEventListener("click", () => {
    void (async () => {
      try {
        const status = await api.getMicStatus();
        applyDevices(draft, status.devices);
      } catch (error) {
        note.textContent = messageOf(error);
        note.className = "field-note bad";
      }
    })();
  });
  const health = el("div", "audio-fix");
  health.hidden = true;
  panel.append(meter, el("p", "field-note", "Talk and watch the bar move. If it stays still, pick another mic."), health, select, refresh, note);
  void paintMicFix(health);
  return panel;
}

function showFix(slot: HTMLElement, message: string, label: string, fix: () => Promise<string>): void {
  const text = el("p", "audio-fix-text", message);
  const action = button(label, "primary");
  action.addEventListener("click", () => {
    action.disabled = true;
    void (async () => {
      try {
        const done = await fix();
        slot.replaceChildren(el("p", "audio-fix-text ok", done));
        window.setTimeout(() => (slot.hidden = true), 2600);
      } catch (error) {
        action.disabled = false;
        text.textContent = messageOf(error);
      }
    })();
  });
  slot.replaceChildren(text, action);
  slot.hidden = false;
}

async function paintMicFix(slot: HTMLElement): Promise<void> {
  try {
    const health = await api.audioHealth();
    if (!health.micQuiet) return;
    showFix(slot, "Your microphone is turned down too low, so Bolo can't hear you.", "Fix it", async () => {
      await api.fixMicLevel();
      return "Fixed. Your microphone is ready.";
    });
  } catch {
    /* best effort */
  }
}

async function paintMeter(): Promise<void> {
  const panel = document.querySelector<HTMLElement>("[data-mic-panel]");
  const fill = document.querySelector<HTMLElement>("[data-meter-fill]");
  if (!panel || panel.hidden || !fill) return;
  try {
    const level = Math.max(0, Math.min(1, await api.micLevel()));
    fill.style.transform = `scaleX(${level})`;
    const meter = fill.parentElement;
    meter?.setAttribute("aria-valuenow", String(Math.round(level * 100)));
  } catch {
    fill.style.transform = "scaleX(0)";
  }
}

function fillMicOptions(select: HTMLSelectElement, draft: Draft): void {
  const current = draft.settings.microphone ?? "";
  select.replaceChildren();
  const auto = document.createElement("option");
  auto.value = "";
  auto.textContent = "Automatic (same as Windows)";
  select.append(auto);
  for (const mic of draft.mics) {
    const option = document.createElement("option");
    option.value = mic.name;
    option.textContent = mic.isDefault ? `${mic.name} (system default)` : mic.name;
    select.append(option);
  }
  const match = current ? draft.mics.find((mic) => normMic(mic.name) === normMic(current)) : undefined;
  if (current && !match) {
    const missing = document.createElement("option");
    missing.value = current;
    missing.textContent = `${current} (not connected)`;
    select.append(missing);
  }
  select.value = match ? match.name : current;
}

function applyDevices(draft: Draft, devices: MicInfo[]): void {
  draft.mics = devices;
  const select = document.querySelector<HTMLSelectElement>("[data-mic-select]");
  if (select) fillMicOptions(select, draft);
  const note = document.querySelector<HTMLElement>("[data-mic-note]");
  if (note) {
    note.textContent = micSummary(draft).note;
    note.className = "field-note";
  }
  refreshPills(draft);
}

function normMic(name: string): string {
  return name.trim().toLowerCase().split(/\s+/).join(" ");
}

function micSummary(draft: Draft): { value: string; note: string } {
  const pref = draft.settings.microphone;
  const def = draft.mics.find((mic) => mic.isDefault) ?? draft.mics[0];
  if (!pref) {
    if (!def) {
      return {
        value: "No microphone",
        note: "No microphone is connected. Plug one in and Bolo will use it automatically.",
      };
    }
    return {
      value: def.name,
      note: `In use: ${def.name} (system default). This follows Windows when you plug or unplug a headset.`,
    };
  }
  const match = draft.mics.find((mic) => normMic(mic.name) === normMic(pref));
  if (match) {
    return {
      value: match.name,
      note: `In use: ${match.name}. If you unplug it, Bolo uses the system default and switches back when it returns.`,
    };
  }
  if (!def) {
    return {
      value: "Unavailable",
      note: `${pref} isn't connected, and no other microphone is available.`,
    };
  }
  return {
    value: def.name,
    note: `${pref} isn't connected. Using ${def.name} until it's back.`,
  };
}

function languagePanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "languages",
    "Languages",
    "Tell Bolo what you speak and how you want your words written.",
  );
  const langLabel = el("label", "stack");
  langLabel.append(el("span", "label", "I speak"));
  const lang = document.createElement("select");
  lang.setAttribute("aria-label", "Language");
  for (const [code, name] of LANGUAGES) {
    const option = document.createElement("option");
    option.value = code;
    option.textContent = name;
    lang.append(option);
  }
  if (![...lang.options].some((option) => option.value === draft.settings.language)) {
    const extra = document.createElement("option");
    extra.value = draft.settings.language;
    extra.textContent = draft.settings.language;
    lang.append(extra);
  }
  lang.value = draft.settings.language;
  lang.addEventListener("change", () => {
    draft.settings.language = lang.value;
    markDirty(draft);
    refreshOfflineNote(draft);
  });
  langLabel.append(lang);
  const outputLabel = el("label", "stack");
  outputLabel.append(el("span", "label", "Write my words in"));
  const output = document.createElement("select");
  output.setAttribute("aria-label", "Output");
  const outputChoices: Array<["english" | "same", string]> = [
    ["english", "English"],
    ["same", "The language I spoke"],
  ];
  for (const [value, name] of outputChoices) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = name;
    output.append(option);
  }
  output.value = draft.settings.output === "same" ? "same" : "english";
  output.addEventListener("change", () => {
    draft.settings.output = output.value === "same" ? "same" : "english";
    markDirty(draft);
  });
  outputLabel.append(
    output,
    el(
      "p",
      "field-note",
      "English is the default. If you speak Urdu or Roman Urdu, Bolo writes it in English. Pick the other option to keep your own language.",
    ),
  );
  panel.append(langLabel, outputLabel);
  return panel;
}

function enginePanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "engine",
    "Speech quality and offline mode",
    "Whisper runs on Groq at temperature 0. A low-confidence or empty result is tried once more without the prompt. The dictionary and your two most recent transcripts seed that prompt.",
  );
  const models: Array<{ id: WhisperModel; title: string; body: string }> = [
    {
      id: "whisper-large-v3-turbo",
      title: "Whisper Large v3 Turbo",
      body: "Faster, and the default. Best when you want the paste to feel immediate.",
    },
    {
      id: "whisper-large-v3",
      title: "Whisper Large v3",
      body: "Slower, a bit more accurate on names and noisy audio.",
    },
  ];
  const grid = el("div", "choices");
  for (const model of models) {
    const label = document.createElement("label");
    label.className = "choice";
    const input = document.createElement("input");
    input.type = "radio";
    input.name = "model";
    input.value = model.id;
    input.checked = draft.settings.model === model.id;
    input.addEventListener("change", () => {
      draft.settings.model = model.id;
      markDirty(draft);
    });
    const text = el("span", "choice-copy");
    text.append(el("strong", "", model.title), el("span", "", model.body));
    label.append(input, text);
    grid.append(label);
  }
  const offlineLabel = el("label", "stack");
  offlineLabel.append(el("span", "label", "On-device model"));
  const offline = document.createElement("select");
  offline.setAttribute("aria-label", "On-device model");
  for (const choice of OFFLINE_MODELS) {
    const option = document.createElement("option");
    option.value = choice.id;
    option.textContent = choice.label;
    offline.append(option);
  }
  offline.value = knownOfflineModel(draft.settings.offlineModel);
  const offlineNote = el("p", "field-note", offlineModelNote(draft.settings.language, offline.value));
  offlineNote.dataset.offlineNote = "true";
  offline.addEventListener("change", () => {
    draft.settings.offlineModel = offline.value;
    markDirty(draft);
    refreshPills(draft);
    refreshOfflineNote(draft);
  });
  offlineLabel.append(offline, offlineNote);
  panel.append(grid, offlineLabel);
  return panel;
}

function cleanupPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "cleanup",
    "Cleanup and style",
    "Groq punctuates and applies spoken corrections such as “scratch that”, “new line”, and “period”. It does not add or drop what you said. Chat stays casual. Mail and documents stay formal. Code editors skip the rewrite. Verbatim pastes the transcript unchanged. If the selected model is gone, the next one in the list is tried.",
  );
  panel.append(
    switchRow(
      "Verbatim",
      "Paste exactly what was said. Cleanup will not add, drop, or rephrase a word, including um and uh.",
      draft.settings.verbatim,
      (checked) => {
        draft.settings.verbatim = checked;
        markDirty(draft);
      },
    ),
    switchRow(
      "Clean up with Groq",
      "On for a new install. A settings file that already saved this off stays off until you turn it on.",
      draft.settings.cleanupEnabled,
      (checked) => {
        draft.settings.cleanupEnabled = checked;
        markDirty(draft);
      },
    ),
  );
  const modelLabel = el("label", "stack");
  modelLabel.append(el("span", "label", "Cleanup model"));
  const model = document.createElement("select");
  model.setAttribute("aria-label", "Cleanup model");
  const modelChoices: Array<[string, string]> = [
    ["openai/gpt-oss-20b", "openai/gpt-oss-20b — fast"],
    ["openai/gpt-oss-120b", "openai/gpt-oss-120b"],
    ["qwen/qwen3.6-27b", "qwen/qwen3.6-27b"],
  ];
  if (!modelChoices.some(([id]) => id === draft.settings.cleanupModel)) {
    modelChoices.unshift([draft.settings.cleanupModel, draft.settings.cleanupModel]);
  }
  for (const [value, name] of modelChoices) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = name;
    model.append(option);
  }
  model.value = draft.settings.cleanupModel;
  model.addEventListener("change", () => {
    draft.settings.cleanupModel = model.value;
    markDirty(draft);
  });
  modelLabel.append(
    model,
    el(
      "p",
      "field-note",
      "Used for cleanup and for Urdu or Roman Urdu that is translated into English. A removed model returns 404 and Bolo tries the next one instead of skipping cleanup.",
    ),
  );
  panel.append(modelLabel);
  return panel;
}

function dictionaryPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "dictionary",
    "Dictionary",
    "One term per line. Whisper is prompted to spell them as written. Bolo also remembers capitalized names from what you dictate.",
  );
  const area = document.createElement("textarea");
  area.rows = 8;
  area.spellcheck = false;
  area.setAttribute("aria-label", "Custom dictionary");
  area.value = draft.settings.dictionary.join("\n");
  area.addEventListener("input", () => {
    draft.settings.dictionary = area.value
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
    markDirty(draft);
  });
  panel.append(area);
  return panel;
}

function characterPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "character",
    "Character & sounds",
    "Pick who listens while you talk.",
  );
  panel.classList.add("character-panel");
  const s = draft.settings;

  // Live preview of the selected character.
  const preview = el("div", "char-preview");
  const video = document.createElement("video");
  video.className = "char-preview-clip";
  video.muted = true;
  video.loop = true;
  video.autoplay = true;
  video.playsInline = true;
  video.setAttribute("aria-hidden", "true");
  const previewName = el("strong", "char-preview-name");
  const poseRow = el("div", "char-pose-row");
  let previewPose: "listen" | "write" = "listen";
  const paintPreview = () => {
    const urls = clipUrls(s.character);
    const src = previewPose === "listen" ? urls.listen : urls.write;
    if (!sameSrc(video, src)) video.src = src;
    video.playbackRate = 1;
    void video.play().catch(() => {});
    previewName.textContent = findCharacter(s.character).name;
    for (const b of poseRow.querySelectorAll<HTMLButtonElement>("button")) {
      b.classList.toggle("is-on", b.dataset.pose === previewPose);
    }
  };
  for (const [pose, label] of [["listen", "While you talk"], ["write", "While it types"]] as const) {
    const b = button(label, "ghost");
    b.dataset.pose = pose;
    b.addEventListener("click", () => {
      previewPose = pose;
      paintPreview();
    });
    poseRow.append(b);
  }
  const previewCopy = el("div", "char-preview-copy");
  previewCopy.append(el("span", "eyebrow", "Preview"), previewName, poseRow);
  preview.append(video, previewCopy);

  // Featured row.
  const featured = el("div", "char-featured");
  const paintFeatured = () => {
    featured.replaceChildren();
    const ids = FEATURED.includes(s.character) ? FEATURED : [...FEATURED.slice(0, 4), s.character];
    for (const id of ids) {
      const c = findCharacter(id);
      const card = document.createElement("button");
      card.type = "button";
      card.className = "char-feature";
      card.dataset.character = c.id;
      card.classList.toggle("is-on", c.id === s.character);
      card.setAttribute("aria-pressed", String(c.id === s.character));
      card.append(loopVideo(clipUrls(c.id).listen, "char-feature-clip"), el("span", "char-card-name", c.name));
      card.addEventListener("click", () => pickCharacter(c.id));
      featured.append(card);
    }
    if (!(panel.hidden ?? true)) {
      for (const v of featured.querySelectorAll("video")) void v.play().catch(() => {});
    }
  };
  const seeAll = button(`See all ${CHARACTERS.length} characters →`, "text");
  seeAll.classList.add("see-all");
  const all = el("div", "char-all");
  all.hidden = true;
  seeAll.addEventListener("click", () => {
    all.hidden = !all.hidden;
    seeAll.textContent = all.hidden ? `See all ${CHARACTERS.length} characters →` : "Show fewer";
  });
  const pickCharacter = (id: string) => {
    s.character = id;
    markDirty(draft);
    paintFeatured();
    paintGrid();
    paintPreview();
    paintPackHint();
    refreshHome(draft);
  };

  // Tabs and cards.
  const tabs = el("div", "char-tabs");
  tabs.setAttribute("role", "tablist");
  const grid = el("div", "char-grid");
  let group: CharacterGroup = findCharacter(s.character).group;
  const paintGrid = () => {
    grid.replaceChildren();
    for (const b of tabs.querySelectorAll<HTMLButtonElement>("button")) {
      const on = b.dataset.group === group;
      b.classList.toggle("is-on", on);
      b.setAttribute("aria-selected", String(on));
    }
    for (const c of CHARACTERS.filter((item) => item.group === group)) {
      const card = document.createElement("button");
      card.type = "button";
      card.className = "char-card";
      card.dataset.character = c.id;
      card.classList.toggle("is-on", c.id === s.character);
      card.setAttribute("aria-pressed", String(c.id === s.character));
      const img = document.createElement("img");
      img.src = clipUrls(c.id).thumb;
      img.alt = "";
      img.loading = "lazy";
      card.append(img, el("span", "char-card-name", c.name));
      if (c.id === DEFAULT_CHARACTER) card.append(el("span", "char-badge", "Default"));
      card.addEventListener("click", () => pickCharacter(c.id));
      grid.append(card);
    }
  };
  for (const [id, label] of GROUPS) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "char-tab";
    b.dataset.group = id;
    b.setAttribute("role", "tab");
    b.textContent = label;
    b.addEventListener("click", () => {
      group = id;
      paintGrid();
    });
    tabs.append(b);
  }

  // Smart toggles.
  const smart = el("div", "char-smart");
  smart.append(el("h3", "char-sub", "Smart switching"));
  smart.append(
    switchRow(
      "Time of day",
      "From 9 pm to 6 am, Night Owl Mode listens instead.",
      s.smartTimeOfDay,
      (on) => {
        s.smartTimeOfDay = on;
        markDirty(draft);
      },
    ),
    switchRow(
      "Holiday",
      "On Eid, the Eid skin takes over for three days.",
      s.smartHoliday,
      (on) => {
        s.smartHoliday = on;
        markDirty(draft);
      },
    ),
  );
  const mood = switchRow("Mood mirror · coming soon", "The character will react to your tone of voice.", false, () => {});
  mood.classList.add("is-soon");
  const moodInput = mood.querySelector("input");
  if (moodInput) moodInput.disabled = true;
  smart.append(mood);

  // Sounds.
  const sounds = el("div", "card rows sounds-card");
  const soundHead = el("div", "row-head");
  const headCopy = el("span", "switch-copy");
  headCopy.append(el("strong", "", "Sounds"), el("span", "", "A soft sound when Bolo starts and finishes"));
  const select = document.createElement("select");
  select.setAttribute("aria-label", "Sound pack");
  for (const [id, name] of SOUND_PACKS) {
    if (id === "off") continue;
    const option = document.createElement("option");
    option.value = id;
    option.textContent = id === "match" ? "Match my character" : name;
    select.append(option);
  }
  const lastPack = () => window.localStorage.getItem("bolo.lastPack") || "match";
  select.value = s.soundPack === "off" ? lastPack() : s.soundPack;
  let previewAudio: HTMLAudioElement | null = null;
  const playCue = (cue: "start" | "done" | "dismiss") => {
    const pack = resolvePack(select.value, s.character);
    const url = soundUrl(pack, cue);
    if (!url) return;
    previewAudio?.pause();
    previewAudio = new Audio(url);
    previewAudio.volume = s.soundVolume;
    void previewAudio.play().catch(() => {});
  };
  const play = button("▶ Play", "ghost");
  play.setAttribute("aria-label", "Play the start and finish sounds");
  const speakerFix = el("div", "audio-fix");
  speakerFix.hidden = true;
  const playBoth = () => {
    playCue("start");
    window.setTimeout(() => playCue("done"), 900);
  };
  play.addEventListener("click", () => {
    playBoth();
    void (async () => {
      try {
        const health = await api.audioHealth();
        if (!health.outputQuiet) {
          speakerFix.hidden = true;
          return;
        }
        showFix(speakerFix, "Your speakers are muted or turned all the way down, so you can't hear Bolo.", "Turn sound up", async () => {
          await api.fixOutputVolume();
          playBoth();
          return "Sound is on.";
        });
      } catch {
        /* volume check is best effort */
      }
    })();
  });
  const pickRow = el("div", "row-controls");
  pickRow.append(select, play);
  queueMicrotask(() => pickRow.after(speakerFix));
  soundHead.append(headCopy, pickRow);
  const packHint = el("p", "field-note");
  const paintPackHint = () => {
    const resolved = resolvePack(select.value, s.character);
    const name = SOUND_PACKS.find(([id]) => id === resolved)?.[1] ?? "";
    packHint.textContent = select.value === "match" ? `Now: ${name}.` : SOUND_PACKS.find(([id]) => id === select.value)?.[2] ?? "";
  };
  select.addEventListener("change", () => {
    window.localStorage.setItem("bolo.lastPack", select.value);
    if (s.soundPack !== "off") {
      s.soundPack = select.value;
      markDirty(draft);
    }
    paintPackHint();
  });

  const cueList = el("div", "cue-list");
  for (const [cue, title] of [
    ["start", "Start"],
    ["done", "Finish"],
    ["dismiss", "Cancel"],
  ] as const) {
    const b = button(`▶ ${title}`, "text");
    b.classList.add("cue-play");
    b.setAttribute("aria-label", `Play the ${title.toLowerCase()} sound`);
    b.addEventListener("click", () => playCue(cue));
    cueList.append(b);
  }

  const volume = document.createElement("label");
  volume.className = "volume-field";
  const volumeValue = el("span", "volume-value", `${Math.round(s.soundVolume * 100)}%`);
  const range = document.createElement("input");
  range.type = "range";
  range.min = "0";
  range.max = "100";
  range.step = "5";
  range.value = String(Math.round(s.soundVolume * 100));
  range.setAttribute("aria-label", "Sound volume");
  range.addEventListener("input", () => {
    s.soundVolume = Number(range.value) / 100;
    volumeValue.textContent = `${range.value}%`;
    markDirty(draft);
  });
  volume.append(el("span", "volume-label", "Volume"), range, volumeValue);
  const detail = el("div", "sound-detail");
  detail.append(packHint, cueList, volume);
  const paintSoundsOn = () => {
    const on = s.soundPack !== "off";
    select.disabled = !on;
    play.disabled = !on;
    detail.hidden = !on;
  };
  const onSwitch = switchRow("Play sounds", "", s.soundPack !== "off", (on) => {
    if (on) s.soundPack = select.value || lastPack();
    else {
      window.localStorage.setItem("bolo.lastPack", select.value);
      s.soundPack = "off";
    }
    markDirty(draft);
    paintSoundsOn();
  });
  sounds.append(soundHead, onSwitch, detail);
  paintSoundsOn();

  smart.classList.add("card", "rows");
  all.append(tabs, grid);
  panel.append(preview, featured, seeAll, all, sounds, smart);
  paintFeatured();
  paintGrid();
  paintPreview();
  paintPackHint();
  return panel;
}

function historyPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "history",
    "History",
    "The last 50 things you said, newest first. They stay on this PC only.",
  );
  const list = el("div", "history-list");
  list.dataset.history = "true";
  fillHistory(list, draft);
  const clear = button("Clear history", "ghost");
  clear.addEventListener("click", () => void clearHistory(draft, list, panel));
  panel.append(list, clear);
  return panel;
}

function privacyPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "privacy",
    "Privacy",
    "Audio and the transcript go to Groq, using a key stored on this PC, unless Groq can't take the request. Then the same audio stays on this PC and whisper.cpp transcribes it. Bolo does not store audio or text on a server of its own, and it does not train a voice model. Recordings are deleted after a paste unless you turn the switch below on. There is no telemetry.",
  );
  const list = el("div", "history-list");
  list.dataset.historyPrivacy = "true";
  panel.append(
    switchRow(
      "Keep history on this PC",
      "On by default. Turn this off to stop saving new transcripts. Transcripts already saved stay until you clear them.",
      draft.settings.historyEnabled,
      (checked) => {
        draft.settings.historyEnabled = checked;
        markDirty(draft);
        const list = document.querySelector<HTMLElement>("[data-history]");
        if (list) fillHistory(list, draft);
      },
    ),
  );
  const clear = button("Clear history", "ghost");
  clear.addEventListener("click", () => {
    const list = document.querySelector<HTMLElement>("[data-history]");
    if (list) void clearHistory(draft, list, panel);
  });
  panel.append(
    switchRow(
      "Keep recordings on this PC",
      "Off unless you turn it on. Bolo then keeps the last few takes so Listen again can play them. They are never uploaded.",
      draft.settings.keepRecordings,
      (checked) => {
        draft.settings.keepRecordings = checked;
        markDirty(draft);
      },
    ),
  );
  panel.append(clear, el("p", "field-note", "Clear removes every saved transcript from this computer. It does not contact Groq."));
  list.hidden = true;
  return panel;
}

async function clearHistory(draft: Draft, list: HTMLElement, host: HTMLElement): Promise<void> {
  if (draft.history.length === 0) return;
  if (!window.confirm("Clear every saved transcript on this computer?")) return;
  try {
    await api.clearHistory();
    draft.history = [];
    fillHistory(list, draft);
  } catch (error) {
    host.append(el("p", "field-note bad", messageOf(error)));
  }
}

function diagnosticsPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "diagnostics",
    "Help and diagnostics",
    "Push-to-talk dictation for Windows. Hold the hotkey, speak, and the transcript is pasted at the cursor.",
  );
  const facts = el("dl", "facts");
  facts.append(
    fact("Version", draft.info.version),
    fact("Transcription", "Groq Whisper, temperature 0"),
    fact("Cleanup", "Groq openai/gpt-oss-20b"),
    fact("Audio", "16 kHz mono FLAC, WAV if encoding fails"),
    fact("Shortcut listener", draft.info.listenerRunning ? "Running" : "Not running"),
    fact("Last problem", visibleMessage(draft.lastError) ?? "None"),
    fact("Log file", "%APPDATA%\\com.bolo.dictation\\bolo.log"),
  );
  const copy = button("Copy a report", "ghost");
  const note = el("p", "field-note", "Copies version, status, and settings (never your key) so you can paste them into a message.");
  copy.addEventListener("click", () => {
    const report = [
      `Bolo ${draft.info.version} on ${draft.info.platform}`,
      `Listener: ${draft.info.listenerRunning ? "running" : "not running"} ${draft.info.listenerError ?? ""}`.trim(),
      `Last error: ${draft.lastError ?? "none"}`,
      `Key: ${keyPill(draft.keyStatus).value}`,
      `Mic: ${micSummary(draft).value}`,
      `Settings: ${JSON.stringify({ ...draft.settings, dictionary: draft.settings.dictionary.length })}`,
    ].join("\n");
    void api
      .copyText(report)
      .then(() => {
        note.textContent = "Report copied.";
        note.className = "field-note good";
      })
      .catch((error) => {
        note.textContent = messageOf(error);
        note.className = "field-note bad";
      });
  });
  panel.append(facts, tryButton(draft, "Record a test", "ghost"), copy, note);
  return panel;
}

function fact(label: string, value: string): HTMLElement {
  const row = el("div", "fact");
  row.append(el("dt", "", label), el("dd", "", value));
  return row;
}

function fillHistory(list: HTMLElement, draft: Draft): void {
  list.replaceChildren();
  if (!draft.settings.historyEnabled) {
    list.append(el("p", "empty-title", "History is off. New dictation is not saved on this PC."));
  }
  if (draft.history.length === 0) {
    const empty = el("div", "empty");
    empty.append(
      el("p", "empty-title", draft.settings.historyEnabled ? "Nothing dictated yet." : "No saved transcripts."),
      el("p", "", `Hold ${formatHotkey(draft.settings.hotkey)} in any app, speak, and release.`),
    );
    list.append(empty);
    return;
  }
  for (const entry of draft.history) {
    const item = el("article", "history-item");
    const body = el("div");
    body.append(el("p", "history-text", entry.text));
    body.append(
      el(
        "p",
        "history-meta",
        `${formatWhen(entry.createdUnixMs)} · ${formatDuration(entry.durationMs)} · ${modelLabel(entry.model)}${entry.cleaned ? " · cleaned" : ""}`,
      ),
    );
    const actions = el("div", "history-actions");
    const copy = button("Copy", "ghost");
    copy.addEventListener("click", () => {
      void api
        .copyText(entry.text)
        .then(() => {
          copy.textContent = "Copied";
          window.setTimeout(() => {
            copy.textContent = "Copy";
          }, 1200);
        })
        .catch((error) => {
          copy.textContent = messageOf(error);
        });
    });
    const remove = button("Delete", "text");
    remove.addEventListener("click", () => {
      const previous = draft.history.slice();
      draft.history = draft.history.filter((row) => row.id !== entry.id);
      fillHistory(list, draft);
      void api.deleteHistoryEntry(entry.id).catch((error) => {
        draft.history = previous;
        fillHistory(list, draft);
        list.prepend(el("p", "field-note bad", messageOf(error)));
      });
    });
    const fix = button("Fix", "ghost");
    fix.title = "Listen again or type the right words";
    fix.addEventListener("click", () => void openFixDialog(draft, entry.id));
    actions.append(fix, copy, remove);
    item.append(body, actions);
    list.append(item);
  }
}

async function refreshHistory(draft: Draft): Promise<void> {
  try {
    draft.history = await api.getHistory();
    const list = document.querySelector<HTMLElement>("[data-history]");
    if (list) fillHistory(list, draft);
  } catch {
    // The next visit reloads history. A failed refresh should not wipe the list.
  }
}

let saveTimer = 0;
let saveChain: Promise<void> = Promise.resolve();

async function save(draft: Draft): Promise<void> {
  const status = document.querySelector<HTMLElement>("[data-save-status]");
  try {
    await api.saveSettings(draft.settings);
    draft.dirty = false;
    const saved = normalizeSettings(await api.getSettings());
    // Keep edits made while this save was in flight.
    // Update in place: panels keep a reference to this object.
    if (!draft.dirty) Object.assign(draft.settings, saved);
    void announceSettings();
    if (status) {
      status.textContent = "Saved";
      status.className = "save-status good is-shown";
      window.setTimeout(() => status.classList.remove("is-shown"), 1600);
    }
    refreshPills(draft);
  } catch (error) {
    if (status) {
      status.textContent = messageOf(error);
      status.className = "save-status bad is-shown";
    }
  }
}

/** Compare against the attribute: `.src` is absolute, Vite URLs are not. */
function sameSrc(video: HTMLVideoElement, src: string): boolean {
  return video.getAttribute("src") === src;
}

/** Tell the overlay right away so the next take uses the new character and sounds. */
async function announceSettings(): Promise<void> {
  if (!("__TAURI_INTERNALS__" in window)) return;
  try {
    const { emit } = await import("@tauri-apps/api/event");
    await emit("bolo-settings-changed");
  } catch {
    // The overlay also re-reads settings at the start of each take.
  }
}

function markDirty(draft: Draft): void {
  draft.dirty = true;
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    saveChain = saveChain.then(() => save(draft));
  }, 450);
}

function keyPill(status: ApiKeyStatus): { value: string; tone: string } {
  if (status.configured) return { value: status.hint ?? "Saved", tone: "good" };
  if (status.builtin) return { value: "Built-in", tone: "good" };
  return { value: "Not set", tone: "warn" };
}

function syncKeyControls(draft: Draft): void {
  const note = document.querySelector<HTMLElement>("[data-key-note]");
  if (note) {
    note.textContent = keyNote(draft.keyStatus);
    note.className = draft.keyStatus.error ? "field-note bad" : "field-note";
  }
  const clear = document.querySelector<HTMLButtonElement>("[data-key-clear]");
  if (clear) {
    clear.textContent = draft.keyStatus.builtin ? "Return to built-in key" : "Remove";
    clear.hidden = draft.keyStatus.builtin && !draft.keyStatus.configured;
    clear.disabled = !draft.keyStatus.configured;
  }
  const label = document.querySelector<HTMLElement>("[data-own-key-label]");
  if (label) label.hidden = !draft.keyStatus.builtin;
  refreshPills(draft);
}

function keyNote(status: ApiKeyStatus): string {
  if (status.error) return status.error;
  if (status.configured) {
    const stored = `Key ${status.hint ?? ""} is in the OS credential store.`;
    if (status.builtin) return `${stored} Bolo is using it instead of the built-in key.`;
    return stored;
  }
  if (status.builtin) return "Using Bolo built-in key";
  return "Paste a key from console.groq.com. Nothing is sent until you dictate.";
}

function panelEl(id: SectionId, title: string, body: string): HTMLElement {
  const section = el("section", "card");
  section.dataset.panel = id;
  const head = el("div", "card-head");
  if (ADVANCED.some(([sub]) => sub === id)) {
    const back = button("‹ Advanced", "text");
    back.classList.add("back-link");
    back.addEventListener("click", () => {
      const draft = currentDraft;
      if (draft) go(draft, "advanced");
    });
    section.append(back);
  }
  head.append(el("h2", "", title));
  section.append(head, el("p", "card-body", body));
  return section;
}

let currentDraft: Draft | null = null;

function el(tag: string, className?: string, text?: string): HTMLElement {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

function button(text: string, kind: "primary" | "ghost" | "text"): HTMLButtonElement {
  const node = document.createElement("button");
  node.type = "button";
  node.className = kind;
  node.textContent = text;
  return node;
}

function logoHeading(): HTMLElement {
  const heading = el("h1", "wordmark");
  const img = document.createElement("img");
  img.className = "brand-lockup";
  img.src = lockupUrl;
  img.alt = "Bolo";
  heading.append(img);
  return heading;
}

function switchRow(title: string, body: string, checked: boolean, onChange: (value: boolean) => void): HTMLElement {
  const label = document.createElement("label");
  label.className = "switch-row";
  const text = el("span", "switch-copy");
  text.append(el("strong", "", title));
  if (body) text.append(el("span", "", body));
  const input = document.createElement("input");
  input.type = "checkbox";
  input.className = "switch";
  input.setAttribute("role", "switch");
  input.checked = checked;
  input.addEventListener("change", () => onChange(input.checked));
  label.append(text, input);
  return label;
}

function loopVideo(src: string, className: string): HTMLVideoElement {
  const video = document.createElement("video");
  video.className = className;
  video.src = src;
  video.muted = true;
  video.loop = true;
  video.autoplay = true;
  video.playsInline = true;
  video.setAttribute("aria-hidden", "true");
  return video;
}

function homePanel(draft: Draft): HTMLElement {
  const panel = el("section", "home");
  panel.dataset.panel = "home";
  const head = el("div", "page-head");
  head.append(el("h2", "", "Bolo is ready"), el("p", "card-body", "Talk anywhere on your PC. Bolo types it for you."));
  const hero = el("div", "card hero");
  const video = loopVideo(clipUrls(activeCharacter(draft.settings)).listen, "hero-clip");
  video.dataset.homeClip = "true";
  const copy = el("div", "hero-copy");
  const ok = el("p", "ready-dot");
  ok.dataset.homeStatus = "true";
  ok.textContent = draft.info.listenerRunning ? "Listening for your shortcut" : "Starting up…";
  const line = el("p", "hero-line");
  const keys = el("span", "hero-keys", formatHotkey(draft.settings.hotkey));
  keys.dataset.homeKeys = "true";
  line.append("Hold ", keys, " and talk");
  copy.append(ok, line, tryButton(draft, "Try it now", "primary"));
  hero.append(video, copy);
  const share = el("div", "card share-card");
  share.append(
    el("h3", "", "Share Bolo"),
    el("p", "", "A short note in Urdu and English, with the Store link. Nothing is added to what you dictate."),
  );
  const shareActions = el("div", "share-actions");
  const whatsapp = button("WhatsApp", "primary");
  const copyMessage = button("Copy message", "ghost");
  const shareCharacter = button("Share my character", "ghost");
  const shareStatus = el("p", "share-status");
  shareStatus.dataset.shareStatus = "true";
  whatsapp.addEventListener("click", () => {
    shareStatus.textContent = "";
    void api.openShareWhatsapp().then(() => {
      shareStatus.textContent = "Opening WhatsApp.";
    }).catch(() => {
      shareStatus.textContent = "Couldn't open WhatsApp. The message is copied instead.";
      void api.copyShareMessage().catch(() => {});
    });
  });
  copyMessage.addEventListener("click", () => {
    void api.copyShareMessage().then(() => {
      shareStatus.textContent = "Copied.";
    }).catch(() => {
      shareStatus.textContent = "Couldn't copy the message.";
    });
  });
  shareCharacter.addEventListener("click", () => {
    shareStatus.textContent = "";
    void saveCharacterPicture(draft, shareStatus);
  });
  shareActions.append(whatsapp, copyMessage, shareCharacter);
  share.append(shareActions, shareStatus);
  const startup = el("div", "card rows");
  startup.append(
    switchRow("Start Bolo when my PC turns on", "", draft.settings.launchAtStartup, (on) => {
      draft.settings.launchAtStartup = on;
      markDirty(draft);
    }),
  );
  panel.append(head, hero, share, startup);
  const slot = el("div", "rate-slot");
  slot.dataset.rateSlot = "true";
  panel.prepend(slot);
  void fillHome(slot);
  return panel;
}

async function saveCharacterPicture(draft: Draft, status: HTMLElement): Promise<void> {
  const thumb = clipUrls(activeCharacter(draft.settings)).thumb;
  if (!thumb) {
    status.textContent = "This character doesn't have a picture yet.";
    return;
  }
  try {
    const response = await fetch(thumb);
    const bytes = Array.from(new Uint8Array(await response.arrayBuffer()));
    const saved = await api.saveCharacterCard(bytes);
    status.textContent = saved ? "Saved, with bolo.thebetawebsite.com on the picture." : "";
  } catch (error) {
    const message = error instanceof Error ? error.message : "";
    status.textContent = message && !message.includes("canceled") ? message : "The picture wasn't saved.";
  }
}

async function fillHome(slot: HTMLElement): Promise<void> {
  let card: Awaited<ReturnType<typeof api.homeCard>> = null;
  try {
    card = await api.homeCard();
  } catch {
    card = null;
  }
  slot.replaceChildren();
  if (!card) return;
  if (card.kind === "milestone") {
    const box = el("section", "rate-card");
    box.append(el("h3", "", card.title), el("p", "", card.body));
    const actions = el("div", "rate-actions");
    const whatsapp = button("WhatsApp", "primary");
    const copyMessage = button("Copy message", "ghost");
    const dismiss = button("Dismiss", "ghost");
    const count = card.count;
    whatsapp.addEventListener("click", () => {
      void api.openShareWhatsapp().catch(() => api.copyShareMessage().catch(() => {}));
    });
    copyMessage.addEventListener("click", () => {
      void api.copyShareMessage().catch(() => {});
    });
    dismiss.addEventListener("click", () => {
      slot.replaceChildren();
      void api.dismissMilestone(count).catch(() => {});
    });
    actions.append(whatsapp, copyMessage, dismiss);
    box.append(actions);
    slot.append(box);
    return;
  }
  const rating = card;
  const box = el("section", "rate-card");
  box.dataset.rateCard = "true";
  if (rating.rtl) box.dir = "rtl";
  box.append(el("h3", "", rating.title), el("p", "", rating.body));
  const actions = el("div", "rate-actions");
  const rate = button(rating.rateLabel, "primary");
  const later = button(rating.laterLabel, "ghost");
  const feedback = button(rating.feedbackLabel, "text");
  feedback.classList.add("rate-feedback");
  rate.addEventListener("click", () => {
    rate.disabled = true;
    later.disabled = true;
    void api.rateBolo().then((result) => {
      if (result.status === "rated" && result.message) {
        box.replaceChildren(el("p", "rate-thanks", result.message));
        return;
      }
      slot.replaceChildren();
    }).catch(() => {
      slot.replaceChildren();
    });
  });
  later.addEventListener("click", () => {
    slot.replaceChildren();
    void api.ratingNotNow().catch(() => {});
  });
  feedback.addEventListener("click", () => {
    void api.ratingFeedback().catch(() => {});
  });
  actions.append(rate, later);
  box.append(actions, feedback);
  slot.append(box);
}

function refreshHome(draft: Draft): void {
  const clip = document.querySelector<HTMLVideoElement>("[data-home-clip]");
  const src = clipUrls(activeCharacter(draft.settings)).listen;
  if (clip && !sameSrc(clip, src)) clip.src = src;
  const keys = document.querySelector<HTMLElement>("[data-home-keys]");
  if (keys) keys.textContent = formatHotkey(draft.settings.hotkey);
  const status = document.querySelector<HTMLElement>("[data-home-status]");
  if (status) {
    status.textContent = draft.info.listenerRunning ? "Listening for your shortcut" : "The shortcut isn't working right now";
    status.classList.toggle("is-off", !draft.info.listenerRunning);
  }
}

function accuracyPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "accuracy",
    "Accuracy",
    "Debug traces and test recordings stay on this PC. Nothing is uploaded, and your key is never written into them.",
  );
  panel.append(
    switchRow(
      "Save a debug trace for each dictation",
      "Off unless you turn this on. Each take saves the microphone audio, the audio after gain, and the text after each stage.",
      draft.settings.debugCapture,
      (checked) => {
        draft.settings.debugCapture = checked;
        markDirty(draft);
      },
    ),
  );
  const debugActions = el("div", "share-actions");
  const openDebug = button("Open debug folder", "ghost");
  const clearDebug = button("Delete debug traces", "ghost");
  const debugNote = el("p", "field-note", "");
  openDebug.addEventListener("click", () => {
    debugNote.textContent = "";
    void api.openDebugFolder().catch((error) => {
      debugNote.textContent = messageOf(error);
      debugNote.className = "field-note bad";
    });
  });
  clearDebug.addEventListener("click", () => {
    if (!window.confirm("Delete every debug trace on this computer?")) return;
    void api
      .clearDebugFolder()
      .then(() => {
        debugNote.textContent = "Debug traces deleted. They were only on this PC.";
        debugNote.className = "field-note good";
      })
      .catch((error) => {
        debugNote.textContent = messageOf(error);
        debugNote.className = "field-note bad";
      });
  });
  debugActions.append(openDebug, clearDebug);
  panel.append(debugActions, debugNote);

  const head = el("h3", "", "Record the test sentences");
  const setLabel = el("label", "stack");
  setLabel.append(el("span", "label", "Sentence set"));
  const setPicker = document.createElement("select");
  setPicker.setAttribute("aria-label", "Sentence set");
  for (const set of SCRIPT_SETS) {
    const option = document.createElement("option");
    option.value = set.id;
    option.textContent = set.label;
    setPicker.append(option);
  }
  setLabel.append(
    setPicker,
    el(
      "p",
      "field-note",
      "The original 30 stay unchanged so older scores can be compared. The second set is Urdu, mixed Urdu and English, Hindi, and English names.",
    ),
  );
  const progress = el("p", "field-note", "");
  const cue = el("p", "card-body", "");
  const say = el("p", "accuracy-say", "Loading sentences…");
  say.dir = "auto";
  const takeNote = el("p", "field-note", "");
  const actions = el("div", "share-actions");
  const record = button("Record", "primary");
  const redo = button("Redo", "ghost");
  const save = button("Save and next", "ghost");
  redo.disabled = true;
  save.disabled = true;
  actions.append(record, redo, save);
  const folderActions = el("div", "share-actions");
  const openSet = button("Open recordings", "ghost");
  const clearSet = button("Delete recordings", "ghost");
  folderActions.append(openSet, clearSet);
  panel.append(head, setLabel, progress, cue, say, takeNote, actions, folderActions);

  let lines: ScriptLine[] = [];
  let setId = "original";
  let saved = new Set<string>();
  let index = 0;
  let recording = false;
  let starting = false;
  let holding = false;

  const show = () => {
    const line = lines[index];
    const done = lines.filter((item) => saved.has(item.id)).length;
    progress.textContent = lines.length ? `${done} of ${lines.length} saved` : "";
    if (!lines.length) {
      cue.textContent = "";
      record.disabled = true;
      return;
    }
    if (!line) {
      cue.textContent = "";
      say.textContent = "That's the set. The recordings are on this PC, not in the project folder.";
      record.disabled = true;
      return;
    }
    cue.textContent = sentenceCue(line.bucket || line.expectedLanguage, line.cue);
    say.textContent = line.say;
    record.disabled = starting;
    record.textContent = recording ? "Stop" : holding ? "Record again" : saved.has(line.id) ? "Record again" : "Record";
  };

  const jumpToPending = () => {
    const pending = lines.findIndex((line) => !saved.has(line.id));
    index = pending === -1 ? Math.max(lines.length - 1, 0) : pending;
    holding = false;
    save.disabled = true;
    redo.disabled = true;
    show();
  };

  record.addEventListener("click", () => {
    takeNote.textContent = "";
    takeNote.className = "field-note";
    if (!recording) {
      recording = true;
      starting = true;
      holding = false;
      save.disabled = true;
      redo.disabled = true;
      show();
      void api.accuracyRecordStart().then(() => {
        starting = false;
        show();
      }).catch((error) => {
        recording = false;
        starting = false;
        show();
        takeNote.textContent = messageOf(error);
        takeNote.className = "field-note bad";
      });
      return;
    }
    record.disabled = true;
    void api
      .accuracyRecordStop()
      .then((info: HeldInfo) => {
        recording = false;
        holding = true;
        record.disabled = false;
        redo.disabled = false;
        save.disabled = false;
        const peak = info.peak.toFixed(3);
        takeNote.textContent = info.silent
          ? `${info.deviceName} looks silent (peak ${peak}). A headset jack often records nothing. Redo on another microphone.`
          : `${info.deviceName} · ${info.sampleRate} Hz · peak ${peak} · clipping ${info.clippingPct.toFixed(2)}%`;
        takeNote.className = info.silent ? "field-note bad" : "field-note";
        show();
      })
      .catch((error) => {
        recording = false;
        record.disabled = false;
        show();
        takeNote.textContent = messageOf(error);
        takeNote.className = "field-note bad";
      });
  });
  redo.addEventListener("click", () => {
    if (recording || !lines[index]) return;
    holding = false;
    save.disabled = true;
    redo.disabled = true;
    record.click();
  });
  save.addEventListener("click", () => {
    const line = lines[index];
    if (!line || !holding) return;
    save.disabled = true;
    void api
      .accuracySave(line.id)
      .then(() => {
        saved.add(line.id);
        holding = false;
        redo.disabled = true;
        takeNote.textContent = "Saved on this PC.";
        takeNote.className = "field-note good";
        const next = lines.findIndex((item, at) => at > index && !saved.has(item.id));
        index = next === -1 ? lines.length : next;
        show();
      })
      .catch((error) => {
        save.disabled = false;
        takeNote.textContent = messageOf(error);
        takeNote.className = "field-note bad";
      });
  });
  openSet.addEventListener("click", () => {
    void api.accuracyOpenFolder().catch((error) => {
      takeNote.textContent = messageOf(error);
      takeNote.className = "field-note bad";
    });
  });
  clearSet.addEventListener("click", () => {
    if (!window.confirm("Delete the test recordings on this computer?")) return;
    void api
      .accuracyClearFolder()
      .then(() => {
        saved = new Set();
        holding = false;
        index = 0;
        takeNote.textContent = "Recordings deleted. They were only on this PC.";
        takeNote.className = "field-note";
        show();
      })
      .catch((error) => {
        takeNote.textContent = messageOf(error);
        takeNote.className = "field-note bad";
      });
  });

  const load = () => {
    const requested = setId;
    void api
      .accuracyScript(requested)
      .then(async (script) => {
        if (requested !== setId) return;
        lines = script;
        try {
          saved = new Set(await api.accuracySaved());
        } catch {
          saved = new Set();
        }
        jumpToPending();
      })
      .catch((error) => {
        if (requested !== setId) return;
        say.textContent = messageOf(error);
      });
  };
  setPicker.addEventListener("change", () => {
    setId = scriptSetId(setPicker.value);
    if (recording) {
      recording = false;
      void api.accuracyDiscard();
    }
    holding = false;
    starting = false;
    save.disabled = true;
    redo.disabled = true;
    lines = [];
    takeNote.textContent = "";
    say.textContent = "Loading sentences…";
    show();
    load();
  });
  load();
  return panel;
}

function refreshOfflineNote(draft: Draft): void {
  const note = document.querySelector<HTMLElement>("[data-offline-note]");
  if (!note) return;
  note.textContent = offlineModelNote(draft.settings.language, draft.settings.offlineModel);
}

function advancedPanel(draft: Draft): HTMLElement {
  const panel = el("section", "advanced");
  panel.dataset.panel = "advanced";
  const head = el("div", "page-head");
  head.append(el("h2", "", "Advanced"), el("p", "card-body", "You don't need to change anything here. Bolo works fine as it is."));
  const rows = el("div", "card rows");
  for (const [id, title, body] of ADVANCED) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "link-row";
    const text = el("span", "switch-copy");
    text.append(el("strong", "", title), el("span", "", body));
    row.append(text, el("span", "chev", "›"));
    row.addEventListener("click", () => go(draft, id));
    rows.append(row);
  }
  panel.append(head, rows);
  return panel;
}

function loadingShell(): HTMLElement {
  const shell = el("div", "splash");
  shell.setAttribute("role", "status");
  shell.setAttribute("aria-live", "polite");
  const inner = el("div", "splash-inner");
  const mark = el("div", "splash-mark");
  const img = document.createElement("img");
  img.className = "splash-logo";
  img.src = lockupUrl;
  img.alt = "Bolo";
  mark.append(img);
  const waves = el("div", "splash-waves");
  waves.setAttribute("aria-hidden", "true");
  for (let i = 0; i < 5; i += 1) {
    const bar = el("span", "");
    bar.style.setProperty("--i", String(i));
    waves.append(bar);
  }
  const progress = el("div", "splash-bar");
  progress.setAttribute("aria-hidden", "true");
  progress.append(el("span", ""));
  inner.append(el("p", "eyebrow splash-eyebrow", "Dictation"), mark, waves, el("p", "splash-text", "Loading your settings…"), progress);
  shell.append(inner);
  return shell;
}

/** Fade the splash out, only if it became visible (it fades in after 150 ms). */
function leaveSplash(splash: HTMLElement, since: number): Promise<void> {
  const shown = performance.now() - since > 160;
  if (!shown || matchMedia("(prefers-reduced-motion: reduce)").matches) return Promise.resolve();
  splash.classList.add("leaving");
  return new Promise((resolve) => setTimeout(resolve, 180));
}

function errorShell(message: string, retry: () => void): HTMLElement {
  const shell = el("div", "shell error-shell");
  shell.append(el("p", "eyebrow", "Dictation"), logoHeading());
  shell.append(el("p", "lede", "Settings didn't load."));
  const banner = el("div", "banner bad", visibleMessage(message) ?? "Settings didn't load.");
  const retryButton = button("Try again", "primary");
  retryButton.addEventListener("click", retry);
  shell.append(banner, retryButton);
  return shell;
}

function messageOf(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  if (typeof error === "string" && error.trim()) return error.trim();
  return "Something went wrong.";
}

function isModifierKey(key: string): boolean {
  return key === "Control" || key === "Alt" || key === "Shift" || key === "Meta";
}

function normalizeKey(event: KeyboardEvent): string | null {
  if (event.code === "Space") return "space";
  if (/^F\d{1,2}$/.test(event.key)) return event.key.toLowerCase();
  if (/^Key[A-Z]$/.test(event.code)) return event.code.slice(3).toLowerCase();
  if (/^Digit[0-9]$/.test(event.code)) return event.code.slice(5);
  const map: Record<string, string> = {
    Tab: "tab",
    Enter: "enter",
    Backspace: "backspace",
    Delete: "delete",
    Insert: "insert",
    Home: "home",
    End: "end",
    PageUp: "pageup",
    PageDown: "pagedown",
    ArrowLeft: "left",
    ArrowRight: "right",
    ArrowUp: "up",
    ArrowDown: "down",
    Escape: "esc",
  };
  return map[event.key] ?? null;
}

// ---- What Bolo has learned -------------------------------------------------

let memoryView: MemoryView | null = null;

function learnedPanel(_draft: Draft): HTMLElement {
  const panel = panelEl(
    "learned",
    "What Bolo has learned",
    "Spellings you correct stay on this PC. Bolo does not train on your voice, and it does not read other apps.",
  );
  panel.classList.add("learned-panel");
  const body = el("div", "learned-body");
  body.dataset.learned = "true";
  body.append(el("p", "field-note", "Loading…"));
  panel.append(body);
  return panel;
}

function wordsPanel(_draft: Draft): HTMLElement {
  const panel = el("section", "my-words");
  panel.dataset.panel = "words";
  const head = el("div", "page-head");
  head.append(
    el("p", "eyebrow", "Spelling"),
    el("h2", "", "Words Bolo should get right"),
    el(
      "p",
      "card-body",
      "These are already in. Say them the way you normally would. The ones you use most are the ones Bolo listens for first.",
    ),
  );
  const body = el("div", "my-words-body");
  body.dataset.myWords = "true";
  body.append(el("p", "field-note", "Loading…"));
  panel.append(head, body);
  return panel;
}

async function refreshMemory(draft: Draft, view?: MemoryView): Promise<void> {
  try {
    memoryView = view ?? (await api.getMemory());
  } catch (error) {
    const message = el("p", "field-note bad", messageOf(error));
    document.querySelector<HTMLElement>("[data-learned]")?.replaceChildren(message.cloneNode(true));
    document.querySelector<HTMLElement>("[data-my-words]")?.replaceChildren(message);
    return;
  }
  // The backend already saved these. Keep the draft in step so autosave
  // never writes back an older list.
  draft.settings.dictionary = [...memoryView.words];
  draft.settings.keepLearning = memoryView.keepLearning;
  const area = document.querySelector<HTMLTextAreaElement>('[aria-label="Custom dictionary"]');
  if (area && document.activeElement !== area) area.value = memoryView.words.join("\n");
  fillLearned(draft);
  fillMyWords(draft);
}

function fillMyWords(draft: Draft): void {
  const body = document.querySelector<HTMLElement>("[data-my-words]");
  const view = memoryView;
  if (!body || !view) return;
  body.replaceChildren();
  const list = document.createElement("ul");
  list.className = "word-list";
  const entries = view.entries ?? [];
  if (entries.length === 0) {
    list.append(el("li", "field-note", "Nothing here yet."));
  }
  for (const entry of entries) {
    const row = document.createElement("li");
    row.className = "word-row";
    const name = el("span", "word-name", entry.word);
    const side = el("span", "word-side");
    if (!entry.seed) {
      const remove = button("Remove", "text");
      remove.className = "word-remove";
      remove.setAttribute("aria-label", `Remove ${entry.word}`);
      remove.addEventListener("click", () => {
        myWordsAction(draft, () => api.forgetWord(entry.word));
      });
      side.append(remove);
    }
    const tag = el("span", entry.tag === "Used a lot" ? "word-tag is-hot" : "word-tag is-ready", entry.tag);
    side.append(tag);
    row.append(name, side);
    list.append(row);
  }
  const form = el("div", "word-add");
  const input = document.createElement("input");
  input.type = "text";
  input.placeholder = "Add a name or a site";
  input.setAttribute("aria-label", "Add a word");
  input.autocomplete = "off";
  const add = button("Add", "primary");
  const note = el(
    "p",
    "field-note",
    "Fixing the last thing you said also adds the right spelling here. You do not have to set anything up.",
  );
  note.dataset.myWordsNote = "true";
  const kept = el("p", "field-note my-words-kept", "Kept on this PC only");
  const submit = () => {
    const word = input.value.trim();
    if (!word) return;
    myWordsAction(draft, async () => {
      const next = await api.addWord(word);
      input.value = "";
      return next;
    });
  };
  add.addEventListener("click", submit);
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      submit();
    }
  });
  form.append(input, add);
  body.append(list, form, note, kept);
}

function myWordsAction(draft: Draft, run: () => Promise<MemoryView>): void {
  void run()
    .then((view) => refreshMemory(draft, view))
    .catch((error) => {
      const note = document.querySelector<HTMLElement>("[data-my-words-note]");
      if (!note) return;
      note.textContent = messageOf(error);
      note.className = "field-note bad";
    });
}

function memoryAction(draft: Draft, run: () => Promise<MemoryView>): void {
  void run()
    .then((view) => refreshMemory(draft, view))
    .catch((error) => {
      const body = document.querySelector<HTMLElement>("[data-learned]");
      body?.prepend(el("p", "field-note bad", messageOf(error)));
    });
}

function fillLearned(draft: Draft): void {
  const body = document.querySelector<HTMLElement>("[data-learned]");
  const view = memoryView;
  if (!body || !view) return;
  body.replaceChildren();

  body.append(
    switchRow(
      "Keep learning",
      "Remember names and spellings you correct. This does not record or train a voice.",
      view.keepLearning,
      (on) => memoryAction(draft, () => api.setKeepLearning(on)),
    ),
  );

  // Names and words
  const words = el("div", "learned-section");
  words.append(el("h3", "", "Names and words"), el("p", "field-note", "Bolo spells these the way you see them."));
  const chips = el("div", "chips");
  if (view.words.length === 0) chips.append(el("p", "field-note", "Nothing yet."));
  for (const word of view.words) {
    const chip = el("span", "chip");
    chip.append(el("span", "chip-text", word));
    const x = button("✕", "text");
    x.className = "chip-x";
    x.title = `Forget ${word}`;
    x.setAttribute("aria-label", `Forget ${word}`);
    x.addEventListener("click", () => memoryAction(draft, () => api.forgetWord(word)));
    chip.append(x);
    chips.append(chip);
  }
  const addRow = el("div", "add-word");
  const input = document.createElement("input");
  input.type = "text";
  input.placeholder = "Add a name or word";
  input.setAttribute("aria-label", "Add a name or word");
  const add = button("Add", "ghost");
  const doAdd = () => {
    const w = input.value.trim();
    if (w) memoryAction(draft, () => api.addWord(w));
  };
  add.addEventListener("click", doAdd);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      doAdd();
    }
  });
  addRow.append(input, add);
  words.append(chips, addRow);

  // Your fixes
  const fixes = el("div", "learned-section");
  fixes.append(
    el("h3", "", "Your fixes"),
    el("p", "field-note", "When Bolo hears the left side, it writes the right side. Ordinary words such as invoice are left alone."),
  );
  const teach = el("div", "word-add");
  const heard = document.createElement("input");
  heard.type = "text";
  heard.placeholder = "Bolo heard";
  heard.setAttribute("aria-label", "What Bolo heard");
  heard.autocomplete = "off";
  const meant = document.createElement("input");
  meant.type = "text";
  meant.placeholder = "You meant";
  meant.setAttribute("aria-label", "What you meant");
  meant.autocomplete = "off";
  const teachButton = button("Teach this spelling", "ghost");
  const teachNote = el("p", "field-note", "Example: easy ssh → EasySSH. This is a spelling, not voice training.");
  teachNote.dataset.teachNote = "true";
  teachButton.addEventListener("click", () => {
    const from = heard.value.trim();
    const to = meant.value.trim();
    if (!from || !to) return;
    void api
      .teachSpelling(from, to)
      .then((view) => {
        heard.value = "";
        meant.value = "";
        return refreshMemory(draft, view);
      })
      .catch((error) => {
        const slot = document.querySelector<HTMLElement>("[data-teach-note]");
        if (!slot) return;
        slot.textContent = messageOf(error);
        slot.className = "field-note bad";
      });
  });
  teach.append(heard, meant, teachButton);
  fixes.append(teach, teachNote);
  const shownFixes = view.fixes.filter((fix) => fix.from.trim() !== fix.to.trim());
  if (shownFixes.length === 0) fixes.append(el("p", "field-note", "No fixes yet. Use Fix on a History entry to teach Bolo."));
  for (const fix of shownFixes) {
    const row = el("div", "learned-row");
    const text = el("p", "fix-pair");
    text.append(el("span", "fix-from", fix.from), el("span", "fix-arrow", " → "), el("strong", "", fix.to));
    const forget = button("Forget", "text");
    forget.addEventListener("click", () => memoryAction(draft, () => api.forgetFix(fix.from)));
    row.append(text, forget);
    fixes.append(row);
  }

  // Your style
  const style = el("div", "learned-section");
  style.append(el("h3", "", "Your style"), el("p", "field-note", "Short examples of punctuation you like. They are never pasted as-is, and they are not a voice sample."));
  if (view.style.length === 0) style.append(el("p", "field-note", "Nothing yet."));
  view.style.forEach((sample, index) => {
    const row = el("div", "learned-row");
    row.append(el("p", "style-sample", `“${sample}”`));
    const forget = button("Forget", "text");
    forget.addEventListener("click", () => memoryAction(draft, () => api.forgetStyle(index)));
    row.append(forget);
    style.append(row);
  });

  const wipe = button("Forget everything", "ghost");
  wipe.classList.add("danger");
  wipe.addEventListener("click", () => {
    if (wipe.dataset.confirm !== "1") {
      wipe.dataset.confirm = "1";
      wipe.textContent = "Click again to forget everything";
      window.setTimeout(() => {
        wipe.dataset.confirm = "";
        wipe.textContent = "Forget everything";
      }, 4000);
      return;
    }
    memoryAction(draft, () => api.forgetEverything());
  });
  const foot = el("div", "learned-foot");
  foot.append(wipe, el("p", "field-note", "Your History and settings are not touched."));
  body.append(words, fixes, style, foot);
}

// ---- Fix last --------------------------------------------------------------

let fixOpen = false;

async function openFixDialog(draft: Draft, id: string | null): Promise<void> {
  if (fixOpen) document.querySelector(".fix-dialog")?.remove();
  let target;
  try {
    target = await api.fixTarget(id);
  } catch (error) {
    const status = document.querySelector<HTMLElement>("[data-save-status]");
    if (status) {
      status.textContent = messageOf(error);
      status.className = "save-status bad is-shown";
    }
    return;
  }
  const fixId = target.id || null;
  fixOpen = true;
  const dialog = document.createElement("dialog");
  dialog.className = "fix-dialog";
  const close = () => {
    fixOpen = false;
    dialog.close();
    dialog.remove();
  };
  dialog.addEventListener("cancel", () => {
    fixOpen = false;
    dialog.remove();
  });
  const head = el("div", "fix-head");
  head.append(el("h2", "", target.isLast ? "Fix last dictation" : "Fix this dictation"));
  const x = button("✕", "text");
  x.setAttribute("aria-label", "Close");
  x.addEventListener("click", close);
  head.append(x);
  const said = el("blockquote", "fix-said", target.text);
  const note = el("p", "field-note", "");
  note.setAttribute("aria-live", "polite");

  const relisten = button("↻ Listen again carefully", "primary");
  const relistenNote = el(
    "p",
    "field-note",
    target.canRelisten
      ? "Bolo listens to the same recording again with its most careful setting."
      : "The recording for this one is gone. Type the right words instead.",
  );
  relisten.disabled = !target.canRelisten;

  const area = document.createElement("textarea");
  area.rows = 3;
  area.value = target.text;
  area.setAttribute("aria-label", "Type the right words");
  const typeLabel = el("label", "fix-label", "Or type the right words");
  const save = button("Save fix", "ghost");

  const busy = (on: boolean) => {
    relisten.disabled = on || !target.canRelisten;
    save.disabled = on;
  };
  const done = (result: { text: string; message: string; learned: string[] }) => {
    said.textContent = result.text;
    area.value = result.text;
    note.textContent = result.learned.length
      ? `${result.message} Learned: ${result.learned.join(", ")}.`
      : result.message;
    note.className = "field-note good";
    busy(false);
    void refreshHistory(draft);
    void refreshMemory(draft);
  };
  const fail = (error: unknown) => {
    note.textContent = messageOf(error);
    note.className = "field-note bad";
    busy(false);
  };
  relisten.addEventListener("click", () => {
    busy(true);
    note.textContent = "Listening again…";
    note.className = "field-note";
    void api.fixRelisten(fixId).then(done).catch(fail);
  });
  save.addEventListener("click", () => {
    const text = area.value.trim();
    if (!text) return;
    busy(true);
    void api.fixWithText(fixId, text).then(done).catch(fail);
  });

  dialog.append(head, el("p", "fix-kicker", "Bolo wrote:"), said, relisten, relistenNote, typeLabel, area, save, note);
  document.body.append(dialog);
  dialog.showModal();
}
