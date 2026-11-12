//! Decide when Bolo is unsure about a take, so the overlay offers Fix last.
//! The tray item and the History Fix button are always available.
//!
//! Cleanup falling back is not, by itself, a reason to offer Fix last. The
//! paste in that case is still the speaker's words (local filler cleanup of
//! the transcript). Fix last is for a take whose inserted text is the doubtful
//! part: a failed chunk, the on-device fallback, low confidence, a collapsed
//! repeat, a translation or mixed-language pass, a second pass, or a cleanup
//! rewrite that changed a lot of the words.

/// What one finished take looked like.
#[derive(Default, Clone)]
pub struct Signals {
    /// Speech path of every chunk, e.g. "transcribe", "mixed-llama-repass-lowconf".
    pub paths: Vec<String>,
    /// Engine of every chunk, "Groq" or "On this PC".
    pub engines: Vec<String>,
    /// Some chunks failed and only the rest was pasted.
    pub partial: bool,
    /// Cleanup was supposed to run.
    pub cleanup_wanted: bool,
    /// `CleanOutcome.why`, or empty when cleanup never ran.
    /// Kept on the signal so a log can name the fallback. It does not, by
    /// itself, open Fix last: the pasted words are still the speaker's.
    #[allow(dead_code)]
    pub cleanup_why: String,
    /// Text before cleanup.
    pub raw: String,
    /// Text that was pasted.
    pub pasted: String,
}

/// The reason Bolo is unsure, or None when the take looks fine.
pub fn reason(s: &Signals) -> Option<&'static str> {
    if s.partial {
        return Some("part of the take failed");
    }
    if s.engines.iter().any(|e| !e.is_empty() && e != "Groq") {
        return Some("offline fallback");
    }
    if s.paths.iter().any(|p| p.contains("collapsed")) {
        return Some("repeat loop collapsed");
    }
    if s.paths.iter().any(|p| p.contains("translate") || p.contains("mixed")) {
        return Some("translated or mixed language");
    }
    if s.paths.iter().any(|p| p.contains("repass")) {
        return Some("second pass replaced the first");
    }
    if s.paths.iter().any(|p| p.contains("lowconf")) {
        return Some("low speech confidence");
    }
    if s.cleanup_wanted && heavy_change(&s.raw, &s.pasted) {
        return Some("cleanup changed a lot");
    }
    None
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// More than 40% of the words differ (by longest common subsequence).
/// Very short takes are ignored: two words changing there is normal tidying.
pub fn heavy_change(raw: &str, pasted: &str) -> bool {
    let a = words(raw);
    let b = words(pasted);
    let longest = a.len().max(b.len());
    if longest < 5 {
        return false;
    }
    let mut row = vec![0usize; b.len() + 1];
    for x in &a {
        let mut prev = 0;
        for (j, y) in b.iter().enumerate() {
            let keep = row[j + 1];
            row[j + 1] = if x == y { prev + 1 } else { row[j + 1].max(row[j]) };
            prev = keep;
        }
    }
    let common = row[b.len()];
    (longest - common) * 10 > longest * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fine() -> Signals {
        Signals {
            paths: vec!["transcribe".into()],
            engines: vec!["Groq".into()],
            partial: false,
            cleanup_wanted: true,
            cleanup_why: "applied".into(),
            raw: "um so send the invoice to Ahmed by Friday".into(),
            pasted: "So send the invoice to Ahmed by Friday.".into(),
        }
    }

    #[test]
    fn a_clean_english_take_is_not_unsure() {
        assert_eq!(reason(&fine()), None);
    }

    #[test]
    fn cleanup_off_or_verbatim_is_fine() {
        for why in ["off", "verbatim", "empty", "applied", "brief"] {
            let s = Signals { cleanup_why: why.into(), ..fine() };
            assert_eq!(reason(&s), None, "{why}");
        }
        let s = Signals { cleanup_wanted: false, cleanup_why: String::new(), ..fine() };
        assert_eq!(reason(&s), None);
    }

    #[test]
    fn translated_and_mixed_takes_are_unsure() {
        for path in ["translate", "mixed-llama"] {
            let s = Signals { paths: vec![path.into()], ..fine() };
            assert!(reason(&s).is_some(), "{path}");
        }
    }

    #[test]
    fn any_chunk_counts() {
        let s = Signals { paths: vec!["transcribe".into(), "transcribe-collapsed".into()], ..fine() };
        assert_eq!(reason(&s), Some("repeat loop collapsed"));
    }

    #[test]
    fn urdu_repass_low_confidence_partial_and_offline_are_unsure() {
        assert!(reason(&Signals { paths: vec!["transcribe-repass".into()], ..fine() }).is_some());
        assert_eq!(
            reason(&Signals { paths: vec!["transcribe-lowconf".into()], ..fine() }),
            Some("low speech confidence")
        );
        assert!(reason(&Signals { partial: true, ..fine() }).is_some());
        assert_eq!(
            reason(&Signals { engines: vec!["On this PC".into()], ..fine() }),
            Some("offline fallback")
        );
    }

    #[test]
    fn cleanup_fallback_alone_does_not_offer_fix_last() {
        // The words were inserted. Only the rewrite failed, so Fix last would
        // fire on every take while Groq cleanup is down.
        for why in [
            "rejected empty rewrite",
            "timeout",
            "",
            "http 403 The model is blocked at the organization level",
            "model missing: openai/gpt-oss-20b; http 403 blocked",
        ] {
            let s = Signals { cleanup_why: why.into(), ..fine() };
            assert_eq!(reason(&s), None, "{why:?}");
        }
    }

    #[test]
    fn a_failed_cleanup_still_offers_fix_last_when_the_words_are_doubtful() {
        let s = Signals {
            cleanup_why: "http 403 The model is blocked".into(),
            paths: vec!["transcribe-lowconf".into()],
            ..fine()
        };
        assert_eq!(reason(&s), Some("low speech confidence"));
        assert_eq!(
            reason(&Signals { partial: true, cleanup_why: "timeout".into(), ..fine() }),
            Some("part of the take failed")
        );
    }

    #[test]
    fn heavy_rewrite_is_unsure_light_tidy_is_not() {
        assert!(!heavy_change("um so send the invoice to Ahmed by Friday", "So send the invoice to Ahmed by Friday."));
        assert!(heavy_change(
            "please ask tie more about the new design today",
            "Kindly check with the manager regarding tomorrow's plan."
        ));
        assert!(!heavy_change("ok fine", "Okay, fine."));
        let s = Signals {
            raw: "please ask tie more about the new design today".into(),
            pasted: "Kindly check with the manager regarding tomorrow's plan.".into(),
            ..fine()
        };
        assert_eq!(reason(&s), Some("cleanup changed a lot"));
    }

    #[test]
    fn whisper_doubt_thresholds() {
        use crate::transcribe::doubtful;
        assert!(!doubtful("Ship it today.", Some(-0.2), Some(0.02)));
        assert!(doubtful("Ship it today.", Some(-0.9), Some(0.02)));
        assert!(doubtful("Ship it today.", Some(-0.2), Some(0.8)));
        assert!(!doubtful("", Some(-2.0), Some(0.9)));
        assert!(!doubtful("Ship it today.", None, None));
    }

    #[test]
    fn esc_only_counts_while_the_card_is_visible() {
        use crate::listener::{escape_hits_fix_card, FIX_CARD_ESC};
        use std::sync::atomic::Ordering;
        FIX_CARD_ESC.store(false, Ordering::SeqCst);
        assert!(!escape_hits_fix_card(0x1B, true));
        FIX_CARD_ESC.store(true, Ordering::SeqCst);
        assert!(!escape_hits_fix_card(0x41, true), "other keys are ignored");
        assert!(!escape_hits_fix_card(0x1B, false), "key-up is ignored");
        assert!(escape_hits_fix_card(0x1B, true));
        assert!(!escape_hits_fix_card(0x1B, true), "one press only");
    }
}
