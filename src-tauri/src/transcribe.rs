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
            .user_agent(concat!("Bolo.com/", env!("CARGO_PKG_VERSION")))
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
        if !prompt.is_empty() {
            prompt.push(' ');
        }
        prompt.push_str("Recent dictation: ");
        prompt.push_str(&snippets.join(" | "));
        prompt.push('.');
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    pub tries: u8,
}

/// A second Whisper call is worth it when the first result is empty or the
/// model itself is unsure. Temperature stays 0; the retry drops the prompt so
/// the request is not identical.
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
) -> anyhow::Result<Transcript> {
    let model = normalize_model(model).context("unknown Whisper model")?;
    let first = request_transcript(client, api_key, audio.clone(), model, language, prompt.as_deref()).await?;
    if !should_retry(&first.text, first.avg_logprob, first.no_speech_prob) {
        return Ok(Transcript {
            text: first.text,
            tries: 1,
        });
    }
    match request_transcript(client, api_key, audio, model, language, None).await {
        Ok(second) => Ok(Transcript {
            text: prefer_attempt(first, second),
            tries: 2,
        }),
        Err(_) if !first.text.trim().is_empty() => Ok(Transcript {
            text: first.text,
            tries: 2,
        }),
        Err(err) => Err(err),
    }
}

fn prefer_attempt(first: Attempt, second: Attempt) -> String {
    let primary = first.text.trim();
    let retry = second.text.trim();
    if primary.is_empty() {
        return retry.to_string();
    }
    if retry.is_empty() {
        return primary.to_string();
    }
    let first_bad = should_retry(primary, first.avg_logprob, first.no_speech_prob);
    let second_bad = should_retry(retry, second.avg_logprob, second.no_speech_prob);
    if first_bad && !second_bad {
        return retry.to_string();
    }
    if !first_bad && second_bad {
        return primary.to_string();
    }
    if retry.chars().count() >= primary.chars().count() {
        retry.to_string()
    } else {
        primary.to_string()
    }
}

struct Attempt {
    text: String,
    avg_logprob: Option<f32>,
    no_speech_prob: Option<f32>,
}

async fn request_transcript(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    model: &str,
    language: &str,
    prompt: Option<&str>,
) -> anyhow::Result<Attempt> {
    let mut attempt = 0u32;
    loop {
        match request_transcript_once(client, api_key, audio.clone(), model, language, prompt).await {
            Ok(parsed) => return Ok(parsed),
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

async fn request_transcript_once(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    model: &str,
    language: &str,
    prompt: Option<&str>,
) -> Result<Attempt, GroqFailure> {
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
    if let Some(code) = normalize_language(language) {
        form = form.text("language", code);
    }
    if let Some(prompt) = prompt {
        form = form.text("prompt", prompt.to_string());
    }
    let response = match client
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
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
            message: groq_error_message(code, &body),
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
                text: text.trim().to_string(),
                avg_logprob,
                no_speech_prob,
            });
        }
    }
    let parsed: SimpleTranscript =
        serde_json::from_str(body).context("transcription response was not JSON")?;
    Ok(Attempt {
        text: parsed.text.trim().to_string(),
        avg_logprob: None,
        no_speech_prob: None,
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
    segments: Option<Vec<VerboseSegment>>,
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
    fn prompt_includes_terms_and_truncates() {
        assert_eq!(whisper_prompt(&[]), None);
        assert_eq!(
            whisper_prompt(&["  Cadence  ".into(), "".into(), "Tauri".into()]).as_deref(),
            Some("Dictation transcript. Spell these terms exactly when spoken: Cadence, Tauri.")
        );
        let huge = vec!["Supercalifragilistic".to_string(); 80];
        let prompt = whisper_prompt(&huge).unwrap();
        assert!(prompt.chars().count() <= 800);
        assert!(prompt.starts_with("Dictation transcript."));
        assert!(prompt.contains("Supercalifragilistic"));
        let with_recent = whisper_prompt_with(
            &["Bolo.com".into()],
            &["Ship the notes to Priya tomorrow morning before the review.".into()],
        )
        .unwrap();
        assert!(with_recent.contains("Bolo.com"));
        assert!(with_recent.contains("Recent dictation"));
        assert!(with_recent.contains("Priya"));
        assert!(with_recent.chars().count() <= 800);
        assert_eq!(whisper_prompt_with(&[], &["  ".into()]), None);
    }

    #[test]
    fn retries_empty_or_low_confidence_results() {
        assert!(should_retry("", None, None));
        assert!(should_retry("   ", Some(-0.2), Some(0.1)));
        assert!(should_retry("hello there", Some(-1.4), Some(0.2)));
        assert!(should_retry("hi", Some(-0.2), Some(0.9)));
        assert!(!should_retry("ship it tomorrow", Some(-0.3), Some(0.05)));
        assert!(!should_retry("a longer confident sentence here", None, Some(0.8)));
        let body = r#"{"text":" Ship it. ","segments":[{"avg_logprob":-0.2,"no_speech_prob":0.01},{"avg_logprob":-0.4,"no_speech_prob":0.2}]}"#;
        let attempt = parse_attempt(body).unwrap();
        assert_eq!(attempt.text, "Ship it.");
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
}
