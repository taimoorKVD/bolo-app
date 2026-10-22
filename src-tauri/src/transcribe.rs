//! Groq Whisper transcription. Audio is sent only to api.groq.com.

use crate::cleanup::groq_error_message;
use anyhow::{anyhow, Context};
use serde::Deserialize;
use std::sync::OnceLock;
use std::time::Duration;

pub const WHISPER_TURBO: &str = "whisper-large-v3-turbo";
pub const WHISPER_LARGE: &str = "whisper-large-v3";

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
            .user_agent(concat!("Cadence/", env!("CARGO_PKG_VERSION")))
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

pub fn whisper_prompt(words: &[String]) -> Option<String> {
    let terms: Vec<&str> = words
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty())
        .collect();
    if terms.is_empty() {
        return None;
    }
    let mut prompt = format!(
        "Dictation transcript. Spell these terms exactly when spoken: {}.",
        terms.join(", ")
    );
    // Whisper prompts are capped at 224 tokens. Stay well under that in characters.
    if prompt.chars().count() > 800 {
        let truncated: String = prompt.chars().take(800).collect();
        prompt = match truncated.rfind(',') {
            Some(index) if index > 20 => truncated[..index].to_string(),
            _ => truncated,
        };
    }
    Some(prompt)
}

pub async fn transcribe(
    client: &reqwest::Client,
    api_key: &str,
    wav: Vec<u8>,
    model: &str,
    language: &str,
    prompt: Option<String>,
) -> anyhow::Result<String> {
    let model = normalize_model(model).context("unknown Whisper model")?;
    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("dictation.wav")
        .mime_str("audio/wav")
        .context("couldn't build the audio upload")?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model)
        .text("response_format", "json")
        .text("temperature", "0");
    if let Some(code) = normalize_language(language) {
        form = form.text("language", code);
    }
    if let Some(prompt) = prompt {
        form = form.text("prompt", prompt);
    }

    let response = client
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .context("couldn't reach Groq")?;
    let status = response.status();
    let body = response.text().await.context("empty transcription response")?;
    if !status.is_success() {
        return Err(anyhow!(groq_error_message(status.as_u16(), &body)));
    }
    let parsed: Transcript =
        serde_json::from_str(&body).context("transcription response was not JSON")?;
    Ok(parsed.text.trim().to_string())
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
struct Transcript {
    text: String,
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
