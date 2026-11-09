//! Local accuracy capture and scoring. Nothing here is uploaded.
//!
//! Debug mode writes one folder per take under the app config directory:
//! the microphone audio before gain, the 16 kHz audio after gain, and a
//! `trace.json` of the Whisper request (never the key) and the text after
//! each stage. The `bolo-score` bin reads those folders, or a scripted
//! recording set, and reports word error rate. It calls Groq only when
//! `--api` or `BOLO_SCORE_AUDIO=1` is set, and only with `GROQ_API_KEY`.

use crate::accuracy::{self, WerScore};
use crate::audio::{self, FinishedAudio};
use crate::builtin;
use crate::chunk;
use crate::isolate;
use crate::store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const CLIP_FULL_SCALE: f32 = 0.99;
/// `normalize_gain` clamps at 0.92. Samples sitting on that ceiling were hard-clipped.
const CLIP_AFTER_GAIN: f32 = 0.915;

static ARMED: AtomicBool = AtomicBool::new(false);
static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static RECORDER: Mutex<Option<audio::RecordingSession>> = Mutex::new(None);
static HELD: Mutex<Option<Held>> = Mutex::new(None);

thread_local! {
    static PENDING: std::cell::RefCell<Pending> = std::cell::RefCell::new(Pending::default());
}

#[derive(Default)]
struct Pending {
    calls: Vec<WhisperCall>,
    raw: Option<String>,
}

struct Session {
    id: String,
    device_name: String,
    chunks: BTreeMap<usize, ChunkRec>,
    errors: Vec<String>,
}

struct ChunkRec {
    captured: Vec<f32>,
    channels: u16,
    sample_rate: u32,
    elapsed_ms: u64,
    processed: Vec<f32>,
    raw: String,
    after: String,
    pinned_language: String,
    calls: Vec<WhisperCall>,
    error: Option<String>,
}

struct Held {
    audio: FinishedAudio,
    device: String,
    peak: f32,
    clipping_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhisperCall {
    pub role: String,
    pub model: String,
    pub language: String,
    pub prompt: Option<String>,
    pub detected_language: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkTrace {
    pub index: u32,
    pub sample_rate: u32,
    pub channels: u16,
    pub captured_ms: u64,
    pub processed_ms: u64,
    pub trimmed_silence_ms: u64,
    pub peak: f32,
    pub clipping_pct: f32,
    pub processed_peak: f32,
    pub processed_clipping_pct: f32,
    pub hard_cut: bool,
    pub raw_whisper: String,
    pub after_redecode: String,
    pub detected_language: String,
    pub pinned_language: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageTexts {
    pub raw_whisper: String,
    pub after_redecode_translate: String,
    pub after_ai_cleanup: String,
    pub after_learned_spellings: String,
    pub after_grammar: String,
    pub final_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    pub id: String,
    pub created_unix_ms: u64,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub hold_ms: u64,
    pub trimmed_silence_ms: u64,
    pub peak: f32,
    pub clipping_pct: f32,
    pub processed_peak: f32,
    pub processed_clipping_pct: f32,
    pub hard_cut: bool,
    pub chunk_count: u32,
    pub whisper: Vec<WhisperCall>,
    /// Language on the first Whisper response, before a re-pin.
    pub detected_language: String,
    pub pinned_language: String,
    pub stages: StageTexts,
    pub grammar_ran: bool,
    pub cleanup_why: String,
    pub latency_ms: u64,
    pub encode_ms: u64,
    pub remote_ms: u64,
    pub cleanup_ms: u64,
    pub paste_ms: u64,
    pub errors: Vec<String>,
    pub insert_method: Option<String>,
    /// False when typing could not be read back. Clipboard paste can be true.
    pub insert_verified: Option<bool>,
}

#[derive(Default)]
pub struct Finish {
    pub hold_ms: u64,
    pub latency_ms: u64,
    pub encode_ms: u64,
    pub remote_ms: u64,
    pub cleanup_ms: u64,
    pub paste_ms: u64,
    pub cleanup_why: String,
    pub after_cleanup: Option<String>,
    pub learned: Option<String>,
    pub grammar: Option<String>,
    pub grammar_ran: bool,
    pub final_text: String,
    pub errors: Vec<String>,
    pub insert_method: Option<String>,
    pub insert_verified: Option<bool>,
}

impl Finish {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            errors: vec![clean(message.into())],
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptLine {
    pub id: &'static str,
    pub kind: &'static str,
    pub expected_language: &'static str,
    pub cue: &'static str,
    pub say: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldInfo {
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub hold_ms: u64,
    pub peak: f32,
    pub clipping_pct: f32,
    pub silent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageScore {
    pub stage: String,
    pub wer: Option<f64>,
    pub edits: usize,
    pub first_dropped: bool,
    pub last_dropped: bool,
    pub truncated: bool,
    /// Errors against the reference that the previous stage did not have.
    pub introduced: Vec<String>,
    /// Words that changed relative to the previous stage's text.
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemReport {
    pub id: String,
    pub kind: String,
    pub reference: Option<String>,
    pub detected_language: String,
    pub expected_language: Option<String>,
    pub language_mistag: bool,
    pub hard_cut: bool,
    pub truncated: bool,
    pub first_dropped: bool,
    pub last_dropped: bool,
    pub insert_method: Option<String>,
    pub insert_verified: Option<bool>,
    pub errors: Vec<String>,
    pub stages: Vec<StageScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub mode: String,
    pub items: Vec<ItemReport>,
    pub mean_wer_final: Option<f64>,
    pub first_word_drops: usize,
    pub last_word_drops: usize,
    pub truncations: usize,
    pub language_mistags: usize,
    pub hard_cuts: usize,
    pub unverified_inserts: usize,
    pub notes: Vec<String>,
}

fn clean(text: String) -> String {
    builtin::redact_secrets(&text)
}

fn session_lock() -> std::sync::MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|err| err.into_inner())
}

pub fn armed() -> bool {
    ARMED.load(Ordering::Relaxed)
}

pub fn arm(id: &str) {
    clear_pending();
    *session_lock() = Some(Session {
        id: id.to_string(),
        device_name: String::new(),
        chunks: BTreeMap::new(),
        errors: Vec::new(),
    });
    ARMED.store(true, Ordering::SeqCst);
}

pub fn disarm() {
    ARMED.store(false, Ordering::SeqCst);
    *session_lock() = None;
    clear_pending();
}

pub fn set_device(name: &str) {
    if !armed() {
        return;
    }
    if let Some(session) = session_lock().as_mut() {
        session.device_name = name.to_string();
    }
}

fn clear_pending() {
    PENDING.with(|pending| *pending.borrow_mut() = Pending::default());
}

/// One Whisper HTTP call. The key is not a parameter and is never stored.
pub fn note_call(
    role: &str,
    model: &str,
    language: &str,
    prompt: Option<&str>,
    detected: &str,
    text: &str,
) {
    if !armed() {
        return;
    }
    let call = WhisperCall {
        role: role.to_string(),
        model: clean(model.to_string()),
        language: clean(language.to_string()),
        prompt: prompt.map(|prompt| clean(prompt.to_string())),
        detected_language: clean(detected.to_string()),
        text: clean(text.to_string()),
    };
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        if role == "raw" && pending.raw.is_none() {
            pending.raw = Some(call.text.clone());
        }
        pending.calls.push(call);
    });
}

pub fn drain_calls() -> (Vec<WhisperCall>, Option<String>) {
    PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        (std::mem::take(&mut pending.calls), pending.raw.take())
    })
}

pub fn note_audio(index: usize, audio: &FinishedAudio, processed: &[f32]) {
    if !armed() {
        return;
    }
    let mut guard = session_lock();
    let Some(session) = guard.as_mut() else {
        return;
    };
    let rec = session.chunks.entry(index).or_insert_with(empty_chunk);
    rec.captured = audio.interleaved.clone();
    rec.channels = audio.channels.max(1);
    rec.sample_rate = audio.sample_rate;
    rec.elapsed_ms = audio.elapsed_ms;
    rec.processed = processed.to_vec();
}

pub fn note_chunk_error(index: usize, message: &str) {
    if !armed() {
        return;
    }
    let message = clean(message.to_string());
    let mut guard = session_lock();
    let Some(session) = guard.as_mut() else {
        return;
    };
    session.errors.push(message.clone());
    let rec = session.chunks.entry(index).or_insert_with(empty_chunk);
    rec.error = Some(message);
}

pub fn attach_text(
    index: usize,
    calls: Vec<WhisperCall>,
    raw: Option<String>,
    pinned_language: &str,
    after: &str,
) {
    if !armed() {
        return;
    }
    let mut guard = session_lock();
    let Some(session) = guard.as_mut() else {
        return;
    };
    let rec = session.chunks.entry(index).or_insert_with(empty_chunk);
    rec.calls.extend(calls);
    if let Some(raw) = raw {
        rec.raw = raw;
    }
    rec.after = clean(after.to_string());
    rec.pinned_language = clean(pinned_language.to_string());
}

fn empty_chunk() -> ChunkRec {
    ChunkRec {
        captured: Vec::new(),
        channels: 1,
        sample_rate: 0,
        elapsed_ms: 0,
        processed: Vec::new(),
        raw: String::new(),
        after: String::new(),
        pinned_language: String::new(),
        calls: Vec::new(),
        error: None,
    }
}

pub fn debug_dir(config: &Path) -> PathBuf {
    config.join("accuracy-debug")
}

pub fn script_dir(config: &Path) -> PathBuf {
    config.join("accuracy-script")
}

pub fn commit(config: &Path, finish: Finish) -> Result<(), String> {
    isolate::refuse_real_config(config)?;
    let Some(session) = take_session() else {
        return Ok(());
    };
    let trace = build_trace(&session, &finish);
    write_bundle(&debug_dir(config).join(&trace.id), &session, &trace)
}

fn take_session() -> Option<Session> {
    ARMED.store(false, Ordering::SeqCst);
    let session = session_lock().take();
    clear_pending();
    session
}

fn build_trace(session: &Session, finish: &Finish) -> Trace {
    let mut chunks = Vec::new();
    let mut whisper = Vec::new();
    let mut captured_samples = 0usize;
    let mut captured_clipped = 0.0f32;
    let mut processed_samples = 0usize;
    let mut processed_clipped = 0.0f32;
    let mut peak = 0.0f32;
    let mut processed_peak = 0.0f32;
    let mut trimmed = 0u64;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    for (index, rec) in &session.chunks {
        let frames = rec.captured.len() / usize::from(rec.channels.max(1));
        let captured_ms = duration_ms(frames, rec.sample_rate);
        let processed_ms = duration_ms(rec.processed.len(), audio::SPEECH_RATE);
        let (chunk_peak, chunk_clip) = levels(&rec.captured, CLIP_FULL_SCALE);
        let (out_peak, out_clip) = levels(&rec.processed, CLIP_AFTER_GAIN);
        peak = peak.max(chunk_peak);
        processed_peak = processed_peak.max(out_peak);
        captured_clipped += chunk_clip / 100.0 * rec.captured.len() as f32;
        processed_clipped += out_clip / 100.0 * rec.processed.len() as f32;
        captured_samples += rec.captured.len();
        processed_samples += rec.processed.len();
        trimmed += captured_ms.saturating_sub(processed_ms);
        if sample_rate == 0 {
            sample_rate = rec.sample_rate;
            channels = rec.channels;
        }
        let detected = rec
            .calls
            .iter()
            .find(|call| call.role == "raw" && !call.detected_language.is_empty())
            .map(|call| call.detected_language.clone())
            .unwrap_or_else(|| rec.pinned_language.clone());
        whisper.extend(rec.calls.iter().cloned());
        chunks.push(ChunkTrace {
            index: *index as u32,
            sample_rate: rec.sample_rate,
            channels: rec.channels,
            captured_ms,
            processed_ms,
            trimmed_silence_ms: captured_ms.saturating_sub(processed_ms),
            peak: chunk_peak,
            clipping_pct: chunk_clip,
            processed_peak: out_peak,
            processed_clipping_pct: out_clip,
            hard_cut: chunk_hard_cut(rec.elapsed_ms, captured_ms),
            raw_whisper: rec.raw.clone(),
            after_redecode: rec.after.clone(),
            detected_language: detected,
            pinned_language: rec.pinned_language.clone(),
            error: rec.error.clone(),
        });
    }
    let raw = chunk::stitch(chunks.iter().map(|chunk| chunk.raw_whisper.as_str()));
    let redecode = chunk::stitch(chunks.iter().map(|chunk| chunk.after_redecode.as_str()));
    let cleanup = finish
        .after_cleanup
        .clone()
        .unwrap_or_else(|| redecode.clone());
    let learned = finish.learned.clone().unwrap_or_else(|| cleanup.clone());
    let grammar = finish.grammar.clone().unwrap_or_else(|| learned.clone());
    let final_text = if finish.final_text.is_empty() && finish.errors.is_empty() {
        grammar.clone()
    } else {
        finish.final_text.clone()
    };
    let detected_language = chunks
        .iter()
        .find_map(|chunk| {
            let language = chunk.detected_language.trim();
            (!language.is_empty()).then(|| language.to_string())
        })
        .unwrap_or_default();
    let pinned_language = chunks
        .iter()
        .rev()
        .find_map(|chunk| {
            let language = chunk.pinned_language.trim();
            (!language.is_empty()).then(|| language.to_string())
        })
        .unwrap_or_else(|| detected_language.clone());
    let mut errors = session.errors.clone();
    errors.extend(finish.errors.iter().cloned());
    errors.retain(|error| !error.is_empty());
    Trace {
        id: session.id.clone(),
        created_unix_ms: store::now_ms(),
        device_name: session.device_name.clone(),
        sample_rate,
        channels,
        hold_ms: finish.hold_ms,
        trimmed_silence_ms: trimmed,
        peak,
        clipping_pct: pct(captured_clipped, captured_samples),
        processed_peak,
        processed_clipping_pct: pct(processed_clipped, processed_samples),
        hard_cut: chunks.iter().any(|chunk| chunk.hard_cut),
        chunk_count: chunks.len() as u32,
        whisper,
        detected_language,
        pinned_language,
        stages: StageTexts {
            raw_whisper: raw,
            after_redecode_translate: redecode,
            after_ai_cleanup: cleanup,
            after_learned_spellings: learned,
            after_grammar: grammar,
            final_text: clean(final_text),
        },
        grammar_ran: finish.grammar_ran,
        cleanup_why: finish.cleanup_why.clone(),
        latency_ms: finish.latency_ms,
        encode_ms: finish.encode_ms,
        remote_ms: finish.remote_ms,
        cleanup_ms: finish.cleanup_ms,
        paste_ms: finish.paste_ms,
        errors: errors.into_iter().map(clean).collect(),
        insert_method: finish.insert_method.clone(),
        insert_verified: finish.insert_verified,
    }
}

/// A forced 30 s cut. The live tail stores the whole hold in `elapsed_ms`,
/// so that clock counts only when it matches the audio still in the buffer.
fn chunk_hard_cut(elapsed_ms: u64, captured_ms: u64) -> bool {
    if captured_ms >= 29_500 {
        return true;
    }
    elapsed_ms >= u64::from(chunk::CHUNK_FORCE_MS) && captured_ms + 1_500 >= elapsed_ms
}

fn pct(clipped_samples: f32, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        clipped_samples * 100.0 / total as f32
    }
}

fn write_bundle(dir: &Path, session: &Session, trace: &Trace) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let same_format = session.chunks.values().all(|chunk| {
        chunk.sample_rate == trace.sample_rate
            && chunk.channels == trace.channels
            && chunk.sample_rate > 0
    });
    if same_format && trace.sample_rate > 0 {
        let mut captured = Vec::new();
        let mut processed = Vec::new();
        for rec in session.chunks.values() {
            captured.extend_from_slice(&rec.captured);
            processed.extend_from_slice(&rec.processed);
        }
        write_pcm(
            &dir.join("captured.wav"),
            &captured,
            trace.channels.max(1),
            trace.sample_rate,
        )?;
        write_pcm(
            &dir.join("processed.wav"),
            &processed,
            1,
            audio::SPEECH_RATE,
        )?;
    }
    if session.chunks.len() > 1 {
        for (index, rec) in &session.chunks {
            write_pcm(
                &dir.join(format!("chunk-{index}-captured.wav")),
                &rec.captured,
                rec.channels.max(1),
                rec.sample_rate,
            )?;
            write_pcm(
                &dir.join(format!("chunk-{index}-processed.wav")),
                &rec.processed,
                1,
                audio::SPEECH_RATE,
            )?;
        }
    }
    // Chunk durations sit beside the stitched stages so a 30 s cut is visible.
    let mut value = serde_json::to_value(trace).map_err(|err| err.to_string())?;
    if let Some(map) = value.as_object_mut() {
        let chunk_value = session
            .chunks
            .iter()
            .map(|(index, rec)| {
                let frames = rec.captured.len() / usize::from(rec.channels.max(1));
                serde_json::json!({
                    "index": index,
                    "capturedMs": duration_ms(frames, rec.sample_rate),
                    "processedMs": duration_ms(rec.processed.len(), audio::SPEECH_RATE),
                    "hardCut": chunk_hard_cut(rec.elapsed_ms, duration_ms(frames, rec.sample_rate)),
                    "rawWhisper": rec.raw,
                    "afterRedecode": rec.after,
                    "detectedLanguage": rec.calls.iter().find(|call| call.role == "raw").map(|call| call.detected_language.as_str()).unwrap_or(""),
                    "error": rec.error,
                })
            })
            .collect::<Vec<_>>();
        map.insert("chunks".into(), serde_json::Value::Array(chunk_value));
    }
    let bytes = serde_json::to_vec_pretty(&value).map_err(|err| err.to_string())?;
    fs::write(dir.join("trace.json"), bytes).map_err(|err| err.to_string())?;
    Ok(())
}

fn duration_ms(frames: usize, rate: u32) -> u64 {
    if rate == 0 {
        0
    } else {
        frames as u64 * 1000 / u64::from(rate)
    }
}

fn levels(samples: &[f32], clip_at: f32) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let mut peak = 0.0f32;
    let mut clipped = 0usize;
    for sample in samples {
        let abs = sample.abs();
        if abs > peak {
            peak = abs;
        }
        if abs >= clip_at {
            clipped += 1;
        }
    }
    (peak, clipped as f32 * 100.0 / samples.len() as f32)
}

pub fn write_pcm(path: &Path, samples: &[f32], channels: u16, rate: u32) -> Result<(), String> {
    if samples.is_empty() || rate == 0 || channels == 0 {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|err| err.to_string())?;
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(value).map_err(|err| err.to_string())?;
    }
    writer.finalize().map_err(|err| err.to_string())
}

pub fn read_wav(path: &Path) -> Result<(Vec<f32>, u32, u16), String> {
    let mut reader = hound::WavReader::open(path).map_err(|err| err.to_string())?;
    let spec = reader.spec();
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?,
        hound::SampleFormat::Int => {
            let max = (1i64 << spec.bits_per_sample.saturating_sub(1).min(31)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| {
                    sample
                        .map(|value| value as f32 / max)
                        .map_err(|err| err.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok((samples, spec.sample_rate, spec.channels))
}

pub fn open_folder(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| err.to_string())?;
    open::that(path).map_err(|err| err.to_string())
}

pub fn clear_folder(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|err| err.to_string())?;
    }
    fs::create_dir_all(path).map_err(|err| err.to_string())
}

pub fn script() -> &'static [ScriptLine] {
    SCRIPT
}

const SCRIPT: &[ScriptLine] = &[
    line("01-bolo", "names", "en", "Say this sentence.", "Open Bolo and start dictating."),
    line("02-taimoor", "names", "en", "Say the domain as one address.", "Send it to taimoorhussain.com."),
    line("03-beta", "names", "en", "Say the domain as one address.", "The site is thebetawebsite.com."),
    line("04-plesk", "names", "en", "", "Log in to Plesk and restart the site."),
    line("05-whatsapp", "names", "en", "", "Message me on WhatsApp when it is done."),
    line("06-mysql", "names", "en", "", "The password is stored in MySQL."),
    line("07-kingdom", "names", "en", "", "Kingdom Vision ships the update tomorrow."),
    line("08-travel-id", "grammar", "en", "Say the letters I D, not the contraction I'd.", "My travel ID is on the desk."),
    line("09-does-it-have", "grammar", "en", "", "Does it have the latest build?"),
    line("10-if-i-were", "grammar", "en", "", "If I were you I would wait."),
    line("11-let-it-go", "grammar", "en", "", "Let it go and ship the fix."),
    line("12-wrote", "grammar", "en", "Say wrote, as in writing an email.", "I wrote the email yesterday."),
    line("13-roots", "grammar", "en", "Say roots, as in the roots of a tree.", "The roots of the tree are deep."),
    line("14-stop", "short", "en", "Just this one word.", "Stop."),
    line("15-copy", "short", "en", "", "Copy that."),
    line("16-undo", "short", "en", "", "Undo."),
    line("17-mixed-email", "mixed", "mixed", "Urdu and English in one sentence.", "Bolo mein yeh email bhej do please."),
    line("18-mixed-meeting", "mixed", "mixed", "", "Meeting kal hai at three on Zoom."),
    line("19-mixed-server", "mixed", "mixed", "", "Please check the server and batao kya hua."),
    line("20-fast", "fast", "en", "Say this one quickly.", "Ship the notes to Priya before Thursday."),
    line("21-mumble", "fast", "en", "Mumble the number on purpose.", "The invoice number is four eight two one."),
    line("22-pause", "pause", "en", "Pause for about two seconds after the word passed.", "The build passed and then the site went down."),
    line("23-pause-and", "pause", "en", "Wait two seconds after the word and.", "Open the dashboard and tell me the error."),
    line(
        "24-long",
        "long",
        "en",
        "Read the whole paragraph at a natural pace and do not stop early. It should run past forty-five seconds. Start on the first word and finish on the last word.",
        "Alpha. Bolo is updating the site at thebetawebsite.com and the mailbox at taimoorhussain.com. Kingdom Vision asked us to open Plesk, restart MySQL, and send a WhatsApp note when the page loads for the first time. Yesterday I wrote the release notes, checked the travel ID on the booking, and asked does it have the latest build. If I were you I would let it go only after the backup finishes and the health check is green. The roots of the problem are in the old config, not in the new theme, and the deploy log shows the stall just after lunch. Please talk through the people who will use this tomorrow morning, the database backup you took, the error that showed up when the worker restarted, and the note you still owe the support queue. Keep going until this whole paragraph is finished, and end on the last word omega.",
    ),
    line("25-first-word", "edges", "en", "The first word matters. Start cleanly.", "Pineapple belongs on the pizza."),
    line("26-last-word", "edges", "en", "The last word matters. Do not trail off.", "Please remember the last word apricot."),
    line("27-hostinger", "names", "en", "", "Hostinger hosts the beta website."),
    line("28-easyssh", "names", "en", "", "Open EasySSH and connect to the server."),
    line("29-there", "grammar", "en", "", "Their cat sat over there."),
    line("30-right-now", "short", "en", "", "Send it right now."),
];

const fn line(
    id: &'static str,
    kind: &'static str,
    expected_language: &'static str,
    cue: &'static str,
    say: &'static str,
) -> ScriptLine {
    ScriptLine {
        id,
        kind,
        expected_language,
        cue,
        say,
    }
}

pub fn record_start(preference: Option<&str>) -> Result<(), String> {
    let mut slot = RECORDER.lock().unwrap_or_else(|err| err.into_inner());
    if slot.is_some() {
        return Err("Already recording. Stop this take first.".into());
    }
    *HELD.lock().unwrap_or_else(|err| err.into_inner()) = None;
    let session = audio::start_recording(preference).map_err(|err| err.to_string())?;
    *slot = Some(session);
    Ok(())
}

pub fn record_stop() -> Result<HeldInfo, String> {
    let session = RECORDER
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
        .ok_or("Not recording.")?;
    let device = session.active_name.clone();
    let audio = session.finish()?;
    let (peak, clipping_pct) = levels(&audio.interleaved, CLIP_FULL_SCALE);
    let info = HeldInfo {
        device_name: device.clone(),
        sample_rate: audio.sample_rate,
        channels: audio.channels,
        hold_ms: audio.elapsed_ms,
        peak,
        clipping_pct,
        silent: peak < audio::SILENT_PEAK,
    };
    *HELD.lock().unwrap_or_else(|err| err.into_inner()) = Some(Held {
        audio,
        device,
        peak,
        clipping_pct,
    });
    Ok(info)
}

pub fn record_discard() {
    if let Some(session) = RECORDER
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        let _ = session.finish();
    }
    *HELD.lock().unwrap_or_else(|err| err.into_inner()) = None;
}

pub fn save_script(config: &Path, id: &str) -> Result<String, String> {
    isolate::refuse_real_config(config)?;
    let line = script()
        .iter()
        .find(|line| line.id == id)
        .ok_or_else(|| format!("Unknown sentence {id}."))?;
    let held = HELD
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
        .ok_or("Record the sentence first.")?;
    let dir = script_dir(config).join(line.id);
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    write_pcm(
        &dir.join("audio.wav"),
        &held.audio.interleaved,
        held.audio.channels.max(1),
        held.audio.sample_rate,
    )?;
    fs::write(dir.join("reference.txt"), line.say).map_err(|err| err.to_string())?;
    let meta = serde_json::json!({
        "id": line.id,
        "kind": line.kind,
        "expectedLanguage": line.expected_language,
        "cue": line.cue,
        "deviceName": held.device,
        "sampleRate": held.audio.sample_rate,
        "channels": held.audio.channels,
        "holdMs": held.audio.elapsed_ms,
        "peak": held.peak,
        "clippingPct": held.clipping_pct,
    });
    fs::write(
        dir.join("meta.json"),
        serde_json::to_vec_pretty(&meta).map_err(|err| err.to_string())?,
    )
    .map_err(|err| err.to_string())?;
    Ok(dir.display().to_string())
}

pub fn saved_script_ids(config: &Path) -> Vec<String> {
    let dir = script_dir(config);
    let Ok(read) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.join("audio.wav").is_file() && path.join("reference.txt").is_file() {
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                ids.push(name.to_string());
            }
        }
    }
    ids.sort();
    ids
}

pub fn score_offline(folder: &Path) -> Result<Report, String> {
    let found = find_dirs(folder, "trace.json");
    let mut items = Vec::new();
    for dir in found {
        let trace: Trace = serde_json::from_slice(
            &fs::read(dir.join("trace.json")).map_err(|err| err.to_string())?,
        )
        .map_err(|err| err.to_string())?;
        let reference = read_text(&dir.join("reference.txt"));
        let expected = meta_language(&dir);
        items.push(score_trace(
            &trace,
            reference.as_deref(),
            expected.as_deref(),
            &dir,
        ));
    }
    let mut notes = vec![
        "Offline re-score. Groq was not called.".into(),
        "Word error rate is empty when a take has no reference.txt beside trace.json.".into(),
    ];
    if items.is_empty() {
        notes.push("No trace.json files in this folder. Debug captures live in the app's accuracy-debug folder. Pass --api to score WAV recordings.".into());
    }
    Ok(summarize("offline", items, notes))
}

pub fn score_online_blocking(folder: &Path) -> Result<Report, String> {
    let opted = std::env::args().any(|arg| arg == "--api")
        || std::env::var("BOLO_SCORE_AUDIO").ok().as_deref() == Some("1");
    if !opted {
        return Err("Refusing to call Groq. Re-run with --api or BOLO_SCORE_AUDIO=1.".into());
    }
    let key = std::env::var("GROQ_API_KEY").map_err(|_| "GROQ_API_KEY is not set.".to_string())?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("GROQ_API_KEY is empty.".into());
    }
    let report = tauri::async_runtime::block_on(score_online(folder, &key));
    let mut key = key;
    key.zeroize_local();
    report
}

trait Wipe {
    fn zeroize_local(&mut self);
}

impl Wipe for String {
    fn zeroize_local(&mut self) {
        // zeroize is already a dependency; call it without importing the trait at the top
        // so a future rename of the method stays local.
        use zeroize::Zeroize;
        self.zeroize();
    }
}

async fn score_online(folder: &Path, api_key: &str) -> Result<Report, String> {
    let dirs = find_recording_dirs(folder);
    let mut items = Vec::new();
    let mut notes = vec![
        "Online score. Each WAV was run through speech_with_groq, cleanup, learned spellings, and grammar.".into(),
        "The Groq key came from GROQ_API_KEY and is not in this report.".into(),
        "Prompt words are the built-in seeds only. Personal memory is not loaded.".into(),
        "Tone is plain, language is auto, output is English, model is the default Whisper model.".into(),
    ];
    if dirs.is_empty() {
        notes.push("No audio.wav or captured.wav with a reference.txt. Record the script in Settings, or add reference.txt next to a debug capture.".into());
    }
    for dir in dirs {
        let wav = dir.join("audio.wav");
        let wav = if wav.is_file() {
            wav
        } else {
            dir.join("captured.wav")
        };
        let reference = read_text(&dir.join("reference.txt")).unwrap_or_default();
        if reference.trim().is_empty() {
            notes.push(format!(
                "{} has audio but no reference.txt, so it was skipped.",
                dir.display()
            ));
            continue;
        }
        let started = Instant::now();
        match evaluate_wav(&wav, api_key).await {
            Ok(mut trace) => {
                trace.latency_ms = started.elapsed().as_millis() as u64;
                if trace.id.is_empty() {
                    trace.id = dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("take")
                        .to_string();
                }
                let expected = meta_language(&dir);
                items.push(score_trace(
                    &trace,
                    Some(&reference),
                    expected.as_deref(),
                    &dir,
                ));
            }
            Err(err) => {
                notes.push(format!("{}: {}", dir.display(), clean(err)));
            }
        }
    }
    Ok(summarize("online", items, notes))
}

async fn evaluate_wav(path: &Path, api_key: &str) -> Result<Trace, String> {
    let (samples, rate, channels) = read_wav(path)?;
    let pieces = split_for_score(&samples, rate, channels);
    let id = path
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("take");
    arm(id);
    set_device("file");
    let settings = store::Settings::default();
    let client = crate::transcribe::http_client();
    for (index, piece) in pieces.iter().enumerate() {
        let _ = drain_calls();
        let audio = FinishedAudio {
            interleaved: piece.samples.clone(),
            channels,
            sample_rate: rate,
            elapsed_ms: piece.elapsed_ms,
            noise_rms: 0.0,
        };
        let (encoded, processed) = match audio::prepare_speech_parts(&audio) {
            Ok(pair) => pair,
            Err(err) => {
                note_audio(index, &audio, &[]);
                note_chunk_error(index, &err.to_string());
                continue;
            }
        };
        note_audio(index, &audio, &processed);
        let words = crate::pipeline::prompt_words(&settings.dictionary, &settings.language);
        let prompt = if crate::transcribe::brief_audio(encoded.speech_secs) {
            Some(crate::transcribe::whisper_prompt_brief())
        } else {
            crate::transcribe::whisper_prompt_with(&words, &[])
        };
        match crate::pipeline::speech_with_groq(
            client,
            api_key,
            encoded.bytes,
            &settings.model,
            &settings.language,
            &settings.output,
            prompt.as_deref(),
            &settings.cleanup_model,
            false,
        )
        .await
        {
            Ok(recognized) => {
                let text =
                    crate::transcribe::guard_short_take(&recognized.text, encoded.speech_secs);
                let (calls, raw) = drain_calls();
                attach_text(index, calls, raw, &recognized.language, &text);
            }
            Err(err) => {
                let (calls, raw) = drain_calls();
                attach_text(index, calls, raw, "", "");
                note_chunk_error(index, &err.to_string());
            }
        }
    }
    let Some(session) = take_session() else {
        return Err("scoring session was not armed".into());
    };
    let redecode = chunk::stitch(session.chunks.values().map(|chunk| chunk.after.as_str()));
    let cleaned = crate::cleanup::clean_transcript(
        client,
        api_key,
        &redecode,
        settings.cleanup_enabled,
        crate::cleanup::Tone::Plain,
        settings.verbatim,
        &settings.cleanup_model,
        false,
    )
    .await;
    let words = crate::vocab::protected(&[]);
    let local = accuracy::local_stages(&cleaned.text, &[], &words, !settings.verbatim);
    let finish = Finish {
        after_cleanup: Some(cleaned.text),
        learned: Some(local.learned),
        grammar: Some(local.grammar.clone()),
        grammar_ran: local.grammar_ran,
        final_text: local.grammar,
        cleanup_why: cleaned.why,
        ..Finish::default()
    };
    Ok(build_trace(&session, &finish))
}

struct Piece {
    samples: Vec<f32>,
    elapsed_ms: u64,
}

/// Split a file the way a live take is cut: a pause after 20 s, or a hard cut at 30 s.
fn split_for_score(interleaved: &[f32], rate: u32, channels: u16) -> Vec<Piece> {
    let channels_n = usize::from(channels.max(1));
    let frame = (rate as usize).saturating_mul(20) / 1000 * channels_n;
    let total_frames = if channels_n == 0 {
        0
    } else {
        interleaved.len() / channels_n
    };
    let total_ms = duration_ms(total_frames, rate);
    if frame == 0 || total_ms < 20_000 {
        return vec![Piece {
            samples: interleaved.to_vec(),
            elapsed_ms: total_ms,
        }];
    }
    let mut cuts = vec![0usize];
    let mut pause_ms = 0u32;
    let mut buf_start = 0usize;
    let mut index = 0usize;
    while index + frame <= interleaved.len() {
        let energy = rms(&interleaved[index..index + frame]);
        pause_ms = audio::advance_pause_ms(pause_ms, energy, audio::speech_threshold(0.0), 20);
        let buffered_frames = (index + frame - buf_start) / channels_n;
        let buffered_ms = duration_ms(buffered_frames, rate) as u32;
        if chunk::should_cut(buffered_ms, pause_ms) {
            cuts.push(index + frame);
            let overlap = chunk::overlap_samples(rate, channels);
            buf_start = (index + frame).saturating_sub(overlap);
            pause_ms = 0;
        }
        index += frame;
    }
    if *cuts.last().unwrap_or(&0) < interleaved.len() {
        cuts.push(interleaved.len());
    }
    let mut pieces = Vec::new();
    for (nth, window) in cuts.windows(2).enumerate() {
        let mut start = window[0];
        let end = window[1];
        if nth > 0 {
            let overlap = chunk::overlap_samples(rate, channels);
            start = start.saturating_sub(overlap);
        }
        if start >= end {
            continue;
        }
        let samples = interleaved[start..end].to_vec();
        let frames = samples.len() / channels_n;
        pieces.push(Piece {
            elapsed_ms: duration_ms(frames, rate),
            samples,
        });
    }
    if pieces.is_empty() {
        pieces.push(Piece {
            samples: interleaved.to_vec(),
            elapsed_ms: total_ms,
        });
    }
    pieces
}

fn rms(chunk: &[f32]) -> f32 {
    if chunk.is_empty() {
        return 0.0;
    }
    (chunk.iter().map(|sample| sample * sample).sum::<f32>() / chunk.len() as f32).sqrt()
}

fn score_trace(
    trace: &Trace,
    reference: Option<&str>,
    expected_language: Option<&str>,
    dir: &Path,
) -> ItemReport {
    let stages = [
        ("raw_whisper", trace.stages.raw_whisper.as_str()),
        (
            "after_redecode_translate",
            trace.stages.after_redecode_translate.as_str(),
        ),
        ("after_ai_cleanup", trace.stages.after_ai_cleanup.as_str()),
        (
            "after_learned_spellings",
            trace.stages.after_learned_spellings.as_str(),
        ),
        ("after_grammar", trace.stages.after_grammar.as_str()),
        ("final", trace.stages.final_text.as_str()),
    ];
    let mut scored = Vec::new();
    let mut previous_errors: Vec<String> = Vec::new();
    let mut previous_text = String::new();
    let mut final_score = None;
    for (name, text) in stages {
        let score = reference.map(|reference| accuracy::wer_score(reference, text));
        let errors = score
            .as_ref()
            .map(|_| error_keys(reference.unwrap_or(""), text))
            .unwrap_or_default();
        let introduced = errors
            .iter()
            .filter(|error| !previous_errors.contains(error))
            .cloned()
            .collect();
        let changed = if previous_text.is_empty() && scored.is_empty() {
            Vec::new()
        } else {
            changes(&previous_text, text)
        };
        let truncated = score
            .as_ref()
            .is_some_and(|score| is_truncated(score, trace.hard_cut));
        if name == "final" {
            final_score = score;
        }
        scored.push(StageScore {
            stage: name.into(),
            wer: score.map(|score| score.wer),
            edits: score.map(|score| score.edits).unwrap_or(0),
            first_dropped: score.is_some_and(|score| score.first_dropped),
            last_dropped: score.is_some_and(|score| score.last_dropped),
            truncated,
            introduced,
            changed,
        });
        previous_errors = errors;
        previous_text = text.to_string();
    }
    let kind = meta_kind(dir).unwrap_or_else(|| "debug".into());
    let id = if trace.id.is_empty() {
        dir.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("take")
            .to_string()
    } else {
        trace.id.clone()
    };
    let final_stage = scored.last();
    ItemReport {
        id,
        kind,
        reference: reference.map(str::to_string),
        detected_language: trace.detected_language.clone(),
        expected_language: expected_language.map(str::to_string),
        language_mistag: expected_language
            .is_some_and(|expected| language_mistag(expected, &trace.detected_language)),
        hard_cut: trace.hard_cut,
        truncated: final_stage.is_some_and(|stage| stage.truncated),
        first_dropped: final_score.is_some_and(|score| score.first_dropped),
        last_dropped: final_score.is_some_and(|score| score.last_dropped),
        insert_method: trace.insert_method.clone(),
        insert_verified: trace.insert_verified,
        errors: trace.errors.clone(),
        stages: scored,
    }
}

fn is_truncated(score: &WerScore, hard_cut: bool) -> bool {
    if score.reference_words >= 8 && score.hypothesis_words * 5 < score.reference_words * 3 {
        return true;
    }
    hard_cut
        && score.reference_words >= 12
        && score.last_dropped
        && score.hypothesis_words + 4 < score.reference_words
}

fn language_mistag(expected: &str, detected: &str) -> bool {
    let detected = detected.trim().to_ascii_lowercase();
    if detected.is_empty() || detected == "offline" {
        return false;
    }
    match expected {
        "en" => detected != "en",
        "mixed" => !matches!(detected.as_str(), "en" | "ur" | "hi"),
        "ur" => !matches!(detected.as_str(), "ur" | "hi"),
        _ => false,
    }
}

fn error_keys(reference: &str, hypothesis: &str) -> Vec<String> {
    alignment_notes(&accuracy::tokens(reference), &accuracy::tokens(hypothesis))
}

fn changes(previous: &str, next: &str) -> Vec<String> {
    alignment_notes(&accuracy::tokens(previous), &accuracy::tokens(next))
        .into_iter()
        .filter(|note| !note.starts_with("eq "))
        .collect()
}

fn alignment_notes(reference: &[String], hypothesis: &[String]) -> Vec<String> {
    let n = reference.len();
    let m = hypothesis.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        dp[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(reference[i - 1] != hypothesis[j - 1]);
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    let mut notes = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0
            && j > 0
            && dp[i][j] == dp[i - 1][j - 1] + usize::from(reference[i - 1] != hypothesis[j - 1])
        {
            if reference[i - 1] != hypothesis[j - 1] {
                notes.push(format!("sub {} -> {}", reference[i - 1], hypothesis[j - 1]));
            }
            i -= 1;
            j -= 1;
        } else if i > 0 && dp[i][j] == dp[i - 1][j] + 1 {
            notes.push(format!("drop {}", reference[i - 1]));
            i -= 1;
        } else if j > 0 {
            notes.push(format!("add {}", hypothesis[j - 1]));
            j -= 1;
        } else {
            break;
        }
    }
    notes.reverse();
    notes
}

fn summarize(mode: &str, items: Vec<ItemReport>, notes: Vec<String>) -> Report {
    let scored: Vec<f64> = items
        .iter()
        .filter_map(|item| item.stages.iter().find(|stage| stage.stage == "final")?.wer)
        .collect();
    let mean = if scored.is_empty() {
        None
    } else {
        Some(scored.iter().sum::<f64>() / scored.len() as f64)
    };
    Report {
        first_word_drops: items.iter().filter(|item| item.first_dropped).count(),
        last_word_drops: items.iter().filter(|item| item.last_dropped).count(),
        truncations: items.iter().filter(|item| item.truncated).count(),
        language_mistags: items.iter().filter(|item| item.language_mistag).count(),
        hard_cuts: items.iter().filter(|item| item.hard_cut).count(),
        unverified_inserts: items
            .iter()
            .filter(|item| {
                item.insert_method.as_deref() == Some("typed")
                    && item.insert_verified == Some(false)
            })
            .count(),
        mode: mode.into(),
        items,
        mean_wer_final: mean,
        notes,
    }
}

fn find_dirs(folder: &Path, marker: &str) -> Vec<PathBuf> {
    if folder.join(marker).is_file() {
        return vec![folder.to_path_buf()];
    }
    let Ok(read) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = read
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(marker).is_file())
        .collect();
    dirs.sort();
    dirs
}

fn find_recording_dirs(folder: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let consider = |path: &Path, dirs: &mut Vec<PathBuf>| {
        let audio = path.join("audio.wav").is_file() || path.join("captured.wav").is_file();
        if audio && path.join("reference.txt").is_file() {
            dirs.push(path.to_path_buf());
        }
    };
    consider(folder, &mut dirs);
    if let Ok(read) = fs::read_dir(folder) {
        for entry in read.flatten() {
            consider(&entry.path(), &mut dirs);
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn meta_language(dir: &Path) -> Option<String> {
    meta_field(dir, "expectedLanguage")
}

fn meta_kind(dir: &Path) -> Option<String> {
    meta_field(dir, "kind")
}

fn meta_field(dir: &Path, field: &str) -> Option<String> {
    let bytes = fs::read(dir.join("meta.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get(field)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

pub fn write_reports(dir: &Path, report: &Report) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let json = serde_json::to_vec_pretty(report).map_err(|err| err.to_string())?;
    fs::write(dir.join("accuracy-report.json"), json).map_err(|err| err.to_string())?;
    fs::write(dir.join("accuracy-report.md"), render_markdown(report))
        .map_err(|err| err.to_string())
}

fn render_markdown(report: &Report) -> String {
    let mut out = String::new();
    out.push_str("# Bolo accuracy report\n\n");
    out.push_str(&format!("Mode: {}\n\n", report.mode));
    if let Some(mean) = report.mean_wer_final {
        out.push_str(&format!("Mean final word error rate: {mean:.3}\n\n"));
    }
    out.push_str(&format!(
        "First-word drops: {}. Last-word drops: {}. Truncations: {}. Language mis-tags: {}. Hard cuts: {}. Unverified typed inserts: {}.\n\n",
        report.first_word_drops, report.last_word_drops, report.truncations, report.language_mistags, report.hard_cuts, report.unverified_inserts
    ));
    for note in &report.notes {
        out.push_str(&format!("- {note}\n"));
    }
    if !report.notes.is_empty() {
        out.push('\n');
    }
    out.push_str("| Item | Raw | Re-decode | Cleanup | Learned | Grammar | Final | First | Last | Cut | Lang |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for item in &report.items {
        let cell = |name: &str| -> String {
            item.stages
                .iter()
                .find(|stage| stage.stage == name)
                .and_then(|stage| stage.wer)
                .map(|wer| format!("{wer:.2}"))
                .unwrap_or_else(|| "—".into())
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            item.id,
            cell("raw_whisper"),
            cell("after_redecode_translate"),
            cell("after_ai_cleanup"),
            cell("after_learned_spellings"),
            cell("after_grammar"),
            cell("final"),
            yes(item.first_dropped),
            yes(item.last_dropped),
            yes(item.hard_cut),
            if item.language_mistag {
                item.detected_language.as_str()
            } else {
                ""
            }
        ));
    }
    out.push('\n');
    for item in &report.items {
        out.push_str(&format!("## {}\n\n", item.id));
        if let Some(reference) = &item.reference {
            out.push_str(&format!("Reference: {reference}\n\n"));
        }
        if item.language_mistag {
            out.push_str(&format!(
                "Language mis-tag: detected `{}`, expected `{}`.\n\n",
                item.detected_language,
                item.expected_language.as_deref().unwrap_or("")
            ));
        }
        if let Some(verified) = item.insert_verified {
            out.push_str(&format!(
                "Insert: {} (verified: {verified}).\n\n",
                item.insert_method.as_deref().unwrap_or("unknown")
            ));
        }
        for error in &item.errors {
            out.push_str(&format!("Error: {error}\n\n"));
        }
        for stage in &item.stages {
            if stage.introduced.is_empty() && stage.changed.is_empty() {
                continue;
            }
            out.push_str(&format!("### {}\n\n", stage.stage));
            if !stage.introduced.is_empty() {
                out.push_str(&format!(
                    "Introduced against the reference: {}.\n\n",
                    stage.introduced.join("; ")
                ));
            }
            if !stage.changed.is_empty() {
                out.push_str(&format!(
                    "Changed from the previous stage: {}.\n\n",
                    stage.changed.join("; ")
                ));
            }
        }
    }
    out
}

fn yes(flag: bool) -> &'static str {
    if flag {
        "yes"
    } else {
        ""
    }
}

/// Entry point for the `bolo-score` binary.
pub fn score_main(args: &[String]) -> Result<(), String> {
    let mut folder: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut api = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--api" => api = true,
            "--out" => {
                out = Some(PathBuf::from(iter.next().ok_or("--out needs a directory")?));
            }
            "--help" | "-h" => {
                println!("{}", help_text());
                return Ok(());
            }
            other if other.starts_with('-') => return Err(format!("Unknown flag {other}")),
            other => {
                if folder.is_some() {
                    return Err("Pass one folder.".into());
                }
                folder = Some(PathBuf::from(other));
            }
        }
    }
    let Some(folder) = folder else {
        println!("{}", help_text());
        return Err("Pass the folder of recordings or debug traces.".into());
    };
    if !folder.is_dir() {
        return Err(format!("{} is not a folder.", folder.display()));
    }
    let api = api || std::env::var("BOLO_SCORE_AUDIO").ok().as_deref() == Some("1");
    let report = if api {
        // score_online_blocking also checks the flag. Set it so the check passes
        // when the user passed --api and the env var is unset.
        std::env::set_var("BOLO_SCORE_AUDIO", "1");
        score_online_blocking(&folder)?
    } else {
        let traces = score_offline(&folder)?;
        if traces.items.is_empty() && !find_recording_dirs(&folder).is_empty() {
            return Err("This folder has WAV recordings but no trace.json. Re-run with --api to call Groq. Nothing was uploaded.".into());
        }
        traces
    };
    let out = out.unwrap_or_else(|| folder.clone());
    write_reports(&out, &report)?;
    println!(
        "Wrote {} and {}",
        out.join("accuracy-report.md").display(),
        out.join("accuracy-report.json").display()
    );
    if let Some(mean) = report.mean_wer_final {
        println!("Items: {}. Mean final WER: {mean:.3}", report.items.len());
    } else {
        println!(
            "Items: {}. No reference text, so word error rate was not computed.",
            report.items.len()
        );
    }
    Ok(())
}

fn help_text() -> &'static str {
    "bolo-score <folder> [--api] [--out <dir>]\n\
     \n\
     Scores debug-mode traces offline, or runs WAV recordings through the real pipeline.\n\
     \n\
     Offline (default): reads trace.json under <folder> and writes accuracy-report.md and\n\
     accuracy-report.json. Does not call Groq. Add reference.txt beside a trace to get\n\
     word error rate.\n\
     \n\
     Online: pass --api or set BOLO_SCORE_AUDIO=1, and set GROQ_API_KEY. Each audio.wav\n\
     (or captured.wav) with a reference.txt is transcribed. The key is never written.\n\
     \n\
     Record the sentences in Settings > Advanced > Accuracy. They are saved in the app\n\
     config folder (accuracy-script), not in the git repo."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_roundtrip_and_trace_redacts_keys() {
        let dir = std::env::temp_dir().join(format!("bolo-measure-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let samples = vec![0.0f32, 0.25, -0.5, 0.995];
        write_pcm(&dir.join("tone.wav"), &samples, 1, 16_000).unwrap();
        let (back, rate, channels) = read_wav(&dir.join("tone.wav")).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(channels, 1);
        assert_eq!(back.len(), samples.len());
        assert!(back[2] < -0.4);

        arm("take");
        set_device("Jack Mic");
        note_audio(
            0,
            &FinishedAudio {
                interleaved: samples.clone(),
                channels: 1,
                sample_rate: 16_000,
                elapsed_ms: 30_000,
                noise_rms: 0.0,
            },
            &[0.2, 0.92, -0.92],
        );
        note_chunk_error(0, "request failed gsk_testkey12345");
        let session = take_session().unwrap();
        let trace = build_trace(
            &session,
            &Finish {
                hold_ms: 31_000,
                final_text: "hello".into(),
                after_cleanup: Some("hello".into()),
                learned: Some("hello".into()),
                grammar: Some("hello".into()),
                ..Finish::default()
            },
        );
        assert!(!trace.hard_cut, "four samples are not a 30 second cut");
        assert!(trace.errors.iter().all(|error| !error.contains("gsk_")));
        assert!(trace
            .errors
            .iter()
            .any(|error| error.contains("[redacted]")));
        assert!(trace.peak > 0.9);
        assert!(trace.processed_clipping_pct > 0.0);
        write_bundle(&dir.join("bundle"), &session, &trace).unwrap();
        let json = fs::read_to_string(dir.join("bundle").join("trace.json")).unwrap();
        assert!(!json.contains("gsk_"));
        assert!(!json.to_lowercase().contains("apikey"));
        assert!(dir.join("bundle").join("captured.wav").is_file());
        assert!(dir.join("bundle").join("processed.wav").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_forces_a_cut_at_thirty_seconds() {
        let rate = 16_000u32;
        let samples = vec![0.2f32; rate as usize * 35];
        let pieces = split_for_score(&samples, rate, 1);
        assert!(pieces.len() >= 2, "{}", pieces.len());
        assert!(pieces[0].elapsed_ms >= 29_000, "{}", pieces[0].elapsed_ms);
        assert!(pieces[0].elapsed_ms < 32_000, "{}", pieces[0].elapsed_ms);
    }

    #[test]
    fn offline_report_names_the_stage_that_changed_the_word() {
        let dir = std::env::temp_dir().join(format!("bolo-score-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let item = dir.join("travel");
        fs::create_dir_all(&item).unwrap();
        let trace = Trace {
            id: "travel".into(),
            created_unix_ms: 1,
            device_name: "Mic".into(),
            sample_rate: 48_000,
            channels: 1,
            hold_ms: 2_000,
            trimmed_silence_ms: 100,
            peak: 0.2,
            clipping_pct: 0.0,
            processed_peak: 0.4,
            processed_clipping_pct: 0.0,
            hard_cut: false,
            chunk_count: 1,
            whisper: vec![WhisperCall {
                role: "raw".into(),
                model: "whisper-large-v3-turbo".into(),
                language: "auto".into(),
                prompt: Some("Bolo.".into()),
                detected_language: "sd".into(),
                text: "My travel ID is on the desk.".into(),
            }],
            detected_language: "sd".into(),
            pinned_language: "en".into(),
            stages: StageTexts {
                raw_whisper: "My travel ID is on the desk.".into(),
                after_redecode_translate: "My travel ID is on the desk.".into(),
                after_ai_cleanup: "My travel ID is on the desk.".into(),
                after_learned_spellings: "My travel ID is on the desk.".into(),
                after_grammar: "My travel I'd is on the desk.".into(),
                final_text: "My travel I'd is on the desk.".into(),
            },
            grammar_ran: true,
            cleanup_why: "applied".into(),
            latency_ms: 400,
            encode_ms: 10,
            remote_ms: 300,
            cleanup_ms: 50,
            paste_ms: 20,
            errors: Vec::new(),
            insert_method: Some("typed".into()),
            insert_verified: Some(false),
        };
        fs::write(
            item.join("trace.json"),
            serde_json::to_vec_pretty(&trace).unwrap(),
        )
        .unwrap();
        fs::write(item.join("reference.txt"), "My travel ID is on the desk.").unwrap();
        fs::write(
            item.join("meta.json"),
            br#"{"expectedLanguage":"en","kind":"grammar"}"#,
        )
        .unwrap();
        let report = score_offline(&dir).unwrap();
        assert_eq!(report.mode, "offline");
        assert_eq!(report.items.len(), 1);
        assert!(report.language_mistags == 1);
        assert_eq!(report.unverified_inserts, 1);
        let item = &report.items[0];
        let grammar = item
            .stages
            .iter()
            .find(|stage| stage.stage == "after_grammar")
            .unwrap();
        assert!(
            grammar
                .introduced
                .iter()
                .any(|note| note.contains("id") && note.contains("i'd")),
            "{:?}",
            grammar.introduced
        );
        let raw = item
            .stages
            .iter()
            .find(|stage| stage.stage == "raw_whisper")
            .unwrap();
        assert_eq!(raw.wer, Some(0.0));
        assert!(raw.introduced.is_empty());
        write_reports(&dir, &report).unwrap();
        let markdown = fs::read_to_string(dir.join("accuracy-report.md")).unwrap();
        assert!(markdown.contains("after_grammar"));
        assert!(!markdown.contains("gsk_"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_and_last_word_drops_are_counted() {
        let score = accuracy::wer_score("Pineapple belongs on the pizza", "belongs on the pizza");
        assert!(score.first_dropped);
        assert!(!score.last_dropped);
        let score = accuracy::wer_score(
            "Please remember the last word apricot",
            "Please remember the last word",
        );
        assert!(!score.first_dropped);
        assert!(score.last_dropped);
    }
}
