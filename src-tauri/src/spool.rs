//! Keep each take's audio on disk until the paste actually lands.
//!
//! A crash after transcription still leaves `text.txt`, so the paste-again
//! hotkey can insert it. Audio files stay beside that text until then.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct Recovered {
    pub id: String,
    pub text: Option<String>,
    pub audio: Vec<Vec<u8>>,
}

pub fn root(config: &Path) -> PathBuf {
    config.join("spool")
}

pub fn write_audio(config: &Path, id: &str, index: usize, bytes: &[u8]) {
    if id.is_empty() || bytes.is_empty() {
        return;
    }
    let dir = root(config).join(id);
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = fs::write(dir.join(format!("{index:04}.audio")), bytes);
}

pub fn write_text(config: &Path, id: &str, text: &str) {
    if id.is_empty() {
        return;
    }
    let dir = root(config).join(id);
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = fs::write(dir.join("text.txt"), text);
}

pub fn remove(config: &Path, id: &str) {
    if id.is_empty() {
        return;
    }
    let _ = fs::remove_dir_all(root(config).join(id));
}

/// Newest spool that still has text or audio. Pasted takes are deleted.
pub fn latest(config: &Path) -> Option<Recovered> {
    let entries = fs::read_dir(root(config)).ok()?;
    let mut best: Option<(SystemTime, PathBuf, String)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let stamp = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(prev, _, _)| stamp >= *prev) {
            best = Some((stamp, path, id));
        }
    }
    let (_, path, id) = best?;
    let text = fs::read_to_string(path.join("text.txt"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    let mut audio = Vec::new();
    if let Ok(files) = fs::read_dir(&path) {
        let mut names: Vec<PathBuf> = files
            .flatten()
            .map(|file| file.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "audio"))
            .collect();
        names.sort();
        for name in names {
            if let Ok(bytes) = fs::read(name) {
                if !bytes.is_empty() {
                    audio.push(bytes);
                }
            }
        }
    }
    if text.is_none() && audio.is_empty() {
        return None;
    }
    Some(Recovered { id, text, audio })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_stays_until_the_paste_and_then_can_be_read_back() {
        let dir = std::env::temp_dir().join(format!(
            "bolo-spool-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write_audio(&dir, "take-a", 0, b"fLaC-chunk");
        write_text(&dir, "take-a", "alpha take");
        let recovered = latest(&dir).expect("spool should keep the take");
        assert_eq!(recovered.id, "take-a");
        assert_eq!(recovered.text.as_deref(), Some("alpha take"));
        assert_eq!(recovered.audio.len(), 1);
        remove(&dir, "take-a");
        assert!(latest(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
