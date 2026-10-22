//! Recording, transcription, and insertion state machine.
//!
//! Presses that arrive while a transcript is still in flight are ignored so a
//! second take cannot overlap the paste. A release shorter than a tap waits
//! briefly for a second press, which latches hands-free mode. A normal hold
//! stops on release with no extra wait.

use crate::audio::{self, SpeechError};
use crate::cleanup;
use crate::context;
use crate::credentials::SecretKey;
use crate::hotkey::{display, HotkeyEdge};
use crate::insert::{self, InsertMethod};
use crate::listener::ListenerControl;
use crate::logbuf::log_line;
use crate::store::{self, HistoryEntry, Settings};
use crate::transcribe;
use crate::warm::{self, LiveTake};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

const MAX_RECORDING: Duration = Duration::from_secs(120);
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
}

enum Phase {
    Idle,
    Recording,
    Busy,
}

struct Engine {
    phase: Phase,
    take: Option<LiveTake>,
    generation: u64,
    /// The hotkey went up before the microphone had armed this take.
    stop_requested: bool,
    hands_free: bool,
    awaiting_second: bool,
    /// The release that ended (or will end) this take was a short tap.
    short_release: bool,
    /// Hold length at the moment of release, before the double-tap wait.
    release_held_ms: u64,
    /// Key-up that ends the take. Set before the double-tap wait so release
    /// time includes that wait.
    release_mark: Option<Instant>,
}

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub history: Mutex<Vec<HistoryEntry>>,
    engine: Mutex<Engine>,
    pub listener: ListenerControl,
    pub config_dir: std::path::PathBuf,
    pub last_error: Mutex<Option<String>>,
    pub warm: warm::WarmMic,
}

impl AppState {
    pub fn new(settings: Settings, history: Vec<HistoryEntry>, config_dir: std::path::PathBuf) -> Self {
        let listener = ListenerControl::new(settings.hotkey.clone());
        let warm = warm::WarmMic::spawn(settings.microphone.clone());
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
            }),
            listener,
            config_dir,
            last_error: Mutex::new(None),
            warm,
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
}

struct Staged {
    held_ms: u64,
    open_wait_ms: u64,
    stop_ms: u64,
    encode_ms: u64,
    bytes: usize,
    groq_ms: u64,
    cleanup_ms: u64,
    /// Empty until cleanup runs. Then "applied", "off", "verbatim", or the fallback reason.
    cleanup_why: String,
    paste_ms: u64,
    tone: String,
    release_started: Instant,
}

struct Outcome {
    text: String,
    cleaned: bool,
    note: Option<String>,
    duration_ms: u64,
    model: String,
    staged: Staged,
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
    CancelOpen,
    Ignore,
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
) -> ReleaseDecision {
    if hands_free || !recording {
        return ReleaseDecision::Ignore;
    }
    if !take_ready {
        return ReleaseDecision::CancelOpen;
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

fn recording_caption(hands_free: bool, note: &Option<String>) -> Option<String> {
    if !hands_free {
        return note.clone();
    }
    Some(match note {
        Some(note) if !note.is_empty() => format!("Hands-free — tap the hotkey to stop · {note}"),
        _ => "Hands-free — tap the hotkey to stop".into(),
    })
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
                },
            );
        }
        PressDecision::Begin => begin(app),
        PressDecision::Ignore => {}
    }
}

fn on_release(app: &AppHandle) {
    let state = app.state::<AppState>();
    let action = {
        let mut engine = lock(&state.engine);
        let held_ms = engine
            .take
            .as_ref()
            .map(|take| take.started.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let decision = release_decision(
            engine.hands_free,
            matches!(engine.phase, Phase::Recording),
            engine.take.is_some(),
            held_ms,
        );
        match decision {
            ReleaseDecision::ArmDoubleTap => {
                engine.awaiting_second = true;
                engine.short_release = true;
                engine.release_held_ms = held_ms;
                engine.release_mark = Some(Instant::now());
                Some((decision, engine.generation))
            }
            ReleaseDecision::CancelOpen => {
                engine.stop_requested = true;
                engine.short_release = true;
                engine.release_held_ms = held_ms;
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
    let generation = {
        let mut engine = lock(&state.engine);
        if !matches!(engine.phase, Phase::Idle) {
            return;
        }
        engine.phase = Phase::Recording;
        engine.generation = engine.generation.wrapping_add(1);
        engine.stop_requested = false;
        engine.hands_free = false;
        engine.awaiting_second = false;
        engine.short_release = false;
        engine.release_held_ms = 0;
        engine.release_mark = None;
        engine.generation
    };
    *lock(&state.last_error) = None;
    let settings = lock(&state.settings).clone();
    if let Some(tray) = app.tray_by_id("cadence") {
        let _ = tray.set_tooltip(Some(format!("Cadence — hold {}", display(&settings.hotkey))));
    }
    let take = match state.warm.begin(settings.microphone.as_deref()) {
        Ok(take) => take,
        Err(err) => {
            mark_idle(app);
            fail(app, &err);
            return;
        }
    };
    if take.used_fallback {
        if let Some(message) = &take.device_note {
            log_line(&state.config_dir, "info", message);
        }
    }
    let (level, stop_flag) = take.meter();
    let started = take.started;
    let note = take.device_note.clone();
    {
        let mut engine = lock(&state.engine);
        let cancelled = engine.stop_requested
            || engine.generation != generation
            || !matches!(engine.phase, Phase::Recording);
        if cancelled {
            engine.phase = Phase::Idle;
            engine.take = None;
            engine.stop_requested = false;
            drop(engine);
            let _ = take.finish();
            return;
        }
        engine.take = Some(take);
    }
    publish(
        app,
        &DictationEvent {
            phase: "recording".into(),
            message: note.clone(),
            elapsed_ms: 0,
            level: 0.0,
        },
    );
    spawn_meter(app.clone(), level, stop_flag, started, note, generation);
}

pub fn end(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (take, generation, short_press, hint_held_ms, release_started) = {
        let mut engine = lock(&state.engine);
        engine.awaiting_second = false;
        if !matches!(engine.phase, Phase::Recording) {
            return;
        }
        if engine.take.is_none() {
            engine.stop_requested = true;
            return;
        }
        let release_started = engine.release_mark.take().unwrap_or_else(Instant::now);
        let short_press = engine.short_release;
        let hint_held_ms = if engine.release_held_ms > 0 {
            engine.release_held_ms
        } else {
            engine
                .take
                .as_ref()
                .map(|take| take.started.elapsed().as_millis() as u64)
                .unwrap_or(0)
        };
        engine.short_release = false;
        engine.hands_free = false;
        engine.release_held_ms = 0;
        engine.phase = Phase::Busy;
        (engine.take.take(), engine.generation, short_press, hint_held_ms, release_started)
    };
    let Some(take) = take else {
        mark_idle(app);
        return;
    };
    let open_wait_ms = take.open_wait_ms;
    let held_ms = take.started.elapsed().as_millis() as u64;
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
    let settings = lock(&state.settings).clone();
    let clock = Clock {
        release_started,
        held_ms,
        open_wait_ms,
        stop_ms,
        short_press,
        hint_held_ms,
    };
    publish(
        app,
        &DictationEvent {
            phase: "transcribing".into(),
            message: None,
            elapsed_ms: held_ms,
            level: 0.0,
        },
    );
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match process(finished, settings, clock).await {
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

pub fn toggle(app: &AppHandle) {
    let state = app.state::<AppState>();
    let recording = matches!(lock(&state.engine).phase, Phase::Recording);
    if recording {
        end(app);
    } else {
        begin(app);
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
            let elapsed = started.elapsed();
            if elapsed >= MAX_RECORDING {
                end_if_generation(&app, generation);
                break;
            }
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
                },
            );
            std::thread::sleep(Duration::from_millis(80));
        }
    });
}

fn end_if_generation(app: &AppHandle, generation: u64) {
    let state = app.state::<AppState>();
    let should_stop = {
        let engine = lock(&state.engine);
        matches!(engine.phase, Phase::Recording) && engine.generation == generation
    };
    if should_stop {
        end(app);
    }
}

async fn process(finished: audio::FinishedAudio, settings: Settings, clock: Clock) -> Result<Outcome, TakeError> {
    let mut staged = Staged {
        held_ms: clock.held_ms,
        open_wait_ms: clock.open_wait_ms,
        stop_ms: clock.stop_ms,
        encode_ms: 0,
        bytes: 0,
        groq_ms: 0,
        cleanup_ms: 0,
        cleanup_why: String::new(),
        paste_ms: 0,
        tone: "none".into(),
        release_started: clock.release_started,
    };
    let encode_started = Instant::now();
    let wav = match audio::prepare_speech(&finished) {
        Ok(wav) => wav,
        Err(err) => {
            staged.encode_ms = encode_started.elapsed().as_millis() as u64;
            if matches!(err, SpeechError::Silent | SpeechError::Empty) {
                return Err(TakeError {
                    message: silence_hint(clock.hint_held_ms, clock.short_press).into(),
                    hint: true,
                    staged,
                });
            }
            return Err(TakeError {
                message: err.to_string(),
                hint: false,
                staged,
            });
        }
    };
    staged.encode_ms = encode_started.elapsed().as_millis() as u64;
    staged.bytes = wav.len();
    if wav.len() > 20 * 1024 * 1024 {
        return Err(TakeError {
            message: "The recording is too large to upload.".into(),
            hint: false,
            staged,
        });
    }
    let tone = context::foreground_tone();
    staged.tone = tone.as_str().into();
    let key = match SecretKey::load() {
        Ok(key) => key,
        Err(err) => {
            return Err(TakeError {
                message: err.to_string(),
                hint: false,
                staged,
            });
        }
    };
    let prompt = transcribe::whisper_prompt(&settings.dictionary);
    let model = settings.model.clone();
    let groq_started = Instant::now();
    let transcript = match transcribe::transcribe(
        transcribe::http_client(),
        key.expose(),
        wav,
        &model,
        &settings.language,
        prompt,
    )
    .await
    {
        Ok(text) => text,
        Err(err) => {
            staged.groq_ms = groq_started.elapsed().as_millis() as u64;
            return Err(TakeError {
                message: err.to_string(),
                hint: false,
                staged,
            });
        }
    };
    staged.groq_ms = groq_started.elapsed().as_millis() as u64;
    let cleanup_started = Instant::now();
    let cleaned = cleanup::clean_transcript(
        transcribe::http_client(),
        key.expose(),
        &transcript,
        settings.cleanup_enabled,
        tone,
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
    })
}

async fn deliver(app: &AppHandle, mut outcome: Outcome, generation: u64) {
    absorb_dictionary(app, &outcome.text);
    let text = outcome.text.clone();
    let paste_started = Instant::now();
    let inserted = tauri::async_runtime::spawn_blocking(move || insert::insert_text(&text)).await;
    outcome.staged.paste_ms = paste_started.elapsed().as_millis() as u64;
    let release_ms = outcome.staged.release_started.elapsed().as_millis() as u64;
    let method = match inserted {
        Ok(Ok(method)) => Some(method),
        Ok(Err(err)) => {
            log_stages(app, &outcome.staged, release_ms);
            remember(app, &outcome);
            mark_idle(app);
            fail(
                app,
                &format!("Couldn't insert the text ({err}). It's saved in History."),
            );
            return;
        }
        Err(err) => {
            log_stages(app, &outcome.staged, release_ms);
            remember(app, &outcome);
            mark_idle(app);
            fail(app, &format!("Couldn't insert the text ({err}). It's saved in History."));
            return;
        }
    };
    log_stages(app, &outcome.staged, release_ms);
    let state = app.state::<AppState>();
    log_line(
        &state.config_dir,
        "info",
        &format!(
            "inserted chars={} cleaned={} via={}",
            outcome.text.chars().count(),
            outcome.cleaned,
            match method {
                Some(InsertMethod::Clipboard) => "clipboard",
                Some(InsertMethod::Typed) => "typed",
                None => "unknown",
            }
        ),
    );
    remember(app, &outcome);
    mark_idle(app);
    let message = outcome.note.clone().unwrap_or_else(|| "Inserted".into());
    publish(
        app,
        &DictationEvent {
            phase: "done".into(),
            message: Some(message),
            elapsed_ms: outcome.duration_ms,
            level: 0.0,
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
    format!(
        "stages held={} open_wait={} stop={} encode={} bytes={} groq={} cleanup={}{why} paste={} release={} tone={}",
        staged.held_ms,
        staged.open_wait_ms,
        staged.stop_ms,
        staged.encode_ms,
        staged.bytes,
        staged.groq_ms,
        staged.cleanup_ms,
        staged.paste_ms,
        release_ms,
        staged.tone
    )
}

fn quote_log(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "'"))
}

fn remember(app: &AppHandle, outcome: &Outcome) {
    let state = app.state::<AppState>();
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
    if let Some(tray) = app.tray_by_id("cadence") {
        let _ = tray.set_tooltip(Some(format!("Cadence — {message}")));
    }
    show_problem(app, message, 8_000);
}

fn hint(app: &AppHandle, message: &str) {
    let state = app.state::<AppState>();
    log_line(&state.config_dir, "info", &format!("hint {message}"));
    show_problem(app, message, 3_500);
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
        (460.0, 210.0)
    } else {
        (440.0, 128.0)
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
            release_decision(false, true, true, TAP_MS),
            ReleaseDecision::EndNow
        );
        assert_eq!(
            release_decision(false, true, true, TAP_MS - 1),
            ReleaseDecision::ArmDoubleTap
        );
        assert_eq!(
            press_decision(false, true, false),
            PressDecision::LatchHandsFree
        );
        assert_eq!(release_decision(true, true, true, 20), ReleaseDecision::Ignore);
        assert_eq!(press_decision(true, false, false), PressDecision::StopHandsFree);
        assert_eq!(press_decision(false, false, false), PressDecision::Ignore);
        assert_eq!(
            release_decision(false, true, false, 0),
            ReleaseDecision::CancelOpen
        );
        assert_eq!(release_decision(false, false, false, 0), ReleaseDecision::Ignore);
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
                groq_ms: 203,
                cleanup_ms: 56,
                cleanup_why: "couldn't reach Groq: timed out".into(),
                paste_ms: 39,
                tone: "casual".into(),
                release_started: Instant::now(),
            },
            304,
        );
        assert!(line.contains("cleanup=56"));
        assert!(line.contains("cleanup_why=\"couldn't reach Groq: timed out\""));
        assert!(line.contains("tone=casual"));
        assert!(!line.contains("cleanup_why=\"\""));
    }
}
