//! Microphone capture, silence trimming, and 16 kHz mono encoding.
//!
//! Groq accepts WAV and FLAC. Bolo.com uploads FLAC when the encoder succeeds
//! and falls back to 16-bit PCM WAV. One minute of mono 16 kHz WAV is about
//! 1.9 MB; FLAC is much smaller, so the upload finishes sooner.

use anyhow::{anyhow, Context};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const TARGET_RATE: u32 = 16_000;
const FRAME_MS: u32 = 20;
const PAD_MS: u32 = 160;
/// Keep a short word. A hard 150 ms floor was rejecting quiet one-syllable takes.
const MIN_SPEECH_SECS: f32 = 0.08;
/// About three minutes of stereo 48 kHz, past the recording watchdog.
pub(crate) const MAX_SAMPLES: usize = 48_000 * 2 * 180;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicInfo {
    pub name: String,
    pub is_default: bool,
}

pub struct SharedAudio {
    pub interleaved: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    pub last_rms: f32,
}

/// Owns the capture thread. The cpal stream itself is not `Send`, so it stays
/// on that thread and is dropped there when recording stops.
pub struct RecordingSession {
    stop_tx: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
    pub audio: Arc<Mutex<SharedAudio>>,
    pub stop: Arc<AtomicBool>,
    pub started: Instant,
    pub error: Arc<Mutex<Option<String>>>,
    pub device_note: Option<String>,
    /// Device this take actually opened. The overlay text is `device_note`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub active_name: String,
    pub used_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedMic {
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicChoice {
    pub active_name: String,
    pub using_fallback: bool,
    pub preference: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicStatus {
    pub preference: Option<String>,
    pub active_name: Option<String>,
    pub using_fallback: bool,
    pub message: Option<String>,
    pub devices: Vec<MicInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct CaptureReport {
    pub device_name: String,
    pub used_fallback: bool,
    pub sample_count: usize,
    pub sample_rate: u32,
    pub channels: u16,
}

pub struct FinishedAudio {
    pub interleaved: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    pub elapsed_ms: u64,
    /// Estimated room tone. The trimmer sets its speech floor from this.
    pub noise_rms: f32,
}

#[derive(Debug)]
pub enum SpeechError {
    Silent,
    Empty,
    Encode(String),
}

impl std::fmt::Display for SpeechError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Silent => write!(f, "Didn't catch any speech."),
            Self::Empty => write!(f, "The recording was empty."),
            Self::Encode(msg) => write!(f, "Couldn't encode audio: {msg}"),
        }
    }
}

pub fn list_microphones() -> anyhow::Result<Vec<MicInfo>> {
    let host = cpal::default_host();
    let listed = host_mics(&host).context("couldn't list microphones")?;
    Ok(listed
        .into_iter()
        .map(|mic| MicInfo {
            name: mic.name,
            is_default: mic.is_default,
        })
        .collect())
}

pub fn mic_status(preference: Option<&str>) -> MicStatus {
    let preference = trimmed(preference);
    match list_microphones() {
        Ok(devices) => {
            let listed: Vec<ListedMic> = devices
                .iter()
                .map(|mic| ListedMic {
                    name: mic.name.clone(),
                    is_default: mic.is_default,
                })
                .collect();
            match resolve_microphone(&listed, preference.as_deref()) {
                Ok(choice) => MicStatus {
                    message: Some(choice.note.unwrap_or_else(|| usage_line(preference.as_deref(), &choice.active_name, false))),
                    active_name: Some(choice.active_name),
                    using_fallback: choice.using_fallback,
                    preference,
                    devices,
                },
                Err(err) => MicStatus {
                    preference,
                    active_name: None,
                    using_fallback: false,
                    message: Some(err),
                    devices,
                },
            }
        }
        Err(err) => MicStatus {
            preference,
            active_name: None,
            using_fallback: false,
            message: Some(err.to_string()),
            devices: Vec::new(),
        },
    }
}

pub fn normalize_mic_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Pick the microphone to show and to try first.
///
/// `None` or an empty preference means "system default (auto)": follow whichever
/// device Windows currently calls the default. A saved name is kept even when
/// that device is missing; the default is used until the name shows up again.
pub fn resolve_microphone(devices: &[ListedMic], preference: Option<&str>) -> Result<MicChoice, String> {
    if devices.is_empty() {
        return Err("No microphone is available. Plug one in and try again.".into());
    }
    let default = devices.iter().find(|device| device.is_default).unwrap_or(&devices[0]);
    let preference = trimmed(preference);
    if let Some(pref) = preference {
        if let Some(found) = find_named(devices, &pref) {
            return Ok(MicChoice {
                active_name: found.name.clone(),
                using_fallback: false,
                preference: Some(pref),
                note: None,
            });
        }
        return Ok(MicChoice {
            active_name: default.name.clone(),
            using_fallback: true,
            note: Some(format!("{pref} isn't connected. Using {}.", default.name)),
            preference: Some(pref),
        });
    }
    Ok(MicChoice {
        active_name: default.name.clone(),
        using_fallback: false,
        preference: None,
        note: None,
    })
}

/// Devices to try, preferred first, then the current default, then the rest.
/// Names are not repeated. A missing preference is skipped rather than failed.
pub fn open_order(devices: &[ListedMic], preference: Option<&str>) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(pref) = trimmed(preference) {
        if let Some(found) = find_named(devices, &pref) {
            push_unique(&mut names, &found.name);
        }
    }
    if let Some(default) = devices.iter().find(|device| device.is_default) {
        push_unique(&mut names, &default.name);
    }
    for device in devices {
        push_unique(&mut names, &device.name);
    }
    names
}

pub fn usage_line(preference: Option<&str>, active: &str, used_fallback: bool) -> String {
    match (trimmed(preference), used_fallback) {
        (Some(pref), true) => format!("Using {active}. {pref} isn't available."),
        (None, true) => format!("Using {active}. The system default didn't open."),
        (None, false) => format!("{active} · system default"),
        (Some(_), false) => active.to_string(),
    }
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn find_named<'a>(devices: &'a [ListedMic], wanted: &str) -> Option<&'a ListedMic> {
    let wanted = normalize_mic_name(wanted);
    devices.iter().find(|device| normalize_mic_name(&device.name) == wanted)
}

fn push_unique(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|existing| normalize_mic_name(existing) == normalize_mic_name(name)) {
        names.push(name.to_string());
    }
}

pub(crate) fn host_mics(host: &cpal::Host) -> anyhow::Result<Vec<ListedMic>> {
    let default_name = host.default_input_device().and_then(|device| device.name().ok());
    let mut mics: Vec<ListedMic> = Vec::new();
    for device in host.input_devices()? {
        let name = device.name().unwrap_or_else(|_| "Microphone".into());
        let is_default = default_name.as_ref() == Some(&name);
        if let Some(existing) = mics.iter_mut().find(|mic| mic.name == name) {
            existing.is_default |= is_default;
            continue;
        }
        mics.push(ListedMic { name, is_default });
    }
    if !mics.is_empty() && mics.iter().all(|mic| !mic.is_default) {
        mics[0].is_default = true;
    }
    Ok(mics)
}

struct OpenedMic {
    active_name: String,
    used_fallback: bool,
    note: String,
}

pub fn start_recording(wanted: Option<&str>) -> anyhow::Result<RecordingSession> {
    let wanted = wanted.map(str::to_string);
    let (ready_tx, ready_rx) = mpsc::channel::<anyhow::Result<OpenedMic>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let audio = Arc::new(Mutex::new(SharedAudio {
        interleaved: Vec::new(),
        channels: 1,
        sample_rate: TARGET_RATE,
        last_rms: 0.0,
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let error = Arc::new(Mutex::new(None));
    let audio_thread = Arc::clone(&audio);
    let stop_flag = Arc::clone(&stop);
    let error_flag = Arc::clone(&error);

    let worker = thread::Builder::new()
        .name("cadence-audio".into())
        .spawn(move || {
            capture_until_stop(
                wanted.as_deref(),
                &audio_thread,
                &stop_flag,
                &error_flag,
                ready_tx,
                stop_rx,
            );
        })
        .context("couldn't start audio capture")?;

    let ready = ready_rx
        .recv()
        .context("audio capture stopped before it started")??;
    Ok(RecordingSession {
        stop_tx: Some(stop_tx),
        worker: Some(worker),
        audio,
        stop,
        started: Instant::now(),
        error,
        device_note: Some(ready.note),
        active_name: ready.active_name,
        used_fallback: ready.used_fallback,
    })
}

/// Record a short take. Used by the Windows self-test and returns as soon as
/// the stream has been healthy long enough to prove the device opened.
#[cfg_attr(not(test), allow(dead_code))]
pub fn capture_for(preference: Option<&str>, duration: Duration) -> Result<CaptureReport, String> {
    let session = start_recording(preference).map_err(|err| err.to_string())?;
    let device_name = session.active_name.clone();
    let used_fallback = session.used_fallback;
    thread::sleep(duration);
    let finished = session.finish()?;
    Ok(CaptureReport {
        device_name,
        used_fallback,
        sample_count: finished.interleaved.len(),
        sample_rate: finished.sample_rate,
        channels: finished.channels,
    })
}

fn capture_until_stop(
    wanted: Option<&str>,
    audio: &Arc<Mutex<SharedAudio>>,
    stop: &Arc<AtomicBool>,
    error: &Arc<Mutex<Option<String>>>,
    ready_tx: Sender<anyhow::Result<OpenedMic>>,
    stop_rx: mpsc::Receiver<()>,
) {
    let host = cpal::default_host();
    let devices = match host_mics(&host) {
        Ok(devices) => devices,
        Err(err) => {
            let _ = ready_tx.send(Err(err));
            return;
        }
    };
    let order = open_order(&devices, wanted);
    if order.is_empty() {
        let _ = ready_tx.send(Err(anyhow!(
            "No microphone is available. Plug one in and try again."
        )));
        return;
    }
    let mut failures = Vec::new();
    for name in &order {
        if let Ok(mut slot) = error.lock() {
            *slot = None;
        }
        match open_named(&host, name, audio, stop, error) {
            Ok(stream) => {
                if let Err(err) = confirm_alive(error) {
                    drop(stream);
                    failures.push(format!("{name}: {err}"));
                    continue;
                }
                let preference = trimmed(wanted);
                let opened_preference = preference
                    .as_deref()
                    .is_some_and(|pref| normalize_mic_name(pref) == normalize_mic_name(name));
                let skipped_first = order.first().map(String::as_str) != Some(name.as_str());
                let used_fallback = match &preference {
                    Some(_) => !opened_preference,
                    None => skipped_first,
                };
                let note = usage_line(preference.as_deref(), name, used_fallback);
                if ready_tx
                    .send(Ok(OpenedMic {
                        active_name: name.clone(),
                        used_fallback,
                        note,
                    }))
                    .is_err()
                {
                    drop(stream);
                    return;
                }
                let _ = stop_rx.recv();
                drop(stream);
                return;
            }
            Err(err) => failures.push(format!("{name}: {err:#}")),
        }
    }
    let mut message = String::from("Couldn't open a microphone.");
    if !failures.is_empty() {
        message.push(' ');
        message.push_str(&failures.join(" "));
    }
    let _ = ready_tx.send(Err(anyhow!(message)));
}

/// Wait briefly for an immediate device-removed error. This runs when a stream
/// is first opened, not on every key press.
pub(crate) fn confirm_alive(error: &Mutex<Option<String>>) -> Result<(), String> {
    for _ in 0..4 {
        thread::sleep(Duration::from_millis(20));
        if let Some(err) = error.lock().ok().and_then(|slot| slot.clone()) {
            return Err(err);
        }
    }
    Ok(())
}

pub(crate) fn open_named(
    host: &cpal::Host,
    name: &str,
    audio: &Arc<Mutex<SharedAudio>>,
    stop: &Arc<AtomicBool>,
    error: &Arc<Mutex<Option<String>>>,
) -> anyhow::Result<cpal::Stream> {
    let wanted = normalize_mic_name(name);
    let device = host
        .input_devices()?
        .find(|device| {
            device
                .name()
                .ok()
                .map(|found| normalize_mic_name(&found) == wanted)
                .unwrap_or(false)
        })
        .with_context(|| format!("microphone \"{name}\" disappeared"))?;
    let supported = device
        .default_input_config()
        .with_context(|| format!("{name} has no default input format"))?;
    let channels = supported.channels();
    let sample_rate = supported.sample_rate().0;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    if let Ok(mut buffer) = audio.lock() {
        buffer.channels = channels;
        buffer.sample_rate = sample_rate;
        buffer.interleaved.clear();
        buffer.last_rms = 0.0;
    }
    let stream = build_stream(
        &device,
        &config,
        sample_format,
        Arc::clone(audio),
        Arc::clone(stop),
        Arc::clone(error),
    )?;
    stream
        .play()
        .with_context(|| format!("couldn't start {name}"))?;
    Ok(stream)
}

impl RecordingSession {
    pub fn finish(mut self) -> Result<FinishedAudio, String> {
        self.halt();
        if let Some(err) = self.error.lock().ok().and_then(|guard| guard.clone()) {
            return Err(err);
        }
        let audio = self
            .audio
            .lock()
            .map_err(|_| "The audio buffer was unavailable.".to_string())?;
        Ok(FinishedAudio {
            interleaved: audio.interleaved.clone(),
            channels: audio.channels,
            sample_rate: audio.sample_rate,
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            noise_rms: 0.0,
        })
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        self.halt();
    }
}

pub struct EncodedSpeech {
    pub bytes: Vec<u8>,
    /// `flac` or `wav`. Logged with the stage timings.
    pub format: &'static str,
}

pub fn prepare_speech(finished: &FinishedAudio) -> Result<EncodedSpeech, SpeechError> {
    let resampled = speech_samples(finished)?;
    encode_speech(&resampled, TARGET_RATE)
}

/// Trimmed, gain-normalized, 16 kHz mono samples ready to encode.
pub fn speech_samples(finished: &FinishedAudio) -> Result<Vec<f32>, SpeechError> {
    if finished.channels == 0 || finished.sample_rate == 0 || finished.interleaved.is_empty() {
        return Err(SpeechError::Empty);
    }
    let mono = downmix(&finished.interleaved, finished.channels);
    let trimmed = trim_silence_with(&mono, finished.sample_rate, speech_threshold(finished.noise_rms));
    if trimmed.is_empty() {
        return Err(SpeechError::Silent);
    }
    let duration = trimmed.len() as f32 / finished.sample_rate as f32;
    if duration < MIN_SPEECH_SECS {
        return Err(SpeechError::Silent);
    }
    let gained = normalize_gain(&trimmed);
    Ok(resample(&gained, finished.sample_rate, TARGET_RATE))
}

/// Boost quiet speech toward a comfortable peak. Already-loud audio is left
/// alone, and a near-silent buffer is not amplified into noise.
pub fn normalize_gain(samples: &[f32]) -> Vec<f32> {
    const TARGET_PEAK: f32 = 0.72;
    const MAX_GAIN: f32 = 6.0;
    let peak = samples.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    if peak < 0.001 {
        return samples.to_vec();
    }
    let gain = (TARGET_PEAK / peak).clamp(1.0, MAX_GAIN);
    if gain <= 1.001 {
        return samples.to_vec();
    }
    samples
        .iter()
        .map(|sample| (sample * gain).clamp(-1.0, 1.0))
        .collect()
}

pub fn downmix(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    if channels == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| {
            let n = frame.len().min(channels).max(1);
            frame.iter().take(n).sum::<f32>() / n as f32
        })
        .collect()
}

/// Speech floor from the idle noise estimate.
///
/// A fixed 0.015 RMS treated a quiet laptop array as silence. The floor stays
/// low enough for soft speech and high enough that pure digital silence is cut.
pub fn speech_threshold(noise_rms: f32) -> f32 {
    (noise_rms.max(0.0) * 3.2 + 0.004).clamp(0.004, 0.018)
}

/// Drop leading and trailing silence. Pauses in the middle of a sentence stay.
pub fn trim_silence(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    trim_silence_with(samples, sample_rate, speech_threshold(0.0))
}

pub fn trim_silence_with(samples: &[f32], sample_rate: u32, threshold: f32) -> Vec<f32> {
    if sample_rate == 0 || samples.is_empty() {
        return Vec::new();
    }
    let frame = ((sample_rate as usize) * FRAME_MS as usize) / 1000;
    if frame == 0 {
        return samples.to_vec();
    }
    let voiced: Vec<bool> = samples
        .chunks(frame)
        .map(|chunk| rms(chunk) >= threshold)
        .collect();
    // Onset needs two voiced frames so a click is not speech. A short hangover
    // keeps a quiet consonant that follows the last loud frame.
    let Some((first, last)) = speech_bounds(&voiced) else {
        return Vec::new();
    };
    const HANGOVER_FRAMES: usize = 3;
    let last = (last + HANGOVER_FRAMES).min(voiced.len().saturating_sub(1));
    let pad = ((sample_rate as usize) * PAD_MS as usize) / 1000;
    let start = (first * frame).saturating_sub(pad);
    let end = ((last + 1) * frame).saturating_add(pad).min(samples.len());
    if start >= end {
        return Vec::new();
    }
    samples[start..end].to_vec()
}

/// First and last frame indexes that belong to a real onset (two voiced frames
/// in a row). A single loud frame is ignored.
fn speech_bounds(voiced: &[bool]) -> Option<(usize, usize)> {
    const ONSET: usize = 2;
    if voiced.is_empty() {
        return None;
    }
    if voiced.len() < ONSET {
        return if voiced.iter().any(|flag| *flag) {
            Some((0, voiced.len() - 1))
        } else {
            None
        };
    }
    let mut start = None;
    let mut run = 0usize;
    for (index, flag) in voiced.iter().enumerate() {
        run = if *flag { run + 1 } else { 0 };
        if run >= ONSET {
            start = Some(index + 1 - ONSET);
            break;
        }
    }
    let start = start?;
    let mut end = None;
    run = 0;
    for index in (0..voiced.len()).rev() {
        run = if voiced[index] { run + 1 } else { 0 };
        if run >= ONSET {
            end = Some(index + ONSET - 1);
            break;
        }
    }
    let end = end?;
    if end < start { None } else { Some((start, end)) }
}

pub fn resample(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    if input.is_empty() || in_rate == 0 || out_rate == 0 {
        return Vec::new();
    }
    if in_rate == out_rate {
        return input.to_vec();
    }
    if in_rate > out_rate {
        return downsample_box(input, in_rate, out_rate);
    }
    let ratio = in_rate as f64 / out_rate as f64;
    let out_len = ((input.len() as u64) * u64::from(out_rate) / u64::from(in_rate)) as usize;
    let mut out = Vec::with_capacity(out_len);
    let last = input.len() - 1;
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = (pos.floor() as usize).min(last);
        let frac = (pos - idx as f64) as f32;
        let a = input[idx];
        let b = input[(idx + 1).min(last)];
        out.push(a + (b - a) * frac);
    }
    out
}

pub fn encode_speech(samples: &[f32], sample_rate: u32) -> Result<EncodedSpeech, SpeechError> {
    if let Ok(bytes) = encode_flac(samples, sample_rate) {
        if bytes.starts_with(b"fLaC") && bytes.len() > 42 {
            return Ok(EncodedSpeech { bytes, format: "flac" });
        }
    }
    let bytes = encode_wav(samples, sample_rate).map_err(|err| SpeechError::Encode(err.to_string()))?;
    Ok(EncodedSpeech { bytes, format: "wav" })
}

fn encode_flac(samples: &[f32], sample_rate: u32) -> anyhow::Result<Vec<u8>> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    if samples.is_empty() || sample_rate == 0 {
        return Err(anyhow!("empty audio"));
    }
    let pcm: Vec<i32> = samples
        .iter()
        .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i32)
        .collect();
    let mut config = flacenc::config::Encoder::default();
    // Fixed predictors and one thread. A short take should not pay to search
    // LPC orders or start a worker pool.
    config.multithread = false;
    config.block_size = 2048;
    config.subframe_coding.use_lpc = false;
    let config = config
        .into_verified()
        .map_err(|err| anyhow!("flac config: {err:?}"))?;
    let source = flacenc::source::MemSource::from_samples(&pcm, 1, 16, sample_rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|err| anyhow!("flac encode: {err:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|err| anyhow!("flac write: {err}"))?;
    Ok(sink.as_slice().to_vec())
}

pub fn encode_wav(samples: &[f32], sample_rate: u32) -> anyhow::Result<Vec<u8>> {
    if samples.is_empty() || sample_rate == 0 {
        return Err(anyhow!("empty audio"));
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(value)?;
    }
    writer.finalize()?;
    Ok(cursor.into_inner())
}

fn downsample_box(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    let ratio = in_rate as f64 / out_rate as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let start = ((i as f64) * ratio) as usize;
        let end = ((((i + 1) as f64) * ratio) as usize)
            .max(start + 1)
            .min(input.len());
        let slice = &input[start..end];
        let avg = slice.iter().sum::<f32>() / slice.len() as f32;
        out.push(avg);
    }
    out
}

fn rms(chunk: &[f32]) -> f32 {
    if chunk.is_empty() {
        return 0.0;
    }
    (chunk.iter().map(|sample| sample * sample).sum::<f32>() / chunk.len() as f32).sqrt()
}

pub(crate) fn build_input_callback<F, E>(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    on_samples: F,
    on_error: E,
) -> anyhow::Result<cpal::Stream>
where
    F: FnMut(Vec<f32>) + Send + 'static,
    E: FnMut(cpal::StreamError) + Send + 'static,
{
    let stream = match sample_format {
        SampleFormat::F32 => {
            let mut on_samples = on_samples;
            let mut on_error = on_error;
            device.build_input_stream(
                config,
                move |data: &[f32], _| on_samples(data.to_vec()),
                move |err| on_error(err),
                None,
            )
        }
        SampleFormat::I16 => {
            let mut on_samples = on_samples;
            let mut on_error = on_error;
            device.build_input_stream(
                config,
                move |data: &[i16], _| {
                    on_samples(data.iter().map(|sample| *sample as f32 / i16::MAX as f32).collect())
                },
                move |err| on_error(err),
                None,
            )
        }
        SampleFormat::U16 => {
            let mut on_samples = on_samples;
            let mut on_error = on_error;
            device.build_input_stream(
                config,
                move |data: &[u16], _| {
                    on_samples(
                        data.iter()
                            .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect(),
                    )
                },
                move |err| on_error(err),
                None,
            )
        }
        SampleFormat::I32 => {
            let mut on_samples = on_samples;
            let mut on_error = on_error;
            device.build_input_stream(
                config,
                move |data: &[i32], _| {
                    on_samples(data.iter().map(|sample| *sample as f32 / i32::MAX as f32).collect())
                },
                move |err| on_error(err),
                None,
            )
        }
        SampleFormat::F64 => {
            let mut on_samples = on_samples;
            let mut on_error = on_error;
            device.build_input_stream(
                config,
                move |data: &[f64], _| on_samples(data.iter().map(|sample| *sample as f32).collect()),
                move |err| on_error(err),
                None,
            )
        }
        other => {
            return Err(anyhow!(
                "This microphone uses an unsupported sample format ({other:?})."
            ))
        }
    }
    .with_context(|| format!("couldn't open the microphone ({sample_format:?})"))?;
    Ok(stream)
}

pub(crate) fn build_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    audio: Arc<Mutex<SharedAudio>>,
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
) -> anyhow::Result<cpal::Stream> {
    let err_audio = Arc::clone(&error);
    let on_error = move |err: cpal::StreamError| {
        if let Ok(mut slot) = err_audio.lock() {
            *slot = Some(format!("Microphone error: {err}"));
        }
    };

    let stream = match sample_format {
        SampleFormat::F32 => {
            let audio = Arc::clone(&audio);
            let stop = Arc::clone(&stop);
            device.build_input_stream(
                config,
                move |data: &[f32], _| push_samples(&audio, &stop, data.iter().copied()),
                on_error,
                None,
            )
        }
        SampleFormat::I16 => {
            let audio = Arc::clone(&audio);
            let stop = Arc::clone(&stop);
            device.build_input_stream(
                config,
                move |data: &[i16], _| {
                    push_samples(&audio, &stop, data.iter().map(|sample| *sample as f32 / i16::MAX as f32))
                },
                on_error,
                None,
            )
        }
        SampleFormat::U16 => {
            let audio = Arc::clone(&audio);
            let stop = Arc::clone(&stop);
            device.build_input_stream(
                config,
                move |data: &[u16], _| {
                    push_samples(
                        &audio,
                        &stop,
                        data.iter()
                            .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0),
                    )
                },
                on_error,
                None,
            )
        }
        SampleFormat::I32 => {
            let audio = Arc::clone(&audio);
            let stop = Arc::clone(&stop);
            device.build_input_stream(
                config,
                move |data: &[i32], _| {
                    push_samples(&audio, &stop, data.iter().map(|sample| *sample as f32 / i32::MAX as f32))
                },
                on_error,
                None,
            )
        }
        SampleFormat::F64 => {
            let audio = Arc::clone(&audio);
            let stop = Arc::clone(&stop);
            device.build_input_stream(
                config,
                move |data: &[f64], _| push_samples(&audio, &stop, data.iter().map(|sample| *sample as f32)),
                on_error,
                None,
            )
        }
        other => {
            return Err(anyhow!(
                "This microphone uses an unsupported sample format ({other:?})."
            ))
        }
    }
    .with_context(|| format!("couldn't open the microphone ({sample_format:?})"))?;
    Ok(stream)
}

fn push_samples(audio: &Mutex<SharedAudio>, stop: &AtomicBool, samples: impl IntoIterator<Item = f32>) {
    if stop.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut audio) = audio.lock() else {
        return;
    };
    if audio.interleaved.len() >= MAX_SAMPLES {
        stop.store(true, Ordering::Relaxed);
        return;
    }
    let start = audio.interleaved.len();
    audio.interleaved.extend(samples);
    let chunk = &audio.interleaved[start..];
    audio.last_rms = rms(chunk);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, ms: u32, amp: f32) -> Vec<f32> {
        let n = (rate as usize) * (ms as usize) / 1000;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (t * 440.0 * std::f32::consts::TAU).sin() * amp
            })
            .collect()
    }

    #[test]
    fn trims_edges_and_keeps_a_middle_pause() {
        let rate = 16_000;
        let mut samples = vec![0.0; rate as usize / 2];
        samples.extend(tone(rate, 300, 0.25));
        samples.extend(vec![0.0; rate as usize / 5]);
        samples.extend(tone(rate, 300, 0.25));
        samples.extend(vec![0.0; rate as usize / 2]);
        let trimmed = trim_silence(&samples, rate);
        assert!(trimmed.len() < samples.len());
        assert!(trimmed.len() > rate as usize / 2);
        assert!(trimmed.iter().any(|sample| sample.abs() > 0.1));
    }

    #[test]
    fn pure_silence_trims_to_nothing() {
        assert!(trim_silence(&vec![0.0; 16_000], 16_000).is_empty());
    }

    #[test]
    fn downmix_averages_stereo_pairs() {
        let mono = downmix(&[0.0, 1.0, 0.5, 0.5], 2);
        assert_eq!(mono, vec![0.5, 0.5]);
    }

    #[test]
    fn resample_48k_to_16k_is_one_third() {
        let input = tone(48_000, 1000, 0.2);
        let output = resample(&input, 48_000, 16_000);
        assert_eq!(output.len(), 16_000);
    }

    #[test]
    fn wav_roundtrip_preserves_length_and_header() {
        let samples = tone(16_000, 200, 0.4);
        let bytes = encode_wav(&samples, 16_000).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let cursor = std::io::Cursor::new(bytes);
        let mut reader = hound::WavReader::new(cursor).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().bits_per_sample, 16);
        let decoded: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(decoded.len(), samples.len());
        assert!(decoded.iter().any(|sample| sample.abs() > 1000));
    }

    #[test]
    fn prepare_speech_rejects_silence_and_encodes_a_tone() {
        let silent = FinishedAudio {
            interleaved: vec![0.0; 8_000],
            channels: 1,
            sample_rate: 16_000,
            elapsed_ms: 500,
            noise_rms: 0.0,
        };
        assert!(matches!(prepare_speech(&silent), Err(SpeechError::Silent)));

        let speech = FinishedAudio {
            interleaved: {
                let mut samples = vec![0.0; 4_000];
                samples.extend(tone(16_000, 400, 0.3));
                samples.extend(vec![0.0; 4_000]);
                samples
            },
            channels: 1,
            sample_rate: 16_000,
            elapsed_ms: 800,
            noise_rms: 0.0,
        };
        let encoded = prepare_speech(&speech).unwrap();
        assert!(encoded.bytes.len() > 42);
        assert_eq!(&encoded.bytes[0..4], b"fLaC");
        assert_eq!(encoded.format, "flac");
    }

    #[test]
    fn a_single_click_does_not_count_as_speech() {
        let mut samples = vec![0.0; 8_000];
        for sample in samples.iter_mut().skip(1_600).take(320) {
            *sample = 0.4;
        }
        assert!(trim_silence(&samples, 16_000).is_empty());
    }

    #[test]
    fn quiet_speech_is_boosted_and_loud_speech_is_not() {
        let quiet = tone(16_000, 200, 0.05);
        let gained = normalize_gain(&quiet);
        let peak = gained.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!(peak > 0.2 && peak <= 0.73, "peak {peak}");
        let loud = tone(16_000, 200, 0.9);
        let unchanged = normalize_gain(&loud);
        let loud_peak = unchanged.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!((loud_peak - 0.9).abs() < 0.05, "loud peak {loud_peak}");
        assert!(normalize_gain(&vec![0.0001; 400]).iter().all(|sample| sample.abs() < 0.001));
    }

    #[test]
    fn quiet_speech_survives_and_room_tone_does_not() {
        let quiet = tone(16_000, 300, 0.012);
        assert!(
            !trim_silence_with(&quiet, 16_000, speech_threshold(0.0)).is_empty(),
            "soft speech should stay above the low floor"
        );
        let hiss = tone(16_000, 300, 0.001);
        assert!(
            trim_silence_with(&hiss, 16_000, speech_threshold(0.008)).is_empty(),
            "a raised noise floor should still reject hiss"
        );
        assert!(speech_threshold(0.0) < 0.01);
        assert!(speech_threshold(0.02) <= 0.018);
    }

    fn headset() -> ListedMic {
        ListedMic {
            name: "Jack Mic (2- Realtek(R) Audio)".into(),
            is_default: false,
        }
    }

    fn laptop() -> ListedMic {
        ListedMic {
            name: "Microphone Array".into(),
            is_default: true,
        }
    }

    #[test]
    fn auto_follows_the_current_default() {
        let unplugged = vec![laptop()];
        let choice = resolve_microphone(&unplugged, None).unwrap();
        assert_eq!(choice.active_name, "Microphone Array");
        assert!(!choice.using_fallback);
        assert!(choice.preference.is_none());

        let replugged = vec![
            ListedMic {
                name: laptop().name,
                is_default: false,
            },
            ListedMic {
                name: "USB Studio Mic".into(),
                is_default: true,
            },
        ];
        let choice = resolve_microphone(&replugged, None).unwrap();
        assert_eq!(choice.active_name, "USB Studio Mic");
        assert!(!choice.using_fallback);
    }

    #[test]
    fn missing_preference_uses_default_and_returns_when_replugged() {
        let preferred = headset();
        let preference = Some(preferred.name.as_str());
        let gone = resolve_microphone(&[laptop()], preference).unwrap();
        assert!(gone.using_fallback);
        assert_eq!(gone.active_name, "Microphone Array");
        assert_eq!(gone.preference.as_deref(), Some(headset().name.as_str()));
        assert!(gone.note.unwrap().contains("isn't connected"));
        assert_eq!(
            usage_line(Some(preferred.name.as_str()), "Microphone Array", true),
            "Using Microphone Array. Jack Mic (2- Realtek(R) Audio) isn't available."
        );
        assert_eq!(
            usage_line(None, "Microphone Array", false),
            "Microphone Array · system default"
        );

        let back = resolve_microphone(&[headset(), laptop()], preference).unwrap();
        assert!(!back.using_fallback);
        assert_eq!(back.active_name, headset().name);
        assert!(back.note.is_none());
    }

    #[test]
    fn name_match_ignores_case_and_extra_spaces() {
        let devices = vec![headset(), laptop()];
        let choice = resolve_microphone(&devices, Some("  jack mic (2- realtek(r) audio)  ")).unwrap();
        assert_eq!(choice.active_name, headset().name);
        assert!(!choice.using_fallback);
    }

    #[test]
    fn open_order_tries_preference_then_default_then_others() {
        let usb = ListedMic {
            name: "USB Studio Mic".into(),
            is_default: false,
        };
        let devices = vec![headset(), laptop(), usb.clone()];
        assert_eq!(
            open_order(&devices, Some(&headset().name)),
            vec![headset().name, laptop().name, usb.name.clone()]
        );
        assert_eq!(
            open_order(&devices, None),
            vec![laptop().name, headset().name, usb.name.clone()]
        );
        assert_eq!(
            open_order(&devices, Some("Not A Real Mic")),
            vec![laptop().name, headset().name, usb.name]
        );
    }

    #[test]
    fn no_devices_is_an_error_and_does_not_invent_one() {
        let err = resolve_microphone(&[], Some("Jack Mic")).unwrap_err();
        assert!(err.contains("No microphone"));
        assert!(open_order(&[], Some("Jack Mic")).is_empty());
    }

    #[test]
    fn records_when_a_microphone_exists() {
        match list_microphones() {
            Ok(mics) if !mics.is_empty() => mics,
            Ok(_) => {
                eprintln!("SKIP records_when_a_microphone_exists: no input device");
                return;
            }
            Err(err) => {
                eprintln!("SKIP records_when_a_microphone_exists: {err}");
                return;
            }
        };
        let report = capture_for(None, Duration::from_millis(400)).expect("capture");
        assert!(!report.device_name.is_empty());
        assert!(report.sample_rate > 0);
        assert!(report.channels > 0);
        assert!(
            report.sample_count > 0,
            "the open microphone produced no samples ({})",
            report.device_name
        );
    }

    #[test]
    fn missing_microphone_falls_back_when_a_device_exists() {
        let mics = match list_microphones() {
            Ok(mics) if !mics.is_empty() => mics,
            _ => {
                eprintln!("SKIP missing_microphone_falls_back_when_a_device_exists: no input device");
                return;
            }
        };
        let missing = "Cadence Missing Mic 9000";
        let report = capture_for(Some(missing), Duration::from_millis(400)).expect("fallback capture");
        assert!(report.used_fallback, "expected a fallback, opened {}", report.device_name);
        assert_ne!(normalize_mic_name(&report.device_name), normalize_mic_name(missing));
        assert!(
            mics.iter().any(|mic| normalize_mic_name(&mic.name) == normalize_mic_name(&report.device_name)),
            "fallback {} is not a listed device",
            report.device_name
        );
        assert!(report.sample_count > 0);
    }
}
