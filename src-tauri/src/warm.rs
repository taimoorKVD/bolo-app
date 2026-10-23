//! A microphone stream that stays open between takes.
//!
//! The capture thread opens the preferred device (or the Windows default) once
//! at launch and leaves it playing. Key-down only copies the one-second ring,
//! so the press does not wait on WASAPI. The stream is closed only when a new
//! one is already playing: a dead stream, a saved microphone change, or a
//! Windows default-device notification whose device actually differs. A name
//! poll never reopens it.

use crate::audio::{self, FinishedAudio};
use crate::chunk;
use cpal::traits::{HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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

    /// Milliseconds from `keydown` to audio that covers that instant.
    ///
    /// Pre-roll already holds samples from before the key, so the wait is 0.
    /// With an empty ring this is the wait for the next callback.
    pub fn first_sample_ms_since(&self, keydown: Instant) -> Option<u64> {
        reported_first_sample_ms(
            self.pre_roll_samples,
            self.first_sample.lock().ok().and_then(|guard| *guard).map(|at| {
                at.saturating_duration_since(keydown).as_millis() as u64
            }),
        )
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
    pub fn spawn(preference: Option<String>, config_dir: Option<PathBuf>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let level = Arc::new(AtomicU32::new(0));
        let live = Arc::new(AtomicBool::new(false));
        let watch = Arc::new(Watch::new(preference.clone(), config_dir));
        let shared = Arc::clone(&level);
        let shared_live = Arc::clone(&live);
        let shared_watch = Arc::clone(&watch);
        let _ = std::thread::Builder::new().name("bolo-warm".into()).spawn(move || {
            run(cmd_rx, shared_watch, shared, shared_live);
        });
        // Ask the thread to record this preference even if spawn raced.
        // The same name does not reopen the stream.
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

    /// Keep the ring across an open. Samples are dropped only when the device
    /// format changes, because the old rate would no longer match the take.
    fn apply_format(&mut self, channels: u16, sample_rate: u32) {
        let channels = channels.max(1);
        let sample_rate = sample_rate.max(1);
        let samples = (u64::from(sample_rate) * u64::from(channels) * u64::from(PRE_ROLL_MS) / 1000) as usize;
        self.cap = samples.max(1);
        let format_changed = self.channels != channels || self.sample_rate != sample_rate;
        self.channels = channels;
        self.sample_rate = sample_rate;
        if format_changed {
            self.ring.clear();
        }
        while self.ring.len() > self.cap {
            self.ring.pop_front();
        }
    }
}

struct Watch {
    preference: Mutex<Option<String>>,
    active_name: Mutex<String>,
    used_fallback: Mutex<bool>,
    note: Mutex<String>,
    preference_epoch: AtomicU64,
    default_event: AtomicBool,
    config_dir: Option<PathBuf>,
}

impl Watch {
    fn new(preference: Option<String>, config_dir: Option<PathBuf>) -> Self {
        Self {
            preference: Mutex::new(preference),
            active_name: Mutex::new(String::new()),
            used_fallback: Mutex::new(false),
            note: Mutex::new(String::new()),
            preference_epoch: AtomicU64::new(0),
            default_event: AtomicBool::new(false),
            config_dir,
        }
    }

    fn set_preference(&self, next: Option<String>) {
        let mut slot = self.preference.lock().unwrap_or_else(|err| err.into_inner());
        if same_preference(&slot, &next) {
            return;
        }
        *slot = next;
        self.preference_epoch.fetch_add(1, Ordering::Relaxed);
    }
}

fn same_preference(current: &Option<String>, next: &Option<String>) -> bool {
    norm_pref(current) == norm_pref(next)
}

fn norm_pref(value: &Option<String>) -> Option<String> {
    value.as_deref().map(str::trim).filter(|value| !value.is_empty()).map(audio::normalize_mic_name)
}

/// Why a capture stream should be opened. `None` leaves the playing stream alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReopenReason {
    Launch,
    StreamError,
    Preference,
    DeviceChanged,
}

impl ReopenReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::StreamError => "stream-error",
            Self::Preference => "preference",
            Self::DeviceChanged => "device-changed",
        }
    }
}

/// A healthy stream stays up across takes. A default-device event reopens only
/// when the device we would open is a different one. `names_differ` is false
/// when the resolved name still matches the open device.
fn plan_reopen(
    have_stream: bool,
    ever_opened: bool,
    preference_dirty: bool,
    default_event: bool,
    names_differ: bool,
) -> Option<ReopenReason> {
    if !have_stream {
        return Some(if ever_opened {
            ReopenReason::StreamError
        } else {
            ReopenReason::Launch
        });
    }
    if preference_dirty && names_differ {
        return Some(ReopenReason::Preference);
    }
    if default_event && names_differ {
        return Some(ReopenReason::DeviceChanged);
    }
    None
}

fn warm_log_line(reason: &str) -> String {
    format!("warm stream opened/reopened reason={reason}")
}

/// `0` when the ring already covered the key. Otherwise the wait for a new sample.
fn reported_first_sample_ms(pre_roll_samples: usize, captured_after_ms: Option<u64>) -> Option<u64> {
    if pre_roll_samples > 0 {
        Some(0)
    } else {
        captured_after_ms
    }
}

fn run(cmd_rx: Receiver<Cmd>, watch: Arc<Watch>, level: Arc<AtomicU32>, live: Arc<AtomicBool>) {
    let state = Arc::new(Mutex::new(Capture::new(Arc::clone(&level))));
    let io_state = Arc::clone(&state);
    let io_live = Arc::clone(&live);
    let io_watch = Arc::clone(&watch);
    let _ = std::thread::Builder::new().name("bolo-warm-io".into()).spawn(move || {
        io_loop(io_state, io_live, io_watch);
    });
    let mut next_id = 1u64;
    loop {
        let cmd = match cmd_rx.recv_timeout(LOOP_SLICE) {
            Ok(cmd) => cmd,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        handle_cmd(cmd, &state, &watch, &mut next_id);
    }
}

fn handle_cmd(cmd: Cmd, state: &Arc<Mutex<Capture>>, watch: &Watch, next_id: &mut u64) {
    match cmd {
        Cmd::Retarget(next) => watch.set_preference(next),
        Cmd::Begin { preference, reply } => {
            // The ring is marked here. Opening a device is the capture thread's
            // job and does not block this reply.
            watch.set_preference(preference);
            let active_name = watch.active_name.lock().unwrap_or_else(|err| err.into_inner()).clone();
            let used_fallback = *watch.used_fallback.lock().unwrap_or_else(|err| err.into_inner());
            let note = watch.note.lock().unwrap_or_else(|err| err.into_inner()).clone();
            match arm_take(state, next_id, &active_name, used_fallback, &note) {
                Ok(armed) => {
                    let id = armed.id;
                    if reply.send(Ok(armed)).is_err() {
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

fn io_loop(state: Arc<Mutex<Capture>>, live: Arc<AtomicBool>, watch: Arc<Watch>) {
    let host = cpal::default_host();
    #[cfg(windows)]
    let _notifications = register_default_notifications(Arc::clone(&watch));
    let mut stream: Option<cpal::Stream> = None;
    let stream_gen = Arc::new(AtomicU64::new(0));
    let mut next_gen = 1u64;
    let mut ever_opened = false;
    let mut seen_preference = 0u64;
    let mut reopen_after = Instant::now();
    let mut last_fail = String::new();
    let mut last_fail_at = Instant::now() - Duration::from_secs(30);

    loop {
        pump_audio_messages();
        let preference = watch.preference.lock().unwrap_or_else(|err| err.into_inner()).clone();
        let active_name = watch.active_name.lock().unwrap_or_else(|err| err.into_inner()).clone();
        let broken = stream.is_some()
            && (!live.load(Ordering::Relaxed)
                || state.lock().ok().and_then(|guard| guard.error.clone()).is_some());
        let have_stream = stream.is_some() && !broken;
        let preference_dirty = watch.preference_epoch.load(Ordering::Relaxed) != seen_preference;
        let default_event = watch.default_event.load(Ordering::Relaxed);
        let check_names = have_stream && (preference_dirty || default_event);
        let names = if check_names {
            device_names_differ(&host, &preference, &active_name)
        } else {
            NameCheck::Same
        };
        let names_differ = matches!(names, NameCheck::Differ);
        let reason = plan_reopen(have_stream, ever_opened, preference_dirty, default_event, names_differ);

        if reason.is_none() {
            if preference_dirty && !matches!(names, NameCheck::Unknown) {
                seen_preference = watch.preference_epoch.load(Ordering::Relaxed);
            }
            if default_event && !matches!(names, NameCheck::Unknown) {
                watch.default_event.store(false, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(40));
            continue;
        }
        let reason = reason.expect("reason");
        if matches!(reason, ReopenReason::StreamError) && Instant::now() < reopen_after {
            std::thread::sleep(Duration::from_millis(40));
            continue;
        }
        // Keep the playing stream until the replacement has started. A press
        // during this open still copies the ring on the other thread.
        let epoch_at_start = watch.preference_epoch.load(Ordering::Relaxed);
        match open_replacing(
            &host,
            &preference,
            &state,
            &mut stream,
            &live,
            &stream_gen,
            &watch,
            &mut next_gen,
        ) {
            Ok(()) => {
                ever_opened = true;
                // A change that arrived while we were opening still needs a pass.
                if watch.preference_epoch.load(Ordering::Relaxed) == epoch_at_start {
                    seen_preference = epoch_at_start;
                }
                if matches!(reason, ReopenReason::DeviceChanged) {
                    watch.default_event.store(false, Ordering::Relaxed);
                }
                reopen_after = Instant::now();
                last_fail.clear();
                log_warm(&watch, &warm_log_line(reason.as_str()));
            }
            Err(err) => {
                reopen_after = Instant::now() + Duration::from_millis(400);
                if err != last_fail || last_fail_at.elapsed() >= Duration::from_secs(5) {
                    log_warm(
                        &watch,
                        &format!("warm stream open failed reason={} err={err}", reason.as_str()),
                    );
                    last_fail = err;
                    last_fail_at = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        }
    }
}

enum NameCheck {
    Same,
    Differ,
    Unknown,
}

fn device_names_differ(host: &cpal::Host, preference: &Option<String>, active_name: &str) -> NameCheck {
    if active_name.is_empty() {
        return NameCheck::Differ;
    }
    let devices = match audio::host_mics(host) {
        Ok(devices) => devices,
        Err(_) => return NameCheck::Unknown,
    };
    if devices.is_empty() {
        return NameCheck::Differ;
    }
    let choice = match audio::resolve_microphone(&devices, preference.as_deref()) {
        Ok(choice) => choice,
        Err(_) => return NameCheck::Unknown,
    };
    if audio::normalize_mic_name(&choice.active_name) == audio::normalize_mic_name(active_name) {
        NameCheck::Same
    } else {
        NameCheck::Differ
    }
}

fn log_warm(watch: &Watch, message: &str) {
    if let Some(dir) = &watch.config_dir {
        crate::logbuf::log_line(dir, "info", message);
    } else {
        eprintln!("info {message}");
    }
}

struct Opened {
    stream: cpal::Stream,
    channels: u16,
    sample_rate: u32,
    generation: u64,
    name: String,
    used_fallback: bool,
    note: String,
}

fn open_replacing(
    host: &cpal::Host,
    preference: &Option<String>,
    state: &Arc<Mutex<Capture>>,
    stream: &mut Option<cpal::Stream>,
    live: &Arc<AtomicBool>,
    stream_gen: &Arc<AtomicU64>,
    watch: &Watch,
    next_gen: &mut u64,
) -> Result<(), String> {
    let devices = audio::host_mics(host).map_err(|err| err.to_string())?;
    let order = audio::open_order(&devices, preference.as_deref());
    if order.is_empty() {
        return Err("No microphone is available.".into());
    }
    let mut failures = Vec::new();
    for name in &order {
        let generation = *next_gen;
        *next_gen = next_gen.wrapping_add(1);
        match open_candidate(host, name, state, live, stream_gen, generation) {
            Ok(mut opened) => {
                finish_opened(&mut opened, preference, name, &order);
                if let Ok(mut guard) = state.lock() {
                    guard.apply_format(opened.channels, opened.sample_rate);
                    guard.error = None;
                }
                if let Ok(mut slot) = watch.used_fallback.lock() {
                    *slot = opened.used_fallback;
                }
                if let Ok(mut slot) = watch.note.lock() {
                    *slot = opened.note;
                }
                if let Ok(mut slot) = watch.active_name.lock() {
                    *slot = opened.name;
                }
                stream_gen.store(opened.generation, Ordering::Relaxed);
                *stream = Some(opened.stream);
                live.store(true, Ordering::Relaxed);
                return Ok(());
            }
            Err(err) => failures.push(format!("{name}: {err}")),
        }
    }
    Err(if failures.is_empty() {
        "Couldn't open a microphone.".into()
    } else {
        format!("Couldn't open a microphone. {}", failures.join(" "))
    })
}

fn open_candidate(
    host: &cpal::Host,
    name: &str,
    state: &Arc<Mutex<Capture>>,
    live: &Arc<AtomicBool>,
    stream_gen: &Arc<AtomicU64>,
    generation: u64,
) -> Result<Opened, String> {
    use cpal::traits::DeviceTrait;
    let wanted = audio::normalize_mic_name(name);
    let device = host
        .input_devices()
        .map_err(|err| format!("couldn't list microphones: {err}"))?
        .find(|device| {
            device
                .name()
                .ok()
                .map(|found| audio::normalize_mic_name(&found) == wanted)
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("microphone \"{name}\" disappeared"))?;
    let supported = device
        .default_input_config()
        .map_err(|err| format!("{name} has no default input format: {err}"))?;
    let channels = supported.channels();
    let sample_rate = supported.sample_rate().0;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let audio_state = Arc::clone(state);
    let err_live = Arc::clone(live);
    let err_state = Arc::clone(state);
    let err_gen = Arc::clone(stream_gen);
    let probe: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let probe_err = Arc::clone(&probe);
    let stream = audio::build_input_callback(
        &device,
        &config,
        sample_format,
        move |samples| push_samples(&audio_state, samples),
        move |err| {
            let message = format!("Microphone error: {err}");
            if let Ok(mut slot) = probe_err.lock() {
                *slot = Some(message.clone());
            }
            if err_gen.load(Ordering::Relaxed) == generation {
                err_live.store(false, Ordering::Relaxed);
                if let Ok(mut guard) = err_state.lock() {
                    guard.error = Some(message);
                }
            }
        },
    )
    .map_err(|err| err.to_string())?;
    stream.play().map_err(|err| format!("couldn't start {name}: {err}"))?;
    // Immediate invalidation shows up within a couple of callbacks. This wait
    // stays on the capture thread, so a key-down still arms the old ring.
    for _ in 0..2 {
        std::thread::sleep(Duration::from_millis(15));
        if let Some(err) = probe.lock().ok().and_then(|slot| slot.clone()) {
            return Err(err);
        }
    }
    let pref_name = name.to_string();
    Ok(Opened {
        stream,
        channels,
        sample_rate,
        generation,
        name: pref_name,
        used_fallback: false,
        note: String::new(),
    })
}

fn finish_opened(
    opened: &mut Opened,
    preference: &Option<String>,
    name: &str,
    order: &[String],
) {
    let pref = preference.as_deref().map(str::trim).filter(|value| !value.is_empty());
    let opened_preference =
        pref.is_some_and(|pref| audio::normalize_mic_name(pref) == audio::normalize_mic_name(name));
    let skipped_first = order.first().map(String::as_str) != Some(name);
    opened.used_fallback = match pref {
        Some(_) => !opened_preference,
        None => skipped_first,
    };
    opened.note = audio::usage_line(pref, name, opened.used_fallback);
}

#[cfg(windows)]
fn pump_audio_messages() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(not(windows))]
fn pump_audio_messages() {}

#[cfg(windows)]
struct DefaultNotifications {
    enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator,
    client: windows::Win32::Media::Audio::IMMNotificationClient,
}

#[cfg(windows)]
impl Drop for DefaultNotifications {
    fn drop(&mut self) {
        unsafe {
            let _ = self.enumerator.UnregisterEndpointNotificationCallback(&self.client);
        }
    }
}

#[cfg(windows)]
fn register_default_notifications(watch: Arc<Watch>) -> Option<DefaultNotifications> {
    default_device::install(watch).ok()
}

#[cfg(windows)]
mod default_device {
    use super::{DefaultNotifications, Watch};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use windows::core::implement;
    use windows::Win32::Media::Audio::{
        IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, DEVICE_STATE,
        EDataFlow, ERole,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    #[implement(IMMNotificationClient)]
    struct DefaultDeviceNotifications {
        watch: Arc<Watch>,
    }

    #[allow(non_snake_case)]
    impl IMMNotificationClient_Impl for DefaultDeviceNotifications_Impl {
        fn OnDeviceStateChanged(
            &self,
            _id: &windows::core::PCWSTR,
            _state: DEVICE_STATE,
        ) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnDeviceAdded(&self, _id: &windows::core::PCWSTR) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnDeviceRemoved(&self, _id: &windows::core::PCWSTR) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnDefaultDeviceChanged(
            &self,
            flow: EDataFlow,
            _role: ERole,
            _id: &windows::core::PCWSTR,
        ) -> windows::core::Result<()> {
            // eRender is 0. Capture and "all" are the default microphone changing.
            if flow.0 != 0 {
                self.watch.default_event.store(true, Ordering::Relaxed);
            }
            Ok(())
        }

        fn OnPropertyValueChanged(
            &self,
            _id: &windows::core::PCWSTR,
            _key: &windows::Win32::Foundation::PROPERTYKEY,
        ) -> windows::core::Result<()> {
            Ok(())
        }
    }

    pub fn install(watch: Arc<Watch>) -> windows::core::Result<DefaultNotifications> {
        let enumerator = unsafe {
            CoCreateInstance::<_, IMMDeviceEnumerator>(
                &MMDeviceEnumerator,
                None::<&windows::core::IUnknown>,
                CLSCTX_ALL,
            )?
        };
        let client: IMMNotificationClient = DefaultDeviceNotifications { watch }.into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client)? };
        Ok(DefaultNotifications { enumerator, client })
    }
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
            guard.apply_format(1, 16_000);
            guard.ring.extend(std::iter::repeat(0.05).take(16_000));
            guard.error = Some("Microphone error: dead".into());
        }
        let watch = Watch::new(None, None);
        let mut next_id = 1u64;
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        handle_cmd(
            Cmd::Begin {
                preference: None,
                reply: tx,
            },
            &state,
            &watch,
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
        assert_eq!(
            reported_first_sample_ms(armed.pre_roll_samples, first_ms),
            Some(0),
            "pre-roll must log keydown_first_sample_ms=0"
        );
        let finished = disarm_take(&state, armed.id).expect("disarm");
        assert!(finished.interleaved.len() >= 16_000 + 3_200);
        let new_ms = (finished.interleaved.len() - 16_000) as u64 * 1000 / 16_000;
        assert!(new_ms >= 200, "only {new_ms} ms arrived after the mark");
        // A same-format open must not wipe the ring the way the idle scan did.
        {
            let mut guard = state.lock().expect("buffer");
            let kept = guard.ring.len();
            let channels = guard.channels;
            let sample_rate = guard.sample_rate;
            guard.apply_format(channels, sample_rate);
            assert_eq!(guard.ring.len(), kept, "reopening the same format cleared the pre-roll");
        }
    }

#[test]
fn stable_default_does_not_reopen_and_preroll_is_instant() {
    assert_eq!(
        plan_reopen(true, true, false, false, false),
        None,
        "a healthy stream must stay open between takes"
    );
    assert_eq!(
        plan_reopen(true, true, false, true, false),
        None,
        "a default-device event for the same device must not reopen"
    );
    assert_eq!(
        plan_reopen(true, true, true, false, false),
        None,
        "saving the microphone that is already open must not reopen"
    );
    assert_eq!(
        plan_reopen(true, true, false, true, true),
        Some(ReopenReason::DeviceChanged)
    );
    assert_eq!(plan_reopen(true, true, true, false, true), Some(ReopenReason::Preference));
    assert_eq!(plan_reopen(false, false, false, false, false), Some(ReopenReason::Launch));
    assert_eq!(
        plan_reopen(false, true, false, false, false),
        Some(ReopenReason::StreamError)
    );
    assert_eq!(warm_log_line("launch"), "warm stream opened/reopened reason=launch");
    assert_eq!(
        warm_log_line("device-changed"),
        "warm stream opened/reopened reason=device-changed"
    );
    assert_eq!(reported_first_sample_ms(16_000, Some(4_000)), Some(0));
    assert_eq!(reported_first_sample_ms(0, Some(40)), Some(40));
    assert_eq!(reported_first_sample_ms(0, None), None);

    let level = Arc::new(AtomicU32::new(0));
    let state = Mutex::new(Capture::new(level));
    {
        let mut guard = state.lock().expect("buffer");
        guard.apply_format(1, 48_000);
        guard.ring.extend(std::iter::repeat(0.01).take(8_000));
        guard.apply_format(1, 48_000);
        assert_eq!(guard.ring.len(), 8_000);
        guard.apply_format(2, 48_000);
        assert!(guard.ring.is_empty(), "a format change drops samples that no longer match");
    }
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
    let warm = WarmMic::spawn(None, None);
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

