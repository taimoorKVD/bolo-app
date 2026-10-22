//! A microphone stream that stays open between takes.
//!
//! Opening WASAPI on key-down cost ~180 ms and clipped the first words. This
//! thread opens the preferred device (or the current default) at startup,
//! keeps a short pre-roll, and starts a take by copying that buffer. Device
//! changes are picked up while idle, using the same fallback order as a
//! one-shot recording.

use crate::audio::{self, FinishedAudio};
use cpal::traits::{HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PRE_ROLL_MS: u32 = 350;
const LOOP_SLICE: Duration = Duration::from_millis(30);

enum Cmd {
    Retarget(Option<String>),
    Begin {
        preference: Option<String>,
        reply: Sender<Result<Armed, String>>,
    },
    End {
        id: u64,
        reply: Sender<Result<FinishedAudio, String>>,
    },
}

struct Armed {
    id: u64,
    active_name: String,
    used_fallback: bool,
    note: String,
    level: Arc<AtomicU32>,
}

pub struct LiveTake {
    id: u64,
    cmd: Sender<Cmd>,
    pub device_note: Option<String>,
    /// The device this take is reading. Logged by the warm thread's note.
    #[allow(dead_code)]
    pub active_name: String,
    pub used_fallback: bool,
    pub started: Instant,
    pub open_wait_ms: u64,
    stop: Arc<std::sync::atomic::AtomicBool>,
    level: Arc<AtomicU32>,
}

impl LiveTake {
    pub fn meter(&self) -> (Arc<AtomicU32>, Arc<std::sync::atomic::AtomicBool>) {
        (Arc::clone(&self.level), Arc::clone(&self.stop))
    }

    pub fn finish(self) -> Result<FinishedAudio, String> {
        self.stop.store(true, Ordering::Relaxed);
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd
            .send(Cmd::End {
                id: self.id,
                reply: reply_tx,
            })
            .map_err(|_| "Microphone watcher stopped.".to_string())?;
        match reply_rx.recv_timeout(Duration::from_millis(500)) {
            Ok(result) => result,
            Err(_) => Err("Stopping the microphone timed out.".into()),
        }
    }
}

pub struct WarmMic {
    cmd: Sender<Cmd>,
}

impl WarmMic {
    pub fn spawn(preference: Option<String>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let initial = preference.clone();
        let _ = std::thread::Builder::new().name("cadence-warm".into()).spawn(move || {
            run(cmd_rx, initial);
        });
        // Ask the thread to open this preference even if spawn raced.
        let _ = cmd_tx.send(Cmd::Retarget(preference));
        Self { cmd: cmd_tx }
    }

    pub fn retarget(&self, preference: Option<String>) {
        let _ = self.cmd.send(Cmd::Retarget(preference));
    }

    pub fn begin(&self, preference: Option<&str>) -> Result<LiveTake, String> {
        let waited = Instant::now();
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd
            .send(Cmd::Begin {
                preference: preference.map(str::to_string),
                reply: reply_tx,
            })
            .map_err(|_| "Microphone watcher stopped.".to_string())?;
        let armed = match reply_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(result) => result?,
            Err(_) => return Err("The microphone didn't respond in time. Try the key again.".into()),
        };
        Ok(LiveTake {
            id: armed.id,
            cmd: self.cmd.clone(),
            device_note: Some(armed.note),
            active_name: armed.active_name,
            used_fallback: armed.used_fallback,
            started: Instant::now(),
            open_wait_ms: waited.elapsed().as_millis() as u64,
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            level: armed.level,
        })
    }
}

struct Capture {
    ring: VecDeque<f32>,
    cap: usize,
    take: Option<Vec<f32>>,
    take_id: u64,
    channels: u16,
    sample_rate: u32,
    last_rms: f32,
    noise_rms: f32,
    error: Option<String>,
    level: Arc<AtomicU32>,
}

impl Capture {
    fn new() -> Self {
        Self {
            ring: VecDeque::new(),
            cap: 16_000,
            take: None,
            take_id: 0,
            channels: 1,
            sample_rate: 16_000,
            last_rms: 0.0,
            noise_rms: 0.004,
            error: None,
            level: Arc::new(AtomicU32::new(0)),
        }
    }

    fn set_format(&mut self, channels: u16, sample_rate: u32) {
        self.channels = channels.max(1);
        self.sample_rate = sample_rate.max(1);
        let samples = (u64::from(self.sample_rate) * u64::from(self.channels) * u64::from(PRE_ROLL_MS) / 1000) as usize;
        self.cap = samples.max(1);
        self.ring.clear();
        self.take = None;
    }
}

fn run(cmd_rx: Receiver<Cmd>, mut preference: Option<String>) {
    let host = cpal::default_host();
    let state = Arc::new(Mutex::new(Capture::new()));
    let mut stream: Option<cpal::Stream> = None;
    let mut active_name = String::new();
    let mut used_fallback = false;
    let mut note = String::new();
    let mut next_id = 1u64;
    let mut last_scan = Instant::now() - Duration::from_secs(2);

    loop {
        let cmd = match cmd_rx.try_recv() {
            Ok(cmd) => Some(cmd),
            Err(mpsc::TryRecvError::Empty) => match cmd_rx.recv_timeout(LOOP_SLICE) {
                Ok(cmd) => Some(cmd),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
            Err(mpsc::TryRecvError::Disconnected) => break,
        };
        if let Some(cmd) = cmd {
            handle_cmd(
                cmd,
                &host,
                &state,
                &mut preference,
                &mut stream,
                &mut active_name,
                &mut used_fallback,
                &mut note,
                &mut next_id,
            );
            continue;
        }

        let recording = state.lock().ok().is_some_and(|guard| guard.take.is_some());
        let broken = state.lock().ok().and_then(|guard| guard.error.clone()).is_some();
        if !recording && (stream.is_none() || broken || last_scan.elapsed() >= Duration::from_secs(1)) {
            last_scan = Instant::now();
            if stream.is_none() || broken || device_should_change(&host, &preference, &active_name) {
                let _ = reopen(
                    &host,
                    &preference,
                    &state,
                    &mut stream,
                    &mut active_name,
                    &mut used_fallback,
                    &mut note,
                );
            }
        }
    }
}

fn handle_cmd(
    cmd: Cmd,
    host: &cpal::Host,
    state: &Arc<Mutex<Capture>>,
    preference: &mut Option<String>,
    stream: &mut Option<cpal::Stream>,
    active_name: &mut String,
    used_fallback: &mut bool,
    note: &mut String,
    next_id: &mut u64,
) {
    match cmd {
        Cmd::Retarget(next) => *preference = next,
        Cmd::Begin { preference: asked, reply } => {
            // None means system default. Always replace so a saved headset
            // name does not stick after the user switches back to auto.
            *preference = asked;
            let broken = state.lock().ok().and_then(|guard| guard.error.clone()).is_some();
            if stream.is_none() || broken {
                let _ = reopen(host, preference, state, stream, active_name, used_fallback, note);
            }
            let _ = reply.send(arm_take(state, next_id, active_name, *used_fallback, note));
        }
        Cmd::End { id, reply } => {
            let _ = reply.send(disarm_take(state, id));
        }
    }
}

fn device_should_change(host: &cpal::Host, preference: &Option<String>, active_name: &str) -> bool {
    let Ok(devices) = audio::host_mics(host) else {
        return false;
    };
    if devices.is_empty() {
        return active_name.is_empty();
    }
    let Ok(choice) = audio::resolve_microphone(&devices, preference.as_deref()) else {
        return false;
    };
    audio::normalize_mic_name(&choice.active_name) != audio::normalize_mic_name(active_name)
}

fn reopen(
    host: &cpal::Host,
    preference: &Option<String>,
    state: &Arc<Mutex<Capture>>,
    stream: &mut Option<cpal::Stream>,
    active_name: &mut String,
    used_fallback: &mut bool,
    note: &mut String,
) -> Result<(), String> {
    *stream = None;
    if let Ok(mut guard) = state.lock() {
        guard.error = None;
    }
    let devices = audio::host_mics(host).map_err(|err| err.to_string())?;
    let order = audio::open_order(&devices, preference.as_deref());
    if order.is_empty() {
        active_name.clear();
        return Err("No microphone is available.".into());
    }
    let mut failures = Vec::new();
    for name in &order {
        match open_one(host, name, state) {
            Ok(opened) => {
                let pref = preference.as_deref().map(str::trim).filter(|value| !value.is_empty());
                let opened_preference =
                    pref.is_some_and(|pref| audio::normalize_mic_name(pref) == audio::normalize_mic_name(name));
                let skipped_first = order.first().map(String::as_str) != Some(name.as_str());
                *used_fallback = match pref {
                    Some(_) => !opened_preference,
                    None => skipped_first,
                };
                *note = audio::usage_line(pref, name, *used_fallback);
                *active_name = name.clone();
                *stream = Some(opened);
                return Ok(());
            }
            Err(err) => failures.push(format!("{name}: {err:#}")),
        }
    }
    active_name.clear();
    Err(if failures.is_empty() {
        "Couldn't open a microphone.".into()
    } else {
        format!("Couldn't open a microphone. {}", failures.join(" "))
    })
}

fn open_one(host: &cpal::Host, name: &str, state: &Arc<Mutex<Capture>>) -> anyhow::Result<cpal::Stream> {
    use cpal::traits::DeviceTrait;
    let wanted = audio::normalize_mic_name(name);
    let device = host
        .input_devices()?
        .find(|device| {
            device
                .name()
                .ok()
                .map(|found| audio::normalize_mic_name(&found) == wanted)
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow::anyhow!("microphone \"{name}\" disappeared"))?;
    let supported = device
        .default_input_config()
        .map_err(|err| anyhow::anyhow!("{name} has no default input format: {err}"))?;
    let channels = supported.channels();
    let sample_rate = supported.sample_rate().0;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    if let Ok(mut guard) = state.lock() {
        guard.set_format(channels, sample_rate);
        guard.error = None;
    }
    let audio_state = Arc::clone(state);
    let err_state = Arc::clone(state);
    let stream = audio::build_input_callback(
        &device,
        &config,
        sample_format,
        move |samples| push_samples(&audio_state, samples),
        move |err| {
            if let Ok(mut guard) = err_state.lock() {
                guard.error = Some(format!("Microphone error: {err}"));
            }
        },
    )?;
    stream.play().map_err(|err| anyhow::anyhow!("couldn't start {name}: {err}"))?;
    // Give the callback a moment to report an immediate invalidation.
    let bridge = Arc::clone(state);
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(20));
        if let Some(err) = bridge.lock().ok().and_then(|guard| guard.error.clone()) {
            return Err(anyhow::anyhow!(err));
        }
    }
    Ok(stream)
}

fn push_samples(state: &Mutex<Capture>, samples: impl IntoIterator<Item = f32>) {
    let Ok(mut state) = state.lock() else {
        return;
    };
    if state.error.is_some() {
        return;
    }
    let mut chunk = Vec::new();
    for sample in samples {
        state.ring.push_back(sample);
        while state.ring.len() > state.cap {
            state.ring.pop_front();
        }
        if let Some(take) = state.take.as_mut() {
            if take.len() < audio::MAX_SAMPLES {
                take.push(sample);
            }
        }
        chunk.push(sample);
        if chunk.len() >= 480 {
            let energy = rms(&chunk);
            note_energy(&mut state, energy);
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        let energy = rms(&chunk);
        note_energy(&mut state, energy);
    }
}

fn note_energy(state: &mut Capture, energy: f32) {
    state.last_rms = energy;
    state.level.store(energy.to_bits(), Ordering::Relaxed);
    if state.take.is_none() && energy < 0.02 {
        state.noise_rms = state.noise_rms * 0.9 + energy * 0.1;
    }
}

fn rms(chunk: &[f32]) -> f32 {
    if chunk.is_empty() {
        return 0.0;
    }
    (chunk.iter().map(|sample| sample * sample).sum::<f32>() / chunk.len() as f32).sqrt()
}

fn arm_take(
    state: &Mutex<Capture>,
    next_id: &mut u64,
    active_name: &str,
    used_fallback: bool,
    note: &str,
) -> Result<Armed, String> {
    if active_name.is_empty() {
        return Err("No microphone is available. Plug one in and try again.".into());
    }
    let mut guard = state.lock().map_err(|_| "The audio buffer was unavailable.".to_string())?;
    if let Some(err) = guard.error.clone() {
        return Err(err);
    }
    let id = *next_id;
    *next_id = next_id.wrapping_add(1);
    guard.take_id = id;
    guard.take = Some(guard.ring.iter().copied().collect());
    Ok(Armed {
        id,
        active_name: active_name.to_string(),
        used_fallback,
        note: note.to_string(),
        level: Arc::clone(&guard.level),
    })
}

fn disarm_take(state: &Mutex<Capture>, id: u64) -> Result<FinishedAudio, String> {
    let mut guard = state.lock().map_err(|_| "The audio buffer was unavailable.".to_string())?;
    if guard.take_id != id {
        return Err("This take already ended.".into());
    }
    let samples = guard.take.take().unwrap_or_default();
    let err = guard.error.clone();
    guard.error = None;
    if samples.is_empty() {
        if let Some(err) = err {
            return Err(err);
        }
    }
    Ok(FinishedAudio {
        interleaved: samples,
        channels: guard.channels,
        sample_rate: guard.sample_rate,
        elapsed_ms: 0,
        noise_rms: guard.noise_rms,
    })
}

