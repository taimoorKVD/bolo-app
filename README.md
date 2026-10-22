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
| Groq API key | Saved in Windows Credential Manager as a local generic credential, target `com.cadence.dictation/groq-api-key`. A save counts only after the same value is read back. It is not written to `settings.json` and is not logged. |
| Hotkey | Hold to record, release to transcribe. Modifier-only chords need two modifiers so a lone Ctrl cannot start a recording. |
| Microphone | **System default (auto)** is the default. It follows the Windows default when you plug or unplug a headset. A named mic is remembered: if it is missing or will not open, Cadence uses another live mic without failing the take, and switches back when the preferred one returns. The settings window and the recording tally show which mic is in use. |
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

## Tests

On Windows, from the repo root:

```powershell
npm run test:windows
```

That runs `scripts/selftest.ps1`, which prints a pass/fail line for microphone selection and a real capture (skipped when the machine has no input device), the Credential Manager save/read round trip, a paste into a hidden edit control, the Groq transcription and Llama cleanup calls, and the rest of the unit tests. Each group has a time limit. If one hangs, the script stops that process and still runs the groups after it.

The paste check does not steal foreground focus and does not send Ctrl+V. It writes the clipboard, then sends `WM_PASTE` to a real Edit control — the message that control handles for Ctrl+V — and fails within a few seconds if any step stalls.

Set `GROQ_API_KEY` in the environment before running if you want the live Whisper and Llama checks. They transcribe `src-tauri/tests/fixtures/dictation.wav` (the spoken line is "Hello um cadence this is a dictation test.") and send a filler-and-correction sentence to Llama. Without the variable those cases print `SKIP` and do not fail.

```powershell
$env:GROQ_API_KEY = "gsk_..."
npm run test:windows
```

## Continuous integration

[`.github/workflows/windows-installer.yml`](.github/workflows/windows-installer.yml) runs the Rust tests on Ubuntu and the Windows self-test on `windows-latest` for every push. Add a repository secret named `GROQ_API_KEY` to include the live Groq cases. The MSI and NSIS packages are built on `workflow_dispatch` or a `v*` tag.

## How a take works

1. A low-level keyboard hook watches for the chord. The trigger key is swallowed so Ctrl+Space does not type a space. A Win chord sends an unassigned virtual key before Win is released so the Start menu stays closed.
2. The preferred microphone, or the current Windows default, is resolved again at the start of the take. A missing or unplugged device falls through to the next live input. The default device is watched for hot-plug changes.
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

`cargo test --manifest-path src-tauri/Cargo.toml` covers silence trimming, WAV encoding, filler removal, hotkey chords, settings, history, and microphone selection (system default, a missing preferred mic, and switching back when it returns). A short real capture runs only when the machine has an input device. Live Groq calls run only when `GROQ_API_KEY` is set. Otherwise those cases print `SKIP` and pass.

Linux can compile and unit-test the project. The global hook, Credential Manager, and Ctrl+V path are Windows-specific; on other systems a polling listener and a best-effort paste exist so the UI can be exercised, and they are not the supported dictation path.

## What was verified here, and what was not

Verified while building:

- `cargo test` on Linux: 36 tests. Microphone selection, the speech fixture trim, and the Groq cases are included. This machine has no input device and no `GROQ_API_KEY`, so the real capture and the live Whisper/Llama cases printed `SKIP`.
- `cargo check --tests --target x86_64-pc-windows-gnu`, which typechecks the Windows hook, the Credential Manager read/write path, device-change notifications, and the edit-control paste test. The MSVC target did not finish here because `tauri-winres` needs `llvm-rc`, which this environment does not have. A Windows machine with the C++ build tools compiles that resource step.
- `tsc --noEmit` for the settings window and overlay
- A headless Chrome pass of the browser preview: System default (auto), the in-use line, a missing headset falling back without dropping the preference, a saved settings round trip, the persistent error banner and Dismiss, the overlay mic caption and error wrap, and a 390px layout

Not verified on a real Windows machine:

- The low-level hook actually swallowing keys and suppressing the Start menu
- WASAPI capture from a physical microphone, including an unplug while Cadence is open
- Credential Manager read/write of a live Groq key (the test exists and runs in Windows CI; it was not executed here)
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
