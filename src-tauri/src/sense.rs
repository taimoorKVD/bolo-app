//! Sentence-aware homophone repair.
//!
//! High-confidence swaps ("I roots the email" -> "I wrote the email") run
//! locally so a take stays instant. A leftover ambiguous word can be checked
//! with the small Groq model (`openai/gpt-oss-20b`, the cheap fast production
//! model after `llama-3.1-8b-instant` left the developer tier). The model may
//! change only a sound-alike, must keep My words spelled exactly, and on any
//! failure the text already in hand is what gets pasted.

use std::time::Duration;

use crate::cleanup::{self, CleanupFail};

/// Hard cap so a meaning check cannot make dictation feel slow.
/// Generation of a one-word fix on gpt-oss-20b is a few hundred milliseconds;
/// anything slower is discarded and the local text is pasted.
pub const SENSE_TIMEOUT: Duration = Duration::from_millis(700);
pub const SENSE_TIMEOUT_MS: u64 = 700;

const PRONOUNS: &[&str] = &["i", "you", "we", "they", "he", "she", "who", "please", "just"];
const GERUNDS: &[&str] = &[
    "going", "coming", "trying", "doing", "saying", "getting", "making", "taking", "looking", "running", "leaving", "starting", "here",
];
const NOUNS: &[&str] = &[
    "cat", "dog", "email", "house", "car", "friend", "team", "office", "name", "phone", "book", "bag", "account", "website", "message",
    "letter",
];
const TOO_NEXT: &[&str] = &[
    "much", "many", "late", "early", "soon", "long", "short", "big", "small", "fast", "slow", "hard", "easy", "good", "bad", "far", "hot",
    "cold",
];
const COUNT_NEXT: &[&str] = &[
    "day", "days", "week", "weeks", "hour", "hours", "minute", "minutes", "month", "months", "year", "years", "people", "times",
];
const COUNT_PREV: &[&str] = &["for", "in", "after", "within", "about", "around"];
const SEND: &[&str] = &["send", "check", "read", "got", "get", "wrote"];
const LOC_PREV: &[&str] = &[
    "over", "out", "from", "than", "to", "in", "on", "at", "sit", "sat", "go", "goes", "went", "stay", "stayed", "put", "left", "right",
    "be",
];
const ROOTS_NOUN: &[&str] = &[
    "the",
    "a",
    "an",
    "our",
    "my",
    "its",
    "their",
    "these",
    "those",
    "deep",
    "plant",
    "grass",
    "cultural",
    "square",
    "historical",
    "strong",
    "shallow",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenseOutcome {
    pub text: String,
    pub used_model: bool,
    pub latency_ms: u64,
    pub why: String,
}

/// Local, deterministic homophone pass. Protected spellings are left alone.
pub fn local_homophones(text: &str, protected: &[String]) -> String {
    let mut toks: Vec<String> = text.split_whitespace().map(|token| token.to_string()).collect();
    if toks.is_empty() {
        return text.to_string();
    }
    for index in 0..toks.len() {
        let word = core(&toks[index]);
        if word.is_empty() || is_protected(&word, protected) {
            continue;
        }
        let prev = index.checked_sub(1).map(|i| core(&toks[i])).unwrap_or_default();
        let prev2 = index.checked_sub(2).map(|i| core(&toks[i])).unwrap_or_default();
        let next = toks.get(index + 1).map(|token| core(token)).unwrap_or_default();
        let next2 = toks.get(index + 2).map(|token| core(token)).unwrap_or_default();
        let Some(replacement) = swap(&word, &prev, &prev2, &next, &next2) else {
            continue;
        };
        toks[index] = stitch(&toks[index], &replacement);
    }
    toks.join(" ")
}

/// True when a homophone is still sitting in a context the local rules refuse
/// to guess. Common sentences are false, so the model call (and its latency)
/// is skipped.
pub fn needs_model(text: &str) -> bool {
    let cores: Vec<String> = text.split_whitespace().map(|token| core(token)).collect();
    for (index, word) in cores.iter().enumerate() {
        if word != "roots" {
            continue;
        }
        let next = cores.get(index + 1).map(String::as_str).unwrap_or("");
        let prev = index.checked_sub(1).map(|i| cores[i].as_str()).unwrap_or("");
        if matches!(next, "of" | "for" | "and" | "or" | "") {
            continue;
        }
        if ROOTS_NOUN.contains(&prev) {
            continue;
        }
        return true;
    }
    false
}

pub fn accept_sense(original: &str, candidate: &str, protected: &[String]) -> bool {
    let original_words = cores(original);
    let candidate_words = cores(candidate);
    if original_words.len() != candidate_words.len() {
        return false;
    }
    let mut swaps = 0usize;
    for (before, after) in original_words.iter().zip(&candidate_words) {
        if before.eq_ignore_ascii_case(after) {
            continue;
        }
        if is_protected(before, protected) {
            return false;
        }
        if !crate::vocab::sounds_alike(before, after) {
            return false;
        }
        swaps += 1;
        if swaps > 2 {
            return false;
        }
    }
    true
}

/// Ask the small model to fix one leftover sound-alike. Any timeout, empty
/// answer, or rewrite that touches a protected spelling keeps `text`.
pub async fn model_fix(
    client: &reqwest::Client,
    api_key: &str,
    text: &str,
    model: &str,
    protected: &[String],
    using_builtin: bool,
) -> SenseOutcome {
    let started = std::time::Instant::now();
    let system = sense_system(protected);
    let result = cleanup::complete_chat(client, api_key, model, &system, text.trim(), 64, using_builtin, SENSE_TIMEOUT, 320).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(raw) => {
            let cleaned = cleanup::salvage_model_text(&raw);
            if accept_sense(text, &cleaned, protected) && !cleaned.trim().is_empty() {
                SenseOutcome {
                    text: cleaned,
                    used_model: true,
                    latency_ms,
                    why: format!("sense applied {latency_ms}ms"),
                }
            } else {
                SenseOutcome {
                    text: text.to_string(),
                    used_model: false,
                    latency_ms,
                    why: format!("sense rejected {latency_ms}ms; kept transcript"),
                }
            }
        }
        Err(CleanupFail::Missing) => SenseOutcome {
            text: text.to_string(),
            used_model: false,
            latency_ms,
            why: format!("sense model missing {latency_ms}ms; kept transcript"),
        },
        Err(CleanupFail::Other(err)) => SenseOutcome {
            text: text.to_string(),
            used_model: false,
            latency_ms,
            why: format!("sense fallback {latency_ms}ms: {err}"),
        },
    }
}

fn sense_system(protected: &[String]) -> String {
    let mut system = String::from(
        "Fix only a word that does not fit this dictated sentence because it is the wrong sound-alike. \
Examples: roots becomes wrote in \"I roots the email\"; their becomes they're in \"their going home\"; \
to becomes too in \"to much\". Do not add words. Do not drop words. Do not change punctuation, names, \
or word order. If the sentence already fits, return it unchanged. Return only the sentence.",
    );
    if !protected.is_empty() {
        let shown: Vec<&str> = protected.iter().take(40).map(String::as_str).collect();
        system.push_str(" Never change these spellings: ");
        system.push_str(&shown.join(", "));
        system.push('.');
    }
    system
}

fn swap(word: &str, prev: &str, prev2: &str, next: &str, next2: &str) -> Option<&'static str> {
    match word {
        "roots" if PRONOUNS.contains(&prev) && !matches!(next, "of" | "for" | "and" | "or") => Some("wrote"),
        "write" if next == "now" || next == "away" => Some("right"),
        "write"
            if matches!(prev, "the" | "a" | "an")
                && matches!(next, "way" | "one" | "time" | "thing" | "place" | "choice" | "answer" | "word") =>
        {
            Some("right")
        }
        "right"
            if PRONOUNS.contains(&prev) && matches!(next, "a" | "an" | "the" | "this" | "that" | "my" | "your" | "it" | "him" | "her") =>
        {
            Some("write")
        }
        "their" if matches!(next, "is" | "are" | "was" | "were") => Some("there"),
        "their" if GERUNDS.contains(&next) => Some("they're"),
        "their" if matches!(prev, "over" | "out" | "from" | "than") && next.is_empty() => Some("there"),
        "there" if NOUNS.contains(&next) && !LOC_PREV.contains(&prev) => Some("their"),
        "they're" if NOUNS.contains(&next) => Some("their"),
        "male" if SEND.contains(&prev) && !NOUNS.contains(&next) && !matches!(next, "colleague" | "model" | "voice" | "lead") => {
            Some("mail")
        }
        "male"
            if matches!(prev, "the" | "your" | "my" | "this")
                && SEND.contains(&prev2)
                && matches!(next, "today" | "yesterday" | "tomorrow" | "please" | "now" | "out" | "") =>
        {
            Some("mail")
        }
        "weight" if matches!(next, "for" | "until" | "here" | "please") => Some("wait"),
        "weight" if matches!(next, "a" | "an") && matches!(next2, "minute" | "second" | "moment" | "sec" | "bit") => Some("wait"),
        "wait" if next == "of" => Some("weight"),
        "to" if TOO_NEXT.contains(&next) => Some("too"),
        "to" if next == "of" => Some("two"),
        "to" if COUNT_PREV.contains(&prev) && COUNT_NEXT.contains(&next) => Some("two"),
        _ => None,
    }
}

fn cores(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|token| core(token))
        .filter(|word| !word.is_empty())
        .collect()
}

fn core(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}')
        .replace('\u{2019}', "'")
        .to_ascii_lowercase()
}

fn is_protected(word: &str, protected: &[String]) -> bool {
    protected
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(word) || entry.split_whitespace().any(|part| part.eq_ignore_ascii_case(word)))
}

fn stitch(original: &str, replacement: &str) -> String {
    let start = original.find(|c: char| c.is_alphanumeric()).unwrap_or(0);
    let end = original
        .rfind(|c: char| c.is_alphanumeric())
        .map(|index| index + original[index..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(start);
    let mut repl = replacement.to_string();
    if original[start..].chars().next().is_some_and(|c| c.is_uppercase()) {
        let mut chars = repl.chars();
        if let Some(first) = chars.next() {
            repl = first.to_uppercase().to_string() + chars.as_str();
        }
    }
    format!("{}{}{}", &original[..start], repl, &original[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn report_homophones_are_fixed_and_safe_contexts_stay() {
        let cases = [
            ("I roots the email yesterday.", "I wrote the email yesterday."),
            ("their going home", "they're going home"),
            ("over their", "over there"),
            ("there cat sat there", "their cat sat there"),
            ("their is a problem", "there is a problem"),
            ("they're cat", "their cat"),
            ("there is a cat over there", "there is a cat over there"),
            ("their cat", "their cat"),
            ("they're going home", "they're going home"),
            ("write now", "right now"),
            ("the write way", "the right way"),
            ("I right the email", "I write the email"),
            ("right now", "right now"),
            ("write the email", "write the email"),
            ("send the male today", "send the mail today"),
            ("a male colleague", "a male colleague"),
            ("send the mail", "send the mail"),
            ("weight for me", "wait for me"),
            ("the weight of it", "the weight of it"),
            ("the wait of the box", "the weight of the box"),
            ("wait for me", "wait for me"),
            ("to much sugar", "too much sugar"),
            ("to of them", "two of them"),
            ("for to days", "for two days"),
            ("to the store", "to the store"),
            ("too much", "too much"),
            ("two of them", "two of them"),
            ("the roots of the tree", "the roots of the tree"),
            ("he roots for the team", "he roots for the team"),
        ];
        for (input, expected) in cases {
            assert_eq!(local_homophones(input, &bare()), expected, "{input}");
        }
    }

    #[test]
    fn a_protected_spelling_is_not_rewritten() {
        let words = vec!["Roots".to_string()];
        assert_eq!(
            local_homophones("I roots the email yesterday.", &words),
            "I roots the email yesterday."
        );
    }

    #[test]
    fn model_is_skipped_for_the_report_phrases() {
        assert!(!needs_model("I wrote the email yesterday."));
        assert!(!needs_model("the roots of the tree"));
        assert!(!needs_model("Open Bolo and start dictating."));
        assert!(needs_model("maybe roots the whole file"));
        assert!(SENSE_TIMEOUT_MS <= 700);
    }

    #[test]
    fn sense_acceptance_keeps_list_spellings() {
        let words = crate::vocab::protected(&[]);
        assert!(accept_sense("I roots the email yesterday.", "I wrote the email yesterday.", &words));
        assert!(!accept_sense("Open Bolo today", "Open Volo today", &words));
        assert!(!accept_sense("ship it tomorrow", "Ship it tomorrow please", &words));
    }
}
