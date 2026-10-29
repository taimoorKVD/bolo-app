//! Login-time hygiene.
//!
//! Older Bolo/Cadence installs registered their own `HKCU\...\Run` entries.
//! Two entries pointing at two different `cadence.exe` copies race at login,
//! so the hotkey sometimes belongs to stale code.
//!
//! Scope is deliberately narrow: only values whose *name* is one this app
//! itself has ever written are considered, and only when they launch a
//! different cadence.exe. Other programs' Run entries are never touched.
//! Uses the registry API directly (no hidden `reg.exe` child processes).

use std::path::Path;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE,
};

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Value names earlier Bolo/Cadence builds wrote. `Bolo.com` (current) is
/// rewritten by apply_autostart and is never deleted here.
const LEGACY_RUN_NAMES: &[&str] = &["Bolo", "Cadence", "cadence"];

pub fn clean_stale_autostart(log_dir: &Path) {
    let Ok(current) = std::env::current_exe() else { return };
    let current = current.to_string_lossy().to_lowercase();
    unsafe {
        let mut key = HKEY::default();
        let sub = HSTRING::from(RUN_SUBKEY);
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), Some(0), KEY_QUERY_VALUE | KEY_SET_VALUE, &mut key).is_err() {
            return;
        }
        for name in LEGACY_RUN_NAMES {
            let Some(data) = read_string(key, name) else { continue };
            let lower = data.to_lowercase();
            if !lower.contains("cadence.exe") || lower.contains(&current) {
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

unsafe fn read_string(key: HKEY, name: &str) -> Option<String> {
    let hname = HSTRING::from(name);
    let mut size = 0u32;
    RegQueryValueExW(key, PCWSTR(hname.as_ptr()), None, None, None, Some(&mut size)).ok().ok()?;
    if size == 0 { return None; }
    let mut buf = vec![0u8; size as usize];
    RegQueryValueExW(key, PCWSTR(hname.as_ptr()), None, None, Some(buf.as_mut_ptr()), Some(&mut size)).ok().ok()?;
    let wide: Vec<u16> = buf[..size as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&wide).trim_end_matches('\0').to_string())
}
