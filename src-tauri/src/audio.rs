//! Microphone capture, silence trimming, and 16 kHz mono WAV encoding.
//!
//! Groq accepts WAV and FLAC. Cadence uploads 16-bit PCM WAV: one minute of
//! mono 16 kHz audio is about 1.9 MB, well under Groq's upload limit, and the
//! encoder has no native dependency.

use anyhow::{anyhow, Context};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

const TARGET_RATE: u32 = 16_000;
const FRAME_MS: u32 = 20;
const SILENCE_RMS: f32 = 0.015;
const PAD_MS: u32 = 140;
const MIN_SPEECH_SECS: f32 = 0.15;
/// About three minutes of stereo 48 kHz, past the recording watchdog.
const MAX_SAMPLES: usize = 48_000 * 2 * 180;

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
}

pub struct FinishedAudio {
    pub interleaved: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    pub elapsed_ms: u64,
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
    let default_name = host.default_input_device().and_then(|device| device.name().ok());
    let mut mics = Vec::new();
    for device in host.input_devices().context("couldn't list microphones")? {
        let name = device.name().unwrap_or_else(|_| "Microphone".into());
        let is_default = default_name.as_ref() == Some(&name);
        mics.push(MicInfo { name, is_default });
    }
    Ok(mics)
}

struct CaptureReady {
    device_note: Option<String>,
}

pub fn start_recording(wanted: Option<&str>) -> anyhow::Result<RecordingSession> {
    let wanted = wanted.map(str::to_string);
    let (ready_tx, ready_rx) = mpsc::channel::<anyhow::Result<CaptureReady>>();
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
            let opened = open_input(wanted.as_deref(), &audio_thread, &stop_flag, &error_flag);
            match opened {
                Ok((stream, note)) => {
                    if ready_tx.send(Ok(CaptureReady { device_note: note })).is_err() {
                        return;
                    }
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                }
            }
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
        device_note: ready.device_note,
    })
}

fn open_input(
    wanted: Option<&str>,
    audio: &Arc<Mutex<SharedAudio>>,
    stop: &Arc<AtomicBool>,
    error: &Arc<Mutex<Option<String>>>,
) -> anyhow::Result<(cpal::Stream, Option<String>)> {
    let host = cpal::default_host();
    let (device, device_note) = select_device(&host, wanted)?;
    let supported = device
        .default_input_config()
        .context("the microphone has no default input format")?;
    let channels = supported.channels();
    let sample_rate = supported.sample_rate().0;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    if let Ok(mut buffer) = audio.lock() {
        buffer.channels = channels;
        buffer.sample_rate = sample_rate;
    }
    let stream = build_stream(
        &device,
        &config,
        sample_format,
        Arc::clone(audio),
        Arc::clone(stop),
        Arc::clone(error),
    )?;
    stream.play().context("couldn't start microphone capture")?;
    Ok((stream, device_note))
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

pub fn prepare_speech(finished: &FinishedAudio) -> Result<Vec<u8>, SpeechError> {
    if finished.channels == 0 || finished.sample_rate == 0 || finished.interleaved.is_empty() {
        return Err(SpeechError::Empty);
    }
    let mono = downmix(&finished.interleaved, finished.channels);
    let trimmed = trim_silence(&mono, finished.sample_rate);
    if trimmed.is_empty() {
        return Err(SpeechError::Silent);
    }
    let duration = trimmed.len() as f32 / finished.sample_rate as f32;
    if duration < MIN_SPEECH_SECS {
        return Err(SpeechError::Silent);
    }
    let resampled = resample(&trimmed, finished.sample_rate, TARGET_RATE);
    encode_wav(&resampled, TARGET_RATE).map_err(|err| SpeechError::Encode(err.to_string()))
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

/// Drop leading and trailing silence. Pauses in the middle of a sentence stay.
pub fn trim_silence(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if sample_rate == 0 || samples.is_empty() {
        return Vec::new();
    }
    let frame = ((sample_rate as usize) * FRAME_MS as usize) / 1000;
    if frame == 0 {
        return samples.to_vec();
    }
    let voiced: Vec<bool> = samples
        .chunks(frame)
        .map(|chunk| rms(chunk) >= SILENCE_RMS)
        .collect();
    let Some(first) = voiced.iter().position(|voiced| *voiced) else {
        return Vec::new();
    };
    let last = voiced.iter().rposition(|voiced| *voiced).unwrap_or(first);
    let pad = ((sample_rate as usize) * PAD_MS as usize) / 1000;
    let start = (first * frame).saturating_sub(pad);
    let end = ((last + 1) * frame).saturating_add(pad).min(samples.len());
    if start >= end {
        return Vec::new();
    }
    samples[start..end].to_vec()
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

fn select_device(host: &cpal::Host, wanted: Option<&str>) -> anyhow::Result<(Device, Option<String>)> {
    if let Some(name) = wanted.map(str::trim).filter(|name| !name.is_empty()) {
        if let Some(device) = host.input_devices()?.find(|device| {
            device.name().ok().as_deref() == Some(name)
        }) {
            return Ok((device, None));
        }
        let fallback = host
            .default_input_device()
            .context("No microphone is available")?;
        return Ok((
            fallback,
            Some(format!(
                "Saved microphone \"{name}\" isn't connected. Using the system default."
            )),
        ));
    }
    let device = host
        .default_input_device()
        .context("No microphone is available. Plug one in and try again.")?;
    Ok((device, None))
}

fn build_stream(
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
        };
        let wav = prepare_speech(&speech).unwrap();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
    }
}
