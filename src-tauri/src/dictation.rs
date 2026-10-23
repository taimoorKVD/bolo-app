//! Recording, transcription, and insertion state machine.
//!
//! Presses that arrive while a transcript is still in flight are ignored so a
//! second take cannot overlap the paste. A release shorter than a tap waits
//! briefly for a second press, which latches hands-free mode. A normal hold
//! stops on release with no extra wait.

use crate::audio::FinishedAudio;
use crate::cleanup;
use crate::context;
use crate::credentials::SecretKey;
use crate::hotkey::{display, HotkeyEdge};
use crate::insert::{self, InsertMethod};
use crate::listener::ListenerControl;
use crate::logbuf::log_line;
use crate::pipeline::{self, Product};
use crate::spool;
use crate::store::{self, HistoryEntry, Settings};
use crate::transcribe;
use crate::warm::{self, LiveTake};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// A release shorter than this may be the first half of a double-tap.
const TAP_MS: u64 = 240;
/// How long a short release waits for the second tap.
const DOUBLE_TAP_WAIT_MS: u64 = 320;
/// Takes shorter than this get the hold-the-key hint when they are silent.
const HINT_HOLD_MS: u64 = 450;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationEvent {
    pub phase: String,
    pub message: Option<String>,
    pub elapsed_ms: u64,
    pub level: f32,
    /// "Groq", "On this PC", or "Groq + on this PC" once a take has a transcript.
    #[serde(default)]
    pub engine: Option<String>,
}

enum Phase {
    Idle,
    /// The mic is opening. A release here must not cancel the take.
    Arming,
    Recording,
    Busy,
}

struct Engine {
    phase: Phase,
    take: Option<LiveTake>,
    generation: u64,
    /// The hotkey went up before the microphone had armed this take.
    /// Kept so a release during arming is remembered, not used to drop audio.
    stop_requested: bool,
    /// Key-down time. Hold length is measured from here, not from when the mic arms.
    press_at: Option<Instant>,
    released_during_arm: bool,
    spool_id: Option<String>,
    hands_free: bool,
    awaiting_second: bool,
    /// The release that ended (or will end) this take was a short tap.
    short_release: bool,
    /// Hold length at the moment of release, before the double-tap wait.
    release_held_ms: u64,
    /// Key-up that ends the take. Set before the double-tap wait so release
    /// time includes that wait.
    release_mark: Option<Instant>,
    /// Milliseconds from key-down until the Listening overlay was shown.
    overlay_visible_ms: u64,
}

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub history: Mutex<Vec<HistoryEntry>>,
    engine: Mutex<Engine>,
    pub listener: ListenerControl,
    pub config_dir: std::path::PathBuf,
    pub last_error: Mutex<Option<String>>,
    pub warm: warm::WarmMic,
    slots: Arc<pipeline::Slots>,
    jobs: Mutex<pipeline::Jobs>,
    /// Held from the moment chunks are drained until they are queued to transcribe,
    /// so stop cannot miss a chunk the meter already took.
    pump_gate: Mutex<()>,
    last_text: Mutex<Option<String>>,
}

impl AppState {
    pub fn new(settings: Settings, history: Vec<HistoryEntry>, config_dir: std::path::PathBuf) -> Self {
        let listener = ListenerControl::new(settings.hotkey.clone());
        listener.set_paste_hotkey(settings.paste_hotkey.clone());
        let warm = warm::WarmMic::spawn(settings.microphone.clone(), Some(config_dir.clone()));
        Self {
            settings: Mutex::new(settings),
            history: Mutex::new(history),
            engine: Mutex::new(Engine {
                phase: Phase::Idle,
                take: None,
                generation: 0,
                stop_requested: false,
                hands_free: false,
                awaiting_second: false,
                short_release: false,
                release_held_ms: 0,
                release_mark: None,
                press_at: None,
                released_during_arm: false,
                spool_id: None,
                overlay_visible_ms: 0,
            }),
            listener,
            config_dir,
            last_error: Mutex::new(None),
            warm,
            slots: pipeline::Slots::new(2),
            jobs: Mutex::new(pipeline::Jobs::new()),
            pump_gate: Mutex::new(()),
            last_text: Mutex::new(None),
        }
    }
}

struct Clock {
    release_started: Instant,
    held_ms: u64,
    open_wait_ms: u64,
    stop_ms: u64,
    short_press: bool,
    hint_held_ms: u64,
    overlay_ms: u64,
    first_sample_ms: Option<u64>,
}

struct Staged {
    held_ms: u64,
    open_wait_ms: u64,
    stop_ms: u64,
    encode_ms: u64,
    bytes: usize,
    encode_fmt: String,
    groq_ms: u64,
    groq_tries: u8,
    cleanup_ms: u64,
    /// Empty until cleanup runs. Then "applied", "off", "verbatim", or the fallback reason.
    cleanup_why: String,
    paste_ms: u64,
    tone: String,
    release_started: Instant,
    engine: String,
    overlay_ms: u64,
    first_sample_ms: Option<u64>,
    speech_language: String,
    speech_path: String,
}

struct Outcome {
    text: String,
    cleaned: bool,
    note: Option<String>,
    duration_ms: u64,
    model: String,
    staged: Staged,
    spool_id: String,
    engine: Option<String>,
}

struct TakeError {
    message: String,
    hint: bool,
    staged: Staged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PressDecision {
    Begin,
    StopHandsFree,
    LatchHandsFree,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReleaseDecision {
    EndNow,
    ArmDoubleTap,
    /// The key came up while the microphone was still opening. Keep the take.
    NoteArmRelease,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AfterArm {
    /// The key is still down. This is the first press starting capture.
    Listen,
    /// They let go after a real hold. Transcribe what was captured.
    EndTake,
    /// A short tap. Wait to see if it becomes hands-free.
    WaitSecondTap,
}

/// What to do once the microphone finishes opening.
pub(crate) fn after_arm(released_during_arm: bool, press_age_ms: u64) -> AfterArm {
    if !released_during_arm {
        AfterArm::Listen
    } else if press_age_ms < TAP_MS {
        AfterArm::WaitSecondTap
    } else {
        AfterArm::EndTake
    }
}

pub(crate) fn press_decision(hands_free: bool, awaiting_second: bool, idle: bool) -> PressDecision {
    if hands_free {
        PressDecision::StopHandsFree
    } else if awaiting_second {
        PressDecision::LatchHandsFree
    } else if idle {
        PressDecision::Begin
    } else {
        PressDecision::Ignore
    }
}

pub(crate) fn release_decision(
    hands_free: bool,
    recording: bool,
    take_ready: bool,
    held_ms: u64,
    arming: bool,
) -> ReleaseDecision {
    if hands_free {
        return ReleaseDecision::Ignore;
    }
    // A release before the take exists used to cancel it. That made the first
    // press look like a failure: the key-up queued during a slow open was
    // treated as a zero-length tap, the audio was dropped, and the overlay
    // said "Not inserted". Remember the release and start capture anyway.
    if arming || (recording && !take_ready) {
        return ReleaseDecision::NoteArmRelease;
    }
    if !recording {
        return ReleaseDecision::Ignore;
    }
    if held_ms < TAP_MS {
        ReleaseDecision::ArmDoubleTap
    } else {
        ReleaseDecision::EndNow
    }
}

pub fn silence_hint(held_ms: u64, short_press: bool) -> &'static str {
    if short_press || held_ms < HINT_HOLD_MS {
        "Hold the hotkey while you talk, or double-tap it for hands-free."
    } else {
        "Didn't catch any speech. Speak a little louder, or check the microphone in Settings."
    }
}

fn recording_caption(hands_free: bool, _note: &Option<String>) -> Option<String> {
    if hands_free {
        Some("Hands-free — tap the hotkey to stop".into())
    } else {
        None
    }
}

fn instant_listen_event(level: f32) -> DictationEvent {
    DictationEvent {
        phase: "recording".into(),
        message: None,
        elapsed_ms: 0,
        level,
        engine: None,
    }
}

pub fn hotkey(app: &AppHandle, edge: HotkeyEdge) {
    match edge {
        HotkeyEdge::Pressed => on_press(app),
        HotkeyEdge::Released => on_release(app),
    }
}

fn on_press(app: &AppHandle) {
    let state = app.state::<AppState>();
    let decision = {
        let engine = lock(&state.engine);
        press_decision(
            engine.hands_free,
            engine.awaiting_second,
            matches!(engine.phase, Phase::Idle),
        )
    };
    match decision {
        PressDecision::StopHandsFree => {
            {
                let mut engine = lock(&state.engine);
                engine.hands_free = false;
            }
            end(app);
        }
        PressDecision::LatchHandsFree => {
            let note = {
                let mut engine = lock(&state.engine);
                engine.awaiting_second = false;
                engine.hands_free = true;
                engine.short_release = false;
                engine.release_mark = None;
                engine.take.as_ref().and_then(|take| take.device_note.clone())
            };
            let started = lock(&state.engine)
                .take
                .as_ref()
                .map(|take| take.started.elapsed().as_millis() as u64)
                .unwrap_or(0);
            publish(
                app,
                &DictationEvent {
                    phase: "recording".into(),
                    message: recording_caption(true, &note),
                    elapsed_ms: started,
                    level: 0.0,
                    engine: None,
                },
            );
        }
        PressDecision::Begin => {
            insert::note_foreground();
            {
                let mut engine = lock(&state.engine);
                engine.press_at = Some(Instant::now());
            }
            begin(app);
        }
        PressDecision::Ignore => {}
    }
}

fn on_release(app: &AppHandle) {
    let state = app.state::<AppState>();
    let action = {
        let mut engine = lock(&state.engine);
        let held_ms = engine
            .press_at
            .map(|start| start.elapsed().as_millis() as u64)
            .or_else(|| {
                engine
                    .take
                    .as_ref()
                    .map(|take| take.started.elapsed().as_millis() as u64)
            })
            .unwrap_or(0);
        let decision = release_decision(
            engine.hands_free,
            matches!(engine.phase, Phase::Recording),
            engine.take.is_some(),
            held_ms,
            matches!(engine.phase, Phase::Arming),
        );
        match decision {
            ReleaseDecision::ArmDoubleTap => {
                engine.awaiting_second = true;
                engine.short_release = true;
                engine.release_held_ms = held_ms;
                engine.release_mark = Some(Instant::now());
                Some((decision, engine.generation))
            }
            ReleaseDecision::NoteArmRelease => {
                engine.released_during_arm = true;
                engine.short_release = held_ms < TAP_MS;
                engine.release_held_ms = held_ms;
                engine.release_mark = Some(Instant::now());
                None
            }
            ReleaseDecision::EndNow => {
                engine.short_release = false;
                engine.release_held_ms = held_ms;
                engine.release_mark = Some(Instant::now());
                Some((decision, engine.generation))
            }
            ReleaseDecision::Ignore => None,
        }
    };
    match action {
        Some((ReleaseDecision::EndNow, _)) => end(app),
        Some((ReleaseDecision::ArmDoubleTap, generation)) => arm_double_tap(app.clone(), generation),
        _ => {}
    }
}

fn arm_double_tap(app: AppHandle, generation: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(DOUBLE_TAP_WAIT_MS));
        let state = app.state::<AppState>();
        let fire = {
            let engine = lock(&state.engine);
            engine.awaiting_second
                && engine.generation == generation
                && matches!(engine.phase, Phase::Recording)
        };
        if fire {
            end(&app);
        }
    });
}

pub fn begin(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (generation, keydown) = {
        let mut engine = lock(&state.engine);
        if !matches!(engine.phase, Phase::Idle) {
            return;
        }
        // Arming, not Recording: the hotkey thread stays free to see the key-up,
        // and Record a test cannot toggle itself off before the mic is open.
        engine.phase = Phase::Arming;
        engine.generation = engine.generation.wrapping_add(1);
        engine.stop_requested = false;
        engine.released_during_arm = false;
        engine.hands_free = false;
        engine.awaiting_second = false;
        engine.short_release = false;
        engine.release_held_ms = 0;
        engine.release_mark = None;
        let keydown = Instant::now();
        engine.press_at = Some(keydown);
        engine.spool_id = Some(store::new_history_id());
        engine.overlay_visible_ms = 0;
        (engine.generation, keydown)
    };
    *lock(&state.last_error) = None;
    let settings = lock(&state.settings).clone();
    if let Some(tray) = app.tray_by_id("bolo") {
        let _ = tray.set_tooltip(Some(format!("Bolo.com — hold {}", display(&settings.hotkey))));
    }
    // Listening is on screen before the warm thread is asked to do anything.
    // That thread only copies the pre-roll; it does not open the microphone.
    publish(app, &instant_listen_event(state.warm.level()));
    let overlay_ms = keydown.elapsed().as_millis() as u64;
    {
        let mut engine = lock(&state.engine);
        if engine.generation == generation {
            engine.overlay_visible_ms = overlay_ms;
        }
    }
    log_line(
        &state.config_dir,
        "info",
        &format!("listen keydown_overlay_ms={overlay_ms}"),
    );
    let app = app.clone();
    std::thread::Builder::new()
        .name("bolo-arm".into())
        .spawn(move || {
            let state = app.state::<AppState>();
            let preference = lock(&state.settings).microphone.clone();
            match state.warm.begin(preference.as_deref()) {
                Ok(take) => on_armed(&app, take, generation),
                Err(err) => {
                    let still = {
                        let engine = lock(&state.engine);
                        engine.generation == generation && matches!(engine.phase, Phase::Arming)
                    };
                    if still {
                        mark_idle(&app);
                        fail(&app, &err);
                    }
                }
            }
        })
        .ok();
}

fn on_armed(app: &AppHandle, take: LiveTake, generation: u64) {
    let state = app.state::<AppState>();
    let (level, stop_flag) = take.meter();
    let started = take.started;
    let note = take.device_note.clone();
    let action = {
        let mut engine = lock(&state.engine);
        if engine.generation != generation || !matches!(engine.phase, Phase::Arming) {
            drop(engine);
            let _ = take.finish();
            return;
        }
        let released = engine.released_during_arm;
        let age = engine
            .press_at
            .map(|start| start.elapsed().as_millis() as u64)
            .unwrap_or(0);
        engine.released_during_arm = false;
        engine.stop_requested = false;
        engine.take = Some(take);
        engine.phase = Phase::Recording;
        after_arm(released, age)
    };
    if let Some(message) = note.as_ref().filter(|message| !message.is_empty()) {
        log_line(&state.config_dir, "info", message);
    }
    match action {
        AfterArm::Listen => {
            spawn_meter(app.clone(), level, stop_flag, started, None, generation);
        }
        AfterArm::WaitSecondTap => {
            spawn_meter(app.clone(), level, stop_flag, started, None, generation);
            {
                let mut engine = lock(&state.engine);
                engine.awaiting_second = true;
            }
            arm_double_tap(app.clone(), generation);
        }
        AfterArm::EndTake => end(app),
    }
}

pub fn end(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (take, generation, short_press, hint_held_ms, release_started, held_ms, spool_id, overlay_ms, keydown) = {
        let mut engine = lock(&state.engine);
        engine.awaiting_second = false;
        if !matches!(engine.phase, Phase::Recording) {
            return;
        }
        if engine.take.is_none() {
            // Still arming would be a different phase. If we got here with no
            // take, remember the release instead of dropping the next one.
            engine.released_during_arm = true;
            return;
        }
        let overlay_ms = engine.overlay_visible_ms;
        let keydown = engine.press_at;
        let release_started = engine.release_mark.take().unwrap_or_else(Instant::now);
        let short_press = engine.short_release;
        let held_ms = engine
            .press_at
            .map(|start| start.elapsed().as_millis() as u64)
            .unwrap_or_else(|| {
                engine
                    .take
                    .as_ref()
                    .map(|take| take.started.elapsed().as_millis() as u64)
                    .unwrap_or(0)
            });
        let hint_held_ms = if engine.release_held_ms > 0 {
            engine.release_held_ms
        } else {
            held_ms
        };
        let spool_id = engine.spool_id.clone().unwrap_or_else(store::new_history_id);
        engine.short_release = false;
        engine.hands_free = false;
        engine.release_held_ms = 0;
        engine.phase = Phase::Busy;
        (
            engine.take.take(),
            engine.generation,
            short_press,
            hint_held_ms,
            release_started,
            held_ms,
            spool_id,
            overlay_ms,
            keydown,
        )
    };
    let Some(take) = take else {
        mark_idle(app);
        return;
    };
    let first_sample_ms = keydown.and_then(|start| take.first_sample_ms_since(start));
    if let Some(ms) = first_sample_ms {
        log_line(
            &state.config_dir,
            "info",
            &format!("listen keydown_first_sample_ms={ms}"),
        );
    }
    let open_wait_ms = take.open_wait_ms;
    let bus = take.chunk_bus();
    enqueue(app, take.drain_chunks());
    let stop_started = Instant::now();
    let finished = match take.finish() {
        Ok(mut finished) => {
            finished.elapsed_ms = held_ms;
            finished
        }
        Err(err) => {
            mark_idle(app);
            fail(app, &err);
            return;
        }
    };
    let stop_ms = stop_started.elapsed().as_millis() as u64;
    let (flights, settings) = {
        // The meter holds this from drain through enqueue. Waiting for it means
        // a chunk it already pulled is in the flight list before we take it.
        let _gate = lock(&state.pump_gate);
        let mut tail = Vec::new();
        if let Ok(mut queue) = bus.lock() {
            tail.extend(queue.drain(..));
        }
        if !finished.interleaved.is_empty() {
            tail.push(finished);
        }
        enqueue(app, tail);
        let flights = lock(&state.jobs).take();
        let settings = lock(&state.settings).clone();
        (flights, settings)
    };
    let clock = Clock {
        release_started,
        held_ms,
        open_wait_ms,
        stop_ms,
        short_press,
        hint_held_ms,
        overlay_ms,
        first_sample_ms,
    };
    publish(
        app,
        &DictationEvent {
            phase: "transcribing".into(),
            message: None,
            elapsed_ms: held_ms,
            level: 0.0,
            engine: None,
        },
    );
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let products = tauri::async_runtime::spawn_blocking(move || pipeline::collect(flights))
            .await
            .unwrap_or_default();
        match process(products, settings, clock, spool_id).await {
            Ok(outcome) => deliver(&app, outcome, generation).await,
            Err(err) => {
                let release_ms = err.staged.release_started.elapsed().as_millis() as u64;
                log_stages(&app, &err.staged, release_ms);
                mark_idle(&app);
                if err.hint {
                    hint(&app, &err.message);
                } else {
                    fail(&app, &err.message);
                }
            }
        }
    });
}

fn enqueue(app: &AppHandle, audios: Vec<FinishedAudio>) {
    if audios.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    let settings = lock(&state.settings).clone();
    let recent: Vec<String> = lock(&state.history)
        .iter()
        .take(2)
        .map(|entry| entry.text.clone())
        .collect();
    let spool_id = lock(&state.engine)
        .spool_id
        .clone()
        .unwrap_or_else(store::new_history_id);
    pipeline::launch(
        &state.slots,
        &state.jobs,
        audios,
        pipeline::JobEnv {
            settings,
            recent,
            config_dir: state.config_dir.clone(),
            spool_id,
        },
    );
}

pub fn toggle(app: &AppHandle) {
    insert::note_foreground();
    let state = app.state::<AppState>();
    let phase = {
        let engine = lock(&state.engine);
        if matches!(engine.phase, Phase::Recording) {
            "recording"
        } else if matches!(engine.phase, Phase::Arming) {
            "arming"
        } else if matches!(engine.phase, Phase::Idle) {
            "idle"
        } else {
            "busy"
        }
    };
    match phase {
        "recording" => end(app),
        // A second click while the first is still opening used to cancel it.
        "arming" => {}
        "idle" => {
            {
                let mut engine = lock(&state.engine);
                if engine.press_at.is_none() {
                    engine.press_at = Some(Instant::now());
                }
            }
            begin(app);
        }
        _ => {}
    }
}

pub fn paste_last(app: &AppHandle) {
    let state = app.state::<AppState>();
    let busy = {
        let engine = lock(&state.engine);
        !matches!(engine.phase, Phase::Idle)
    };
    if busy {
        return;
    }
    let text = lock(&state.last_text).clone().or_else(|| {
        spool::latest(&state.config_dir).and_then(|recovered| recovered.text)
    });
    let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
        fail(app, "There's no take to paste again yet.");
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || {
        insert::note_foreground();
        match insert::insert_text(&text) {
            Ok(inserted) if inserted.verified && matches!(inserted.method, InsertMethod::Clipboard) => {
                let state = app.state::<AppState>();
                if let Some(recovered) = spool::latest(&state.config_dir) {
                    if recovered.text.as_deref() == Some(text.as_str()) {
                        spool::remove(&state.config_dir, &recovered.id);
                    }
                }
                publish(
                    &app,
                    &DictationEvent {
                        phase: "done".into(),
                        message: Some("Inserted".into()),
                        elapsed_ms: 0,
                        level: 0.0,
                        engine: None,
                    },
                );
                let generation = lock(&app.state::<AppState>().engine).generation;
                schedule_hide(app, generation, 1_600);
            }
            Ok(_) => {
                publish(
                    &app,
                    &DictationEvent {
                        phase: "typed".into(),
                        message: Some("Typed".into()),
                        elapsed_ms: 0,
                        level: 0.0,
                        engine: None,
                    },
                );
                schedule_hide(app.clone(), lock(&app.state::<AppState>().engine).generation, 1_600);
            }
            Err(err) => fail(&app, &format!("Couldn't paste the last take ({err}).")),
        }
    });
}

pub fn recover_spool(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(recovered) = spool::latest(&state.config_dir) else {
        return;
    };
    if let Some(text) = recovered.text {
        *lock(&state.last_text) = Some(text.clone());
        log_line(
            &state.config_dir,
            "info",
            &format!(
                "recovered take {} ({} chars) that hadn't finished pasting",
                recovered.id,
                text.chars().count()
            ),
        );
    } else if !recovered.audio.is_empty() {
        log_line(
            &state.config_dir,
            "info",
            &format!(
                "recovered take {} audio that hadn't been transcribed; paste-again will retry it",
                recovered.id
            ),
        );
    }
}

fn spawn_meter(
    app: AppHandle,
    level: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    started: Instant,
    note: Option<String>,
    generation: u64,
) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            pump_chunks(&app);
            let elapsed = started.elapsed();
            let hands_free = app
                .state::<AppState>()
                .engine
                .lock()
                .map(|engine| engine.hands_free)
                .unwrap_or(false);
            let rms = f32::from_bits(level.load(Ordering::Relaxed));
            let _ = app.emit(
                "dictation-state",
                DictationEvent {
                    phase: "recording".into(),
                    message: recording_caption(hands_free, &note),
                    elapsed_ms: elapsed.as_millis() as u64,
                    level: (rms * 4.5).clamp(0.0, 1.0),
                    engine: None,
                },
            );
            std::thread::sleep(Duration::from_millis(80));
        }
        pump_chunks(&app);
        let _ = generation;
    });
}

fn pump_chunks(app: &AppHandle) {
    let state = app.state::<AppState>();
    let _gate = lock(&state.pump_gate);
    let queued = {
        let engine = lock(&state.engine);
        match engine.take.as_ref() {
            Some(take) => take.drain_chunks(),
            None => Vec::new(),
        }
    };
    enqueue(app, queued);
}

async fn process(
    products: Vec<Product>,
    settings: Settings,
    clock: Clock,
    spool_id: String,
) -> Result<Outcome, TakeError> {
    let engine = pipeline::engine_label(&products).unwrap_or_default();
    let mut staged = Staged {
        held_ms: clock.held_ms,
        open_wait_ms: clock.open_wait_ms,
        stop_ms: clock.stop_ms,
        encode_ms: products.iter().map(|product| product.encode_ms).max().unwrap_or(0),
        bytes: products.iter().map(|product| product.bytes).sum(),
        encode_fmt: products
            .iter()
            .find(|product| !product.format.is_empty())
            .map(|product| product.format.clone())
            .unwrap_or_else(|| "flac".into()),
        groq_ms: products.iter().map(|product| product.remote_ms).max().unwrap_or(0),
        groq_tries: products.iter().map(|product| product.tries).max().unwrap_or(0),
        cleanup_ms: 0,
        cleanup_why: String::new(),
        paste_ms: 0,
        tone: "none".into(),
        release_started: clock.release_started,
        engine: engine.clone(),
        overlay_ms: clock.overlay_ms,
        first_sample_ms: clock.first_sample_ms,
        speech_language: String::new(),
        speech_path: String::new(),
    };
    let (speech_language, speech_path) = speech_meta(&products);
    staged.speech_language = speech_language;
    staged.speech_path = speech_path;
    let stitched = match pipeline::stitch_products(&products) {
        Ok(text) => text,
        Err(message) => {
            let hint = products.iter().all(|product| product.silence || product.error.is_none())
                && products.iter().any(|product| product.silence);
            return Err(TakeError {
                message,
                hint: hint && products.iter().all(|product| product.error.is_none()),
                staged,
            });
        }
    };
    if stitched.trim().is_empty() {
        let only_silence = products.iter().all(|product| product.error.is_none());
        return Err(TakeError {
            message: if only_silence {
                silence_hint(clock.hint_held_ms, clock.short_press).into()
            } else {
                "Nothing to insert.".into()
            },
            hint: true,
            staged,
        });
    }
    let tone = context::foreground_tone();
    staged.tone = tone.as_str().into();
    let key = match SecretKey::load() {
        Ok(key) => key,
        Err(err) => {
            // The transcript already exists. Keep it so paste-again can use it
            // even when cleanup cannot reach Groq.
            let _ = err;
            return Ok(Outcome {
                text: stitched,
                cleaned: false,
                note: None,
                duration_ms: clock.held_ms,
                model: settings.model.clone(),
                staged,
                spool_id,
                engine: pipeline::engine_label(&products),
            });
        }
    };
    let model = settings.model.clone();
    let cleanup_started = Instant::now();
    let using_builtin = key.is_builtin();
    let cleaned = cleanup::clean_transcript(
        transcribe::http_client(),
        key.expose(),
        &stitched,
        settings.cleanup_enabled && !settings.verbatim,
        if settings.verbatim { cleanup::Tone::Verbatim } else { tone },
        settings.verbatim,
        &settings.cleanup_model,
        using_builtin,
    )
    .await;
    staged.cleanup_ms = cleanup_started.elapsed().as_millis() as u64;
    staged.cleanup_why = cleaned.why;
    drop(key);
    let text = cleaned.text.trim().to_string();
    if text.is_empty() {
        return Err(TakeError {
            message: "Nothing to insert.".into(),
            hint: true,
            staged,
        });
    }
    Ok(Outcome {
        text,
        cleaned: cleaned.used_model,
        note: cleaned.note,
        duration_ms: clock.held_ms,
        model,
        staged,
        spool_id,
        engine: pipeline::engine_label(&products),
    })
}

async fn deliver(app: &AppHandle, mut outcome: Outcome, generation: u64) {
    absorb_dictionary(app, &outcome.text);
    let state = app.state::<AppState>();
    spool::write_text(&state.config_dir, &outcome.spool_id, &outcome.text);
    *lock(&state.last_text) = Some(outcome.text.clone());
    let text = outcome.text.clone();
    let paste_started = Instant::now();
    let inserted = tauri::async_runtime::spawn_blocking(move || insert::insert_text(&text)).await;
    outcome.staged.paste_ms = paste_started.elapsed().as_millis() as u64;
    let release_ms = outcome.staged.release_started.elapsed().as_millis() as u64;
    let inserted = match inserted {
        Ok(Ok(inserted)) => inserted,
        Ok(Err(err)) => {
            log_stages(app, &outcome.staged, release_ms);
            remember(app, &outcome);
            mark_idle(app);
            fail(
                app,
                &format!("Couldn't insert the text ({err}). It's saved in History. Paste it again with the paste hotkey."),
            );
            return;
        }
        Err(err) => {
            log_stages(app, &outcome.staged, release_ms);
            remember(app, &outcome);
            mark_idle(app);
            fail(
                app,
                &format!("Couldn't insert the text ({err}). It's saved in History. Paste it again with the paste hotkey."),
            );
            return;
        }
    };
    // "Inserted" only after the target accepted the paste. Typing is reported
    // as Typed, including when the field can't be read back.
    let word = insert::status_word(inserted.method, inserted.verified);
    let verified_paste = word == "Inserted";
    if verified_paste {
        spool::remove(&state.config_dir, &outcome.spool_id);
    }
    log_stages(app, &outcome.staged, release_ms);
    log_line(
        &state.config_dir,
        "info",
        &format!(
            "inserted chars={} cleaned={} via={} verified={} engine={}",
            outcome.text.chars().count(),
            outcome.cleaned,
            match inserted.method {
                InsertMethod::Clipboard => "clipboard",
                InsertMethod::Typed => "typed",
            },
            inserted.verified,
            outcome.engine.as_deref().unwrap_or("none")
        ),
    );
    remember(app, &outcome);
    mark_idle(app);
    let (phase, fallback) = if verified_paste {
        ("done", "Inserted")
    } else {
        ("typed", "Typed")
    };
    let message = outcome.note.clone().unwrap_or_else(|| fallback.into());
    publish(
        app,
        &DictationEvent {
            phase: phase.into(),
            message: Some(message),
            elapsed_ms: outcome.duration_ms,
            level: 0.0,
            engine: outcome.engine.clone(),
        },
    );
    schedule_hide(app.clone(), generation, 1_600);
}

fn absorb_dictionary(app: &AppHandle, text: &str) {
    let state = app.state::<AppState>();
    let added = {
        let mut settings = lock(&state.settings);
        let added = cleanup::learn_terms(text, &mut settings.dictionary);
        if added.is_empty() {
            return;
        }
        if let Err(err) = store::save_settings(&state.config_dir, &settings) {
            let added_set = added.clone();
            settings.dictionary.retain(|term| !added_set.iter().any(|name| name == term));
            log_line(&state.config_dir, "error", &format!("dictionary save: {err}"));
            return;
        }
        added
    };
    for name in &added {
        log_line(&state.config_dir, "info", &format!("dictionary +{name}"));
    }
}

fn log_stages(app: &AppHandle, staged: &Staged, release_ms: u64) {
    let state = app.state::<AppState>();
    log_line(&state.config_dir, "info", &stage_line(staged, release_ms));
}

fn stage_line(staged: &Staged, release_ms: u64) -> String {
    let why = if staged.cleanup_why.is_empty() {
        String::new()
    } else {
        format!(" cleanup_why={}", quote_log(&staged.cleanup_why))
    };
    let fmt = if staged.encode_fmt.is_empty() {
        "wav"
    } else {
        staged.encode_fmt.as_str()
    };
    let mut line = format!(
        "stages held={} open_wait={} stop={} encode={} bytes={} fmt={} groq={} tries={} cleanup={}{why} paste={} release={} tone={}",
        staged.held_ms,
        staged.open_wait_ms,
        staged.stop_ms,
        staged.encode_ms,
        staged.bytes,
        fmt,
        staged.groq_ms,
        staged.groq_tries,
        staged.cleanup_ms,
        staged.paste_ms,
        release_ms,
        staged.tone
    );
    if !staged.engine.is_empty() {
        line.push_str(" engine=");
        line.push_str(&staged.engine);
    }
    let first_sample = match staged.first_sample_ms {
        Some(ms) => ms.to_string(),
        None => "na".into(),
    };
    let language = if staged.speech_language.is_empty() {
        "na"
    } else {
        staged.speech_language.as_str()
    };
    let path = if staged.speech_path.is_empty() {
        "na"
    } else {
        staged.speech_path.as_str()
    };
    line.push_str(&format!(
        " keydown_overlay_ms={} keydown_first_sample_ms={} language={} path={}",
        staged.overlay_ms, first_sample, language, path
    ));
    line
}

fn speech_meta(products: &[Product]) -> (String, String) {
    let language = products
        .iter()
        .find_map(|product| {
            let language = product.language.trim();
            if language.is_empty() {
                None
            } else {
                Some(language.to_string())
            }
        })
        .unwrap_or_default();
    let path = products
        .iter()
        .find_map(|product| {
            let path = product.speech_path.trim();
            if path.is_empty() {
                None
            } else {
                Some(path.to_string())
            }
        })
        .unwrap_or_default();
    (language, path)
}

fn quote_log(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "'"))
}

fn remember(app: &AppHandle, outcome: &Outcome) {
    let state = app.state::<AppState>();
    if !lock(&state.settings).history_enabled {
        return;
    }
    let entry = HistoryEntry {
        id: store::new_history_id(),
        text: outcome.text.clone(),
        created_unix_ms: store::now_ms(),
        model: outcome.model.clone(),
        cleaned: outcome.cleaned,
        duration_ms: outcome.duration_ms,
    };
    let mut history = lock(&state.history);
    store::push_history(&mut history, entry);
    if let Err(err) = store::save_history(&state.config_dir, &history) {
        log_line(&state.config_dir, "error", &err);
    }
}

fn fail(app: &AppHandle, message: &str) {
    let state = app.state::<AppState>();
    log_line(&state.config_dir, "error", message);
    *lock(&state.last_error) = Some(message.to_string());
    if let Some(tray) = app.tray_by_id("bolo") {
        let _ = tray.set_tooltip(Some(format!("Bolo.com — {message}")));
    }
    show_problem(app, message, 8_000);
}

fn hint(app: &AppHandle, message: &str) {
    let state = app.state::<AppState>();
    log_line(&state.config_dir, "info", &format!("hint {message}"));
    // Silence and a short tap are normal. Hide the pill instead of flashing an error.
    publish(
        app,
        &DictationEvent {
            phase: "idle".into(),
            message: None,
            elapsed_ms: 0,
            level: 0.0,
            engine: None,
        },
    );
}

fn show_problem(app: &AppHandle, message: &str, hide_ms: u64) {
    let state = app.state::<AppState>();
    let generation = lock(&state.engine).generation;
    publish(
        app,
        &DictationEvent {
            phase: "error".into(),
            message: Some(message.to_string()),
            elapsed_ms: 0,
            level: 0.0,
            engine: None,
        },
    );
    schedule_hide(app.clone(), generation, hide_ms);
}

fn schedule_hide(app: AppHandle, generation: u64, delay_ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(delay_ms));
        let state = app.state::<AppState>();
        let still_idle = {
            let engine = lock(&state.engine);
            engine.generation == generation && matches!(engine.phase, Phase::Idle)
        };
        if still_idle {
            publish(
                &app,
                &DictationEvent {
                    phase: "idle".into(),
                    message: None,
                    elapsed_ms: 0,
                    level: 0.0,
                    engine: None,
                },
            );
        }
    });
}

fn mark_idle(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut engine = lock(&state.engine);
    engine.phase = Phase::Idle;
    engine.take = None;
    engine.stop_requested = false;
    engine.hands_free = false;
    engine.awaiting_second = false;
    engine.press_at = None;
    engine.released_during_arm = false;
}

fn publish(app: &AppHandle, event: &DictationEvent) {
    let _ = app.emit("dictation-state", event);
    let Some(overlay) = app.get_webview_window("overlay") else {
        return;
    };
    if event.phase == "idle" {
        let _ = overlay.hide();
        return;
    }
    let (width, height) = if event.phase == "error" {
        (340.0, 148.0)
    } else {
        (280.0, 56.0)
    };
    let _ = overlay.set_size(LogicalSize::new(width, height));
    position_overlay(&overlay);
    let _ = overlay.show();
}

fn position_overlay(window: &WebviewWindow) {
    let cursor = window.cursor_position().ok();
    let monitors = window.available_monitors().unwrap_or_default();
    let monitor = monitors.into_iter().find(|monitor| {
        let Some(cursor) = cursor else {
            return false;
        };
        let pos = monitor.position();
        let size = monitor.size();
        cursor.x >= pos.x as f64
            && cursor.x < pos.x as f64 + size.width as f64
            && cursor.y >= pos.y as f64
            && cursor.y < pos.y as f64 + size.height as f64
    });
    let monitor = match monitor {
        Some(monitor) => monitor,
        None => match window.primary_monitor() {
            Ok(Some(monitor)) => monitor,
            _ => return,
        },
    };
    let size = window.outer_size().unwrap_or(PhysicalSize::new(360, 92));
    let pos = monitor.position();
    let mon_size = monitor.size();
    let margin = (78.0 * monitor.scale_factor()) as i32;
    let x = pos.x + (mon_size.width as i32 - size.width as i32) / 2;
    let y = pos.y + mon_size.height as i32 - size.height as i32 - margin;
    let _ = window.set_position(PhysicalPosition::new(x, y.max(pos.y)));
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_tap_does_not_delay_a_normal_hold() {
        assert_eq!(press_decision(false, false, true), PressDecision::Begin);
        assert_eq!(
            release_decision(false, true, true, TAP_MS, false),
            ReleaseDecision::EndNow
        );
        assert_eq!(
            release_decision(false, true, true, TAP_MS - 1, false),
            ReleaseDecision::ArmDoubleTap
        );
        assert_eq!(
            press_decision(false, true, false),
            PressDecision::LatchHandsFree
        );
        assert_eq!(release_decision(true, true, true, 20, false), ReleaseDecision::Ignore);
        assert_eq!(press_decision(true, false, false), PressDecision::StopHandsFree);
        assert_eq!(press_decision(false, false, false), PressDecision::Ignore);
        assert_eq!(
            release_decision(false, true, false, 0, true),
            ReleaseDecision::NoteArmRelease
        );
        assert_eq!(release_decision(false, false, false, 0, false), ReleaseDecision::Ignore);
    }

    #[test]
    fn short_press_hint_is_not_the_silence_error() {
        let short = silence_hint(180, true);
        assert!(short.contains("double-tap"));
        assert!(!short.contains("Didn't catch"));
        let quiet = silence_hint(2_000, false);
        assert!(quiet.contains("louder"));
        assert_eq!(recording_caption(true, &None).as_deref(), Some("Hands-free — tap the hotkey to stop"));
    }

    #[test]
    fn stage_line_quotes_why_cleanup_did_not_apply() {
        let line = stage_line(
            &Staged {
                held_ms: 2097,
                open_wait_ms: 0,
                stop_ms: 0,
                encode_ms: 1,
                bytes: 70124,
                encode_fmt: "flac".into(),
                groq_ms: 203,
                groq_tries: 1,
                cleanup_ms: 56,
                cleanup_why: "couldn't reach Groq: timed out".into(),
                paste_ms: 39,
                tone: "casual".into(),
                release_started: Instant::now(),
                engine: "Groq".into(),
                overlay_ms: 4,
                first_sample_ms: Some(11),
                speech_language: "ur".into(),
                speech_path: "mixed-llama".into(),
            },
            304,
        );
        assert!(line.contains("fmt=flac"));
        assert!(line.contains("tries=1"));
        assert!(line.contains("cleanup=56"));
        assert!(line.contains("cleanup_why=\"couldn't reach Groq: timed out\""));
        assert!(line.contains("tone=casual"));
        assert!(line.contains("engine=Groq"));
        assert!(line.contains("keydown_overlay_ms=4"));
        assert!(line.contains("keydown_first_sample_ms=11"));
        assert!(line.contains("language=ur"));
        assert!(line.contains("path=mixed-llama"));
        assert!(!line.contains("cleanup_why=\"\""));
    }

    #[test]
    fn keydown_shows_listening_immediately() {
        let event = instant_listen_event(0.4);
        assert_eq!(event.phase, "recording");
        assert!(event.message.is_none());
        assert_ne!(event.phase, "arming");
        let started = Instant::now();
        let _ = instant_listen_event(0.0);
        let overlay_ms = started.elapsed().as_millis() as u64;
        assert!(overlay_ms < 50, "building the Listening event took {overlay_ms} ms");
    }

    #[test]
    fn first_press_starts_capture() {
        // Still holding when the mic opens: listen. This is the first press.
        assert_eq!(after_arm(false, 40), AfterArm::Listen);
        assert_eq!(after_arm(false, 2_000), AfterArm::Listen);
        // A key-up during arming must not cancel the take.
        assert_eq!(
            release_decision(false, false, false, 0, true),
            ReleaseDecision::NoteArmRelease
        );
        // Hold time is the age of the key-down, so a slow open is not a tap.
        assert_eq!(
            release_decision(false, true, true, 800, false),
            ReleaseDecision::EndNow
        );
        assert_eq!(after_arm(true, 800), AfterArm::EndTake);
        assert_eq!(after_arm(true, 80), AfterArm::WaitSecondTap);
        assert!(crate::chunk::recording_cap().is_none());
    }
}
