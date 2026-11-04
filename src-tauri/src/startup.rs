//! Login-time hygiene.
//!
//! Older Bolo/Cadence installs registered their own `HKCU\...\Run` entries.
//! Two entries pointing at two different copies race at login, so the hotkey
//! sometimes belongs to stale code.
//!
//! Scope is deliberately narrow: only values whose *name* is one this app
//! itself has ever written are considered, and only when they launch a
//! different Bolo binary (`bolo.exe`, or `cadence.exe` from 0.1.6 and earlier).
//! Other programs' Run entries are never touched.
//! The current product name `Bolo.com` is rewritten by `apply_autostart` on
//! every launch and by the NSIS post-install hook during an update.

#[cfg(windows)]
use std::path::Path;

/// Shipped executable. Task Manager, the installer, and the autostart path use this.
#[cfg_attr(not(windows), allow(dead_code))]
pub const SHIPPED_EXE: &str = "bolo.exe";
/// 0.1.6 and earlier. Still recognized so an update can drop the old Run target.
#[cfg_attr(not(windows), allow(dead_code))]
pub const LEGACY_EXE: &str = "cadence.exe";

/// True when a Run value launches one of our binaries, and not the one running now.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn legacy_run_is_stale(value: &str, current_exe: &str) -> bool {
    let lower = value.to_lowercase();
    let current = current_exe.to_lowercase();
    if current.is_empty() {
        return false;
    }
    let ours = lower.contains(LEGACY_EXE) || lower.contains(SHIPPED_EXE);
    ours && !lower.contains(&current)
}

#[cfg(windows)]
const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Value names earlier Bolo/Cadence builds wrote. `Bolo.com` (current) is
/// rewritten by apply_autostart and is never deleted here.
#[cfg(windows)]
const LEGACY_RUN_NAMES: &[&str] = &["Bolo", "Cadence", "cadence"];

#[cfg(windows)]
pub fn clean_stale_autostart(log_dir: &Path) {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
        KEY_QUERY_VALUE, KEY_SET_VALUE,
    };

    let Ok(current) = std::env::current_exe() else { return };
    let current = current.to_string_lossy().to_string();
    unsafe {
        let mut key = HKEY::default();
        let sub = HSTRING::from(RUN_SUBKEY);
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), Some(0), KEY_QUERY_VALUE | KEY_SET_VALUE, &mut key).is_err() {
            return;
        }
        for name in LEGACY_RUN_NAMES {
            let Some(data) = read_string(key, name) else { continue };
            if !legacy_run_is_stale(&data, &current) {
                continue;
            }
            let hname = HSTRING::from(*name);
            let removed = RegDeleteValueW(key, PCWSTR(hname.as_ptr())).is_ok();
            crate::logbuf::log_line(
                log_dir,
                "info",
                &format!("startup removed legacy autostart entry name={name} ok={removed} target={data}"),
            );
        }
        let _ = RegCloseKey(key);
    }
}

#[cfg(windows)]
unsafe fn read_string(key: windows::Win32::System::Registry::HKEY, name: &str) -> Option<String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Registry::RegQueryValueExW;

    let hname = HSTRING::from(name);
    let mut size = 0u32;
    RegQueryValueExW(key, PCWSTR(hname.as_ptr()), None, None, None, Some(&mut size)).ok().ok()?;
    if size == 0 { return None; }
    let mut buf = vec![0u8; size as usize];
    RegQueryValueExW(key, PCWSTR(hname.as_ptr()), None, None, Some(buf.as_mut_ptr()), Some(&mut size)).ok().ok()?;
    let wide: Vec<u16> = buf[..size as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&wide).trim_end_matches('\0').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_016_cadence_autostart_is_stale_once_bolo_exe_is_running() {
        let old = r"C:\Users\A\AppData\Local\Bolo.com\cadence.exe";
        let current = r"C:\Users\A\AppData\Local\Bolo.com\bolo.exe";
        assert!(legacy_run_is_stale(old, current));
        assert!(legacy_run_is_stale(r#""C:\Users\A\AppData\Local\Bolo.com\Cadence.exe""#, current));
        assert!(!legacy_run_is_stale(current, current));
        assert!(!legacy_run_is_stale(r"C:\Program Files\Other\helper.exe", current));
        assert!(!legacy_run_is_stale(old, ""));
        assert_eq!(SHIPPED_EXE, "bolo.exe");
        assert_eq!(LEGACY_EXE, "cadence.exe");
    }
}
