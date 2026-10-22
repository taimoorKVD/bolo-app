//! Recording, transcription, and insertion state machine.
//!
//! Presses that arrive while a transcript is still in flight are ignored so a
//! second take cannot overlap the paste.

use crate::audio::{self, RecordingSession};
use crate::cleanup;
use crate::credentials::SecretKey;
use crate::insert::{self, InsertMethod};
use crate::logbuf::log_line;
use crate::store::{self, HistoryEntry, Settings};
use crate::transcribe;
use crate::listener::ListenerControl;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

const MAX_RECORDING: Duration = Duration::from_secs(120);

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
    session: Option<RecordingSession>,
    generation: u64,
    /// The hotkey was released before the microphone finished opening.
    stop_requested: bool,
}

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub history: Mutex<Vec<HistoryEntry>>,
    engine: Mutex<Engine>,
    pub listener: ListenerControl,
    pub config_dir: std::path::PathBuf,
}

impl AppState {
    pub fn new(settings: Settings, history: Vec<HistoryEntry>, config_dir: std::path::PathBuf) -> Self {
        let listener = ListenerControl::new(settings.hotkey.clone());
        Self {
            settings: Mutex::new(settings),
            history: Mutex::new(history),
            engine: Mutex::new(Engine {
                phase: Phase::Idle,
                session: None,
                generation: 0,
                stop_requested: false,
            }),
            listener,
            config_dir,
        }
    }
}

struct Outcome {
    text: String,
    cleaned: bool,
    note: Option<String>,
    duration_ms: u64,
    model: String,
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
        engine.generation
    };
    let settings = lock(&state.settings).clone();
    let session = match audio::start_recording(settings.microphone.as_deref()) {
        Ok(session) => session,
        Err(err) => {
            mark_idle(app);
            fail(app, &err.to_string());
            return;
        }
    };
    let audio = Arc::clone(&session.audio);
    let stop = Arc::clone(&session.stop);
    let started = session.started;
    let note = session.device_note.clone();
    {
        let mut engine = lock(&state.engine);
        let cancelled = engine.stop_requested
            || engine.generation != generation
            || !matches!(engine.phase, Phase::Recording);
        if cancelled {
            engine.phase = Phase::Idle;
            engine.session = None;
            engine.stop_requested = false;
            return;
        }
        engine.session = Some(session);
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
    spawn_meter(app.clone(), audio, stop, started, note, generation);
}

pub fn end(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (session, generation) = {
        let mut engine = lock(&state.engine);
        if !matches!(engine.phase, Phase::Recording) {
            return;
        }
        if engine.session.is_none() {
            engine.stop_requested = true;
            return;
        }
        engine.phase = Phase::Busy;
        (engine.session.take(), engine.generation)
    };
    let Some(session) = session else {
        mark_idle(app);
        return;
    };
    let finished = match session.finish() {
        Ok(finished) => finished,
        Err(err) => {
            mark_idle(app);
            fail(app, &err);
            return;
        }
    };
    let settings = lock(&state.settings).clone();
    publish(
        app,
        &DictationEvent {
            phase: "transcribing".into(),
            message: None,
            elapsed_ms: finished.elapsed_ms,
            level: 0.0,
        },
    );
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match process(finished, settings).await {
            Ok(outcome) => deliver(&app, outcome, generation).await,
            Err(err) => {
                mark_idle(&app);
                fail(&app, &err);
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
    audio: Arc<Mutex<audio::SharedAudio>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    started: std::time::Instant,
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
            let level = audio
                .lock()
                .map(|buffer| (buffer.last_rms * 4.5).clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let _ = app.emit(
                "dictation-state",
                DictationEvent {
                    phase: "recording".into(),
                    message: note.clone(),
                    elapsed_ms: elapsed.as_millis() as u64,
                    level,
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

async fn process(finished: audio::FinishedAudio, settings: Settings) -> Result<Outcome, String> {
    let duration_ms = finished.elapsed_ms;
    let wav = audio::prepare_speech(&finished).map_err(|err| err.to_string())?;
    if wav.len() > 20 * 1024 * 1024 {
        return Err("The recording is too large to upload.".into());
    }
    let key = SecretKey::load().map_err(|err| err.to_string())?;
    let prompt = transcribe::whisper_prompt(&settings.dictionary);
    let model = settings.model.clone();
    let transcript = transcribe::transcribe(
        transcribe::http_client(),
        key.expose(),
        wav,
        &model,
        &settings.language,
        prompt,
    )
    .await
    .map_err(|err| err.to_string())?;
    let cleaned = cleanup::clean_transcript(
        transcribe::http_client(),
        key.expose(),
        &transcript,
        settings.cleanup_enabled,
    )
    .await;
    drop(key);
    let text = cleaned.text.trim().to_string();
    if text.is_empty() {
        return Err("Nothing to insert.".into());
    }
    Ok(Outcome {
        text,
        cleaned: cleaned.used_model,
        note: cleaned.note,
        duration_ms,
        model,
    })
}

async fn deliver(app: &AppHandle, outcome: Outcome, generation: u64) {
    let text = outcome.text.clone();
    let inserted = tauri::async_runtime::spawn_blocking(move || insert::insert_text(&text)).await;
    let method = match inserted {
        Ok(Ok(method)) => Some(method),
        Ok(Err(err)) => {
            remember(app, &outcome);
            mark_idle(app);
            fail(
                app,
                &format!("Couldn't insert the text ({err}). It's saved in History."),
            );
            return;
        }
        Err(err) => {
            remember(app, &outcome);
            mark_idle(app);
            fail(app, &format!("Couldn't insert the text ({err}). It's saved in History."));
            return;
        }
    };
    remember(app, &outcome);
    let state = app.state::<AppState>();
    log_line(
        &state.config_dir,
        "info",
        &format!(
            "inserted chars={} ms={} cleaned={} via={}",
            outcome.text.chars().count(),
            outcome.duration_ms,
            outcome.cleaned,
            match method {
                Some(InsertMethod::Clipboard) => "clipboard",
                Some(InsertMethod::Typed) => "typed",
                None => "unknown",
            }
        ),
    );
    mark_idle(app);
    let message = outcome.note.unwrap_or_else(|| "Inserted".into());
    publish(
        app,
        &DictationEvent {
            phase: "done".into(),
            message: Some(message),
            elapsed_ms: outcome.duration_ms,
            level: 0.0,
        },
    );
    schedule_hide(app.clone(), generation);
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
    schedule_hide(app.clone(), generation);
}

fn schedule_hide(app: AppHandle, generation: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1600));
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
    engine.session = None;
    engine.stop_requested = false;
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
