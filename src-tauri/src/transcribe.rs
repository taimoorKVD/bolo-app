//! Groq Whisper transcription. Audio is sent only to api.groq.com.

use crate::cleanup::groq_error_message;
use anyhow::Context;
use serde::Deserialize;
use std::sync::OnceLock;
use std::time::Duration;

pub const WHISPER_TURBO: &str = "whisper-large-v3-turbo";
pub const WHISPER_LARGE: &str = "whisper-large-v3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// HTTP 429. One backoff, then the on-device model may take the chunk.
    RateLimit,
    /// Network failure or HTTP 5xx. A few backoffs, then the on-device model.
    Unavailable,
    /// The key was rejected. Do not hide that behind the on-device model.
    Rejected,
    Other,
}

#[derive(Debug)]
pub struct GroqFailure {
    pub message: String,
    pub kind: FailKind,
}

impl std::fmt::Display for GroqFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for GroqFailure {}

pub fn failure_kind(err: &anyhow::Error) -> Option<FailKind> {
    err.downcast_ref::<GroqFailure>().map(|failure| failure.kind)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPlan {
    /// Wait, then try the same request again.
    Backoff { ms: u64 },
    /// Stop retrying. `fallback` means whisper.cpp should take this chunk.
    Stop { fallback: bool },
}

/// `attempt` is how many failures have already been seen, starting at 0.
pub fn retry_plan(kind: FailKind, attempt: u32) -> RetryPlan {
    match kind {
        FailKind::Rejected | FailKind::Other => RetryPlan::Stop { fallback: false },
        FailKind::RateLimit => {
            if attempt == 0 {
                RetryPlan::Backoff { ms: 700 }
            } else {
                RetryPlan::Stop { fallback: true }
            }
        }
        FailKind::Unavailable => {
            if attempt < 2 {
                RetryPlan::Backoff { ms: 250 * 3u64.pow(attempt) }
            } else {
                RetryPlan::Stop { fallback: true }
            }
        }
    }
}

pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(4)
            .tcp_keepalive(Duration::from_secs(30))
            .http2_keep_alive_interval(Duration::from_secs(20))
            .http2_keep_alive_timeout(Duration::from_secs(10))
            .http2_keep_alive_while_idle(true)
            .user_agent(concat!("Bolo/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client")
    })
}

/// Open a connection to Groq before the first take so the upload does not pay
/// for TCP and TLS. The response body is irrelevant and the key is not sent.
pub async fn prewarm(log_dir: &std::path::Path) {
    let started = std::time::Instant::now();
    let result = http_client().get("https://api.groq.com/").send().await;
    let ms = started.elapsed().as_millis();
    match result {
        Ok(response) => crate::logbuf::log_line(
            log_dir,
            "info",
            &format!("groq prewarm ms={ms} status={}", response.status().as_u16()),
        ),
        Err(err) => crate::logbuf::log_line(
            log_dir,
            "info",
            &format!("groq prewarm ms={ms} error={err}"),
        ),
    }
}

pub fn normalize_model(model: &str) -> Option<&'static str> {
    match model {
        WHISPER_TURBO => Some(WHISPER_TURBO),
        WHISPER_LARGE => Some(WHISPER_LARGE),
        _ => None,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn whisper_prompt(words: &[String]) -> Option<String> {
    whisper_prompt_with(words, &[])
}

/// Dictionary terms plus the last couple of local transcripts, capped at 800 characters.
pub fn whisper_prompt_with(words: &[String], recent: &[String]) -> Option<String> {
    let terms: Vec<&str> = words
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty())
        .collect();
    let snippets: Vec<String> = recent
        .iter()
        .take(2)
        .map(|text| snippet(text, 140))
        .filter(|text| !text.is_empty())
        .collect();
    if terms.is_empty() && snippets.is_empty() {
        return None;
    }
    let mut prompt = String::new();
    if !terms.is_empty() {
        prompt.push_str("Dictation transcript. Spell these terms exactly when spoken: ");
        prompt.push_str(&terms.join(", "));
        prompt.push('.');
    }
    if !snippets.is_empty() {
        // Prior text as a style example only — no meta label. Whisper often
        // echoes labels like "Recent dictation:" into the pasted transcript.
        if !prompt.is_empty() {
            prompt.push(' ');
        }
        prompt.push_str(&snippets.join(" "));
    }
    // Whisper prompts are capped at 224 tokens. Stay well under that in characters.
    if prompt.chars().count() > 800 {
        let truncated: String = prompt.chars().take(800).collect();
        prompt = match truncated.rfind(['.', ',', '|']) {
            Some(index) if index > 40 => truncated[..index].to_string(),
            _ => truncated,
        };
    }
    Some(prompt)
}

fn snippet(text: &str, max_chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        flat
    } else {
        let cut: String = flat.chars().take(max_chars).collect();
        format!("{cut}…")
    }
}

/// Drop Whisper prompt echoes that sometimes prepend the pasted transcript.
fn strip_prompt_leak(text: &str) -> String {
    let mut out = text.trim().to_string();
    loop {
        let before = out.clone();
        let trimmed = out.trim_start();
        let next = if let Some(rest) = trimmed.strip_prefix("Recent dictation:") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("Dictation transcript.") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("Dictation transcript:") {
            rest
        } else {
            break;
        };
        out = next
            .trim_start()
            .trim_matches(|c| c == '"' || c == '\'' || c == '“' || c == '”')
            .trim()
            .to_string();
        if out == before {
            break;
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    pub tries: u8,
    /// ISO-ish code from verbose_json (`en`, `ur`), or empty when the body omitted it.
    pub language: String,
}

/// A second Whisper call is worth it when the first result is empty or the
/// model itself is unsure. Temperature stays 0; the retry drops the prompt so
/// the request is not identical.
/// "Please try again in 3.2s" / "in 850ms" from a Groq 429 message.
pub fn retry_after_ms(message: &str) -> Option<u64> {
    let at = message.find("try again in ")? + "try again in ".len();
    let rest = &message[at..];
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let value: f64 = num.parse().ok()?;
    let unit = &rest[num.len()..];
    if unit.starts_with("ms") {
        Some(value as u64)
    } else if unit.starts_with('s') {
        Some((value * 1000.0) as u64)
    } else if unit.starts_with('m') {
        Some((value * 60_000.0) as u64)
    } else {
        None
    }
}

/// Phrases Whisper invents on silence or noise.
pub fn stock_phrase(text: &str) -> bool {
    let t: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    matches!(
        t.as_str(),
        "thank you" | "thanks" | "thank you very much" | "thanks for watching" | "thank you for watching"
            | "you" | "bye" | "bye bye" | "please subscribe" | "subtitles by the amaraorg community" | "شکریہ"
    )
}

pub fn should_retry(text: &str, avg_logprob: Option<f32>, no_speech_prob: Option<f32>) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    let words = trimmed.split_whitespace().count();
    if avg_logprob.is_some_and(|value| value < -1.0) {
        return true;
    }
    no_speech_prob.is_some_and(|value| value > 0.7) && words < 3
}

pub async fn transcribe(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    model: &str,
    language: &str,
    prompt: Option<String>,
    using_builtin: bool,
) -> anyhow::Result<Transcript> {
    let model = normalize_model(model).context("unknown Whisper model")?;
    let first = request_audio(client, api_key, audio.clone(), TRANSCRIPTIONS, model, language, prompt.as_deref(), using_builtin).await?;
    let mut first = first;
    if stock_phrase(&first.text) && first.no_speech_prob.unwrap_or(0.0) > 0.25 {
        // "Thank you." / "Thanks for watching." on a take that is mostly noise.
        first.text.clear();
    }
    if !should_retry(&first.text, first.avg_logprob, first.no_speech_prob) {
        return Ok(Transcript {
            text: first.text,
            tries: 1,
            language: first.language,
        });
    }
    match request_audio(client, api_key, audio, TRANSCRIPTIONS, model, language, None, using_builtin).await {
        Ok(second) => {
            let chosen = prefer_attempt(first, second);
            Ok(Transcript {
                text: chosen.text,
                tries: 2,
                language: chosen.language,
            })
        }
        Err(_) if !first.text.trim().is_empty() => Ok(Transcript {
            text: first.text,
            tries: 2,
            language: first.language,
        }),
        Err(err) => Err(err),
    }
}

/// Groq translates with `whisper-large-v3` only. Turbo does not translate.
pub async fn translate_audio(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    prompt: Option<&str>,
    using_builtin: bool,
) -> anyhow::Result<Transcript> {
    let first = request_audio(client, api_key, audio.clone(), TRANSLATIONS, WHISPER_LARGE, "", prompt, using_builtin).await?;
    if !first.text.trim().is_empty() {
        return Ok(Transcript {
            text: first.text,
            tries: 1,
            language: first.language,
        });
    }
    let second = request_audio(client, api_key, audio, TRANSLATIONS, WHISPER_LARGE, "", None, using_builtin).await?;
    let language = if second.language.is_empty() {
        first.language
    } else {
        second.language
    };
    Ok(Transcript {
        text: second.text,
        tries: 2,
        language,
    })
}

const TRANSCRIPTIONS: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const TRANSLATIONS: &str = "https://api.groq.com/openai/v1/audio/translations";

fn prefer_attempt(first: Attempt, second: Attempt) -> Attempt {
    let primary = first.text.trim();
    let retry = second.text.trim();
    let pick_second = if primary.is_empty() {
        true
    } else if retry.is_empty() {
        false
    } else {
        let first_bad = should_retry(primary, first.avg_logprob, first.no_speech_prob);
        let second_bad = should_retry(retry, second.avg_logprob, second.no_speech_prob);
        if first_bad && !second_bad {
            true
        } else if !first_bad && second_bad {
            false
        } else {
            retry.chars().count() >= primary.chars().count()
        }
    };
    let fallback_language = if first.language.is_empty() {
        second.language.clone()
    } else {
        first.language.clone()
    };
    let mut chosen = if pick_second { second } else { first };
    if chosen.language.is_empty() {
        chosen.language = fallback_language;
    }
    chosen.text = strip_prompt_leak(&chosen.text);
    chosen
}

struct Attempt {
    text: String,
    avg_logprob: Option<f32>,
    no_speech_prob: Option<f32>,
    language: String,
}

async fn request_audio(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    endpoint: &str,
    model: &str,
    language: &str,
    prompt: Option<&str>,
    using_builtin: bool,
) -> anyhow::Result<Attempt> {
    let mut attempt = 0u32;
    loop {
        match request_audio_once(client, api_key, audio.clone(), endpoint, model, language, prompt, using_builtin).await {
            Ok(parsed) => return Ok(parsed),
            Err(failure) if failure.kind == FailKind::RateLimit && attempt < 3 => {
                // Free Groq keys allow ~20 requests a minute. Groq says how long
                // to wait; a few seconds is better than dropping the take.
                match retry_after_ms(&failure.message) {
                    Some(ms) if ms <= 8_000 => {
                        std::thread::sleep(Duration::from_millis(ms + 150));
                        attempt += 1;
                    }
                    _ => match retry_plan(failure.kind, attempt.min(1)) {
                        RetryPlan::Backoff { ms } => {
                            std::thread::sleep(Duration::from_millis(ms));
                            attempt += 1;
                        }
                        RetryPlan::Stop { .. } => return Err(anyhow::Error::from(failure)),
                    },
                }
            }
            Err(failure) => match retry_plan(failure.kind, attempt) {
                RetryPlan::Backoff { ms } => {
                    std::thread::sleep(Duration::from_millis(ms));
                    attempt += 1;
                }
                RetryPlan::Stop { .. } => return Err(anyhow::Error::from(failure)),
            },
        }
    }
}

async fn request_audio_once(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    endpoint: &str,
    model: &str,
    language: &str,
    prompt: Option<&str>,
    using_builtin: bool,
) -> Result<Attempt, GroqFailure> {
    let translating = endpoint.ends_with("/translations");
    let model = if translating { WHISPER_LARGE } else { model };
    let part = match audio_part(audio) {
        Ok(part) => part,
        Err(err) => {
            return Err(GroqFailure {
                message: err.to_string(),
                kind: FailKind::Other,
            })
        }
    };
    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string())
        .text("response_format", "verbose_json")
        .text("temperature", "0");
    if !translating {
        if let Some(code) = normalize_language(language) {
            form = form.text("language", code);
        }
    }
    if let Some(prompt) = prompt {
        form = form.text("prompt", prompt.to_string());
    }
    let response = match client
        .post(endpoint)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            return Err(GroqFailure {
                message: format!("couldn't reach Groq: {err}"),
                kind: FailKind::Unavailable,
            })
        }
    };
    let status = response.status();
    let body = match response.text().await {
        Ok(body) => body,
        Err(err) => {
            return Err(GroqFailure {
                message: format!("empty transcription response: {err}"),
                kind: if status.is_server_error() {
                    FailKind::Unavailable
                } else {
                    FailKind::Other
                },
            })
        }
    };
    if !status.is_success() {
        let code = status.as_u16();
        let kind = match code {
            401 | 403 => FailKind::Rejected,
            429 => FailKind::RateLimit,
            500..=599 => FailKind::Unavailable,
            _ => FailKind::Other,
        };
        return Err(GroqFailure {
            message: groq_error_message(code, &body, using_builtin),
            kind,
        });
    }
    parse_attempt(&body).map_err(|err| GroqFailure {
        message: err.to_string(),
        kind: FailKind::Other,
    })
}

fn audio_part(bytes: Vec<u8>) -> anyhow::Result<reqwest::multipart::Part> {
    let flac = bytes.starts_with(b"fLaC");
    let (name, mime) = if flac {
        ("dictation.flac", "audio/flac")
    } else {
        ("dictation.wav", "audio/wav")
    };
    reqwest::multipart::Part::bytes(bytes)
        .file_name(name)
        .mime_str(mime)
        .context("couldn't build the audio upload")
}

fn parse_attempt(body: &str) -> anyhow::Result<Attempt> {
    if let Ok(verbose) = serde_json::from_str::<VerboseBody>(body) {
        if let Some(text) = verbose.text {
            let (avg_logprob, no_speech_prob) = verbose
                .segments
                .as_deref()
                .map(confidence)
                .unwrap_or((None, None));
            return Ok(Attempt {
                text: strip_prompt_leak(&text),
                avg_logprob,
                no_speech_prob,
                language: verbose.language.map(|value| language_code(&value)).unwrap_or_default(),
            });
        }
    }
    let parsed: SimpleTranscript =
        serde_json::from_str(body).context("transcription response was not JSON")?;
    Ok(Attempt {
        text: strip_prompt_leak(&parsed.text),
        avg_logprob: None,
        no_speech_prob: None,
        language: String::new(),
    })
}

fn confidence(segments: &[VerboseSegment]) -> (Option<f32>, Option<f32>) {
    let logs: Vec<f32> = segments.iter().filter_map(|segment| segment.avg_logprob).collect();
    let noises: Vec<f32> = segments.iter().filter_map(|segment| segment.no_speech_prob).collect();
    let avg = if logs.is_empty() {
        None
    } else {
        Some(logs.iter().sum::<f32>() / logs.len() as f32)
    };
    let noise = noises.into_iter().reduce(f32::max);
    (avg, noise)
}

pub fn normalize_language(language: &str) -> Option<String> {
    let language = language.trim().to_ascii_lowercase();
    if language.is_empty() || language == "auto" {
        return None;
    }
    if (2..=3).contains(&language.len()) && language.chars().all(|c| c.is_ascii_lowercase()) {
        Some(language)
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
struct SimpleTranscript {
    text: String,
}

#[derive(Debug, Deserialize)]
struct VerboseBody {
    text: Option<String>,
    language: Option<String>,
    segments: Option<Vec<VerboseSegment>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechRoute {
    /// Keep the transcript in the language that was spoken.
    Transcribe,
    /// `/audio/translations` with whisper-large-v3.
    Translate,
    /// Transcribe, then Llama into English without adding or dropping meaning.
    Mixed,
}

pub fn normalize_output(value: &str) -> &'static str {
    if value.trim().eq_ignore_ascii_case("same") {
        "same"
    } else {
        "english"
    }
}

/// A fixed non-English language other than Urdu can go straight to translation.
pub fn direct_translate(output: &str, setting: &str) -> bool {
    if normalize_output(output) != "english" {
        return false;
    }
    match normalize_language(setting).as_deref() {
        Some(code) if code != "en" && code != "ur" => true,
        _ => false,
    }
}

pub fn speech_route(output: &str, setting: &str, detected: &str, text: &str) -> SpeechRoute {
    if normalize_output(output) != "english" {
        return SpeechRoute::Transcribe;
    }
    let setting_code = normalize_language(setting).unwrap_or_default();
    if setting_code == "en" {
        return SpeechRoute::Transcribe;
    }
    let detected = language_code(detected);
    let urdu = setting_code == "ur" || detected == "ur" || mixed_urdu(text);
    if !urdu {
        if detected.is_empty() || detected == "en" {
            return SpeechRoute::Transcribe;
        }
        return SpeechRoute::Translate;
    }
    if mixed_urdu(text) || (has_latin(text) && (setting_code == "ur" || detected == "ur")) {
        return SpeechRoute::Mixed;
    }
    if setting_code == "ur" || detected == "ur" {
        return SpeechRoute::Translate;
    }
    SpeechRoute::Transcribe
}

pub fn language_code(raw: &str) -> String {
    let name = raw.trim().to_ascii_lowercase();
    match name.as_str() {
        "" => String::new(),
        "english" | "en" => "en".into(),
        "urdu" | "ur" => "ur".into(),
        "hindi" | "hi" => "hi".into(),
        "spanish" | "es" => "es".into(),
        "french" | "fr" => "fr".into(),
        "german" | "de" => "de".into(),
        "portuguese" | "pt" => "pt".into(),
        "italian" | "it" => "it".into(),
        "dutch" | "nl" => "nl".into(),
        "polish" | "pl" => "pl".into(),
        "russian" | "ru" => "ru".into(),
        "ukrainian" | "uk" => "uk".into(),
        "arabic" | "ar" => "ar".into(),
        "hebrew" | "he" => "he".into(),
        "japanese" | "ja" => "ja".into(),
        "korean" | "ko" => "ko".into(),
        "chinese" | "zh" => "zh".into(),
        "turkish" | "tr" => "tr".into(),
        "persian" | "farsi" | "fa" => "fa".into(),
        "punjabi" | "pa" => "pa".into(),
        other => {
            if (2..=3).contains(&other.len()) && other.chars().all(|c| c.is_ascii_lowercase()) {
                other.to_string()
            } else {
                other.to_string()
            }
        }
    }
}

fn mixed_urdu(text: &str) -> bool {
    has_arabic(text) && has_latin(text)
}

fn has_latin(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_alphabetic())
}

fn has_arabic(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(
            c,
            '\u{0600}'..='\u{06FF}'
                | '\u{0750}'..='\u{077F}'
                | '\u{08A0}'..='\u{08FF}'
                | '\u{FB50}'..='\u{FDFF}'
                | '\u{FE70}'..='\u{FEFF}'
        )
    })
}

#[derive(Debug, Deserialize)]
struct VerboseSegment {
    avg_logprob: Option<f32>,
    no_speech_prob: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_is_parsed() {
        assert_eq!(retry_after_ms("Please try again in 3s. Need more"), Some(3000));
        assert_eq!(retry_after_ms("try again in 1.5s"), Some(1500));
        assert_eq!(retry_after_ms("try again in 640ms"), Some(640));
        assert_eq!(retry_after_ms("nope"), None);
    }

    #[test]
    fn stock_phrases_are_recognized() {
        assert!(stock_phrase(" Thank you. "));
        assert!(stock_phrase("Thanks for watching!"));
        assert!(!stock_phrase("Thank you for the report."));
    }

    #[test]
    fn prompt_includes_terms_and_truncates() {
        assert_eq!(whisper_prompt(&[]), None);
        assert_eq!(
            whisper_prompt(&["  Bolo  ".into(), "".into(), "Tauri".into()]).as_deref(),
            Some("Dictation transcript. Spell these terms exactly when spoken: Bolo, Tauri.")
        );
        let huge = vec!["Supercalifragilistic".to_string(); 80];
        let prompt = whisper_prompt(&huge).unwrap();
        assert!(prompt.chars().count() <= 800);
        assert!(prompt.starts_with("Dictation transcript."));
        assert!(prompt.contains("Supercalifragilistic"));
        let with_recent = whisper_prompt_with(
            &["Bolo".into()],
            &["Ship the notes to Priya tomorrow morning before the review.".into()],
        )
        .unwrap();
        assert!(with_recent.contains("Bolo"));
        assert!(!with_recent.contains("Recent dictation"));
        assert!(with_recent.contains("Priya"));
        assert!(with_recent.chars().count() <= 800);
        assert_eq!(whisper_prompt_with(&[], &["  ".into()]), None);
        assert_eq!(
            strip_prompt_leak("Recent dictation:\"Hey, can you listen to me?\""),
            "Hey, can you listen to me?"
        );
        assert_eq!(strip_prompt_leak("Ship it tomorrow."), "Ship it tomorrow.");
    }

    #[test]
    fn retries_empty_or_low_confidence_results() {
        assert!(should_retry("", None, None));
        assert!(should_retry("   ", Some(-0.2), Some(0.1)));
        assert!(should_retry("hello there", Some(-1.4), Some(0.2)));
        assert!(should_retry("hi", Some(-0.2), Some(0.9)));
        assert!(!should_retry("ship it tomorrow", Some(-0.3), Some(0.05)));
        assert!(!should_retry("a longer confident sentence here", None, Some(0.8)));
        let body = r#"{"text":" Ship it. ","language":"english","segments":[{"avg_logprob":-0.2,"no_speech_prob":0.01},{"avg_logprob":-0.4,"no_speech_prob":0.2}]}"#;
        let attempt = parse_attempt(body).unwrap();
        assert_eq!(attempt.text, "Ship it.");
        assert_eq!(attempt.language, "en");
        assert!(attempt.avg_logprob.unwrap() < -0.2);
        assert!((attempt.no_speech_prob.unwrap() - 0.2).abs() < 0.001);
        let plain = parse_attempt(r#"{"text":"hello"}"#).unwrap();
        assert_eq!(plain.text, "hello");
        assert_eq!(retry_plan(FailKind::RateLimit, 0), RetryPlan::Backoff { ms: 700 });
        assert_eq!(retry_plan(FailKind::RateLimit, 1), RetryPlan::Stop { fallback: true });
        assert_eq!(retry_plan(FailKind::Unavailable, 0), RetryPlan::Backoff { ms: 250 });
        assert_eq!(retry_plan(FailKind::Unavailable, 1), RetryPlan::Backoff { ms: 750 });
        assert_eq!(retry_plan(FailKind::Unavailable, 2), RetryPlan::Stop { fallback: true });
        assert_eq!(retry_plan(FailKind::Rejected, 0), RetryPlan::Stop { fallback: false });
        let failure = GroqFailure {
            message: "Groq is rate limiting requests right now".into(),
            kind: FailKind::RateLimit,
        };
        let wrapped = anyhow::Error::from(failure);
        assert_eq!(failure_kind(&wrapped), Some(FailKind::RateLimit));
        assert!(plain.avg_logprob.is_none());
    }

    #[test]
    fn language_auto_is_omitted() {
        assert_eq!(normalize_language("auto"), None);
        assert_eq!(normalize_language(" EN "), Some("en".into()));
        assert_eq!(normalize_language("english"), None);
        assert_eq!(normalize_model("whisper-large-v3"), Some(WHISPER_LARGE));
        assert_eq!(normalize_model("gpt-4o"), None);
    }

    #[test]
    fn english_output_routes_urdu_and_other_languages() {
        assert_eq!(normalize_output("same"), "same");
        assert_eq!(normalize_output("English"), "english");
        assert!(!direct_translate("same", "es"));
        assert!(direct_translate("english", "es"));
        assert!(!direct_translate("english", "ur"));
        assert!(!direct_translate("english", "auto"));
        assert_eq!(speech_route("same", "ur", "ur", "سلام"), SpeechRoute::Transcribe);
        assert_eq!(speech_route("english", "en", "en", "hello"), SpeechRoute::Transcribe);
        assert_eq!(speech_route("english", "auto", "english", "hello"), SpeechRoute::Transcribe);
        assert_eq!(speech_route("english", "auto", "french", "bonjour"), SpeechRoute::Translate);
        assert_eq!(speech_route("english", "auto", "urdu", "یہ اردو ہے"), SpeechRoute::Translate);
        assert_eq!(
            speech_route("english", "auto", "urdu", "meeting کل ہے"),
            SpeechRoute::Mixed
        );
        assert_eq!(
            speech_route("english", "ur", "", "kal meeting hai at 5"),
            SpeechRoute::Mixed
        );
        assert_eq!(language_code("Urdu"), "ur");
        assert_eq!(language_code("english"), "en");
    }
}
