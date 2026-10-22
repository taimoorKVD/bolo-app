//! Settings and transcription history on disk.
//!
//! The API key is intentionally not a field here. It lives in the OS credential store.

use crate::hotkey::{validate_hotkey, Hotkey};
use crate::transcribe::{normalize_language, normalize_model, WHISPER_TURBO};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_HISTORY: usize = 50;
const MAX_DICTIONARY: usize = 200;
const MAX_TERM_CHARS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub hotkey: Hotkey,
    pub microphone: Option<String>,
    pub model: String,
    pub cleanup_enabled: bool,
    pub language: String,
    pub dictionary: Vec<String>,
    pub launch_at_startup: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: Hotkey::default(),
            microphone: None,
            model: WHISPER_TURBO.to_string(),
            cleanup_enabled: false,
            language: "auto".into(),
            dictionary: Vec::new(),
            launch_at_startup: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub text: String,
    pub created_unix_ms: u64,
    pub model: String,
    pub cleaned: bool,
    pub duration_ms: u64,
}

pub fn prepare_settings(mut settings: Settings) -> Settings {
    settings.model = settings.model.trim().to_string();
    settings.language = settings.language.trim().to_ascii_lowercase();
    if settings.language.is_empty() {
        settings.language = "auto".into();
    }
    if let Some(name) = settings.microphone {
        let name = name.trim().to_string();
        settings.microphone = if name.is_empty() { None } else { Some(name) };
    }
    if let Some(key) = settings.hotkey.key {
        let key = key.trim().to_ascii_lowercase();
        settings.hotkey.key = if key.is_empty() { None } else { Some(key) };
    }
    settings.dictionary = settings
        .dictionary
        .into_iter()
        .map(|term| term.trim().to_string())
        .filter(|term| !term.is_empty())
        .collect();
    settings
}

pub fn validate_settings(settings: &Settings) -> Result<(), String> {
    validate_hotkey(&settings.hotkey)?;
    if normalize_model(&settings.model).is_none() {
        return Err("Choose Whisper Large v3 Turbo or Whisper Large v3.".into());
    }
    if settings.language != "auto" && normalize_language(&settings.language).is_none() {
        return Err("Language should be \"auto\" or a 2–3 letter code like \"en\".".into());
    }
    if settings.dictionary.len() > MAX_DICTIONARY {
        return Err(format!("The dictionary is limited to {MAX_DICTIONARY} entries."));
    }
    if settings.dictionary.iter().any(|term| term.chars().count() > MAX_TERM_CHARS) {
        return Err(format!(
            "Each dictionary entry must be {MAX_TERM_CHARS} characters or fewer."
        ));
    }
    if let Some(name) = &settings.microphone {
        if name.chars().count() > 200 {
            return Err("That microphone name is too long.".into());
        }
    }
    Ok(())
}

pub fn normalize_settings(mut settings: Settings) -> Settings {
    settings.model = normalize_model(&settings.model)
        .unwrap_or(WHISPER_TURBO)
        .to_string();
    if normalize_language(&settings.language).is_none() {
        settings.language = "auto".into();
    }
    if validate_hotkey(&settings.hotkey).is_err() {
        settings.hotkey = Hotkey::default();
    }
    settings.dictionary = settings
        .dictionary
        .into_iter()
        .map(|term| term.trim().to_string())
        .filter(|term| !term.is_empty())
        .take(MAX_DICTIONARY)
        .map(|term| term.chars().take(MAX_TERM_CHARS).collect())
        .collect();
    if let Some(name) = settings.microphone.as_deref() {
        if name.trim().is_empty() {
            settings.microphone = None;
        }
    }
    settings
}

pub fn load_settings(dir: &Path) -> Settings {
    let path = settings_path(dir);
    let Ok(bytes) = fs::read(&path) else {
        return Settings::default();
    };
    match serde_json::from_slice::<Settings>(&bytes) {
        Ok(settings) => normalize_settings(settings),
        Err(err) => {
            crate::logbuf::log_line(dir, "error", &format!("settings file ignored: {err}"));
            Settings::default()
        }
    }
}

pub fn save_settings(dir: &Path, settings: &Settings) -> Result<(), String> {
    validate_settings(settings)?;
    fs::create_dir_all(dir).map_err(|err| format!("couldn't create the config folder: {err}"))?;
    write_atomic(&settings_path(dir), settings)
        .map_err(|err| format!("couldn't save settings: {err}"))
}

pub fn load_history(dir: &Path) -> Vec<HistoryEntry> {
    let Ok(bytes) = fs::read(history_path(dir)) else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<HistoryEntry>>(&bytes).unwrap_or_default()
}

pub fn push_history(entries: &mut Vec<HistoryEntry>, mut entry: HistoryEntry) {
    if entry.text.chars().count() > 20_000 {
        entry.text = entry.text.chars().take(20_000).collect();
        entry.text.push('…');
    }
    entries.insert(0, entry);
    entries.truncate(MAX_HISTORY);
}

pub fn save_history(dir: &Path, entries: &[HistoryEntry]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|err| format!("couldn't create the config folder: {err}"))?;
    write_atomic(&history_path(dir), entries).map_err(|err| format!("couldn't save history: {err}"))
}

pub fn new_history_id() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{millis:x}-{n:x}")
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn settings_path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

fn history_path(dir: &Path) -> PathBuf {
    dir.join("history.json")
}

fn write_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let parsed: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, Settings::default());
        assert!(parsed.microphone.is_none());
        assert!(!parsed.cleanup_enabled);
        assert_eq!(parsed.model, WHISPER_TURBO);
        let blank: Settings = serde_json::from_str(r#"{"microphone":"   "}"#).unwrap();
        assert!(normalize_settings(blank).microphone.is_none());
    }

    #[test]
    fn rejects_unknown_model_and_lone_shift() {
        let mut settings = Settings::default();
        settings.model = "whisper-1".into();
        assert!(validate_settings(&settings).is_err());
        settings.model = WHISPER_TURBO.into();
        settings.hotkey = Hotkey {
            ctrl: false,
            alt: false,
            shift: true,
            win: false,
            key: None,
        };
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn history_keeps_the_newest_fifty() {
        let mut entries = Vec::new();
        for i in 0..60 {
            push_history(
                &mut entries,
                HistoryEntry {
                    id: i.to_string(),
                    text: format!("line {i}"),
                    created_unix_ms: i,
                    model: WHISPER_TURBO.into(),
                    cleaned: false,
                    duration_ms: 10,
                },
            );
        }
        assert_eq!(entries.len(), 50);
        assert_eq!(entries[0].id, "59");
        assert_eq!(entries.last().unwrap().id, "10");
    }

    #[test]
    fn roundtrip_on_disk() {
        let dir = std::env::temp_dir().join(format!("cadence-test-{}", now_ms()));
        let _ = fs::remove_dir_all(&dir);
        let mut settings = Settings::default();
        settings.dictionary = vec!["Cadence".into(), "Tauri".into()];
        settings.language = "en".into();
        save_settings(&dir, &settings).unwrap();
        let loaded = load_settings(&dir);
        assert_eq!(loaded.dictionary, settings.dictionary);
        assert_eq!(loaded.language, "en");
        let mut history = Vec::new();
        push_history(
            &mut history,
            HistoryEntry {
                id: "a".into(),
                text: "hello".into(),
                created_unix_ms: 1,
                model: WHISPER_TURBO.into(),
                cleaned: false,
                duration_ms: 400,
            },
        );
        save_history(&dir, &history).unwrap();
        assert_eq!(load_history(&dir)[0].text, "hello");
        let _ = fs::remove_dir_all(&dir);
    }
}
