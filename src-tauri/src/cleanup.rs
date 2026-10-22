//! Transcript cleanup.
//!
//! With the setting off, only standalone hesitation sounds ("um", "uh") are
//! removed. With it on, the transcript is sent to a Llama model hosted by Groq
//! to fix punctuation and apply spoken self-corrections. No other provider is used.

use anyhow::{anyhow, Context};
use serde::Deserialize;
use zeroize::Zeroize;

pub const CLEANUP_MODEL: &str = "llama-3.3-70b-versatile";

const SYSTEM_PROMPT: &str = "\
You edit a speech-to-text transcript. Return only the corrected transcript.

Rules:
- Remove filler and hesitation sounds (um, uh, er, ah, hmm) when they are not part of a real word.
- Fix punctuation and capitalization.
- Apply spoken self-corrections. If the speaker says \"scratch that\", \"no wait\", \"I mean\", or similar, keep the replacement and drop the abandoned phrase.
- Do not answer questions, summarize, or add information that was not spoken.
- Do not wrap the result in quotes or add a preamble.
- If the transcript is already clean, return it unchanged.

Examples:
- \"Let's meet Tuesday scratch that Wednesday\" -> \"Let's meet Wednesday.\"
- \"Email um Sarah, I mean Sam, at example.com\" -> \"Email Sam at example.com.\"
- \"The budget is fifty thousand no wait fifty-five thousand\" -> \"The budget is fifty-five thousand.\"";

#[derive(Debug)]
pub struct CleanOutcome {
    pub text: String,
    /// True when the Llama cleanup actually produced the text.
    pub used_model: bool,
    pub note: Option<String>,
}

pub async fn clean_transcript(
    client: &reqwest::Client,
    api_key: &str,
    transcript: &str,
    enabled: bool,
) -> CleanOutcome {
    let fallback = strip_fillers(transcript);
    if !enabled {
        return CleanOutcome {
            text: fallback,
            used_model: false,
            note: None,
        };
    }
    if transcript.trim().is_empty() {
        return CleanOutcome {
            text: String::new(),
            used_model: false,
            note: None,
        };
    }
    match request_cleanup(client, api_key, transcript).await {
        Ok(cleaned) if accept_cleanup(transcript, &cleaned) => CleanOutcome {
            text: cleaned,
            used_model: true,
            note: None,
        },
        Ok(_) => CleanOutcome {
            text: fallback,
            used_model: false,
            note: Some("Cleanup was skipped because the rewrite didn't look usable.".into()),
        },
        Err(err) => CleanOutcome {
            text: fallback,
            used_model: false,
            note: Some(format!("Cleanup was skipped ({err}).")),
        },
    }
}

/// Remove standalone "um" / "uh" (and stretched forms like "umm").
/// Hyphenated words such as "uh-huh" are left alone.
pub fn strip_fillers(input: &str) -> String {
    let mut kept = Vec::new();
    for token in input.split_whitespace() {
        if !is_filler_token(token) {
            kept.push(token);
        }
    }
    tidy_spacing(&kept.join(" "))
}

pub fn accept_cleanup(original: &str, candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    let limit = original.chars().count().saturating_mul(2).saturating_add(80);
    candidate.chars().count() <= limit.max(32)
}

fn is_filler_token(token: &str) -> bool {
    if token.contains('-') || token.contains('—') {
        return false;
    }
    let trimmed = token.trim_matches(|c: char| {
        matches!(c, ',' | '.' | '!' | '?' | ';' | ':' | '"' | '\'' | '“' | '”' | '(' | ')')
    });
    let lower = trimmed.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'u' {
        return false;
    }
    let mark = bytes[1];
    if mark != b'm' && mark != b'h' {
        return false;
    }
    bytes[1..].iter().all(|byte| *byte == mark)
}

fn tidy_spacing(input: &str) -> String {
    let mut out = input.trim().to_string();
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out = out
        .replace(" ,", ",")
        .replace(" .", ".")
        .replace(" !", "!")
        .replace(" ?", "?")
        .replace(" ;", ";")
        .replace(" :", ":");
    while out.contains(",,") {
        out = out.replace(",,", ",");
    }
    while out.contains(", ,") {
        out = out.replace(", ,", ",");
    }
    let trimmed = out.trim().trim_start_matches([',', ';', ':']).trim();
    trimmed.to_string()
}

async fn request_cleanup(
    client: &reqwest::Client,
    api_key: &str,
    transcript: &str,
) -> anyhow::Result<String> {
    let user = format!("Transcript:\n\"\"\"\n{}\n\"\"\"", transcript.trim());
    let max_tokens = (transcript.chars().count() as u32 / 2 + 128).clamp(128, 1024);
    let body = serde_json::json!({
        "model": CLEANUP_MODEL,
        "temperature": 0,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": user }
        ]
    });
    let response = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .context("couldn't reach Groq")?;
    let status = response.status();
    let bytes = response.text().await.context("empty cleanup response")?;
    if !status.is_success() {
        return Err(anyhow!(groq_error_message(status.as_u16(), &bytes)));
    }
    let parsed: ChatResponse = serde_json::from_str(&bytes).context("cleanup response was not JSON")?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .context("cleanup response had no text")?;
    Ok(unwrap_model_text(&content))
}

fn unwrap_model_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        return trimmed[1..trimmed.len() - 1].trim().to_string();
    }
    trimmed.to_string()
}

pub(crate) fn groq_error_message(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<GroqErrorBody>(body)
        .ok()
        .and_then(|parsed| parsed.error.and_then(|error| error.message))
        .unwrap_or_else(|| compact(body, 180));
    match status {
        401 | 403 => "Groq rejected the API key. Check it in Settings.".into(),
        413 => "The recording was too large to upload.".into(),
        429 => format!("Groq rate limit reached. {detail}"),
        _ => format!("Groq returned {status}. {detail}"),
    }
}

fn compact(body: &str, max: usize) -> String {
    let one_line = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let cut: String = one_line.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GroqErrorBody {
    error: Option<GroqErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct GroqErrorDetail {
    message: Option<String>,
}

/// Best-effort wipe of a secret that is about to be dropped.
pub fn wipe(secret: &mut String) {
    secret.zeroize();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_um_and_uh() {
        assert_eq!(strip_fillers("Um, I think so"), "I think so");
        assert_eq!(strip_fillers("hello um world"), "hello world");
        assert_eq!(strip_fillers("hello, uh, world"), "hello, world");
        assert_eq!(strip_fillers("I um, think"), "I think");
        assert_eq!(strip_fillers("the umbrella is uh red"), "the umbrella is red");
        assert_eq!(strip_fillers("uh-huh, sure"), "uh-huh, sure");
        assert_eq!(strip_fillers("umm uhh okay"), "okay");
        assert_eq!(strip_fillers(""), "");
    }

    #[test]
    fn rejects_runaway_rewrites() {
        let original = "ship it tomorrow";
        assert!(accept_cleanup(original, "Ship it tomorrow."));
        let essay = "word ".repeat(80);
        assert!(!accept_cleanup(original, &essay));
        assert!(!accept_cleanup(original, "   "));
    }

    #[test]
    fn maps_auth_errors_without_echoing_a_key() {
        let message = groq_error_message(401, "{\"error\":{\"message\":\"invalid\"}}");
        assert!(message.contains("API key"));
        assert!(!message.to_lowercase().contains("gsk_"));
    }
}
