import { api, onDictation, onListenerStatus, onMicStatus } from "./api";
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
import { listenerNotice, visibleMessage } from "./notices";
import type {
  ApiKeyStatus,
  AppInfo,
  DictationEvent,
  HistoryEntry,
  Hotkey,
  MicInfo,
  Settings,
  WhisperModel,
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
  | "history"
  | "advanced"
  | "key"
  | "engine"
  | "cleanup"
  | "dictionary"
  | "privacy"
  | "paste"
  | "diagnostics";

/** Main pages, with an icon and a title. Advanced sits apart at the bottom. */
const NAV: Array<[SectionId, string, string]> = [
  ["home", "⌂", "Home"],
  ["shortcut", "⌨", "Shortcut"],
  ["character", "☺", "Character & sounds"],
  ["languages", "◍", "Languages"],
  ["history", "↺", "History"],
];

/** Rows on the Advanced page. Each opens its own page. */
const ADVANCED: Array<[SectionId, string, string]> = [
  ["key", "Your own service key", "Use your own key instead of the built-in one"],
  ["engine", "Speech quality and offline mode", "Faster or more accurate, and what happens without internet"],
  ["cleanup", "Text clean-up style", "How much Bolo tidies your words"],
  ["dictionary", "My words list", "Names and words Bolo should spell right"],
  ["privacy", "Privacy", "What stays on this PC"],
  ["paste", "How text is pasted", "Paste the last take again"],
  ["diagnostics", "Help and diagnostics", "Version, status, and a report you can copy"],
];

const FEATURED = ["classic", "c02", "c04", "c07", "c06"];

function normalizeSettings(settings: Settings): Settings {
  return {
    ...settings,
    historyEnabled: settings.historyEnabled !== false,
    offlineModel: settings.offlineModel || "base.en",
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
  root.replaceChildren(loadingShell());
  try {
    const [settings, keyStatus, mics, history, info] = await Promise.all([
      api.getSettings(),
      api.apiKeyStatus(),
      api.listMicrophones().catch(() => [] as MicInfo[]),
      api.getHistory(),
      api.getAppInfo(),
    ]);
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
    advancedPanel(draft),
    keyPanel(draft),
    enginePanel(draft),
    cleanupPanel(draft),
    dictionaryPanel(draft),
    privacyPanel(draft),
    pastePanel(draft),
    diagnosticsPanel(draft),
  );
  main.append(status, form);
  shell.append(nav, main);
  root.append(shell);
  showSection(form, draft.section);

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
    if (event.phase === "done" || event.phase === "idle") void refreshHistory(draft);
    refreshPills(draft);
  });
  void onMicStatus((status) => applyDevices(draft, status.devices));
  void watchListener(draft, form);
  window.addEventListener("bolo-history", () => void refreshHistory(draft));
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
  const titles: Record<string, string> = {
    arming: "Listening",
    recording: "Listening",
    transcribing: "Transcribing",
    inserting: "Inserting",
    done: "Inserted",
    typed: "Typed",
    error: "Dictation failed",
  };
  banner.textContent = draft.live.message
    ? `${titles[draft.live.phase] ?? "Bolo"} — ${draft.live.message}`
    : titles[draft.live.phase] ?? "Bolo";
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
    "Bolo works without this. If you have your own Groq key, paste it here. It stays in Windows Credential Manager and is never written into the settings file.",
  );
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
        draft.keyStatus = await api.setApiKey(input.value);
        input.value = "";
        syncKeyControls(draft);
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
        await api.clearApiKey();
        draft.keyStatus = await api.apiKeyStatus();
        input.value = "";
        syncKeyControls(draft);
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
  panel.append(
    note,
    ownLabel,
    row,
    link,
  );
  return panel;
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
  panel.append(meter, el("p", "field-note", "Talk and watch the bar move. If it stays still, pick another mic."), select, refresh, note);
  return panel;
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
  const offlineChoices: Array<[string, string]> = [
    ["base.en", "base.en — smaller English model"],
    ["small.en", "small.en — larger English model"],
    ["off", "Off — Groq only"],
  ];
  for (const [value, name] of offlineChoices) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = name;
    offline.append(option);
  }
  const selected = draft.settings.offlineModel === "small.en" || draft.settings.offlineModel === "off"
    ? draft.settings.offlineModel
    : "base.en";
  offline.value = selected;
  offline.addEventListener("change", () => {
    draft.settings.offlineModel = offline.value;
    markDirty(draft);
    refreshPills(draft);
  });
  offlineLabel.append(
    offline,
    el(
      "p",
      "field-note",
      "If Groq is rate limiting, down, or unreachable, Bolo transcribes this take on the PC with whisper.cpp and tries Groq again on the next one. The model downloads the first time it's needed.",
    ),
  );
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
  play.addEventListener("click", () => {
    playCue("start");
    window.setTimeout(() => playCue("done"), 900);
  });
  const pickRow = el("div", "row-controls");
  pickRow.append(select, play);
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
    "Audio and the transcript go to Groq, using the key stored on this PC, unless Groq can't take the request. Then the same audio stays on this PC and whisper.cpp transcribes it. Bolo does not store audio or text on a server of its own. A take's audio stays in the local spool until the paste is confirmed. There is no telemetry.",
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
    actions.append(copy, remove);
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
  const startup = el("div", "card rows");
  startup.append(
    switchRow("Start Bolo when my PC turns on", "", draft.settings.launchAtStartup, (on) => {
      draft.settings.launchAtStartup = on;
      markDirty(draft);
    }),
  );
  panel.append(head, hero, startup);
  return panel;
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
  const shell = el("div", "shell loading-shell");
  shell.append(el("p", "eyebrow", "Dictation"), logoHeading());
  shell.append(el("p", "lede", "Loading your settings…"));
  shell.setAttribute("role", "status");
  return shell;
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
