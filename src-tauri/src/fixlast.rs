//! Fix last: listen to a kept take again, or take the user's typed words.
//!
//! The newest take can be replaced where it was pasted (undo once, paste the
//! new text) for two minutes after a confirmed clipboard paste. Older takes,
//! or ones Bolo couldn't confirm, are fixed in History and copied instead.

use crate::cleanup;
use crate::credentials::SecretKey;
use crate::dictation::AppState;
use crate::insert;
use crate::logbuf::log_line;
use crate::memory::{self, Correction};
use crate::spool;
use crate::store;
use crate::transcribe;
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const REPLACE_WINDOW: Duration = Duration::from_secs(120);

pub struct LastTake {
    pub spool_id: String,
    pub history_id: Option<String>,
    pub text: String,
    pub target: Option<isize>,
    pub at: Instant,
    pub verified_paste: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MemoryView {
    pub words: Vec<String>,
    pub fixes: Vec<Correction>,
    pub style: Vec<String>,
    pub keep_learning: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FixTarget {
    /// History id, or empty for a take that is not in History.
    pub id: String,
    pub text: String,
    pub can_relisten: bool,
    pub is_last: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FixResult {
    pub text: String,
    pub message: String,
    /// What was learned from a typed fix, e.g. "tie more → Taimoor".
    pub learned: Vec<String>,
}

/// A fix the tray or overlay asked for before the settings window was ready.
static PENDING_FIX: Mutex<Option<String>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn view(state: &AppState) -> MemoryView {
    let settings = lock(&state.settings);
    let memory = memory::snapshot();
    MemoryView {
        words: settings.dictionary.clone(),
        fixes: memory.corrections,
        style: memory.style,
        keep_learning: settings.keep_learning,
    }
}

fn save_settings(state: &AppState, settings: &store::Settings) -> Result<(), String> {
    store::save_settings(&state.config_dir, settings)
}

#[tauri::command]
pub fn get_memory(state: tauri::State<'_, AppState>) -> MemoryView {
    view(&state)
}

#[tauri::command]
pub fn forget_word(state: tauri::State<'_, AppState>, word: String) -> Result<MemoryView, String> {
    {
        let mut settings = lock(&state.settings);
        memory::update(|m| memory::forget_word(m, &mut settings.dictionary, word.trim()));
        save_settings(&state, &settings)?;
    }
    Ok(view(&state))
}

#[tauri::command]
pub fn add_word(state: tauri::State<'_, AppState>, word: String) -> Result<MemoryView, String> {
    let word = word.trim().to_string();
    if word.is_empty() || word.chars().count() > 64 {
        return Err("Use a word or name up to 64 letters.".into());
    }
    {
        let mut settings = lock(&state.settings);
        if settings.dictionary.len() >= 200 {
            return Err("The words list is full (200). Forget a few first.".into());
        }
        if !settings.dictionary.iter().any(|w| w.eq_ignore_ascii_case(&word)) {
            settings.dictionary.push(word.clone());
        }
        memory::update(|m| m.blocked.retain(|w| !w.eq_ignore_ascii_case(&word)));
        save_settings(&state, &settings)?;
    }
    Ok(view(&state))
}

#[tauri::command]
pub fn forget_fix(state: tauri::State<'_, AppState>, from: String) -> MemoryView {
    memory::update(|m| m.corrections.retain(|c| c.from != from));
    view(&state)
}

#[tauri::command]
pub fn forget_style(state: tauri::State<'_, AppState>, index: usize) -> MemoryView {
    memory::update(|m| {
        if index < m.style.len() {
            m.style.remove(index);
        }
    });
    view(&state)
}

#[tauri::command]
pub fn set_keep_learning(state: tauri::State<'_, AppState>, on: bool) -> Result<MemoryView, String> {
    {
        let mut settings = lock(&state.settings);
        settings.keep_learning = on;
        save_settings(&state, &settings)?;
    }
    Ok(view(&state))
}

#[tauri::command]
pub fn forget_everything(state: tauri::State<'_, AppState>) -> Result<MemoryView, String> {
    {
        let mut settings = lock(&state.settings);
        let saved = settings.dictionary.clone();
        memory::update(|m| memory::forget_everything(m, &mut settings.dictionary));
        if let Err(err) = save_settings(&state, &settings) {
            settings.dictionary = saved;
            return Err(err);
        }
    }
    log_line(&state.config_dir, "info", "memory: forgot everything");
    Ok(view(&state))
}

/// Which take a fix is about. `None` or empty means the newest one.
fn resolve(state: &AppState, id: Option<&str>) -> Option<FixTarget> {
    let id = id.filter(|id| !id.is_empty());
    let last = lock(&state.last_take);
    let history = lock(&state.history);
    let wanted = id.map(str::to_string).or_else(|| {
        last.as_ref()
            .and_then(|l| l.history_id.clone())
            .or_else(|| if last.is_none() { history.first().map(|e| e.id.clone()) } else { None })
    });
    let is_last_id = |hid: &str| last.as_ref().and_then(|l| l.history_id.as_deref()) == Some(hid);
    if let Some(hid) = wanted {
        let entry = history.iter().find(|e| e.id == hid)?;
        let audio_id = entry.audio_id.clone().unwrap_or_default();
        return Some(FixTarget {
            id: entry.id.clone(),
            text: entry.text.clone(),
            can_relisten: !spool::audio(&state.config_dir, &audio_id).is_empty(),
            is_last: is_last_id(&hid),
        });
    }
    // History off: only the newest take, from memory.
    let last = last.as_ref()?;
    Some(FixTarget {
        id: String::new(),
        text: last.text.clone(),
        can_relisten: !spool::audio(&state.config_dir, &last.spool_id).is_empty(),
        is_last: true,
    })
}

fn audio_for(state: &AppState, target: &FixTarget) -> Vec<Vec<u8>> {
    let audio_id = if target.id.is_empty() {
        lock(&state.last_take).as_ref().map(|l| l.spool_id.clone()).unwrap_or_default()
    } else {
        lock(&state.history)
            .iter()
            .find(|e| e.id == target.id)
            .and_then(|e| e.audio_id.clone())
            .unwrap_or_default()
    };
    spool::audio(&state.config_dir, &audio_id)
}

#[tauri::command]
pub fn fix_target(state: tauri::State<'_, AppState>, id: Option<String>) -> Result<FixTarget, String> {
    resolve(&state, id.as_deref()).ok_or_else(|| "There's no take to fix yet.".into())
}

/// The tray or overlay asked for a fix: open the settings window on it.
pub fn open_fix(app: &AppHandle, id: Option<String>) {
    *lock(&PENDING_FIX) = Some(id.clone().unwrap_or_default());
    crate::show_settings(app);
    let _ = app.emit("bolo-open-fix", id.unwrap_or_default());
}

#[tauri::command]
pub fn take_fix_request() -> Option<String> {
    lock(&PENDING_FIX).take()
}

#[tauri::command]
pub fn fix_requested() {
    *lock(&PENDING_FIX) = None;
}

/// Overlay buttons: "relisten" or "type".
#[tauri::command]
pub async fn overlay_fix(app: AppHandle, action: String) -> Result<(), String> {
    match action.as_str() {
        "type" => {
            crate::dictation::show_phase(&app, "idle", None);
            open_fix(&app, None);
            Ok(())
        }
        "relisten" => {
            crate::dictation::show_phase(&app, "transcribing", Some("Listening again…".into()));
            match relisten(&app, None).await {
                Ok(result) => {
                    crate::dictation::show_phase(&app, "done", Some(result.message));
                    crate::dictation::hide_later(&app, 2_200);
                    Ok(())
                }
                Err(err) => {
                    crate::dictation::show_phase(&app, "error", Some(err.clone()));
                    crate::dictation::hide_later(&app, 4_000);
                    Err(err)
                }
            }
        }
        _ => {
            crate::dictation::show_phase(&app, "idle", None);
            Ok(())
        }
    }
}

#[tauri::command]
pub async fn fix_relisten(app: AppHandle, id: Option<String>) -> Result<FixResult, String> {
    relisten(&app, id).await
}

async fn relisten(app: &AppHandle, id: Option<String>) -> Result<FixResult, String> {
    let state = app.state::<AppState>();
    let target = resolve(&state, id.as_deref()).ok_or("There's no take to fix yet.")?;
    let audio = audio_for(&state, &target);
    if audio.is_empty() {
        return Err("Bolo no longer has the sound for this one. Type the right words instead.".into());
    }
    let settings = lock(&state.settings).clone();
    let key = SecretKey::load().map_err(|_| "Listening again needs the internet. Type the right words instead.".to_string())?;
    let using_builtin = key.is_builtin();
    let words = crate::pipeline::prompt_words(&settings.dictionary);
    // No recent takes in the prompt: a wrong take must not steer its own redo.
    let prompt = transcribe::whisper_prompt_with(&words, &[]);
    let mut parts = Vec::new();
    for chunk in audio {
        let got = crate::pipeline::speech_with_groq(
            transcribe::http_client(),
            key.expose(),
            chunk,
            transcribe::WHISPER_LARGE,
            &settings.language,
            &settings.output,
            prompt.as_deref(),
            &settings.cleanup_model,
            using_builtin,
        )
        .await
        .map_err(|err| format!("Couldn't listen again ({err}). Type the right words instead."))?;
        if !got.text.trim().is_empty() {
            parts.push(got.text.trim().to_string());
        }
    }
    let _ = crate::diag::take_translated();
    let raw = parts.join(" ");
    if raw.trim().is_empty() {
        drop(key);
        return Err("Bolo heard nothing on the second listen. Type the right words instead.".into());
    }
    let cleaned = cleanup::clean_transcript(
        transcribe::http_client(),
        key.expose(),
        &raw,
        settings.cleanup_enabled && !settings.verbatim,
        if settings.verbatim { cleanup::Tone::Verbatim } else { cleanup::Tone::Plain },
        settings.verbatim,
        &settings.cleanup_model,
        using_builtin,
    )
    .await;
    drop(key);
    let text = memory::apply_corrections(cleaned.text.trim(), &memory::snapshot().corrections);
    log_line(&state.config_dir, "info", &format!("fix relisten chars={} same={}", text.chars().count(), text == target.text));
    if text.trim() == target.text.trim() {
        return Ok(FixResult {
            text,
            message: "Listened again and got the same words. Type the right words instead.".into(),
            learned: Vec::new(),
        });
    }
    let message = place(app, &target, &text).await;
    Ok(FixResult { text, message, learned: Vec::new() })
}

#[tauri::command]
pub async fn fix_with_text(app: AppHandle, id: Option<String>, text: String) -> Result<FixResult, String> {
    let state = app.state::<AppState>();
    let target = resolve(&state, id.as_deref()).ok_or("There's no take to fix yet.")?;
    let text = text.trim().to_string();
    if !text.chars().any(char::is_alphanumeric) {
        return Err("Type the words you meant first.".into());
    }
    let mut learned = Vec::new();
    if lock(&state.settings).keep_learning {
        let mut settings = lock(&state.settings);
        let pair = memory::diff_fix(&target.text, &text);
        let added = memory::update(|m| {
            if let Some((from, to)) = &pair {
                memory::add_correction(m, from, to, store::now_ms());
            }
            let source = pair.as_ref().map(|(_, to)| to.clone()).unwrap_or_default();
            memory::learn_from_fix(m, &source, &mut settings.dictionary)
        });
        if !added.is_empty() {
            if let Err(err) = store::save_settings(&state.config_dir, &settings) {
                log_line(&state.config_dir, "error", &format!("dictionary save: {err}"));
            }
        }
        if let Some((from, to)) = pair {
            learned.push(format!("{from} → {to}"));
        }
        for word in added {
            log_line(&state.config_dir, "info", &format!("dictionary +{word} (fix)"));
        }
    }
    let message = place(&app, &target, &text).await;
    Ok(FixResult { text, message, learned })
}

/// Save the fixed text and put it where the user needs it.
async fn place(app: &AppHandle, target: &FixTarget, text: &str) -> String {
    let state = app.state::<AppState>();
    if !target.id.is_empty() {
        let mut history = lock(&state.history);
        if let Some(entry) = history.iter_mut().find(|e| e.id == target.id) {
            entry.text = text.to_string();
        }
        if let Err(err) = store::save_history(&state.config_dir, &history) {
            log_line(&state.config_dir, "error", &err);
        }
    }
    let _ = app.emit("bolo-history", ());
    let replace = if target.is_last {
        let mut last = lock(&state.last_take);
        let plan = last.as_ref().and_then(|l| l.target.map(|hwnd| (hwnd, l.verified_paste && l.at.elapsed() < REPLACE_WINDOW)));
        if let Some(l) = last.as_mut() {
            l.text = text.to_string();
            l.at = Instant::now();
        }
        plan
    } else {
        None
    };
    if let Some((hwnd, undo)) = replace {
        let owned = text.to_string();
        let done = tauri::async_runtime::spawn_blocking(move || insert::replace_in(hwnd, &owned, undo)).await;
        match done {
            Ok(Ok(_)) if undo => return "Fixed. The new words replaced the old ones.".into(),
            Ok(Ok(_)) => {
                return "Pasted the fixed words. If the old words are still there, delete them.".into()
            }
            Ok(Err(err)) => log_line(&state.config_dir, "info", &format!("fix paste skipped: {err}")),
            Err(err) => log_line(&state.config_dir, "info", &format!("fix paste skipped: {err}")),
        }
    }
    match insert::set_clipboard_text(text) {
        Ok(()) => "Fixed and copied. Paste it with Ctrl+V where you need it.".into(),
        Err(_) => "Fixed in History.".into(),
    }
}
