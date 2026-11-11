//! Transcribe cut chunks while the take is still recording.
//!
//! Two chunks run at a time. The rest wait, so a long take does not stampede Groq.
//! Each chunk tries Groq first and falls back to whisper.cpp on rate limit,
//! a rejected key, a server error, or a network failure. The following take tries Groq again.

use crate::audio::{self, FinishedAudio, SpeechError};
use crate::provider::{self, RouterAction};
use crate::offline;
use crate::spool;
use crate::store::Settings;
use crate::cleanup;
use crate::logbuf::log_line;
use crate::transcribe::{self, FailKind, SpeechRoute};
use std::path::Path;
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
    let (encoded, processed) = match audio::prepare_speech_parts(&audio) {
        Ok(parts) => parts,
        Err(SpeechError::Silent | SpeechError::Empty) => {
            crate::measure::note_audio(index, &audio, &[]);
            crate::measure::note_chunk_error(index, "silent or empty audio");
            product.silence = true;
            product.encode_ms = encode_started.elapsed().as_millis() as u64;
            return product;
        }
        Err(err) => {
            crate::measure::note_audio(index, &audio, &[]);
            crate::measure::note_chunk_error(index, &err.to_string());
            product.encode_ms = encode_started.elapsed().as_millis() as u64;
            product.error = Some(err.to_string());
            return product;
        }
    };
    crate::measure::note_audio(index, &audio, &processed);
    drop(processed);
    product.encode_ms = encode_started.elapsed().as_millis() as u64;
    product.bytes = encoded.bytes.len();
    product.format = encoded.format.to_string();
    spool::write_audio(&env.config_dir, &env.spool_id, index, &encoded.bytes);
    let remote_started = Instant::now();
    match tauri::async_runtime::block_on(recognize(
        encoded.bytes,
        env,
        crate::managed::audio_minutes(audio.interleaved.len(), audio.channels, audio.sample_rate),
        encoded.format,
        encoded.speech_secs,
    )) {
        Ok(recognized) => {
            product.text = transcribe::guard_short_take(&recognized.text, encoded.speech_secs);
            product.tries = recognized.tries;
            product.engine = recognized.engine;
            product.language = recognized.language.clone();
            product.speech_path = recognized.path;
            let (calls, raw) = crate::measure::drain_calls();
            crate::measure::attach_text(index, calls, raw, &recognized.language, &product.text);
        }
        Err(err) => {
            let (calls, raw) = crate::measure::drain_calls();
            crate::measure::attach_text(index, calls, raw, "", "");
            crate::measure::note_chunk_error(index, &err);
            product.error = Some(err);
        }
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

async fn recognize(bytes: Vec<u8>, env: &JobEnv, minutes: f64, _format: &str, speech_secs: f32) -> Result<Recognized, String> {
    let offline_model = env.settings.offline_model.clone();
    // A one-second clip was continuing the prompt ("Bolo and Easy, it's such a
    // truth…"). A normal sentence gets rare spellings only — not an instruction
    // and not the previous paste, which Whisper continues as if it were speech.
    let prompt = if transcribe::brief_audio(speech_secs) {
        Some(transcribe::whisper_prompt_brief())
    } else {
        transcribe::whisper_prompt_with(&prompt_words(&env.settings.dictionary, &env.settings.language), &env.recent)
    };
    let model = env.settings.model.clone();
    let language = env.settings.language.clone();
    let output = env.settings.output.clone();
    let cleanup_model = env.settings.cleanup_model.clone();
    // Managed Bolo first, once an account is configured. It runs the same
    // routing as the Groq path (vocabulary prompt, Urdu clean pass, Mixed
    // translation). Any failure falls through to the user's own key, then
    // on-device, exactly as before.
    if crate::managed::is_configured() {
        let via = Via::Managed { minutes };
        if let Ok(recognized) = speech_via(&via, bytes.clone(), &model, &language, &output, prompt.as_deref(), &cleanup_model).await {
            if !recognized.text.trim().is_empty() {
                log_speech(env, &recognized.language, &format!("managed-{}", recognized.path));
                return Ok(recognized);
            }
        }
    }
    match provider::load_ready_keys(&env.config_dir) {

        Ok((mut store, keys)) => {
            let mut last_err: Option<anyhow::Error> = None;
            let mut saw_outage = false;
            let mut force_offline = false;
            for key in keys {
                let using_builtin = key.builtin;
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
                match groq {
                    Ok(recognized) => {
                        store.record_success(&key.id);
                        provider::save_store(&env.config_dir, &store);
                        log_speech(env, &recognized.language, &recognized.path);
                        return Ok(recognized);
                    }
                    Err(err) => {
                        let action = match transcribe::failure_details(&err) {
                            Some(failure) => store.record_failure(&key.id, failure),
                            None => RouterAction::Switch,
                        };
                        provider::save_store(&env.config_dir, &store);
                        last_err = Some(err);
                        match action {
                            RouterAction::RetrySame => {
                                // speech_with_groq already retried burst limits; try next key.
                                continue;
                            }
                            RouterAction::Switch => continue,
                            RouterAction::Outage => {
                                saw_outage = true;
                                continue;
                            }
                            RouterAction::Offline => {
                                force_offline = true;
                                break;
                            }
                        }
                    }
                }
            }
            let err = last_err.unwrap_or_else(|| anyhow::anyhow!("No Groq API key is ready."));
            let offline_ok = offline::normalize_model(&offline_model) != "off"
                && (force_offline || saw_outage || should_fallback(&err));
            if offline_ok {
                match on_device_text(&env.config_dir, &offline_model, &language, &bytes).await {
                    Ok(text) => {
                        log_speech(env, "offline", "transcribe");
                        crate::measure::note_call("raw", &offline_model, "offline", None, "offline", &text);
                        return Ok(Recognized {
                            text,
                            tries: 1,
                            engine: "On this PC".into(),
                            language: "offline".into(),
                            path: "transcribe".into(),
                        });
                    }
                    Err(local) => return Err(offline_error(&err.to_string(), local)),
                }
            }
            Err(err.to_string())
        }
        Err(err) => {
            if offline::normalize_model(&offline_model) == "off" {
                return Err(err.to_string());
            }
            match on_device_text(&env.config_dir, &offline_model, &language, &bytes).await {
                Ok(text) => {
                    log_speech(env, "offline", "transcribe");
                    crate::measure::note_call("raw", &offline_model, "offline", None, "offline", &text);
                    Ok(Recognized {
                        text,
                        tries: 1,
                        engine: "On this PC".into(),
                        language: "offline".into(),
                        path: "transcribe".into(),
                    })
                }
                Err(local) => Err(offline_error(&err.to_string(), local)),
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
    let via = Via::Groq { client, api_key, using_builtin };
    speech_via(&via, audio, model, language, output, prompt, cleanup_model).await
}

/// Where speech goes. The user's Groq key and managed Bolo share one routing
/// path, so the vocabulary prompt, language pins, the Urdu clean pass and the
/// Mixed (LLM) translation behave the same on both.
pub(crate) enum Via<'a> {
    Groq { client: &'a reqwest::Client, api_key: &'a str, using_builtin: bool },
    Managed { minutes: f64 },
    #[cfg(test)]
    Fake(&'a fake::FakeVia),
}

impl Via<'_> {
    fn engine(&self) -> &'static str {
        match self {
            Via::Groq { .. } => "Groq",
            Via::Managed { .. } => "Bolo",
            #[cfg(test)]
            Via::Fake(_) => "Fake",
        }
    }

    async fn transcribe(&self, audio: Vec<u8>, model: &str, language: &str, prompt: Option<String>) -> anyhow::Result<transcribe::Transcript> {
        match self {
            Via::Groq { client, api_key, using_builtin } => {
                transcribe::transcribe(client, api_key, audio, model, language, prompt, *using_builtin).await
            }
            Via::Managed { minutes } => crate::managed::whisper(&audio, *minutes, "transcribe", language, prompt.as_deref())
                .await
                .ok_or_else(|| anyhow::anyhow!("managed speech unavailable")),
            #[cfg(test)]
            Via::Fake(fake) => fake.transcribe(model, language, prompt),
        }
    }

    async fn translate_audio(&self, audio: Vec<u8>, prompt: Option<&str>) -> anyhow::Result<transcribe::Transcript> {
        match self {
            Via::Groq { client, api_key, using_builtin } => {
                transcribe::translate_audio(client, api_key, audio, prompt, *using_builtin).await
            }
            Via::Managed { minutes } => crate::managed::whisper(&audio, *minutes, "translate", "", prompt)
                .await
                .ok_or_else(|| anyhow::anyhow!("managed speech unavailable")),
            #[cfg(test)]
            Via::Fake(fake) => fake.translate(prompt),
        }
    }

    async fn translate_text(&self, text: &str, cleanup_model: &str) -> cleanup::CleanOutcome {
        match self {
            Via::Groq { client, api_key, using_builtin } => {
                cleanup::translate_preserving(client, api_key, text, cleanup_model, *using_builtin).await
            }
            Via::Managed { .. } => crate::managed::translate_text(text).await,
            #[cfg(test)]
            Via::Fake(fake) => fake.translate_text(text),
        }
    }
}

pub(crate) async fn speech_via(
    via: &Via<'_>,
    audio: Vec<u8>,
    model: &str,
    language: &str,
    output: &str,
    prompt: Option<&str>,
    cleanup_model: &str,
) -> anyhow::Result<Recognized> {
    let mut flags = SpeechFlags::default();
    let mut got = speech_raw(via, audio, model, language, output, prompt, cleanup_model, &mut flags).await?;
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
    got.text = transcribe::recover_brief_tech(&got.text);
    Ok(got)
}

#[allow(clippy::too_many_arguments)]
async fn speech_raw(
    via: &Via<'_>,
    audio: Vec<u8>,
    model: &str,
    language: &str,
    output: &str,
    prompt: Option<&str>,
    cleanup_model: &str,
    flags: &mut SpeechFlags,
) -> anyhow::Result<Recognized> {
    if transcribe::direct_translate(output, language) {
        let translated = via.translate_audio(audio, prompt).await?;
        flags.doubtful |= translated.doubtful;
        crate::measure::note_call("translate", model, language, prompt, &translated.language, &translated.text);
        let code = transcribe::normalize_language(language).unwrap_or_else(|| "unknown".into());
        return Ok(Recognized {
            text: translated.text,
            tries: translated.tries,
            engine: via.engine().into(),
            language: code,
            path: "translate".into(),
        });
    }
    let model = effective_model(model, language);
    let urdu_set = transcribe::normalize_language(language).as_deref() == Some("ur");
    let hindi_set = keep_hindi_script(language, output);
    let first_prompt = if urdu_set {
        Some(transcribe::URDU_PROMPT.to_string())
    } else if hindi_set {
        Some(transcribe::HINDI_PROMPT.to_string())
    } else {
        prompt.map(str::to_string)
    };
    crate::diag::note(format!("whisper#1 model={model} lang={language} prompt={:?}", first_prompt));
    let mut spoken = via.transcribe(audio.clone(), model, language, first_prompt.clone()).await?;
    crate::measure::note_call("raw", model, language, first_prompt.as_deref(), &spoken.language, &spoken.text);
    crate::diag::note(format!("whisper#1 detected={} text={}", spoken.language, spoken.text));
    if prefer_english_pass(language, output, &spoken.text, &spoken.language, spoken.doubtful) {
        // A short or low-confidence tag (often Urdu or Hindi on an English
        // phrase) must not throw the words away or send them through
        // translation. Hear it once more as English. If that pass fails,
        // the text we already have is what gets inserted.
        crate::diag::note("short or unclear language tag; transcribing again in English");
        if let Ok(pinned) = via.transcribe(audio.clone(), model, "en", prompt.map(str::to_string)).await {
            crate::diag::note(format!("english pass text={}", pinned.text));
            if !pinned.text.trim().is_empty() {
                let tries = spoken.tries.saturating_add(pinned.tries);
                spoken = pinned;
                spoken.tries = tries;
                if transcribe::language_code(&spoken.language).is_empty() {
                    spoken.language = "en".into();
                }
                flags.repass = true;
                crate::measure::note_call("redecode", model, "en", prompt, &spoken.language, &spoken.text);
            }
        }
    } else if unlikely_language(language, &spoken.language) {
        // Short or accented English is sometimes detected as Spanish, Welsh,
        // etc., which then got translated and came out garbled. Bolo users
        // speak English and Urdu, so pin English and transcribe again.
        // Urdu is often detected as Hindi and written in Devanagari, which an
        // Urdu speaker can't use. Pin Urdu for that case, English otherwise.
        let pin = if transcribe::looks_like_roman_urdu(&spoken.text)
            && transcribe::normalize_language(language).as_deref() != Some("en")
        {
            "ur"
        } else if hindi_like(&spoken.language, &spoken.text) && !transcribe::latin_only(&spoken.text) {
            // Only an explicit Hindi + "same language" setting keeps Devanagari.
            // Auto and Urdu still pin Urdu: Whisper often writes Urdu in Devanagari.
            if keep_hindi_script(language, output) { "hi" } else { "ur" }
        } else {
            "en"
        };
        let pin_prompt = if pin == "ur" {
            Some(transcribe::URDU_PROMPT.to_string())
        } else if pin == "hi" {
            Some(transcribe::HINDI_PROMPT.to_string())
        } else {
            prompt.map(str::to_string)
        };
        crate::diag::note(format!("repin lang={pin} prompt={:?}", pin_prompt));
        if let Ok(pinned) = via.transcribe(audio.clone(), model, pin, pin_prompt.clone()).await {
            crate::diag::note(format!("repin text={}", pinned.text));
            if keep_repass(&spoken, &pinned) {
                let tries = spoken.tries.saturating_add(pinned.tries);
                spoken = pinned;
                spoken.tries = tries;
                spoken.language = pin.into();
                flags.repass = true;
                crate::measure::note_call("redecode", model, pin, pin_prompt.as_deref(), &spoken.language, &spoken.text);
            }
        }
    }
    if urdu_needs_clean_pass(language, &spoken.language, &spoken.text) && !flags.repass {
        // Auto-detect heard Urdu but the English prompt leaked English
        // look-alike words in. Transcribe again as Urdu with an Urdu prompt.
        crate::diag::note("urdu clean pass (auto heard Urdu; redo with the Urdu prompt)");
        if let Ok(pinned) = via.transcribe(audio.clone(), model, "ur", Some(transcribe::URDU_PROMPT.to_string())).await {
            crate::diag::note(format!("urdu clean pass text={}", pinned.text));
            if !pinned.text.trim().is_empty() && keep_repass(&spoken, &pinned) {
                // The Urdu prompt has no Latin letters, so a mixed sentence can
                // come back with the English words rewritten in Urdu script.
                // Keep the pass that still has those English words.
                let merged = preserve_english(&spoken.text, &pinned.text);
                if merged.trim() != spoken.text.trim() {
                    let tries = spoken.tries.saturating_add(pinned.tries);
                    spoken = pinned;
                    spoken.text = merged;
                    spoken.tries = tries;
                    spoken.language = "ur".into();
                    flags.repass = true;
                    crate::measure::note_call("redecode", model, "ur", Some(transcribe::URDU_PROMPT), &spoken.language, &spoken.text);
                }
            }
        }
    }
    if keep_hindi_script(language, output)
        && transcribe::has_arabic(&spoken.text)
        && !transcribe::has_devanagari(&spoken.text)
        && !flags.repass
    {
        // The user asked for Hindi, and the first pass came back in Urdu script.
        crate::diag::note("hindi script pass (setting is Hindi; redo without the Urdu prompt)");
        if let Ok(pinned) = via.transcribe(audio.clone(), model, "hi", Some(transcribe::HINDI_PROMPT.to_string())).await {
            crate::diag::note(format!("hindi script pass text={}", pinned.text));
            if transcribe::has_devanagari(&pinned.text) && keep_repass(&spoken, &pinned) {
                let tries = spoken.tries.saturating_add(pinned.tries);
                spoken = pinned;
                spoken.tries = tries;
                spoken.language = "hi".into();
                flags.repass = true;
                crate::measure::note_call("redecode", model, "hi", Some(transcribe::HINDI_PROMPT), &spoken.language, &spoken.text);
            }
        }
    }
    if keep_hindi_script(language, output) && transcribe::has_devanagari(&spoken.text) {
        spoken.language = "hi".into();
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
            engine: via.engine().into(),
            language: source_language(language, &detected),
            path: "transcribe".into(),
        }),
        SpeechRoute::Translate => {
            // A later translation failure must not throw away a transcript Groq
            // already returned. That was dropping Urdu takes and showing the
            // offline-model notice instead of pasting the words.
            match via.translate_audio(audio, prompt).await {
                Ok(translated) if !translated.text.trim().is_empty() => {
                    flags.doubtful |= translated.doubtful;
                    crate::measure::note_call("translate", model, language, prompt, &translated.language, &translated.text);
                    Ok(Recognized {
                        text: translated.text,
                        tries: spoken.tries.saturating_add(translated.tries),
                        engine: via.engine().into(),
                        language: source_language(language, &detected),
                        path: "translate".into(),
                    })
                }
                Ok(_) | Err(_) if !spoken.text.trim().is_empty() => {
                    crate::diag::note("translation failed; keeping the online transcript");
                    Ok(Recognized {
                        text: spoken.text,
                        tries: spoken.tries,
                        engine: via.engine().into(),
                        language: source_language(language, &detected),
                        path: "transcribe".into(),
                    })
                }
                Ok(translated) => Ok(Recognized {
                    text: translated.text,
                    tries: spoken.tries.saturating_add(translated.tries),
                    engine: via.engine().into(),
                    language: source_language(language, &detected),
                    path: "translate".into(),
                }),
                Err(err) => Err(err),
            }
        }
        SpeechRoute::Mixed => {
            crate::diag::note(format!("llm translate input={}", spoken.text));
            let outcome = via.translate_text(&spoken.text, cleanup_model).await;
            crate::diag::note(format!("llm translate used={} why={} loop={} output={}", outcome.used_model, outcome.why, transcribe::repetitive(&outcome.text), outcome.text));
            if outcome.used_model && !transcribe::repetitive(&outcome.text) {
                crate::measure::note_call("translate", cleanup_model, "text", None, &detected, &outcome.text);
                return Ok(Recognized {
                    text: outcome.text,
                    tries: spoken.tries,
                    engine: via.engine().into(),
                    language: source_language(language, &detected),
                    path: "mixed-llama".into(),
                });
            }
            match via.translate_audio(audio, None).await {
                Ok(translated) if !transcribe::repetitive(&translated.text) || spoken.text.trim().is_empty() => {
                    crate::measure::note_call("translate", model, language, None, &translated.language, &translated.text);
                    Ok(Recognized {
                        text: translated.text,
                        tries: spoken.tries.saturating_add(translated.tries),
                        engine: via.engine().into(),
                        language: source_language(language, &detected),
                        path: "translate".into(),
                    })
                }
                Ok(_) | Err(_) if !spoken.text.trim().is_empty() => Ok(Recognized {
                    text: spoken.text,
                    tries: spoken.tries,
                    engine: via.engine().into(),
                    language: source_language(language, &detected),
                    path: "transcribe".into(),
                }),
                Ok(translated) => Ok(Recognized {
                    text: translated.text,
                    tries: translated.tries,
                    engine: via.engine().into(),
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
/// Prefer the Urdu-prompt redo unless it dropped Latin words the first pass heard.
pub(crate) fn preserve_english(original: &str, redo: &str) -> String {
    let latin = latin_words(original);
    if latin.is_empty() {
        return redo.to_string();
    }
    let redo_latin = latin_words(redo);
    let kept = latin.iter().all(|word| redo_latin.iter().any(|other| other.eq_ignore_ascii_case(word)));
    if kept { redo.to_string() } else { original.to_string() }
}

fn latin_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|token| token.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_string())
        .filter(|word| word.chars().any(|c| c.is_ascii_alphabetic()) && word.chars().all(|c| c.is_ascii_alphanumeric()))
        .collect()
}

pub(crate) fn urdu_needs_clean_pass(setting: &str, detected: &str, text: &str) -> bool {
    // diag.log showed the English-prompt pass spelling بہتری as بیٹری
    // ("battery") even in pure Urdu script, so Urdu script on auto gets
    // the Urdu-prompt pass. Latin-only English that Whisper tagged as Urdu
    // does not: that pass rewrites the sentence into Urdu and the English
    // output path then translates the rewrite.
    transcribe::normalize_language(setting).is_none()
        && transcribe::language_code(detected) == "ur"
        && (transcribe::has_arabic(text) || !transcribe::has_latin(text))
}

/// A doubtful second pass must not replace a decode Whisper was sure about.
pub(crate) fn keep_repass(first: &transcribe::Transcript, second: &transcribe::Transcript) -> bool {
    if second.text.trim().is_empty() {
        return false;
    }
    if first.text.trim().is_empty() {
        return true;
    }
    !(second.doubtful && !first.doubtful)
}

/// Auto language, English output, and a tag other than English on a short
/// or low-confidence take. Urdu is kept only when the take is long enough
/// and Whisper was sure.
pub(crate) fn prefer_english_pass(setting: &str, output: &str, text: &str, detected: &str, doubtful: bool) -> bool {
    if transcribe::normalize_language(setting).is_some() || transcribe::normalize_output(output) != "english" {
        return false;
    }
    let code = transcribe::language_code(detected);
    if code.is_empty() || code == "en" {
        return false;
    }
    doubtful || text.split_whitespace().count() < 3
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
    transcribe::language_code(detected) == "hi" || transcribe::has_devanagari(text)
}

/// Devanagari is kept only when the user set "I speak" to Hindi and "Write my
/// words in" to the language they spoke. Auto and Urdu keep the Urdu repin,
/// because Whisper often writes Urdu speech in Devanagari. English output
/// still translates, so Hindi is not pasted when the user asked for English.
pub(crate) fn keep_hindi_script(setting: &str, output: &str) -> bool {
    transcribe::normalize_output(output) == "same"
        && transcribe::normalize_language(setting).as_deref() == Some("hi")
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

async fn on_device_text(config: &Path, selected: &str, language: &str, bytes: &[u8]) -> Result<String, String> {
    if offline::multilingual_download_wanted(config, selected, language) {
        crate::diag::note("multilingual model missing; downloading in the background");
        offline::prefetch(config.to_path_buf(), "ggml-base".into());
    }
    let model = offline::resolve_fallback(config, selected, language)?;
    offline::transcribe_encoded(config, model, language, bytes).await
}

/// Shown when a take produced no words. The real cause stays in the log.
pub const PLAIN_MISS: &str = "Couldn't catch that, please try again";

pub fn plain_take_notice(raw: &str) -> &'static str {
    let _ = raw;
    PLAIN_MISS
}

pub async fn retry_encoded(bytes: Vec<u8>, env: &JobEnv) -> Result<Recognized, String> {
    recognize(bytes, env, 0.05, "flac", 1.0).await
}

fn offline_error(groq: &str, local: String) -> String {
    if offline::is_multilingual_notice(&local) {
        local
    } else {
        format!("{groq} {local}")
    }
}

fn should_fallback(err: &anyhow::Error) -> bool {
    matches!(
        transcribe::failure_kind(err),
        Some(FailKind::RateLimit | FailKind::QuotaExhausted | FailKind::Unavailable | FailKind::Rejected)
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
        assert!(
            !urdu_needs_clean_pass("auto", "urdu", "please send the invoice by Friday"),
            "latin English tagged Urdu stays English"
        );
        assert!(!transcribe::URDU_PROMPT.chars().any(|c| c.is_ascii_alphabetic()));
        assert!(!transcribe::HINDI_PROMPT.chars().any(|c| c.is_ascii_alphabetic()));
        assert!(transcribe::has_devanagari(transcribe::HINDI_PROMPT));
        assert!(!transcribe::has_arabic(transcribe::HINDI_PROMPT));
        assert!(keep_hindi_script("hi", "same"));
        assert!(keep_hindi_script("HI", "Same"));
        assert!(!keep_hindi_script("hi", "english"), "English output still translates");
        assert!(!keep_hindi_script("auto", "same"), "auto keeps the Urdu repin");
        assert!(!keep_hindi_script("ur", "same"));
        let sure = transcribe::Transcript { text: "send the invoice".into(), tries: 1, language: "en".into(), doubtful: false };
        let worse = transcribe::Transcript { text: "send the invoice tomorrow somehow".into(), tries: 1, language: "en".into(), doubtful: true };
        let better = transcribe::Transcript { text: "send the invoice today".into(), tries: 1, language: "en".into(), doubtful: false };
        assert!(!keep_repass(&sure, &worse));
        assert!(keep_repass(&sure, &better));
        assert!(keep_repass(&worse, &better));
        assert!(prefer_english_pass("auto", "english", "ہاں", "ur", false), "a short Urdu tag is retried in English");
        assert!(prefer_english_pass("auto", "english", "please send the long invoice today", "ur", true), "low confidence is retried");
        assert!(!prefer_english_pass("auto", "english", "آج meeting ہے", "ur", false), "a clear Urdu sentence stays");
        assert!(!prefer_english_pass("auto", "same", "ہاں", "ur", false));
        assert!(!prefer_english_pass("en", "english", "ہاں", "ur", false));
        let notice = plain_take_notice(offline::URDU_OFFLINE_NOTICE);
        assert_eq!(notice, PLAIN_MISS);
        assert!(!notice.to_lowercase().contains("model"));
        assert!(!notice.to_lowercase().contains("urdu"));
        assert!(!notice.to_lowercase().contains("groq"));
    }
    #[test]
    fn urdu_capable_takes_use_the_full_model() {
        use crate::transcribe::{WHISPER_LARGE, WHISPER_TURBO};
        assert_eq!(super::effective_model(WHISPER_TURBO, "auto"), WHISPER_LARGE);
        assert_eq!(super::effective_model(WHISPER_TURBO, "ur"), WHISPER_LARGE);
        assert_eq!(super::effective_model(WHISPER_TURBO, "hi"), WHISPER_LARGE);
        assert_eq!(super::effective_model(WHISPER_TURBO, "en"), WHISPER_TURBO);
        assert_eq!(super::effective_model(WHISPER_LARGE, "en"), WHISPER_LARGE);
    }

    use super::*;

    #[test]
    fn a_rejected_key_falls_back_to_the_on_device_model() {
        let err = anyhow::Error::from(transcribe::GroqFailure {
            message: transcribe::REJECTED_KEY_MESSAGE.into(),
            kind: FailKind::Rejected,
            explicit_quota: false,
            retry_after_ms: None,
            offline_hint: false,
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
pub(crate) fn prompt_words(dictionary: &[String], language: &str) -> Vec<String> {
    let memory = crate::memory::snapshot();
    let ranked = crate::memory::rank_my_words(&memory);
    let mut words = crate::vocab::protected(&ranked);
    crate::vocab::set_current(words.clone());
    let mut extra = 0usize;
    for term in dictionary
        .iter()
        .cloned()
        .chain(crate::memory::prompt_terms(&memory))
        .chain(crate::hints::starter_hints(language))
    {
        if extra >= 24 {
            break;
        }
        if crate::cleanup::should_drop_learned(&term) {
            continue;
        }
        if !words.iter().any(|word| word.eq_ignore_ascii_case(&term)) {
            words.push(term);
            extra += 1;
        }
    }
    words
}

#[cfg(test)]
pub(crate) mod fake {
    use crate::cleanup::CleanOutcome;
    use crate::transcribe::Transcript;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Scripted speech backend that records every call (kind, language, prompt).
    #[derive(Default)]
    pub struct FakeVia {
        pub calls: Mutex<Vec<(String, String, Option<String>)>>,
        replies: Mutex<VecDeque<(String, String)>>,
        translation: Option<String>,
        pub fail_translate: bool,
        pub doubt: bool,
    }

    impl FakeVia {
        pub fn new(replies: &[(&str, &str)], translation: Option<&str>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                replies: Mutex::new(replies.iter().map(|(t, l)| (t.to_string(), l.to_string())).collect()),
                translation: translation.map(str::to_string),
                fail_translate: false,
                doubt: false,
            }
        }
        fn push(&self, kind: String, language: &str, prompt: Option<String>) {
            self.calls.lock().unwrap().push((kind, language.to_string(), prompt));
        }
        pub fn transcribe(&self, model: &str, language: &str, prompt: Option<String>) -> anyhow::Result<Transcript> {
            self.push(format!("transcribe:{model}"), language, prompt);
            let (text, language) = self.replies.lock().unwrap().pop_front().ok_or_else(|| anyhow::anyhow!("no reply"))?;
            Ok(Transcript { text, tries: 1, language, doubtful: self.doubt })
        }
        pub fn translate(&self, prompt: Option<&str>) -> anyhow::Result<Transcript> {
            self.push("translate".into(), "", prompt.map(str::to_string));
            if self.fail_translate {
                return Err(anyhow::anyhow!("couldn't reach Groq"));
            }
            Ok(Transcript { text: "translated audio".into(), tries: 1, language: "en".into(), doubtful: false })
        }
        pub fn translate_text(&self, text: &str) -> CleanOutcome {
            self.push("translate_text".into(), "", Some(text.to_string()));
            match &self.translation {
                Some(t) => CleanOutcome { text: t.clone(), used_model: true, note: None, why: "fake".into() },
                None => CleanOutcome { text: text.to_string(), used_model: false, note: None, why: "fake".into() },
            }
        }
    }
}

#[cfg(test)]
mod via_tests {
    use super::fake::FakeVia;
    use super::*;

    fn run(fake: &FakeVia, language: &str, output: &str, prompt: Option<&str>) -> Recognized {
        tauri::async_runtime::block_on(speech_via(
            &Via::Fake(fake),
            vec![1, 2, 3],
            transcribe::WHISPER_TURBO,
            language,
            output,
            prompt,
            "cleanup-model",
        ))
        .expect("speech")
    }

    #[test]
    fn vocabulary_prompt_and_pinned_language_reach_the_backend() {
        let fake = FakeVia::new(&[("Deploy to Supabase today", "en")], None);
        let got = run(&fake, "en", "same", Some("Supabase, Taimoor"));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "en");
        assert_eq!(calls[0].2.as_deref(), Some("Supabase, Taimoor"));
        assert_eq!(got.engine, "Fake");
        assert_eq!(got.path, "transcribe");
    }

    #[test]
    fn auto_urdu_gets_the_urdu_prompt_clean_pass() {
        // The redo drops the Latin word the first pass heard, so that word stays.
        // The Urdu-prompt pass still runs.
        let fake = FakeVia::new(&[("آج battery ختم", "ur"), ("آج بیٹری ختم", "ur")], None);
        let got = run(&fake, "auto", "same", Some("Bolo"));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].2.as_deref(), Some("Bolo"));
        assert_eq!(calls[1].1, "ur");
        assert_eq!(calls[1].2.as_deref(), Some(transcribe::URDU_PROMPT));
        assert!(calls[1].0.contains(transcribe::WHISPER_LARGE), "Urdu uses the large model: {:?}", calls[1]);
        assert_eq!(got.text, "آج battery ختم");
        assert!(!got.path.contains("repass"));
    }

    #[test]
    fn urdu_clean_pass_is_kept_when_english_words_survive() {
        let fake = FakeVia::new(&[("آج battery ختم", "ur"), ("آج battery ٹھیک", "ur")], None);
        let got = run(&fake, "auto", "same", Some("Bolo"));
        assert_eq!(got.text, "آج battery ٹھیک");
        assert!(got.path.ends_with("-repass"));
    }

    #[test]
    fn urdu_to_english_uses_llm_translation_of_the_urdu_text() {
        let fake = FakeVia::new(&[("آج meeting ہے", "ur"), ("آج meeting ہے", "ur")], Some("There is a meeting today."));
        let got = run(&fake, "auto", "english", None);
        let calls = fake.calls.lock().unwrap();
        assert!(calls.iter().any(|c| c.0 == "translate_text" && c.2.as_deref() == Some("آج meeting ہے")));
        assert_eq!(got.text, "There is a meeting today.");
        assert!(got.path.starts_with("mixed-llama"));
    }

    #[test]
    fn english_mistagged_as_urdu_is_not_translated() {
        let fake = FakeVia::new(&[("please send the invoice by Friday", "ur")], None);
        let got = run(&fake, "auto", "english", Some("Bolo"));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "no Urdu redo and no translation: {calls:?}");
        assert_eq!(got.text, "please send the invoice by Friday");
        assert_eq!(got.path, "transcribe");
    }

    #[test]
    fn latin_text_tagged_hindi_is_repinned_to_english() {
        let fake = FakeVia::new(
            &[("please send the invoice", "hi"), ("please send the invoice by Friday", "en")],
            None,
        );
        let got = run(&fake, "auto", "english", Some("Bolo"));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!(calls[1].1, "en");
        assert_ne!(calls[1].2.as_deref(), Some(transcribe::URDU_PROMPT));
        assert_eq!(got.text, "please send the invoice by Friday");
    }

    #[test]
    fn devanagari_is_repinned_to_urdu() {
        let fake = FakeVia::new(&[("यह हिंदी है", "hi"), ("یہ اردو ہے", "ur")], None);
        let got = run(&fake, "auto", "same", None);
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!(calls[1].1, "ur");
        assert_eq!(calls[1].2.as_deref(), Some(transcribe::URDU_PROMPT));
        assert_eq!(got.text, "یہ اردو ہے");
    }

    #[test]
    fn hindi_setting_keeps_devanagari_when_output_is_the_spoken_language() {
        let fake = FakeVia::new(&[("आज मौसम अच्छा है", "hi")], None);
        let got = run(&fake, "hi", "same", Some("Bolo"));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "no Urdu repin: {calls:?}");
        assert_eq!(calls[0].1, "hi");
        assert_eq!(calls[0].2.as_deref(), Some(transcribe::HINDI_PROMPT));
        assert_ne!(calls[0].2.as_deref(), Some(transcribe::URDU_PROMPT));
        assert!(calls[0].0.contains(transcribe::WHISPER_LARGE));
        assert_eq!(got.text, "आज मौसम अच्छा है");
        assert_eq!(got.language, "hi");
        assert_eq!(got.path, "transcribe");
    }

    #[test]
    fn hindi_setting_redoes_urdu_script_into_devanagari() {
        let fake = FakeVia::new(&[("آج موسم اچھا ہے", "ur"), ("आज मौसम अच्छा है", "hi")], None);
        let got = run(&fake, "hi", "same", None);
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!(calls[1].1, "hi");
        assert_eq!(calls[1].2.as_deref(), Some(transcribe::HINDI_PROMPT));
        assert_eq!(got.text, "आज मौसम अच्छा है");
        assert_eq!(got.language, "hi");
        assert!(got.path.ends_with("-repass"));
    }

    #[test]
    fn short_urdu_tag_is_heard_again_in_english_and_that_text_is_kept() {
        let fake = FakeVia::new(&[("ہاں", "ur"), ("yes", "en")], None);
        let got = run(&fake, "auto", "english", None);
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!(calls[1].1, "en");
        assert_eq!(got.text, "yes");
        assert!(got.path.contains("repass"));
    }

    #[test]
    fn unclear_urdu_keeps_the_online_words_when_the_english_pass_fails() {
        let mut fake = FakeVia::new(&[("ہاں جی", "ur")], None);
        fake.fail_translate = true;
        let got = run(&fake, "auto", "english", None);
        assert_eq!(got.text, "ہاں جی");
        assert!(!got.text.is_empty());
    }

    #[test]
    fn low_confidence_urdu_prefers_the_english_pass() {
        let mut fake = FakeVia::new(
            &[("آج meeting ہے اور ہم دفتر جائیں گے", "ur"), ("There is a meeting and we will go to the office", "en")],
            None,
        );
        fake.doubt = true;
        let got = run(&fake, "auto", "english", None);
        assert_eq!(got.text, "There is a meeting and we will go to the office");
        assert!(got.path.contains("repass"));
    }

    #[test]
    fn online_transcript_is_kept_when_translation_fails() {
        let mut fake = FakeVia::new(&[("आज मौसम अच्छा है", "hi")], None);
        fake.fail_translate = true;
        let got = run(&fake, "auto", "english", None);
        assert_eq!(got.text, "आज मौसम अच्छा है");
        assert_eq!(got.path, "transcribe");
        assert!(!got.text.is_empty());
    }

    #[test]
    fn hindi_with_english_output_still_translates() {
        let fake = FakeVia::new(&[("आज मौसम अच्छा है", "hi")], None);
        let got = run(&fake, "hi", "english", Some("Bolo"));
        let calls = fake.calls.lock().unwrap();
        assert!(calls.iter().all(|call| call.0 == "translate"), "{calls:?}");
        assert_eq!(got.text, "translated audio");
        assert_eq!(got.path, "translate");
    }

    #[test]
    fn pinned_urdu_still_uses_the_urdu_prompt_not_hindi() {
        let fake = FakeVia::new(&[("سلام", "ur")], None);
        let got = run(&fake, "ur", "same", Some("vocab"));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0].2.as_deref(), Some(transcribe::URDU_PROMPT));
        assert_eq!(got.text, "سلام");
    }

    #[test]
    fn pinned_urdu_setting_uses_urdu_prompt_first() {
        let fake = FakeVia::new(&[("سلام", "ur")], None);
        run(&fake, "ur", "same", Some("vocab"));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls[0].2.as_deref(), Some(transcribe::URDU_PROMPT));
    }
}
