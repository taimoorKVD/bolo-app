//! Login-time hygiene.
//!
//! Older Bolo/Cadence installs registered their own `HKCU\...\Run` entries.
//! Two entries pointing at two different `cadence.exe` copies race at login:
//! the single-instance plugin keeps whichever starts first (possibly an old
//! build) and the other exits, so the hotkey sometimes belongs to stale code.
//! This removes Run values that launch a different cadence.exe than this one.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn clean_stale_autostart(log_dir: &Path) {
    let Ok(current) = std::env::current_exe() else { return };
    let current = current.to_string_lossy().to_lowercase();
    let Ok(output) = Command::new("reg")
        .args(["query", RUN_KEY])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let Some((name, data)) = parse_value(line) else { continue };
        let lower = data.to_lowercase();
        if !lower.contains("cadence.exe") || lower.contains(&current) {
            continue;
        }
        let removed = Command::new("reg")
            .args(["delete", RUN_KEY, "/v", name, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        crate::logbuf::log_line(
            log_dir,
            "info",
            &format!("startup removed stale autostart entry name={name} ok={removed} target={data}"),
        );
    }
}

/// `    Name    REG_SZ    C:\path\cadence.exe` -> ("Name", "C:\path\cadence.exe")
fn parse_value(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    for kind in ["REG_SZ", "REG_EXPAND_SZ"] {
        let marker = format!("    {kind}    ");
        if let Some(at) = line.find(&marker) {
            return Some((line[..at].trim(), line[at + marker.len()..].trim()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_reg_query_lines() {
        let line = r"    Bolo    REG_SZ    C:\Users\x\AppData\Local\Bolo\cadence.exe";
        assert_eq!(
            super::parse_value(line),
            Some(("Bolo", r"C:\Users\x\AppData\Local\Bolo\cadence.exe"))
        );
    }
}
