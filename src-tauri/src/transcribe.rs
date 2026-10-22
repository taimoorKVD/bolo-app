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
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("Cadence/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client")
    })
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
    let mut prompt = format!("Terms: {}", terms.join(", "));
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
            Some("Terms: Cadence, Tauri")
        );
        let huge = vec!["Supercalifragilistic".to_string(); 80];
        let prompt = whisper_prompt(&huge).unwrap();
        assert!(prompt.chars().count() <= 800);
        assert!(prompt.starts_with("Terms: "));
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
