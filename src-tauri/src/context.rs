//! Foreground-app tone. Cleanup follows the app the user is typing into.
//! Editors and terminals skip the model entirely.

use crate::cleanup::Tone;

pub fn classify(exe: &str, title: &str) -> Tone {
    let stem = exe_stem(exe);
    if VERBATIM.iter().any(|name| stem == *name) {
        return Tone::Verbatim;
    }
    if CASUAL.iter().any(|name| stem.contains(name)) {
        return Tone::Casual;
    }
    if FORMAL_EXE.iter().any(|name| stem.contains(name)) {
        return Tone::Formal;
    }
    let title = title.to_ascii_lowercase();
    let browser = matches!(stem.as_str(), "chrome" | "msedge" | "firefox" | "brave");
    if browser && (title.contains("gmail") || title.contains("outlook")) {
        return Tone::Formal;
    }
    Tone::Plain
}

pub fn foreground_tone() -> Tone {
    #[cfg(windows)]
    {
        windows_foreground()
    }
    #[cfg(not(windows))]
    {
        Tone::Plain
    }
}

/// Short name of the app a take went into, for the overlay's
/// "Inserted into Notepad". None when the window has no usable name.
pub fn app_label(target: Option<isize>) -> Option<String> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        let hwnd = windows::Win32::Foundation::HWND(target? as *mut core::ffi::c_void);
        let title = unsafe { window_title(hwnd) };
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        let exe = if pid == 0 { String::new() } else { process_image(pid) };
        return label_from(&exe, &title);
    }
    #[cfg(not(windows))]
    {
        let _ = target;
        None
    }
}

/// Known apps by exe; otherwise the title's last " - " part ("notes.txt - Notepad").
pub fn label_from(exe: &str, title: &str) -> Option<String> {
    let stem = exe_stem(exe);
    let known = match stem.as_str() {
        "notepad" => Some("Notepad"),
        "winword" => Some("Word"),
        "excel" => Some("Excel"),
        "outlook" | "olk" => Some("Outlook"),
        "powerpnt" => Some("PowerPoint"),
        "onenote" => Some("OneNote"),
        "chrome" => Some("Chrome"),
        "msedge" => Some("Edge"),
        "firefox" => Some("Firefox"),
        "brave" => Some("Brave"),
        "whatsapp" | "whatsapp.root" => Some("WhatsApp"),
        "slack" => Some("Slack"),
        "discord" => Some("Discord"),
        "ms-teams" | "teams" => Some("Teams"),
        "telegram" => Some("Telegram"),
        "code" | "code-insiders" => Some("VS Code"),
        "cursor" => Some("Cursor"),
        "windowsterminal" | "wt" => Some("Terminal"),
        "notepad++" => Some("Notepad++"),
        _ => None,
    };
    if let Some(name) = known {
        return Some(name.to_string());
    }
    let tail = title.rsplit(" - ").next().unwrap_or("").trim();
    let count = tail.chars().count();
    if title.contains(" - ") && (1..=24).contains(&count) {
        return Some(tail.to_string());
    }
    None
}

fn exe_stem(exe: &str) -> String {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    let lower = file.to_ascii_lowercase();
    lower.trim_end_matches(".exe").to_string()
}

const VERBATIM: &[&str] = &[
    "code",
    "code-insiders",
    "cursor",
    "windowsterminal",
    "wt",
    "cmd",
    "powershell",
    "pwsh",
    "devenv",
    "idea64",
    "pycharm",
    "webstorm",
    "rider",
    "rustrover",
    "sublime_text",
    "notepad++",
];

const CASUAL: &[&str] = &[
    "slack",
    "discord",
    "teams",
    "whatsapp",
    "telegram",
    "messenger",
    "signal",
];

const FORMAL_EXE: &[&str] = &["winword", "outlook", "powerpnt", "onenote"];

#[cfg(windows)]
fn windows_foreground() -> Tone {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return Tone::Plain;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let title = window_title(hwnd);
        let exe = if pid == 0 {
            String::new()
        } else {
            process_image(pid)
        };
        classify(&exe, &title)
    }
}

#[cfg(windows)]
unsafe fn window_title(hwnd: windows::Win32::Foundation::HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let mut buf = [0u16; 512];
    let count = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if count <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..count as usize])
    }
}

#[cfg(windows)]
fn process_image(pid: u32) -> String {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows_core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        if ok.is_err() || len == 0 {
            return String::new();
        }
        let n = (len as usize).min(buf.len());
        String::from_utf16_lossy(&buf[..n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_names_the_target_app() {
        assert_eq!(label_from(r"C:\Windows\notepad.exe", "Meeting notes.txt - Notepad").as_deref(), Some("Notepad"));
        assert_eq!(label_from(r"C:\x\WINWORD.EXE", "Doc1 - Word").as_deref(), Some("Word"));
        assert_eq!(label_from(r"C:\x\foo.exe", "Report - Foo Editor").as_deref(), Some("Foo Editor"));
        assert_eq!(label_from(r"C:\x\foo.exe", "Untitled"), None);
        assert_eq!(label_from("", ""), None);
    }

    #[test]
    fn tone_follows_the_foreground_app() {
        assert_eq!(classify(r"C:\Program Files\Slack\slack.exe", "general"), Tone::Casual);
        assert_eq!(classify("Discord.exe", ""), Tone::Casual);
        assert_eq!(classify("WINWORD.EXE", "Notes"), Tone::Formal);
        assert_eq!(classify("OUTLOOK.EXE", "Inbox"), Tone::Formal);
        assert_eq!(
            classify(r"C:\Program Files\Google\Chrome\Application\chrome.exe", "Gmail - Inbox"),
            Tone::Formal
        );
        assert_eq!(classify("chrome.exe", "GitHub"), Tone::Plain);
        assert_eq!(classify(r"C:\Users\a\AppData\Local\Programs\cursor\Cursor.exe", "main.rs"), Tone::Verbatim);
        assert_eq!(classify("Code.exe", "dictation.rs"), Tone::Verbatim);
        assert_eq!(classify("WindowsTerminal.exe", "pwsh"), Tone::Verbatim);
        assert_eq!(classify("powershell.exe", ""), Tone::Verbatim);
        assert_eq!(classify("wt.exe", ""), Tone::Verbatim);
        assert_eq!(classify("notepad.exe", "todo"), Tone::Plain);
    }
}
