//! A microphone stream that stays open between takes.
//!
//! Opening WASAPI on key-down cost seconds and showed "Starting" before
//! "Listening". This thread opens the preferred device (or the current
//! default) at startup, keeps about a second of pre-roll, and starts a take
//! by copying that buffer. Key-down never opens a device. If the stream has
//! died, it is reopened on this thread after the take is already marked.
//! Device changes are picked up while idle, using the same fallback order as
//! a one-shot recording.

use crate::audio::{self, FinishedAudio};
use crate::chunk;
use cpal::traits::{HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PRE_ROLL_MS: u32 = 1000;
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
    chunks: Arc<Mutex<VecDeque<FinishedAudio>>>,
    pre_roll_samples: usize,
    first_sample: Arc<Mutex<Option<Instant>>>,
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
    stop: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    chunks: Arc<Mutex<VecDeque<FinishedAudio>>>,
    /// Samples already in the ring when the key went down.
    pub pre_roll_samples: usize,
    first_sample: Arc<Mutex<Option<Instant>>>,
}

impl LiveTake {
    pub fn drain_chunks(&self) -> Vec<FinishedAudio> {
        match self.chunks.lock() {
            Ok(mut queue) => queue.drain(..).collect(),
            Err(err) => err.into_inner().drain(..).collect(),
        }
    }
}

impl LiveTake {
    pub fn chunk_bus(&self) -> Arc<Mutex<VecDeque<FinishedAudio>>> {
        Arc::clone(&self.chunks)
    }

    pub fn meter(&self) -> (Arc<AtomicU32>, Arc<AtomicBool>) {
        (Arc::clone(&self.level), Arc::clone(&self.stop))
    }

    /// Milliseconds from `keydown` to the first sample that arrived after the
    /// pre-roll was copied. `None` until that callback lands.
    pub fn first_sample_ms_since(&self, keydown: Instant) -> Option<u64> {
        let at = self.first_sample.lock().ok().and_then(|guard| *guard)?;
        Some(at.saturating_duration_since(keydown).as_millis() as u64)
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
    level: Arc<AtomicU32>,
    live: Arc<AtomicBool>,
}

impl WarmMic {
    pub fn spawn(preference: Option<String>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let level = Arc::new(AtomicU32::new(0));
        let live = Arc::new(AtomicBool::new(false));
        let initial = preference.clone();
        let shared = Arc::clone(&level);
        let shared_live = Arc::clone(&live);
        let _ = std::thread::Builder::new().name("bolo-warm".into()).spawn(move || {
            run(cmd_rx, initial, shared, shared_live);
        });
        // Ask the thread to open this preference even if spawn raced.
        let _ = cmd_tx.send(Cmd::Retarget(preference));
        Self { cmd: cmd_tx, level, live }
    }

    /// True once a capture stream is playing. Key-down does not wait on this.
    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::Relaxed)
    }

    /// Latest input level, 0.0–1.0, from the open microphone.
    pub fn level(&self) -> f32 {
        let rms = f32::from_bits(self.level.load(Ordering::Relaxed));
        if !rms.is_finite() || rms < 0.0 {
            0.0
        } else {
            (rms * 4.5).clamp(0.0, 1.0)
        }
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
        // The first open can sit behind a device scan. Eight seconds covers a
        // slow headset without failing the press that started it. If this
        // wait expires, the warm thread disarms when the reply is gone.
        let armed = match reply_rx.recv_timeout(Duration::from_secs(8)) {
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
            stop: Arc::new(AtomicBool::new(false)),
            level: armed.level,
            chunks: armed.chunks,
            pre_roll_samples: armed.pre_roll_samples,
            first_sample: armed.first_sample,
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
    chunks: Arc<Mutex<VecDeque<FinishedAudio>>>,
    take_silence_ms: u32,
    /// How many samples were copied out of the ring when this take was armed.
    take_mark_len: usize,
    /// Set on the first sample that arrives after that copy.
    first_sample: Arc<Mutex<Option<Instant>>>,
}

impl Capture {
    fn new(level: Arc<AtomicU32>) -> Self {
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
            level,
            chunks: Arc::new(Mutex::new(VecDeque::new())),
            take_silence_ms: 0,
            take_mark_len: 0,
            first_sample: Arc::new(Mutex::new(None)),
        }
    }

    fn apply_format(&mut self, channels: u16, sample_rate: u32, preserve_take: bool) {
        let channels = channels.max(1);
        let sample_rate = sample_rate.max(1);
        let samples = (u64::from(sample_rate) * u64::from(channels) * u64::from(PRE_ROLL_MS) / 1000) as usize;
        self.cap = samples.max(1);
        let format_changed = self.channels != channels || self.sample_rate != sample_rate;
        self.channels = channels;
        self.sample_rate = sample_rate;
        if preserve_take && self.take.is_some() {
            if format_changed {
                self.ring.clear();
            }
            return;
        }
        self.ring.clear();
        self.take = None;
    }
}

fn run(cmd_rx: Receiver<Cmd>, mut preference: Option<String>, level: Arc<AtomicU32>, live: Arc<AtomicBool>) {
    let host = cpal::default_host();
    let state = Arc::new(Mutex::new(Capture::new(level)));
    let mut stream: Option<cpal::Stream> = None;
    let mut active_name = String::new();
    let mut used_fallback = false;
    let mut note = String::new();
    let mut next_id = 1u64;
    let mut last_scan = Instant::now() - Duration::from_secs(2);
    let mut reopen_after = Instant::now();
    let mut deferred = VecDeque::new();

    loop {
        let cmd = if let Some(cmd) = deferred.pop_front() {
            Some(cmd)
        } else {
            match cmd_rx.try_recv() {
                Ok(cmd) => Some(cmd),
                Err(mpsc::TryRecvError::Empty) => match cmd_rx.recv_timeout(LOOP_SLICE) {
                    Ok(cmd) => Some(cmd),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                },
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        };
        if let Some(cmd) = cmd {
            handle_cmd(
                cmd,
                &state,
                &mut preference,
                &mut active_name,
                &mut used_fallback,
                &mut note,
                &mut next_id,
            );
            continue;
        }

        let recording = state.lock().ok().is_some_and(|guard| guard.take.is_some());
        let broken = state.lock().ok().and_then(|guard| guard.error.clone()).is_some();
        let missing = stream.is_none() || broken || !live.load(Ordering::Relaxed);
        let idle_scan = !recording && last_scan.elapsed() >= Duration::from_secs(1);
        if (missing && Instant::now() >= reopen_after) || idle_scan {
            if missing || device_should_change(&host, &preference, &active_name) {
                last_scan = Instant::now();
                // A dead stream is reopened even while a take is armed, and the
                // samples already copied stay in that take. Idle scans yield as
                // soon as a Begin arrives so a press is not stuck behind a sweep.
                match reopen(
                    &host,
                    &preference,
                    &state,
                    &mut stream,
                    &mut active_name,
                    &mut used_fallback,
                    &mut note,
                    &live,
                    recording,
                    Some(&cmd_rx),
                    &mut deferred,
                ) {
                    Reopen::Yielded => continue,
                    Reopen::Failed(_) => {
                        reopen_after = Instant::now() + Duration::from_millis(400);
                    }
                    Reopen::Ready => reopen_after = Instant::now(),
                }
            }
        }
    }
}

fn handle_cmd(
    cmd: Cmd,
    state: &Arc<Mutex<Capture>>,
    preference: &mut Option<String>,
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
            // The ring is marked here. Opening a device is the loop's job,
            // after this reply, so a dead mic cannot stall the hotkey.
            *preference = asked;
            match arm_take(state, next_id, active_name, *used_fallback, note) {
                Ok(armed) => {
                    let id = armed.id;
                    if reply.send(Ok(armed)).is_err() {
                        // The press already gave up. Don't leave this take armed
                        // or the next press ends the wrong buffer.
                        let _ = disarm_take(state, id);
                    }
                }
                Err(err) => {
                    let _ = reply.send(Err(err));
                }
            }
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

enum Reopen {
    Ready,
    Yielded,
    #[allow(dead_code)]
    Failed(String),
}

fn reopen(
    host: &cpal::Host,
    preference: &Option<String>,
    state: &Arc<Mutex<Capture>>,
    stream: &mut Option<cpal::Stream>,
    active_name: &mut String,
    used_fallback: &mut bool,
    note: &mut String,
    live: &Arc<AtomicBool>,
    preserve_take: bool,
    interrupt: Option<&Receiver<Cmd>>,
    deferred: &mut VecDeque<Cmd>,
) -> Reopen {
    if pull_interrupt(interrupt, deferred) {
        return Reopen::Yielded;
    }
    *stream = None;
    live.store(false, Ordering::Relaxed);
    if let Ok(mut guard) = state.lock() {
        guard.error = None;
    }
    let devices = match audio::host_mics(host) {
        Ok(devices) => devices,
        Err(err) => return Reopen::Failed(err.to_string()),
    };
    let order = audio::open_order(&devices, preference.as_deref());
    if order.is_empty() {
        active_name.clear();
        return Reopen::Failed("No microphone is available.".into());
    }
    let mut failures = Vec::new();
    for name in &order {
        if pull_interrupt(interrupt, deferred) {
            return Reopen::Yielded;
        }
        match open_one(host, name, state, live, preserve_take, interrupt, deferred) {
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
                live.store(true, Ordering::Relaxed);
                return Reopen::Ready;
            }
            Err(OpenFail::Yielded) => return Reopen::Yielded,
            Err(OpenFail::Failed(err)) => failures.push(format!("{name}: {err:#}")),
        }
    }
    active_name.clear();
    Reopen::Failed(if failures.is_empty() {
        "Couldn't open a microphone.".into()
    } else {
        format!("Couldn't open a microphone. {}", failures.join(" "))
    })
}

fn pull_interrupt(interrupt: Option<&Receiver<Cmd>>, deferred: &mut VecDeque<Cmd>) -> bool {
    let Some(rx) = interrupt else {
        return false;
    };
    let mut pulled = false;
    while let Ok(cmd) = rx.try_recv() {
        deferred.push_back(cmd);
        pulled = true;
    }
    pulled
}

enum OpenFail {
    Yielded,
    Failed(anyhow::Error),
}

fn open_one(
    host: &cpal::Host,
    name: &str,
    state: &Arc<Mutex<Capture>>,
    live: &Arc<AtomicBool>,
    preserve_take: bool,
    interrupt: Option<&Receiver<Cmd>>,
    deferred: &mut VecDeque<Cmd>,
) -> Result<cpal::Stream, OpenFail> {
    use cpal::traits::DeviceTrait;
    let wanted = audio::normalize_mic_name(name);
    let device = host
        .input_devices()
        .map_err(|err| OpenFail::Failed(anyhow::anyhow!("couldn't list microphones: {err}")))?
        .find(|device| {
            device
                .name()
                .ok()
                .map(|found| audio::normalize_mic_name(&found) == wanted)
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow::anyhow!("microphone \"{name}\" disappeared"))
        .map_err(OpenFail::Failed)?;
    let supported = device
        .default_input_config()
        .map_err(|err| OpenFail::Failed(anyhow::anyhow!("{name} has no default input format: {err}")))?;
    let channels = supported.channels();
    let sample_rate = supported.sample_rate().0;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    if let Ok(mut guard) = state.lock() {
        guard.apply_format(channels, sample_rate, preserve_take);
        guard.error = None;
    }
    let audio_state = Arc::clone(state);
    let err_state = Arc::clone(state);
    let err_live = Arc::clone(live);
    let stream = audio::build_input_callback(
        &device,
        &config,
        sample_format,
        move |samples| push_samples(&audio_state, samples),
        move |err| {
            err_live.store(false, Ordering::Relaxed);
            if let Ok(mut guard) = err_state.lock() {
                guard.error = Some(format!("Microphone error: {err}"));
            }
        },
    )
    .map_err(OpenFail::Failed)?;
    stream
        .play()
        .map_err(|err| OpenFail::Failed(anyhow::anyhow!("couldn't start {name}: {err}")))?;
    // Give the callback a moment to report an immediate invalidation.
    // An idle scan bails here if a press arrived, so that press opens the mic itself.
    let bridge = Arc::clone(state);
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(20));
        if pull_interrupt(interrupt, deferred) {
            return Err(OpenFail::Yielded);
        }
        if let Some(err) = bridge.lock().ok().and_then(|guard| guard.error.clone()) {
            return Err(OpenFail::Failed(anyhow::anyhow!(err)));
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
            take.push(sample);
            if take.len() > state.take_mark_len {
                if let Ok(mut slot) = state.first_sample.lock() {
                    if slot.is_none() {
                        *slot = Some(Instant::now());
                    }
                }
            }
        }
        chunk.push(sample);
        if chunk.len() >= 480 {
            let energy = rms(&chunk);
            let frame_ms = frame_duration_ms(chunk.len(), state.sample_rate, state.channels);
            note_energy(&mut state, energy, frame_ms);
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        let energy = rms(&chunk);
        let frame_ms = frame_duration_ms(chunk.len(), state.sample_rate, state.channels);
        note_energy(&mut state, energy, frame_ms);
    }
}

fn frame_duration_ms(samples: usize, sample_rate: u32, channels: u16) -> u32 {
    let channels = usize::from(channels.max(1));
    let rate = sample_rate.max(1) as usize;
    ((samples / channels) as u64 * 1000 / rate as u64) as u32
}

fn note_energy(state: &mut Capture, energy: f32, frame_ms: u32) {
    state.last_rms = energy;
    state.level.store(energy.to_bits(), Ordering::Relaxed);
    if state.take.is_none() && energy < 0.02 {
        state.noise_rms = state.noise_rms * 0.9 + energy * 0.1;
        return;
    }
    if state.take.is_some() {
        let threshold = audio::speech_threshold(state.noise_rms);
        if energy >= threshold {
            state.take_silence_ms = 0;
        } else {
            state.take_silence_ms = state.take_silence_ms.saturating_add(frame_ms.max(1));
        }
        maybe_cut(state);
    }
}

fn maybe_cut(state: &mut Capture) {
    let Some(samples) = state.take.as_ref() else {
        return;
    };
    let channels = usize::from(state.channels.max(1));
    if channels == 0 || samples.is_empty() {
        return;
    }
    let frames = samples.len() / channels;
    let buffered_ms = (frames as u64 * 1000 / u64::from(state.sample_rate.max(1))) as u32;
    if !chunk::should_cut(buffered_ms, state.take_silence_ms) {
        return;
    }
    let overlap = chunk::overlap_samples(state.sample_rate, state.channels);
    let Some(mut all) = state.take.take() else {
        return;
    };
    if all.len() <= overlap {
        state.take = Some(all);
        return;
    }
    let next = all[all.len() - overlap..].to_vec();
    let noise = state.noise_rms;
    let channels_n = state.channels;
    let sample_rate = state.sample_rate;
    let bus = Arc::clone(&state.chunks);
    state.take = Some(next);
    state.take_silence_ms = 0;
    let mut queue = bus.lock().unwrap_or_else(|err| err.into_inner());
    queue.push_back(FinishedAudio {
        interleaved: all.split_off(0),
        channels: channels_n,
        sample_rate,
        elapsed_ms: u64::from(buffered_ms),
        noise_rms: noise,
    });
    drop(queue);
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
    let mut guard = state.lock().map_err(|_| "The audio buffer was unavailable.".to_string())?;
    // A stale callback error must not block the press. The loop reopens the
    // device after this reply and appends into the take we just marked.
    guard.error = None;
    let id = *next_id;
    *next_id = next_id.wrapping_add(1);
    guard.take_id = id;
    guard.take_silence_ms = 0;
    let chunks = Arc::new(Mutex::new(VecDeque::new()));
    guard.chunks = Arc::clone(&chunks);
    let pre_roll_samples = guard.ring.len();
    guard.take_mark_len = pre_roll_samples;
    guard.take = Some(guard.ring.iter().copied().collect());
    let first_sample = Arc::new(Mutex::new(None));
    guard.first_sample = Arc::clone(&first_sample);
    let note = if note.is_empty() && active_name.is_empty() {
        "Microphone reconnecting".to_string()
    } else {
        note.to_string()
    };
    Ok(Armed {
        id,
        active_name: active_name.to_string(),
        used_fallback,
        note,
        level: Arc::clone(&guard.level),
        chunks,
        pre_roll_samples,
        first_sample,
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

#[test]
fn arm_marks_preroll_without_opening_a_device() {
        let level = Arc::new(AtomicU32::new(0));
        let state = Arc::new(Mutex::new(Capture::new(level)));
        {
            let mut guard = state.lock().expect("buffer");
            guard.apply_format(1, 16_000, false);
            guard.ring.extend(std::iter::repeat(0.05).take(16_000));
            guard.error = Some("Microphone error: dead".into());
        }
        let mut preference = None;
        let mut active_name = String::new();
        let mut used_fallback = false;
        let mut note = String::new();
        let mut next_id = 1u64;
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        handle_cmd(
            Cmd::Begin {
                preference: None,
                reply: tx,
            },
            &state,
            &mut preference,
            &mut active_name,
            &mut used_fallback,
            &mut note,
            &mut next_id,
        );
        let armed = rx
            .recv_timeout(Duration::from_millis(50))
            .expect("the mark should reply at once")
            .expect("a dead stream still arms the pre-roll");
        let overlay_ms = started.elapsed().as_millis() as u64;
        assert!(overlay_ms < 50, "marking the buffer took {overlay_ms} ms");
        assert_eq!(armed.pre_roll_samples, 16_000);
        let sample_started = Instant::now();
        push_samples(&state, std::iter::repeat(0.2).take(3_200));
        let first_ms = armed.first_sample.lock().ok().and_then(|slot| *slot).map(|at| {
            at.saturating_duration_since(sample_started).as_millis() as u64
        });
        eprintln!(
            "MEASURE keydown_overlay_ms={overlay_ms} keydown_first_sample_ms={}",
            first_ms.unwrap_or(0)
        );
        assert!(first_ms.unwrap_or(999) < 50, "first sample was not marked");
        let finished = disarm_take(&state, armed.id).expect("disarm");
        assert!(finished.interleaved.len() >= 16_000 + 3_200);
        let new_ms = (finished.interleaved.len() - 16_000) as u64 * 1000 / 16_000;
        assert!(new_ms >= 200, "only {new_ms} ms arrived after the mark");
    }


#[cfg(all(test, windows))]
#[test]
fn first_press_starts_capture() {
    // Decision coverage also lives in dictation. Here the first Begin has to
    // arm a real device when Windows has one, and a second Begin has to arm too.
    if std::env::var("CI")
        .ok()
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("true") || value == "1")
    {
        eprintln!("SKIP first_press_starts_capture: CI=true");
        return;
    }
    let warm = WarmMic::spawn(None);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !warm.is_live() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(30));
    }
    if !warm.is_live() {
        eprintln!("SKIP first_press_starts_capture: no usable capture device");
        return;
    }
    match warm.begin(None) {
        Ok(take) => {
            std::thread::sleep(Duration::from_millis(180));
            let finished = take.finish().expect("the first press should stop cleanly");
            assert!(finished.sample_rate > 0, "the first press did not open a capture stream");
            assert!(!finished.interleaved.is_empty(), "the first press captured no audio");
            let second = warm.begin(None).expect("the second press should arm");
            let _ = second.finish();
        }
        Err(err) => {
            eprintln!("SKIP first_press_starts_capture: {err}");
        }
    }
}

