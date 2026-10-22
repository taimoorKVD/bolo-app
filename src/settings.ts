import { api, onDictation, onListenerStatus, onMicStatus } from "./api";
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
];

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
  lastError: string | null;
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
      settings,
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
      lastError: info.lastError,
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
  const rail = buildRail(draft);
  const main = el("main", "main");
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
    apiSection(draft),
    hotkeySection(draft),
    micSection(draft),
    modelSection(draft),
    dictionarySection(draft),
    startupSection(draft),
    saveBar(draft),
  );
  main.append(form, historySection(draft));
  shell.append(rail, main);
  root.append(shell);

  void onDictation((event) => {
    draft.live = event.phase === "idle" ? null : event;
    if (event.phase === "error") {
      const message = visibleMessage(event.message);
      if (message) draft.lastError = message;
    }
    if (event.phase === "recording") draft.lastError = null;
    const banner = form.querySelector<HTMLElement>("[data-live]");
    if (banner) banner.replaceWith(liveBanner(draft));
    placeAlert(form, draft);
    const test = document.getElementById("test-record");
    if (test) test.textContent = event.phase === "recording" ? "Stop test" : "Record a test";
    if (event.phase === "recording") refreshRail(draft);
    if (event.phase === "done" || event.phase === "idle") {
      void refreshHistory(draft);
    }
  });
  void onMicStatus((status) => {
    applyDevices(draft, status.devices);
  });
  void watchListener(draft, form);
  window.addEventListener("cadence-history", () => void refreshHistory(draft));
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

function buildRail(draft: Draft): HTMLElement {
  const rail = el("aside", "rail");
  rail.append(
    el("p", "eyebrow", "Dictation"),
    el("h1", "wordmark", "Cadence"),
    el(
      "p",
      "lede",
      "Hold a key anywhere in Windows. Speak. Let go. The words land at the cursor.",
    ),
  );
  const list = el("dl", "status-list");
  list.append(
    statusRow("API key", draft.keyStatus.configured ? draft.keyStatus.hint ?? "Saved" : "Not set"),
    statusRow("Hotkey", formatHotkey(draft.settings.hotkey)),
    statusRow("In use", micSummary(draft).value),
  );
  rail.append(list);
  rail.append(
    el(
      "p",
      "privacy",
      "Audio is sent to Groq with your key, then discarded. Cadence does not keep the recording.",
    ),
  );
  if (draft.info.platform !== "windows") {
    rail.append(
      el(
        "p",
        "platform-note",
        "This session is not Windows. The global hook, Credential Manager, and paste path are implemented for Windows.",
      ),
    );
  }
  rail.append(el("p", "version", `Version ${draft.info.version}`));
  return rail;
}

function statusRow(label: string, value: string): HTMLElement {
  const row = el("div", "status-row");
  row.append(el("dt", "", label), el("dd", "", value));
  return row;
}

function liveBanner(draft: Draft): HTMLElement {
  const banner = el("div", "banner live");
  banner.dataset.live = "true";
  if (!draft.live) {
    banner.hidden = true;
    return banner;
  }
  const titles: Record<string, string> = {
    recording: "Listening…",
    transcribing: "Transcribing…",
    done: "Inserted",
    error: "Dictation failed",
  };
  banner.textContent = draft.live.message
    ? `${titles[draft.live.phase] ?? "Cadence"} — ${draft.live.message}`
    : titles[draft.live.phase] ?? "Cadence";
  if (draft.live.phase === "error") banner.classList.add("bad");
  return banner;
}

function alertBanner(draft: Draft): HTMLElement | null {
  const message = visibleMessage(draft.lastError);
  if (!message) return null;
  const banner = el("div", "banner bad alert");
  banner.dataset.alert = "true";
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

function apiSection(draft: Draft): HTMLElement {
  const section = sectionEl("01", "Groq API key", "Stored in Windows Credential Manager. It never goes into the settings file.");
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
  const save = button("Save key", "primary");
  const remove = button("Remove", "ghost");
  remove.disabled = !draft.keyStatus.configured;
  const note = el("p", "field-note");
  note.textContent = keyNote(draft.keyStatus);
  if (draft.keyStatus.error) note.classList.add("bad");
  save.addEventListener("click", () => {
    void (async () => {
      save.disabled = true;
      try {
        draft.keyStatus = await api.setApiKey(input.value);
        input.value = "";
        note.textContent = `Saved in the OS credential store ${draft.keyStatus.hint ?? ""}.`;
        note.className = "field-note good";
        remove.disabled = false;
        refreshRail(draft);
      } catch (error) {
        note.textContent = messageOf(error);
        note.className = "field-note bad";
      } finally {
        save.disabled = false;
      }
    })();
  });
  remove.addEventListener("click", () => {
    void (async () => {
      try {
        await api.clearApiKey();
        draft.keyStatus = { configured: false, hint: null, error: null };
        note.textContent = "API key removed.";
        note.className = "field-note";
        remove.disabled = true;
        refreshRail(draft);
      } catch (error) {
        note.textContent = messageOf(error);
        note.className = "field-note bad";
      }
    })();
  });
  const link = button("Open Groq console", "text");
  link.addEventListener("click", () => void api.openGroqKeys().catch((error) => {
    note.textContent = messageOf(error);
    note.className = "field-note bad";
  }));
  row.append(input, save, remove);
  section.append(row, note, link);
  return section;
}

function hotkeySection(draft: Draft): HTMLElement {
  const section = sectionEl(
    "02",
    "Push to talk",
    "Hold the chord to record. Release it to transcribe and paste. Double-tap to leave the mic open, then tap once to stop. Default is Ctrl + Win.",
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
      refreshRail(draft);
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
    refreshRail(draft);
  });
  const test = button(draft.live?.phase === "recording" ? "Stop test" : "Record a test", "ghost");
  test.id = "test-record";
  test.addEventListener("click", () => {
    (document.activeElement as HTMLElement | null)?.blur();
    void api.toggleTestRecording();
  });
  const testNote = el(
    "p",
    "field-note",
    "Record a test uses the selected microphone and pastes into whichever app is focused when you stop.",
  );
  section.append(display, presets, hint, el("div", "test-row", ""), testNote);
  section.querySelector(".test-row")?.append(test);
  return section;
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
      refreshRail(draft);
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

function micSection(draft: Draft): HTMLElement {
  const section = sectionEl(
    "03",
    "Microphone",
    "System default (auto) follows whichever microphone Windows is using. Pick one to prefer it — if you unplug it, Cadence uses the default and switches back when it returns.",
  );
  const select = document.createElement("select");
  select.setAttribute("aria-label", "Microphone");
  select.dataset.micSelect = "true";
  fillMicOptions(select, draft);
  select.addEventListener("change", () => {
    draft.settings.microphone = select.value || null;
    markDirty(draft);
    const note = section.querySelector<HTMLElement>("[data-mic-note]");
    if (note) note.textContent = micSummary(draft).note;
    refreshRail(draft);
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
  section.append(select, refresh, note);
  return section;
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
  refreshRail(draft);
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
        note: "No microphone is connected. Plug one in and Cadence will use it automatically.",
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
      note: `In use: ${match.name}. If you unplug it, Cadence uses the system default and switches back when it returns.`,
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

function modelSection(draft: Draft): HTMLElement {
  const section = sectionEl(
    "04",
    "Transcription",
    "Whisper on Groq. Cleanup uses Llama 3.1 8B Instant, also on Groq, and gives up quickly so a slow rewrite never holds the paste.",
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

  const cleanup = toggle(
    "Clean up with Llama",
    "On for a new install. Llama 3.1 8B Instant punctuates and applies spoken corrections such as \"scratch that\", \"new line\", and \"period\". Chat stays casual, mail and documents stay formal, and code editors skip the rewrite. If the model is slow, Cadence pastes the local edit instead.",
    draft.settings.cleanupEnabled,
    (checked) => {
      draft.settings.cleanupEnabled = checked;
      markDirty(draft);
    },
  );
  section.append(grid, langLabel, cleanup);
  return section;
}

function dictionarySection(draft: Draft): HTMLElement {
  const section = sectionEl(
    "05",
    "Custom dictionary",
    "One term per line. Whisper is prompted to spell them as written. Cadence also remembers capitalized names from what you dictate.",
  );
  const area = document.createElement("textarea");
  area.rows = 5;
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
  section.append(area);
  return section;
}

function startupSection(draft: Draft): HTMLElement {
  const section = sectionEl("06", "Startup", "Cadence starts in the tray. The settings window stays closed until you open it.");
  section.append(
    toggle(
      "Launch at startup",
      "Registers Cadence with Windows startup for the current user.",
      draft.settings.launchAtStartup,
      (checked) => {
        draft.settings.launchAtStartup = checked;
        markDirty(draft);
      },
    ),
  );
  return section;
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

function historySection(draft: Draft): HTMLElement {
  const section = sectionEl("07", "Recent", "The last 50 transcripts, newest first. Audio is not stored.");
  section.classList.add("history");
  const list = el("div", "history-list");
  list.dataset.history = "true";
  fillHistory(list, draft);
  const clear = button("Clear history", "text");
  clear.addEventListener("click", () => {
    if (draft.history.length === 0) return;
    if (!window.confirm("Clear every saved transcript on this computer?")) return;
    void (async () => {
      await api.clearHistory();
      draft.history = [];
      fillHistory(list, draft);
    })().catch((error) => {
      const note = el("p", "field-note bad", messageOf(error));
      section.append(note);
    });
  });
  section.append(list, clear);
  return section;
}

function fillHistory(list: HTMLElement, draft: Draft): void {
  list.replaceChildren();
  if (draft.history.length === 0) {
    const empty = el("div", "empty");
    empty.append(
      el("p", "empty-title", "Nothing dictated yet."),
      el(
        "p",
        "",
        `Hold ${formatHotkey(draft.settings.hotkey)} in any app, speak, and release.`,
      ),
    );
    list.append(empty);
    return;
  }
  for (const entry of draft.history) {
    const item = el("article", "history-item");
    const body = el("div");
    body.append(el("p", "history-text", entry.text));
    const meta = el(
      "p",
      "history-meta",
      `${formatWhen(entry.createdUnixMs)} · ${formatDuration(entry.durationMs)} · ${modelLabel(entry.model)}${entry.cleaned ? " · cleaned" : ""}`,
    );
    body.append(meta);
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
      refreshRail(draft);
    }
    await api.saveSettings(draft.settings);
    draft.dirty = false;
    draft.settings = await api.getSettings();
    if (status) {
      status.textContent = "Settings saved.";
      status.className = "save-status good";
    }
    refreshRail(draft);
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

function markDirty(draft: Draft): void {
  draft.dirty = true;
  const status = document.querySelector<HTMLElement>("[data-save-status]");
  if (status && !status.classList.contains("bad")) {
    status.textContent = "Unsaved changes";
    status.className = "save-status";
  }
}

function refreshRail(draft: Draft): void {
  const rail = document.querySelector(".rail");
  rail?.replaceWith(buildRail(draft));
}

function keyNote(status: ApiKeyStatus): string {
  if (status.error) return status.error;
  if (status.configured) return `Key ${status.hint ?? ""} is in the OS credential store.`;
  return "Paste a key from console.groq.com. Nothing is sent until you dictate.";
}

function sectionEl(index: string, title: string, body: string): HTMLElement {
  const section = el("section", "section");
  const head = el("div", "section-head");
  head.append(el("span", "index", index), el("h2", "", title));
  section.append(head, el("p", "section-body", body));
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

function loadingShell(): HTMLElement {
  const shell = el("div", "shell loading-shell");
  shell.append(el("p", "eyebrow", "Dictation"), el("h1", "wordmark", "Cadence"));
  shell.append(el("p", "lede", "Loading your settings…"));
  shell.setAttribute("role", "status");
  return shell;
}

function errorShell(message: string, retry: () => void): HTMLElement {
  const shell = el("div", "shell error-shell");
  shell.append(el("p", "eyebrow", "Dictation"), el("h1", "wordmark", "Cadence"));
  shell.append(el("p", "lede", "Settings didn't load."));
  const banner = el("div", "banner bad", visibleMessage(message) ?? "Settings didn't load.");
  const retryButton = button("Try again", "primary");
  retryButton.addEventListener("click", retry);
  shell.append(banner, retryButton);
  return shell;
}

function messageOf(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string") return error;
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
