//! Offline text-stage gate. These tests always run in `cargo test` and in CI.
//! They do not call Groq and they do not read a microphone.
//!
//! The four grammar cases below are already-correct sentences. `grammar::polish`
//! rewrites them. Fixing `grammar.rs` is out of scope for this change, so the
//! test records the current wrong text and fails if that text drifts or if the
//! sentence is finally kept (remove the case from the known-failure list then).

use crate::accuracy;
use crate::grammar;
use crate::memory::{self, Correction};
use crate::store::{self, Settings};
use crate::transcribe::{self, PROMPT_TERM_BUDGET};
use crate::vocab::{self, SEED_WORDS};

/// input, text a correct grammar pass would keep, text polish emits today.
const GRAMMAR_KNOWN: &[(&str, &str, &str)] = &[
    ("travel ID", "Travel ID.", "Travel I'd."),
    ("Does it have", "Does it have.", "Does it has."),
    ("Let it go", "Let it go.", "Let it goes."),
    ("If I were you", "If I were you.", "If I was you."),
];

#[test]
fn gate_grammar_known_failures_stay_visible() {
    for (input, correct, current) in GRAMMAR_KNOWN {
        let got = grammar::polish(input);
        if got == *correct {
            panic!(
                "{input:?} is now kept. Remove it from GRAMMAR_KNOWN. Fixing grammar.rs was out of scope; this PR only measures it.\nGot: {got}"
            );
        }
        assert_eq!(
            got, *current,
            "grammar.rs changed {input:?}. Update the known-failure note in accuracy_gate.rs and say whether the corruption is fixed. Got: {got}"
        );
        eprintln!("KNOWN FAILURE (grammar.rs, not fixed here): {input:?} -> {got:?}, correct would be {correct:?}");
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
        stages.learned.to_lowercase().contains(" id ") || stages.learned.ends_with("ID"),
        "learned spellings changed travel ID: {}",
        stages.learned
    );
    assert!(!stages.learned.contains("I'd"), "{}", stages.learned);
    assert_ne!(
        stages.grammar, stages.learned,
        "grammar did not change travel ID; the known failure may be fixed"
    );
    assert!(
        stages.grammar.contains("I'd"),
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
fn gate_prompt_known_failure_mishearings_crowd_out_real_terms() {
    // Same order prompt_words uses: ranked My words (here, auto-learned
    // mishearings), then the seed run, then other real terms.
    let mut words = Vec::new();
    for n in 0..40 {
        words.push(format!("Hostfinger{n}"));
    }
    words.push("Volo".into());
    for seed in SEED_WORDS {
        words.push((*seed).to_string());
    }
    for term in ["Plesk", "WhatsApp", "MySQL"] {
        words.push((*term).to_string());
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
        "mishearings no longer fill the prompt\n{prompt}"
    );
    let missing: Vec<&str> = ["Plesk", "WhatsApp", "MySQL"]
        .into_iter()
        .filter(|term| !lower.contains(&term.to_lowercase()))
        .collect();
    if missing.is_empty() {
        panic!("the prompt now includes Plesk, WhatsApp, and MySQL. Remove this known failure.\n{prompt}");
    }
    eprintln!(
        "KNOWN FAILURE (prompt truncated to ~{PROMPT_TERM_BUDGET} chars, not fixed here): missing {missing:?}"
    );
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
