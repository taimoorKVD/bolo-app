//! Offline text-stage gate. These tests always run in `cargo test` and in CI.
//! They do not call Groq and they do not read a microphone.

use crate::accuracy;
use crate::grammar;
use crate::memory::{self, Correction};
use crate::store::{self, Settings};
use crate::transcribe::{self, PROMPT_TERM_BUDGET};
use crate::vocab::{self, SEED_WORDS};

/// Correct English that grammar must keep (capitalization / period only).
const GRAMMAR_MUST_KEEP: &[(&str, &str)] = &[
    ("travel ID", "Travel ID."),
    ("Does it have", "Does it have."),
    ("Let it go", "Let it go."),
    ("If I were you", "If I were you."),
];

#[test]
fn gate_grammar_keeps_correct_english() {
    for (input, correct) in GRAMMAR_MUST_KEEP {
        assert_eq!(grammar::polish(input), *correct, "{input}");
    }
}

#[test]
fn gate_grammar_still_keeps_sentences_it_already_handles() {
    assert_eq!(
        grammar::polish("I wrote the email yesterday."),
        "I wrote the email yesterday."
    );
    assert_eq!(
        grammar::polish("The roots of the tree are deep."),
        "The roots of the tree are deep."
    );
    assert_eq!(
        grammar::polish("she don't like the update"),
        "She doesn't like the update."
    );
}

#[test]
fn gate_grammar_leaves_urdu_and_mixed_roman_alone() {
    assert_eq!(grammar::polish("میں ٹھیک ہوں"), "میں ٹھیک ہوں");
    assert_eq!(grammar::polish("bolo kya haal hai"), "bolo kya haal hai");
    assert_eq!(
        grammar::polish("acha please karo yeh update"),
        "acha please karo yeh update"
    );
    // Two Roman Urdu markers → skip English grammar entirely (no period / caps).
    let mixed = "meeting kal hai please karo";
    assert_eq!(grammar::polish(mixed), mixed);
    let stages = accuracy::local_stages(mixed, &[], &[], true);
    assert_eq!(stages.grammar, mixed);
    assert!(!stages.grammar.contains("Meeting"));
}

#[test]
fn gate_learned_spellings_fix_a_taught_word_and_leave_travel_id() {
    let words = vocab::protected(&[]);
    assert_eq!(
        vocab::correct("Open Volo and start", &words),
        "Open Bolo and start"
    );
    let corrections = [Correction {
        from: "Hostfinger".into(),
        to: "Hostinger".into(),
        created_unix_ms: 1,
    }];
    assert_eq!(
        memory::apply_corrections("Open Hostfinger today", &corrections),
        "Open Hostinger today"
    );
    let stages = accuracy::local_stages("My travel ID is on the desk.", &[], &words, true);
    assert!(
        stages.learned.to_lowercase().contains(" id ") || stages.learned.contains("ID"),
        "learned spellings changed travel ID: {}",
        stages.learned
    );
    assert!(!stages.learned.contains("I'd"), "{}", stages.learned);
    assert!(
        !stages.grammar.contains("I'd"),
        "grammar must keep travel ID: {}",
        stages.grammar
    );
    assert!(
        stages.grammar.contains("ID"),
        "grammar output changed: {}",
        stages.grammar
    );
    assert_eq!(
        accuracy::polish_text("Open Volo", &[], &words, true),
        stages_match("Open Volo", &words)
    );
}

fn stages_match(text: &str, words: &[String]) -> String {
    accuracy::local_stages(text, &[], words, true).grammar
}

#[test]
fn gate_prompt_keeps_seeds_when_the_list_is_short() {
    let prompt = transcribe::whisper_prompt(&["Bolo".into(), "taimoorhussain.com".into()]).unwrap();
    assert!(prompt.contains("Bolo"));
    assert!(prompt.contains("taimoorhussain.com"));
    assert!(prompt.chars().count() <= PROMPT_TERM_BUDGET + 8);
}

#[test]
fn gate_prompt_keeps_bilingual_seeds_when_mishearings_fill_the_list() {
    // Ranked My words (here, auto-learned mishearings), then the seed run.
    // Seeds keep a reserved slice so Plesk / WhatsApp / MySQL stay available
    // for mixed Urdu/English takes.
    let mut words = Vec::new();
    for n in 0..12 {
        words.push(format!("Hostfinger{n}"));
    }
    words.push("Volo".into());
    for seed in SEED_WORDS {
        words.push((*seed).to_string());
    }
    let prompt = transcribe::whisper_prompt(&words).expect("prompt");
    assert!(
        prompt.chars().count() <= PROMPT_TERM_BUDGET + 8,
        "prompt grew past the budget: {}",
        prompt.chars().count()
    );
    let lower = prompt.to_lowercase();
    assert!(
        lower.contains("hostfinger") || lower.contains("volo"),
        "leading terms should still appear\n{prompt}"
    );
    for term in ["plesk", "whatsapp", "mysql", "bolo"] {
        assert!(lower.contains(term), "seed {term} missing from prompt\n{prompt}");
    }
}

#[test]
fn gate_debug_capture_defaults_off_and_old_settings_stay_off() {
    assert!(!Settings::default().debug_capture);
    let dir = std::env::temp_dir().join(format!("bolo-gate-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("settings.json"), br#"{"language":"en"}"#).unwrap();
    let loaded = store::load_settings(&dir);
    assert!(!loaded.debug_capture);
    assert_eq!(loaded.language, "en");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gate_script_covers_the_unmeasured_cases() {
    let lines = crate::measure::script();
    assert!(lines.len() >= 30, "{}", lines.len());
    let mut ids = lines.iter().map(|line| line.id).collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), lines.len(), "duplicate script ids");
    let say = lines
        .iter()
        .map(|line| line.say.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    for term in [
        "bolo",
        "taimoorhussain.com",
        "thebetawebsite.com",
        "plesk",
        "whatsapp",
        "mysql",
        "kingdom vision",
        "travel id",
        "does it have",
        "if i were you",
        "let it go",
        "wrote",
        "roots",
    ] {
        assert!(say.contains(term), "script missing {term}");
    }
    assert!(lines
        .iter()
        .any(|line| line.kind == "short" && line.say.split_whitespace().count() <= 2));
    assert!(lines.iter().any(|line| line.kind == "mixed"));
    assert!(lines.iter().any(|line| line.kind == "fast"));
    assert!(lines.iter().any(|line| line.kind == "pause"));
    assert!(lines
        .iter()
        .any(|line| line.kind == "edges" && line.say.to_lowercase().starts_with("pineapple")));
    assert!(lines
        .iter()
        .any(|line| line.say.to_lowercase().ends_with("apricot.")));
    let long = lines
        .iter()
        .find(|line| line.kind == "long")
        .expect("long take");
    assert!(
        long.say.split_whitespace().count() >= 100,
        "{}",
        long.say.split_whitespace().count()
    );
    assert!(long.cue.to_lowercase().contains("forty-five"));
}
