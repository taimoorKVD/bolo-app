//! Test and smoke runs must not touch the real Bolo config or Groq key.
//!
//! `cargo test` always uses a temp directory created for this process.
//! A packaged app that is pointed at `BOLO_CONFIG_DIR` (the Windows smoke
//! test does this) stores its key beside that folder instead of in
//! Credential Manager.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// Where a normal launch keeps settings. Tests must not write here.
pub fn real_user_config_dir() -> PathBuf {
    config_root().join("com.bolo.dictation")
}

/// The pre-rename config folder. Still read at startup, never written by tests.
pub fn real_legacy_config_dir() -> PathBuf {
    config_root().join("com.cadence.dictation")
}

fn config_root() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Users\Default\AppData\Roaming"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from("/tmp/bolo-no-home"))
    }
}

/// One temp folder for this test process. Never the user profile.
pub fn isolated_config_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("bolo-test-config-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

pub fn is_inside_real_config(path: &Path) -> bool {
    within(path, &real_user_config_dir()) || within(path, &real_legacy_config_dir())
}

/// Tests return an error instead of creating or changing the real config.
pub fn refuse_real_config(path: &Path) -> Result<(), String> {
    if cfg!(test) && is_inside_real_config(path) {
        return Err("refusing to write the real Bolo config from a test".into());
    }
    Ok(())
}

fn within(path: &Path, root: &Path) -> bool {
    let path = norm(path);
    let root = norm(root);
    path == root || path.starts_with(&root)
}

fn norm(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_isolated_folder_is_not_the_real_config() {
        let isolated = isolated_config_dir();
        let real = real_user_config_dir();
        let legacy = real_legacy_config_dir();
        assert!(isolated.starts_with(std::env::temp_dir()));
        assert!(!is_inside_real_config(&isolated));
        assert!(is_inside_real_config(&real));
        assert!(is_inside_real_config(&real.join("settings.json")));
        assert!(is_inside_real_config(&legacy.join("history.json")));
        assert!(refuse_real_config(&real.join("settings.json")).is_err());
        assert!(refuse_real_config(&isolated.join("settings.json")).is_ok());
        assert_ne!(isolated, real);
        assert_ne!(isolated, legacy);
    }
}
