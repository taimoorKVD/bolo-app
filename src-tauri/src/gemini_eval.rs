//! Optional Gemini transcription comparison. Not part of a take.
//!
//! Documented model, read from Google's guide on 2026-11-06:
//! <https://ai.google.dev/gemini-api/docs/generate-content/transcribe>
//! and the model card
//! <https://ai.google.dev/gemini-api/docs/models/gemini-3.5-transcribe>.
//!
//! - Model id: `gemini-3.5-transcribe`
//! - Call: `POST /v1beta/models/gemini-3.5-transcribe:generateContent`
//! - Header: `x-goog-api-key` (never a query string, never logged)
//! - The published guide uploads audio with the Files API, then passes `fileData`.
//!   This harness sends `inlineData` instead, so a lab clip is not left in a
//!   Gemini file store. WAV and FLAC are documented MIME types.
//! - `generationConfig.audioTranscriptionConfig.mode` is `VERBATIM`.
//!   `SMART` is documented to drop fillers and rewrite spoken corrections
//!   ("actually no, Wednesday" becomes a different sentence). That fights
//!   scratch-that / no-wait / I-mean, so it is not a candidate for Bolo.
//!
//! Fallback stays off. Nothing here is called from dictation. A live run
//! happens only when both `BOLO_GEMINI_EVAL=1` and `GEMINI_API_KEY` are set,
//! against the lab clip `tests/fixtures/dictation.wav` (the spoken line is
//! "Hello um cadence this is a dictation test."). That clip is not an accent
//! sample. No latency or accuracy number is invented when the call does not run.

use serde_json::{json, Value};

/// Dictation never calls Gemini until a reviewed eval says it is safe.
pub const FALLBACK_ENABLED: bool = false;
/// A live call also requires `BOLO_GEMINI_EVAL=1`. The default test sends nothing.
pub const SPENDING_ENABLED: bool = false;
pub const MODEL: &str = "gemini-3.5-transcribe";
pub const ENDPOINT: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-transcribe:generateContent";
/// Lab clip already in the repo. Not a person's accent sample.
pub const LAB_REFERENCE: &str = "Hello um cadence this is a dictation test.";

pub fn request_body(mime: &str, audio: &[u8]) -> Value {
    json!({
        "contents": [{
            "parts": [{
                "inlineData": {
                    "mimeType": mime,
                    "data": base64(audio)
                }
            }]
        }],
        "generationConfig": {
            "audioTranscriptionConfig": {
                "mode": "VERBATIM"
            }
        }
    })
}

pub fn transcript_from(body: &Value) -> Option<String> {
    let parts = body.pointer("/candidates/0/content/parts")?.as_array()?;
    let mut text = String::new();
    for part in parts {
        if let Some(piece) = part.get("text").and_then(|value| value.as_str()) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(piece.trim());
        }
    }
    let text = text.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let n = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8) | bytes[index + 2] as u32;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        index += 3;
    }
    if index < bytes.len() {
        let left = bytes.len() - index;
        let mut n = (bytes[index] as u32) << 16;
        if left == 2 {
            n |= (bytes[index + 1] as u32) << 8;
        }
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if left == 2 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_fallback_stays_off_and_the_request_is_verbatim() {
        assert!(!FALLBACK_ENABLED);
        assert!(!SPENDING_ENABLED);
        assert_eq!(MODEL, "gemini-3.5-transcribe");
        assert!(!ENDPOINT.contains("key="));
        let body = request_body("audio/wav", b"RIFF");
        assert_eq!(
            body.pointer("/generationConfig/audioTranscriptionConfig/mode").and_then(|v| v.as_str()),
            Some("VERBATIM")
        );
        let encoded = body.pointer("/contents/0/parts/0/inlineData/data").and_then(|v| v.as_str()).unwrap();
        assert_eq!(encoded, "UklGRg==");
        assert!(body.to_string().contains("audio/wav"));
        assert!(!body.to_string().contains("SMART"));
        let parsed = transcript_from(&json!({"candidates":[{"content":{"parts":[{"text":"Hello um cadence"}]}}]}));
        assert_eq!(parsed.as_deref(), Some("Hello um cadence"));
        assert!(transcript_from(&json!({"error":{"message":"nope"}})).is_none());
    }

    /// Repeatable live comparison. Prints the blocker and passes when it cannot run.
    /// It does not claim an accent score. The lab line is the fixture's known words.
    #[test]
    fn gemini_lab_fixture_is_opt_in() {
        if std::env::var("BOLO_GEMINI_EVAL").ok().as_deref() != Some("1") {
            eprintln!(
                "GEMINI_EVAL blocked: BOLO_GEMINI_EVAL is not 1, so no request was sent. Fallback remains disabled. Documented model is {MODEL} in VERBATIM mode. This is not an accuracy or latency result."
            );
            return;
        }
        let Ok(key) = std::env::var("GEMINI_API_KEY") else {
            eprintln!(
                "GEMINI_EVAL blocked: GEMINI_API_KEY is unset. No request was sent. Fallback remains disabled. This is not an accuracy or latency result."
            );
            return;
        };
        if key.trim().is_empty() {
            eprintln!("GEMINI_EVAL blocked: GEMINI_API_KEY is empty. No request was sent.");
            return;
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dictation.wav");
        let Ok(audio) = std::fs::read(&path) else {
            eprintln!("GEMINI_EVAL blocked: lab fixture missing at {}", path.display());
            return;
        };
        let client = crate::transcribe::http_client();
        let body = request_body("audio/wav", &audio);
        let started = std::time::Instant::now();
        let result = tauri::async_runtime::block_on(async {
            client
                .post(ENDPOINT)
                .header("x-goog-api-key", key.trim())
                .json(&body)
                .send()
                .await
        });
        let elapsed_ms = started.elapsed().as_millis();
        let response = match result {
            Ok(response) => response,
            Err(err) => {
                let message = crate::builtin::redact_secrets(&err.to_string());
                eprintln!("GEMINI_EVAL blocked: request failed after {elapsed_ms} ms ({message}). No accuracy number. Fallback remains disabled.");
                return;
            }
        };
        let status = response.status();
        let raw = match tauri::async_runtime::block_on(response.text()) {
            Ok(text) => text,
            Err(err) => {
                let message = crate::builtin::redact_secrets(&err.to_string());
                eprintln!("GEMINI_EVAL blocked: could not read the response after {elapsed_ms} ms ({message}). No accuracy number.");
                return;
            }
        };
        if !status.is_success() {
            let detail = crate::builtin::redact_secrets(&raw.chars().take(180).collect::<String>());
            eprintln!(
                "GEMINI_EVAL blocked: HTTP {status} after {elapsed_ms} ms. {detail} No accuracy number. Fallback remains disabled."
            );
            return;
        }
        let parsed: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => {
                eprintln!("GEMINI_EVAL blocked: response was not JSON after {elapsed_ms} ms. No accuracy number.");
                return;
            }
        };
        let Some(text) = transcript_from(&parsed) else {
            eprintln!("GEMINI_EVAL blocked: response had no transcript after {elapsed_ms} ms. No accuracy number.");
            return;
        };
        let score = crate::harness::wer(LAB_REFERENCE, &text);
        eprintln!(
            "GEMINI_EVAL lab_fixture wer={score:.3} elapsed_ms={elapsed_ms} mode=VERBATIM model={MODEL} note=lab clip only, not an accent benchmark. Fallback remains disabled."
        );
        let _ = text;
    }
}
