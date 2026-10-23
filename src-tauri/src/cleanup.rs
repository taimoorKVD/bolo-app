//! Transcript cleanup.
//!
//! With the setting off, only standalone hesitation sounds ("um", "uh") are
//! removed. With it on, the transcript is sent to a Llama model hosted by Groq
//! to fix punctuation and apply spoken self-corrections. No other provider is used.

use anyhow::{anyhow, Context};
use serde::Deserialize;
use zeroize::Zeroize;

/// Groq's smallest fast chat model. Cleanup has a hard timeout and falls back
/// to the local editor, so a slow response never holds the paste.
pub const CLEANUP_MODEL: &str = "llama-3.1-8b-instant";
const CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Slack, Discord, Teams.
    Casual,
    /// Mail and documents.
    Formal,
    /// Everything else.
    Plain,
    /// Editors and terminals: do not paraphrase.
    Verbatim,
}

impl Tone {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Casual => "casual",
            Self::Formal => "formal",
            Self::Plain => "plain",
            Self::Verbatim => "verbatim",
        }
    }
}

#[derive(Debug)]
pub struct CleanOutcome {
    pub text: String,
    /// True when the Llama cleanup actually produced the text.
    pub used_model: bool,
    pub note: Option<String>,
    /// Why `used_model` is true or false. Written to the stage log.
    pub why: String,
}

pub async fn clean_transcript(
    client: &reqwest::Client,
    api_key: &str,
    transcript: &str,
    enabled: bool,
    tone: Tone,
    verbatim: bool,
) -> CleanOutcome {
    if verbatim {
        return skipped(transcript.trim().to_string(), "verbatim");
    }
    let fallback = polish_locally(transcript);
    if !enabled {
        return skipped(fallback, "off");
    }
    if matches!(tone, Tone::Verbatim) {
        return skipped(fallback, "verbatim");
    }
    if transcript.trim().is_empty() {
        return skipped(fallback, "empty");
    }
    let first = request_cleanup(client, api_key, transcript, tone, false).await;
    let first_outcome = outcome_from_model(transcript, first);
    if first_outcome.used_model {
        return first_outcome;
    }
    // The happy path is one request. A failure gets one simpler retry, and the
    // stage log keeps both reasons.
    let second = request_cleanup(client, api_key, transcript, tone, true).await;
    let second_outcome = outcome_from_model(transcript, second);
    if second_outcome.used_model {
        return CleanOutcome {
            text: second_outcome.text,
            used_model: true,
            note: None,
            why: compact_why(&format!("applied after retry ({})", first_outcome.why)),
        };
    }
    CleanOutcome {
        text: second_outcome.text,
        used_model: false,
        note: second_outcome.note.or(first_outcome.note),
        why: compact_why(&format!("{}; retry: {}", first_outcome.why, second_outcome.why)),
    }
}

fn skipped(text: String, why: &str) -> CleanOutcome {
    CleanOutcome {
        text,
        used_model: false,
        note: None,
        why: why.to_string(),
    }
}

/// Turn a model response into the text we paste, and a reason safe to log.
fn outcome_from_model(transcript: &str, result: anyhow::Result<String>) -> CleanOutcome {
    let fallback = polish_locally(transcript);
    match result {
        Ok(cleaned) => {
            let cleaned = salvage_model_text(&cleaned);
            if accept_cleanup(transcript, &cleaned) {
                return CleanOutcome {
                    text: polish_model_output(&cleaned),
                    used_model: true,
                    note: None,
                    why: "applied".into(),
                };
            }
            let trimmed = cleaned.trim();
            let why = if trimmed.is_empty() {
                "rejected empty rewrite".into()
            } else {
                format!(
                    "rejected rewrite ({} chars, transcript {} chars)",
                    trimmed.chars().count(),
                    transcript.trim().chars().count()
                )
            };
            CleanOutcome {
                text: fallback,
                used_model: false,
                note: Some("Cleanup was skipped because the rewrite didn't look usable.".into()),
                why,
            }
        }
        Err(err) => CleanOutcome {
            text: fallback,
            used_model: false,
            note: None,
            why: compact_why(&format!("{err:#}")),
        },
    }
}

fn compact_why(value: &str) -> String {
    let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 220 {
        flat
    } else {
        let cut: String = flat.chars().take(220).collect();
        format!("{cut}…")
    }
}

/// Local pass used when the model is off, too slow, or the focused app is code.
pub fn polish_locally(input: &str) -> String {
    let text = strip_fillers(input);
    let text = apply_self_correction(&text);
    apply_commands(&text)
}

fn polish_model_output(input: &str) -> String {
    let text = apply_self_correction(input);
    apply_commands(&text)
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
    let orig_words = content_tokens(original);
    let cand_words = content_tokens(candidate);
    if cand_words.is_empty() {
        return !orig_words.is_empty() && candidate.chars().count() <= 8;
    }
    if !multiset_subset(&cand_words, &orig_words) {
        return false;
    }
    let dropped = orig_words.len().saturating_sub(cand_words.len());
    let allowance = (orig_words.len() / 3).max(4);
    dropped <= allowance
}

fn content_tokens(input: &str) -> Vec<String> {
    input
        .split_whitespace()
        .filter(|token| !is_filler_token(token))
        .filter_map(|token| {
            let trimmed = token.trim_matches(|c: char| !c.is_alphanumeric());
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_lowercase())
            }
        })
        .collect()
}

fn multiset_subset(candidate: &[String], original: &[String]) -> bool {
    let mut counts = std::collections::HashMap::<&str, usize>::new();
    for word in original {
        *counts.entry(word.as_str()).or_insert(0) += 1;
    }
    for word in candidate {
        let Some(left) = counts.get_mut(word.as_str()) else {
            return false;
        };
        if *left == 0 {
            return false;
        }
        *left -= 1;
    }
    true
}

/// English for mixed Urdu / Roman Urdu. Meaning stays; nothing is added.
pub async fn translate_preserving(
    client: &reqwest::Client,
    api_key: &str,
    transcript: &str,
) -> CleanOutcome {
    let original = transcript.trim();
    if original.is_empty() {
        return skipped(String::new(), "empty");
    }
    let system = "Translate this transcript into English. Preserve the meaning exactly. Do not add, drop, explain, or summarize. Keep names, numbers, and words that are already English unchanged. If the speech mixes Urdu and English, or Roman Urdu, translate only the parts that are not English. Return only the English text, with no preamble and no quotes.";
    let max_tokens = (original.chars().count() as u32 / 2 + 96).clamp(64, 400);
    let body = serde_json::json!({
        "model": CLEANUP_MODEL,
        "temperature": 0,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": original }
        ]
    });
    let result = async {
        let response = client
            .post("https://api.groq.com/openai/v1/chat/completions")
            .timeout(CLEANUP_TIMEOUT)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .context("couldn't reach Groq")?;
        let status = response.status();
        let bytes = response.text().await.context("empty translation response")?;
        if !status.is_success() {
            return Err(anyhow!(groq_error_message(status.as_u16(), &bytes)));
        }
        let parsed: ChatResponse = serde_json::from_str(&bytes).context("translation response was not JSON")?;
        let message = parsed
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message)
            .context("translation response had no text")?;
        message_text(message.content.as_ref()).context("translation response had no text")
    }
    .await;
    match result {
        Ok(text) => {
            let text = salvage_model_text(&text);
            if accept_translation(original, &text) {
                CleanOutcome {
                    text,
                    used_model: true,
                    note: None,
                    why: "translated".into(),
                }
            } else {
                CleanOutcome {
                    text: original.to_string(),
                    used_model: false,
                    note: None,
                    why: "rejected translation".into(),
                }
            }
        }
        Err(err) => CleanOutcome {
            text: original.to_string(),
            used_model: false,
            note: None,
            why: compact_why(&format!("{err:#}")),
        },
    }
}

fn accept_translation(original: &str, candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    let lower = candidate.to_ascii_lowercase();
    if lower.starts_with("here is")
        || lower.starts_with("here's")
        || lower.starts_with("sure,")
        || lower.starts_with("translation")
    {
        return false;
    }
    let orig = original.chars().count().max(1);
    candidate.chars().count() <= orig.saturating_mul(4).saturating_add(120)
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

fn style_line(tone: Tone) -> &'static str {
    match tone {
        Tone::Casual => "Tone: chat. Keep contractions. Light punctuation.",
        Tone::Formal => "Tone: email or document. Complete sentences and clear punctuation.",
        Tone::Plain | Tone::Verbatim => "Tone: neutral dictation. Punctuate sentences.",
    }
}

fn salvage_model_text(value: &str) -> String {
    let mut text = value.trim().to_string();
    if text.starts_with("```") {
        if let Some(newline) = text.find('\n') {
            text = text[newline + 1..].trim().to_string();
        }
        if let Some(fence) = text.rfind("```") {
            text = text[..fence].trim().to_string();
        }
    }
    text = unwrap_model_text(&text);
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    if lines.len() >= 2 && is_preamble(lines[0]) {
        text = lines[1..].join("\n");
    }
    text.trim().to_string()
}

fn is_preamble(line: &str) -> bool {
    let lower = line
        .trim()
        .trim_end_matches([':', '.', ' '])
        .to_ascii_lowercase();
    if lower.chars().count() > 80 {
        return false;
    }
    [
        "here is",
        "here's",
        "edited",
        "sure",
        "certainly",
        "transcript",
        "corrected",
        "the edited",
        "output",
    ]
    .iter()
    .any(|head| lower.starts_with(head))
}

async fn request_cleanup(
    client: &reqwest::Client,
    api_key: &str,
    transcript: &str,
    tone: Tone,
    plain: bool,
) -> anyhow::Result<String> {
    let system = if plain {
        "Correct this dictation. Return only the corrected words, with no introduction and no quotes.".to_string()
    } else {
        format!(
            "Punctuate a dictation transcript. Return only those words.\n{}\nDo not add words. Do not drop words, except standalone um or uh, and except the words a spoken correction replaces (scratch that, no wait, I mean). Do not paraphrase, explain, or wrap the result in quotes.",
            style_line(tone)
        )
    };
    let user = transcript.trim();
    let max_tokens = (transcript.chars().count() as u32 / 2 + if plain { 96 } else { 48 }).clamp(48, 320);
    let body = serde_json::json!({
        "model": CLEANUP_MODEL,
        "temperature": 0,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ]
    });
    let response = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .timeout(CLEANUP_TIMEOUT)
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
    let message = parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message)
        .context("cleanup response had no text")?;
    if let Some(text) = message_text(message.content.as_ref()) {
        return Ok(text);
    }
    if let Some(refusal) = message.refusal.filter(|text| !text.trim().is_empty()) {
        return Err(anyhow!(compact_why(&refusal)));
    }
    Err(anyhow!("cleanup response had no text"))
}

fn message_text(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                if let Some(text) = part.as_str() {
                    out.push_str(text);
                } else if let Some(text) = part.get("text").and_then(|item| item.as_str()) {
                    out.push_str(text);
                }
            }
            if out.trim().is_empty() { None } else { Some(out) }
        }
        _ => None,
    }
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
        429 => format!(
            "Groq is rate limiting requests right now ({detail}). Bolo.com will use the on-device model for this take if it's ready, then try Groq again next time."
        ),
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
    content: Option<serde_json::Value>,
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GroqErrorBody {
    error: Option<GroqErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct GroqErrorDetail {
    message: Option<String>,
}

/// Replace the last spoken self-correction and keep the words that survived it.
pub fn apply_self_correction(input: &str) -> String {
    let words: Vec<&str> = input.split_whitespace().collect();
    let markers: [&[&str]; 3] = [&["scratch", "that"], &["no", "wait"], &["i", "mean"]];
    let mut found: Option<(usize, usize)> = None;
    for (index, _) in words.iter().enumerate() {
        for marker in markers {
            if words.len() - index >= marker.len()
                && marker.iter().zip(&words[index..]).all(|(want, got)| same_word(got, want))
            {
                found = Some((index, marker.len()));
            }
        }
    }
    let Some((at, len)) = found else {
        return input.trim().to_string();
    };
    let before = &words[..at];
    let after = &words[at + len..];
    if after.is_empty() {
        return before.join(" ");
    }
    if before.is_empty() {
        return after.join(" ");
    }
    let drop_n = before.len().min(after.len()).clamp(1, 6).min(before.len());
    let keep = before.len() - drop_n;
    let mut kept: Vec<&str> = before[..keep].to_vec();
    kept.extend_from_slice(after);
    tidy_spacing(&kept.join(" "))
}

/// Spoken punctuation and line breaks. Longer phrases win over single words.
pub fn apply_commands(input: &str) -> String {
    let words: Vec<&str> = input.split_whitespace().collect();
    let commands: &[(&[&str], &str)] = &[
        (&["new", "paragraph"], "\n\n"),
        (&["new", "line"], "\n"),
        (&["question", "mark"], "?"),
        (&["exclamation", "point"], "!"),
        (&["exclamation", "mark"], "!"),
        (&["full", "stop"], "."),
        (&["open", "quote"], "\""),
        (&["close", "quote"], "\""),
        (&["semicolon"], ";"),
        (&["colon"], ":"),
        (&["comma"], ","),
        (&["period"], "."),
        (&["dash"], "—"),
    ];
    apply_commands_plain(&words, commands)
}

fn apply_commands_plain(words: &[&str], commands: &[(&[&str], &str)]) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < words.len() {
        let mut matched = false;
        for (phrase, insert) in commands {
            if words.len() - index >= phrase.len()
                && phrase.iter().zip(&words[index..]).all(|(want, got)| same_word(got, want))
            {
                push_command(&mut out, insert);
                index += phrase.len();
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(words[index]);
        index += 1;
    }
    tidy_newlines(&out)
}

fn push_command(out: &mut String, insert: &str) {
    if insert.starts_with('\n') {
        while out.ends_with(' ') {
            out.pop();
        }
        out.push_str(insert);
        return;
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out.push_str(insert);
    if insert != "\"" {
        out.push(' ');
    }
}

fn tidy_newlines(input: &str) -> String {
    let mut text = input.trim().to_string();
    while text.contains(" \n") {
        text = text.replace(" \n", "\n");
    }
    text.replace("  ", " ")
}

fn same_word(got: &str, want: &str) -> bool {
    let bare = got.trim_matches(|c: char| matches!(c, ',' | '.' | '!' | '?' | ';' | ':' | '"' | '\''));
    bare.eq_ignore_ascii_case(want)
}

const LEARN_STOP: &[&str] = &[
    "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday", "January",
    "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December", "Today", "Tomorrow", "Please", "Thanks", "Hello", "There", "This",
    "That", "When", "What", "Where", "Which", "While", "After", "Before", "About", "Could",
    "Would", "Should", "Their", "They", "Them",
];

/// Remember capitalized names that are not just the first word of the take.
pub fn learn_terms(text: &str, dictionary: &mut Vec<String>) -> Vec<String> {
    let mut added = Vec::new();
    for (index, word) in text.split_whitespace().enumerate() {
        if added.len() >= 3 || dictionary.len() >= 200 {
            break;
        }
        if index == 0 {
            continue;
        }
        let bare = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if !learnable(bare) {
            continue;
        }
        if dictionary.iter().any(|term| term.eq_ignore_ascii_case(bare)) {
            continue;
        }
        dictionary.push(bare.to_string());
        added.push(bare.to_string());
    }
    added
}

fn learnable(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 4 || chars.len() > 40 {
        return false;
    }
    if !chars[0].is_uppercase() || !chars.iter().any(|c| c.is_lowercase()) {
        return false;
    }
    if !chars.iter().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    !LEARN_STOP.iter().any(|stop| stop.eq_ignore_ascii_case(word))
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
        assert!(!accept_cleanup(original, "Please ship it tomorrow right away"));
        assert!(accept_cleanup(
            "Let's meet Tuesday scratch that Wednesday",
            "Let's meet Wednesday"
        ));
    }

    #[test]
    fn verbatim_toggle_keeps_every_word() {
        let outcome = tauri::async_runtime::block_on(clean_transcript(
            crate::transcribe::http_client(),
            "unused",
            "deploy um scratch that ship it period",
            true,
            Tone::Plain,
            true,
        ));
        assert!(!outcome.used_model);
        assert_eq!(outcome.why, "verbatim");
        assert_eq!(outcome.text, "deploy um scratch that ship it period");
        assert!(!accept_translation("سلام", "Here is the translation: hello"));
        assert!(accept_translation("یہ English ہے", "This is English"));
    }

    #[test]
    fn local_editor_applies_corrections_and_commands() {
        assert_eq!(
            apply_self_correction("Let's meet Tuesday scratch that Wednesday"),
            "Let's meet Wednesday"
        );
        assert_eq!(
            apply_self_correction("The budget is fifty thousand no wait fifty-five thousand"),
            "The budget is fifty-five thousand"
        );
        assert_eq!(polish_locally("Email um Sarah I mean Sam"), "Email Sam");
        assert_eq!(apply_commands("Ship it tomorrow period"), "Ship it tomorrow.");
        assert_eq!(apply_commands("First line new line second line"), "First line\nsecond line");
        assert_eq!(apply_commands("Is this right question mark"), "Is this right?");
    }

    #[test]
    fn learns_a_mid_sentence_name_and_skips_weekdays() {
        let mut dictionary = vec!["Cadence".into()];
        let added = learn_terms("Meet Priya on Wednesday about Tauri", &mut dictionary);
        assert_eq!(added, vec!["Priya".to_string(), "Tauri".to_string()]);
        assert!(dictionary.iter().any(|term| term == "Priya"));
        assert!(!dictionary.iter().any(|term| term == "Wednesday"));
        assert!(learn_terms("Priya again", &mut dictionary).is_empty());
    }

    #[test]
    fn verbatim_skips_the_model_and_still_polishes() {
        let outcome = tauri::async_runtime::block_on(clean_transcript(
            crate::transcribe::http_client(),
            "unused",
            "deploy um scratch that ship it period",
            true,
            Tone::Verbatim,
            false,
        ));
        assert!(!outcome.used_model);
        assert_eq!(outcome.why, "verbatim");
        assert_eq!(outcome.text, "ship it.");
    }

    #[test]
    fn disabled_cleanup_records_why_and_still_polishes() {
        let outcome = tauri::async_runtime::block_on(clean_transcript(
            crate::transcribe::http_client(),
            "unused",
            "hello um there",
            false,
            Tone::Plain,
            false,
        ));
        assert!(!outcome.used_model);
        assert_eq!(outcome.why, "off");
        assert_eq!(outcome.text, "hello there");
        assert!(outcome.note.is_none());
    }

    #[test]
    fn model_error_falls_back_and_keeps_the_reason() {
        let outcome = outcome_from_model("hello um there", Err(anyhow!("timed out talking to Groq")));
        assert!(!outcome.used_model);
        assert_eq!(outcome.text, "hello there");
        assert!(outcome.note.is_none());
        assert!(outcome.why.contains("timed out"));
        assert!(!outcome.why.contains('\n'));
    }

    #[test]
    fn empty_or_huge_rewrite_is_rejected_with_a_reason() {
        let empty = outcome_from_model("ship it tomorrow", Ok("   ".into()));
        assert!(!empty.used_model);
        assert_eq!(empty.why, "rejected empty rewrite");
        assert_eq!(empty.text, "ship it tomorrow");
        assert!(empty.note.is_some());

        let essay = "word ".repeat(80);
        let huge = outcome_from_model("ship it tomorrow", Ok(essay));
        assert!(!huge.used_model);
        assert!(huge.why.contains("rejected rewrite"));
        assert!(huge.why.contains("transcript"));
    }

    #[test]
    fn usable_rewrite_is_marked_applied() {
        let outcome = outcome_from_model("ship it tomorrow", Ok("Ship it tomorrow.".into()));
        assert!(outcome.used_model);
        assert_eq!(outcome.why, "applied");
        assert_eq!(outcome.text, "Ship it tomorrow.");
    }

    #[test]
    fn preamble_and_fences_still_count_as_applied() {
        let fenced = outcome_from_model("ship it tomorrow", Ok("```\nShip it tomorrow.\n```".into()));
        assert!(fenced.used_model, "{}", fenced.why);
        assert_eq!(fenced.text, "Ship it tomorrow.");
        let preamble = outcome_from_model(
            "ship it tomorrow",
            Ok("Here is the edit:\nShip it tomorrow.".into()),
        );
        assert!(preamble.used_model, "{}", preamble.why);
        assert_eq!(preamble.text, "Ship it tomorrow.");
    }

    #[test]
    fn maps_auth_errors_without_echoing_a_key() {
        let message = groq_error_message(401, "{\"error\":{\"message\":\"invalid\"}}");
        assert!(message.contains("API key"));
        assert!(!message.to_lowercase().contains("gsk_"));
    }
}
