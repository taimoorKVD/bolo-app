# Microsoft Store listing draft: Bolo

**App name (reserve in Partner Center):** Bolo  
**Category:** Productivity (or Utilities & tools)  
**Pricing:** Free

## Short description
Hold a hotkey, speak, and Bolo types your words wherever your cursor is.

## Description
Bolo is push-to-talk voice dictation for Windows. Hold your hotkey, talk naturally, and let go. Bolo transcribes what you said and pastes the text into whatever app you're using: email, chat, documents, code editors, or the browser.

Bolo is fast. Transcription runs on Groq Whisper and usually finishes in under a second. Optional cleanup removes filler words and fixes punctuation, so the text reads like you typed it.

## Features
- Push-to-talk global hotkey that works in any app
- Transcript pasted straight at your cursor
- Fast, accurate speech recognition (Groq Whisper)
- Optional AI cleanup: removes filler words, fixes punctuation and tone
- Small overlay that shows when Bolo is listening
- Use the built-in service or bring your own Groq API key
- Optional launch at Windows startup
- Lives quietly in the system tray

## Privacy policy URL
https://bolo.thebetawebsite.com/privacy (a standalone page is recommended; the Store needs a policy because the app uses the microphone and sends audio over the network). Interim option: https://bolo.thebetawebsite.com/#privacy

Policy must state: audio is recorded only while the hotkey is held; it is sent to Groq for transcription (and optional cleanup) and not stored by Bolo; API keys are kept in Windows Credential Manager; no ads or tracking.

## Store questionnaire notes
- Capabilities: Microphone (dictation), runFullTrust (global hotkey and paste into other apps). Justification for runFullTrust: "Bolo is a desktop app that listens for a global hotkey and inserts text into the active app; this requires full-trust desktop access."
- Age rating: no user-generated content shared and no purchases. Expect 3+/Everyone.
- Screenshots needed (1366x768 or larger): settings window, overlay while listening, before/after dictation in a text editor.
- Support contact: website URL or support email.
