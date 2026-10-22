//! Watch for microphones being plugged, unplugged, or chosen as the Windows default.
//!
//! Every recording already re-resolves the device. This thread updates the
//! settings window while Cadence is idle. On Windows it also registers an
//! endpoint notification callback so a hot-plug is noticed immediately; a
//! one-second poll covers the same changes if the callback cannot be installed.

use crate::audio::{self, MicStatus};
use crate::dictation::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub fn spawn(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("cadence-mics".into())
        .spawn(move || watch(app));
}

fn watch(app: AppHandle) {
    // The first enumeration initializes COM on this thread (cpal does that for
    // WASAPI). The notification client is registered afterwards so it shares
    // that apartment instead of fighting a second CoInitialize.
    let mut last = String::new();
    publish(&app, &mut last);
    let changed = Arc::new(AtomicBool::new(false));
    let _notifications = register_notifications(&app, Arc::clone(&changed));
    let mut last_scan = Instant::now();
    loop {
        pump_messages();
        let due = changed.swap(false, Ordering::Relaxed) || last_scan.elapsed() >= Duration::from_secs(1);
        if due {
            last_scan = Instant::now();
            publish(&app, &mut last);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn publish(app: &AppHandle, last: &mut String) {
    let preference = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap_or_else(|err| err.into_inner());
        settings.microphone.clone()
    };
    let status = audio::mic_status(preference.as_deref());
    let fingerprint = fingerprint(&status);
    if fingerprint != *last {
        *last = fingerprint;
        let _ = app.emit("mic-status", status);
    }
}

fn fingerprint(status: &MicStatus) -> String {
    let devices = status
        .devices
        .iter()
        .map(|mic| format!("{}:{}", mic.name, mic.is_default as u8))
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "{:?}|{:?}|{}|{:?}|{devices}",
        status.preference, status.active_name, status.using_fallback, status.message
    )
}

#[cfg(windows)]
fn pump_messages() {
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
fn pump_messages() {}

#[cfg(windows)]
struct Notifications {
    pub(crate) enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator,
    pub(crate) client: windows::Win32::Media::Audio::IMMNotificationClient,
}

#[cfg(windows)]
impl Drop for Notifications {
    fn drop(&mut self) {
        unsafe {
            let _ = self.enumerator.UnregisterEndpointNotificationCallback(&self.client);
        }
    }
}

#[cfg(windows)]
fn register_notifications(app: &AppHandle, changed: Arc<AtomicBool>) -> Option<Notifications> {
    match endpoint::install(changed) {
        Ok(notifications) => Some(notifications),
        Err(err) => {
            log_watch(app, &format!("microphone notifications unavailable: {err}"));
            None
        }
    }
}

/// COM callback lives at module scope. `#[implement]` emits `impl` blocks,
/// which Rust does not allow inside a function.
#[cfg(windows)]
mod endpoint {
    use super::Notifications;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use windows::core::implement;
    use windows::Win32::Media::Audio::{
        IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, DEVICE_STATE, EDataFlow, ERole,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    #[implement(IMMNotificationClient)]
    struct EndpointNotifications {
        changed: Arc<AtomicBool>,
    }

    #[allow(non_snake_case)]
    impl IMMNotificationClient_Impl for EndpointNotifications_Impl {
        fn OnDeviceStateChanged(&self, _id: &windows::core::PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()> {
            self.changed.store(true, Ordering::Relaxed);
            Ok(())
        }

        fn OnDeviceAdded(&self, _id: &windows::core::PCWSTR) -> windows::core::Result<()> {
            self.changed.store(true, Ordering::Relaxed);
            Ok(())
        }

        fn OnDeviceRemoved(&self, _id: &windows::core::PCWSTR) -> windows::core::Result<()> {
            self.changed.store(true, Ordering::Relaxed);
            Ok(())
        }

        fn OnDefaultDeviceChanged(
            &self,
            _flow: EDataFlow,
            _role: ERole,
            _id: &windows::core::PCWSTR,
        ) -> windows::core::Result<()> {
            self.changed.store(true, Ordering::Relaxed);
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

    pub fn install(changed: Arc<AtomicBool>) -> windows::core::Result<Notifications> {
        let enumerator = unsafe {
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None::<&windows::core::IUnknown>, CLSCTX_ALL)?
        };
        let client: IMMNotificationClient = EndpointNotifications { changed }.into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client)? };
        Ok(Notifications { enumerator, client })
    }
}

#[cfg(windows)]
fn log_watch(app: &AppHandle, message: &str) {
    let state = app.state::<AppState>();
    crate::logbuf::log_line(&state.config_dir, "info", message);
}

#[cfg(not(windows))]
fn register_notifications(_app: &AppHandle, _changed: Arc<AtomicBool>) -> Option<()> {
    None
}
