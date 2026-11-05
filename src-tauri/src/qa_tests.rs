//! Module-by-module QA coverage for pure logic: hotkeys, settings, history,
//! transcription routing (incl. Urdu / large-v3), retries, cleanup, insertion
//! guards, chunking, offline model, characters and sound packs.
//! Nothing here touches Credential Manager, the network, the mic, or the desktop.

use crate::chunk::{overlap_samples, should_cut, stitch};
use crate::cleanup::{
    accept_cleanup, apply_commands, apply_self_correction, learn_terms, polish_locally,
    polish_model_output, strip_fillers,
};
use crate::hotkey::*;
use crate::insert::{status_word, text_landed, InsertMethod};
use crate::offline;
use crate::pipeline::{effective_model, engine_label, has_words, stitch_products, unlikely_language, Product};
use crate::store::*;
use crate::transcribe::*;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bolo-qa-{tag}-{}-{}", std::process::id(), now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hk(ctrl: bool, alt: bool, shift: bool, win: bool, key: Option<&str>) -> Hotkey {
    Hotkey { ctrl, alt, shift, win, key: key.map(str::to_string) }
}

fn snap(kinds: &[KeyKind]) -> KeySnapshot {
    snapshot_from_kinds(kinds)
}

// ---------- Shortcut page / hotkey ----------

#[test]
fn hotkey_validation_matches_the_shortcut_page_rules() {
    assert!(validate_hotkey(&Hotkey::default()).is_ok());
    assert!(validate_hotkey(&Hotkey::paste_again()).is_ok());
    assert!(validate_hotkey(&hk(true, false, false, false, Some("space"))).is_ok());
    assert!(validate_hotkey(&hk(false, true, false, false, Some("f12"))).is_ok());
    assert!(validate_hotkey(&hk(false, false, false, false, Some("space"))).is_err(), "no modifier");
    assert!(validate_hotkey(&hk(true, false, false, false, None)).is_err(), "lone Ctrl");
    assert!(validate_hotkey(&hk(true, false, false, false, Some("f13"))).is_err());
    assert!(validate_hotkey(&hk(true, false, false, false, Some("@"))).is_err());
    assert!(validate_hotkey(&hk(true, false, false, false, Some("f0"))).is_err());
}

#[test]
fn hotkey_display_reads_like_the_ui() {
    assert_eq!(display(&Hotkey::default()), "Ctrl + Win");
    assert_eq!(display(&Hotkey::paste_again()), "Ctrl + Win + V");
    assert_eq!(display(&hk(true, true, true, false, Some("f8"))), "Ctrl + Alt + Shift + F8");
    assert_eq!(display(&hk(false, false, false, false, None)), "Not set");
    assert_eq!(pretty_key("pageup"), "Page Up");
    assert_eq!(pretty_key("esc"), "Esc");
}

#[test]
fn ctrl_win_is_not_held_with_an_extra_key_or_a_missing_modifier() {
    let h = Hotkey::default();
    assert!(is_held(&snap(&[KeyKind::Ctrl, KeyKind::Win]), &h));
    assert!(!is_held(&snap(&[KeyKind::Ctrl]), &h));
    assert!(!is_held(&snap(&[KeyKind::Ctrl, KeyKind::Win, KeyKind::Shift]), &h));
    assert!(!is_held(&snap(&[KeyKind::Ctrl, KeyKind::Win, KeyKind::Named("a".into())]), &h));
}

#[test]
fn plain_ctrl_and_typing_are_never_swallowed() {
    let h = Hotkey::default();
    // Ctrl alone (Ctrl+C etc.) must reach the app.
    assert!(!should_swallow_keydown(&KeyKind::Ctrl, &snap(&[KeyKind::Ctrl]), &h));
    assert!(!should_swallow_keydown(&KeyKind::Named("c".into()), &snap(&[KeyKind::Ctrl, KeyKind::Named("c".into())]), &h));
    // Completing the chord is swallowed so Start does not open.
    assert!(should_swallow_keydown(&KeyKind::Win, &snap(&[KeyKind::Ctrl, KeyKind::Win]), &h));
}

#[test]
fn hold_release_hold_emits_clean_edges_and_pause_is_silent() {
    let mut edges = EdgeListener::default();
    assert_eq!(edges.observe(true, false), Some(HotkeyEdge::Pressed));
    assert_eq!(edges.observe(true, false), None, "auto-repeat");
    assert_eq!(edges.observe(false, false), Some(HotkeyEdge::Released));
    assert_eq!(edges.observe(false, false), None);
    assert_eq!(edges.observe(true, true), None, "paused while capturing a new shortcut");
    assert!(edges.holding());
    assert_eq!(edges.observe(true, false), None, "no fake press after unpausing mid-hold");
}

#[test]
fn win_and_ctrl_virtual_keys_fold_correctly() {
    for vk in [0x5Bu16, 0x5C] {
        assert_eq!(kind_from_vk(vk), KeyKind::Win);
    }
    for vk in [0x11u16, 0xA2, 0xA3] {
        assert_eq!(kind_from_vk(vk), KeyKind::Ctrl);
    }
    for vk in [0x10u16, 0xA0, 0xA1] {
        assert_eq!(kind_from_vk(vk), KeyKind::Shift);
    }
}

// ---------- Settings save / load (every page) ----------

#[test]
fn default_settings_are_valid_and_zero_setup() {
    let s = Settings::default();
    assert!(validate_settings(&s).is_ok());
    assert_eq!(s.language, "auto");
    assert_eq!(s.output, "english");
    assert_eq!(s.character, "classic");
    assert_eq!(s.sound_pack, "match");
    assert!(s.history_enabled && s.cleanup_enabled && s.launch_at_startup);
    assert!(s.microphone.is_none(), "auto mic so a layman never has to pick one");
}

#[test]
fn every_option_survives_save_and_reload() {
    let dir = tmp("roundtrip");
    let mut s = Settings::default();
    s.hotkey = hk(true, false, true, false, Some("space"));
    s.paste_hotkey = hk(true, true, false, false, Some("v"));
    s.microphone = Some("USB Mic".into());
    s.model = WHISPER_LARGE.into();
    s.language = "ur".into();
    s.output = "same".into();
    s.cleanup_enabled = false;
    s.verbatim = true;
    s.history_enabled = false;
    s.launch_at_startup = false;
    s.offline_model = "small.en".into();
    s.dictionary = vec!["Bolo".into(), "Taimoor".into()];
    s.character = "c17".into();
    s.sound_pack = "robot".into();
    s.sound_volume = 0.25;
    s.smart_mood = true;
    s.smart_time_of_day = true;
    s.smart_holiday = true;
    save_settings(&dir, &s).unwrap();
    assert_eq!(load_settings(&dir), s);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn invalid_settings_are_rejected_and_nothing_is_written() {
    let dir = tmp("invalid");
    let mut s = Settings::default();
    s.hotkey = hk(true, false, false, false, None);
    assert!(save_settings(&dir, &s).is_err());
    assert!(!dir.join("settings.json").exists());
    let mut s = Settings::default();
    s.language = "english".into();
    assert!(validate_settings(&s).is_err());
    let mut s = Settings::default();
    s.output = "french".into();
    assert!(validate_settings(&s).is_err());
    let mut s = Settings::default();
    s.cleanup_model = "bad model!".into();
    assert!(validate_settings(&s).is_err());
    let mut s = Settings::default();
    s.dictionary = vec!["x".repeat(500)];
    assert!(validate_settings(&s).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn corrupt_or_missing_settings_file_falls_back_to_defaults() {
    let dir = tmp("corrupt");
    assert_eq!(load_settings(&dir), Settings::default());
    std::fs::write(dir.join("settings.json"), b"{not json").unwrap();
    assert_eq!(load_settings(&dir), Settings::default());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hand_edited_bad_values_self_heal_on_load() {
    let dir = tmp("heal");
    std::fs::write(
        dir.join("settings.json"),
        br#"{"model":"whisper-9","language":"English","hotkey":{"ctrl":true},"pasteHotkey":{},"cleanupModel":"no spaces allowed","character":"c99","soundPack":"LOUD","soundVolume":7,"output":"klingon","offlineModel":"huge","microphone":"   "}"#,
    )
    .unwrap();
    let s = load_settings(&dir);
    assert_eq!(s.model, WHISPER_TURBO);
    assert_eq!(s.language, "auto");
    assert_eq!(s.hotkey, Hotkey::default());
    assert_eq!(s.paste_hotkey, Hotkey::paste_again());
    assert_eq!(s.cleanup_model, crate::cleanup::CLEANUP_MODEL);
    assert_eq!(s.character, "classic");
    assert_eq!(s.sound_pack, "match");
    assert_eq!(s.sound_volume, 1.0);
    assert_eq!(s.output, "english");
    assert_eq!(s.offline_model, "base.en");
    assert_eq!(s.microphone, None);
    assert!(validate_settings(&s).is_ok(), "healed settings must be saveable");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn prepare_trims_what_the_ui_sends() {
    let mut s = Settings::default();
    s.language = "  UR ".into();
    s.microphone = Some("   ".into());
    s.hotkey.key = Some(" SPACE ".into());
    s.dictionary = vec!["  Bolo ".into(), "   ".into()];
    s.character = " C05 ".into();
    s.sound_volume = -3.0;
    let s = prepare_settings(s);
    assert_eq!(s.language, "ur");
    assert_eq!(s.microphone, None);
    assert_eq!(s.hotkey.key.as_deref(), Some("space"));
    assert_eq!(s.dictionary, vec!["Bolo".to_string()]);
    assert_eq!(s.character, "c05");
    assert_eq!(s.sound_volume, 0.0);
    let mut nan = Settings::default();
    nan.sound_volume = f64::NAN;
    assert_eq!(prepare_settings(nan).sound_volume, 0.6);
}

// ---------- Character & sounds ----------

fn assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("src").join("assets")
}

#[test]
fn every_selectable_character_has_its_listen_and_write_clips() {
    for clip in ["listen.webm", "write.webm", "hide.webm", "thumb.png"] {
        assert!(assets().join("characters").join("classic").join(clip).is_file(), "classic {clip}");
    }
    for n in 2..=28 {
        let id = format!("c{n:02}");
        let mut s = Settings::default();
        s.character = id.clone();
        assert_eq!(prepare_settings(s).character, id, "{id} must be accepted");
        // Every character is re-made without a background: three clips and a still.
        for clip in ["listen.webm", "write.webm", "hide.webm", "thumb.png"] {
            let path = assets().join("characters").join(&id).join(clip);
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            assert!(len > 1024, "{} missing or empty", path.display());
        }
    }
    let mut s = Settings::default();
    s.character = "c29".into();
    assert_eq!(prepare_settings(s).character, "classic");
}

#[test]
fn every_sound_pack_has_start_done_and_dismiss() {
    for pack in ["soft-chime", "chiptune", "robot", "magic", "wood", "paper", "neon"] {
        let mut s = Settings::default();
        s.sound_pack = pack.to_uppercase();
        assert_eq!(prepare_settings(s).sound_pack, pack);
        for cue in ["start.wav", "done.wav", "dismiss.wav"] {
            let path = assets().join("sounds").join(pack).join(cue);
            let bytes = std::fs::read(&path).unwrap_or_default();
            assert!(bytes.len() > 44 && &bytes[0..4] == b"RIFF", "{} is not a WAV", path.display());
        }
    }
    for id in ["match", "off"] {
        let mut s = Settings::default();
        s.sound_pack = id.into();
        assert_eq!(prepare_settings(s).sound_pack, id);
    }
}

// ---------- History ----------

#[test]
fn history_is_newest_first_capped_and_long_text_is_cut() {
    let mut entries = Vec::new();
    for i in 0..60 {
        push_history(
            &mut entries,
            HistoryEntry { id: new_history_id(), text: format!("take {i}"), created_unix_ms: i, model: WHISPER_TURBO.into(), cleaned: true, duration_ms: 100 , audio_id: None},
        );
    }
    assert_eq!(entries.len(), 50);
    assert_eq!(entries[0].text, "take 59");
    push_history(
        &mut entries,
        HistoryEntry { id: new_history_id(), text: "a".repeat(25_000), created_unix_ms: 0, model: String::new(), cleaned: false, duration_ms: 0 , audio_id: None},
    );
    assert_eq!(entries[0].text.chars().count(), 20_001);
    let ids: std::collections::HashSet<_> = entries.iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids.len(), entries.len(), "ids are unique");
}

#[test]
fn history_round_trips_on_disk_and_survives_corruption() {
    let dir = tmp("history");
    let entry = HistoryEntry { id: new_history_id(), text: "میں ٹھیک ہوں".into(), created_unix_ms: 1, model: WHISPER_LARGE.into(), cleaned: false, duration_ms: 900 , audio_id: None};
    save_history(&dir, &[entry.clone()]).unwrap();
    assert_eq!(load_history(&dir), vec![entry]);
    std::fs::write(dir.join("history.json"), b"[{").unwrap();
    assert!(load_history(&dir).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

// ---------- Languages / transcription routing ----------

#[test]
fn urdu_and_auto_use_large_v3_and_english_keeps_turbo() {
    assert_eq!(effective_model(WHISPER_TURBO, "auto"), WHISPER_LARGE);
    assert_eq!(effective_model(WHISPER_TURBO, "ur"), WHISPER_LARGE);
    assert_eq!(effective_model(WHISPER_TURBO, "en"), WHISPER_TURBO);
    assert_eq!(effective_model(WHISPER_LARGE, "en"), WHISPER_LARGE);
    assert_eq!(normalize_model("whisper-1"), None);
}

#[test]
fn speech_routes_for_every_language_option() {
    assert_eq!(speech_route("same", "ur", "urdu", "میں ٹھیک ہوں"), SpeechRoute::Transcribe);
    assert_eq!(speech_route("english", "en", "english", "hello"), SpeechRoute::Transcribe);
    assert_eq!(speech_route("english", "auto", "english", "hello"), SpeechRoute::Transcribe);
    assert_eq!(speech_route("english", "auto", "urdu", "میں ٹھیک ہوں"), SpeechRoute::Translate);
    assert_eq!(speech_route("english", "ur", "", "میں office جا رہا ہوں"), SpeechRoute::Mixed);
    assert_eq!(speech_route("english", "auto", "french", "bonjour"), SpeechRoute::Translate);
    assert!(direct_translate("english", "fr"));
    assert!(!direct_translate("english", "ur"));
    assert!(!direct_translate("english", "en"));
    assert!(!direct_translate("same", "fr"));
    assert_eq!(language_code("Urdu"), "ur");
    assert_eq!(language_code(" Farsi "), "fa");
    assert!(unlikely_language("auto", "french"));
    assert!(!unlikely_language("auto", "urdu"));
    assert!(!unlikely_language("fr", "french"));
}

#[test]
fn rate_limit_waits_parse_compound_durations() {
    assert_eq!(retry_after_ms("Please try again in 250ms."), Some(250));
    assert_eq!(retry_after_ms("Please try again in 2.5s."), Some(2500));
    assert_eq!(retry_after_ms("Please try again in 1m30s."), Some(90_000));
    assert_eq!(retry_after_ms("Please try again in 7m12.5s. Need more"), Some(432_500));
    assert_eq!(retry_after_ms("rate limited"), None);
}

#[test]
fn retry_plan_backs_off_then_falls_back_to_this_pc() {
    assert_eq!(retry_plan(FailKind::RateLimit, 0), RetryPlan::Backoff { ms: 700 });
    assert_eq!(retry_plan(FailKind::RateLimit, 1), RetryPlan::Stop { fallback: true });
    assert_eq!(retry_plan(FailKind::Unavailable, 0), RetryPlan::Backoff { ms: 250 });
    assert_eq!(retry_plan(FailKind::Unavailable, 1), RetryPlan::Backoff { ms: 750 });
    assert_eq!(retry_plan(FailKind::Unavailable, 2), RetryPlan::Stop { fallback: true });
    assert_eq!(retry_plan(FailKind::Rejected, 0), RetryPlan::Backoff { ms: 400 }, "a rejected key is retried once");
    assert_eq!(retry_plan(FailKind::Rejected, 1), RetryPlan::Stop { fallback: true }, "then the on-device model takes the chunk");
    assert_eq!(retry_plan(FailKind::QuotaExhausted, 0), RetryPlan::Stop { fallback: true });
    assert_eq!(retry_plan(FailKind::QuotaExhausted, 3), RetryPlan::Stop { fallback: true });
}

#[test]
fn hallucinations_on_silence_are_caught_but_real_speech_is_not() {
    assert!(stock_phrase("Thanks for watching!"));
    assert!(stock_phrase("Bye."));
    assert!(!stock_phrase("Thanks for the update"));
    assert!(should_retry("ok", Some(-1.5), None));
    assert!(should_retry("you", None, Some(0.9)));
    assert!(!should_retry("send the report today", Some(-0.2), Some(0.9)));
}

// ---------- Cleanup ----------

#[test]
fn ordinary_words_are_not_turned_into_punctuation() {
    assert_eq!(apply_commands("the trial period ended yesterday"), "the trial period ended yesterday");
    assert_eq!(apply_commands("add a dash of salt"), "add a dash of salt");
    assert_eq!(apply_commands("Send it today period"), "Send it today.");
    assert_eq!(apply_commands("hello comma how are you"), "hello, how are you");
    assert_eq!(apply_commands("one new paragraph two"), "one\n\ntwo");
}

#[test]
fn model_output_is_not_mangled_by_the_local_pass() {
    assert_eq!(polish_model_output("Your free period starts on the first, comma included."), "Your free period starts on the first, comma included.");
    assert_eq!(polish_model_output("What I mean is that we ship today."), "What I mean is that we ship today.");
    assert_eq!(polish_model_output("Is this right question mark"), "Is this right?");
}

#[test]
fn i_mean_only_corrects_when_it_is_a_correction() {
    assert_eq!(apply_self_correction("Email Sarah I mean Sam"), "Email Sam");
    assert_eq!(apply_self_correction("What I mean is simple"), "What I mean is simple");
    assert_eq!(apply_self_correction("Stop it I mean it"), "Stop it I mean it");
    assert_eq!(polish_locally("um I think so"), "I think so");
}

#[test]
fn cleanup_never_accepts_added_words() {
    assert!(accept_cleanup("ship it tomorrow", "Ship it tomorrow."));
    assert!(!accept_cleanup("ship it tomorrow", "Ship it tomorrow, boss."));
    assert!(!accept_cleanup("ship it tomorrow", ""));
    assert_eq!(strip_fillers("uh-huh that works umm"), "uh-huh that works");
}

#[test]
fn learning_adds_at_most_three_names_per_take() {
    let mut dict = Vec::new();
    learn_terms("Met Priya Rahul Ahmed Fatima today", &mut dict);
    let added = learn_terms("Met Priya Rahul Ahmed Fatima today", &mut dict);
    assert_eq!(added.len(), 3);
}

// ---------- Insert / paste guards ----------

#[test]
fn lone_punctuation_and_urdu_full_stop_are_not_speech() {
    for t in [".", "?", "۔", " ... ", "", "!!"] {
        assert!(!has_words(t), "{t:?}");
    }
    for t in ["ہاں", "42", "ok"] {
        assert!(has_words(t));
    }
}

fn product(text: &str, engine: &str, error: Option<&str>) -> Product {
    Product {
        text: text.into(),
        engine: engine.into(),
        tries: 1,
        encode_ms: 0,
        remote_ms: 0,
        bytes: 0,
        format: "wav".into(),
        error: error.map(str::to_string),
        silence: false,
        language: String::new(),
        speech_path: String::new(),
    }
}

#[test]
fn stitching_drops_punctuation_only_chunks_and_reports_failures() {
    assert_eq!(stitch_products(&[product(".", "Groq", None)]), Ok(String::new()));
    assert_eq!(stitch_products(&[product("hello", "Groq", None), product("?", "Groq", None)]), Ok("hello".into()));
    assert!(stitch_products(&[product("", "Groq", Some("boom"))]).is_err());
    assert_eq!(engine_label(&[product("a", "Groq", None), product("b", "On this PC", None)]).as_deref(), Some("Groq + on this PC"));
    assert_eq!(engine_label(&[]), None);
}

#[test]
fn paste_status_and_landing_check() {
    assert_eq!(status_word(InsertMethod::Clipboard, true), "Inserted");
    assert_eq!(status_word(InsertMethod::Clipboard, false), "Typed");
    assert_eq!(status_word(InsertMethod::Typed, true), "Typed");
    assert!(text_landed("", "hello", "hello"));
    assert!(!text_landed("hello", "hello", "hello"), "unchanged control");
    assert!(!text_landed("", "", "  "), "empty phrase");
    assert!(!text_landed("abc", "abcd", "hello"));
}

// ---------- Long takes / chunking / offline fallback ----------

#[test]
fn chunk_cuts_on_pause_after_20s_or_forced_at_30s() {
    assert!(!should_cut(19_999, 5_000));
    assert!(should_cut(20_000, 280));
    assert!(!should_cut(25_000, 100));
    assert!(should_cut(30_000, 0));
    assert_eq!(overlap_samples(16_000, 1), 12_800);
    assert_eq!(overlap_samples(16_000, 0), 12_800);
    assert_eq!(stitch(["  ", "hi there", ""]), "hi there");
}

#[test]
fn offline_model_names_normalize() {
    assert_eq!(offline::normalize_model("Small"), "small.en");
    assert_eq!(offline::normalize_model("none"), "off");
    assert_eq!(offline::normalize_model("???"), "base.en");
    assert_eq!(offline::model_label("off"), "Off");
}
#[test]
fn paste_hotkey_equal_to_dictation_hotkey_is_rejected() {
    let mut s = Settings::default();
    s.paste_hotkey = s.hotkey.clone();
    assert!(crate::store::validate_settings(&s).is_err());
}

#[test]
fn a_failed_chunk_keeps_the_chunks_that_worked() {
    let (text, note) = crate::pipeline::stitch_partial(&[
        product("ship the notes", "Groq", None),
        product("", "Groq", Some("boom")),
        product("before Thursday", "Groq", None),
    ])
    .unwrap();
    assert!(text.contains("ship the notes") && text.contains("before Thursday"));
    assert_eq!(note.as_deref(), Some(crate::pipeline::PARTIAL_NOTE));
    assert!(crate::pipeline::stitch_partial(&[product("", "Groq", Some("boom"))]).is_err());
    assert_eq!(crate::pipeline::stitch_partial(&[product("hi there", "Groq", None)]).unwrap(), ("hi there".to_string(), None));
}

#[test]
fn unknown_on_device_model_is_refused_on_save() {
    let mut s = Settings::default();
    s.offline_model = "huge".into();
    assert!(crate::store::validate_settings(&s).is_err());
    for ok in ["off", "base.en", "small.en"] {
        s.offline_model = ok.into();
        assert!(crate::store::validate_settings(&s).is_ok(), "{ok}");
    }
}
