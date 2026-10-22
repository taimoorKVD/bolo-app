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

const CLIPBOARD_PUBLISH_DELAY: Duration = Duration::from_millis(40);
const PASTE_SETTLE_DELAY: Duration = Duration::from_millis(250);

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

#[cfg(all(test, windows))]
#[test]
fn pastes_into_a_real_edit_control() {
    use std::sync::mpsc;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageW,
        GetWindowTextW, GetWindowThreadProcessId, PostMessageW, RegisterClassW, SetForegroundWindow,
        ShowWindow, TranslateMessage, HMENU, MSG, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
        WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE, WM_CLOSE,
    };
    use windows::core::{w, PCWSTR};

    unsafe extern "system" fn paste_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    let (tx, rx) = mpsc::channel::<(usize, usize)>();
    let window = std::thread::spawn(move || unsafe {
        let class_name: Vec<u16> = "CadencePasteTest\0".encode_utf16().collect();
        let module = GetModuleHandleW(None).expect("module");
        let instance = HINSTANCE(module.0);
        let wc = WNDCLASSW {
            style: Default::default(),
            lpfnWndProc: Some(paste_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: Default::default(),
            hCursor: Default::default(),
            hbrBackground: Default::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
        };
        let _atom = RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            w!("Cadence paste"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            80,
            80,
            560,
            220,
            None,
            None,
            Some(instance),
            None,
        )
        .expect("parent window");
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0080),
            16,
            16,
            500,
            32,
            Some(hwnd),
            None::<HMENU>,
            Some(instance),
            None,
        )
        .expect("edit control");
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(edit));
        tx.send((hwnd.0 as usize, edit.0 as usize)).expect("send hwnd");
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 != 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    });

    let (parent, edit) = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("window opened");
    let parent = HWND(parent as *mut core::ffi::c_void);
    let edit = HWND(edit as *mut core::ffi::c_void);
    unsafe {
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let our_thread = GetCurrentThreadId();
        if foreground_thread != 0 && foreground_thread != our_thread {
            let _ = AttachThreadInput(foreground_thread, our_thread, true);
        }
        let _ = SetForegroundWindow(parent);
        let _ = SetFocus(Some(edit));
        if foreground_thread != 0 && foreground_thread != our_thread {
            let _ = AttachThreadInput(foreground_thread, our_thread, false);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    insert_text("cadence paste ok").expect("insert");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let mut buffer = [0u16; 256];
    let length = unsafe { GetWindowTextW(edit, &mut buffer) };
    let text = String::from_utf16_lossy(&buffer[..length.max(0) as usize]);
    unsafe {
        let _ = PostMessageW(Some(parent), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    let _ = window.join();
    assert!(
        text.to_lowercase().contains("cadence paste ok"),
        "edit control contained {text:?}"
    );
}
