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
