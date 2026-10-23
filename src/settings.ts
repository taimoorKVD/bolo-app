import { api, onDictation, onListenerStatus, onMicStatus } from "./api";
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
  | "general"
  | "hotkey"
  | "microphone"
  | "language"
  | "cleanup"
  | "dictionary"
  | "history"
  | "privacy"
  | "about";

const NAV: Array<[SectionId, string, string]> = [
  ["general", "General", "Account"],
  ["hotkey", "Hotkey", "Push to talk"],
  ["microphone", "Microphone", "Input"],
  ["language", "Language", "Accuracy"],
  ["cleanup", "Cleanup", "Style"],
  ["dictionary", "Dictionary", "Names"],
  ["history", "History", "On this PC"],
  ["privacy", "Privacy", "Local only"],
  ["about", "About", "Bolo.com"],
];

function normalizeSettings(settings: Settings): Settings {
  return {
    ...settings,
    historyEnabled: settings.historyEnabled !== false,
    offlineModel: settings.offlineModel || "base.en",
    pasteHotkey: settings.pasteHotkey ?? { ctrl: true, alt: false, shift: false, win: true, key: "v" },
    output: settings.output === "same" ? "same" : "english",
    verbatim: settings.verbatim === true,
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
    render(root, {
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
      section: "general",
    });
    return { ok: true, detail: JSON.stringify(settings) };
  } catch (error) {
    const message = messageOf(error);
    root.replaceChildren(errorShell(message, () => void mountSettings(root)));
    return { ok: false, error: message };
  }
}

function render(root: HTMLElement, draft: Draft): void {
  root.replaceChildren();
  const shell = el("div", "shell");
  const nav = buildNav(draft);
  const main = el("main", "stage");
  const form = document.createElement("form");
  form.className = "sheet";
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void save(draft, form);
  });

  const alert = alertBanner(draft);
  form.append(
    ...(alert ? [alert] : []),
    liveBanner(draft),
    listenerBanner(draft),
    generalPanel(draft),
    hotkeyPanel(draft),
    micPanel(draft),
    languagePanel(draft),
    cleanupPanel(draft),
    dictionaryPanel(draft),
    historyPanel(draft),
    privacyPanel(draft),
    aboutPanel(draft),
    saveBar(draft),
  );
  main.append(form);
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
    const test = document.getElementById("test-record") as HTMLButtonElement | null;
    if (test) {
      test.textContent = event.phase === "recording" ? "Stop test" : "Record a test";
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
  for (const button of document.querySelectorAll<HTMLButtonElement>("[data-nav]")) {
    const on = button.dataset.nav === id;
    button.classList.toggle("is-on", on);
    if (on) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  }
}

function buildNav(draft: Draft): HTMLElement {
  const rail = el("aside", "rail");
  const brand = el("div", "brand");
  brand.append(
    el("p", "eyebrow", "Dictation"),
    logoHeading(),
    el("p", "lede", "Hold a key anywhere in Windows. Speak. Let go. The words land at the cursor."),
  );
  const menu = el("nav", "sidenav");
  menu.setAttribute("aria-label", "Settings");
  for (const [id, title, hint] of NAV) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "nav-item";
    button.dataset.nav = id;
    button.append(el("span", "nav-title", title), el("span", "nav-hint", hint));
    button.addEventListener("click", () => {
      draft.section = id;
      const form = document.querySelector<HTMLElement>(".sheet");
      if (form) showSection(form, id);
    });
    menu.append(button);
  }
  const pills = el("div", "pills");
  pills.append(
    pill("key", "Key", draft.keyStatus.configured ? draft.keyStatus.hint ?? "Saved" : "Not set", draft.keyStatus.configured ? "good" : "warn"),
    pill("hotkey", "Hotkey", formatHotkey(draft.settings.hotkey), "neutral"),
    pill("mic", "Mic", micSummary(draft).value, "neutral"),
    pill("engine", "Engine", draft.lastEngine, "good"),
  );
  rail.append(brand, menu, pills);
  if (draft.info.platform !== "windows") {
    rail.append(
      el(
        "p",
        "platform-note",
        "This session is not Windows. The global hook, Credential Manager, and paste path run on Windows.",
      ),
    );
  }
  rail.append(el("p", "version", `Version ${draft.info.version}`));
  return rail;
}

function pill(name: string, label: string, value: string, tone: string): HTMLElement {
  const node = el("div", `status-pill tone-${tone}`);
  node.dataset.pill = name;
  node.append(el("span", "pill-k", label), el("span", "pill-v", value));
  return node;
}

function refreshPills(draft: Draft): void {
  const key = document.querySelector<HTMLElement>('[data-pill="key"]');
  const hotkey = document.querySelector<HTMLElement>('[data-pill="hotkey"]');
  const mic = document.querySelector<HTMLElement>('[data-pill="mic"]');
  setPill(key, draft.keyStatus.configured ? draft.keyStatus.hint ?? "Saved" : "Not set", draft.keyStatus.configured ? "good" : "warn");
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
    ? `${titles[draft.live.phase] ?? "Bolo.com"} — ${draft.live.message}`
    : titles[draft.live.phase] ?? "Bolo.com";
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

function generalPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "general",
    "Account",
    "The Groq key stays in Windows Credential Manager. It is never written into the settings file.",
  );
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
  const remove = button("Remove", "ghost");
  remove.disabled = !draft.keyStatus.configured;
  const note = el("p", "field-note");
  note.textContent = keyNote(draft.keyStatus);
  if (draft.keyStatus.error) note.classList.add("bad");
  saveKey.addEventListener("click", () => {
    void (async () => {
      saveKey.disabled = true;
      try {
        draft.keyStatus = await api.setApiKey(input.value);
        input.value = "";
        note.textContent = `Saved in the OS credential store ${draft.keyStatus.hint ?? ""}.`;
        note.className = "field-note good";
        remove.disabled = false;
        refreshPills(draft);
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
        draft.keyStatus = { configured: false, hint: null, error: null };
        note.textContent = "API key removed from this PC, including the previous Cadence entry.";
        note.className = "field-note";
        remove.disabled = true;
        refreshPills(draft);
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
    row,
    note,
    link,
    toggle(
      "Launch at startup",
      "Bolo.com starts in the tray for this Windows user. The settings window stays closed until you open it.",
      draft.settings.launchAtStartup,
      (checked) => {
        draft.settings.launchAtStartup = checked;
        markDirty(draft);
      },
    ),
  );
  return panel;
}

function hotkeyPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "hotkey",
    "Push to talk",
    "Hold the chord to record. Release it to transcribe and paste. Double-tap to leave the mic open, then tap once to stop. The default is Ctrl + Win.",
  );
  const display = el("div", "hotkey-display");
  const kbd = el("kbd", "hotkey-kbd", formatHotkey(draft.settings.hotkey));
  const change = button("Change", "ghost");
  const reset = button("Reset", "text");
  display.append(kbd, change, reset);
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
    "Pick a preset, or click Change and press a chord. Escape cancels. Win-key chords are presets because Windows may not deliver the Win key to this window.",
  );
  change.addEventListener("click", () => startCapture(draft, kbd, hint, change));
  reset.addEventListener("click", () => {
    draft.settings.hotkey = { ctrl: true, alt: false, shift: false, win: true, key: null };
    kbd.textContent = formatHotkey(draft.settings.hotkey);
    markDirty(draft);
    refreshPills(draft);
  });
  const test = button(draft.live?.phase === "recording" ? "Stop test" : "Record a test", "ghost");
  test.id = "test-record";
  test.addEventListener("click", () => {
    (document.activeElement as HTMLElement | null)?.blur();
    void api.toggleTestRecording();
  });
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
      "Press this when a take didn't land, or after Bolo.com starts again. The audio stays on this PC until a paste is confirmed.",
    ),
  );
  panel.append(
    display,
    presets,
    hint,
    test,
    el(
      "p",
      "field-note",
      "Record a test uses the selected microphone and pastes into whichever app is focused when you stop. The first press starts listening.",
    ),
    pasteLabel,
  );
  return panel;
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
      hint.textContent = "Saved locally. Click Save settings to keep it.";
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
    "microphone",
    "Microphone",
    "System default follows whichever microphone Windows is using. Pick one to prefer it. If you unplug it, Bolo.com uses the default and switches back when it returns.",
  );
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
  panel.append(meter, el("p", "field-note", "Live level from the microphone that is open now."), select, refresh, note);
  return panel;
}

async function paintMeter(): Promise<void> {
  const panel = document.querySelector<HTMLElement>('[data-panel="microphone"]');
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
  auto.textContent = "System default (auto)";
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
        note: "No microphone is connected. Plug one in and Bolo.com will use it automatically.",
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
      note: `In use: ${match.name}. If you unplug it, Bolo.com uses the system default and switches back when it returns.`,
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
    "language",
    "Language and accuracy",
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
  const langLabel = el("label", "stack");
  langLabel.append(el("span", "label", "Language"));
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
  outputLabel.append(el("span", "label", "Output"));
  const output = document.createElement("select");
  output.setAttribute("aria-label", "Output");
  const outputChoices: Array<["english" | "same", string]> = [
    ["english", "English (translate)"],
    ["same", "Same as spoken"],
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
      "English (translate) is the default. Urdu, Roman Urdu, and any other language come back in English. Same as spoken leaves the transcript in the language you used.",
    ),
  );
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
      "If Groq is rate limiting, down, or unreachable, Bolo.com transcribes this take on the PC with whisper.cpp and tries Groq again on the next one. The model downloads the first time it's needed.",
    ),
  );
  panel.append(grid, langLabel, outputLabel, offlineLabel);
  return panel;
}

function cleanupPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "cleanup",
    "Cleanup and style",
    "Llama 3.1 8B Instant punctuates and applies spoken corrections such as “scratch that”, “new line”, and “period”. It does not add or drop what you said. Chat stays casual. Mail and documents stay formal. Code editors skip the rewrite. Verbatim pastes the transcript unchanged.",
  );
  panel.append(
    toggle(
      "Verbatim",
      "Paste exactly what was said. Cleanup will not add, drop, or rephrase a word, including um and uh.",
      draft.settings.verbatim,
      (checked) => {
        draft.settings.verbatim = checked;
        markDirty(draft);
      },
    ),
    toggle(
      "Clean up with Llama",
      "On for a new install. A settings file that already saved this off stays off until you turn it on.",
      draft.settings.cleanupEnabled,
      (checked) => {
        draft.settings.cleanupEnabled = checked;
        markDirty(draft);
      },
    ),
  );
  return panel;
}

function dictionaryPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "dictionary",
    "Dictionary",
    "One term per line. Whisper is prompted to spell them as written. Bolo.com also remembers capitalized names from what you dictate.",
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

function historyPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "history",
    "History",
    "The last 50 transcripts, newest first, stored only on this PC. History keeps the words. A take's audio stays only until the paste is confirmed.",
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
    "Audio and the transcript go to Groq, using the key stored on this PC, unless Groq can't take the request. Then the same audio stays on this PC and whisper.cpp transcribes it. Bolo.com does not store audio or text on a server of its own. A take's audio stays in the local spool until the paste is confirmed. There is no telemetry.",
  );
  const list = el("div", "history-list");
  list.dataset.historyPrivacy = "true";
  panel.append(
    toggle(
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

function aboutPanel(draft: Draft): HTMLElement {
  const panel = panelEl(
    "about",
    "About Bolo.com",
    "Push-to-talk dictation for Windows. Hold the hotkey, speak, and the transcript is pasted at the cursor.",
  );
  const facts = el("dl", "facts");
  facts.append(
    fact("Version", draft.info.version),
    fact("Transcription", "Groq Whisper, temperature 0"),
    fact("Cleanup", "Groq Llama 3.1 8B Instant"),
    fact("Audio", "16 kHz mono FLAC, WAV if encoding fails"),
    fact("Carryover", "Settings and the Groq key from Cadence (com.cadence.dictation) are copied forward."),
  );
  panel.append(facts);
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

async function save(draft: Draft, form: HTMLFormElement): Promise<void> {
  const status = form.querySelector<HTMLElement>("[data-save-status]");
  const buttonEl = form.querySelector<HTMLButtonElement>("button[type=submit]");
  if (buttonEl) {
    buttonEl.disabled = true;
    buttonEl.textContent = "Saving…";
  }
  try {
    const keyInput = form.querySelector<HTMLInputElement>("[data-api-key]");
    if (keyInput && keyInput.value.trim()) {
      draft.keyStatus = await api.setApiKey(keyInput.value);
      keyInput.value = "";
      const keyNoteEl = form.querySelector<HTMLElement>(".key-row + .field-note");
      if (keyNoteEl) {
        keyNoteEl.textContent = `Saved in the OS credential store ${draft.keyStatus.hint ?? ""}.`;
        keyNoteEl.className = "field-note good";
      }
      refreshPills(draft);
    }
    await api.saveSettings(draft.settings);
    draft.dirty = false;
    const saved = await api.getSettings();
    draft.settings = normalizeSettings(saved);
    if (status) {
      status.textContent = "Settings saved.";
      status.className = "save-status good";
    }
    refreshPills(draft);
  } catch (error) {
    if (status) {
      status.textContent = messageOf(error);
      status.className = "save-status bad";
    }
  } finally {
    if (buttonEl) {
      buttonEl.disabled = false;
      buttonEl.textContent = "Save settings";
    }
  }
}

function saveBar(draft: Draft): HTMLElement {
  const bar = el("div", "savebar");
  const status = el("p", `save-status ${draft.statusTone}`, draft.status);
  status.dataset.saveStatus = "true";
  const save = button(draft.saving ? "Saving…" : "Save settings", "primary");
  save.type = "submit";
  save.disabled = draft.saving;
  bar.append(status, save);
  return bar;
}

function markDirty(draft: Draft): void {
  draft.dirty = true;
  const status = document.querySelector<HTMLElement>("[data-save-status]");
  if (status && !status.classList.contains("bad")) {
    status.textContent = "Unsaved changes";
    status.className = "save-status";
  }
}

function keyNote(status: ApiKeyStatus): string {
  if (status.error) return status.error;
  if (status.configured) return `Key ${status.hint ?? ""} is in the OS credential store. A key saved by Cadence is copied forward.`;
  return "Paste a key from console.groq.com. Nothing is sent until you dictate.";
}

function panelEl(id: SectionId, title: string, body: string): HTMLElement {
  const section = el("section", "card");
  section.dataset.panel = id;
  const head = el("div", "card-head");
  head.append(el("h2", "", title));
  section.append(head, el("p", "card-body", body));
  return section;
}

function toggle(title: string, body: string, checked: boolean, onChange: (value: boolean) => void): HTMLElement {
  const label = document.createElement("label");
  label.className = "toggle";
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = checked;
  input.addEventListener("change", () => onChange(input.checked));
  const text = el("span", "toggle-copy");
  text.append(el("strong", "", title), el("span", "", body));
  label.append(input, text);
  return label;
}

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
  img.alt = "Bolo.com";
  heading.append(img);
  return heading;
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
