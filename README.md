# Cadence

Cadence is a Windows push-to-talk dictation app. Hold a global hotkey in any application, speak, and release. Cadence trims the silence, transcribes the audio with Groq Whisper, and pastes the text at the caret.

It runs from the system tray. A small tally appears while you are recording. Settings live in their own window.

Audio is uploaded to Groq as 16 kHz mono WAV. Groq also accepts FLAC; WAV keeps the encoder free of native dependencies and stays small (about 1.9 MB per minute). No other AI provider is used.

## What you need

- Windows 10 or 11, 64-bit
- The [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (already present on Windows 11; the installer can fetch the bootstrapper on Windows 10)
- A [Groq API key](https://console.groq.com/keys)
- For development: [Rust](https://rustup.rs/) stable, Node.js 22, and the [Microsoft C++ build tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)

The NSIS installer is per-user and does not need an administrator account. A global keyboard hook and simulated paste only work in apps running at the same privilege level, so an elevated window (for example a terminal opened as Administrator) will not receive dictation.

## Run it

```powershell
npm install
npm run tauri dev
```

Cadence starts in the tray. The settings window opens on first launch, when no API key is stored, or when you left-click the tray icon. Right-click the tray icon for Settings and Quit. Closing the settings window hides it; the app keeps running.

Hold **Ctrl + Win** (the default), speak, and release. Ctrl + Space and a few other chords are presets in Settings.

### Settings

| Setting | Behavior |
| --- | --- |
| Groq API key | Saved in Windows Credential Manager under `com.cadence.dictation` / `groq-api-key`. It is not written to `settings.json` and is not logged. |
| Hotkey | Hold to record, release to transcribe. Modifier-only chords need two modifiers so a lone Ctrl cannot start a recording. |
| Microphone | System default, or a named input device. If the saved device is unplugged, Cadence falls back to the default and says so. |
| Model | `whisper-large-v3-turbo` (default) or `whisper-large-v3`. |
| Cleanup | Off by default. On, the transcript is rewritten by Groq-hosted `llama-3.3-70b-versatile` (punctuation, fillers, spoken self-corrections such as "scratch that"). Off, only standalone "um" and "uh" are removed. |
| Language | `auto`, or an ISO 639-1 code passed to Whisper. |
| Dictionary | One term per line, sent as the Whisper prompt so names and jargon are spelled your way. |
| Launch at startup | Current-user startup registration. |
| History | The last 50 inserted transcripts. Audio is not kept. |

Recordings longer than two minutes stop and transcribe on their own. Silence-only takes are not uploaded.

### Where files live

Settings and history are in the app config directory (`settings.json`, `history.json`). A diagnostic log is `cadence.log` next to them. The log records durations and errors, not the API key.

## Build an installer

On Windows:

```powershell
npm install
npm run tauri build
```

Tauri writes both packages:

- `src-tauri/target/release/bundle/msi/` — MSI
- `src-tauri/target/release/bundle/nsis/` — NSIS `.exe` setup

The build is unsigned. SmartScreen will warn until the binary is signed with a code-signing certificate. That is expected for a local build.

WebView2 is configured as a silent download bootstrapper, so a Windows 10 machine without the runtime can install it during setup if it is online.

## Continuous integration

[`.github/workflows/windows-installer.yml`](.github/workflows/windows-installer.yml) runs the Rust unit tests on Ubuntu and, on `workflow_dispatch` or a `v*` tag, builds the MSI and NSIS artifacts on `windows-latest`.

## How a take works

1. A low-level keyboard hook watches for the chord. The trigger key is swallowed so Ctrl+Space does not type a space. A Win chord sends an unassigned virtual key before Win is released so the Start menu stays closed.
2. The default microphone (or the one you picked) records while the chord is held.
3. Leading and trailing silence is trimmed. Speech is resampled to 16 kHz mono and encoded as 16-bit PCM WAV.
4. The WAV is posted to `https://api.groq.com/openai/v1/audio/transcriptions`.
5. Optional Llama cleanup uses `https://api.groq.com/openai/v1/chat/completions`. If cleanup fails, Cadence still pastes the filler-stripped transcript.
6. The previous text clipboard is saved, the transcript is placed on the clipboard, Ctrl+V is sent, and the previous text is restored. If the clipboard holds an image, or the paste cannot be sent, Cadence types the text instead.

## Development notes

The interface can be previewed without the desktop shell:

```bash
npm install
npm run dev
```

Open `http://127.0.0.1:1420` for settings and `http://127.0.0.1:1420/?view=overlay` for the recording tally. Outside the Tauri shell those pages use local preview data and do not call Groq.

`cargo test --manifest-path src-tauri/Cargo.toml` covers silence trimming, WAV encoding, filler removal, hotkey chords, settings, and history. It does not open a microphone or call Groq.

Linux can compile and unit-test the project. The global hook, Credential Manager, and Ctrl+V path are Windows-specific; on other systems a polling listener and a best-effort paste exist so the UI can be exercised, and they are not the supported dictation path.

## What was verified here, and what was not

Verified while building:

- `cargo test` on Linux: 24 tests for silence trimming, WAV encoding, filler removal, hotkey chords, settings, and history
- `cargo check --target x86_64-pc-windows-gnu`, which typechecks the Windows hook, Credential Manager backend (`keyring`), and `SendInput` paste path (`cfg(windows)` is the same on the GNU and MSVC targets). The MSVC target did not finish here because `tauri-winres` needs `llvm-rc`, which this environment does not have. A Windows machine with the C++ build tools compiles that resource step.
- `tsc --noEmit` for the settings window and overlay
- A headless Chrome pass of the browser preview: hotkey presets, unsaved state, API key validation, history, the record-a-test cycle, a 390px layout, and the overlay phases

Not verified on a real Windows machine:

- The low-level hook actually swallowing keys and suppressing the Start menu
- WASAPI capture from a physical microphone
- Credential Manager read/write of a live Groq key
- WebView2 overlay transparency, always-on-top, and click-through
- Ctrl+V restoration timing in Notepad, browsers, and Electron apps
- The MSI/NSIS installer, SmartScreen, and the startup registry entry
- A live Groq transcription or Llama cleanup call

## Next steps on Windows

1. Install Rust, Node 22, and the C++ build tools, then `npm run tauri dev`.
2. Put a Groq key in Settings and dictate into Notepad with Ctrl+Win and with Ctrl+Space.
3. Confirm the previous clipboard text comes back, then try an app that ignores Ctrl+V so the typing fallback runs.
4. Toggle Llama cleanup and a custom dictionary term.
5. `npm run tauri build`, install the MSI or NSIS package, and check tray autostart after a sign-out.
6. If SmartScreen is a problem for distribution, sign the installer with an Authenticode certificate.
