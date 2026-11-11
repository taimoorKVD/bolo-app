//! Keep each take's audio on disk. After the paste lands it is marked
//! `pasted` and kept (newest three takes) so Fix last can listen again.
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

/// The paste landed. The take is no longer offered for crash recovery.
/// Audio stays only when the caller has not discarded it.
pub fn mark_pasted(config: &Path, id: &str) {
    if id.is_empty() {
        return;
    }
    let dir = root(config).join(id);
    if dir.is_dir() {
        let _ = fs::write(dir.join("pasted"), b"1");
    }
}

/// Delete audio files for one take. The transcript file stays.
pub fn discard_audio(config: &Path, id: &str) {
    if id.is_empty() || id.contains(['/', '\\', '.']) {
        return;
    }
    discard_dir(&root(config).join(id));
}

/// Delete every saved recording under the spool. Transcripts stay.
pub fn discard_all_audio(config: &Path) {
    let Ok(entries) = fs::read_dir(root(config)) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            discard_dir(&entry.path());
        }
    }
}

fn discard_dir(path: &Path) {
    let Ok(files) = fs::read_dir(path) else {
        return;
    };
    for file in files.flatten() {
        let path = file.path();
        if path.extension().is_some_and(|ext| ext == "audio") {
            let _ = fs::remove_file(path);
        }
    }
}

/// The kept audio chunks of one take, in order. Empty when it is gone.
pub fn audio(config: &Path, id: &str) -> Vec<Vec<u8>> {
    if id.is_empty() || id.contains(['/', '\\', '.']) {
        return Vec::new();
    }
    read_audio(&root(config).join(id))
}

fn read_audio(path: &Path) -> Vec<Vec<u8>> {
    let mut audio = Vec::new();
    if let Ok(files) = fs::read_dir(path) {
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
    audio
}

/// Keep only the newest `keep` takes (plus `current`). Typed inserts can't
/// be confirmed, so without this every take's audio stayed on disk forever.
pub fn prune(config: &Path, current: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(root(config)) else {
        return;
    };
    let mut dirs: Vec<(SystemTime, PathBuf, String)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let stamp = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (stamp, entry.path(), entry.file_name().to_string_lossy().to_string())
        })
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path, id) in dirs.into_iter().skip(keep) {
        if id != current {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// How many times a saved recording is sent again after the online service
/// was unreachable. After this, the audio stays on disk but is not retried.
pub const RETRY_LIMIT: u32 = 4;

fn retry_count(dir: &Path) -> u32 {
    fs::read_to_string(dir.join("retries"))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

pub fn note_retry(config: &Path, id: &str) {
    if id.is_empty() {
        return;
    }
    let dir = root(config).join(id);
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let next = retry_count(&dir).saturating_add(1);
    let _ = fs::write(dir.join("retries"), next.to_string());
}

/// Newest take that has audio, no transcript, and retries left.
/// That recording is tried again when the online service is back.
pub fn untranscribed(config: &Path) -> Option<Recovered> {
    let entries = fs::read_dir(root(config)).ok()?;
    let mut best: Option<(SystemTime, Recovered)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path.join("pasted").is_file() || path.join("text.txt").is_file() {
            continue;
        }
        if retry_count(&path) >= RETRY_LIMIT {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let audio = read_audio(&path);
        if audio.is_empty() {
            continue;
        }
        let stamp = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(prev, _)| stamp >= *prev) {
            best = Some((stamp, Recovered { id, text: None, audio }));
        }
    }
    best.map(|(_, recovered)| recovered)
}

/// Newest spool that still has text or audio. Pasted takes are skipped.
pub fn latest(config: &Path) -> Option<Recovered> {
    let entries = fs::read_dir(root(config)).ok()?;
    let mut best: Option<(SystemTime, PathBuf, String)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path.join("pasted").is_file() {
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
    let audio = read_audio(&path);
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
        mark_pasted(&dir, "take-a");
        assert!(latest(&dir).is_none(), "a pasted take is not recovered again");
        assert_eq!(audio(&dir, "take-a"), vec![b"fLaC-chunk".to_vec()]);
        discard_audio(&dir, "take-a");
        assert!(audio(&dir, "take-a").is_empty(), "audio is not kept without an explicit choice");
        assert_eq!(
            fs::read_to_string(root(&dir).join("take-a").join("text.txt")).unwrap(),
            "alpha take"
        );
        write_audio(&dir, "take-b", 0, b"more-audio");
        discard_all_audio(&dir);
        assert!(audio(&dir, "take-b").is_empty());
        assert!(audio(&dir, "..").is_empty());
        remove(&dir, "take-a");
        assert!(audio(&dir, "take-a").is_empty());
        write_audio(&dir, "take-c", 0, b"held");
        let pending = untranscribed(&dir).expect("audio without text can be retried");
        assert_eq!(pending.id, "take-c");
        assert!(pending.text.is_none());
        for _ in 0..RETRY_LIMIT {
            note_retry(&dir, "take-c");
        }
        assert!(untranscribed(&dir).is_none(), "retries stop after the limit");
        write_text(&dir, "take-c", "caught it");
        assert!(untranscribed(&dir).is_none(), "a transcript is not sent again");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_keeps_only_the_newest_takes() {
        let dir = std::env::temp_dir().join(format!("bolo-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for n in 0..6 {
            write_text(&dir, &format!("take-{n}"), "x");
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        prune(&dir, "take-0", 3);
        let mut left: Vec<String> = fs::read_dir(root(&dir)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, vec!["take-0", "take-3", "take-4", "take-5"]);
        assert_eq!(latest(&dir).unwrap().id, "take-5");
        let _ = fs::remove_dir_all(&dir);
    }
}
