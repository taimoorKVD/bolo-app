//! Text posted after Whisper: the user's fixes, My words (names, brands,
//! spoken emails), then local homophones. No network. The optional Groq
//! meaning check lives in `sense` and is skipped unless a homophone is still
//! ambiguous.

use crate::memory::{self, Correction};
use crate::sense;
use crate::vocab;

/// Text after the local passes inside `polish_text`, in the order they run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStages {
    /// Saved fixes, then My words / seed spellings. Grammar has not run.
    pub learned: String,
    /// `grammar::polish` after the local homophone pass. Equals `learned`
    /// when `homophones` is off (verbatim), because that pass is skipped.
    pub grammar: String,
    pub grammar_ran: bool,
}

/// `homophones` is off for verbatim takes. Names, domains, and saved fixes
/// still apply: those are spellings, not a rewrite.
pub fn local_stages(text: &str, corrections: &[Correction], words: &[String], homophones: bool) -> LocalStages {
    let text = crate::transcribe::recover_brief_tech(text);
    let text = memory::apply_corrections(&text, corrections);
    let learned = vocab::correct(&text, words);
    if !homophones {
        return LocalStages {
            grammar: learned.clone(),
            grammar_ran: false,
            learned,
        };
    }
    let text = crate::mishear::apply(&learned);
    let text = sense::local_homophones(&text, words);
    let grammar = crate::grammar::polish(&text);
    LocalStages {
        learned,
        grammar,
        grammar_ran: true,
    }
}

pub fn polish_text(text: &str, corrections: &[Correction], words: &[String], homophones: bool) -> String {
    local_stages(text, corrections, words, homophones).grammar
}

/// Tokens used by `wer`: lower case, punctuation dropped, `@` and `.` kept.
pub fn tokens(text: &str) -> Vec<String> {
    wer_tokens(text)
}

/// Word error rate. Punctuation is ignored; `@` and `.` stay so an email is
/// one word. Empty reference and empty hypothesis score 0.
pub fn wer(reference: &str, hypothesis: &str) -> f64 {
    score_tokens(&wer_tokens(reference), &wer_tokens(hypothesis)).wer
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WerScore {
    pub wer: f64,
    pub edits: usize,
    pub reference_words: usize,
    pub hypothesis_words: usize,
    /// The reference's first word is missing or replaced.
    pub first_dropped: bool,
    /// The reference's last word is missing or replaced.
    pub last_dropped: bool,
}

pub fn wer_score(reference: &str, hypothesis: &str) -> WerScore {
    score_tokens(&wer_tokens(reference), &wer_tokens(hypothesis))
}

fn score_tokens(reference: &[String], hypothesis: &[String]) -> WerScore {
    let edits = if reference.is_empty() {
        if hypothesis.is_empty() { 0 } else { hypothesis.len() }
    } else {
        edit(reference, hypothesis)
    };
    let wer = if reference.is_empty() {
        if hypothesis.is_empty() { 0.0 } else { 1.0 }
    } else {
        edits as f64 / reference.len() as f64
    };
    let first_dropped = reference.first().is_some_and(|word| hypothesis.first() != Some(word));
    let last_dropped = reference.last().is_some_and(|word| hypothesis.last() != Some(word));
    WerScore {
        wer,
        edits,
        reference_words: reference.len(),
        hypothesis_words: hypothesis.len(),
        first_dropped,
        last_dropped,
    }
}

fn wer_tokens(text: &str) -> Vec<String> {
    use unicode_normalization::UnicodeNormalization;
    // NFC folds alef+madda and other composed Urdu letters before the split,
    // so a decomposed hypothesis matches a composed reference.
    let text: String = text.nfc().collect();
    let text = strip_wer_marks(&text);
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '@' && c != '.' && c != '\'')
        .map(|word| word.trim_matches(|c: char| c == '.' || c == '\'').to_string())
        .filter(|word| !word.is_empty())
        .collect()
}

/// Zero-width and bidi marks are not letters; left in, they split one Urdu
/// word into two. Urdu punctuation (۔ ، ؟) and the Devanagari danda are not words.
fn strip_wer_marks(text: &str) -> String {
    text.chars().filter(|c| !wer_ignored(*c)).collect()
}

fn wer_ignored(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{200B}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
            | '\u{060C}'
            | '\u{061B}'
            | '\u{061F}'
            | '\u{06D4}'
            | '\u{0964}'
            | '\u{0965}'
    )
}

fn edit(a: &[String], b: &[String]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, right) in b.iter().enumerate() {
            let cost = usize::from(left != right);
            let del = prev[j + 1] + 1;
            let ins = row[j] + 1;
            let sub = prev[j] + cost;
            row.push(del.min(ins).min(sub));
        }
        prev = row;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{self, Memory};
    use crate::pipeline;
    use crate::transcribe::{self, WHISPER_LARGE, WHISPER_TURBO};
    use crate::vocab::SEED_WORDS;
    use std::time::Instant;

    fn words() -> Vec<String> {
        vocab::protected(&[])
    }

    #[test]
    fn report_phrases_are_fixed_at_the_text_level() {
        let words = words();
        let cases = [
            ("Open Volo and start dictating.", "Open Bolo and start dictating."),
            ("Ask Roomba about Crock", "Ask RoomBoss about Grok."),
            ("Roombos then GROV", "RoomBoss then Grok."),
            ("PoloApp and BoloApp", "Bolo and Bolo."),
            ("Open Volo.com and start dictating.", "Open Bolo.com and start dictating."),
            ("Send it to taimurhussain.com", "Send it to taimoorhussain.com."),
            ("Send it to taimoorhussain.com", "Send it to taimoorhussain.com."),
            ("hello at taimoorhussain dot com", "hello@taimoorhussain.com"),
            ("I roots the email yesterday.", "I wrote the email yesterday."),
            ("their going home now", "They're going home now."),
            ("over their", "Over there."),
            ("there cat sat there", "Their cat sat there."),
            ("I right the email", "I write the email."),
            ("write now please", "Right now please."),
            ("send the male today", "Send the mail today."),
            ("weight for the reply", "Wait for the reply."),
            ("to much detail", "Too much detail."),
            ("for to days", "For two days."),
            // Far from the name. A fuzzy swap here would "fix" unrelated domains.
            ("Send it to Durmusnsusen.com", "Send it to Durmusnsusen.com."),
            ("easy ssh", "EasySSH"),
            ("easy, it's such are", "EasySSH"),
            ("Bolo and Easy, it's such a truth. Nerd thing, imagination.", "EasySSH"),
            ("open easy ssh please", "Open EasySSH please."),
            ("I think it's such a good idea", "I think it's such a good idea."),
            ("ssh into the server", "SSH into the server."),
        ];
        let mut wer_sum = 0.0;
        for (heard, expected) in cases {
            let got = polish_text(heard, &[], &words, true);
            let score = wer(expected, &got);
            wer_sum += score;
            assert_eq!(got, expected, "heard {heard}");
            assert_eq!(score, 0.0, "{heard}");
        }
        let mean = wer_sum / cases.len() as f64;
        eprintln!(
            "ACCURACY text phrases={} mean_wer={mean:.3} model_added_ms=0 (local homophones; sense model skipped)",
            cases.len()
        );
        assert_eq!(mean, 0.0);
    }

    #[test]
    fn fix_last_saves_the_right_word_immediately_and_repairs_the_garble() {
        let mut memory = Memory::default();
        let mut dictionary = Vec::new();
        let (from, to) =
            memory::diff_fix("Send it to Durmusnsusen.com please", "Send it to taimoorhussain.com please").expect("one-word fix");
        let added = memory::record_fix(&mut memory, &mut dictionary, &from, &to, 1);
        assert!(memory.pending.is_empty(), "no twice-seen wait");
        assert!(
            memory.my_words.iter().any(|word| word.eq_ignore_ascii_case("taimoorhussain.com")),
            "{:?}",
            memory.my_words
        );
        assert!(
            memory.blocked.iter().any(|word| word.to_lowercase().contains("durmus")),
            "the mishearing stays out of the prompt: {:?}",
            memory.blocked
        );
        assert!(added.iter().all(|word| !word.to_lowercase().contains("durmus")));
        let ranked = memory::rank_my_words(&memory);
        let list = vocab::protected(&ranked);
        let again = polish_text("Send it to Durmusnsusen.com", &memory.corrections, &list, true);
        assert_eq!(again, "Send it to taimoorhussain.com.");
        assert_eq!(wer("Send it to taimoorhussain.com", &again), 0.0);
        // A second sighting is not required, and a common English fix is not a prompt term.
        assert!(!memory::add_my_word(&mut memory, "wrote"));
        assert!(!memory.my_words.iter().any(|word| word.eq_ignore_ascii_case("wrote")));
    }

    #[test]
    fn most_used_words_lead_the_prompt_and_seeds_still_fit() {
        let mut memory = Memory::default();
        memory.my_words = vec!["RareClient".into(), "Bolo".into(), "DailyClient".into()];
        memory.uses = vec![
            memory::WordUse {
                word: "DailyClient".into(),
                uses: 40,
            },
            memory::WordUse {
                word: "Bolo".into(),
                uses: 10,
            },
            memory::WordUse {
                word: "RareClient".into(),
                uses: 1,
            },
        ];
        let ranked = memory::rank_my_words(&memory);
        assert_eq!(ranked, vec!["DailyClient".to_string(), "Bolo".into(), "RareClient".into()]);
        let mut prompt_terms = vocab::protected(&ranked);
        for index in 0..40 {
            prompt_terms.push(format!("FillerWord{index}XXXXXXXXXXXX"));
        }
        let prompt = transcribe::whisper_prompt_with(&prompt_terms, &[]).expect("prompt");
        assert!(prompt.chars().count() <= 800, "{}", prompt.chars().count());
        let daily = prompt.find("DailyClient").expect("daily");
        let rare = prompt.find("RareClient").expect("rare my word stays ahead of the seed block");
        let kingdom = prompt.find("Kingdom Vision").expect("seed");
        assert!(daily < rare && rare < kingdom, "my words stay ahead of the seeds\n{prompt}");
        assert!(prompt.contains("taimoorhussain.com"));
        assert!(prompt.contains("Kingdom Vision"));
        assert!(!prompt.contains("FillerWord39"), "the tail is what the cap drops\n{prompt}");
        for seed in ["Bolo", "Bolo.com", "Kingdom Vision"] {
            assert!(SEED_WORDS.contains(&seed));
        }
    }

    #[test]
    fn local_pass_latency_is_invisible_on_a_take() {
        let words = words();
        let samples = [
            "Open Volo and start dictating.",
            "hello at taimoorhussain dot com",
            "I roots the email yesterday.",
            "their going home now over their",
            "Send it to taimurhussain.com tomorrow morning please",
        ];
        let rounds = 200usize;
        let started = Instant::now();
        for _ in 0..rounds {
            for sample in samples {
                let _ = polish_text(sample, &[], &words, true);
            }
        }
        let per_us = started.elapsed().as_micros() / (rounds * samples.len()) as u128;
        eprintln!("ACCURACY local_polish_us={per_us} sense_budget_ms={}", sense::SENSE_TIMEOUT_MS);
        assert!(per_us < 2_000, "local polish took {per_us}us");
        assert!(sense::SENSE_TIMEOUT_MS <= 700);
        assert!(crate::grammar::cleanup_timeout_ms(24) <= 1_200);
        assert!(crate::grammar::cleanup_timeout_ms(900) <= 2_200);
    }

    #[test]
    fn urdu_keeps_large_v3_and_english_words_in_a_mixed_sentence() {
        assert_eq!(pipeline::effective_model(WHISPER_TURBO, "auto"), WHISPER_LARGE);
        assert_eq!(pipeline::effective_model(WHISPER_TURBO, "ur"), WHISPER_LARGE);
        assert_eq!(pipeline::effective_model(WHISPER_TURBO, "en"), WHISPER_TURBO);
        let mixed = "میں Bolo office میں ہوں";
        let redo_dropped_english = "میں بولو دفتر میں ہوں";
        assert_eq!(pipeline::preserve_english(mixed, redo_dropped_english), mixed);
        let redo_kept_english = "میں Bolo office میں بہتری ہے";
        assert_eq!(pipeline::preserve_english(mixed, redo_kept_english), redo_kept_english);
        assert_eq!(pipeline::preserve_english("یہ بہتری ہے", "یہ بہتری ہے۔"), "یہ بہتری ہے۔");
        assert!(!transcribe::URDU_PROMPT.chars().any(|c| c.is_ascii_alphabetic()));
    }

    #[test]
    fn silence_and_steady_noise_produce_no_text() {
        use crate::audio::{self, FinishedAudio, SpeechError};
        let rate = 16_000u32;
        let silence = FinishedAudio {
            interleaved: vec![0.0; rate as usize],
            channels: 1,
            sample_rate: rate,
            elapsed_ms: 1000,
            noise_rms: 0.0,
        };
        assert!(matches!(audio::speech_samples(&silence), Err(SpeechError::Silent)));
        let mut state = 0x1234_5678u32;
        let noise: Vec<f32> = (0..rate as usize)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state as f32 / u32::MAX as f32 - 0.5) * 0.08
            })
            .collect();
        let noise = FinishedAudio {
            interleaved: noise,
            channels: 1,
            sample_rate: rate,
            elapsed_ms: 1000,
            noise_rms: 0.0,
        };
        assert!(
            matches!(audio::speech_samples(&noise), Err(SpeechError::Silent)),
            "background noise must not be uploaded"
        );
    }

    #[test]
    fn urdu_wer_folds_nfc_and_drops_punctuation_and_zero_width() {
        assert_eq!(wer("سلام علیکم", "سلام۔ علیکم"), 0.0);
        assert_eq!(wer("سلام علیکم", "سلام، علیکم؟"), 0.0);
        assert_eq!(wer("سلام", "س\u{200b}لام"), 0.0);
        assert_eq!(wer("سلام", "س\u{200c}لام"), 0.0);
        assert_eq!(wer("سلام", "س\u{200d}لام\u{feff}"), 0.0);
        assert!(!tokens("سلام۔،؟").iter().any(|word| word.chars().any(|c| matches!(c, '۔' | '،' | '؟'))));
        let composed = "\u{0622}ج";
        let decomposed = "\u{0627}\u{0653}ج";
        assert_ne!(composed, decomposed);
        assert_eq!(tokens(composed), tokens(decomposed));
        assert_eq!(wer(composed, decomposed), 0.0);
        assert_eq!(wer("बच्चे खेल रहे हैं।", "बच्चे खेल रहे हैं"), 0.0);
        let dropped = wer_score("آج موسم اچھا ہے", "موسم اچھا ہے");
        assert!(dropped.first_dropped);
        assert!(!dropped.last_dropped);
        assert_eq!(
            tokens("priya.sharma@kingdomvision.com"),
            vec!["priya.sharma@kingdomvision.com".to_string()]
        );
        assert_eq!(wer("Send it right now.", "Send it right now"), 0.0);
    }

    /// Live Groq is optional. Without `BOLO_ACCURACY_KEY` this returns before
    /// any network call and does not read or write the user's config or key.
    #[test]
    fn live_groq_sense_is_optional() {
        let Ok(key) = std::env::var("BOLO_ACCURACY_KEY") else {
            eprintln!("ACCURACY live sense skipped (BOLO_ACCURACY_KEY unset)");
            return;
        };
        if key.trim().is_empty() {
            return;
        }
        let words = words();
        let started = Instant::now();
        let outcome = tauri::async_runtime::block_on(sense::model_fix(
            transcribe::http_client(),
            key.trim(),
            "maybe roots the whole file today",
            crate::cleanup::CLEANUP_MODEL,
            &words,
            false,
        ));
        let ms = started.elapsed().as_millis();
        eprintln!(
            "ACCURACY live sense_ms={ms} budget={} why={} text={}",
            sense::SENSE_TIMEOUT_MS,
            outcome.why,
            outcome.text
        );
        assert!(ms < 2_000, "sense call took {ms}ms");
        assert!(
            outcome.text.to_lowercase().contains("bolo") || !outcome.text.to_lowercase().contains("volo"),
            "{}",
            outcome.text
        );
        let _ = words;
    }
}
