# Cadence

Cadence is a Windows push-to-talk dictation app. Hold a global hotkey in any application, speak, and release. Cadence trims the silence, transcribes the audio with Groq Whisper, and pastes the text at the caret.

It runs from the system tray. A small tally appears while you are recording. Settings live in their own window.

Audio is uploaded to Groq as 16 kHz mono WAV. A short take is about 32 KB per second, so compressing it to FLAC would cost more time than it saves. No other AI provider is used.

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
| Hotkey | Hold to record, release to transcribe. A release shorter than about a quarter second waits briefly for a second tap, which leaves the mic open until you tap again. A longer hold stops the moment you release. Modifier-only chords need two modifiers so a lone Ctrl cannot start a recording. |
| Microphone | **System default (auto)** is the default. It follows the Windows default when you plug or unplug a headset. A named mic is remembered: if it is missing or will not open, Cadence uses another live mic without failing the take, and switches back when the preferred one returns. The settings window and the recording tally show which mic is in use. |
| Model | `whisper-large-v3-turbo` (default) or `whisper-large-v3`. |
| Cleanup | On when `settings.json` has no `cleanupEnabled` field (a new install). A file that already saved `false` stays off until you turn it on. On, Groq-hosted `llama-3.1-8b-instant` punctuates and applies spoken corrections. The request gives up after 1.2 seconds and Cadence pastes the local edit instead. Slack, Discord, and Teams stay casual. Word, Outlook, PowerPoint, OneNote, and a browser whose title says Gmail or Outlook stay formal. Code editors and terminals skip the model. The local edit also handles "scratch that", "no wait", "I mean", and spoken punctuation such as "new line" and "period". |
| Language | `auto`, or an ISO 639-1 code passed to Whisper. |
| Dictionary | One term per line. Whisper is prompted to spell them exactly. After a take, capitalized names that are not the first word (and not weekdays or months) are added, up to three per take. |
| Launch at startup | Current-user startup registration. |
| History | The last 50 inserted transcripts. Audio is not kept. |

Recordings longer than two minutes stop and transcribe on their own. Silence-only takes are not uploaded.

### Where files live

Settings and history are in the app config directory (`settings.json`, `history.json`). A diagnostic log is `cadence.log` next to them. The log records durations and errors, not the API key and not the transcript.

After each take the log has one stage line. Times are milliseconds. `held` is how long the microphone was open. `open_wait` is how long the hotkey waited for that stream (it should be a few milliseconds once the mic is warm). `stop` is draining the buffer. `encode` is trim plus 16 kHz WAV. `groq` is the Whisper response. `cleanup` is the Llama call, or the local edit when the model is skipped. `paste` is clipboard write, Ctrl+V, and restore. `release` is from the key-up that ends the take through the paste. `tone` is `casual`, `formal`, `plain`, `verbatim`, or `none` when nothing was said. An older log line `inserted chars=N ms=N` used `ms` for hold time, not for how long the paste took.

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

That runs `scripts/selftest.ps1`, which prints a pass/fail line for microphone selection and a real capture (skipped when the machine has no input device), the Credential Manager save/read round trip, a paste into a hidden edit control, the Groq transcription and Llama cleanup calls, the rest of the unit tests, and a startup smoke test. Each group has a time limit. If one hangs, the script stops that process and still runs the groups after it.

The smoke test builds the release app (`npm run build`, then `cargo build --release`) and launches `cadence.exe` with `CADENCE_SMOKE=1`. The settings window, created only after app state is registered, invokes `get_settings` the same way the UI does. The test fails if that command errors, if it does not return, or if a UTF-8 BOM settings file (PowerShell `ConvertTo-Json` shape, microphone null, cleanup on) is not what comes back. Quit Cadence first if it is already running; a second instance exits without running the check.

The paste check does not steal foreground focus and does not send Ctrl+V. It writes the clipboard, then sends `WM_PASTE` to a real Edit control — the message that control handles for Ctrl+V — and fails within a few seconds if any step stalls.

Set `GROQ_API_KEY` in the environment before running if you want the live Whisper, Llama, and latency checks. They transcribe `src-tauri/tests/fixtures/dictation.wav` (the spoken line is "Hello um cadence this is a dictation test.") and send a filler-and-correction sentence to Llama. The latency case prints `LATENCY encode_ms=… groq_ms=… cleanup_ms=…` and fails if Whisper takes 8 seconds or cleanup takes 4. Without the variable those cases print `SKIP` and do not fail. The encode timing case always runs and prints `LATENCY encode_fixture_ms=… encode_3s_ms=…`.

```powershell
$env:GROQ_API_KEY = "gsk_..."
npm run test:windows
```

## Continuous integration

[`.github/workflows/windows-installer.yml`](.github/workflows/windows-installer.yml) runs the Rust tests on Ubuntu and the Windows self-test on `windows-latest` for every push. Add a repository secret named `GROQ_API_KEY` to include the live Groq cases. The MSI and NSIS packages are built on `workflow_dispatch` or a `v*` tag.

## How a take works

1. A low-level keyboard hook watches for the chord. The trigger key is swallowed so Ctrl+Space does not type a space. A Win chord sends an unassigned virtual key before Win is released so the Start menu stays closed. Double-tap (two presses that each last under about a quarter second, with the second inside a third of a second) latches hands-free. The next press stops.
2. The microphone stream is opened at startup and kept open, with about 350 ms of pre-roll. Key-down copies that buffer instead of opening WASAPI again, so the first word is not clipped by device startup. While idle, Cadence re-resolves the preferred mic about once a second and switches back when a missing headset returns. It does not switch devices in the middle of a take.
3. Leading and trailing silence is trimmed against an adaptive floor (a quiet room keeps quieter speech; a noisy room asks for more). Speech is resampled to 16 kHz mono and encoded as 16-bit PCM WAV. A press that never contains speech shows a short hint instead of a failure, including "hold the hotkey, or double-tap for hands-free" when the press itself was short.
4. The WAV is posted to `https://api.groq.com/openai/v1/audio/transcriptions` on a persistent HTTP/2 client. Cadence opens that connection at startup. The dictionary is the Whisper prompt.
5. Cleanup calls `https://api.groq.com/openai/v1/chat/completions` with `llama-3.1-8b-instant` unless the focused app is a code editor or terminal, or the setting is off. The call is capped at 1.2 seconds. Timeout, error, or a rewrite that grew too far falls back to the local edit, which still removes um/uh, applies "scratch that" / "no wait" / "I mean", and spoken punctuation.
6. The previous text clipboard is saved, the transcript is placed on the clipboard, Ctrl+V is sent, and the previous text is restored. The two waits are 8 ms for the clipboard to publish and 25 ms before restore. If the clipboard holds an image, or the paste cannot be sent, Cadence types the text instead.

`tauri dev` is a debug build of dependencies, but this crate is compiled at optimization level 3 so capture and encoding are not left in debug mode while you develop. The release profile uses link-time optimization.

## Development notes

The interface can be previewed without the desktop shell:

```bash
npm install
npm run dev
```

Open `http://127.0.0.1:1420` for settings and `http://127.0.0.1:1420/?view=overlay` for the recording tally. Outside the Tauri shell those pages use local preview data and do not call Groq.

`cargo test --manifest-path src-tauri/Cargo.toml` covers silence trimming, adaptive speech threshold, WAV encoding, filler removal, spoken corrections and punctuation commands, foreground-app tone, hotkey chords, double-tap decisions, settings, history, and microphone selection (system default, a missing preferred mic, and switching back when it returns). A short real capture runs only when the machine has an input device. Live Groq calls, including the latency budget, run only when `GROQ_API_KEY` is set. Otherwise those cases print `SKIP` and pass. The encode timing case always prints a `LATENCY` line.

Linux can compile and unit-test the project. The global hook, Credential Manager, and Ctrl+V path are Windows-specific; on other systems a polling listener and a best-effort paste exist so the UI can be exercised, and they are not the supported dictation path.

## What was verified here, and what was not

Verified while building:

- `cargo test` on Linux: 46 tests. Microphone selection, the speech fixture trim, cleanup, tone, double-tap, and the Groq cases are included. This machine has no input device and no `GROQ_API_KEY`, so the real capture and the live Whisper, Llama, and latency cases printed `SKIP`. The local encode timing printed `encode_fixture_ms=0` (89,644 bytes) and `encode_3s_ms=1` (96,044 bytes) with this crate optimized.
- `cargo check --tests --target x86_64-pc-windows-gnu`, which typechecks the Windows hook, the Credential Manager read/write path, device-change notifications, foreground-app detection, and the edit-control paste test. The MSVC target did not finish here because `tauri-winres` needs `llvm-rc`, which this environment does not have. A Windows machine with the C++ build tools compiles that resource step.
- `tsc --noEmit` for the settings window and overlay
- A browser pass of the settings preview for the cleanup default, the Llama 3.1 copy, and the double-tap sentence

Not verified on a real Windows machine:

- The low-level hook actually swallowing keys and suppressing the Start menu, and the double-tap hands-free latch
- WASAPI capture from a physical microphone with the stream kept warm, including an unplug while Cadence is open and the first words of a take
- The stage timings in `cadence.log` on a real take (the encode numbers above are a local fixture, not a Groq round trip)
- Foreground-app tone (Slack versus Word versus a terminal)
- Credential Manager read/write of a live Groq key (the test exists and runs in Windows CI; it was not executed here)
- WebView2 overlay transparency, always-on-top, and click-through
- Ctrl+V restoration timing in Notepad, browsers, and Electron apps
- The MSI/NSIS installer, SmartScreen, and the startup registry entry
- A live Groq transcription, Llama cleanup call, or the latency budget (no `GROQ_API_KEY` in this environment)
- The release-app startup smoke test (`get_settings` from the real settings window). It is part of `scripts/selftest.ps1` and the Windows CI job; this machine is Linux, so the exe was not launched here

## Next steps on Windows

1. Install Rust, Node 22, and the C++ build tools, then `npm run tauri dev`.
2. Put a Groq key in Settings and dictate into Notepad with Ctrl+Win and with Ctrl+Space.
3. Confirm the previous clipboard text comes back, then try an app that ignores Ctrl+V so the typing fallback runs.
4. If Cleanup is off in an existing `settings.json`, turn it on. Dictate "meet Priya scratch that Wednesday period" and confirm the dictionary picked up a capitalized name. Try a double-tap in Slack and a hold in a terminal.
5. `npm run tauri build`, install the MSI or NSIS package, and check tray autostart after a sign-out.
6. If SmartScreen is a problem for distribution, sign the installer with an Authenticode certificate.
