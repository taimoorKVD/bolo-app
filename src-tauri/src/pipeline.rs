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
use crate::transcribe::{self, FailKind};
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
        Ok((text, tries, engine)) => {
            product.text = text;
            product.tries = tries;
            product.engine = engine;
        }
        Err(err) => product.error = Some(err),
    }
    product.remote_ms = remote_started.elapsed().as_millis() as u64;
    product
}

async fn recognize(bytes: Vec<u8>, env: &JobEnv) -> Result<(String, u8, String), String> {
    let offline_model = env.settings.offline_model.clone();
    let prompt = transcribe::whisper_prompt_with(&env.settings.dictionary, &env.recent);
    let model = env.settings.model.clone();
    let language = env.settings.language.clone();
    match SecretKey::load() {
        Ok(key) => {
            let groq = transcribe::transcribe(
                transcribe::http_client(),
                key.expose(),
                bytes.clone(),
                &model,
                &language,
                prompt,
            )
            .await;
            drop(key);
            match groq {
                Ok(transcript) => Ok((transcript.text, transcript.tries, "Groq".into())),
                Err(err) if should_fallback(&err) && offline::normalize_model(&offline_model) != "off" => {
                    match offline::transcribe_encoded(&env.config_dir, &offline_model, &bytes).await {
                        Ok(text) => Ok((text, 1, "On this PC".into())),
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
                Ok(text) => Ok((text, 1, "On this PC".into())),
                Err(local) => Err(format!("{err} {local}")),
            }
        }
    }
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
