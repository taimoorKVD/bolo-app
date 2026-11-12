//! Transcript cleanup.
//!
//! With the setting off, only standalone hesitation sounds ("um", "uh") are
//! removed. With it on, the transcript is sent to a Llama model hosted by Groq
//! to fix punctuation and apply spoken self-corrections. No other provider is used.

use anyhow::anyhow;
use serde::Deserialize;
use zeroize::Zeroize;

/// Fast Groq instruct model. `llama-3.1-8b-instant` was removed for the
/// developer tier on 16 Aug 2026 and now returns 404.
pub const CLEANUP_MODEL: &str = "openai/gpt-oss-20b";
const TRANSLATE_MODEL: &str = "openai/gpt-oss-120b";

/// Tried in order when the preferred model is missing. The saved setting is
/// first; these follow so a 404 does not fail the paste.
pub const CLEANUP_MODELS: &[&str] = &[
    "openai/gpt-oss-20b",
    "openai/gpt-oss-120b",
    // qwen/qwen3.6-27b was shut down for developer keys on 14 Sep 2026.
    "qwen/qwen3.8-27b",
];
/// Punctuation cleanup gives up after 1.2s and pastes the transcript it already has.
pub const CLEANUP_TIMEOUT_MS: u64 = 1_200;
const CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(CLEANUP_TIMEOUT_MS);
const TRANSLATE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(9000);

fn model_order(preferred: &str) -> Vec<String> {
    let mut out = Vec::new();
    let preferred = preferred.trim();
    if !preferred.is_empty() {
        out.push(preferred.to_string());
    }
    for model in CLEANUP_MODELS {
        if !out.iter().any(|have| have == *model) {
            out.push((*model).to_string());
        }
    }
    if out.is_empty() {
        out.push(CLEANUP_MODEL.to_string());
    }
    out
}

fn is_missing_model(status: u16, body: &str) -> bool {
    if status == 404 {
        return true;
    }
    // 400: the id is gone. 403: org or project permissions block that model
    // while the same key still transcribes with Whisper. A bare 401/403
    // ("invalid api key") is a real key failure and stays here as false.
    if status == 400 || status == 403 {
        return model_unavailable_body(body);
    }
    false
}

fn model_unavailable_body(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "does not exist",
        "do not have access",
        "model_not_found",
        "model_not_available",
        "unknown model",
        "invalid model",
        "decommissioned",
        "no longer supported",
        "has been deprecated",
        "model_permission",
        "permissions_error",
        "is blocked",
        "blocked at the",
    ];
    if MARKERS.iter().any(|marker| lower.contains(marker)) {
        return true;
    }
    lower.contains("not found") && lower.contains("model")
}

/// HTTP status plus a short redacted body for the stage log. Never the API key,
/// and never the user-facing "brief problem" sentence that used to hide it.
pub(crate) fn provider_detail(status: u16, body: &str) -> String {
    let failure = crate::provider::classify_to_failure(status, body, false);
    crate::builtin::redact_secrets(&failure.log_detail)
}

pub(crate) enum CleanupFail {
    /// This model id is gone or not allowed. `detail` is safe to log.
    Missing { detail: String },
    Other(String),
}

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
    model: &str,
    using_builtin: bool,
) -> CleanOutcome {
    // Restore "easy ssh" before a model can treat the mis-hearing as a sentence to finish.
    let transcript = crate::transcribe::recover_brief_tech(transcript);
    let transcript = transcript.as_str();
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
    // A few words have nothing to restructure. The model was inventing a sentence around them.
    if brief_phrase(transcript) {
        return skipped(fallback, "brief");
    }
    let models = model_order(model);
    let mut missed = Vec::new();
    let mut last_detail = String::new();
    for model_id in &models {
        match request_cleanup(client, api_key, transcript, tone, false, model_id, using_builtin).await {
            Err(CleanupFail::Missing { detail }) => {
                last_detail = detail;
                missed.push(model_id.clone());
                continue;
            }
            other => {
                let first_outcome = outcome_from_model(transcript, cleanup_result(other));
                if first_outcome.used_model {
                    return note_model(first_outcome, model_id, &missed);
                }
                // A timeout or a dead connection already has the raw transcript.
                // Waiting out a second attempt is what made those takes feel stuck.
                if !should_retry_cleanup(&first_outcome.why) {
                    return note_fallback(first_outcome, &missed);
                }
                // A removed model already moved on. Any other failure gets one
                // simpler retry on this model, then the local edit.
                match request_cleanup(client, api_key, transcript, tone, true, model_id, using_builtin).await {
                    Err(CleanupFail::Missing { detail }) => {
                        last_detail = detail;
                        missed.push(model_id.clone());
                        continue;
                    }
                    second => {
                        let second_outcome = outcome_from_model(transcript, cleanup_result(second));
                        if second_outcome.used_model {
                            return note_model(
                                CleanOutcome {
                                    text: second_outcome.text,
                                    used_model: true,
                                    note: None,
                                    why: compact_why(&format!(
                                        "applied after retry ({})",
                                        first_outcome.why
                                    )),
                                },
                                model_id,
                                &missed,
                            );
                        }
                        let mut why = format!("{}; retry: {}", first_outcome.why, second_outcome.why);
                        if !missed.is_empty() {
                            why = format!("model missing: {}; {why}", missed.join(", "));
                        }
                        return CleanOutcome {
                            text: second_outcome.text,
                            used_model: false,
                            note: second_outcome.note.or(first_outcome.note),
                            why: compact_why(&why),
                        };
                    }
                }
            }
        }
    }
    let mut why = format!("model missing: {}", missed.join(", "));
    if !last_detail.is_empty() {
        why = format!("{why}; {last_detail}");
    }
    CleanOutcome {
        text: polish_locally(transcript),
        used_model: false,
        note: None,
        why: compact_why(&why),
    }
}

fn note_fallback(mut outcome: CleanOutcome, missed: &[String]) -> CleanOutcome {
    if !missed.is_empty() {
        outcome.why = compact_why(&format!("model missing: {}; {}", missed.join(", "), outcome.why));
    }
    outcome
}

/// Network failures and timeouts paste the transcript already in hand.
/// An empty or rejected rewrite may still be worth one simpler retry.
pub(crate) fn should_retry_cleanup(why: &str) -> bool {
    let lower = why.to_ascii_lowercase();
    // A 4xx from the provider will not succeed by repeating the same body.
    // Model-not-available responses never reach here: they walk the model list.
    if lower.contains("http 400") || lower.contains("http 401") || lower.contains("http 403") {
        return false;
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return true;
    }
    !(lower.contains("couldn't reach")
        || lower.contains("could not reach")
        || lower.contains("error sending request"))
}

fn cleanup_result(result: Result<String, CleanupFail>) -> anyhow::Result<String> {
    match result {
        Ok(text) => Ok(text),
        Err(CleanupFail::Missing { detail }) => {
            Err(anyhow!(if detail.is_empty() { "model missing".to_string() } else { detail }))
        }
        Err(CleanupFail::Other(message)) => Err(anyhow!(message)),
    }
}

fn note_model(mut outcome: CleanOutcome, model: &str, missed: &[String]) -> CleanOutcome {
    outcome.why = if missed.is_empty() {
        compact_why(&format!("{} model={model}", outcome.why))
    } else {
        compact_why(&format!(
            "{} model={model} after {}",
            outcome.why,
            missed.join(", ")
        ))
    };
    outcome
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
            // The whole rewrite can be too free (an added word) and still contain
            // one sound-alike that is right. Keep that swap, drop the rest.
            if let Some(saved) = salvage_homophones(transcript, &cleaned, &crate::vocab::current()) {
                return CleanOutcome {
                    text: polish_model_output(&saved),
                    used_model: true,
                    note: None,
                    why: "applied salvaged homophones".into(),
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
    if flat.chars().count() <= 360 {
        flat
    } else {
        let cut: String = flat.chars().take(360).collect();
        format!("{cut}…")
    }
}

/// Local pass used when the model is off, too slow, or the focused app is code.
pub fn polish_locally(input: &str) -> String {
    let text = strip_fillers(input);
    let text = apply_self_correction(&text);
    apply_commands(&text)
}

/// The model already applied spoken corrections and punctuation. Only the
/// unambiguous markers run again here: "I mean" and single words such as
/// "period" are ordinary English ("what I mean is", "the trial period") in a
/// rewrite, so they are left alone unless a single command word ends the take.
pub(crate) fn polish_model_output(input: &str) -> String {
    let text = self_correction(input, false);
    commands_with(&text, true)
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

/// The user's protected words, and the one kind of word change allowed.
fn vocab_line() -> String {
    let words = crate::vocab::current();
    let mut line = String::from(
        "\nIf a word is a sound-alike that makes no sense in the sentence (roots for wrote, there for their), use the word the whole sentence needs; change nothing else. Write a spoken \"dot com\" in a web address as .com.",
    );
    if !words.is_empty() {
        line.push_str("\nSpell these exactly as written here, including case and dots: ");
        line.push_str(&words.join(", "));
        line.push('.');
    }
    line
}

pub fn accept_cleanup(original: &str, candidate: &str) -> bool {
    accept_cleanup_with(original, candidate, &crate::vocab::current())
}

/// The rewrite may only reuse the transcript's words, except a few
/// sound-alike swaps ("roots" -> "wrote") and words from the protected list
/// ("taimoorhussain dot com" -> "taimoorhussain.com").
pub fn accept_cleanup_with(original: &str, candidate: &str, vocab: &[String]) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    let orig_words = content_tokens(original);
    let cand_words = content_tokens(candidate);
    if cand_words.is_empty() {
        return !orig_words.is_empty() && candidate.chars().count() <= 8;
    }
    if !multiset_subset(&cand_words, &orig_words) && !allowed_swaps(&cand_words, &orig_words, vocab) {
        return false;
    }
    let length_drop = orig_words.len().saturating_sub(cand_words.len());
    let folded = unmatched_words(&orig_words, &cand_words)
        .iter()
        .filter(|word| folded_into_spelling(word, &cand_words, vocab))
        .count();
    // A same-length sound-alike swap is not a drop. "dot com" folding into a
    // protected spelling is not one either. Other missing words are.
    let unexplained = length_drop.saturating_sub(folded);
    unexplained <= drop_allowance(original, orig_words.len())
}

/// "dot" / "com" disappearing into taimoorhussain.com is a spelling, not a deleted word.
fn folded_into_spelling(word: &str, candidate: &[String], vocab: &[String]) -> bool {
    let word_l = word.to_ascii_lowercase();
    let glue = matches!(word_l.as_str(), "dot" | "at" | "underscore" | "dash" | "hyphen");
    for entry in vocab {
        let entry_l = entry.to_ascii_lowercase();
        let parts: Vec<&str> = entry_l.split(|c: char| !c.is_ascii_alphanumeric()).filter(|part| !part.is_empty()).collect();
        let present = candidate.iter().any(|token| token.trim_end_matches('.').eq_ignore_ascii_case(entry));
        if !present {
            continue;
        }
        if glue || parts.iter().any(|part| *part == word_l) {
            return true;
        }
    }
    false
}

fn unmatched_words<'a>(original: &'a [String], candidate: &[String]) -> Vec<&'a str> {
    let mut left: Vec<&str> = original.iter().map(String::as_str).collect();
    for word in candidate {
        if let Some(index) = left.iter().position(|have| *have == word.as_str() || crate::grammar::same_lexeme(have, word)) {
            left.swap_remove(index);
        }
    }
    left
}

/// How many content words a cleanup rewrite may omit.
///
/// Fillers are already gone from the token lists. The old rule allowed four
/// drops on every take (`len / 3`, at least 4), so a six-word sentence could
/// lose most of its meaning and still be pasted. A spoken correction
/// ("scratch that") is the case that is supposed to drop words.
pub(crate) fn drop_allowance(original: &str, count: usize) -> usize {
    if has_spoken_correction(original) {
        (count / 2).max(3)
    } else {
        (count / 10).min(2)
    }
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

fn allowed_swaps(candidate: &[String], original: &[String], vocab: &[String]) -> bool {
    let mut left: Vec<&str> = original.iter().map(String::as_str).collect();
    let mut extra: Vec<&str> = Vec::new();
    for word in candidate {
        if let Some(i) = left.iter().position(|w| *w == word.as_str() || crate::grammar::same_lexeme(w, word)) {
            left.swap_remove(i);
        } else {
            extra.push(word.as_str());
        }
    }
    let protected: Vec<String> = vocab
        .iter()
        .flat_map(|w| {
            let lower = w.to_lowercase();
            let mut parts: Vec<String> = lower
                .split_whitespace()
                .map(|p| p.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
                .collect();
            parts.push(lower.split_whitespace().collect());
            parts
        })
        .collect();
    let budget = (original.len() / 8).max(1);
    let mut swaps = 0;
    for word in extra {
        if protected.iter().any(|p| p == word) {
            continue;
        }
        let Some(i) = left.iter().position(|w| crate::vocab::sounds_alike(w, word) && consonant_swap_ok(w, word)) else {
            return false;
        };
        left.swap_remove(i);
        swaps += 1;
        if swaps > budget {
            return false;
        }
    }
    true
}

/// "ssh" sounds a bit like "such". That swap is how a product name became a sentence.
fn consonant_swap_ok(original: &str, candidate: &str) -> bool {
    !consonant_token(original) || consonant_token(candidate)
}

fn consonant_token(word: &str) -> bool {
    let letters: String = word.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    letters.len() >= 2 && !letters.chars().any(|c| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y'))
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
    model: &str,
    using_builtin: bool,
) -> CleanOutcome {
    let original = transcript.trim();
    if original.is_empty() {
        return skipped(String::new(), "empty");
    }
    let system = "You translate a speech-to-text transcript of someone dictating in Urdu (often mixed with English words, sometimes Roman Urdu) into the natural English they meant. The transcript comes from automatic speech recognition and may contain mis-heard words; use the surrounding context to recover the intended word (for example a word that sounds like a tech term). Common mis-hearings: بیٹری (battery) is usually بہتری (improvement, betterment) unless a real battery fits; بہتر means better. Keep English words the speaker used, especially names, brands, emails, URLs, and technical words (WhatsApp, GitHub, animation, background, box, responsive, robot, website, app), spelled as they are. Write proper English: correct grammar, punctuation, and capitalization. Translate faithfully and completely: every point the speaker made, in the same order, as clear first-person English. Do not summarize, add ideas, answer questions, or explain. Return only the English text, with no preamble and no quotes.";
    let max_tokens = (original.chars().count() as u32 / 2 + 96).clamp(64, 400);
    // Urdu meaning needs the bigger model; the small one translated word by word.
    let mut models = model_order(model);
    if let Some(pos) = models.iter().position(|m| m == TRANSLATE_MODEL) {
        let big = models.remove(pos);
        models.insert(0, big);
    }
    let mut missed = Vec::new();
    let mut last_detail = String::new();
    for model_id in &models {
        match post_chat(client, api_key, model_id, system, original, max_tokens, using_builtin, TRANSLATE_TIMEOUT, 768).await {
            Err(CleanupFail::Missing { detail }) => {
                last_detail = detail;
                missed.push(model_id.clone());
                continue;
            }
            Err(CleanupFail::Other(err)) => {
                let why = if missed.is_empty() {
                    err
                } else {
                    format!("model missing: {}; {err}", missed.join(", "))
                };
                return CleanOutcome {
                    text: original.to_string(),
                    used_model: false,
                    note: None,
                    why: compact_why(&why),
                };
            }
            Ok(text) => {
                let text = salvage_model_text(&text);
                if accept_translation(original, &text) {
                    let why = if missed.is_empty() {
                        format!("translated model={model_id}")
                    } else {
                        format!("translated model={model_id} after {}", missed.join(", "))
                    };
                    return CleanOutcome {
                        text,
                        used_model: true,
                        note: None,
                        why: compact_why(&why),
                    };
                }
                return CleanOutcome {
                    text: original.to_string(),
                    used_model: false,
                    note: None,
                    why: "rejected translation".into(),
                };
            }
        }
    }
    let mut why = format!("model missing: {}", missed.join(", "));
    if !last_detail.is_empty() {
        why = format!("{why}; {last_detail}");
    }
    CleanOutcome {
        text: original.to_string(),
        used_model: false,
        note: None,
        why: compact_why(&why),
    }
}

pub(crate) fn accept_translation(original: &str, candidate: &str) -> bool {
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
    if !crate::hints::translation_keeps_english(original, candidate) {
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
        Tone::Formal => "Tone: email or document. Clear punctuation. Leave a short phrase short.",
        Tone::Plain | Tone::Verbatim => "Tone: neutral dictation. Punctuate sentences.",
    }
}

/// Same-length rewrites only. A protected spelling is never taken from the
/// model. Anything that is not the same word or a sound-alike aborts, and the
/// caller pastes the transcript it already had.
pub(crate) fn salvage_homophones(original: &str, candidate: &str, vocab: &[String]) -> Option<String> {
    let orig: Vec<&str> = original.split_whitespace().collect();
    let cand: Vec<&str> = candidate.split_whitespace().collect();
    if orig.is_empty() || orig.len() != cand.len() {
        return None;
    }
    let mut changed = false;
    let mut out = Vec::with_capacity(orig.len());
    for (before, after) in orig.iter().zip(cand.iter()) {
        let left = alnum_core(before);
        let right = alnum_core(after);
        if left.is_empty() || right.is_empty() || left.eq_ignore_ascii_case(&right) {
            out.push((*after).to_string());
            continue;
        }
        if listed(&left, vocab) {
            out.push((*before).to_string());
            continue;
        }
        if crate::vocab::sounds_alike(&left, &right) && consonant_swap_ok(&left, &right) {
            out.push((*after).to_string());
            changed = true;
            continue;
        }
        return None;
    }
    changed.then(|| out.join(" "))
}

fn alnum_core(token: &str) -> String {
    token.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase()
}

fn listed(word: &str, vocab: &[String]) -> bool {
    vocab.iter().any(|entry| {
        entry.eq_ignore_ascii_case(word)
            || entry.split(|c: char| !c.is_alphanumeric()).any(|part| part.eq_ignore_ascii_case(word))
    })
}

pub(crate) fn salvage_model_text(value: &str) -> String {
    let value = final_channel(value).unwrap_or_else(|| value.to_string());
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

pub(crate) fn cleanup_system(tone: Tone, plain: bool) -> String {
    let system = if plain {
        "Correct this dictation. Return only the corrected words, with no introduction and no quotes. If it is a short phrase, return that phrase. Do not invent a sentence."
            .to_string()
    } else {
        format!(
            "Fix grammar, punctuation, capitalization, and sentence structure in a dictation transcript. Return only those words.\n{}\nKeep the speaker's words and meaning. You may change a word's form for agreement (go/goes/went, is/are/was, a/an, don't/doesn't) and you may capitalize. Do not add content words, names, or ideas. Do not drop words, except standalone um or uh, and except the words a spoken correction replaces (scratch that, no wait, I mean). If you are unsure what a word was, leave that word as it appears. Do not swap in a more common word. Do not paraphrase, explain, or wrap the result in quotes. A short phrase stays a short phrase: never expand it into a new sentence. Keep technical names and acronyms (EasySSH, SSH, API, HTTP, JSON, URL) spelled that way.",
            style_line(tone)
        ) + &style_samples_line()
    };
    system + &vocab_line()
}

/// Five content words or fewer, unless the speaker corrected themselves.
/// Those takes are punctuated locally. A model pass was rewriting them.
pub(crate) fn brief_phrase(text: &str) -> bool {
    if has_spoken_correction(text) {
        return false;
    }
    content_tokens(text).len() <= 5
}

fn has_spoken_correction(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("scratch that") || lower.contains("no wait") || lower.contains("i mean")
}

async fn request_cleanup(
    client: &reqwest::Client,
    api_key: &str,
    transcript: &str,
    tone: Tone,
    plain: bool,
    model: &str,
    using_builtin: bool,
) -> Result<String, CleanupFail> {
    let system = cleanup_system(tone, plain);
    let user = transcript.trim();
    let chars = user.chars().count();
    let max_tokens = (chars as u32 / 2 + if plain { 96 } else { 64 }).clamp(64, 400);
    // Reasoning used to be a flat 768 tokens, which blew the 1.2s budget on
    // short takes and the paste fell back to raw text. Scale the wait and the
    // reasoning headroom with the length of this take.
    let timeout = std::time::Duration::from_millis(crate::grammar::cleanup_timeout_ms(chars));
    let pad = (chars as u32 / 6 + 96).clamp(96, 320);
    let text = post_chat(client, api_key, model, &system, user, max_tokens, using_builtin, timeout, pad).await?;
    if text.trim().is_empty() {
        return Err(CleanupFail::Other("cleanup response had no text".into()));
    }
    Ok(text)
}

pub(crate) async fn complete_chat(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
    using_builtin: bool,
    timeout: std::time::Duration,
    reasoning_pad: u32,
) -> Result<String, CleanupFail> {
    post_chat(client, api_key, model, system, user, max_tokens, using_builtin, timeout, reasoning_pad).await
}

/// gpt-oss puts the answer after a final channel. When that was left in
/// `content` or `reasoning` and `content` itself was empty, cleanup logged
/// "had no text" and the take kept the raw transcript.
fn final_channel(value: &str) -> Option<String> {
    let marker = "<|channel|>final";
    let at = value.rfind(marker)?;
    let rest = &value[at + marker.len()..];
    let body = rest.split_once("<|message|>").map(|(_, text)| text).unwrap_or(rest);
    let body = body.trim().trim_end_matches("<|return|>").trim();
    if body.is_empty() { None } else { Some(body.to_string()) }
}

async fn post_chat(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
    _using_builtin: bool,
    timeout: std::time::Duration,
    reasoning_pad: u32,
) -> Result<String, CleanupFail> {
    let mut body = serde_json::json!({
        "model": model,
        "temperature": 0,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ]
    });
    // gpt-oss models reason before answering, and reasoning tokens count
    // against max_tokens. With the small budget used here the answer was
    // often cut off before any text ("cleanup response had no text").
    // gpt-oss reasons before it answers, and those tokens count against the
    // completion budget. `max_tokens` is deprecated and was ignored, so the
    // answer was cut off ("cleanup response had no text", 6 takes in the log).
    // `max_completion_tokens` is the budget Groq actually applies.
    if model.starts_with("openai/gpt-oss") {
        let budget = max_tokens.saturating_add(reasoning_pad);
        body["reasoning_effort"] = serde_json::json!("low");
        body["reasoning_format"] = serde_json::json!("hidden");
        body["max_completion_tokens"] = serde_json::json!(budget);
        body["max_tokens"] = serde_json::json!(budget);
    } else if model.starts_with("qwen/") {
        body["reasoning_effort"] = serde_json::json!("none");
        body["max_completion_tokens"] = serde_json::json!(max_tokens);
    } else {
        body["max_completion_tokens"] = serde_json::json!(max_tokens);
    }
    let response = match client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .timeout(timeout)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            let failure = crate::provider::network_to_failure(crate::transcribe::error_chain(&err));
            return Err(CleanupFail::Other(failure.message));
        }
    };
    let status = response.status();
    let bytes = match response.text().await {
        Ok(bytes) => bytes,
        Err(err) => return Err(CleanupFail::Other(format!("empty cleanup response: {err}"))),
    };
    if !status.is_success() {
        let code = status.as_u16();
        // Log the status and a redacted body. The friendly key sentence hid
        // every 401 and 403, including a model the org has not allowed, so the
        // next model was never tried and the stage log could not say why.
        let detail = provider_detail(code, &bytes);
        if is_missing_model(code, &bytes) {
            return Err(CleanupFail::Missing { detail });
        }
        return Err(CleanupFail::Other(detail));
    }
    let parsed: ChatResponse = match serde_json::from_str(&bytes) {
        Ok(parsed) => parsed,
        Err(_) => return Err(CleanupFail::Other("cleanup response was not JSON".into())),
    };
    let Some(message) = parsed.choices.into_iter().next().map(|choice| choice.message) else {
        return Err(CleanupFail::Other("cleanup response had no text".into()));
    };
    if let Some(text) = message_text(message.content.as_ref()) {
        if let Some(final_text) = final_channel(&text) {
            return Ok(final_text);
        }
        if !text.trim().is_empty() {
            return Ok(text);
        }
    }
    if let Some(reasoning) = message.reasoning.as_deref() {
        if let Some(final_text) = final_channel(reasoning) {
            return Ok(final_text);
        }
    }
    if let Some(refusal) = message.refusal.filter(|text| !text.trim().is_empty()) {
        return Err(CleanupFail::Other(compact_why(&refusal)));
    }
    Err(CleanupFail::Other("cleanup response had no text".into()))
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

pub(crate) fn groq_error_message(status: u16, body: &str, using_builtin: bool) -> String {
    let detail = serde_json::from_str::<GroqErrorBody>(body)
        .ok()
        .and_then(|parsed| parsed.error.and_then(|error| error.message))
        .unwrap_or_else(|| compact(body, 180));
    let message = match status {
        401 | 403 => crate::transcribe::REJECTED_KEY_MESSAGE.to_string(),
        413 => "The recording was too large to upload.".to_string(),
        429 if using_builtin => {
            "Bolo's built-in key is rate limited right now. Add your own Groq API key in Settings to keep dictating.".to_string()
        }
        429 => format!(
            "Groq is rate limiting requests right now ({detail}). Bolo will use the on-device model for this take if it's ready, then try Groq again next time."
        ),
        _ => format!("Groq returned {status}. {detail}"),
    };
    crate::builtin::redact_secrets(&message)
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
    /// Present when the final channel was not copied into `content`.
    #[serde(default, alias = "reasoning_content")]
    reasoning: Option<String>,
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
    self_correction(input, true)
}

/// "I mean" is a correction in "Email Sarah I mean Sam", not in
/// "what I mean is" or "I mean it".
fn i_mean_is_correction(words: &[&str], at: usize) -> bool {
    const NOT_AFTER: &[&str] = &["what", "you", "i", "do", "if", "know"];
    const NOT_BEFORE: &[&str] = &[
        "is", "was", "it", "that", "this", "to", "by", "what", "well", "yes", "no", "really", "business",
    ];
    if at == 0 {
        return false;
    }
    if NOT_AFTER.iter().any(|w| same_word(words[at - 1], w)) {
        return false;
    }
    match words.get(at + 2) {
        Some(next) => !NOT_BEFORE.iter().any(|w| same_word(next, w)),
        None => false,
    }
}

fn self_correction(input: &str, allow_i_mean: bool) -> String {
    let words: Vec<&str> = input.split_whitespace().collect();
    let markers: [&[&str]; 3] = [&["scratch", "that"], &["no", "wait"], &["i", "mean"]];
    let mut found: Option<(usize, usize)> = None;
    for (index, _) in words.iter().enumerate() {
        for marker in markers {
            if words.len() - index >= marker.len()
                && marker.iter().zip(&words[index..]).all(|(want, got)| same_word(got, want))
                && (marker[0] != "i" || (allow_i_mean && i_mean_is_correction(&words, index)))
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
    commands_with(input, false)
}

fn commands_with(input: &str, strict: bool) -> String {
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
    apply_commands_plain(&words, commands, strict)
}

/// A lone command word after "the", "a", "trial"... is a noun, not punctuation.
fn single_word_is_noun(words: &[&str], index: usize) -> bool {
    const BEFORE: &[&str] = &[
        "a", "an", "the", "this", "that", "each", "every", "per", "my", "your", "our", "their", "his",
        "her", "its", "trial", "grace", "time", "waiting", "notice", "billing", "free", "first", "last",
        "next", "same", "one", "long", "short", "em", "en",
    ];
    const AFTER: &[&str] = &["of", "cancer", "is", "was", "ends", "ended", "starts", "board", "cam"];
    let before = index.checked_sub(1).map(|i| words[i]);
    if before.is_some_and(|w| BEFORE.iter().any(|b| same_word(w, b))) {
        return true;
    }
    words.get(index + 1).is_some_and(|w| AFTER.iter().any(|a| same_word(w, a)))
}

fn apply_commands_plain(words: &[&str], commands: &[(&[&str], &str)], strict: bool) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < words.len() {
        let mut matched = false;
        for (phrase, insert) in commands {
            if words.len() - index >= phrase.len()
                && phrase.iter().zip(&words[index..]).all(|(want, got)| same_word(got, want))
                && (phrase.len() > 1
                    || (!single_word_is_noun(words, index) && (!strict || index + 1 == words.len())))
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

/// Ordinary English words that get capitalized at a sentence start or by the
/// model and were being learned as "terms" (Then, Currently, Whatever, Easy,
/// Override, Like, Urdu...). They're never worth putting in the prompt.
const COMMON_WORDS: &[&str] = &[
    "about", "above", "after", "again", "against", "also", "always", "another", "anyone", "anything", "around", "because",
    "before", "being", "below", "between", "both", "bring", "cannot", "change", "check", "close", "come", "could",
    "currently", "daily", "different", "doing", "done", "down", "during", "each", "early", "easy", "either", "else",
    "enough", "even", "ever", "every", "everyone", "everything", "example", "fine", "first", "following", "from", "full",
    "give", "going", "good", "great", "have", "having", "here", "however", "just", "keep", "kind", "know",
    "last", "later", "least", "left", "less", "like", "little", "look", "made", "make", "many", "maybe",
    "mean", "might", "more", "most", "much", "must", "need", "never", "next", "nice", "note", "nothing",
    "number", "only", "open", "other", "over", "override", "part", "people", "place", "please", "point", "pretty",
    "probably", "quite", "rather", "ready", "really", "right", "same", "says", "second", "seems", "send", "should",
    "show", "side", "simple", "since", "small", "some", "something", "sometimes", "soon", "sorry", "start", "still",
    "such", "sure", "take", "than", "thank", "thanks", "that", "their", "them", "then", "there", "these",
    "thing", "things", "think", "this", "those", "though", "through", "time", "today", "together", "tomorrow", "totally",
    "under", "until", "very", "want", "well", "were", "what", "whatever", "when", "where", "whether", "which", "wrote",
    "while", "will", "with", "within", "without", "work", "would", "wrong", "yeah", "year", "years", "your",
    "hello", "okay", "alright", "actually", "basically", "literally", "anyway", "also", "maybe", "english", "urdu", "hindi",
    "arabic", "project", "software", "architect", "system", "update", "version", "design", "section", "product", "page", "button",
    "settings", "website", "application", "issue", "problem", "testing", "working", "words",
    // Ordinary words that must not be promoted into names.
    "invoice", "deadline", "roadmap",
    "committee", "team", "lead", "senior", "vision", "wired", "alternative", "proportion", "proportional",
];

/// One-take mishearings that were saved as if they were names. Not prompted again.
const ONE_OFF_MISHEARS: &[&str] = &[
    "aand", "volo", "polo", "poloapp", "boloapp", "roomba", "roombos", "crock", "droc", "smtb", "grov",
    "passat", "justine", "tori", "cfs", "dbcd", "proportual",
];

const KEEP_ACRONYMS: &[&str] = &[
    "API", "SSH", "SQL", "HTTP", "HTTPS", "JSON", "URL", "AWS", "CSS", "HTML", "PDF",
];

/// Drop a saved dictionary entry that is ordinary English, a known mishearing,
/// or a short acronym that was not one of the real technical terms above.
/// Emails, domains, and mixed-case brands stay.
pub fn should_drop_learned(word: &str) -> bool {
    let word = word.trim();
    if word.is_empty() {
        return true;
    }
    if word.contains('@') || word.contains('.') {
        return false;
    }
    if crate::vocab::is_seed(word) || is_common_word(word) {
        return is_common_word(word);
    }
    let lower = word.to_ascii_lowercase();
    if ONE_OFF_MISHEARS.iter().any(|bad| bad.eq_ignore_ascii_case(&lower)) {
        return true;
    }
    let letters = word.chars().filter(|c| c.is_ascii_alphabetic()).count();
    let acronym = letters >= 2
        && letters <= 5
        && word.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    acronym && !KEEP_ACRONYMS.iter().any(|keep| keep.eq_ignore_ascii_case(word))
}

/// Test helper: `learn_terms_with` with a per-thread pending list.
#[cfg(test)]
pub fn learn_terms(text: &str, dictionary: &mut Vec<String>) -> Vec<String> {
    thread_local! {
        static PENDING: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    PENDING.with(|pending| learn_terms_with(text, dictionary, &mut pending.borrow_mut(), &[]))
}

/// Remember brand/tech-shaped terms (GitHub, ReactJS, AWS, Win11) at once, and
/// plain capitalized names only after they show up mid-sentence in two takes.
/// Common English words and sentence-start words are never learned.
/// `pending` holds names seen once and is saved in memory.json so it survives
/// a restart. `blocked` words are never learned.
pub fn learn_terms_with(
    text: &str,
    dictionary: &mut Vec<String>,
    pending: &mut Vec<String>,
    blocked: &[String],
) -> Vec<String> {
    let mut added = Vec::new();
    let mut seen_this_take: Vec<String> = Vec::new();
    let mut after_stop = true;
    for word in text.split_whitespace() {
        let sentence_start = after_stop;
        after_stop = word.ends_with(['.', '?', '!', ':']);
        if added.len() >= 3 || dictionary.len() >= 200 {
            break;
        }
        if sentence_start {
            continue;
        }
        let bare = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if !learnable(bare) {
            continue;
        }
        if crate::vocab::intended_term(bare).is_some() || should_drop_learned(bare) {
            continue;
        }
        if blocked.iter().any(|term| term.eq_ignore_ascii_case(bare)) {
            continue;
        }
        if dictionary.iter().any(|term| term.eq_ignore_ascii_case(bare)) {
            continue;
        }
        if near_miss_of_known(bare, dictionary) {
            // "BoloM" next to "Bolo", "KBHRMS" next to "KVHRMS": a
            // mishearing of a word Bolo already knows, not a new word.
            continue;
        }
        // Every term, tech-shaped or not, needs a second take before it is
        // learned. 0.1.19 learned "BoloM" and "DDeveloper" from one take.
        if seen_this_take.iter().any(|w| w.eq_ignore_ascii_case(bare)) {
            continue;
        }
        if let Some(pos) = pending.iter().position(|w| w.eq_ignore_ascii_case(bare)) {
            pending.remove(pos);
        } else {
            seen_this_take.push(bare.to_string());
            pending.push(bare.to_string());
            if pending.len() > 200 {
                pending.remove(0);
            }
            continue;
        }
        dictionary.push(bare.to_string());
        added.push(bare.to_string());
    }
    added
}

/// A few of the user's own recent sentences, from memory.json.
static STYLE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn set_style(samples: Vec<String>) {
    *STYLE.lock().unwrap_or_else(|e| e.into_inner()) = samples;
}

fn style_samples_line() -> String {
    let samples = STYLE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if samples.is_empty() {
        return String::new();
    }
    let joined: Vec<String> = samples.iter().take(3).map(|s| format!("\"{s}\"")).collect();
    format!(
        "\nFor reference only, this speaker usually writes like: {}. Match their spelling of names and their punctuation habits. Never copy words from these examples.",
        joined.join(" ")
    )
}

/// A tech-shaped or all-caps word one edit away from a known term
/// (KBHRMS vs KVHRMS, BoloM vs Bolo), or with a doubled leading capital
/// (DDeveloper). Plain names (Taha, Talha) are left alone, and a plural
/// "s" (API -> APIs) is not a near miss.
pub(crate) fn near_miss_of_known(word: &str, dictionary: &[String]) -> bool {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() >= 3
        && chars[0].is_ascii_uppercase()
        && chars[0] == chars[1]
        && chars[2].is_ascii_lowercase()
    {
        return true;
    }
    let caps = chars.len() >= 2 && chars.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if !(tech_shaped(word) || caps) {
        return false;
    }
    let lower = word.to_ascii_lowercase();
    dictionary.iter().any(|term| {
        let term_lower = term.to_ascii_lowercase();
        if term_lower.chars().count() < 4 || term_lower == lower {
            return false;
        }
        if word.ends_with('s') && word[..word.len() - 1].eq_ignore_ascii_case(term) {
            return false;
        }
        edit_distance_at_most_one(&lower, &term_lower)
    })
}

fn edit_distance_at_most_one(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let mut i = 0;
    while i < short.len() && short[i] == long[i] {
        i += 1;
    }
    if short.len() == long.len() {
        i >= short.len() || short[i + 1..] == long[i + 1..]
    } else {
        short[i..] == long[i + 1..]
    }
}

/// Internal capitals (GitHub), digits (Win11), or an acronym (AWS).
fn tech_shaped(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    let inner_upper = chars.iter().skip(1).any(|c| c.is_ascii_uppercase()) && chars.iter().any(|c| c.is_ascii_lowercase());
    let digit = chars.iter().any(|c| c.is_ascii_digit()) && chars.iter().any(|c| c.is_ascii_alphabetic());
    // Short all-caps tokens (SMTB, GROV) were learned from a single mishearing.
    // They wait for a second take, the same as a capitalized name.
    inner_upper || digit
}

pub fn is_common_word(word: &str) -> bool {
    LEARN_STOP.iter().chain(COMMON_WORDS.iter()).any(|stop| stop.eq_ignore_ascii_case(word))
}

fn learnable(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 2 || chars.len() > 40 {
        return false;
    }
    if !chars.iter().all(|c| c.is_ascii_alphanumeric()) || !chars[0].is_ascii_alphabetic() {
        return false;
    }
    let acronym = chars.len() >= 2 && chars.len() <= 6 && chars.iter().all(|c| c.is_ascii_uppercase());
    if !tech_shaped(word)
        && !acronym
        && (chars.len() < 4 || !chars[0].is_uppercase() || !chars.iter().any(|c| c.is_lowercase()))
    {
        return false;
    }
    !is_common_word(word)
}

/// Best-effort wipe of a secret that is about to be dropped.
pub fn wipe(secret: &mut String) {
    secret.zeroize();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_may_fix_homophones_and_protected_words_only() {
        let vocab = crate::vocab::protected(&[]);
        assert!(accept_cleanup_with("I roots the email yesterday", "I wrote the email yesterday.", &vocab));
        assert!(accept_cleanup_with(
            "send it to taimoorhussain dot com",
            "Send it to taimoorhussain.com.",
            &vocab
        ));
        assert!(accept_cleanup_with("open volo and start dictating", "Open Bolo and start dictating.", &vocab));
        assert!(accept_cleanup_with("i goes to the store", "I go to the store.", &vocab));
        assert!(!accept_cleanup_with("i goes to the store", "I went shopping later", &vocab));
        // Still rejected: added words, and too many swaps.
        assert!(!accept_cleanup_with("ship it tomorrow", "Ship it tomorrow, boss.", &vocab));
        assert!(!accept_cleanup_with("easy ssh", "easy, it's such are", &vocab));
        assert!(!accept_cleanup_with("use easy ssh today now", "use easy such today now", &vocab));
        assert!(!accept_cleanup_with("I roots the email", "I wrote the mail later", &vocab));
        assert!(!accept_cleanup_with("their cat sat there", "There cats sit their", &vocab));
    }

    #[test]
    fn cleanup_prompt_lists_protected_words() {
        let line = vocab_line();
        assert!(line.contains("taimoorhussain.com"));
        assert!(line.contains("roots for wrote"));
        let system = cleanup_system(Tone::Plain, false);
        assert!(system.contains("never expand it into a new sentence"));
        assert!(system.contains("leave that word as it appears"));
        assert!(system.contains("EasySSH"));
        assert!(!cleanup_system(Tone::Formal, false).contains("Complete sentences"));
        let plain = cleanup_system(Tone::Plain, true);
        assert!(plain.contains("Do not invent a sentence"));
    }

    #[test]
    fn a_short_technical_phrase_never_calls_the_model() {
        let outcome = tauri::async_runtime::block_on(clean_transcript(
            crate::transcribe::http_client(),
            "unused",
            "easy, it's such are",
            true,
            Tone::Formal,
            false,
            CLEANUP_MODEL,
            false,
        ));
        assert!(!outcome.used_model);
        assert_eq!(outcome.why, "brief");
        assert_eq!(outcome.text, "EasySSH");
        let garbled = tauri::async_runtime::block_on(clean_transcript(
            crate::transcribe::http_client(),
            "unused",
            "Bolo and Easy, it's such a truth. Nerd thing, imagination.",
            true,
            Tone::Plain,
            false,
            CLEANUP_MODEL,
            false,
        ));
        assert_eq!(garbled.why, "brief");
        assert_eq!(garbled.text, "EasySSH");
        assert!(brief_phrase("ship it tomorrow"));
        assert!(!brief_phrase("Email Sarah I mean Sam"));
        let sentence = tauri::async_runtime::block_on(clean_transcript(
            crate::transcribe::http_client(),
            "unused",
            "ship it tomorrow",
            true,
            Tone::Plain,
            false,
            CLEANUP_MODEL,
            false,
        ));
        assert_eq!(sentence.why, "brief");
        assert_eq!(sentence.text, "ship it tomorrow");
    }

    #[test]
    fn mishearings_of_known_terms_are_never_learned() {
        let mut dictionary = vec!["Bolo".to_string(), "KVHRMS".to_string(), "API".to_string(), "Taha".to_string()];
        for _ in 0..3 {
            learn_terms("so the BoloM app and KBHRMS and DDeveloper work", &mut dictionary);
        }
        assert_eq!(dictionary.len(), 4, "{dictionary:?}");
        assert!(near_miss_of_known("BoloM", &dictionary));
        assert!(near_miss_of_known("KBHRMS", &dictionary));
        assert!(near_miss_of_known("DDeveloper", &dictionary));
        assert!(!near_miss_of_known("APIs", &dictionary));
        assert!(!near_miss_of_known("Talha", &dictionary));
        assert!(!near_miss_of_known("GitHub", &dictionary));
    }

    #[test]
    fn a_word_repeated_inside_one_take_is_not_learned() {
        let mut dictionary = Vec::new();
        assert!(learn_terms("ask Zorbex then Zorbex then Zorbex", &mut dictionary).is_empty());
        assert_eq!(learn_terms("call Zorbex now", &mut dictionary), vec!["Zorbex".to_string()]);
    }

    #[test]
    fn never_learns_common_words_and_learns_tech_terms_at_once() {
        let mut dictionary = Vec::new();
        for _ in 0..3 {
            learn_terms("Ok so Then we go. Currently it is Easy, Whatever, Override it Like that in Urdu and English", &mut dictionary);
        }
        assert!(dictionary.is_empty(), "{dictionary:?}");
        // Tech terms need a second take too (0.1.19 learned "BoloM" at once).
        assert!(learn_terms("we deploy on GitHub with AWS and Win11", &mut dictionary).is_empty());
        let added = learn_terms("we deploy on GitHub with Win11 again", &mut dictionary);
        assert_eq!(added, vec!["GitHub".to_string(), "Win11".to_string()]);
        let added = learn_terms("use AWS again tomorrow", &mut dictionary);
        assert_eq!(added, vec!["AWS".to_string()]);
        assert!(should_drop_learned("Only"));
        assert!(should_drop_learned("SMTB"));
        assert!(should_drop_learned("Volo"));
        assert!(!should_drop_learned("Taimoor"));
        assert!(!should_drop_learned("priya.sharma@kingdomvision.com"));
        assert!(!should_drop_learned("GitHub"));
        assert!(is_common_word("currently") && !is_common_word("Laravel"));
    }

    #[test]
    fn missing_model_falls_through_the_list() {
        let order = model_order("llama-3.1-8b-instant");
        assert_eq!(order[0], "llama-3.1-8b-instant");
        assert_eq!(order[1], "openai/gpt-oss-20b");
        assert_eq!(order[2], "openai/gpt-oss-120b");
        assert_eq!(order[3], "qwen/qwen3.8-27b");
        assert_eq!(model_order(""), vec![
            "openai/gpt-oss-20b".to_string(),
            "openai/gpt-oss-120b".to_string(),
            "qwen/qwen3.8-27b".to_string(),
        ]);
        assert_eq!(model_order("openai/gpt-oss-20b").len(), 3);
        let gone = r#"{"error":{"message":"The model `llama-3.1-8b-instant` does not exist or you do not have access to it."}}"#;
        assert!(is_missing_model(404, gone));
        assert!(is_missing_model(400, "The model does not exist"));
        assert!(!is_missing_model(429, gone));
        assert!(!is_missing_model(401, "invalid api key"));
        // A blocked model is HTTP 403, which used to be reported as a rejected
        // key. Whisper on the same key still returns 200. Walk the list.
        let blocked = r#"{"error":{"message":"The model `openai/gpt-oss-20b` is blocked at the organization level.","type":"permissions_error","code":"model_permission_blocked_org"}}"#;
        assert!(is_missing_model(403, blocked));
        assert!(is_missing_model(403, "The model has been decommissioned"));
        assert!(!is_missing_model(403, "invalid api key"));
        assert!(!is_missing_model(403, ""));
        assert!(!is_missing_model(401, blocked));
        let detail = provider_detail(403, &format!("{blocked} gsk_testsecretvalue123"));
        assert!(detail.starts_with("http 403"), "{detail}");
        assert!(detail.to_ascii_lowercase().contains("blocked"), "{detail}");
        assert!(!detail.contains("gsk_"), "{detail}");
        assert!(!detail.contains(crate::transcribe::REJECTED_KEY_MESSAGE), "{detail}");
        assert_eq!(CLEANUP_MODEL, "openai/gpt-oss-20b");
    }

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
        assert!(
            !accept_cleanup(
                "please send the invoice to Ahmed by Friday morning",
                "Send the invoice Friday."
            ),
            "dropping most of a sentence is a paraphrase"
        );
        assert_eq!(drop_allowance("please send the invoice to Ahmed by Friday morning", 9), 0);
        assert!(drop_allowance("Let's meet Tuesday scratch that Wednesday", 6) >= 3);
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
            CLEANUP_MODEL,
            false,
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
        let mut dictionary = vec!["Bolo".into()];
        assert!(learn_terms("Meet Priya on Wednesday about Tauri", &mut dictionary).is_empty());
        let added = learn_terms("Call Priya then ship Tauri", &mut dictionary);
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
            CLEANUP_MODEL,
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
            CLEANUP_MODEL,
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
    fn unreachable_cleanup_is_not_retried_and_a_homophone_can_be_salvaged() {
        assert!(should_retry_cleanup("couldn't reach Groq: timed out"));
        assert!(should_retry_cleanup("error sending request: timeout"));
        assert!(!should_retry_cleanup("couldn't reach Groq"));
        assert!(should_retry_cleanup("rejected empty rewrite"));
        assert!(should_retry_cleanup("rejected rewrite (40 chars, transcript 12 chars)"));
        assert!(!should_retry_cleanup("http 403 The model is blocked"));
        assert!(!should_retry_cleanup("http 401 invalid api key"));
        let saved = salvage_homophones(
            "I roots the email yesterday",
            "I wrote the email yesterday",
            &crate::vocab::protected(&[]),
        );
        assert_eq!(saved.as_deref(), Some("I wrote the email yesterday"));
        assert!(salvage_homophones("Open Bolo today", "Open Volo today", &crate::vocab::protected(&[])).is_none());
        assert!(salvage_homophones("ship it", "ship it tomorrow", &[]).is_none());
        let channeled = salvage_model_text("<|channel|>analysis<|message|>thinking<|channel|>final<|message|>Ship it tomorrow.");
        assert_eq!(channeled, "Ship it tomorrow.");
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
        let message = groq_error_message(401, "{\"error\":{\"message\":\"invalid\"}}", false);
        assert_eq!(message, crate::transcribe::REJECTED_KEY_MESSAGE);
        let lower = message.to_lowercase();
        assert!(!lower.contains("settings"));
        assert!(!lower.contains("windows"));
        assert!(!lower.contains("api key"));
        assert!(!lower.contains("gsk_"));
        let builtin = groq_error_message(403, "{\"error\":{\"message\":\"invalid\"}}", true);
        assert_eq!(builtin, message);
    }

    #[test]
    fn builtin_rate_limit_suggests_an_own_key() {
        let body = "{\"error\":{\"message\":\"rate limit gsk_test_fixture_key_value_xxxxxxxx\"}}";
        let message = groq_error_message(429, body, true);
        assert!(message.contains("built-in key is rate limited"));
        assert!(message.contains("Add your own Groq API key"));
        assert!(!message.contains("gsk_"), "{message}");
        let own = groq_error_message(429, "{\"error\":{\"message\":\"rate limit\"}}", false);
        assert!(own.contains("rate limiting requests right now"));
        assert!(!own.contains("Add your own Groq API key"));
    }
}
