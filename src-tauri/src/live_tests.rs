//! Live Groq checks. They run only when GROQ_API_KEY is set; otherwise they
//! print SKIP and pass so CI without the secret stays green.

use crate::audio::{self, FinishedAudio};
use crate::cleanup::{self, Tone};
use crate::transcribe;
use std::future::Future;
use std::io::Cursor;
use std::time::Instant;

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
        noise_rms: 0.0,
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
    let samples_f = audio::speech_samples(&finished).expect("samples");
    let encoded = audio::prepare_speech(&finished).expect("encode");
    assert_eq!(&encoded.bytes[0..4], b"fLaC");
    assert_eq!(encoded.format, "flac");
    let samples = samples_f.len();
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
    let wav = audio::prepare_speech(&finished).expect("encode").bytes;
    let text = wait(transcribe::transcribe(
        transcribe::http_client(),
        &key,
        wav,
        transcribe::WHISPER_TURBO,
        "en",
        None,
    ))
    .expect("transcribe");
    assert_phrase(&text.text);
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
        Tone::Plain,
    ));
    assert!(
        outcome.used_model,
        "cleanup did not use Llama: {}",
        outcome.why
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
    let wav = audio::prepare_speech(&finished).expect("encode").bytes;
    let transcript = wait(transcribe::transcribe(
        transcribe::http_client(),
        &key,
        wav,
        transcribe::WHISPER_TURBO,
        "en",
        None,
    ))
    .expect("transcribe");
    let cleaned = cleanup::strip_fillers(&transcript.text);
    assert_phrase(&cleaned);
    assert!(
        !words_of(&cleaned).iter().any(|word| word == "um" || word == "uh"),
        "fillers remained in {cleaned:?}"
    );
}

#[test]
fn encode_short_clip_stays_under_a_second() {
    let finished = decode_fixture();
    let fixture_started = Instant::now();
    let fixture_wav = audio::prepare_speech(&finished).expect("fixture");
    let fixture_ms = fixture_started.elapsed().as_millis();

    let rate = 48_000u32;
    let frames = rate * 3;
    let tone = (0..frames)
        .map(|index| {
            let t = index as f32 / rate as f32;
            (t * 220.0 * std::f32::consts::TAU).sin() * 0.2
        })
        .collect::<Vec<_>>();
    let clip = FinishedAudio {
        interleaved: tone,
        channels: 1,
        sample_rate: rate,
        elapsed_ms: 3_000,
        noise_rms: 0.0,
    };
    let started = Instant::now();
    let wav = audio::prepare_speech(&clip).expect("tone");
    let ms = started.elapsed().as_millis();
    eprintln!(
        "LATENCY encode_fixture_ms={fixture_ms} fixture_bytes={} encode_3s_ms={ms} bytes={} fmt={}",
        fixture_wav.bytes.len(),
        wav.bytes.len(),
        wav.format
    );
    assert!(fixture_ms < 1_500, "fixture encode took {fixture_ms} ms");
    assert!(ms < 1_500, "3s encode took {ms} ms");
}

#[test]
fn live_latency_stages_stay_under_the_budget() {
    let Some(key) = require_key("live_latency_stages_stay_under_the_budget") else {
        return;
    };
    let finished = decode_fixture();
    let encode_started = Instant::now();
    let wav = audio::prepare_speech(&finished).expect("encode");
    let encode_ms = encode_started.elapsed().as_millis();
    let bytes = wav.bytes.len();
    let groq_started = Instant::now();
    let text = wait(transcribe::transcribe(
        transcribe::http_client(),
        &key,
        wav.bytes,
        transcribe::WHISPER_TURBO,
        "en",
        transcribe::whisper_prompt(&["Bolo.com".into()]),
    ))
    .expect("transcribe");
    let groq_ms = groq_started.elapsed().as_millis();
    let cleanup_started = Instant::now();
    let outcome = wait(cleanup::clean_transcript(
        transcribe::http_client(),
        &key,
        "Let's meet Tuesday scratch that Wednesday period",
        true,
        Tone::Plain,
    ));
    let cleanup_ms = cleanup_started.elapsed().as_millis();
    eprintln!(
        "LATENCY encode_ms={encode_ms} bytes={bytes} groq_ms={groq_ms} cleanup_ms={cleanup_ms} cleaned={} model={}",
        outcome.used_model,
        cleanup::CLEANUP_MODEL
    );
    assert!(groq_ms < 8_000, "whisper took {groq_ms} ms");
    assert!(cleanup_ms < 4_000, "cleanup took {cleanup_ms} ms");
    assert_phrase(&text.text);
    let lower = outcome.text.to_lowercase();
    assert!(lower.contains("wednesday"), "{}", outcome.text);
    assert!(lower.contains('.') || lower.ends_with("wednesday"), "{}", outcome.text);
}
