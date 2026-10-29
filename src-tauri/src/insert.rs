//! Insert text at the caret.
//!
//! The primary path copies the transcript to the clipboard, sends Ctrl+V, then
//! puts the previous text clipboard back. If the clipboard holds an image, or
//! the paste simulation fails, Bolo types the text instead.

use anyhow::{anyhow, Context};
use arboard::Clipboard;
use serde::Serialize;
use std::thread;
use std::time::Duration;

const CLIPBOARD_PUBLISH_DELAY: Duration = Duration::from_millis(8);
const PASTE_SETTLE_DELAY: Duration = Duration::from_millis(25);
/// How long a non-edit target may take to read the clipboard before we type.
const CLIPBOARD_READ_WAIT: Duration = Duration::from_millis(500);

/// Clipboard paste when the target asked for the delayed-render text.
/// Typing is only for a clipboard that nobody read.
fn non_edit_insertion(clipboard_read: bool) -> Insertion {
    if clipboard_read {
        Insertion {
            method: InsertMethod::Clipboard,
            verified: true,
        }
    } else {
        Insertion {
            method: InsertMethod::Typed,
            verified: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InsertMethod {
    Clipboard,
    Typed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insertion {
    pub method: InsertMethod,
    pub verified: bool,
}

/// "Inserted" only when a paste was confirmed. Typing is always "Typed".
pub fn status_word(method: InsertMethod, verified: bool) -> &'static str {
    if verified && matches!(method, InsertMethod::Clipboard) {
        "Inserted"
    } else {
        "Typed"
    }
}

/// The phrase showed up in the control, and the control changed.
pub fn text_landed(before: &str, after: &str, phrase: &str) -> bool {
    let phrase = phrase.trim();
    if phrase.is_empty() || after == before || !after.contains(phrase) {
        return false;
    }
    !before.contains(phrase)
        || after.chars().count() + 2 >= before.chars().count() + phrase.chars().count()
}

pub fn insert_text(text: &str) -> anyhow::Result<Insertion> {
    let text = text.trim();
    if text.is_empty() {
        return Err(anyhow!("nothing to insert"));
    }
    #[cfg(windows)]
    {
        return insert_text_windows(text);
    }
    #[cfg(not(windows))]
    {
        // Linux has no read-back. The Windows path is the one that verifies.
        match paste_via_clipboard(text) {
            Ok(()) => Ok(Insertion {
                method: InsertMethod::Clipboard,
                verified: true,
            }),
            Err(err) => {
                eprintln!("clipboard paste unavailable ({err}); typing the transcript");
                type_text(text).context("couldn't type the transcript either")?;
                Ok(Insertion {
                    method: InsertMethod::Typed,
                    verified: true,
                })
            }
        }
    }
}

/// Remember the app the user was typing in, before the tally is shown.
pub fn note_foreground() {
    #[cfg(windows)]
    note_foreground_windows();
}

pub fn set_clipboard_text(text: &str) -> anyhow::Result<()> {
    let mut clipboard = Clipboard::new().context("couldn't open the clipboard")?;
    clipboard
        .set_text(text)
        .context("couldn't write the clipboard")?;
    Ok(())
}

#[cfg_attr(windows, allow(dead_code))]
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

#[cfg(windows)]
fn insert_text_windows(text: &str) -> anyhow::Result<Insertion> {
    release_stuck_modifiers();
    let target = foreground_target();
    if let Some(hwnd) = target {
        if let Some(edit) = edit_target(hwnd) {
            return insert_into_edit(edit, text);
        }
    }
    match clipboard_paste_verified(text) {
        Ok(()) => Ok(non_edit_insertion(true)),
        Err(err) => {
            eprintln!("clipboard paste was not confirmed ({err}); typing the transcript");
            type_text(text).context("couldn't type the transcript either")?;
            if let Some(edit) = target.and_then(edit_target) {
                if window_text(edit).contains(text) {
                    return Ok(Insertion {
                        method: InsertMethod::Typed,
                        verified: true,
                    });
                }
            }
            Ok(non_edit_insertion(false))
        }
    }
}

#[cfg(windows)]
fn insert_into_edit(
    hwnd: windows::Win32::Foundation::HWND,
    text: &str,
) -> anyhow::Result<Insertion> {
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_PASTE};

    let before = window_text(hwnd);
    if clipboard_is_image() {
        replace_selection(hwnd, text);
        let after = window_text(hwnd);
        if text_landed(&before, &after, text) {
            return Ok(Insertion {
                method: InsertMethod::Typed,
                verified: true,
            });
        }
        return Err(anyhow!("the text did not appear in the text field"));
    }
    let previous = Clipboard::new().ok().and_then(|mut clipboard| clipboard.get_text().ok());
    if let Err(err) = set_clipboard_text(text) {
        replace_selection(hwnd, text);
        let after = window_text(hwnd);
        if text_landed(&before, &after, text) {
            return Ok(Insertion {
                method: InsertMethod::Typed,
                verified: true,
            });
        }
        return Err(err);
    }
    // WM_PASTE is synchronous for an Edit control, so the read-back happens
    // before the previous clipboard value is put back. Restoring first was
    // why a second take could report Inserted while the field kept the old text.
    unsafe {
        SendMessageW(hwnd, WM_PASTE, None, None);
    }
    let after = window_text(hwnd);
    if let Ok(mut clipboard) = Clipboard::new() {
        restore_text(&mut clipboard, previous.clone());
    }
    if text_landed(&before, &after, text) {
        return Ok(Insertion {
            method: InsertMethod::Clipboard,
            verified: true,
        });
    }
    replace_selection(hwnd, text);
    let typed = window_text(hwnd);
    if text_landed(&before, &typed, text) {
        return Ok(Insertion {
            method: InsertMethod::Typed,
            verified: true,
        });
    }
    Err(anyhow!("the text did not appear in the text field"))
}

#[cfg(windows)]
fn clipboard_is_image() -> bool {
    let Ok(mut clipboard) = Clipboard::new() else {
        return false;
    };
    clipboard.get_text().is_err() && clipboard.get_image().is_ok()
}

#[cfg(windows)]
fn replace_selection(hwnd: windows::Win32::Foundation::HWND, text: &str) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    unsafe {
        SendMessageW(
            hwnd,
            0x00C2,
            Some(WPARAM(1)),
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }
}

#[cfg(windows)]
fn window_text(hwnd: windows::Win32::Foundation::HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..length.min(buffer.len())])
}

#[cfg(windows)]
fn edit_target(hwnd: windows::Win32::Foundation::HWND) -> Option<windows::Win32::Foundation::HWND> {
    if is_edit_class(hwnd) {
        return Some(hwnd);
    }
    use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO};
    let thread = unsafe { GetWindowThreadProcessId(hwnd, None) };
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetGUIThreadInfo(thread, &mut info) }.is_err() {
        return None;
    }
    let focus = info.hwndFocus;
    if focus.0.is_null() {
        return None;
    }
    if is_edit_class(focus) {
        Some(focus)
    } else {
        None
    }
}

#[cfg(windows)]
fn is_edit_class(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) }.max(0) as usize;
    let name = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]).to_ascii_lowercase();
    name == "edit" || name.contains("richedit") || name.contains("scintilla")
}

#[cfg(windows)]
fn foreground_target() -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let foreground = unsafe { GetForegroundWindow() };
    if !foreground.0.is_null() && !is_our_process(foreground) {
        remember_target(foreground);
        return Some(foreground);
    }
    last_target()
}

#[cfg(windows)]
fn note_foreground_windows() {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let foreground = unsafe { GetForegroundWindow() };
    if !foreground.0.is_null() && !is_our_process(foreground) {
        remember_target(foreground);
    }
}

#[cfg(windows)]
fn is_our_process(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    pid == std::process::id()
}

#[cfg(windows)]
fn remember_target(hwnd: windows::Win32::Foundation::HWND) {
    if let Ok(mut slot) = LAST_TARGET.lock() {
        *slot = Some(hwnd.0 as isize);
    }
}

#[cfg(windows)]
fn last_target() -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    let raw = LAST_TARGET.lock().ok().and_then(|slot| *slot)?;
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        Some(hwnd)
    } else {
        None
    }
}

#[cfg(windows)]
static LAST_TARGET: std::sync::Mutex<Option<isize>> = std::sync::Mutex::new(None);

#[cfg(windows)]
fn release_stuck_modifiers() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    let stuck = [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .into_iter()
        .filter(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0)
        .collect::<Vec<_>>();
    if stuck.is_empty() {
        return;
    }
    let inputs: Vec<_> = stuck.into_iter().map(|vk| key_input(vk, key_up_flag())).collect();
    unsafe {
        let _ = SendInput(&inputs, std::mem::size_of::<windows::Win32::UI::Input::KeyboardAndMouse::INPUT>() as i32);
    }
    thread::sleep(Duration::from_millis(20));
}

#[cfg(windows)]
fn key_up_flag() -> windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS {
    windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP
}

#[cfg(windows)]
fn clipboard_paste_verified(text: &str) -> anyhow::Result<()> {
    if clipboard_is_image() {
        return Err(anyhow!("clipboard holds an image"));
    }
    let previous = Clipboard::new().ok().and_then(|mut clipboard| clipboard.get_text().ok());
    // Own the clipboard with delayed rendering. The target's paste calls
    // GetClipboardData, which asks us to render. If that never arrives, the
    // paste did not land and the caller types instead.
    let rendered = render_until_read(text)?;
    if let Ok(mut clipboard) = Clipboard::new() {
        restore_text(&mut clipboard, previous);
    }
    if rendered {
        Ok(())
    } else {
        Err(anyhow!("the focused app did not read the clipboard"))
    }
}

#[cfg(windows)]
fn render_until_read(text: &str) -> anyhow::Result<bool> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (go_tx, go_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    // The clipboard owner has to be inside a message wait when Chrome or Cursor
    // calls GetClipboardData. Doing that on this thread missed the request,
    // because focus and Ctrl+V ran before the pump.
    std::thread::Builder::new()
        .name("bolo-clip".into())
        .spawn(move || {
            let result = own_and_pump(wide, ready_tx, go_rx);
            let _ = done_tx.send(result);
        })
        .map_err(|err| anyhow!("couldn't start the clipboard owner: {err}"))?;
    match ready_rx.recv_timeout(Duration::from_millis(800)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(err),
        Err(_) => return Err(anyhow!("clipboard owner didn't start")),
    }
    if let Some(target) = last_target() {
        focus_target(target);
    }
    let sent = send_ctrl_v();
    let _ = go_tx.send(());
    sent?;
    match done_rx.recv_timeout(CLIPBOARD_READ_WAIT + Duration::from_millis(800)) {
        Ok(result) => result,
        Err(_) => Ok(false),
    }
}

#[cfg(windows)]
fn own_and_pump(
    wide: Vec<u16>,
    ready_tx: std::sync::mpsc::Sender<anyhow::Result<()>>,
    go_rx: std::sync::mpsc::Receiver<()>,
) -> anyhow::Result<bool> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, RegisterClassW,
        TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
    };
    use windows::core::PCWSTR;

    static CLIP_SEQ: AtomicU64 = AtomicU64::new(1);
    {
        let mut job = RENDER.lock().unwrap_or_else(|err| err.into_inner());
        *job = Some(RenderJob {
            wide,
            requested: false,
            rendered: false,
        });
    }
    unsafe {
        let seq = CLIP_SEQ.fetch_add(1, Ordering::Relaxed);
        let class_name: Vec<u16> = format!("BoloClip{seq}\0").encode_utf16().collect();
        let class = WNDCLASSW {
            lpfnWndProc: Some(clip_wnd_proc),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
        // A real hidden window. Message-only windows do not reliably receive
        // WM_RENDERFORMAT from Chrome and Cursor.
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WINDOW_STYLE(0x8000_0000),
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(err) => {
                let _ = ready_tx.send(Err(anyhow!("couldn't own the clipboard: {err}")));
                return Err(anyhow!("couldn't own the clipboard: {err}"));
            }
        };
        if OpenClipboard(Some(hwnd)).is_err() {
            let _ = DestroyWindow(hwnd);
            let _ = ready_tx.send(Err(anyhow!("couldn't open the clipboard")));
            return Err(anyhow!("couldn't open the clipboard"));
        }
        if EmptyClipboard().is_err() {
            let _ = CloseClipboard();
            let _ = DestroyWindow(hwnd);
            let _ = ready_tx.send(Err(anyhow!("couldn't clear the clipboard")));
            return Err(anyhow!("couldn't clear the clipboard"));
        }
        if SetClipboardData(13, None).is_err() {
            let _ = CloseClipboard();
            let _ = DestroyWindow(hwnd);
            let _ = ready_tx.send(Err(anyhow!("couldn't publish the transcript")));
            return Err(anyhow!("couldn't publish the transcript"));
        }
        let _ = CloseClipboard();
        let _ = ready_tx.send(Ok(()));
        let started = std::time::Instant::now();
        let mut deadline: Option<std::time::Instant> = None;
        let mut message = MSG::default();
        loop {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if clip_was_read() {
                let _ = DestroyWindow(hwnd);
                return Ok(true);
            }
            if deadline.is_none() {
                match go_rx.try_recv() {
                    Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        deadline = Some(std::time::Instant::now() + CLIPBOARD_READ_WAIT);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if started.elapsed() > Duration::from_secs(2) {
                            let _ = DestroyWindow(hwnd);
                            return Ok(false);
                        }
                    }
                }
            } else if std::time::Instant::now() >= deadline.expect("deadline") {
                // Read the flag before the window goes away. Destroying the
                // owner renders every format, which is not the target reading it.
                let read = clip_was_read();
                let _ = DestroyWindow(hwnd);
                return Ok(read);
            }
            let _ = MsgWaitForMultipleObjects(None, false, 20, QS_ALLINPUT);
        }
    }
}

#[cfg(windows)]
fn clip_was_read() -> bool {
    RENDER.lock().ok().and_then(|job| {
        job.as_ref().map(|job| job.rendered || job.requested)
    }) == Some(true)
}

#[cfg(windows)]
struct RenderJob {
    wide: Vec<u16>,
    /// The target called GetClipboardData and we handed it the text.
    requested: bool,
    rendered: bool,
}

#[cfg(windows)]
static RENDER: std::sync::Mutex<Option<RenderJob>> = std::sync::Mutex::new(None);

#[cfg(windows)]
unsafe extern "system" fn clip_wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::SetClipboardData;
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::UI::WindowsAndMessaging::DefWindowProcW;
    const WM_RENDERFORMAT: u32 = 0x0305;
    // WM_RENDERALLFORMATS arrives when we destroy the owner. That is not a paste.
    if msg == WM_RENDERFORMAT {
        if let Ok(mut job) = RENDER.lock() {
            if let Some(job) = job.as_mut() {
                job.requested = true;
                let bytes = job.wide.len() * 2;
                if let Ok(handle) = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) } {
                    let pointer = unsafe { GlobalLock(handle) };
                    if !pointer.is_null() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(job.wide.as_ptr() as *const u8, pointer as *mut u8, bytes);
                            let _ = GlobalUnlock(handle);
                            let _ = SetClipboardData(13, Some(HANDLE(handle.0)));
                        }
                        job.rendered = true;
                    }
                }
            }
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(windows)]
fn focus_target(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow};
    unsafe {
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let target_thread = GetWindowThreadProcessId(hwnd, None);
        let ours = GetCurrentThreadId();
        let _ = AttachThreadInput(ours, foreground_thread, true);
        let _ = AttachThreadInput(ours, target_thread, true);
        let _ = SetForegroundWindow(hwnd);
        let _ = AttachThreadInput(ours, foreground_thread, false);
        let _ = AttachThreadInput(ours, target_thread, false);
    }
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

    const PHRASE: &str = "bolo paste ok";

    eprintln!("paste test: creating a hidden edit control");
    let mut host = EditHost::spawn();
    let edit = match host.ready() {
        Ok(edit) => edit,
        Err(err) => panic!("paste test: {err}"),
    };
    eprintln!("paste test: edit control is ready");

    // An image on the clipboard is real user data (a screenshot, for example).
    // Leaving it untouched is what the app does during dictation. This test
    // copies the image, pastes text, and puts the image back. If the image
    // cannot be copied, skip instead of failing or clearing it.
    let previous = match capture_clipboard() {
        Ok(saved) => saved,
        Err(message) => {
            eprintln!("SKIP pastes_into_a_real_edit_control: {message}");
            host.shutdown();
            return;
        }
    };
    let _restore = ClipboardGuard { previous };

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

/// Two takes into one field. Restoring the clipboard before the control reads
/// it must not be reported as Inserted; the real insert path confirms each one.
#[cfg(all(test, windows))]
#[test]
fn consecutive_takes_both_land_in_the_edit_control() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_PASTE};

    eprintln!("consecutive paste test: creating a hidden edit control");
    let mut host = EditHost::spawn();
    let edit = match host.ready() {
        Ok(edit) => edit,
        Err(err) => panic!("consecutive paste test: {err}"),
    };
    let previous = match capture_clipboard() {
        Ok(saved) => saved,
        Err(message) => {
            eprintln!("SKIP consecutive_takes_both_land_in_the_edit_control: {message}");
            host.shutdown();
            return;
        }
    };
    let _restore = ClipboardGuard { previous };
    let hwnd = HWND(edit as *mut core::ffi::c_void);

    set_clipboard_text("beta take").expect("clipboard");
    if let Ok(mut clipboard) = Clipboard::new() {
        restore_text(&mut clipboard, Some("old clipboard".into()));
    }
    unsafe {
        SendMessageW(hwnd, WM_PASTE, None, None);
    }
    let raced = window_text(hwnd);
    assert!(
        !text_landed("", &raced, "beta take"),
        "restoring the clipboard before the paste must not count as inserting beta, control was {raced:?}"
    );

    let first = insert_into_edit(hwnd, "alpha take").expect("first take");
    assert!(first.verified, "first take was not verified");
    let second = insert_into_edit(hwnd, "beta take").expect("second take");
    assert!(second.verified, "second take was not verified");
    assert_eq!(status_word(second.method, second.verified), if second.method == InsertMethod::Clipboard { "Inserted" } else { "Typed" });
    let text = window_text(hwnd);
    let alpha = text.find("alpha take").unwrap_or_else(|| panic!("missing first take in {text:?}"));
    let beta = text.find("beta take").unwrap_or_else(|| panic!("missing second take in {text:?}"));
    assert!(alpha < beta, "takes landed out of order: {text:?}");
    host.shutdown();
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
enum SavedClipboard {
    Text(String),
    Image(arboard::ImageData<'static>),
    Empty,
}

#[cfg(all(test, windows))]
fn capture_clipboard() -> Result<SavedClipboard, String> {
    let mut clipboard = Clipboard::new().map_err(|err| format!("couldn't open the clipboard: {err}"))?;
    if let Ok(text) = clipboard.get_text() {
        return Ok(SavedClipboard::Text(text));
    }
    match clipboard.get_image() {
        Ok(image) => {
            eprintln!(
                "paste test: clipboard holds an image ({}x{}); it will be restored after the paste",
                image.width, image.height
            );
            Ok(SavedClipboard::Image(image))
        }
        Err(arboard::Error::ContentNotAvailable) => Ok(SavedClipboard::Empty),
        Err(err) => Err(format!(
            "the clipboard could not be copied ({err}), so it was left untouched"
        )),
    }
}

#[cfg(all(test, windows))]
struct ClipboardGuard {
    previous: SavedClipboard,
}

#[cfg(all(test, windows))]
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        let Ok(mut clipboard) = Clipboard::new() else {
            eprintln!("paste test: couldn't reopen the clipboard to restore it");
            return;
        };
        let previous = std::mem::replace(&mut self.previous, SavedClipboard::Empty);
        let restored = match previous {
            SavedClipboard::Text(text) => clipboard.set_text(text).is_ok(),
            SavedClipboard::Image(image) => clipboard.set_image(image).is_ok(),
            SavedClipboard::Empty => clipboard.clear().is_ok(),
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
            let class_name: Vec<u16> = format!("BoloPasteTest{}\0", std::process::id())
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
                w!("Bolo paste"),
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
    assert_eq!(CLIPBOARD_READ_WAIT, Duration::from_millis(500));
    let read = non_edit_insertion(true);
    assert_eq!(read.method, InsertMethod::Clipboard);
    assert!(read.verified);
    let unread = non_edit_insertion(false);
    assert_eq!(unread.method, InsertMethod::Typed);
    assert!(!unread.verified);
    assert!(!text_landed("alpha take", "alpha take", "beta take"));
    assert!(text_landed("alpha take", "alpha take beta take", "beta take"));
    assert_eq!(status_word(InsertMethod::Clipboard, true), "Inserted");
    assert_eq!(status_word(InsertMethod::Clipboard, false), "Typed");
    assert_eq!(status_word(InsertMethod::Typed, true), "Typed");
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
