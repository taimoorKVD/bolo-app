//! Insert text at the caret.
//!
//! The primary path copies the transcript to the clipboard, sends Ctrl+V, then
//! puts the previous text clipboard back. If the clipboard holds an image, or
//! the paste simulation fails, Cadence types the text instead.

use anyhow::{anyhow, Context};
use arboard::Clipboard;
use serde::Serialize;
use std::thread;
use std::time::Duration;

const CLIPBOARD_PUBLISH_DELAY: Duration = Duration::from_millis(8);
const PASTE_SETTLE_DELAY: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InsertMethod {
    Clipboard,
    Typed,
}

pub fn insert_text(text: &str) -> anyhow::Result<InsertMethod> {
    let text = text.trim();
    if text.is_empty() {
        return Err(anyhow!("nothing to insert"));
    }
    match paste_via_clipboard(text) {
        Ok(()) => Ok(InsertMethod::Clipboard),
        Err(err) => {
            eprintln!("clipboard paste unavailable ({err}); typing the transcript");
            type_text(text).context("couldn't type the transcript either")?;
            Ok(InsertMethod::Typed)
        }
    }
}

pub fn set_clipboard_text(text: &str) -> anyhow::Result<()> {
    let mut clipboard = Clipboard::new().context("couldn't open the clipboard")?;
    clipboard
        .set_text(text)
        .context("couldn't write the clipboard")?;
    Ok(())
}

fn paste_via_clipboard(text: &str) -> anyhow::Result<()> {
    let mut clipboard = Clipboard::new().context("couldn't open the clipboard")?;
    let previous_text = clipboard.get_text().ok();
    if previous_text.is_none() && clipboard.get_image().is_ok() {
        return Err(anyhow!("clipboard holds an image"));
    }
    clipboard
        .set_text(text)
        .context("couldn't write the clipboard")?;
    thread::sleep(CLIPBOARD_PUBLISH_DELAY);
    if let Err(err) = send_ctrl_v() {
        restore_text(&mut clipboard, previous_text);
        return Err(err);
    }
    thread::sleep(PASTE_SETTLE_DELAY);
    restore_text(&mut clipboard, previous_text);
    Ok(())
}

fn restore_text(clipboard: &mut Clipboard, previous: Option<String>) {
    let restored = if let Some(previous) = previous {
        clipboard.set_text(previous).is_ok()
    } else {
        clipboard.clear().is_ok()
    };
    if !restored {
        eprintln!("couldn't restore the previous clipboard contents");
    }
}

#[cfg(windows)]
fn send_ctrl_v() -> anyhow::Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_CONTROL, VK_V,
    };

    let inputs = [
        key_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_V, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_V, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err(anyhow!("Ctrl+V simulation was incomplete"));
    }
    Ok(())
}

#[cfg(windows)]
fn key_input(
    vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT};
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(windows)]
fn type_text(text: &str) -> anyhow::Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};

    let mut inputs = Vec::new();
    for unit in text.encode_utf16() {
        inputs.push(unicode_input(unit, false));
        inputs.push(unicode_input(unit, true));
    }
    for chunk in inputs.chunks(32) {
        let sent = unsafe { SendInput(chunk, std::mem::size_of::<INPUT>() as i32) };
        if sent != chunk.len() as u32 {
            return Err(anyhow!("text simulation was incomplete"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn unicode_input(unit: u16, up: bool) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
    };
    let flags = if up {
        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
    } else {
        KEYEVENTF_UNICODE
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(not(windows))]
fn send_ctrl_v() -> anyhow::Result<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|err| anyhow!("input setup: {err}"))?;
    enigo
        .key(Key::Control, Direction::Press)
        .map_err(|err| anyhow!("input: {err}"))?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(|err| anyhow!("input: {err}"))?;
    enigo
        .key(Key::Control, Direction::Release)
        .map_err(|err| anyhow!("input: {err}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn type_text(text: &str) -> anyhow::Result<()> {
    use enigo::{Enigo, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|err| anyhow!("input setup: {err}"))?;
    enigo
        .text(text)
        .map_err(|err| anyhow!("input: {err}"))?;
    Ok(())
}

/// Paste into a real Win32 EDIT control without taking foreground focus.
///
/// Ctrl+V via SendInput only reaches whichever window is foreground, and a
/// `GetMessage` loop does not end on `WM_CLOSE` alone (that was the hang).
/// This test writes the clipboard, then sends `WM_PASTE` straight to the
/// control — the message an Edit control handles for Ctrl+V. Every wait is
/// bounded. The control stays hidden.
#[cfg(all(test, windows))]
#[test]
fn pastes_into_a_real_edit_control() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextW, SendMessageW, WM_PASTE};

    const PHRASE: &str = "cadence paste ok";

    eprintln!("paste test: creating a hidden edit control");
    let mut host = EditHost::spawn();
    let edit = match host.ready() {
        Ok(edit) => edit,
        Err(err) => panic!("paste test: {err}"),
    };
    eprintln!("paste test: edit control is ready");

    let previous = match Clipboard::new() {
        Ok(mut clipboard) => match clipboard.get_text() {
            Ok(text) => Some(text),
            Err(_) if clipboard.get_image().is_ok() => {
                panic!("paste test: the clipboard holds an image, so it was left untouched")
            }
            Err(_) => None,
        },
        Err(err) => panic!("paste test: couldn't open the clipboard: {err}"),
    };
    let _restore = ClipboardGuard {
        had_text: previous.is_some(),
        previous,
    };

    within("writing the clipboard", || {
        set_clipboard_text(PHRASE).unwrap_or_else(|err| panic!("paste test: couldn't write the clipboard: {err}"));
    });
    let written = within("reading the clipboard back", || {
        Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .unwrap_or_else(|err| panic!("paste test: clipboard read-back failed: {err}"))
    });
    assert_eq!(
        written, PHRASE,
        "paste test: clipboard returned {written:?} after writing {PHRASE:?}"
    );
    eprintln!("paste test: clipboard round trip ok, sending WM_PASTE");

    within("WM_PASTE", move || {
        let edit = HWND(edit as *mut core::ffi::c_void);
        unsafe {
            SendMessageW(edit, WM_PASTE, None, None);
        }
    });
    let text = within("reading the edit control", move || {
        let edit = HWND(edit as *mut core::ffi::c_void);
        let mut buffer = [0u16; 256];
        let length = unsafe { GetWindowTextW(edit, &mut buffer) }.max(0) as usize;
        String::from_utf16_lossy(&buffer[..length.min(buffer.len())])
    });
    eprintln!("paste test: edit control contains {text:?}");
    host.shutdown();

    assert!(
        text.to_lowercase().contains(PHRASE),
        "paste test: WM_PASTE did not insert the clipboard text. The edit control contained {text:?}."
    );
}

#[cfg(all(test, windows))]
const PASTE_STEP: Duration = Duration::from_secs(5);

#[cfg(all(test, windows))]
fn within<T: Send + 'static>(what: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
    use std::sync::mpsc::{self, RecvTimeoutError};
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(work());
    });
    match rx.recv_timeout(PASTE_STEP) {
        Ok(value) => value,
        Err(RecvTimeoutError::Timeout) => panic!(
            "paste test: timed out after {}s during {what}. This step does not wait for foreground focus.",
            PASTE_STEP.as_secs()
        ),
        Err(RecvTimeoutError::Disconnected) => panic!("paste test: {what} stopped before it finished"),
    }
}

#[cfg(all(test, windows))]
struct ClipboardGuard {
    previous: Option<String>,
    had_text: bool,
}

#[cfg(all(test, windows))]
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        let Ok(mut clipboard) = Clipboard::new() else {
            eprintln!("paste test: couldn't reopen the clipboard to restore it");
            return;
        };
        let restored = if self.had_text {
            clipboard.set_text(self.previous.take().unwrap_or_default()).is_ok()
        } else {
            clipboard.clear().is_ok()
        };
        if !restored {
            eprintln!("paste test: couldn't restore the previous clipboard");
        }
    }
}

#[cfg(all(test, windows))]
struct EditHost {
    quit: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    ready_rx: Option<std::sync::mpsc::Receiver<Result<usize, String>>>,
}

#[cfg(all(test, windows))]
impl EditHost {
    fn spawn() -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc;
        use std::sync::Arc;
        use windows::Win32::Foundation::HINSTANCE;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, RegisterClassW, TranslateMessage,
            UnregisterClassW, HMENU, MSG, PM_NOREMOVE, PM_REMOVE, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
            WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        };
        use windows::core::{w, PCWSTR};

        let quit = Arc::new(AtomicBool::new(false));
        let quit_thread = Arc::clone(&quit);
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = thread::spawn(move || unsafe {
            let class_name: Vec<u16> = format!("CadencePasteTest{}\0", std::process::id())
                .encode_utf16()
                .collect();
            let module = match GetModuleHandleW(None) {
                Ok(module) => module,
                Err(err) => {
                    let _ = ready_tx.send(Err(format!("GetModuleHandleW failed: {err}")));
                    return;
                }
            };
            let instance = HINSTANCE(module.0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(paste_wnd_proc),
                hInstance: instance,
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                let _ = ready_tx.send(Err(format!(
                    "RegisterClassW failed: {}",
                    windows::core::Error::from_win32()
                )));
                return;
            }
            let parent = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class_name.as_ptr()),
                w!("Cadence paste"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                320,
                120,
                None,
                None,
                Some(instance),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(err) => {
                    let _ = UnregisterClassW(PCWSTR(class_name.as_ptr()), Some(instance));
                    let _ = ready_tx.send(Err(format!("parent window failed: {err}")));
                    return;
                }
            };
            let edit = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0080),
                0,
                0,
                300,
                24,
                Some(parent),
                None::<HMENU>,
                Some(instance),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(err) => {
                    let _ = DestroyWindow(parent);
                    let _ = UnregisterClassW(PCWSTR(class_name.as_ptr()), Some(instance));
                    let _ = ready_tx.send(Err(format!("edit control failed: {err}")));
                    return;
                }
            };
            // Create the thread queue before publishing the handle, so a later
            // WM_PASTE is dispatched instead of waiting for a queue.
            let mut message = MSG::default();
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            if ready_tx.send(Ok(edit.0 as usize)).is_err() {
                let _ = DestroyWindow(parent);
                let _ = UnregisterClassW(PCWSTR(class_name.as_ptr()), Some(instance));
                return;
            }
            while !quit_thread.load(Ordering::Relaxed) {
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                thread::sleep(Duration::from_millis(10));
            }
            let _ = DestroyWindow(parent);
            let _ = UnregisterClassW(PCWSTR(class_name.as_ptr()), Some(instance));
        });
        Self {
            quit,
            thread: Some(thread),
            ready_rx: Some(ready_rx),
        }
    }

    fn ready(&mut self) -> Result<usize, String> {
        use std::sync::mpsc::RecvTimeoutError;
        let rx = self.ready_rx.take().ok_or_else(|| "edit control was already claimed".to_string())?;
        match rx.recv_timeout(PASTE_STEP) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(format!(
                "timed out after {}s waiting for the edit control. The window thread did not report ready.",
                PASTE_STEP.as_secs()
            )),
            Err(RecvTimeoutError::Disconnected) => {
                Err("the window thread exited before creating the edit control".to_string())
            }
        }
    }

    fn shutdown(&mut self) {
        self.stop(true);
    }

    fn stop(&mut self, fail_on_timeout: bool) {
        use std::sync::atomic::Ordering;
        use std::sync::mpsc;
        self.quit.store(true, Ordering::Relaxed);
        let Some(thread) = self.thread.take() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = thread.join();
            let _ = tx.send(());
        });
        if rx.recv_timeout(PASTE_STEP).is_err() {
            let message = format!(
                "paste test: timed out after {}s waiting for the edit-control thread to exit",
                PASTE_STEP.as_secs()
            );
            if fail_on_timeout {
                panic!("{message}");
            }
            eprintln!("{message}");
        }
    }
}

#[cfg(all(test, windows))]
impl Drop for EditHost {
    fn drop(&mut self) {
        self.stop(false);
    }
}

#[cfg(test)]
mod delay_tests {
    use super::*;

    #[test]
    fn clipboard_paste_does_not_sleep_through_the_take() {
        assert!(CLIPBOARD_PUBLISH_DELAY <= Duration::from_millis(15));
        assert!(PASTE_SETTLE_DELAY <= Duration::from_millis(40));
    }
}

#[cfg(all(test, windows))]
unsafe extern "system" fn paste_wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::DefWindowProcW;
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
