//! End-to-end harness: real speech WAVs -> capture-equivalent buffer -> the
//! same encode / Groq / route / cleanup / insert path a hotkey take uses.
//! Ignored by default. Run with:
//!   BOLO_HARNESS_DIR=<dir with refs.json + wavs> cargo test --release harness -- --ignored --nocapture
//! Optional: BOLO_HARNESS_RUNS=n (passes), BOLO_HARNESS_PASTE=1 (insert into the focused window).
//! The API key is read from the app's own store and never printed.

use crate::audio::{self, FinishedAudio, SpeechError};
use crate::cleanup::{self, Tone};
use crate::credentials::SecretKey;
use crate::pipeline;
use crate::transcribe;
use std::time::Instant;

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|w| w.trim_matches('\'').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

fn edit<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(x != y)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

pub fn wer(reference: &str, hyp: &str) -> f64 {
    let r = words(reference);
    let h = words(hyp);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    edit(&r, &h) as f64 / r.len() as f64
}

pub fn cer(reference: &str, hyp: &str) -> f64 {
    let r: Vec<char> = words(reference).concat().chars().collect();
    let h: Vec<char> = words(hyp).concat().chars().collect();
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    edit(&r, &h) as f64 / r.len() as f64
}

fn load(path: &std::path::Path) -> (Vec<f32>, u32) {
    let reader = hound::WavReader::open(path).expect("wav");
    let spec = reader.spec();
    let s: Vec<f32> = reader.into_samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
    (s, spec.sample_rate)
}

/// Mimic a real take: lead-in before speaking, a tail after, room noise, and
/// a mic gain (1.0 normal, ~0.03 for a mic whose Windows level is very low).
fn take(speech: &[f32], rate: u32, gain: f32, seed: u32) -> FinishedAudio {
    let mut x = seed.wrapping_mul(2654435761).max(1);
    let mut noise = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        (x as f32 / u32::MAX as f32 - 0.5) * 0.002 * gain.max(0.05)
    };
    let lead = (rate as usize * 350) / 1000;
    let tail = (rate as usize * 300) / 1000;
    let mut out = Vec::with_capacity(lead + speech.len() + tail);
    for _ in 0..lead {
        out.push(noise());
    }
    for s in speech {
        out.push(s * gain + noise());
    }
    for _ in 0..tail {
        out.push(noise());
    }
    let noise_rms = 0.0006 * gain.max(0.05);
    FinishedAudio {
        elapsed_ms: out.len() as u64 * 1000 / rate as u64,
        interleaved: out,
        channels: 1,
        sample_rate: rate,
        noise_rms,
    }
}

#[test]
#[ignore]
fn harness() {
    let Ok(dir) = std::env::var("BOLO_HARNESS_DIR") else {
        eprintln!("SKIP harness: BOLO_HARNESS_DIR not set");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let runs: usize = std::env::var("BOLO_HARNESS_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let pace: u64 = std::env::var("BOLO_HARNESS_PACE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(3500);
    let paste = std::env::var("BOLO_HARNESS_PASTE").ok().as_deref() == Some("1");
    let refs: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("refs.json")).unwrap()).unwrap();
    let key = SecretKey::load().expect("api key");
    let builtin = key.is_builtin();
    let client = transcribe::http_client();
    let dictionary: Vec<String> = ["GitHub", "Figma", "Bolo"].iter().map(|s| s.to_string()).collect();
    let prompt = transcribe::whisper_prompt_with(&dictionary, &[]);
    let model = std::env::var("BOLO_HARNESS_MODEL").unwrap_or_else(|_| transcribe::WHISPER_TURBO.into());
    let mut lines = Vec::new();
    let (mut total, mut ok) = (0usize, 0usize);
    let mut lat = Vec::new();
    let mut wer_sum = (0.0f64, 0usize, 0.0f64, 0usize);
    for pass in 0..runs {
        for (n, item) in refs.iter().enumerate() {
            let id = item["id"].as_str().unwrap();
            let lang = item["lang"].as_str().unwrap();
            let text = item["text"].as_str().unwrap();
            let (speech, rate) = load(&dir.join(format!("{id}.wav")));
            for (cond, gain) in [("normal", 1.0f32), ("quiet", 0.03f32), ("vquiet", 0.004f32)] {
                std::thread::sleep(std::time::Duration::from_millis(pace));
                total += 1;
                let finished = take(&speech, rate, gain, (pass * 100 + n) as u32 + 7);
                let t0 = Instant::now();
                let output = if lang == "en" { "english" } else { "same" };
                let result: Result<String, String> = (|| {
                    let encoded = audio::prepare_speech(&finished).map_err(|e| match e {
                        SpeechError::Silent | SpeechError::Empty => "silent (dropped as no speech)".to_string(),
                        other => other.to_string(),
                    })?;
                    let rec = tauri::async_runtime::block_on(pipeline::speech_with_groq(
                        client, key.expose(), encoded.bytes, &model, "auto", output,
                        prompt.as_deref(), "openai/gpt-oss-20b", builtin,
                    ))
                    .map_err(|e| e.to_string())?;
                    let cleaned = tauri::async_runtime::block_on(cleanup::clean_transcript(
                        client, key.expose(), &rec.text, true, Tone::Plain, false, "openai/gpt-oss-20b", builtin,
                    ));
                    Ok(cleaned.text)
                })();
                let ms = t0.elapsed().as_millis() as u64;
                match result {
                    Ok(out) => {
                        let w = wer(text, &out);
                        let c = cer(text, &out);
                        if lang == "en" { wer_sum.0 += w; wer_sum.1 += 1; } else { wer_sum.2 += c; wer_sum.3 += 1; }
                        let good = !out.trim().is_empty() && if lang == "en" { w <= 0.34 } else { c <= 0.4 };
                        let mut pasted = String::from("-");
                        if paste && !out.trim().is_empty() {
                            pasted = match crate::insert::insert_text(&format!("[{id}/{cond}] {} ", out.trim())) {
                                Ok(ins) => format!("{:?}/{}", ins.method, ins.verified),
                                Err(e) => format!("ERR {e}"),
                            };
                            std::thread::sleep(std::time::Duration::from_millis(250));
                        }
                        if good { ok += 1; lat.push(ms); }
                        lines.push(format!("{} pass={pass} {id} {cond} ms={ms} wer={w:.2} cer={c:.2} paste={pasted} out={:?}", if good {"OK  "} else {"FAIL"}, out));
                    }
                    Err(e) => lines.push(format!("FAIL pass={pass} {id} {cond} ms={ms} err={e}")),
                }
            }
        }
        // Silence must produce nothing (no "Thank you." hallucination).
        total += 1;
        let silent = take(&[], 48_000, 1.0, 99 + pass as u32);
        let mut s = silent;
        s.interleaved.extend(std::iter::repeat(0.0003).take(48_000 * 2));
        match audio::prepare_speech(&s) {
            Err(_) => { ok += 1; lines.push(format!("OK   pass={pass} silence dropped locally")); }
            Ok(enc) => {
                let rec = tauri::async_runtime::block_on(pipeline::speech_with_groq(
                    client, key.expose(), enc.bytes, &model, "auto", "english", prompt.as_deref(), "openai/gpt-oss-20b", builtin));
                let t = rec.map(|r| r.text).unwrap_or_default();
                if t.trim().is_empty() { ok += 1; }
                lines.push(format!("{} pass={pass} silence out={t:?}", if t.trim().is_empty() {"OK  "} else {"FAIL"}));
            }
        }
    }
    drop(key);
    lat.sort();
    let p = |q: f64| lat.get(((lat.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0);
    for l in &lines { println!("{l}"); }
    println!(
        "SUMMARY model={model} runs={total} success={ok} rate={:.1}% en_wer={:.3} ur_cer={:.3} latency_ms p50={} p90={} max={}",
        ok as f64 * 100.0 / total as f64,
        wer_sum.0 / wer_sum.1.max(1) as f64,
        wer_sum.2 / wer_sum.3.max(1) as f64,
        p(0.5), p(0.9), lat.last().copied().unwrap_or(0)
    );
}
