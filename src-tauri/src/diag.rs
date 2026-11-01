//! Local-only diagnostics for this PC build: what Whisper heard, what the
//! translator got and returned, and what was pasted, per take. Written to
//! diag.log beside settings.json, last 20 takes. Never contains keys.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub const ENABLED: bool = true;
const KEEP: usize = 20;
const MARK: &str = "=== take ";

static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static TRANSLATED: AtomicBool = AtomicBool::new(false);
static TRANSLATED_TEXTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Remember a pasted translation so it isn't fed back to Whisper as
/// "recent text" (an English mis-translation then biased the next Urdu take).
pub fn remember_translation(text: &str) {
    if let Ok(mut list) = TRANSLATED_TEXTS.lock() {
        list.push(text.trim().to_string());
        if list.len() > 10 {
            list.remove(0);
        }
    }
}

pub fn was_translation(text: &str) -> bool {
    TRANSLATED_TEXTS.lock().map(|list| list.iter().any(|t| t == text.trim())).unwrap_or(false)
}

pub fn note(line: impl Into<String>) {
    if !ENABLED {
        return;
    }
    if let Ok(mut notes) = NOTES.lock() {
        notes.push(line.into());
    }
}

/// The current take was translated, so its English isn't the user's own words.
pub fn set_translated(value: bool) {
    TRANSLATED.store(value, Ordering::SeqCst);
}

pub fn take_translated() -> bool {
    TRANSLATED.swap(false, Ordering::SeqCst)
}

pub fn flush(config: &Path, id: &str, pasted: &str) {
    let notes: Vec<String> = NOTES.lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default();
    if !ENABLED {
        return;
    }
    let path = config.join("diag.log");
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let block = render(id, &notes, pasted);
    let _ = std::fs::write(&path, trim_blocks(&format!("{old}{block}"), KEEP));
}

fn render(id: &str, notes: &[String], pasted: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut out = format!("{MARK}{id} at {stamp}\n");
    for line in notes {
        out.push_str("  ");
        out.push_str(&line.replace('\n', " "));
        out.push('\n');
    }
    out.push_str(&format!("  pasted: {pasted}\n\n"));
    out
}

fn trim_blocks(all: &str, keep: usize) -> String {
    let starts: Vec<usize> = all.match_indices(MARK).map(|(i, _)| i).collect();
    if starts.len() <= keep {
        return all.to_string();
    }
    all[starts[starts.len() - keep]..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_last_takes() {
        let mut all = String::new();
        for n in 0..25 {
            all.push_str(&render(&format!("t{n}"), &["raw: x".into()], "y"));
        }
        let kept = trim_blocks(&all, 20);
        assert_eq!(kept.matches(MARK).count(), 20);
        assert!(kept.starts_with("=== take t5 "));
    }
}
