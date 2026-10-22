//! Live Groq checks. They run only when GROQ_API_KEY is set; otherwise they
//! print SKIP and pass so CI without the secret stays green.

use crate::audio::{self, FinishedAudio};
use crate::cleanup;
use crate::transcribe;
use std::future::Future;
use std::io::Cursor;

const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/dictation.wav");

fn require_key(test: &str) -> Option<String> {
    match std::env::var("GROQ_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key.trim().to_string()),
        _ => {
            eprintln!("SKIP {test}: GROQ_API_KEY is not set");
            None
        }
    }
}

fn wait<F: Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

fn decode_fixture() -> FinishedAudio {
    let reader = hound::WavReader::new(Cursor::new(FIXTURE)).expect("fixture wav");
    let spec = reader.spec();
    assert_eq!(spec.channels, 1, "fixture is mono");
    assert_eq!(spec.bits_per_sample, 16);
    let interleaved = reader
        .into_samples::<i16>()
        .map(|sample| sample.expect("sample") as f32 / i16::MAX as f32)
        .collect::<Vec<_>>();
    let elapsed_ms = interleaved.len() as u64 * 1000 / u64::from(spec.sample_rate.max(1));
    FinishedAudio {
        interleaved,
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        elapsed_ms,
    }
}

fn words_of(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn assert_phrase(text: &str) {
    let words = words_of(text);
    for expected in ["hello", "cadence", "dictation", "test"] {
        assert!(
            words.iter().any(|word| word == expected),
            "missing {expected} in {text:?}"
        );
    }
}

#[test]
fn fixture_trims_silence_and_encodes_16k_mono() {
    let finished = decode_fixture();
    let raw_ms = finished.elapsed_ms;
    let wav = audio::prepare_speech(&finished).expect("encode");
    let mut reader = hound::WavReader::new(Cursor::new(&wav)).unwrap();
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, 16_000);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.bits_per_sample, 16);
    let samples = reader.samples::<i16>().count();
    let out_ms = samples as u64 * 1000 / 16_000;
    assert!(
        out_ms + 80 < raw_ms,
        "leading silence should have been trimmed ({out_ms} ms from {raw_ms} ms)"
    );
    assert!(samples > 16_000, "the spoken phrase should survive the trim");
}

#[test]
fn live_whisper_transcribes_the_fixture() {
    let Some(key) = require_key("live_whisper_transcribes_the_fixture") else {
        return;
    };
    let finished = decode_fixture();
    let wav = audio::prepare_speech(&finished).expect("encode");
    let text = wait(transcribe::transcribe(
        transcribe::http_client(),
        &key,
        wav,
        transcribe::WHISPER_TURBO,
        "en",
        None,
    ))
    .expect("transcribe");
    assert_phrase(&text);
}

#[test]
fn live_llama_cleanup_applies_a_spoken_correction() {
    let Some(key) = require_key("live_llama_cleanup_applies_a_spoken_correction") else {
        return;
    };
    let outcome = wait(cleanup::clean_transcript(
        transcribe::http_client(),
        &key,
        "Let's meet Tuesday um scratch that Wednesday",
        true,
    ));
    assert!(
        outcome.used_model,
        "cleanup did not use Llama: {}",
        outcome.note.unwrap_or_default()
    );
    let lower = outcome.text.to_lowercase();
    assert!(lower.contains("wednesday"), "{}", outcome.text);
    assert!(!lower.contains("scratch that"), "{}", outcome.text);
    assert!(!words_of(&outcome.text).iter().any(|word| word == "um"), "{}", outcome.text);
}

#[test]
fn live_pipeline_trims_encodes_transcribes_and_strips_fillers() {
    let Some(key) = require_key("live_pipeline_trims_encodes_transcribes_and_strips_fillers") else {
        return;
    };
    let finished = decode_fixture();
    let wav = audio::prepare_speech(&finished).expect("encode");
    let transcript = wait(transcribe::transcribe(
        transcribe::http_client(),
        &key,
        wav,
        transcribe::WHISPER_TURBO,
        "en",
        None,
    ))
    .expect("transcribe");
    let cleaned = cleanup::strip_fillers(&transcript);
    assert_phrase(&cleaned);
    assert!(
        !words_of(&cleaned).iter().any(|word| word == "um" || word == "uh"),
        "fillers remained in {cleaned:?}"
    );
}
