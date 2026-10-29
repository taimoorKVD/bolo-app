//! Global push-to-talk listener.
//!
//! Windows installs a low-level keyboard hook so the chord can be swallowed
//! (Ctrl+Space does not type a space, and a Win chord does not open the Start
//! menu). Other platforms poll key state for development; they cannot swallow
//! keys and are not the supported dictation path.

use crate::hotkey::{is_held, EdgeListener, Hotkey, HotkeyEdge};
#[cfg(not(windows))]
use crate::hotkey::KeySnapshot;
#[cfg(windows)]
use crate::hotkey::{kind_from_vk, should_swallow_keydown, snapshot_from_kinds, KeyKind};
#[cfg(windows)]
use std::collections::HashSet;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
#[cfg(not(windows))]
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenerStatus {
    pub running: bool,
    pub error: Option<String>,
}

type StatusHook = Arc<dyn Fn(ListenerStatus) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Dictation(HotkeyEdge),
    PasteAgain,
}

#[derive(Clone)]
pub struct ListenerControl {
    pub hotkey: Arc<RwLock<Hotkey>>,
    pub paste_hotkey: Arc<RwLock<Hotkey>>,
    pub paused: Arc<AtomicBool>,
    pub status: Arc<Mutex<ListenerStatus>>,
    /// Win32 id of the hook thread (0 until it starts). Used to post a reinstall.
    pub hook_thread: Arc<AtomicU32>,
    /// `KBDLLHOOKSTRUCT.time` of the last key the hook saw (GetTickCount base).
    pub last_key_tick: Arc<AtomicU32>,
    on_status: Arc<Mutex<Option<StatusHook>>>,
}

impl ListenerControl {
    pub fn new(hotkey: Hotkey) -> Self {
        Self {
            hotkey: Arc::new(RwLock::new(hotkey)),
            paste_hotkey: Arc::new(RwLock::new(Hotkey::paste_again())),
            paused: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(ListenerStatus {
                running: false,
                error: None,
            })),
            hook_thread: Arc::new(AtomicU32::new(0)),
            last_key_tick: Arc::new(AtomicU32::new(0)),
            on_status: Arc::new(Mutex::new(None)),
        }
    }

    /// Called on the hook thread whenever the listener starts or stops.
    /// The settings page listens so it does not keep the startup status.
    pub fn set_on_status(&self, hook: impl Fn(ListenerStatus) + Send + Sync + 'static) {
        if let Ok(mut slot) = self.on_status.lock() {
            *slot = Some(Arc::new(hook));
        }
    }

    pub fn current_hotkey(&self) -> Hotkey {
        self.hotkey
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn set_hotkey(&self, hotkey: Hotkey) {
        *self.hotkey.write().unwrap_or_else(|err| err.into_inner()) = hotkey;
    }

    /// Ask the hook thread to drop and reinstall its keyboard hook. Windows
    /// silently unhooks a low-level hook that misses LowLevelHooksTimeout,
    /// which happens under load at login and after resume.
    pub fn request_reinstall(&self) -> bool {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::{LPARAM, WPARAM};
            use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
            let id = self.hook_thread.load(Ordering::SeqCst);
            if id == 0 {
                return false;
            }
            unsafe { PostThreadMessageW(id, WM_REINSTALL, WPARAM(0), LPARAM(0)).is_ok() }
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn current_paste_hotkey(&self) -> Hotkey {
        self.paste_hotkey
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn set_paste_hotkey(&self, hotkey: Hotkey) {
        *self.paste_hotkey.write().unwrap_or_else(|err| err.into_inner()) = hotkey;
    }
}

#[cfg(windows)]
const WM_REINSTALL: u32 = 0x8000 + 0x42; // WM_APP + 0x42

pub fn spawn(control: ListenerControl, tx: Sender<InputEvent>) {
    let reported = control.clone();
    let thread_status = reported.clone();
    match thread::Builder::new().name("cadence-hotkey".into()).spawn(move || {
        if let Err(err) = run_platform(control, tx) {
            set_status(&thread_status, false, Some(err));
        }
    }) {
        Ok(_) => {}
        Err(err) => set_status(&reported, false, Some(err.to_string())),
    }
}

fn set_status(control: &ListenerControl, running: bool, error: Option<String>) {
    let snapshot = ListenerStatus { running, error };
    if let Ok(mut status) = control.status.lock() {
        *status = snapshot.clone();
    }
    let notify = control.on_status.lock().ok().and_then(|hook| hook.clone());
    if let Some(notify) = notify {
        notify(snapshot);
    }
}

#[cfg(windows)]
fn run_platform(control: ListenerControl, tx: Sender<InputEvent>) -> Result<(), String> {
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
        KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    struct HookShared {
        control: ListenerControl,
        tx: Sender<InputEvent>,
        down: HashSet<u16>,
        swallowed: HashSet<u16>,
        edges: EdgeListener,
        paste_edges: EdgeListener,
    }

    static HOOK: Mutex<Option<HookShared>> = Mutex::new(None);
    static LAST_TICK: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code < 0 {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Injected events include our own silent key. Ignoring them avoids
        // re-entering the hook lock from inside SendInput.
        if info.flags.0 & 0x10 != 0 {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        let message = wparam.0 as u32;
        let is_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
        let is_up = message == WM_KEYUP || message == WM_SYSKEYUP;
        if !is_down && !is_up {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        let vk = info.vkCode as u16;
        LAST_TICK.store(info.time, Ordering::Relaxed);
        let swallow = {
            let mut guard = HOOK.lock().unwrap_or_else(|err| err.into_inner());
            let Some(shared) = guard.as_mut() else {
                return unsafe { CallNextHookEx(None, code, wparam, lparam) };
            };
            on_key(shared, vk, is_down)
        };
        if swallow {
            LRESULT(1)
        } else {
            unsafe { CallNextHookEx(None, code, wparam, lparam) }
        }
    }

    fn on_key(shared: &mut HookShared, vk: u16, is_down: bool) -> bool {
        shared.control.last_key_tick.store(LAST_TICK.load(Ordering::Relaxed), Ordering::Relaxed);
        let repeat = is_down && shared.down.contains(&vk);
        if is_down {
            shared.down.insert(vk);
        } else {
            shared.down.remove(&vk);
        }
        let kinds: Vec<KeyKind> = shared.down.iter().copied().map(kind_from_vk).collect();
        let snap = snapshot_from_kinds(&kinds);
        let hotkey = shared.control.current_hotkey();
        let paste_hotkey = shared.control.current_paste_hotkey();
        let paused = shared.control.paused.load(Ordering::SeqCst);
        let kind = kind_from_vk(vk);
        let was_holding = shared.edges.holding();

        let mut swallow = false;
        if is_down {
            if repeat {
                swallow = shared.swallowed.contains(&vk);
            } else if should_swallow_keydown(&kind, &snap, &hotkey)
                || should_swallow_keydown(&kind, &snap, &paste_hotkey)
            {
                shared.swallowed.insert(vk);
                swallow = true;
            }
        } else if shared.swallowed.remove(&vk) {
            swallow = true;
        }

        // A lone Win tap opens the Start menu on key-up. If this chord used Win,
        // tap an unassigned virtual key first so Windows treats it as a combo.
        if !is_down && matches!(kind, KeyKind::Win) && hotkey.win && was_holding {
            send_silent_key();
        }

        if let Some(edge) = shared.edges.observe(is_held(&snap, &hotkey), paused) {
            let _ = shared.tx.send(InputEvent::Dictation(edge));
        }
        if let Some(HotkeyEdge::Pressed) = shared.paste_edges.observe(is_held(&snap, &paste_hotkey), paused) {
            let _ = shared.tx.send(InputEvent::PasteAgain);
        }
        swallow
    }

    {
        let mut guard = HOOK.lock().map_err(|err| err.to_string())?;
        *guard = Some(HookShared {
            control: control.clone(),
            tx,
            down: HashSet::new(),
            swallowed: HashSet::new(),
            edges: EdgeListener::default(),
            paste_edges: EdgeListener::default(),
        });
    }

    use windows::Win32::System::Threading::{
        GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
    };
    use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, UnhookWindowsHookEx, HHOOK, PM_NOREMOVE};

    let started = std::time::Instant::now();
    unsafe {
        // The hook callback must return within LowLevelHooksTimeout or Windows
        // drops it. At login the CPU is saturated, so run this thread first.
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
        // Create the message queue before publishing the id so a posted
        // reinstall is never lost.
        let mut probe = MSG::default();
        let _ = PeekMessageW(&mut probe, None, 0, 0, PM_NOREMOVE);
        control.hook_thread.store(GetCurrentThreadId(), Ordering::SeqCst);
    }

    let install = |control: &ListenerControl| -> HHOOK {
        // Retry with backoff: SetWindowsHookExW can fail while the desktop
        // session is still coming up.
        let mut delay = 100u64;
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) } {
                Ok(hook) => {
                    set_status(control, true, None);
                    return hook;
                }
                Err(err) => {
                    let message = format!("couldn't install the keyboard hook (attempt {attempt}): {err}");
                    set_status(control, false, Some(message));
                    thread::sleep(std::time::Duration::from_millis(delay));
                    delay = (delay * 2).min(5_000);
                }
            }
        }
    };

    let mut hook = install(&control);
    crate::listener::note_timing("hook_installed_ms", started.elapsed().as_millis() as u64);
    let reset_keys = || {
        if let Ok(mut guard) = HOOK.lock() {
            if let Some(shared) = guard.as_mut() {
                shared.down.clear();
                shared.swallowed.clear();
                shared.edges = EdgeListener::default();
                shared.paste_edges = EdgeListener::default();
            }
        }
    };
    let _ = &LAST_TICK;
    unsafe {
        let mut message = MSG::default();
        loop {
            // GetMessageW returns nonzero for a message, 0 for WM_QUIT, and -1 on failure.
            // BOOL is a newtype around that i32; as_bool() would hide the -1 error.
            let status = GetMessageW(&mut message, None, 0, 0).0;
            if status == 0 {
                break;
            }
            if status < 0 {
                let _ = UnhookWindowsHookEx(hook);
                return Err("the keyboard hook message loop failed".into());
            }
            if message.hwnd.0.is_null() && message.message == WM_REINSTALL {
                let at = std::time::Instant::now();
                let _ = UnhookWindowsHookEx(hook);
                reset_keys();
                hook = install(&control);
                crate::listener::note_timing("hook_reinstalled_ms", at.elapsed().as_millis() as u64);
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
    set_status(&control, false, None);
    Ok(())
}

type TimingHook = Box<dyn Fn(&str, u64) + Send + Sync>;
static TIMING: Mutex<Option<TimingHook>> = Mutex::new(None);

/// Route listener timings to the app log (set once from setup).
pub fn set_timing_sink(sink: impl Fn(&str, u64) + Send + Sync + 'static) {
    if let Ok(mut slot) = TIMING.lock() {
        *slot = Some(Box::new(sink));
    }
}

#[allow(dead_code)]
pub(crate) fn note_timing(name: &str, ms: u64) {
    if let Ok(slot) = TIMING.lock() {
        if let Some(sink) = slot.as_ref() {
            sink(name, ms);
        }
    }
}

#[cfg(windows)]
fn send_silent_key() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY,
    };
    let silent = VIRTUAL_KEY(0xE8);
    let inputs = [key_input(silent, false), key_input(silent, true)];
    unsafe {
        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }

    fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
}

#[cfg(not(windows))]
fn run_platform(control: ListenerControl, tx: Sender<InputEvent>) -> Result<(), String> {
    use device_query::{DeviceQuery, DeviceState, Keycode};

    let device = DeviceState::new();
    set_status(&control, true, None);
    let mut edges = EdgeListener::default();
    let mut paste_edges = EdgeListener::default();
    loop {
        let hotkey = control.current_hotkey();
        let paste_hotkey = control.current_paste_hotkey();
        let paused = control.paused.load(Ordering::SeqCst);
        let keys: Vec<Keycode> = device.get_keys();
        let snap = snapshot_from_keycodes(&keys);
        if let Some(edge) = edges.observe(is_held(&snap, &hotkey), paused) {
            let _ = tx.send(InputEvent::Dictation(edge));
        }
        if let Some(HotkeyEdge::Pressed) = paste_edges.observe(is_held(&snap, &paste_hotkey), paused) {
            let _ = tx.send(InputEvent::PasteAgain);
        }
        thread::sleep(Duration::from_millis(12));
    }
}

#[cfg(not(windows))]
fn snapshot_from_keycodes(keys: &[device_query::Keycode]) -> KeySnapshot {
    use device_query::Keycode;
    let mut snap = KeySnapshot::default();
    for key in keys {
        match key {
            Keycode::LControl | Keycode::RControl => snap.ctrl = true,
            Keycode::LAlt | Keycode::RAlt | Keycode::LOption | Keycode::ROption => snap.alt = true,
            Keycode::LShift | Keycode::RShift => snap.shift = true,
            Keycode::LMeta | Keycode::RMeta | Keycode::Command => snap.win = true,
            other => {
                snap.keys.insert(keycode_name(*other));
            }
        }
    }
    snap
}

#[cfg(not(windows))]
fn keycode_name(key: device_query::Keycode) -> String {
    use device_query::Keycode;
    match key {
        Keycode::Space => "space".into(),
        Keycode::Tab => "tab".into(),
        Keycode::Enter | Keycode::NumpadEnter => "enter".into(),
        Keycode::Backspace => "backspace".into(),
        Keycode::Delete => "delete".into(),
        Keycode::Insert => "insert".into(),
        Keycode::Home => "home".into(),
        Keycode::End => "end".into(),
        Keycode::PageUp => "pageup".into(),
        Keycode::PageDown => "pagedown".into(),
        Keycode::Left => "left".into(),
        Keycode::Right => "right".into(),
        Keycode::Up => "up".into(),
        Keycode::Down => "down".into(),
        Keycode::Escape => "esc".into(),
        Keycode::A => "a".into(),
        Keycode::B => "b".into(),
        Keycode::C => "c".into(),
        Keycode::D => "d".into(),
        Keycode::E => "e".into(),
        Keycode::F => "f".into(),
        Keycode::G => "g".into(),
        Keycode::H => "h".into(),
        Keycode::I => "i".into(),
        Keycode::J => "j".into(),
        Keycode::K => "k".into(),
        Keycode::L => "l".into(),
        Keycode::M => "m".into(),
        Keycode::N => "n".into(),
        Keycode::O => "o".into(),
        Keycode::P => "p".into(),
        Keycode::Q => "q".into(),
        Keycode::R => "r".into(),
        Keycode::S => "s".into(),
        Keycode::T => "t".into(),
        Keycode::U => "u".into(),
        Keycode::V => "v".into(),
        Keycode::W => "w".into(),
        Keycode::X => "x".into(),
        Keycode::Y => "y".into(),
        Keycode::Z => "z".into(),
        Keycode::Key0 | Keycode::Numpad0 => "0".into(),
        Keycode::Key1 | Keycode::Numpad1 => "1".into(),
        Keycode::Key2 | Keycode::Numpad2 => "2".into(),
        Keycode::Key3 | Keycode::Numpad3 => "3".into(),
        Keycode::Key4 | Keycode::Numpad4 => "4".into(),
        Keycode::Key5 | Keycode::Numpad5 => "5".into(),
        Keycode::Key6 | Keycode::Numpad6 => "6".into(),
        Keycode::Key7 | Keycode::Numpad7 => "7".into(),
        Keycode::Key8 | Keycode::Numpad8 => "8".into(),
        Keycode::Key9 | Keycode::Numpad9 => "9".into(),
        Keycode::F1 => "f1".into(),
        Keycode::F2 => "f2".into(),
        Keycode::F3 => "f3".into(),
        Keycode::F4 => "f4".into(),
        Keycode::F5 => "f5".into(),
        Keycode::F6 => "f6".into(),
        Keycode::F7 => "f7".into(),
        Keycode::F8 => "f8".into(),
        Keycode::F9 => "f9".into(),
        Keycode::F10 => "f10".into(),
        Keycode::F11 => "f11".into(),
        Keycode::F12 => "f12".into(),
        other => format!("other:{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_hook_fires_when_the_listener_reports_in() {
        let control = ListenerControl::new(Hotkey::default());
        let seen = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&seen);
        control.set_on_status(move |status| {
            *slot.lock().unwrap_or_else(|err| err.into_inner()) = Some(status);
        });
        set_status(&control, true, None);
        let status = seen.lock().unwrap_or_else(|err| err.into_inner()).clone();
        let status = status.expect("hook should have run");
        assert!(status.running);
        assert!(status.error.is_none());

        set_status(&control, false, Some("couldn't install the keyboard hook".into()));
        let status = seen
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
            .expect("hook should have run again");
        assert!(!status.running);
        assert_eq!(
            status.error.as_deref(),
            Some("couldn't install the keyboard hook")
        );
    }
}
