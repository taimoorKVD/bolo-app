//! In-app updates for the direct-download (NSIS) build.
//!
//! Compiled only with the `updater` cargo feature. The Microsoft Store build
//! (`scripts/build-msix.ps1`) disables it because the Store updates the app.

use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub version: String,
    pub notes: String,
    pub date: Option<String>,
}

#[cfg(feature = "updater")]
mod imp {
    use super::UpdateInfo;
    use crate::logbuf::log_line;
    use std::sync::Mutex;
    use std::time::Duration;
    use tauri::{AppHandle, Emitter, Manager};
    use tauri_plugin_updater::{Update, UpdaterExt};

    const FIRST_CHECK: Duration = Duration::from_secs(45);
    const EVERY: Duration = Duration::from_secs(4 * 60 * 60);

    #[derive(Default)]
    pub struct Pending {
        update: Mutex<Option<Update>>,
        announced: Mutex<Option<String>>,
    }

    pub fn enabled() -> bool {
        std::env::var("BOLO_DISABLE_UPDATER").ok().as_deref() != Some("1")
    }

    pub fn init<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
        builder.plugin(tauri_plugin_updater::Builder::new().build())
    }

    pub fn spawn(app: &AppHandle) {
        app.manage(Pending::default());
        if !enabled() {
            return;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            sleep(FIRST_CHECK).await;
            loop {
                let _ = check(&app, true).await;
                sleep(EVERY).await;
            }
        });
    }

    async fn sleep(duration: Duration) {
        let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(duration)).await;
    }

    fn dir(app: &AppHandle) -> std::path::PathBuf {
        app.state::<crate::dictation::AppState>().config_dir.clone()
    }

    pub async fn check(app: &AppHandle, background: bool) -> Result<Option<UpdateInfo>, String> {
        let started = std::time::Instant::now();
        let result = async {
            let updater = app.updater().map_err(|err| err.to_string())?;
            updater.check().await.map_err(|err| err.to_string())
        }
        .await;
        let ms = started.elapsed().as_millis();
        match result {
            Ok(Some(update)) => {
                let info = UpdateInfo {
                    current_version: update.current_version.clone(),
                    version: update.version.clone(),
                    notes: update.body.clone().unwrap_or_default(),
                    date: update.date.map(|date| date.to_string()),
                };
                log_line(&dir(app), "info", &format!("update available version={} check_ms={ms}", info.version));
                let pending = app.state::<Pending>();
                *pending.update.lock().unwrap_or_else(|err| err.into_inner()) = Some(update);
                let first_time = {
                    let mut seen = pending.announced.lock().unwrap_or_else(|err| err.into_inner());
                    let fresh = seen.as_deref() != Some(info.version.as_str());
                    *seen = Some(info.version.clone());
                    fresh
                };
                if background && first_time {
                    let _ = app.emit("update-available", &info);
                    crate::show_settings(app);
                }
                Ok(Some(info))
            }
            Ok(None) => {
                log_line(&dir(app), "info", &format!("update check: up to date check_ms={ms}"));
                Ok(None)
            }
            Err(err) => {
                log_line(&dir(app), "warn", &format!("update check failed check_ms={ms} err={err}"));
                Err(err)
            }
        }
    }

    pub fn pending(app: &AppHandle) -> Option<UpdateInfo> {
        let pending = app.state::<Pending>();
        let guard = pending.update.lock().unwrap_or_else(|err| err.into_inner());
        guard.as_ref().map(|update| UpdateInfo {
            current_version: update.current_version.clone(),
            version: update.version.clone(),
            notes: update.body.clone().unwrap_or_default(),
            date: update.date.map(|date| date.to_string()),
        })
    }

    pub async fn install(app: &AppHandle) -> Result<(), String> {
        let update = {
            let pending = app.state::<Pending>();
            let taken = pending.update.lock().unwrap_or_else(|err| err.into_inner()).take();
            taken
        };
        let update = match update {
            Some(update) => update,
            None => {
                check(app, false).await?;
                let pending = app.state::<Pending>();
                let taken = pending.update.lock().unwrap_or_else(|err| err.into_inner()).take();
                taken.ok_or_else(|| "Bolo is already up to date.".to_string())?
            }
        };
        log_line(&dir(app), "info", &format!("update installing version={}", update.version));
        let progress_app = app.clone();
        let mut received: u64 = 0;
        update
            .download_and_install(
                move |chunk, total| {
                    received += chunk as u64;
                    let _ = progress_app.emit(
                        "update-progress",
                        serde_json::json!({ "received": received, "total": total }),
                    );
                },
                || {},
            )
            .await
            .map_err(|err| {
                log_line(&dir(app), "error", &format!("update install failed: {err}"));
                err.to_string()
            })?;
        // On Windows the NSIS installer takes over and closes Bolo; restart in
        // case it returns.
        app.restart();
    }
}

#[cfg(not(feature = "updater"))]
mod imp {
    use super::UpdateInfo;
    use tauri::AppHandle;

    pub fn enabled() -> bool {
        false
    }
    pub fn init<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
        builder
    }
    pub fn spawn(_app: &AppHandle) {}
    pub async fn check(_app: &AppHandle, _background: bool) -> Result<Option<UpdateInfo>, String> {
        Ok(None)
    }
    pub fn pending(_app: &AppHandle) -> Option<UpdateInfo> {
        None
    }
    pub async fn install(_app: &AppHandle) -> Result<(), String> {
        Err("Updates for this copy of Bolo come from the Microsoft Store.".into())
    }
}

pub use imp::{check, enabled, init, install, pending, spawn};

#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    if !enabled() {
        return Ok(None);
    }
    check(&app, false).await
}

#[tauri::command]
pub fn pending_update(app: tauri::AppHandle) -> Option<UpdateInfo> {
    pending(&app)
}

#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    install(&app).await
}

#[tauri::command]
pub fn updates_enabled() -> bool {
    enabled()
}
