//! Transcribe cut chunks while the take is still recording.
//!
//! Two chunks run at a time. The rest wait, so a long take does not stampede Groq.
//! Each chunk tries Groq first and falls back to whisper.cpp on rate limit,
//! a rejected key, a server error, or a network failure. The following take tries Groq again.

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

pub const PARTIAL_NOTE: &str = "Part of this take didn't transcribe. The rest was inserted, and the full recording is kept.";

/// Like `stitch_products`, but a failed piece no longer throws away the pieces
/// that did transcribe. Returns the text plus a note when something was lost.
pub fn stitch_partial(products: &[Product]) -> Result<(String, Option<String>), String> {
    match stitch_products(products) {
        Ok(text) => Ok((text, None)),
        Err(error) => {
            let parts: Vec<&str> = products
                .iter()
                .filter(|p| p.error.is_none() && has_words(&p.text))
                .map(|p| p.text.as_str())
                .collect();
            if parts.is_empty() {
                Err(error)
            } else {
                Ok((crate::chunk::stitch(parts), Some(PARTIAL_NOTE.to_string())))
            }
        }
    }
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
        // A very short take can come back as just "." or "?". That is not
        // speech, and pasting a lone punctuation mark looks like a bug.
        if has_words(&product.text) {
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

pub fn has_words(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
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

/// Signals from one Groq recognition that feed the "unsure" decision.
#[derive(Default)]
pub(crate) struct SpeechFlags {
    /// A second pass (Urdu prompt or pinned language) replaced the first text.
    pub(crate) repass: bool,
    /// Whisper reported low confidence.
    pub(crate) doubtful: bool,
}

pub(crate) struct Recognized {
    pub(crate) text: String,
    pub(crate) tries: u8,
    pub(crate) engine: String,
    pub(crate) language: String,
    pub(crate) path: String,
}

async fn recognize(bytes: Vec<u8>, env: &JobEnv) -> Result<Recognized, String> {
    let offline_model = env.settings.offline_model.clone();
    let prompt = transcribe::whisper_prompt_with(&prompt_words(&env.settings.dictionary), &env.recent);
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

/// Turbo turns Urdu into Hindi script or broken Roman Urdu. The full model got
/// every Urdu test right (CER 0.10 vs 0.50) at similar speed, so use it
/// whenever Urdu is possible. English-only users keep Turbo.
pub(crate) fn effective_model<'a>(model: &'a str, language: &str) -> &'a str {
    let code = transcribe::normalize_language(language).unwrap_or_default();
    if model == transcribe::WHISPER_TURBO && code != "en" {
        transcribe::WHISPER_LARGE
    } else {
        model
    }
}

pub(crate) async fn speech_with_groq(
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
    let mut flags = SpeechFlags::default();
    let mut got = speech_with_groq_raw(client, api_key, audio, model, language, output, prompt, cleanup_model, using_builtin, &mut flags).await?;
    if flags.repass {
        got.path.push_str("-repass");
    }
    if flags.doubtful {
        got.path.push_str("-lowconf");
    }
    crate::diag::note(format!("route path={} language={} text={}", got.path, got.language, got.text));
    crate::diag::set_translated(got.path.contains("translate") || got.path.contains("mixed"));
    if transcribe::repetitive(&got.text) {
        crate::diag::note("loop detected in final text, collapsing");
        got.text = transcribe::collapse_repeats(&got.text);
        got.path = format!("{}-collapsed", got.path);
    }
    Ok(got)
}

#[allow(clippy::too_many_arguments)]
async fn speech_with_groq_raw(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    model: &str,
    language: &str,
    output: &str,
    prompt: Option<&str>,
    cleanup_model: &str,
    using_builtin: bool,
    flags: &mut SpeechFlags,
) -> anyhow::Result<Recognized> {
    if transcribe::direct_translate(output, language) {
        let translated = transcribe::translate_audio(client, api_key, audio, prompt, using_builtin).await?;
        flags.doubtful |= translated.doubtful;
        let code = transcribe::normalize_language(language).unwrap_or_else(|| "unknown".into());
        return Ok(Recognized {
            text: translated.text,
            tries: translated.tries,
            engine: "Groq".into(),
            language: code,
            path: "translate".into(),
        });
    }
    let model = effective_model(model, language);
    let urdu_set = transcribe::normalize_language(language).as_deref() == Some("ur");
    let first_prompt = if urdu_set { Some(transcribe::URDU_PROMPT.to_string()) } else { prompt.map(str::to_string) };
    crate::diag::note(format!("whisper#1 model={model} lang={language} prompt={:?}", first_prompt));
    let mut spoken = transcribe::transcribe(client, api_key, audio.clone(), model, language, first_prompt, using_builtin).await?;
    crate::diag::note(format!("whisper#1 detected={} text={}", spoken.language, spoken.text));
    if unlikely_language(language, &spoken.language) {
        // Short or accented English is sometimes detected as Spanish, Welsh,
        // etc., which then got translated and came out garbled. Bolo users
        // speak English and Urdu, so pin English and transcribe again.
        // Urdu is often detected as Hindi and written in Devanagari, which an
        // Urdu speaker can't use. Pin Urdu for that case, English otherwise.
        let pin = if hindi_like(&spoken.language, &spoken.text) { "ur" } else { "en" };
        let pin_prompt = if pin == "ur" { Some(transcribe::URDU_PROMPT.to_string()) } else { prompt.map(str::to_string) };
        crate::diag::note(format!("repin lang={pin} prompt={:?}", pin_prompt));
        if let Ok(pinned) = transcribe::transcribe(client, api_key, audio.clone(), model, pin, pin_prompt, using_builtin).await {
            crate::diag::note(format!("repin text={}", pinned.text));
            if !pinned.text.trim().is_empty() {
                let tries = spoken.tries.saturating_add(pinned.tries);
                spoken = pinned;
                spoken.tries = tries;
                spoken.language = pin.into();
                flags.repass = true;
            }
        }
    }
    if urdu_needs_clean_pass(language, &spoken.language, &spoken.text) {
        // Auto-detect heard Urdu but the English prompt leaked English
        // look-alike words in. Transcribe again as Urdu with an Urdu prompt.
        crate::diag::note("urdu clean pass (auto heard Urdu; redo with the Urdu prompt)");
        if let Ok(pinned) = transcribe::transcribe(client, api_key, audio.clone(), model, "ur", Some(transcribe::URDU_PROMPT.to_string()), using_builtin).await {
            crate::diag::note(format!("urdu clean pass text={}", pinned.text));
            if !pinned.text.trim().is_empty() {
                let tries = spoken.tries.saturating_add(pinned.tries);
                spoken = pinned;
                spoken.tries = tries;
                spoken.language = "ur".into();
                flags.repass = true;
            }
        }
    }
    flags.doubtful |= spoken.doubtful;
    let detected = spoken.language.clone();
    let mut route = transcribe::speech_route(output, language, &detected, &spoken.text);
    // Whisper's audio translation loops and drops most of a long Urdu take.
    // We already have the Urdu text, so translate that with the LLM instead.
    if route == SpeechRoute::Translate && transcribe::has_arabic(&spoken.text) && !transcribe::repetitive(&spoken.text) {
        route = SpeechRoute::Mixed;
    }
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
            flags.doubtful |= translated.doubtful;
            Ok(Recognized {
                text: translated.text,
                tries: spoken.tries.saturating_add(translated.tries),
                engine: "Groq".into(),
                language: source_language(language, &detected),
                path: "translate".into(),
            })
        }
        SpeechRoute::Mixed => {
            crate::diag::note(format!("llm translate input={}", spoken.text));
            let outcome = cleanup::translate_preserving(client, api_key, &spoken.text, cleanup_model, using_builtin).await;
            crate::diag::note(format!("llm translate used={} why={} loop={} output={}", outcome.used_model, outcome.why, transcribe::repetitive(&outcome.text), outcome.text));
            if outcome.used_model && !transcribe::repetitive(&outcome.text) {
                return Ok(Recognized {
                    text: outcome.text,
                    tries: spoken.tries,
                    engine: "Groq".into(),
                    language: source_language(language, &detected),
                    path: "mixed-llama".into(),
                });
            }
            match transcribe::translate_audio(client, api_key, audio, None, using_builtin).await {
                Ok(translated) if !transcribe::repetitive(&translated.text) || spoken.text.trim().is_empty() => Ok(Recognized {
                    text: translated.text,
                    tries: spoken.tries.saturating_add(translated.tries),
                    engine: "Groq".into(),
                    language: source_language(language, &detected),
                    path: "translate".into(),
                }),
                Ok(_) | Err(_) if !spoken.text.trim().is_empty() => Ok(Recognized {
                    text: spoken.text,
                    tries: spoken.tries,
                    engine: "Groq".into(),
                    language: source_language(language, &detected),
                    path: "transcribe".into(),
                }),
                Ok(translated) => Ok(Recognized {
                    text: translated.text,
                    tries: translated.tries,
                    engine: "Groq".into(),
                    language: source_language(language, &detected),
                    path: "translate".into(),
                }),
                Err(err) => Err(err),
            }
        }
    }
}

/// Auto language, Whisper heard Urdu, and the text has Latin words in it,
/// so the English prompt may have bent Urdu words into English ones.
pub(crate) fn urdu_needs_clean_pass(setting: &str, detected: &str, _text: &str) -> bool {
    // diag.log showed the English-prompt pass spelling بہتری as بیٹری
    // ("battery") even in pure Urdu script, so any Urdu heard on auto gets
    // the Urdu-prompt pass, not only takes with Latin words.
    transcribe::normalize_language(setting).is_none() && transcribe::language_code(detected) == "ur"
}

/// Auto language and Whisper picked something other than English or Urdu.
pub(crate) fn unlikely_language(setting: &str, detected: &str) -> bool {
    if transcribe::normalize_language(setting).is_some() {
        return false;
    }
    let code = transcribe::language_code(detected);
    !(code.is_empty() || code == "en" || code == "ur")
}

fn hindi_like(detected: &str, text: &str) -> bool {
    transcribe::language_code(detected) == "hi" || text.chars().any(|c| ('\u{0900}'..='\u{097F}').contains(&c))
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
        Some(FailKind::RateLimit | FailKind::Unavailable | FailKind::Rejected)
    )
}

#[cfg(test)]
mod tests {

    #[test]
    fn whisper_loops_are_detected_and_collapsed() {
        let bad = "Ok, so is there any, yeah. So is there any, yeah. So is there any, yeah.";
        assert!(transcribe::repetitive(bad));
        assert_eq!(transcribe::collapse_repeats(bad), "Ok, so is there any, yeah.");
        assert!(transcribe::repetitive("thank you thank you thank you thank you"));
        assert!(!transcribe::repetitive("Ok, and then we need to work on their traffic side as well to get more and more traffic on this, on my website."));
        assert!(!transcribe::repetitive("I said many things, but it only transcript the above line as it is."));
        assert!(!transcribe::repetitive("very very good"));
        let fine = "First point. Second point is different. Third one too.";
        assert_eq!(transcribe::collapse_repeats(fine), fine);
    }

    #[test]
    fn urdu_with_english_words_on_auto_gets_a_clean_urdu_pass() {
        assert!(urdu_needs_clean_pass("auto", "urdu", "کیا کوئی battery باقی ہے"));
        assert!(urdu_needs_clean_pass("auto", "urdu", "کیا کوئی بہتری باقی ہے"));
        assert!(!urdu_needs_clean_pass("auto", "english", "is there any betterment left"));
        assert!(!urdu_needs_clean_pass("ur", "urdu", "کیا کوئی battery باقی ہے"));
        assert!(!transcribe::URDU_PROMPT.chars().any(|c| c.is_ascii_alphabetic()));
    }
    #[test]
    fn urdu_capable_takes_use_the_full_model() {
        use crate::transcribe::{WHISPER_LARGE, WHISPER_TURBO};
        assert_eq!(super::effective_model(WHISPER_TURBO, "auto"), WHISPER_LARGE);
        assert_eq!(super::effective_model(WHISPER_TURBO, "ur"), WHISPER_LARGE);
        assert_eq!(super::effective_model(WHISPER_TURBO, "en"), WHISPER_TURBO);
        assert_eq!(super::effective_model(WHISPER_LARGE, "en"), WHISPER_LARGE);
    }

    use super::*;

    #[test]
    fn a_rejected_key_falls_back_to_the_on_device_model() {
        let err = anyhow::Error::from(transcribe::GroqFailure {
            message: transcribe::REJECTED_KEY_MESSAGE.into(),
            kind: FailKind::Rejected,
        });
        assert!(should_fallback(&err));
        let other = anyhow::Error::msg("not a groq failure");
        assert!(!should_fallback(&other));
    }

    #[test]
    fn lone_punctuation_is_not_speech() {
        assert!(!has_words("."));
        assert!(!has_words(" ؟ ... "));
        assert!(has_words("ok."));
        assert!(has_words("سلام"));
    }

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

/// The words list plus what the user's fixes taught (Fix last).
/// My words and the built-in names come first, so they survive the prompt cap.
pub(crate) fn prompt_words(dictionary: &[String]) -> Vec<String> {
    let memory = crate::memory::snapshot();
    let mut words = crate::vocab::protected(&memory.my_words);
    crate::vocab::set_current(words.clone());
    for term in dictionary.iter().cloned().chain(crate::memory::prompt_terms(&memory)) {
        if !words.iter().any(|word| word.eq_ignore_ascii_case(&term)) {
            words.push(term);
        }
    }
    words
}
