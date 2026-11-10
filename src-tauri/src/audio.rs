//! Microphone capture, silence trimming, and 16 kHz mono encoding.
//!
//! Groq accepts WAV and FLAC. Bolo uploads FLAC when the encoder succeeds
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
    /// Device this take actually opened.
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

/// Microphones that delivered pure digital silence on a recent take or probe.
/// Windows often makes an empty headset jack the default input; we then fall
/// back to a microphone that actually hears.
static SILENT_MICS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
/// Microphones that delivered real signal (probe or take). Remembered so the
/// next take goes straight to a mic that is known to hear.
static LIVE_MICS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Below this peak a buffer is digital silence: a real mic always has hiss.
pub const SILENT_PEAK: f32 = 0.0005;

pub fn mark_silent_mic(name: &str) -> bool {
    let key = normalize_mic_name(name);
    LIVE_MICS.lock().unwrap_or_else(|err| err.into_inner()).retain(|item| *item != key);
    let mut list = SILENT_MICS.lock().unwrap_or_else(|err| err.into_inner());
    if list.contains(&key) {
        return false;
    }
    list.push(key);
    true
}

pub fn clear_silent_mic(name: &str) {
    let key = normalize_mic_name(name);
    SILENT_MICS.lock().unwrap_or_else(|err| err.into_inner()).retain(|item| *item != key);
}

/// Remember that this microphone hears (and is therefore not silent).
pub fn mark_live_mic(name: &str) {
    clear_silent_mic(name);
    let key = normalize_mic_name(name);
    let mut list = LIVE_MICS.lock().unwrap_or_else(|err| err.into_inner());
    if !list.contains(&key) {
        list.push(key);
    }
}

pub fn clear_silent_mics() {
    SILENT_MICS.lock().unwrap_or_else(|err| err.into_inner()).clear();
    LIVE_MICS.lock().unwrap_or_else(|err| err.into_inner()).clear();
}

pub fn is_silent_mic(name: &str) -> bool {
    let key = normalize_mic_name(name);
    SILENT_MICS.lock().unwrap_or_else(|err| err.into_inner()).contains(&key)
}

fn is_live_mic(name: &str) -> bool {
    let key = normalize_mic_name(name);
    LIVE_MICS.lock().unwrap_or_else(|err| err.into_inner()).contains(&key)
}

/// A headphone/combo jack or line input: often listed even when nothing is
/// plugged in, and then it records pure zeros.
/// Speakers, headphones, and loopback endpoints are not microphones.
/// Windows sometimes lists them as capture devices ("Speakers (Realtek)").
/// Recording one of those hears playback, not the person, and the opening
/// words are lost while Bolo switches away.
pub fn is_output_like(name: &str) -> bool {
    let n = normalize_mic_name(name);
    if n.contains("microphone") || n.contains("mic array") || n.contains("array") {
        return false;
    }
    const MARKERS: &[&str] = &[
        "speaker",
        "speakers",
        "headphone",
        "headphones",
        "loopback",
        "stereo mix",
        "what u hear",
        "wave out",
        "playback",
        "output",
    ];
    MARKERS.iter().any(|marker| n.contains(marker))
}

pub fn is_jack_like(name: &str) -> bool {
    let n = normalize_mic_name(name);
    n.contains("jack") || n.contains("line in") || n.contains("line-in") || n.contains("stereo mix")
}

/// A microphone built into the laptop or webcam: always physically present.
pub fn is_builtin_like(name: &str) -> bool {
    let n = normalize_mic_name(name);
    n.contains("array") || n.contains("internal") || n.contains("built-in") || n.contains("builtin") || n.contains("integrated")
}

/// Lower is better. Known-live first, then built-in, then other mics, then
/// unconfirmed jacks, then mics known to record silence.
pub(crate) fn mic_rank(name: &str, silent: &dyn Fn(&str) -> bool, live: &dyn Fn(&str) -> bool) -> u8 {
    if silent(name) {
        4
    } else if live(name) {
        0
    } else if is_builtin_like(name) {
        1
    } else if is_jack_like(name) {
        3
    } else {
        2
    }
}

/// "Automatic" choice: the Windows default unless it is known to be silent,
/// or it is an unconfirmed jack while a better microphone is available.
pub(crate) fn pick_auto<'a>(
    devices: &'a [ListedMic],
    silent: &dyn Fn(&str) -> bool,
    live: &dyn Fn(&str) -> bool,
) -> &'a ListedMic {
    let inputs: Vec<&'a ListedMic> = devices.iter().filter(|device| !is_output_like(&device.name)).collect();
    let devices: Vec<&'a ListedMic> = if inputs.is_empty() {
        devices.iter().collect()
    } else {
        inputs
    };
    let default = devices.iter().copied().find(|device| device.is_default).unwrap_or(devices[0]);
    let default_rank = mic_rank(&default.name, silent, live);
    if default_rank <= 2 {
        return default;
    }
    let best = devices
        .iter()
        .copied()
        .min_by_key(|device| mic_rank(&device.name, silent, live))
        .unwrap_or(default);
    if mic_rank(&best.name, silent, live) < default_rank {
        best
    } else {
        default
    }
}

fn auto_default(devices: &[ListedMic]) -> &ListedMic {
    pick_auto(devices, &is_silent_mic, &is_live_mic)
}

/// When `active` recorded pure silence, the microphone Bolo moves to next.
/// `None` when there is nothing better (every other mic is silent too).
pub(crate) fn fallback_from<'a>(
    devices: &'a [ListedMic],
    active: &str,
    silent: &dyn Fn(&str) -> bool,
    live: &dyn Fn(&str) -> bool,
) -> Option<&'a ListedMic> {
    let active = normalize_mic_name(active);
    devices
        .iter()
        .filter(|device| normalize_mic_name(&device.name) != active && !silent(&device.name))
        .min_by_key(|device| mic_rank(&device.name, silent, live))
}

/// The microphone to switch to after `active` came back silent, using what
/// Bolo has learned so far.
pub fn silent_fallback(active: &str) -> Option<String> {
    let devices = host_mics(&cpal::default_host()).ok()?;
    fallback_from(&devices, active, &is_silent_mic, &is_live_mic).map(|mic| mic.name.clone())
}

/// The message for a take with no speech. The "check your mic" hint only
/// appears when no other microphone could have heard the user.
pub fn silent_take_message(silent_mic: Option<&str>, fallback: Option<&str>, default_hint: &str) -> String {
    match (silent_mic, fallback) {
        (Some(_), Some(next)) => format!("Switched to {next}. Please say that again."),
        _ => default_hint.to_string(),
    }
}

/// Peak of a take; digital silence (an empty jack) is essentially zero.
/// Open every input briefly and remember which ones deliver pure digital
/// silence (an empty jack, a level of 0). Runs at startup and when devices
/// change, so the very first take already uses a microphone that hears.
pub fn probe_inputs() -> Vec<(String, f32)> {
    let Ok(devices) = list_microphones() else { return Vec::new() };
    let mut out = Vec::new();
    for mic in devices {
        let Ok(session) = start_recording(Some(&mic.name)) else { continue };
        thread::sleep(Duration::from_millis(350));
        let Ok(finished) = session.finish() else { continue };
        // Skip the first 60 ms: some drivers start with a zeroed buffer.
        let skip = (finished.sample_rate as usize * finished.channels.max(1) as usize * 60) / 1000;
        let heard = finished.interleaved.get(skip..).unwrap_or(&[]);
        // An empty buffer means the device never delivered samples (often because
        // the warm stream already has it). That is not proof the mic is dead.
        if heard.is_empty() {
            out.push((mic.name, 0.0));
            continue;
        }
        let peak = peak_level(heard);
        if peak < SILENT_PEAK {
            mark_silent_mic(&mic.name);
        } else {
            mark_live_mic(&mic.name);
        }
        out.push((mic.name, peak));
    }
    out
}

pub fn peak_level(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()))
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
    let default = auto_default(devices);
    let preference = trimmed(preference);
    if let Some(pref) = preference {
        if let Some(found) = find_named(devices, &pref) {
            if is_silent_mic(&found.name) && !is_silent_mic(&default.name) && default.name != found.name {
                return Ok(MicChoice {
                    active_name: default.name.clone(),
                    using_fallback: true,
                    note: Some(format!("{pref} isn't picking up sound. Using {}.", default.name)),
                    preference: Some(pref),
                });
            }
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
    order_avoiding(devices, preference, None)
}

/// Like [`open_order`], but a microphone that just dropped is tried last when
/// another input exists. The avoided name is not marked silent: a stall is not
/// proof the hardware is dead.
pub(crate) fn order_avoiding(devices: &[ListedMic], preference: Option<&str>, avoid: Option<&str>) -> Vec<String> {
    let mut names = order_with(devices, preference, &is_silent_mic, &is_live_mic);
    let Some(avoid) = avoid.map(str::trim).filter(|name| !name.is_empty()) else {
        return names;
    };
    if let Some(index) = names.iter().position(|name| normalize_mic_name(name) == normalize_mic_name(avoid)) {
        if names.len() > 1 {
            let name = names.remove(index);
            names.push(name);
        }
    }
    names
}

/// Preference (unless it is known silent), then the automatic pick, then the
/// rest best-first; microphones known to record silence go last.
pub(crate) fn order_with(
    devices: &[ListedMic],
    preference: Option<&str>,
    silent: &dyn Fn(&str) -> bool,
    live: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let mut names = Vec::new();
    let inputs: Vec<&ListedMic> = devices.iter().filter(|device| !is_output_like(&device.name)).collect();
    let devices: Vec<&ListedMic> = if inputs.is_empty() {
        devices.iter().collect()
    } else {
        inputs
    };
    if let Some(pref) = trimmed(preference) {
        if let Some(found) = devices.iter().find(|device| {
            normalize_mic_name(&device.name) == normalize_mic_name(&pref)
        }) {
            if !silent(&found.name) {
                push_unique(&mut names, &found.name);
            }
        }
    }
    if !devices.is_empty() {
        let owned: Vec<ListedMic> = devices.iter().map(|device| (*device).clone()).collect();
        push_unique(&mut names, &pick_auto(&owned, silent, live).name);
    }
    let mut rest: Vec<&ListedMic> = devices.clone();
    rest.sort_by_key(|device| mic_rank(&device.name, silent, live));
    for device in rest {
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
        if is_output_like(&name) {
            continue;
        }
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
        .name("bolo-audio".into())
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
    if finished.interleaved.is_empty() || finished.sample_rate == 0 || finished.channels == 0 {
        return Err(format!(
            "{device_name} opened but did not deliver any audio"
        ));
    }
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
                if ready_tx
                    .send(Ok(OpenedMic {
                        active_name: name.clone(),
                        used_fallback,
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
    // A virtual or unplugged endpoint can be listed and still have no mix
    // format. cpal dereferences that pointer. Reject it before opening.
    ensure_capture_format(name)?;
    let supported = device
        .default_input_config()
        .with_context(|| format!("{name} has no default input format"))?;
    let channels = supported.channels();
    let sample_rate = supported.sample_rate().0;
    if channels == 0 || sample_rate == 0 {
        return Err(anyhow!(
            "{name} cannot deliver audio (channels={channels}, rate={sample_rate})"
        ));
    }
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
    /// Trimmed speech length. Short clips use a smaller Whisper prompt.
    pub speech_secs: f32,
}

pub fn prepare_speech(finished: &FinishedAudio) -> Result<EncodedSpeech, SpeechError> {
    Ok(prepare_speech_parts(finished)?.0)
}

/// Encoded upload plus the 16 kHz samples it was made from (after gain).
/// Debug capture listens to those samples; the encoder output is unchanged.
pub fn prepare_speech_parts(finished: &FinishedAudio) -> Result<(EncodedSpeech, Vec<f32>), SpeechError> {
    let resampled = speech_samples(finished)?;
    let speech_secs = resampled.len() as f32 / TARGET_RATE as f32;
    let mut encoded = encode_speech(&resampled, TARGET_RATE)?;
    encoded.speech_secs = speech_secs;
    Ok((encoded, resampled))
}

/// Sample rate of the buffer `prepare_speech` uploads.
pub const SPEECH_RATE: u32 = TARGET_RATE;

/// Trimmed, gain-normalized, 16 kHz mono samples ready to encode.
pub fn speech_samples(finished: &FinishedAudio) -> Result<Vec<f32>, SpeechError> {
    if finished.channels == 0 || finished.sample_rate == 0 || finished.interleaved.is_empty() {
        return Err(SpeechError::Empty);
    }
    // DC and rumble, once boosted, look like a vowel to Whisper and come back
    // as the wrong word. A 90 Hz high-pass leaves speech and fricatives.
    let mono = highpass(&downmix(&finished.interleaved, finished.channels), finished.sample_rate);
    let mut trimmed = trim_silence_with(&mono, finished.sample_rate, speech_threshold(finished.noise_rms));
    if trimmed.is_empty() {
        // A mic whose Windows level is very low still carries speech, just
        // far below the fixed floor. Look for a clear rise over the take's own
        // quiet part before calling it silence, so quiet mics still work.
        if let Some(threshold) = relative_threshold(&mono, finished.sample_rate) {
            trimmed = trim_silence_with(&mono, finished.sample_rate, threshold);
        }
    }
    if trimmed.is_empty() || noise_only(&trimmed, finished.sample_rate) {
        return Err(SpeechError::Silent);
    }
    let duration = trimmed.len() as f32 / finished.sample_rate as f32;
    if duration < MIN_SPEECH_SECS {
        return Err(SpeechError::Silent);
    }
    let gained = normalize_gain(&trimmed);
    Ok(resample(&gained, finished.sample_rate, TARGET_RATE))
}

/// Lift the body of the speech, not whatever single sample is loudest.
///
/// Peak gain left a mumbled phrase quiet whenever a plosive or a click set
/// the peak, and it also amplified room hiss by up to 60x when the whole
/// take was quiet. The 90th-percentile amplitude is the speech. A lone click
/// may clip; the words get louder.
pub fn normalize_gain(samples: &[f32]) -> Vec<f32> {
    const BODY_TARGET: f32 = 0.42;
    const PEAK_LIMIT: f32 = 0.92;
    const MAX_GAIN: f32 = 24.0;
    let peak = samples.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    if peak < 0.0003 {
        return samples.to_vec();
    }
    let body = percentile_abs(samples, 0.90);
    if body < 0.0003 {
        return samples.to_vec();
    }
    let mut gain = (BODY_TARGET / body).clamp(1.0, MAX_GAIN);
    // The speech itself is the peak. Don't push that into clipping.
    if peak * gain > PEAK_LIMIT && body > peak * 0.45 {
        gain = (PEAK_LIMIT / peak).clamp(1.0, gain);
    }
    if gain <= 1.001 {
        return samples.to_vec();
    }
    samples
        .iter()
        .map(|sample| (sample * gain).clamp(-PEAK_LIMIT, PEAK_LIMIT))
        .collect()
}

fn percentile_abs(samples: &[f32], quantile: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut levels: Vec<f32> = samples.iter().map(|sample| sample.abs()).collect();
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let index = ((levels.len() - 1) as f32 * quantile.clamp(0.0, 1.0)).round() as usize;
    levels[index.min(levels.len() - 1)]
}

/// One-pole high-pass. Removes DC and low rumble before gain and the speech floor.
fn highpass(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() || sample_rate == 0 {
        return samples.to_vec();
    }
    let rc = 1.0 / (2.0 * std::f32::consts::PI * 90.0);
    let dt = 1.0 / sample_rate as f32;
    let alpha = rc / (rc + dt);
    let mut out = Vec::with_capacity(samples.len());
    let mut prev_x = 0.0f32;
    let mut prev_y = 0.0f32;
    for &sample in samples {
        let y = alpha * (prev_y + sample - prev_x);
        out.push(y);
        prev_x = sample;
        prev_y = y;
    }
    out
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

/// Energy that is still a quiet consonant, not a pause.
///
/// Chunking used the vowel floor, so a soft ending counted as silence and a
/// long take could cut the last sound off. A real pause sits under this.
pub fn soft_speech_floor(threshold: f32) -> f32 {
    (threshold * 0.35).clamp(0.0016, 0.008)
}

/// Advance the pause clock. Soft speech resets it so a quiet ending is not a boundary.
pub fn advance_pause_ms(previous_ms: u32, energy: f32, threshold: f32, frame_ms: u32) -> u32 {
    if energy >= soft_speech_floor(threshold) {
        0
    } else {
        previous_ms.saturating_add(frame_ms.max(1))
    }
}

/// Speech floor taken from the take itself: frames clearly above the quiet
/// 20th percentile count as speech. `None` when the take is flat (digital
/// silence or steady noise), so silence never reaches the transcriber.
pub fn relative_threshold(samples: &[f32], sample_rate: u32) -> Option<f32> {
    let frame = ((sample_rate as usize) * FRAME_MS as usize) / 1000;
    if frame == 0 || samples.len() < frame * 4 {
        return None;
    }
    let mut levels: Vec<f32> = samples.chunks(frame).map(rms).collect();
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = levels[levels.len() / 5].max(0.00002);
    let loud = levels[(levels.len() * 95) / 100];
    if loud < 0.0004 || loud < floor * 6.0 {
        return None;
    }
    Some((floor * 3.0).max(loud * 0.08).max(0.0002))
}

/// Steady hiss or room noise: energy with no vowel-like (low zero-crossing)
/// frame. A tone or a spoken vowel crosses zero slowly and is kept, so a
/// quiet mic still works. Silence and this noise never reach Whisper, which
/// is what was inventing "thank you" and echoing the prompt.
pub fn noise_only(samples: &[f32], sample_rate: u32) -> bool {
    let frame = ((sample_rate as usize) * FRAME_MS as usize) / 1000;
    if frame == 0 || samples.len() < frame * 8 {
        return false;
    }
    let mut frames = 0usize;
    let mut vowelish = 0usize;
    let mut noisy = 0usize;
    for chunk in samples.chunks(frame) {
        if chunk.len() < frame / 2 {
            break;
        }
        frames += 1;
        let energy = rms(chunk);
        if energy < 0.004 {
            continue;
        }
        let rate = zero_cross_rate(chunk);
        if rate < 0.12 {
            vowelish += 1;
        }
        if rate > 0.25 {
            noisy += 1;
        }
    }
    if !(frames >= 8 && vowelish == 0 && noisy * 2 >= frames) {
        return false;
    }
    // A spoken "ssh" is a short loud burst with quiet edges. Steady hiss is
    // flat from the first sample to the last, and that is what we drop.
    !fricative_burst(samples, sample_rate)
}

fn fricative_burst(samples: &[f32], sample_rate: u32) -> bool {
    let n = samples.len();
    let rate = sample_rate as usize;
    if rate == 0 || n < rate / 10 || n > rate * 2 {
        return false;
    }
    let third = (n / 3).max(1);
    if third >= n {
        return false;
    }
    let edge = rms(&samples[..third]).max(rms(&samples[n - third..]));
    let mid = rms(&samples[third..n - third]);
    mid >= 0.004 && mid > edge * 1.8
}

/// Vowels clear the energy floor. Fricatives ("s", "sh") are quieter and
/// cross zero quickly; a vowel-only floor was cutting them off "easy ssh".
fn frame_is_speech(chunk: &[f32], threshold: f32) -> bool {
    let energy = rms(chunk);
    if energy >= threshold {
        return true;
    }
    let floor = (threshold * 0.45).max(0.0022);
    energy >= floor && zero_cross_rate(chunk) >= 0.16
}

fn zero_cross_rate(chunk: &[f32]) -> f32 {
    if chunk.len() < 2 {
        return 0.0;
    }
    let mut crosses = 0usize;
    for pair in chunk.windows(2) {
        if (pair[0] >= 0.0 && pair[1] < 0.0) || (pair[0] < 0.0 && pair[1] >= 0.0) {
            crosses += 1;
        }
    }
    crosses as f32 / chunk.len() as f32
}

/// Drop leading and trailing silence. Pauses in the middle of a sentence stay.
#[cfg(test)]
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
        .map(|chunk| frame_is_speech(chunk, threshold))
        .collect();
    // Onset needs two voiced frames so a click is not speech. A short hangover
    // keeps a quiet consonant that follows the last loud frame.
    let Some((first, last)) = speech_bounds(&voiced) else {
        return Vec::new();
    };
    // A quiet consonant after the last loud frame is still the word. Two
    // silent frames end it, so a real pause in the middle is not filled in.
    let last = extend_quiet_tail(samples, frame, threshold, last);
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

/// How far a decaying consonant may run past the last loud frame (20 ms frames).
const QUIET_TAIL_FRAMES: usize = 12;

fn extend_quiet_tail(samples: &[f32], frame: usize, threshold: f32, last: usize) -> usize {
    if frame == 0 {
        return last;
    }
    let mut end = last;
    let mut missed = 0usize;
    let limit = (last + QUIET_TAIL_FRAMES).min(samples.len() / frame);
    let mut index = last + 1;
    while index < limit {
        let start = index * frame;
        let stop = ((index + 1) * frame).min(samples.len());
        if start >= stop {
            break;
        }
        if soft_frame(&samples[start..stop], threshold) {
            end = index;
            missed = 0;
        } else {
            missed += 1;
            if missed >= 2 {
                break;
            }
        }
        index += 1;
    }
    end
}

fn soft_frame(chunk: &[f32], threshold: f32) -> bool {
    let energy = rms(chunk);
    if energy < soft_speech_floor(threshold) {
        return false;
    }
    energy >= threshold * 0.45 || zero_cross_rate(chunk) >= 0.11
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
            return Ok(EncodedSpeech { bytes, format: "flac", speech_secs: 0.0 });
        }
    }
    let bytes = encode_wav(samples, sample_rate).map_err(|err| SpeechError::Encode(err.to_string()))?;
    Ok(EncodedSpeech { bytes, format: "wav", speech_secs: 0.0 })
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
                move |data: &[f32], _| on_samples(capture_samples(data, |sample| sample)),
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
                    on_samples(capture_samples(data, |sample| sample as f32 / i16::MAX as f32))
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
                    on_samples(capture_samples(data, |sample| {
                        (sample as f32 / u16::MAX as f32) * 2.0 - 1.0
                    }))
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
                    on_samples(capture_samples(data, |sample| sample as f32 / i32::MAX as f32))
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
                move |data: &[f64], _| on_samples(capture_samples(data, |sample| sample as f32)),
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
                move |data: &[f32], _| {
                    push_samples(&audio, &stop, capture_samples(data, |sample| sample))
                },
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
                    push_samples(
                        &audio,
                        &stop,
                        capture_samples(data, |sample| sample as f32 / i16::MAX as f32),
                    )
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
                        capture_samples(data, |sample| (sample as f32 / u16::MAX as f32) * 2.0 - 1.0),
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
                    push_samples(
                        &audio,
                        &stop,
                        capture_samples(data, |sample| sample as f32 / i32::MAX as f32),
                    )
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
                move |data: &[f64], _| {
                    push_samples(&audio, &stop, capture_samples(data, |sample| sample as f32))
                },
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

/// One WASAPI period. Larger than this is not a real packet; a null endpoint
/// has been seen to report a nonsense frame count with a null pointer.
const MAX_CAPTURE_PACKET: usize = 48_000 * 8;

/// Copy one capture callback. WASAPI sets the data pointer to null for a
/// silence packet (`AUDCLNT_BUFFERFLAGS_SILENT`) while still reporting a frame
/// count. Reading that pointer is an access violation (0xC0000005). Treat it
/// as silence instead of touching the pointer.
fn capture_samples<T: Copy>(packet: &[T], convert: impl Fn(T) -> f32) -> Vec<f32> {
    let ptr = packet.as_ptr();
    read_capture_packet(ptr, packet.len(), |index| convert(unsafe { *ptr.add(index) }))
}

fn read_capture_packet<T>(ptr: *const T, len: usize, read: impl Fn(usize) -> f32) -> Vec<f32> {
    let len = len.min(MAX_CAPTURE_PACKET);
    if len == 0 || ptr.is_null() {
        return vec![0.0; len];
    }
    (0..len).map(read).collect()
}

/// A listed endpoint can still be unable to capture: no mix format, or a
/// format with no channels. cpal dereferences the mix-format pointer, which
/// access-violates on a null or virtual device. Check it first and return an
/// error the caller can show.
fn ensure_capture_format(name: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        windows_capture_format(name)
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        Ok(())
    }
}

#[cfg(windows)]
fn windows_capture_format(name: &str) -> anyhow::Result<()> {
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::{
        eCapture, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::core::GUID;

    // Same key cpal uses for Device::name (DEVPKEY_Device_FriendlyName).
    const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    unsafe {
        // Already initialized on this thread is fine. A different apartment
        // model is also fine; cpal initializes COM again before it opens.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator = CoCreateInstance::<_, IMMDeviceEnumerator>(
            &MMDeviceEnumerator,
            None::<&windows::core::IUnknown>,
            CLSCTX_ALL,
        )
        .context("couldn't list capture endpoints")?;
        let collection = enumerator
            .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
            .context("couldn't list capture endpoints")?;
        let count = collection.GetCount().context("couldn't count capture endpoints")?;
        let wanted = normalize_mic_name(name);
        for index in 0..count {
            let device = collection.Item(index)?;
            let Some(found) = endpoint_friendly_name(&device, &FRIENDLY_NAME) else {
                continue;
            };
            if normalize_mic_name(&found) != wanted {
                continue;
            }
            let client = device
                .Activate::<IAudioClient>(CLSCTX_ALL, None)
                .with_context(|| format!("{name} cannot deliver audio"))?;
            let format = client
                .GetMixFormat()
                .with_context(|| format!("{name} has no capture format"))?;
            if format.is_null() {
                return Err(anyhow!("{name} cannot deliver audio"));
            }
            let (channels, rate, align, bits) = {
                let header = &*format;
                (
                    header.nChannels,
                    header.nSamplesPerSec,
                    header.nBlockAlign,
                    header.wBitsPerSample,
                )
            };
            CoTaskMemFree(Some(format as *const std::ffi::c_void));
            if channels == 0 || rate == 0 || align == 0 || bits == 0 {
                return Err(anyhow!(
                    "{name} cannot deliver audio (channels={channels}, rate={rate})"
                ));
            }
            return Ok(());
        }
    }
    // cpal will report a missing name. Don't hide a device we failed to match.
    Ok(())
}

#[cfg(windows)]
pub(crate) unsafe fn friendly_name(
    device: &windows::Win32::Media::Audio::IMMDevice,
    key: &windows::Win32::Foundation::PROPERTYKEY,
) -> Option<String> {
    endpoint_friendly_name(device, key)
}

#[cfg(windows)]
unsafe fn endpoint_friendly_name(
    device: &windows::Win32::Media::Audio::IMMDevice,
    key: &windows::Win32::Foundation::PROPERTYKEY,
) -> Option<String> {
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::System::Variant::VT_LPWSTR;

    let store = device.OpenPropertyStore(STGM_READ).ok()?;
    let value = store.GetValue(key).ok()?;
    let variant = &value.Anonymous.Anonymous;
    if variant.vt != VT_LPWSTR {
        return None;
    }
    let ptr = variant.Anonymous.pwszVal.0 as *const u16;
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
        if len > 1024 {
            return None;
        }
    }
    let words = std::slice::from_raw_parts(ptr, len);
    Some(String::from_utf16_lossy(words))
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
    fn quiet_mic_speech_is_kept_but_flat_noise_is_not() {
        let rate = 16_000u32;
        let mut quiet = vec![0.00005f32; rate as usize / 2];
        quiet.extend((0..rate as usize).map(|i| (i as f32 * 0.05).sin() * 0.003));
        quiet.extend(vec![0.00005f32; rate as usize / 2]);
        let finished = FinishedAudio { interleaved: quiet, channels: 1, sample_rate: rate, elapsed_ms: 2000, noise_rms: 0.0 };
        assert!(speech_samples(&finished).is_ok(), "quiet speech must survive");
        let flat = FinishedAudio { interleaved: vec![0.0003f32; rate as usize * 2], channels: 1, sample_rate: rate, elapsed_ms: 2000, noise_rms: 0.0 };
        assert!(matches!(speech_samples(&flat), Err(SpeechError::Silent)));
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

    /// Synthetic fixture: a loud vowel, a quiet consonant under the vowel floor,
    /// then silence. Not a recording of a person.
    #[test]
    fn a_quiet_ending_stays_and_a_pause_does_not_fill_in() {
        let rate = 16_000u32;
        let mut samples = vec![0.0; rate as usize / 5];
        samples.extend(tone(rate, 200, 0.25));
        let mut state = 0x00C0_FFEEu32;
        let quiet: Vec<f32> = (0..(rate as usize * 160 / 1000))
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state as f32 / u32::MAX as f32 - 0.5) * 0.012
            })
            .collect();
        let quiet_rms = rms(&quiet);
        let threshold = speech_threshold(0.0);
        assert!(quiet_rms < threshold, "fixture must sit under the vowel floor, rms {quiet_rms}");
        assert!(
            quiet_rms >= soft_speech_floor(threshold),
            "fixture must still be a consonant, rms {quiet_rms}"
        );
        samples.extend(quiet);
        samples.extend(vec![0.0; rate as usize / 5]);
        let trimmed = trim_silence_with(&samples, rate, threshold);
        let secs = trimmed.len() as f32 / rate as f32;
        assert!(secs > 0.30, "the quiet ending should stay on the clip, got {secs:.3}s");

        let mut paused = tone(rate, 200, 0.25);
        paused.extend(vec![0.0; rate as usize / 5]);
        paused.extend(tone(rate, 200, 0.25));
        let kept = trim_silence_with(&paused, rate, threshold);
        assert!(kept.len() > rate as usize * 400 / 1000, "a middle pause stays between the words");

        let threshold = speech_threshold(0.0);
        assert_eq!(advance_pause_ms(200, 0.02, threshold, 20), 0, "a vowel is not a pause");
        assert_eq!(
            advance_pause_ms(200, quiet_rms, threshold, 20),
            0,
            "a quiet ending must not open a chunk boundary"
        );
        assert!(advance_pause_ms(260, 0.0002, threshold, 20) >= 280);
        assert!(!crate::chunk::should_cut(20_000, advance_pause_ms(0, quiet_rms, threshold, 280)));
        assert!(crate::chunk::should_cut(20_000, advance_pause_ms(260, 0.0002, threshold, 20)));
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
        let mut mumbled = tone(16_000, 180, 0.02);
        for sample in mumbled.iter_mut().take(6) {
            *sample = 0.95;
        }
        let lifted = normalize_gain(&mumbled);
        let body = lifted[32..].iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!(body > 0.15, "a click must not leave the mumbled phrase quiet, body {body}");
        assert!(lifted.iter().all(|sample| sample.abs() <= 0.93));
    }

    #[test]
    fn dc_offset_is_not_sent_to_whisper() {
        let rate = 16_000u32;
        let dc = FinishedAudio {
            interleaved: vec![0.2; rate as usize],
            channels: 1,
            sample_rate: rate,
            elapsed_ms: 1000,
            noise_rms: 0.0,
        };
        assert!(
            matches!(speech_samples(&dc), Err(SpeechError::Silent)),
            "a DC offset used to look like a vowel and come back as the wrong words"
        );
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

    #[test]
    fn a_trailing_fricative_is_kept_and_a_flat_hiss_is_not() {
        let rate = 16_000u32;
        let mut samples = vec![0.0; rate as usize / 5];
        samples.extend(tone(rate, 200, 0.25));
        let mut state = 0x1234_5678u32;
        let fricative: Vec<f32> = (0..(rate as usize * 400 / 1000))
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state as f32 / u32::MAX as f32 - 0.5) * 0.012
            })
            .collect();
        let fricative_rms = rms(&fricative);
        assert!(fricative_rms < speech_threshold(0.0), "fixture should sit under the vowel floor, rms {fricative_rms}");
        samples.extend(fricative);
        samples.extend(vec![0.0; rate as usize / 5]);
        let trimmed = trim_silence_with(&samples, rate, speech_threshold(0.0));
        let secs = trimmed.len() as f32 / rate as f32;
        assert!(secs > 0.7, "the quiet ssh tail should stay on the clip, got {secs:.3}s");

        let mut burst = vec![0.0; rate as usize / 5];
        burst.extend((0..(rate as usize * 350 / 1000)).map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state as f32 / u32::MAX as f32 - 0.5) * 0.08
        }));
        burst.extend(vec![0.0; rate as usize / 5]);
        let ssh = FinishedAudio {
            interleaved: burst,
            channels: 1,
            sample_rate: rate,
            elapsed_ms: 800,
            noise_rms: 0.0,
        };
        assert!(speech_samples(&ssh).is_ok(), "a spoken ssh burst is speech");
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
        let none = |_: &str| false;
        assert_eq!(
            order_with(&devices, None, &none, &none),
            vec![laptop().name, usb.name.clone(), headset().name]
        );
        assert_eq!(
            order_with(&devices, Some("Not A Real Mic"), &none, &none),
            vec![laptop().name, usb.name, headset().name]
        );
    }

    fn jack_default() -> Vec<ListedMic> {
        vec![
            ListedMic { name: "Jack Mic (Realtek(R) Audio)".into(), is_default: true },
            ListedMic { name: "Microphone Array (Realtek(R) Audio)".into(), is_default: false },
        ]
    }

    #[test]
    fn unconfirmed_jack_default_prefers_the_built_in_array() {
        let devices = jack_default();
        let none = |_: &str| false;
        assert_eq!(pick_auto(&devices, &none, &none).name, devices[1].name);
        // A jack that has proven it hears (a plugged-in headset) stays the default.
        let jack_live = |n: &str| is_jack_like(n);
        assert_eq!(pick_auto(&devices, &none, &jack_live).name, devices[0].name);
        // A normal default (USB mic) is still followed as before.
        let usb = vec![
            ListedMic { name: "USB Studio Mic".into(), is_default: true },
            ListedMic { name: "Microphone Array".into(), is_default: false },
        ];
        assert_eq!(pick_auto(&usb, &none, &none).name, "USB Studio Mic");
    }

    #[test]
    fn silent_default_falls_back_to_a_mic_with_signal() {
        let devices = vec![
            ListedMic { name: "Headset Microphone".into(), is_default: true },
            ListedMic { name: "Microphone Array".into(), is_default: false },
        ];
        let silent = |n: &str| n == "Headset Microphone";
        let live = |n: &str| n == "Microphone Array";
        assert_eq!(pick_auto(&devices, &silent, &live).name, "Microphone Array");
        assert_eq!(
            order_with(&devices, None, &silent, &live),
            vec!["Microphone Array".to_string(), "Headset Microphone".to_string()]
        );
        // A saved microphone that records zeros is tried last, not first.
        assert_eq!(
            order_with(&devices, Some("Headset Microphone"), &silent, &live),
            vec!["Microphone Array".to_string(), "Headset Microphone".to_string()]
        );
        assert_eq!(
            fallback_from(&devices, "Headset Microphone", &silent, &live).map(|m| m.name.as_str()),
            Some("Microphone Array")
        );
    }

    #[test]
    fn a_stalled_mic_is_tried_after_the_others() {
        let devices = vec![
            ListedMic { name: "Stall Mic A (bolo-order)".into(), is_default: true },
            ListedMic { name: "Stall Mic B (bolo-order)".into(), is_default: false },
        ];
        assert_eq!(
            order_avoiding(&devices, None, Some("stall mic a (bolo-order)")),
            vec![
                "Stall Mic B (bolo-order)".to_string(),
                "Stall Mic A (bolo-order)".to_string(),
            ]
        );
        let only = vec![ListedMic { name: "Stall Mic A (bolo-order)".into(), is_default: true }];
        assert_eq!(
            order_avoiding(&only, None, Some("Stall Mic A (bolo-order)")),
            vec!["Stall Mic A (bolo-order)".to_string()],
            "the only microphone is still opened"
        );
    }

    #[test]
    fn every_mic_silent_has_no_fallback_and_keeps_the_hint() {
        let devices = jack_default();
        let all = |_: &str| true;
        let none = |_: &str| false;
        assert!(fallback_from(&devices, &devices[0].name, &all, &none).is_none());
        assert_eq!(pick_auto(&devices, &all, &none).name, devices[0].name);
        let hint = "Didn't catch any speech.";
        assert_eq!(silent_take_message(Some("Jack Mic"), None, hint), hint);
        assert_eq!(silent_take_message(None, None, hint), hint);
        assert_eq!(
            silent_take_message(Some("Jack Mic"), Some("Microphone Array"), hint),
            "Switched to Microphone Array. Please say that again."
        );
    }

    #[test]
    fn jack_and_builtin_names_are_recognised() {
        assert!(is_jack_like("Jack Mic (Realtek(R) Audio)"));
        assert!(is_jack_like("Line In (High Definition Audio)"));
        assert!(!is_jack_like("Microphone Array (Intel Smart Sound)"));
        assert!(is_builtin_like("Microphone Array (2- Realtek(R) Audio)"));
        assert!(is_builtin_like("Microphone Array (Realtek(R) Audio)"));
        assert!(is_output_like("Speakers (2- Realtek(R) Audio)"));
        assert!(is_output_like("Headphones (Realtek(R) Audio)"));
        assert!(is_output_like("Stereo Mix (Realtek(R) Audio)"));
        assert!(!is_output_like("Microphone Array (2- Realtek(R) Audio)"));
        assert!(!is_output_like("Headset Microphone"));
        assert!(is_builtin_like("Internal Microphone"));
        assert!(!is_builtin_like("USB Studio Mic"));
    }

    #[test]
    fn no_devices_is_an_error_and_does_not_invent_one() {
        let err = resolve_microphone(&[], Some("Jack Mic")).unwrap_err();
        assert!(err.contains("No microphone"));
        assert!(open_order(&[], Some("Jack Mic")).is_empty());
    }

    #[test]
    fn a_null_capture_packet_is_silence() {
        let samples = read_capture_packet(std::ptr::null::<f32>(), 6, |_| unreachable!("null packet was read"));
        assert_eq!(samples, vec![0.0; 6]);
        let tone = [0.25f32, -0.5];
        assert_eq!(capture_samples(&tone, |sample| sample), vec![0.25, -0.5]);
        assert!(read_capture_packet(std::ptr::null::<i16>(), 0, |_| unreachable!()).is_empty());
    }

    #[test]
    fn records_when_a_microphone_exists() {
        if hosted_ci() {
            eprintln!("SKIP records_when_a_microphone_exists: CI=true");
            return;
        }
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
        let report = match capture_for(None, Duration::from_millis(400)) {
            Ok(report) => report,
            Err(err) => {
                eprintln!("SKIP records_when_a_microphone_exists: {err}");
                return;
            }
        };
        assert!(!report.device_name.is_empty());
        assert!(report.sample_rate > 0);
        assert!(report.channels > 0);
        assert!(report.sample_count > 0);
    }

    #[test]
    fn listening_overlay_is_under_100ms_and_keeps_the_first_200ms() {
        if hosted_ci() {
            eprintln!("SKIP listening_overlay_is_under_100ms_and_keeps_the_first_200ms: CI=true");
            return;
        }
        let warm = crate::warm::WarmMic::spawn(None, None);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !warm.is_live() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if !warm.is_live() {
            eprintln!("SKIP listening_overlay_is_under_100ms_and_keeps_the_first_200ms: no usable capture device");
            return;
        }
        std::thread::sleep(Duration::from_millis(80));
        let keydown = Instant::now();
        let take = match warm.begin(None) {
            Ok(take) => take,
            Err(err) => {
                eprintln!("SKIP listening_overlay_is_under_100ms_and_keeps_the_first_200ms: {err}");
                return;
            }
        };
        // Listening is published before this reply. The reply is the hotkey's
        // only wait, and it must not include a device open.
        let overlay_ms = keydown.elapsed().as_millis() as u64;
        eprintln!("MEASURE keydown_overlay_ms={overlay_ms}");
        assert!(
            overlay_ms < 100,
            "Listening took {overlay_ms} ms; key-down must only mark the warm buffer"
        );
        std::thread::sleep(Duration::from_millis(280));
        let first_sample_ms = take.first_sample_ms_since(keydown);
        let pre_roll = take.pre_roll_samples;
        eprintln!(
            "MEASURE keydown_first_sample_ms={}",
            first_sample_ms.map(|ms| ms.to_string()).unwrap_or_else(|| "na".into())
        );
        let finished = take.finish().expect("the warm take should stop");
        let channels = usize::from(finished.channels.max(1));
        let new_samples = finished.interleaved.len().saturating_sub(pre_roll);
        let new_frames = new_samples / channels;
        let new_ms = new_frames as u64 * 1000 / u64::from(finished.sample_rate.max(1));
        if let Some(ms) = first_sample_ms {
            assert!(ms < 200, "first sample arrived {ms} ms after key-down");
        } else {
            panic!("no sample arrived after key-down");
        }
        assert!(
            new_ms >= 200,
            "only {new_ms} ms of audio was captured after key-down (pre-roll {pre_roll} samples)"
        );
    }

    fn hosted_ci() -> bool {
        matches!(
            std::env::var("CI").ok().as_deref().map(|value| value.eq_ignore_ascii_case("true") || value == "1"),
            Some(true)
        )
    }

    #[test]
    fn missing_microphone_falls_back_when_a_device_exists() {
        if hosted_ci() {
            eprintln!("SKIP missing_microphone_falls_back_when_a_device_exists: CI=true");
            return;
        }
        let mics = match list_microphones() {
            Ok(mics) if !mics.is_empty() => mics,
            _ => {
                eprintln!("SKIP missing_microphone_falls_back_when_a_device_exists: no input device");
                return;
            }
        };
        let missing = "Bolo Missing Mic 9000";
        let report = match capture_for(Some(missing), Duration::from_millis(400)) {
            Ok(report) => report,
            Err(err) => {
                eprintln!("SKIP missing_microphone_falls_back_when_a_device_exists: {err}");
                return;
            }
        };
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
