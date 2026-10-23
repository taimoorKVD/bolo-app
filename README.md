# Bolo.com

Bolo.com is a Windows push-to-talk dictation app. Hold a global hotkey in any application, speak, and release. Bolo.com trims the silence, transcribes the audio with Groq Whisper, and pastes the text at the caret. Long takes are cut on pauses into roughly 20–30 second pieces and transcribed while you are still talking, so letting go stays quick. There is no recording-length cap. An earlier build stopped a take at 120 seconds, and the live buffer stopped accepting samples after about 180 seconds of stereo 48 kHz audio.

It runs from the system tray. A small tally appears while you are recording. Settings live in their own window: General, Hotkey, Microphone, Language, Cleanup, Dictionary, History, Privacy, and About.

Audio is uploaded to Groq as 16 kHz mono FLAC, or WAV if FLAC encoding fails. No other AI provider is used. Nothing is stored on a Bolo.com server, and there is no telemetry.

If you already used Cadence, the Groq key in Credential Manager and the files in the `com.cadence.dictation` config folder are copied forward. The old key entry and the old folder are left in place until you remove the key or delete the folder yourself.

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

Bolo.com starts in the tray. The settings window opens on first launch, when no API key is stored, or when you left-click the tray icon. Right-click the tray icon for Settings and Quit. Closing the settings window hides it; the app keeps running.

Hold **Ctrl + Win** (the default), speak, and release. Ctrl + Space and a few other chords are presets in Settings.

### Settings

| Setting | Behavior |
| --- | --- |
| Groq API key | Saved in Windows Credential Manager as a local generic credential, target `com.bolo.dictation/groq-api-key`. A key still stored as `com.cadence.dictation/groq-api-key` or the older keyring target `groq-api-key.com.cadence.dictation` is read and copied forward. A save counts only after the same value is read back. It is not written to `settings.json` and is not logged. Remove deletes all three entries. |
| Hotkey | Hold to record, release to transcribe. The first press starts listening, including while the microphone is still opening. A release shorter than about a quarter second waits briefly for a second tap, which leaves the mic open until you tap again. A longer hold stops the moment you release. Modifier-only chords need two modifiers so a lone Ctrl cannot start a recording. Paste the last take again with Ctrl+Win+V (changeable in Settings). |
| Microphone | **System default (auto)** is the default. It follows the Windows default when you plug or unplug a headset. A named mic is remembered: if it is missing or will not open, Bolo.com uses another live mic without failing the take, and switches back when the preferred one returns. The sidebar shows which mic is in use, and the Microphone section shows a live level. |
| Language and model | `whisper-large-v3-turbo` (default) or `whisper-large-v3`. Language is `auto`, or an ISO 639-1 code passed to Whisper. Temperature is 0. An empty or low-confidence result is sent once more without the prompt. Network errors and HTTP 5xx are retried with backoff. A Groq 429 is retried once, then described as a rate limit. If Groq is rate limiting, returning 5xx, or unreachable, Bolo.com uses a local whisper.cpp model (`base.en` by default, or `small.en`) and tries Groq again on the next take. The model and the Windows whisper.cpp build download on first use. The tally shows which engine produced the text. |
| Cleanup | On when `settings.json` has no `cleanupEnabled` field (a new install). A file that already saved `false` stays off until you turn it on. On, Groq-hosted `llama-3.1-8b-instant` punctuates and applies spoken corrections. The request gives up after 1.2 seconds. A failure is tried once more with a shorter instruction. If that also fails, Bolo.com pastes the local edit and logs both reasons in `cleanup_why`. Slack, Discord, and Teams stay casual. Word, Outlook, PowerPoint, OneNote, and a browser whose title says Gmail or Outlook stay formal. Code editors and terminals skip the model. The local edit also handles "scratch that", "no wait", "I mean", and spoken punctuation such as "new line" and "period". |
| Dictionary | One term per line. Whisper is prompted to spell them exactly, along with the two most recent local transcripts. After a take, capitalized names that are not the first word (and not weekdays or months) are added, up to three per take. |
| History | The last 50 inserted transcripts, on this PC only. Turn **Keep history on this PC** off to stop saving new ones. Clear removes the ones already saved. |
| Launch at startup | Current-user startup registration. |
| Privacy | Audio and text go only to Groq. History is a local file. There is no telemetry. |

A long take keeps going and is cut into pieces while you are still speaking. Silence-only takes are not uploaded. A single click is not treated as speech. Quiet speech is boosted before it is encoded; loud speech is left alone.

### Where files live

The app identifier is `com.bolo.dictation`. Settings and history are in that config directory (`settings.json`, `history.json`). On first launch, if that folder has no `settings.json`, both files are copied from the sibling `com.cadence.dictation` folder. The old folder is not deleted.

A diagnostic log is `bolo.log` next to the new settings. The log records durations and errors, not the API key and not the transcript.

After each take the log has one stage line. Times are milliseconds. `held` is how long the microphone was open. `open_wait` is how long the hotkey waited for that stream (it should be a few milliseconds once the mic is warm). `stop` is draining the buffer. `encode` is trim, gain, resample, and FLAC (or WAV). `fmt` is `flac` or `wav`. `groq` is the Whisper response. `tries` is 1, or 2 when a low-confidence result was retried. `cleanup` is the Llama call, or the local edit when the model is skipped. `cleanup_why` is `applied` when that call's text was pasted, `applied after retry (...)` when the second call was the one pasted, or the reason it was not (`off`, `verbatim`, `empty`, `rejected …`, or the Groq error, with `retry:` when both calls failed). `paste` is clipboard write, Ctrl+V, and restore. `release` is from the key-up that ends the take through the paste. `tone` is `casual`, `formal`, `plain`, `verbatim`, or `none` when nothing was said. `inserted … cleaned=true` means the Llama text was pasted. `cleaned=false` means the local edit was pasted; the reason is `cleanup_why` on the stage line.

## Build an installer

On Windows:

```powershell
npm install
npm run tauri build
```

The installer product name is Bolo.com. The app icon is the Urdu word بولو (Bolo, speak) in gold Nastaliq on a dark tile, with sound waves beside it, from `src-tauri/icons/icon-source.png` (1024×1024). `npm run tauri icon src-tauri/icons/icon-source.png` regenerates the icon set. `python3 scripts/render-icon.py` runs that and redraws the 16, 24, and 32 pixel sizes with a larger word and the waves omitted, so the letters stay readable. That icon is the exe, the tray, the window, the NSIS setup and uninstaller, and the WiX product icon shown in Add/Remove Programs. The settings sidebar and the installer artwork use the horizontal lockup: the tile, Bolo.com, and بولو.

Tauri writes both packages:

- `src-tauri/target/release/bundle/msi/` — MSI
- `src-tauri/target/release/bundle/nsis/` — NSIS `.exe` setup

The build is unsigned. SmartScreen will warn until the binary is signed with a code-signing certificate. That is expected for a local build.

WebView2 is configured as a silent download bootstrapper, so a Windows 10 machine without the runtime can install it during setup if it is online.

The development binary is still named `cadence.exe`. That keeps the Windows self-test path stable.

## Tests

On Windows, from the repo root:

```powershell
npm run test:windows
```

That runs `scripts/selftest.ps1`, which prints a pass/fail line for microphone selection and a real capture (skipped when the machine has no input device), the Credential Manager save/read round trip, settings and key carryover from Cadence, a paste into a hidden edit control, the Groq transcription and Llama cleanup calls, the rest of the unit tests, a startup smoke test, and an icon check. Each group has a time limit. If one hangs, the script stops that process and still runs the groups after it.

The icon check builds the NSIS setup (`npm run tauri -- build --bundles nsis`) and reads the icon resources out of `cadence.exe` and the setup `.exe`. Each image inside `src-tauri/icons/icon.ico` has to be present, byte for byte, in both files. The WiX package uses that same `.ico` as `ARPPRODUCTICON` when `npm run tauri build` produces the MSI.

The smoke test builds the release app with `npm run tauri -- build --no-bundle`. That is the same frontend embedding the installer uses: the Tauri CLI passes `tauri/custom-protocol` and embeds `frontendDist`. A plain `cargo build --release` does not, so that exe loads `http://localhost:1420` and the window shows a connection error. The test launches `cadence.exe` with `CADENCE_SMOKE=1`. The settings window is created only after app state is registered, parked off-screen, kept off the taskbar, and not focused. It still loads the bundled page and invokes `get_settings` the same way the UI does. The test fails if that command errors, if it does not return, or if a UTF-8 BOM settings file (PowerShell `ConvertTo-Json` shape, microphone null, cleanup on, history left on when the field is omitted) is not what comes back. Afterwards the script kills that process tree, including a WebView2 child that outlives the exe. Quit Bolo.com first if it is already running; a second instance exits without running the check.

The paste check does not steal foreground focus and does not send Ctrl+V. It writes the clipboard, then sends `WM_PASTE` to a real Edit control — the message that control handles for Ctrl+V — and fails within a few seconds if any step stalls. If the clipboard holds an image, the test copies it, runs the paste, and puts the image back. If that image cannot be copied, the test prints `SKIP` and leaves the clipboard alone.

Set `GROQ_API_KEY` in the environment before running if you want the live Whisper, Llama, and latency checks. They transcribe `src-tauri/tests/fixtures/dictation.wav` (the spoken line is "Hello um cadence this is a dictation test.") and send a filler-and-correction sentence to Llama. The latency case prints `LATENCY encode_ms=… groq_ms=… cleanup_ms=…` and fails if Whisper takes 8 seconds or cleanup takes 4. Without the variable those cases print `SKIP` and do not fail. The encode timing case always runs and prints `LATENCY encode_fixture_ms=… encode_3s_ms=…`.

```powershell
$env:GROQ_API_KEY = "gsk_..."
npm run test:windows
```

## Continuous integration

[`.github/workflows/windows-installer.yml`](.github/workflows/windows-installer.yml) runs the Rust tests on Ubuntu and the Windows self-test on `windows-latest` for every push. Add a repository secret named `GROQ_API_KEY` to include the live Groq cases. The MSI and NSIS packages are built on `workflow_dispatch` or a `v*` tag. The uploaded artifact is `bolo-windows`.

## How a take works

1. A low-level keyboard hook watches for the chord. The trigger key is swallowed so Ctrl+Space does not type a space. A Win chord sends an unassigned virtual key before Win is released so the Start menu stays closed. Double-tap (two presses that each last under about a quarter second, with the second inside a third of a second) latches hands-free. The next press stops.
2. The microphone stream is opened at startup and kept open, with about 350 ms of pre-roll. Key-down copies that buffer instead of opening WASAPI again, so the first word is not clipped by device startup. While idle, Bolo.com re-resolves the preferred mic about once a second and switches back when a missing headset returns. It does not switch devices in the middle of a take. The same stream publishes the level shown in Settings.
3. Leading and trailing silence is trimmed against an adaptive floor. Speech has to last two frames, so a click is dropped, and a short hangover keeps a quiet consonant. Quiet audio is boosted. Speech is resampled to 16 kHz mono and encoded as FLAC on one thread with fixed predictors. WAV is the fallback. Encoding runs beside the prompt build so the async runtime is not stuck in the encoder. A press that never contains speech shows a short hint instead of a failure, including "hold the hotkey, or double-tap for hands-free" when the press itself was short.
4. The audio is posted to `https://api.groq.com/openai/v1/audio/transcriptions` on a persistent HTTP/2 client. Bolo.com opens that connection at startup. The request uses temperature 0 and `verbose_json`. The prompt is the dictionary plus recent local transcripts. An empty result, an average logprob below -1, or a high no-speech score on a very short result is tried once more without the prompt.
5. Cleanup calls `https://api.groq.com/openai/v1/chat/completions` with `llama-3.1-8b-instant` unless the focused app is a code editor or terminal, or the setting is off. The call is capped at 1.2 seconds. A preamble, a markdown fence, or a content array is still accepted when the words are usable. Timeout, error, or a rewrite that grew too far is retried once. If that also fails, the local edit is pasted and both reasons are logged. The local edit still removes um/uh, applies "scratch that" / "no wait" / "I mean", and spoken punctuation.
6. The previous text clipboard is saved, the transcript is placed on the clipboard, Ctrl+V is sent, and the previous text is restored. The two waits are 8 ms for the clipboard to publish and 25 ms before restore. If the clipboard holds an image, or the paste cannot be sent, Bolo.com types the text instead.

`tauri dev` is a debug build of dependencies, but this crate is compiled at optimization level 3 so capture and encoding are not left in debug mode while you develop. The release profile uses link-time optimization.

## Development notes

The interface can be previewed without the desktop shell:

```bash
npm install
npm run dev
```

Open `http://127.0.0.1:1420` for settings and `http://127.0.0.1:1420/?view=overlay` for the recording tally. Outside the Tauri shell those pages use local preview data and do not call Groq.

`cargo test --manifest-path src-tauri/Cargo.toml` covers silence trimming, the click rejection, gain, FLAC encoding, filler removal, spoken corrections, cleanup reasons, foreground-app tone, hotkey chords, double-tap decisions, settings carryover from Cadence, history, and microphone selection. A short real capture runs only when the machine has an input device. Live Groq calls, including the latency budget, run only when `GROQ_API_KEY` is set. Otherwise those cases print `SKIP` and pass. The encode timing case always prints a `LATENCY` line.

Linux can compile and unit-test the project. The global hook, Credential Manager, and Ctrl+V path are Windows-specific; on other systems a polling listener and a best-effort paste exist so the UI can be exercised, and they are not the supported dictation path.

## What was verified here, and what was not

Verified while building this revision:

- `cargo test` on Linux: 61 tests passed. This machine has no `GROQ_API_KEY`, so the live Whisper, Llama, and latency cases printed `SKIP`. The encode timing case printed `encode_fixture_ms=29` (57,819 bytes, FLAC) and `encode_3s_ms=30` (29,178 bytes, FLAC).
- `cargo check --tests --target x86_64-pc-windows-gnu` passed, including the Credential Manager and paste tests.
- `tsc --noEmit` and the notices check.
- A browser pass of the settings preview: the nine sections, the live meter, the privacy copy, a real error banner, a blank error that does not paint a bar, the hotkey notice clearing once the hook reports in, and a 390px-wide layout with no horizontal page scroll.

Not verified on a real Windows machine:

- The low-level hook actually swallowing keys and suppressing the Start menu, and the double-tap hands-free latch
- WASAPI capture from a physical microphone with the stream kept warm, including an unplug while Bolo.com is open and the first words of a take
- The stage timings in `bolo.log` on a real take
- Foreground-app tone (Slack versus Word versus a terminal)
- Credential Manager read of a live Groq key saved by Cadence (the test exists and runs in Windows CI; it was not executed here)
- WebView2 overlay transparency, always-on-top, and click-through
- Ctrl+V restoration timing in Notepad, browsers, and Electron apps
- The MSI/NSIS installer, SmartScreen, and the startup registry entry
- A live Groq transcription, Llama cleanup call, or the latency budget when `GROQ_API_KEY` is unset
- The bundled-app startup smoke test (`tauri build --no-bundle`, then `get_settings` from the real settings page, window off-screen). It is part of `scripts/selftest.ps1` and the Windows CI job
