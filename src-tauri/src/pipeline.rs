//! Transcribe cut chunks while the take is still recording.
//!
//! Two chunks run at a time. The rest wait, so a long take does not stampede Groq.
//! Each chunk tries Groq first and falls back to whisper.cpp on rate limit,
//! server error, or a network failure. The following take tries Groq again.

use crate::audio::{self, FinishedAudio, SpeechError};
use crate::credentials::SecretKey;
use crate::offline;
use crate::spool;
use crate::store::Settings;
use crate::cleanup;
use crate::logbuf::log_line;
use crate::transcribe::{self, FailKind, SpeechRoute};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub struct Product {
    pub text: String,
    pub engine: String,
    pub tries: u8,
    pub encode_ms: u64,
    pub remote_ms: u64,
    pub bytes: usize,
    pub format: String,
    pub error: Option<String>,
    /// Empty because the chunk was silence, not because transcription failed.
    pub silence: bool,
    pub language: String,
    pub speech_path: String,
}

pub struct Flight {
    pub index: usize,
    pub rx: Receiver<Product>,
}

pub struct Slots {
    left: Mutex<usize>,
    cv: Condvar,
}

impl Slots {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            left: Mutex::new(capacity.max(1)),
            cv: Condvar::new(),
        })
    }

    fn acquire(&self) {
        let mut left = self.left.lock().unwrap_or_else(|err| err.into_inner());
        while *left == 0 {
            left = self.cv.wait(left).unwrap_or_else(|err| err.into_inner());
        }
        *left -= 1;
    }

    fn release(&self) {
        let mut left = self.left.lock().unwrap_or_else(|err| err.into_inner());
        *left += 1;
        self.cv.notify_one();
    }
}

pub struct Jobs {
    pub next: usize,
    pub flights: Vec<Flight>,
}

impl Jobs {
    pub fn new() -> Self {
        Self {
            next: 0,
            flights: Vec::new(),
        }
    }

    pub fn take(&mut self) -> Vec<Flight> {
        std::mem::take(&mut self.flights)
    }
}

#[derive(Clone)]
pub struct JobEnv {
    pub settings: Settings,
    pub recent: Vec<String>,
    pub config_dir: std::path::PathBuf,
    pub spool_id: String,
}

pub fn launch(slots: &Arc<Slots>, jobs: &Mutex<Jobs>, audios: Vec<FinishedAudio>, env: JobEnv) {
    if audios.is_empty() {
        return;
    }
    let mut guard = jobs.lock().unwrap_or_else(|err| err.into_inner());
    for audio in audios {
        let index = guard.next;
        guard.next += 1;
        let (tx, rx) = mpsc::channel();
        guard.flights.push(Flight { index, rx });
        let slots = Arc::clone(slots);
        let env = env.clone();
        let _ = std::thread::Builder::new()
            .name(format!("bolo-chunk-{index}"))
            .spawn(move || {
                slots.acquire();
                let product = transcribe_one(index, audio, &env);
                slots.release();
                let _ = tx.send(product);
            });
    }
}

pub fn collect(mut flights: Vec<Flight>) -> Vec<Product> {
    flights.sort_by_key(|flight| flight.index);
    flights
        .into_iter()
        .map(|flight| match flight.rx.recv_timeout(Duration::from_secs(90)) {
            Ok(product) => product,
            Err(_) => Product {
                text: String::new(),
                engine: String::new(),
                tries: 0,
                encode_ms: 0,
                remote_ms: 0,
                bytes: 0,
                format: String::new(),
                error: Some("A piece of this take didn't finish transcribing.".into()),
                silence: false,
                language: String::new(),
                speech_path: String::new(),
            },
        })
        .collect()
}

pub fn stitch_products(products: &[Product]) -> Result<String, String> {
    let mut failure = None;
    let mut parts = Vec::new();
    for product in products {
        if let Some(error) = &product.error {
            if failure.is_none() {
                failure = Some(error.clone());
            }
            continue;
        }
        if !product.text.trim().is_empty() {
            parts.push(product.text.as_str());
        }
    }
    if parts.is_empty() {
        return match failure {
            Some(error) => Err(error),
            None => Ok(String::new()),
        };
    }
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(crate::chunk::stitch(parts))
}

pub fn engine_label(products: &[Product]) -> Option<String> {
    let mut groq = false;
    let mut local = false;
    for product in products {
        match product.engine.as_str() {
            "Groq" => groq = true,
            "On this PC" => local = true,
            _ => {}
        }
    }
    match (groq, local) {
        (true, true) => Some("Groq + on this PC".into()),
        (true, false) => Some("Groq".into()),
        (false, true) => Some("On this PC".into()),
        _ => None,
    }
}

fn transcribe_one(index: usize, audio: FinishedAudio, env: &JobEnv) -> Product {
    let mut product = Product {
        text: String::new(),
        engine: String::new(),
        tries: 0,
        encode_ms: 0,
        remote_ms: 0,
        bytes: 0,
        format: String::new(),
        error: None,
        silence: false,
        language: String::new(),
        speech_path: String::new(),
    };
    let encode_started = Instant::now();
    let encoded = match audio::prepare_speech(&audio) {
        Ok(encoded) => encoded,
        Err(SpeechError::Silent | SpeechError::Empty) => {
            product.silence = true;
            product.encode_ms = encode_started.elapsed().as_millis() as u64;
            return product;
        }
        Err(err) => {
            product.encode_ms = encode_started.elapsed().as_millis() as u64;
            product.error = Some(err.to_string());
            return product;
        }
    };
    product.encode_ms = encode_started.elapsed().as_millis() as u64;
    product.bytes = encoded.bytes.len();
    product.format = encoded.format.to_string();
    spool::write_audio(&env.config_dir, &env.spool_id, index, &encoded.bytes);
    let remote_started = Instant::now();
    match tauri::async_runtime::block_on(recognize(encoded.bytes, env)) {
        Ok(recognized) => {
            product.text = recognized.text;
            product.tries = recognized.tries;
            product.engine = recognized.engine;
            product.language = recognized.language;
            product.speech_path = recognized.path;
        }
        Err(err) => product.error = Some(err),
    }
    product.remote_ms = remote_started.elapsed().as_millis() as u64;
    product
}

struct Recognized {
    text: String,
    tries: u8,
    engine: String,
    language: String,
    path: String,
}

async fn recognize(bytes: Vec<u8>, env: &JobEnv) -> Result<Recognized, String> {
    let offline_model = env.settings.offline_model.clone();
    let prompt = transcribe::whisper_prompt_with(&env.settings.dictionary, &env.recent);
    let model = env.settings.model.clone();
    let language = env.settings.language.clone();
    let output = env.settings.output.clone();
    let cleanup_model = env.settings.cleanup_model.clone();
    match SecretKey::load() {
        Ok(key) => {
            let using_builtin = key.is_builtin();
            let groq = speech_with_groq(
                transcribe::http_client(),
                key.expose(),
                bytes.clone(),
                &model,
                &language,
                &output,
                prompt.as_deref(),
                &cleanup_model,
                using_builtin,
            )
            .await;
            drop(key);
            match groq {
                Ok(recognized) => {
                    log_speech(env, &recognized.language, &recognized.path);
                    Ok(recognized)
                }
                Err(err) if should_fallback(&err) && offline::normalize_model(&offline_model) != "off" => {
                    match offline::transcribe_encoded(&env.config_dir, &offline_model, &bytes).await {
                        Ok(text) => {
                            log_speech(env, "offline", "transcribe");
                            Ok(Recognized {
                                text,
                                tries: 1,
                                engine: "On this PC".into(),
                                language: "offline".into(),
                                path: "transcribe".into(),
                            })
                        }
                        Err(local) => Err(format!("{err} {local}")),
                    }
                }
                Err(err) => Err(err.to_string()),
            }
        }
        Err(err) => {
            if offline::normalize_model(&offline_model) == "off" {
                return Err(err.to_string());
            }
            match offline::transcribe_encoded(&env.config_dir, &offline_model, &bytes).await {
                Ok(text) => {
                    log_speech(env, "offline", "transcribe");
                    Ok(Recognized {
                        text,
                        tries: 1,
                        engine: "On this PC".into(),
                        language: "offline".into(),
                        path: "transcribe".into(),
                    })
                }
                Err(local) => Err(format!("{err} {local}")),
            }
        }
    }
}

async fn speech_with_groq(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    model: &str,
    language: &str,
    output: &str,
    prompt: Option<&str>,
    cleanup_model: &str,
    using_builtin: bool,
) -> anyhow::Result<Recognized> {
    if transcribe::direct_translate(output, language) {
        let translated = transcribe::translate_audio(client, api_key, audio, prompt, using_builtin).await?;
        let code = transcribe::normalize_language(language).unwrap_or_else(|| "unknown".into());
        return Ok(Recognized {
            text: translated.text,
            tries: translated.tries,
            engine: "Groq".into(),
            language: code,
            path: "translate".into(),
        });
    }
    let spoken = transcribe::transcribe(client, api_key, audio.clone(), model, language, prompt.map(str::to_string), using_builtin).await?;
    let detected = spoken.language.clone();
    let route = transcribe::speech_route(output, language, &detected, &spoken.text);
    match route {
        SpeechRoute::Transcribe => Ok(Recognized {
            text: spoken.text,
            tries: spoken.tries,
            engine: "Groq".into(),
            language: source_language(language, &detected),
            path: "transcribe".into(),
        }),
        SpeechRoute::Translate => {
            let translated = transcribe::translate_audio(client, api_key, audio, prompt, using_builtin).await?;
            Ok(Recognized {
                text: translated.text,
                tries: spoken.tries.saturating_add(translated.tries),
                engine: "Groq".into(),
                language: source_language(language, &detected),
                path: "translate".into(),
            })
        }
        SpeechRoute::Mixed => {
            let outcome = cleanup::translate_preserving(client, api_key, &spoken.text, cleanup_model, using_builtin).await;
            if outcome.used_model {
                return Ok(Recognized {
                    text: outcome.text,
                    tries: spoken.tries,
                    engine: "Groq".into(),
                    language: source_language(language, &detected),
                    path: "mixed-llama".into(),
                });
            }
            match transcribe::translate_audio(client, api_key, audio, prompt, using_builtin).await {
                Ok(translated) => Ok(Recognized {
                    text: translated.text,
                    tries: spoken.tries.saturating_add(translated.tries),
                    engine: "Groq".into(),
                    language: source_language(language, &detected),
                    path: "translate".into(),
                }),
                Err(_) if !spoken.text.trim().is_empty() => Ok(Recognized {
                    text: spoken.text,
                    tries: spoken.tries,
                    engine: "Groq".into(),
                    language: source_language(language, &detected),
                    path: "transcribe".into(),
                }),
                Err(err) => Err(err),
            }
        }
    }
}

fn source_language(setting: &str, detected: &str) -> String {
    let detected = transcribe::language_code(detected);
    if !detected.is_empty() {
        return detected;
    }
    transcribe::normalize_language(setting).unwrap_or_else(|| "unknown".into())
}

fn log_speech(env: &JobEnv, language: &str, path: &str) {
    let language = if language.is_empty() { "unknown" } else { language };
    log_line(
        &env.config_dir,
        "info",
        &format!("speech language={language} path={path}"),
    );
}

fn should_fallback(err: &anyhow::Error) -> bool {
    matches!(
        transcribe::failure_kind(err),
        Some(FailKind::RateLimit | FailKind::Unavailable)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(_index: usize, text: &str, engine: &str, error: Option<&str>) -> Product {
        Product {
            text: text.into(),
            engine: engine.into(),
            tries: 1,
            encode_ms: 1,
            remote_ms: 1,
            bytes: 8,
            format: "flac".into(),
            error: error.map(str::to_string),
            silence: text.is_empty() && error.is_none(),
            language: String::new(),
            speech_path: String::new(),
        }
    }

    #[test]
    fn parallel_chunks_join_in_order_and_name_the_engine() {
        let text = stitch_products(&[
            product(0, "ship the notes to Priya", "Groq", None),
            product(1, "to Priya before Thursday", "On this PC", None),
        ])
        .unwrap();
        assert_eq!(text, "ship the notes to Priya before Thursday");
        assert_eq!(
            engine_label(&[
                product(0, "a", "Groq", None),
                product(1, "b", "On this PC", None)
            ])
            .as_deref(),
            Some("Groq + on this PC")
        );
        assert_eq!(engine_label(&[product(0, "a", "Groq", None)]).as_deref(), Some("Groq"));
        let err = stitch_products(&[product(0, "", "", Some("Groq is rate limiting requests right now."))]);
        assert!(err.unwrap_err().contains("rate limiting"));
    }
}
