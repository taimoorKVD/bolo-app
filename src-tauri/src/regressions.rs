//! Regression tests for bugs that were already fixed.
//! They fail if the old behavior comes back.

use crate::dictation::{after_arm, release_decision, AfterArm, ReleaseDecision, TAP_MS};
use crate::insert;
use crate::pipeline::{has_words, stitch_products, Product};
use crate::startup::{legacy_run_is_stale, LEGACY_EXE, SHIPPED_EXE};
use crate::transcribe::{error_chain, strip_prompt_leak, PREWARM_RETRY_DELAY, PREWARM_TRIES};
use std::error::Error;
use std::fmt;
use std::time::Duration;

#[derive(Debug)]
struct Layer(&'static str, Option<Box<dyn Error + Send + Sync>>);

impl fmt::Display for Layer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for Layer {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.1
            .as_ref()
            .map(|err| err.as_ref() as &(dyn Error + 'static))
    }
}

fn product(text: &str) -> Product {
    Product {
        text: text.into(),
        engine: "Groq".into(),
        tries: 1,
        encode_ms: 0,
        remote_ms: 0,
        bytes: 0,
        format: "flac".into(),
        error: None,
        silence: false,
        language: "en".into(),
        speech_path: "transcribe".into(),
    }
}

#[test]
fn prompt_text_does_not_leak_into_the_transcript() {
    assert_eq!(
        strip_prompt_leak("Recent dictation:\"Hey, can you listen to me?\""),
        "Hey, can you listen to me?"
    );
    assert_eq!(
        strip_prompt_leak("Dictation transcript: \"Ship it tomorrow.\""),
        "Ship it tomorrow."
    );
    assert_eq!(
        strip_prompt_leak(
            "Dictation transcript. Spell these terms exactly when spoken: Bolo, Tauri. Ship it tomorrow."
        ),
        "Ship it tomorrow."
    );
    assert_eq!(
        strip_prompt_leak("Spell these terms exactly when spoken: Bolo."),
        ""
    );
    assert_eq!(strip_prompt_leak("Ship it tomorrow."), "Ship it tomorrow.");
}

#[test]
fn a_lone_punctuation_mark_is_not_pasted() {
    for mark in [".", "?", "!", "۔", "...", "\"", " ", "!!"] {
        assert!(!has_words(mark), "{mark:?} is not speech");
        let err = insert::insert_text(mark).expect_err("punctuation must not be inserted");
        assert!(
            err.to_string().contains("nothing to insert"),
            "{mark:?} -> {err}"
        );
    }
    assert_eq!(stitch_products(&[product("."), product("?")]).unwrap(), "");
    assert_eq!(
        stitch_products(&[product("hello"), product(".")]).unwrap(),
        "hello"
    );
    assert!(has_words("ok."));
}

#[test]
fn hold_to_talk_release_during_arm_does_not_drop_the_take() {
    // The old bug treated key-up while the microphone was still opening as a
    // zero-length tap, dropped the audio, and the overlay said "Not inserted".
    assert_eq!(
        release_decision(false, true, false, 0, true),
        ReleaseDecision::NoteArmRelease
    );
    assert_eq!(
        release_decision(false, false, false, 0, true),
        ReleaseDecision::NoteArmRelease
    );
    assert_eq!(
        after_arm(true, 800),
        AfterArm::EndTake,
        "a real hold still finishes"
    );
    assert_eq!(
        after_arm(false, 800),
        AfterArm::Listen,
        "key still down keeps listening"
    );
    assert_eq!(after_arm(true, 80), AfterArm::WaitSecondTap);
    assert_eq!(
        release_decision(false, true, true, TAP_MS, false),
        ReleaseDecision::EndNow,
        "a normal hold ends on release with no extra wait"
    );
}

#[test]
fn prewarm_retries_once_and_the_connection_cause_is_kept() {
    assert_eq!(PREWARM_TRIES, 2, "one retry after a failed warmup");
    assert!(PREWARM_RETRY_DELAY >= Duration::from_secs(1));
    let err = Layer(
        "error sending request",
        Some(Box::new(Layer("dns error: No such host is known.", None))),
    );
    let chain = error_chain(&err);
    assert!(chain.contains("dns error"), "{chain}");
    assert!(chain.contains("No such host"), "{chain}");
}

#[test]
fn version_strings_and_the_update_identity_match_this_release() {
    let tauri = include_str!("../tauri.conf.json");
    let cargo = include_str!("../Cargo.toml");
    let package = include_str!("../../package.json");
    let lock = include_str!("../../package-lock.json");
    let mock = include_str!("../../src/mock.ts");
    let version = json_version(tauri).expect("tauri version");
    assert_eq!(json_version(package).as_deref(), Some(version.as_str()));
    assert_eq!(json_version(lock).as_deref(), Some(version.as_str()));
    assert!(cargo.contains(&format!("version = \"{version}\"")));
    assert!(mock.contains(&format!("version: \"{version}\"")));
    // 0.1.6 installs live in $LOCALAPPDATA\Bolo.com and unregister under that
    // product name. Renaming it would install a second copy beside the old one.
    assert!(tauri.contains("\"productName\": \"Bolo.com\""));
    assert!(tauri.contains("\"mainBinaryName\": \"bolo\""));
    assert!(tauri.contains("\"identifier\": \"com.bolo.dictation\""));
    assert!(tauri.contains("https://bolo.thebetawebsite.com/updates/latest.json"));
    assert!(cargo.contains("name = \"bolo\""));
    assert!(!cargo.contains("name = \"cadence\""));
    assert_eq!(SHIPPED_EXE, "bolo.exe");
    assert_eq!(LEGACY_EXE, "cadence.exe");
    let old = r"C:\Users\A\AppData\Local\Bolo.com\cadence.exe";
    let current = r"C:\Users\A\AppData\Local\Bolo.com\bolo.exe";
    assert!(legacy_run_is_stale(old, current));
    assert!(!legacy_run_is_stale(current, current));
}

fn json_version(text: &str) -> Option<String> {
    let key = "\"version\":";
    let index = text.find(key)?;
    let rest = text[index + key.len()..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[test]
fn store_manifest_keeps_the_package_identity() {
    let script = include_str!("../../scripts/build-msix.ps1");
    assert!(script.contains("Identity Name=\"$IdentityName\""));
    assert!(script.contains("Publisher=\"$Publisher\""));
    assert!(script.contains("<DisplayName>Bolo</DisplayName>"));
    assert!(script.contains("Executable=\"Bolo.exe\""));
    assert!(script.contains("bolo.exe"));
    assert!(!script.contains("cadence.exe"));
}
