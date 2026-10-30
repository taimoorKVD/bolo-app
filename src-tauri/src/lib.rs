mod audio;
mod builtin;
mod chunk;
mod cleanup;
mod context;
mod credentials;
mod dictation;
mod health;
mod hotkey;
mod insert;
mod levels;
mod listener;
mod logbuf;
mod micwatch;
mod offline;
mod pipeline;
mod spool;
#[cfg(windows)]
mod startup;
mod store;
mod transcribe;
mod updater;
mod warm;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod harness;

use crate::credentials::ApiKeyStatus;
use crate::dictation::AppState;
use crate::hotkey::display;
use crate::listener::InputEvent;
use crate::store::{prepare_settings, HistoryEntry, Settings};
use serde::Serialize;
use std::sync::mpsc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_autostart::ManagerExt;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut builder = tauri::Builder::default();
    #[cfg(windows)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_settings(&app);
        }));
    }
    builder = updater::init(builder);
    builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            api_key_status,
            set_api_key,
            clear_api_key,
            list_microphones,
            get_history,
            delete_history_entry,
            clear_history,
            set_hotkey_capture,
            toggle_test_recording,
            get_app_info,
            dismiss_error,
            get_mic_status,
            mic_level,
            audio_health,
            fix_mic_level,
            fix_output_volume,
            copy_text,
            open_groq_keys,
            smoke_finished,
            updater::check_for_update,
            updater::pending_update,
            updater::install_update,
            updater::updates_enabled
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "settings" && window.app_handle().tray_by_id("bolo").is_some()
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            // Windows are created here, not from tauri.conf.json (`create: false`).
            // Tauri otherwise builds them before this hook, and a release WebView2
            // loads the bundled page and invokes get_settings before manage().
            // A setup error then aborts the process with state still missing, so
            // this hook always returns Ok once state is registered.
            let boot = std::time::Instant::now();
            let smoke = std::env::var("BOLO_SMOKE").ok().as_deref() == Some("1");
            let dir = config_dir(app);
            if let Err(err) = std::fs::create_dir_all(&dir) {
                eprintln!("config directory: {err}");
            }
            let migrated = store::migrate_legacy_config(&dir);
            logbuf::init(&dir);
            if migrated {
                logbuf::log_line(&dir, "info", "copied settings from com.cadence.dictation");
            }
            let settings = store::load_settings(&dir);
            let history = store::load_history(&dir);
            let state = AppState::new(settings.clone(), history, dir.clone());
            let listener = state.listener.clone();
            if !app.manage(state) {
                logbuf::log_line(&dir, "error", "app state was already registered");
            }
            if app.try_state::<AppState>().is_none() {
                logbuf::log_line(&dir, "error", "app state is not visible after manage()");
            }

            let emit_app = app.handle().clone();
            listener.set_on_status(move |status| {
                let _ = emit_app.emit("listener-status", &status);
            });
            let timing_dir = dir.clone();
            listener::set_timing_sink(move |name, ms| {
                logbuf::log_line(&timing_dir, "info", &format!("startup {name}={ms}"));
            });
            let (tx, rx) = mpsc::channel::<InputEvent>();
            // Install the keyboard hook first so the hotkey works even while
            // WebView2 and the tray are still starting.
            listener::spawn(listener, tx);
            logbuf::log_line(&dir, "info", &format!("startup listener_spawned_ms={}", boot.elapsed().as_millis()));

            // WebView2 can take 10+ s on the first launch after install. Building
            // the overlay from a worker thread lets setup return at once; the
            // settings window is created on first open (show_settings).
            if smoke {
                if let Err(err) = create_windows(app, smoke) {
                    logbuf::log_line(&dir, "error", &format!("windows: {err}"));
                }
            } else {
                let win_app = app.handle().clone();
                let win_dir = dir.clone();
                std::thread::spawn(move || {
                    let t0 = std::time::Instant::now();
                    match build_window(&win_app, "overlay") {
                        Ok(overlay) => {
                            logbuf::log_line(&win_dir, "info", &format!("startup overlay_built_ms={} since_boot_ms={}", t0.elapsed().as_millis(), boot.elapsed().as_millis()));
                            let _ = overlay.set_ignore_cursor_events(true);
                            let _ = overlay.hide();
                            warm_overlay(win_app.clone(), win_dir.clone());
                        }
                        Err(err) => logbuf::log_line(&win_dir, "error", &format!("overlay window: {err}")),
                    }
                });
            }
            logbuf::log_line(&dir, "info", &format!("startup windows_created_ms={}", boot.elapsed().as_millis()));
            let handle = app.handle().clone();
            if let Err(err) = std::thread::Builder::new()
                .name("bolo-dictation".into())
                .spawn(move || {
                    while let Ok(event) = rx.recv() {
                        match event {
                            InputEvent::Dictation(edge) => dictation::hotkey(&handle, edge),
                            InputEvent::PasteAgain => dictation::paste_last(&handle),
                        }
                    }
                })
            {
                logbuf::log_line(&dir, "error", &format!("couldn't start dictation: {err}"));
            }
            let prewarm_dir = dir.clone();
            tauri::async_runtime::spawn(async move {
                transcribe::prewarm(&prewarm_dir).await;
            });

            if smoke {
                if let Some(overlay) = app.get_webview_window("overlay") {
                    let _ = overlay.set_ignore_cursor_events(true);
                    let _ = overlay.hide();
                }
            }

            let tray_ok = match setup_tray(app.handle(), &settings.hotkey) {
                Ok(()) => true,
                Err(err) => {
                    logbuf::log_line(&dir, "error", &format!("tray: {err}"));
                    false
                }
            };
            if let Err(err) = apply_autostart(app.handle(), settings.launch_at_startup) {
                logbuf::log_line(&dir, "error", &format!("startup setting: {err}"));
            }
            let key_status = credentials::status();
            let needs_key = builtin::onboarding_needs_key(key_status.configured, key_status.builtin);
            // Smoke must not focus a window onto a monitor. The settings page
            // is parked off-screen in create_windows so get_settings still runs.
            if !smoke && (needs_key || !tray_ok) {
                show_settings(app.handle());
            }
            logbuf::log_line(
                &dir,
                "info",
                &format!("Bolo ready, hold {}", display(&settings.hotkey)),
            );
            logbuf::log_line(&dir, "info", &format!("startup setup_ms={}", boot.elapsed().as_millis()));
            micwatch::spawn(app.handle().clone());
            health::spawn(app.handle().clone());
            updater::spawn(app.handle());
            #[cfg(windows)]
            {
                let startup_dir = dir.clone();
                std::thread::spawn(move || startup::clean_stale_autostart(&startup_dir));
            }
            dictation::recover_spool(app.handle());
            offline::prefetch(dir.clone(), settings.offline_model.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Bolo");
}

fn config_dir(app: &tauri::App) -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("BOLO_CONFIG_DIR") {
        let dir = std::path::PathBuf::from(dir);
        if !dir.as_os_str().is_empty() {
            return dir;
        }
    }
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("bolo"))
}

fn create_windows(app: &tauri::App, smoke: bool) -> Result<(), String> {
    let configs = app.config().app.windows.clone();
    for config in configs {
        if app.get_webview_window(&config.label).is_some() {
            continue;
        }
        let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)
            .map_err(|err| err.to_string())?;
        if smoke {
            // from_config applies `center: true` and does not clear it when a
            // position is set afterwards. The window is created hidden; it is
            // moved off-screen before anything shows it.
            builder = builder.visible(false).focused(false).skip_taskbar(true);
            if config.label == "settings" {
                builder = builder.initialization_script("window.__BOLO_SMOKE__ = true;");
            }
        }
        let window = builder.build().map_err(|err| err.to_string())?;
        if smoke {
            park_offscreen(&window);
            if config.label == "settings" {
                // WebView2 skips navigation on a window that is never shown.
                // Show without activating (focused is false), then park again
                // in case that show pulled the window onto a monitor.
                let _ = window.show();
                park_offscreen(&window);
            } else {
                let _ = window.hide();
            }
        }
    }
    Ok(())
}

/// WebView2 does not load a window that has never been shown, so the first
/// key-down after login used to wait for the overlay page to load. Show it
/// once, off-screen and without focus, so the page is loaded before any press.
fn warm_overlay(app: tauri::AppHandle, dir: std::path::PathBuf) {
    let started = std::time::Instant::now();
    let Some(overlay) = app.get_webview_window("overlay") else { return };
    park_offscreen(&overlay);
    let _ = overlay.show();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let idle = app
            .try_state::<AppState>()
            .map(|state| dictation::is_idle(&state))
            .unwrap_or(true);
        if let Some(overlay) = app.get_webview_window("overlay") {
            if idle {
                let _ = overlay.hide();
            }
        }
        logbuf::log_line(&dir, "info", &format!("startup overlay_warmed_ms={}", started.elapsed().as_millis()));
    });
}

fn park_offscreen<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    // Outside any real virtual-screen layout. Windows parks its own hidden
    // windows in this region.
    let _ = window.set_position(tauri::PhysicalPosition::new(-32_000, -32_000));
}

fn setup_tray(app: &tauri::AppHandle, hotkey: &hotkey::Hotkey) -> tauri::Result<()> {
    let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Bolo", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings_item, &quit_item])?;
    let mut builder = TrayIconBuilder::with_id("bolo")
        .menu(&menu)
        .tooltip(format!("Bolo — hold {}", display(hotkey)))
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_settings(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub(crate) fn show_settings(app: &tauri::AppHandle) {
    if std::env::var("BOLO_SMOKE").ok().as_deref() == Some("1") {
        return;
    }
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    // Created lazily. build() must not run on the main thread from an IPC
    // handler, so build from a worker thread; it dispatches to the main loop.
    let app = app.clone();
    std::thread::spawn(move || match build_window(&app, "settings") {
        Ok(window) => {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        Err(err) => eprintln!("settings window: {err}"),
    });
}

/// Build one window from tauri.conf.json, or return it if it already exists.
fn build_window(app: &tauri::AppHandle, label: &str) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(label) {
        return Ok(window);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == label)
        .cloned()
        .ok_or_else(|| format!("no window config for {label}"))?;
    tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|err| err.to_string())?
        .build()
        .map_err(|err| err.to_string())
}

fn update_tooltip(app: &tauri::AppHandle, hotkey: &hotkey::Hotkey) {
    if let Some(tray) = app.tray_by_id("bolo") {
        let _ = tray.set_tooltip(Some(format!("Bolo — hold {}", display(hotkey))));
    }
}

fn apply_autostart(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let launch = app.autolaunch();
    if enabled {
        launch.enable().map_err(|err| err.to_string())
    } else {
        launch.disable().map_err(|err| err.to_string())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: &'static str,
    listener_running: bool,
    listener_error: Option<String>,
    platform: &'static str,
    last_error: Option<String>,
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> Settings {
    state
        .settings
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    settings: Settings,
) -> Result<(), String> {
    let settings = prepare_settings(settings);
    store::validate_settings(&settings)?;
    let previous = state
        .settings
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    if previous.launch_at_startup != settings.launch_at_startup {
        apply_autostart(&app, settings.launch_at_startup)?;
    }
    if let Err(err) = store::save_settings(&state.config_dir, &settings) {
        if previous.launch_at_startup != settings.launch_at_startup {
            let _ = apply_autostart(&app, previous.launch_at_startup);
        }
        return Err(err);
    }
    state.listener.set_hotkey(settings.hotkey.clone());
    state.listener.set_paste_hotkey(settings.paste_hotkey.clone());
    state.warm.retarget(settings.microphone.clone());
    update_tooltip(&app, &settings.hotkey);
    *state
        .settings
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = settings;
    Ok(())
}

#[tauri::command]
fn api_key_status() -> ApiKeyStatus {
    credentials::status()
}

#[tauri::command]
fn set_api_key(state: tauri::State<'_, AppState>, mut key: String) -> Result<ApiKeyStatus, String> {
    let trimmed = key.trim().to_string();
    cleanup::wipe(&mut key);
    credentials::validate_key(&trimmed)?;
    let saved = credentials::set_api_key(&trimmed);
    let mut trimmed = trimmed;
    cleanup::wipe(&mut trimmed);
    saved.map_err(|err| err.to_string())?;
    let status = credentials::status();
    if !status.configured {
        return Err(status
            .error
            .unwrap_or_else(|| "The API key was not stored.".into()));
    }
    logbuf::log_line(&state.config_dir, "info", "Groq API key stored");
    Ok(status)
}

#[tauri::command]
fn clear_api_key() -> Result<(), String> {
    credentials::clear_api_key().map_err(|err| err.to_string())
}

#[tauri::command]
async fn list_microphones() -> Result<Vec<audio::MicInfo>, String> {
    // Device enumeration can take >1 s; keep it off the main thread so other
    // IPC calls (and window painting) are not queued behind it.
    tauri::async_runtime::spawn_blocking(audio::list_microphones)
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn audio_health(state: tauri::State<'_, AppState>) -> Result<levels::AudioHealth, String> {
    let name = state.warm.active_name();
    tauri::async_runtime::spawn_blocking(move || levels::health(Some(name.as_str()).filter(|n| !n.is_empty())))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn fix_mic_level(state: tauri::State<'_, AppState>) -> Result<levels::AudioHealth, String> {
    let name = state.warm.active_name();
    let warm_name = name.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let name = Some(warm_name.as_str()).filter(|n| !n.is_empty());
        levels::fix_mic(name)?;
        Ok::<_, String>(levels::health(name))
    })
    .await
    .map_err(|e| e.to_string())?;
    if !name.is_empty() {
        audio::clear_silent_mic(&name);
    }
    state.warm.force_reopen();
    logbuf::log_line(&state.config_dir, "info", "mic level raised by user (Fix it)");
    result
}

#[tauri::command]
async fn fix_output_volume() -> Result<levels::AudioHealth, String> {
    tauri::async_runtime::spawn_blocking(|| {
        levels::fix_output()?;
        Ok::<_, String>(levels::health(None))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn mic_level(state: tauri::State<'_, AppState>) -> f32 {
    state.warm.level()
}

#[tauri::command]
fn get_mic_status(state: tauri::State<'_, AppState>) -> audio::MicStatus {
    let preference = state
        .settings
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .microphone
        .clone();
    audio::mic_status(preference.as_deref())
}

#[tauri::command]
fn dismiss_error(state: tauri::State<'_, AppState>) {
    *state
        .last_error
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = None;
}

#[tauri::command]
fn get_history(state: tauri::State<'_, AppState>) -> Vec<HistoryEntry> {
    state
        .history
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[tauri::command]
fn delete_history_entry(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let mut history = state
        .history
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    history.retain(|entry| entry.id != id);
    store::save_history(&state.config_dir, &history)
}

#[tauri::command]
fn clear_history(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut history = state
        .history
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    history.clear();
    store::save_history(&state.config_dir, &history)
}

#[tauri::command]
fn set_hotkey_capture(state: tauri::State<'_, AppState>, paused: bool) {
    state.listener.paused.store(paused, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
fn toggle_test_recording(app: tauri::AppHandle) {
    dictation::toggle(&app);
}

#[tauri::command]
fn get_app_info(state: tauri::State<'_, AppState>) -> AppInfo {
    let status = state
        .listener
        .status
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let last_error = state
        .last_error
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        listener_running: status.running,
        listener_error: status.error,
        platform: std::env::consts::OS,
        last_error,
    }
}

#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    insert::set_clipboard_text(&text).map_err(|err| err.to_string())
}

#[tauri::command]
fn open_groq_keys() -> Result<(), String> {
    open::that("https://console.groq.com/keys").map_err(|err| err.to_string())
}

/// Used by the Windows startup smoke test. Ignored unless BOLO_SMOKE=1,
/// so a page cannot exit the normal app by calling it.
#[tauri::command]
fn smoke_finished(app: tauri::AppHandle, ok: bool, detail: Option<String>) {
    if std::env::var("BOLO_SMOKE").ok().as_deref() != Some("1") {
        return;
    }
    if let Some(path) = std::env::var_os("BOLO_SMOKE_RESULT") {
        let line = if ok {
            format!("OK {}\n", detail.unwrap_or_default())
        } else {
            format!("FAIL {}\n", detail.unwrap_or_default().replace(['\r', '\n'], " "))
        };
        let _ = std::fs::write(path, line);
    }
    app.exit(if ok { 0 } else { 1 });
}
