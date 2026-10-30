//! Settings and transcription history on disk.
//!
//! The API key is intentionally not a field here. It lives in the OS credential store.

use crate::cleanup::CLEANUP_MODEL;
use crate::hotkey::{validate_hotkey, Hotkey};
use crate::offline;
use crate::transcribe::{normalize_language, normalize_model, normalize_output, WHISPER_TURBO};
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
    /// Local transcript history. Missing in older files, so it stays on.
    #[serde(default = "history_on")]
    pub history_enabled: bool,
    /// Inserts the previous transcript again. Missing files keep Ctrl+Win+V.
    #[serde(default = "paste_again_hotkey")]
    pub paste_hotkey: Hotkey,
    /// `base.en`, `small.en`, or `off`. Missing files download base.en on demand.
    #[serde(default = "default_offline_model")]
    pub offline_model: String,
    /// `english` translates into English. `same` keeps the spoken language.
    /// Missing files translate.
    #[serde(default = "output_english")]
    pub output: String,
    /// Paste the transcript with no cleanup and no filler removal.
    #[serde(default)]
    pub verbatim: bool,
    /// Groq chat model for cleanup and mixed-language translation.
    /// A 404 tries the next built-in model.
    #[serde(default = "default_cleanup_model")]
    pub cleanup_model: String,
    /// Overlay character id. Missing or unknown values fall back to `classic`.
    #[serde(default = "default_character")]
    pub character: String,
    /// Sound pack id, `match` or `off`. Missing files follow the character.
    #[serde(default = "default_sound_pack")]
    pub sound_pack: String,
    /// Cue volume from 0 to 1.
    #[serde(default = "default_sound_volume")]
    pub sound_volume: f64,
    #[serde(default)]
    pub smart_mood: bool,
    #[serde(default)]
    pub smart_time_of_day: bool,
    #[serde(default)]
    pub smart_holiday: bool,
}

const SOUND_PACKS: [&str; 9] = [
    "match", "off", "soft-chime", "chiptune", "robot", "magic", "wood", "paper", "neon",
];

fn default_character() -> String {
    "classic".into()
}

fn default_sound_pack() -> String {
    "match".into()
}

fn default_sound_volume() -> f64 {
    0.6
}

fn normalize_character(id: &str) -> String {
    let id = id.trim().to_ascii_lowercase();
    let known = id == "classic"
        || (id.len() == 3
            && id.starts_with('c')
            && id[1..].parse::<u8>().map(|n| (2..=28).contains(&n)).unwrap_or(false));
    if known {
        id
    } else {
        default_character()
    }
}

fn normalize_sound_pack(id: &str) -> String {
    let id = id.trim().to_ascii_lowercase();
    if SOUND_PACKS.contains(&id.as_str()) {
        id
    } else {
        default_sound_pack()
    }
}

fn normalize_volume(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        default_sound_volume()
    }
}

fn default_cleanup_model() -> String {
    CLEANUP_MODEL.to_string()
}

fn output_english() -> String {
    "english".into()
}

fn paste_again_hotkey() -> Hotkey {
    Hotkey::paste_again()
}

fn default_offline_model() -> String {
    "base.en".into()
}

fn history_on() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: Hotkey::default(),
            microphone: None,
            model: WHISPER_TURBO.to_string(),
            cleanup_enabled: true,
            language: "auto".into(),
            dictionary: Vec::new(),
            launch_at_startup: true,
            history_enabled: true,
            paste_hotkey: Hotkey::paste_again(),
            offline_model: "base.en".into(),
            output: "english".into(),
            verbatim: false,
            cleanup_model: CLEANUP_MODEL.to_string(),
            character: default_character(),
            sound_pack: default_sound_pack(),
            sound_volume: default_sound_volume(),
            smart_mood: false,
            smart_time_of_day: false,
            smart_holiday: false,
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
    if let Some(key) = settings.paste_hotkey.key {
        let key = key.trim().to_ascii_lowercase();
        settings.paste_hotkey.key = if key.is_empty() { None } else { Some(key) };
    }
    settings.offline_model = offline::normalize_model(&settings.offline_model).to_string();
    settings.output = normalize_output(&settings.output).to_string();
    settings.cleanup_model = settings.cleanup_model.trim().to_string();
    settings.character = normalize_character(&settings.character);
    settings.sound_pack = normalize_sound_pack(&settings.sound_pack);
    settings.sound_volume = normalize_volume(settings.sound_volume);
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
    validate_hotkey(&settings.paste_hotkey)?;
    match offline::normalize_model(&settings.offline_model) {
        "off" | "base.en" | "small.en" => {}
        _ => return Err("Choose base.en, small.en, or off for the on-device model.".into()),
    }
    match settings.output.as_str() {
        "english" | "same" => {}
        _ => return Err("Output should be English (translate) or Same as spoken.".into()),
    }
    if let Some(message) = cleanup_model_error(&settings.cleanup_model) {
        return Err(message);
    }
    Ok(())
}

fn cleanup_model_error(model: &str) -> Option<String> {
    if model.is_empty()
        || model.chars().count() > 80
        || !model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'))
    {
        return Some("Cleanup model should be a Groq model id such as openai/gpt-oss-20b.".into());
    }
    None
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
    if validate_hotkey(&settings.paste_hotkey).is_err() {
        settings.paste_hotkey = Hotkey::paste_again();
    }
    settings.offline_model = offline::normalize_model(&settings.offline_model).to_string();
    settings.output = normalize_output(&settings.output).to_string();
    if cleanup_model_error(&settings.cleanup_model).is_some() {
        settings.cleanup_model = CLEANUP_MODEL.to_string();
    }
    settings.character = normalize_character(&settings.character);
    settings.sound_pack = normalize_sound_pack(&settings.sound_pack);
    settings.sound_volume = normalize_volume(settings.sound_volume);
    settings
}

/// Copy `settings.json` and `history.json` from a sibling Cadence config
/// directory when this folder does not have settings yet. The old folder is
/// left in place.
pub fn migrate_legacy_config(dir: &Path) -> bool {
    if dir.join("settings.json").exists() {
        return false;
    }
    let Some(parent) = dir.parent() else {
        return false;
    };
    let legacy = parent.join("com.cadence.dictation");
    if legacy == dir || !legacy.is_dir() {
        return false;
    }
    let mut copied = false;
    for name in ["settings.json", "history.json"] {
        let from = legacy.join(name);
        let to = dir.join(name);
        if from.is_file() && !to.exists() && fs::copy(&from, &to).is_ok() {
            copied = true;
        }
    }
    copied
}

pub fn load_settings(dir: &Path) -> Settings {
    let path = settings_path(dir);
    let Ok(bytes) = fs::read(&path) else {
        return Settings::default();
    };
    match parse_json(&bytes) {
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
    parse_json::<Vec<HistoryEntry>>(&bytes).unwrap_or_default()
}

/// PowerShell 5.1 writes JSON as UTF-8 with a BOM, or as UTF-16 LE. Either
/// prefix would make serde reject an otherwise valid settings file.
fn parse_json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return serde_json::from_slice(rest);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let text = String::from_utf16_lossy(&units);
        return serde_json::from_str(&text);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        let text = String::from_utf16_lossy(&units);
        return serde_json::from_str(&text);
    }
    serde_json::from_slice(bytes)
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
        assert!(parsed.cleanup_enabled);
        assert!(parsed.history_enabled);
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
    fn powershell_bom_and_indentation_still_parse() {
        let pretty = r#"{
    "hotkey": {
        "ctrl": true,
        "alt": false,
        "shift": false,
        "win": true,
        "key": null
    },
    "microphone": null,
    "model": "whisper-large-v3-turbo",
    "cleanupEnabled": true,
    "language": "auto",
    "dictionary": [],
    "launchAtStartup": true
}"#;
        let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
        utf8_bom.extend_from_slice(pretty.as_bytes());
        let parsed: Settings = parse_json(&utf8_bom).unwrap();
        let parsed = normalize_settings(parsed);
        assert!(parsed.microphone.is_none());
        assert!(parsed.cleanup_enabled);
        assert!(parsed.launch_at_startup);
        assert_eq!(parsed.model, WHISPER_TURBO);
        assert!(parsed.hotkey.ctrl && parsed.hotkey.win && parsed.hotkey.key.is_none());
        assert_eq!(parsed.offline_model, "base.en");
        assert!(parsed.paste_hotkey.ctrl && parsed.paste_hotkey.win);
        assert_eq!(parsed.paste_hotkey.key.as_deref(), Some("v"));

        let mut utf16 = vec![0xFF, 0xFE];
        for unit in pretty.encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let wide: Settings = parse_json(&utf16).unwrap();
        assert!(normalize_settings(wide).cleanup_enabled);
    }

    #[test]
    fn roundtrip_on_disk() {
        let dir = std::env::temp_dir().join(format!("bolo-test-{}", now_ms()));
        let _ = fs::remove_dir_all(&dir);
        let mut settings = Settings::default();
        settings.dictionary = vec!["Bolo".into(), "Tauri".into()];
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

#[cfg(test)]
mod carryover {
    use super::*;

    #[test]
    fn copies_settings_and_history_from_the_cadence_folder() {
        let parent = std::env::temp_dir().join(format!("bolo-carryover-{}", now_ms()));
        let legacy = parent.join("com.cadence.dictation");
        let next = parent.join("com.bolo.dictation");
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&next).unwrap();
        let mut settings = Settings::default();
        settings.dictionary = vec!["Bolo".into()];
        settings.language = "en".into();
        save_settings(&legacy, &settings).unwrap();
        save_history(
            &legacy,
            &[HistoryEntry {
                id: "old".into(),
                text: "from cadence".into(),
                created_unix_ms: 1,
                model: WHISPER_TURBO.into(),
                cleaned: false,
                duration_ms: 10,
            }],
        )
        .unwrap();
        assert!(migrate_legacy_config(&next));
        let loaded = load_settings(&next);
        assert_eq!(loaded.dictionary, vec!["Bolo".to_string()]);
        assert_eq!(loaded.language, "en");
        assert!(loaded.history_enabled);
        assert_eq!(load_history(&next)[0].text, "from cadence");
        assert!(legacy.join("settings.json").is_file());
        assert!(!migrate_legacy_config(&next));
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn history_stays_on_when_the_old_file_omits_the_field() {
        let raw = r#"{"cleanupEnabled":true,"language":"auto"}"#;
        let parsed: Settings = serde_json::from_str(raw).unwrap();
        assert!(parsed.history_enabled);
        assert!(parsed.cleanup_enabled);
        assert_eq!(parsed.output, "english");
        assert!(!parsed.verbatim);
        assert_eq!(parsed.cleanup_model, CLEANUP_MODEL);
        let legacy = serde_json::from_str::<Settings>(
            r#"{"cleanupEnabled":true,"cleanupModel":"llama-3.1-8b-instant"}"#,
        )
        .unwrap();
        assert_eq!(legacy.cleanup_model, "llama-3.1-8b-instant");
        assert_eq!(normalize_settings(legacy).cleanup_model, "llama-3.1-8b-instant");
    }

    #[test]
    fn saved_settings_and_history_never_store_an_api_key() {
        let dir = std::env::temp_dir().join(format!("bolo-settings-nokey-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        save_settings(&dir, &Settings::default()).unwrap();
        save_history(
            &dir,
            &[HistoryEntry {
                id: "one".into(),
                text: "ship the notes to Priya".into(),
                created_unix_ms: 1,
                model: WHISPER_TURBO.into(),
                cleaned: false,
                duration_ms: 10,
            }],
        )
        .unwrap();
        let settings = fs::read_to_string(dir.join("settings.json")).unwrap();
        let history = fs::read_to_string(dir.join("history.json")).unwrap();
        for raw in [&settings, &history, &serde_json::to_string(&Settings::default()).unwrap()] {
            let lower = raw.to_ascii_lowercase();
            assert!(!lower.contains("gsk_"), "{raw}");
            assert!(!lower.contains("apikey"), "{raw}");
            assert!(!lower.contains("groqkey"), "{raw}");
            assert!(!lower.contains("builtinkey"), "{raw}");
            assert!(!lower.contains("bolo_builtin"), "{raw}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn credential_targets_still_name_cadence() {
        assert!(crate::credentials::CADENCE_TARGET.contains("com.cadence.dictation"));
        assert!(crate::credentials::TARGET.starts_with("com.bolo.dictation"));
        assert!(crate::credentials::LEGACY_TARGET.contains("cadence"));
    }

    #[test]
    fn character_settings_default_and_normalize() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.character, "classic");
        assert_eq!(old.sound_pack, "match");
        assert!((old.sound_volume - 0.6).abs() < 1e-9);
        let mut s = Settings::default();
        s.character = "c99".into();
        s.sound_pack = "whoosh".into();
        s.sound_volume = 4.0;
        let s = normalize_settings(s);
        assert_eq!(s.character, "classic");
        assert_eq!(s.sound_pack, "match");
        assert_eq!(s.sound_volume, 1.0);
        let mut s = Settings::default();
        s.character = "C07".into();
        s.sound_pack = "robot".into();
        let s = prepare_settings(s);
        assert_eq!(s.character, "c07");
        assert_eq!(s.sound_pack, "robot");
    }
}
