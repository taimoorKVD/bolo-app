//! Keeps push-to-talk alive across login, sleep, and hibernate.
//!
//! Every two seconds this thread checks:
//! * a wall-clock jump (the thread could not run, so the PC was asleep or
//!   hibernated): reinstall the keyboard hook, reopen the warm microphone, and
//!   re-warm the Groq connection;
//! * the hook reported not running: reinstall it;
//! * the user has been typing or clicking for a while but the hook has not seen
//!   a single key: Windows probably removed it silently, so reinstall it
//!   (at most once a minute; reinstalling a working hook is harmless).

use crate::dictation::AppState;
use crate::logbuf::log_line;
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Manager};

const TICK: Duration = Duration::from_secs(2);
const RESUME_GAP: Duration = Duration::from_secs(15);
const SILENT_HOOK_MS: u32 = 60_000;
const REINSTALL_EVERY: Duration = Duration::from_secs(60);

pub fn spawn(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("bolo-health".into())
        .spawn(move || run(app));
}

fn run(app: AppHandle) {
    let mut last_wall = SystemTime::now();
    let mut last_reinstall = Instant::now();
    loop {
        std::thread::sleep(TICK);
        let state = app.state::<AppState>();
        let now = SystemTime::now();
        let gap = now.duration_since(last_wall).unwrap_or_default();
        last_wall = now;

        if gap > RESUME_GAP {
            log_line(
                &state.config_dir,
                "info",
                &format!("health resume detected gap_s={}", gap.as_secs()),
            );
            on_resume(&app);
            last_reinstall = Instant::now();
            continue;
        }

        let running = state
            .listener
            .status
            .lock()
            .map(|status| status.running)
            .unwrap_or(true);
        let lost = !running || hook_looks_lost(state.listener.last_key_tick.load(std::sync::atomic::Ordering::Relaxed));
        if lost && last_reinstall.elapsed() >= REINSTALL_EVERY {
            last_reinstall = Instant::now();
            if state.listener.request_reinstall() {
                log_line(
                    &state.config_dir,
                    "info",
                    &format!("health hook reinstall running={running}"),
                );
            }
        }
    }
}

fn on_resume(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.listener.request_reinstall();
    state.warm.force_reopen();
    let dir = state.config_dir.clone();
    tauri::async_runtime::spawn(async move {
        // Give Wi-Fi a moment to reconnect after resume.
        tokio_sleep(Duration::from_secs(5)).await;
        crate::transcribe::prewarm(&dir).await;
    });
}

async fn tokio_sleep(duration: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(duration)).await;
}

#[cfg(windows)]
fn hook_looks_lost(last_key_tick: u32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return false;
    }
    if last_key_tick == 0 {
        return false;
    }
    // Both are GetTickCount values; wrapping_sub handles the 49-day wrap.
    let ahead = info.dwTime.wrapping_sub(last_key_tick);
    ahead < u32::MAX / 2 && ahead > SILENT_HOOK_MS
}

#[cfg(not(windows))]
fn hook_looks_lost(_last_key_tick: u32) -> bool {
    false
}
