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
