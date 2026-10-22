//! Pause cuts and overlap-aware joining for long takes.
//!
//! A take used to stop at 120 seconds (`MAX_RECORDING`), and the live buffer
//! stopped accepting samples after about 180 seconds of stereo 48 kHz audio
//! (`MAX_SAMPLES`). Neither cap is applied anymore. While a take is open, a
//! pause after ~20 seconds — or a hard cut at ~30 seconds — peels off a chunk.
//! The next chunk keeps the last 800 ms so the join can drop the repeated words.

use std::time::Duration;

/// The recording watchdog that used to end a take. It is not enforced.
#[cfg_attr(not(test), allow(dead_code))]
pub const REMOVED_RECORDING_CAP_SECS: u64 = 120;
/// The live sample ceiling, in seconds of stereo 48 kHz, that used to drop audio.
#[cfg_attr(not(test), allow(dead_code))]
pub const REMOVED_SAMPLE_CAP_SECS: u64 = 180;

pub const CHUNK_PAUSE_MS: u32 = 20_000;
pub const CHUNK_FORCE_MS: u32 = 30_000;
pub const PAUSE_HOLD_MS: u32 = 280;
pub const OVERLAP_MS: u32 = 800;

/// Dictation no longer stops a take because of its length.
#[cfg_attr(not(test), allow(dead_code))]
pub fn recording_cap() -> Option<Duration> {
    None
}

/// Cut when a pause arrives after 20 seconds, or at 30 seconds even mid-phrase.
pub fn should_cut(buffered_ms: u32, trailing_silence_ms: u32) -> bool {
    if buffered_ms >= CHUNK_FORCE_MS {
        return true;
    }
    buffered_ms >= CHUNK_PAUSE_MS && trailing_silence_ms >= PAUSE_HOLD_MS
}

pub fn overlap_samples(sample_rate: u32, channels: u16) -> usize {
    let channels = usize::from(channels.max(1));
    (sample_rate.max(1) as usize) * channels * OVERLAP_MS as usize / 1000
}

/// Join chunk transcripts, dropping a duplicated word run at each boundary.
pub fn stitch<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut joined = String::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if joined.is_empty() {
            joined = part.to_string();
            continue;
        }
        joined = join_overlap(&joined, part);
    }
    joined
}

fn join_overlap(left: &str, right: &str) -> String {
    let left_words: Vec<&str> = left.split_whitespace().collect();
    let right_words: Vec<&str> = right.split_whitespace().collect();
    if left_words.is_empty() {
        return right.to_string();
    }
    if right_words.is_empty() {
        return left.to_string();
    }
    let max = left_words.len().min(right_words.len()).min(12);
    let mut cut = 0usize;
    for size in (1..=max).rev() {
        let same = left_words[left_words.len() - size..]
            .iter()
            .zip(&right_words[..size])
            .all(|(left_word, right_word)| normalize_word(left_word) == normalize_word(right_word));
        if same && left_words[left_words.len() - size..].iter().any(|word| !normalize_word(word).is_empty()) {
            cut = size;
            break;
        }
    }
    if cut == 0 {
        return format!("{left} {right}");
    }
    if cut >= right_words.len() {
        return left.to_string();
    }
    format!("{left} {}", right_words[cut..].join(" "))
}

fn normalize_word(word: &str) -> String {
    word.trim_matches(|ch: char| !ch.is_alphanumeric())
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_is_not_capped_at_two_minutes() {
        assert!(recording_cap().is_none());
        assert_eq!(REMOVED_RECORDING_CAP_SECS, 120);
        assert_eq!(REMOVED_SAMPLE_CAP_SECS, 180);
        assert!(!should_cut(19_000, 1_000));
        assert!(should_cut(CHUNK_PAUSE_MS, PAUSE_HOLD_MS));
        assert!(!should_cut(CHUNK_PAUSE_MS, PAUSE_HOLD_MS - 1));
        assert!(should_cut(CHUNK_FORCE_MS, 0));
    }

    #[test]
    fn stitch_drops_the_overlapping_words() {
        let text = stitch([
            "ship the notes to Priya",
            "to Priya before Thursday",
            "and send the recap",
        ]);
        assert_eq!(text, "ship the notes to Priya before Thursday and send the recap");
        let same = stitch(["brown fox.", "fox jumps"]);
        assert_eq!(same, "brown fox. jumps");
        let distinct = stitch(["hello there", "general kenobi"]);
        assert_eq!(distinct, "hello there general kenobi");
        assert_eq!(stitch(["only"]), "only");
        assert_eq!(stitch(["", "  "]), "");
    }
}
