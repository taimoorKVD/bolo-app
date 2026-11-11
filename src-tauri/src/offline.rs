//! On-device fallback through a whisper.cpp binary.
//!
//! Groq stays the default. A 429, a 5xx, or a network failure transcribes with
//! the selected ggml model instead. The next take tries Groq again. The model
//! and the Windows whisper.cpp build download on first use into the config
//! directory. Tests never download.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

const RELEASE: &str = "v1.8.2";
/// SHA-256 of ggml-org/whisper.cpp release v1.8.2 `whisper-bin-x64.zip`.
const WHISPER_BIN_SHA256: &str = "b1514ebc099765e39fa37eb780b92a140a94c86bb0b3b3d98226b38825979732";

/// Shown instead of running an English-only model on Urdu, Hindi, or mixed speech.
pub const URDU_OFFLINE_NOTICE: &str =
    "Urdu needs the online speech service, or download the multilingual offline model.";

struct ModelSpec {
    id: &'static str,
    file: &'static str,
    url: &'static str,
    sha256: &'static str,
    bytes: u64,
    multilingual: bool,
}

/// Hugging Face `lfs.oid` sha256 and byte size for ggerganov/whisper.cpp.
/// `base` and `small` stay aliases of the English models so older saves do not switch.
const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "base.en",
        file: "ggml-base.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        bytes: 147_964_211,
        multilingual: false,
    },
    ModelSpec {
        id: "small.en",
        file: "ggml-small.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        bytes: 487_614_201,
        multilingual: false,
    },
    ModelSpec {
        id: "ggml-base",
        file: "ggml-base.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        bytes: 147_951_465,
        multilingual: true,
    },
    ModelSpec {
        id: "ggml-small",
        file: "ggml-small.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        bytes: 487_601_967,
        multilingual: true,
    },
];

pub fn normalize_model(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "groq" => "off",
        "small" | "small.en" | "ggml-small.en" | "ggml-small.en.bin" => "small.en",
        "base" | "base.en" | "ggml-base.en" | "ggml-base.en.bin" => "base.en",
        "ggml-base" | "base.multi" | "multi-base" | "ggml-base.bin" => "ggml-base",
        "ggml-small" | "small.multi" | "multi-small" | "ggml-small.bin" => "ggml-small",
        _ => "base.en",
    }
}

/// True only for values Bolo actually offers, so a bad save is refused
/// instead of being silently turned into base.en.
pub fn is_known_model(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    matches!(
        value.as_str(),
        "off" | "none" | "groq"
            | "small" | "small.en" | "ggml-small.en" | "ggml-small.en.bin"
            | "base" | "base.en" | "ggml-base.en" | "ggml-base.en.bin"
            | "ggml-base" | "base.multi" | "multi-base" | "ggml-base.bin"
            | "ggml-small" | "small.multi" | "multi-small" | "ggml-small.bin"
    )
}

pub fn model_label(model: &str) -> &'static str {
    match normalize_model(model) {
        "off" => "Off",
        "small.en" => "whisper.cpp small.en",
        "ggml-base" => "whisper.cpp ggml-base",
        "ggml-small" => "whisper.cpp ggml-small",
        _ => "whisper.cpp base.en",
    }
}

pub fn is_english_only(model: &str) -> bool {
    matches!(normalize_model(model), "base.en" | "small.en")
}

pub fn is_multilingual_notice(message: &str) -> bool {
    message == URDU_OFFLINE_NOTICE
}

/// Auto is the mixed Urdu/English setting. English-only users pin `en`.
pub fn language_needs_multilingual(language: &str) -> bool {
    matches!(
        language.trim().to_ascii_lowercase().as_str(),
        "ur" | "hi" | "auto" | "mixed"
    )
}

fn model_spec(model: &str) -> Option<&'static ModelSpec> {
    let id = normalize_model(model);
    MODELS.iter().find(|spec| spec.id == id)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn verify_download(model: &str, bytes: &[u8]) -> Result<(), String> {
    let spec = model_spec(model).ok_or_else(|| "Unknown on-device model.".to_string())?;
    let got = sha256_hex(bytes);
    if got != spec.sha256 {
        return Err(format!(
            "The {} download didn't match its hash, so it was not saved.",
            spec.file
        ));
    }
    Ok(())
}

/// A multilingual file already on disk is preferred for Urdu, Hindi, or mixed
/// speech. Otherwise the installed model runs (English `base.en` is enough
/// for a take that is probably English). The caller downloads a multilingual
/// model in the background when [`multilingual_download_wanted`] is true.
pub fn resolve_fallback(config: &Path, selected: &str, language: &str) -> Result<&'static str, String> {
    let selected = normalize_model(selected);
    if selected == "off" {
        return Err("On-device transcription is turned off in Settings.".into());
    }
    if !language_needs_multilingual(language) || !is_english_only(selected) {
        return Ok(selected);
    }
    if model_present(config, "ggml-small") {
        return Ok("ggml-small");
    }
    if model_present(config, "ggml-base") {
        return Ok("ggml-base");
    }
    Ok(selected)
}

/// True when this take may need Urdu or Hindi and no multilingual file is on disk.
pub fn multilingual_download_wanted(config: &Path, selected: &str, language: &str) -> bool {
    let selected = normalize_model(selected);
    language_needs_multilingual(language)
        && selected != "off"
        && is_english_only(selected)
        && !model_present(config, "ggml-base")
        && !model_present(config, "ggml-small")
}

/// whisper.cpp language flag. English-only models are not pinned to Urdu or Hindi.
/// Auto and mixed stay unpinned so one sentence can contain both languages.
pub fn cli_language_args(model: &str, language: &str) -> Vec<&'static str> {
    let Some(spec) = model_spec(model) else {
        return Vec::new();
    };
    if !spec.multilingual {
        return Vec::new();
    }
    match language.trim().to_ascii_lowercase().as_str() {
        "ur" => vec!["-l", "ur"],
        "hi" => vec!["-l", "hi"],
        "en" => vec!["-l", "en"],
        _ => Vec::new(),
    }
}

fn rounded_mb(bytes: u64) -> u64 {
    (bytes + 500_000) / 1_000_000
}

pub fn multilingual_choice_label(model: &str) -> Option<String> {
    let spec = model_spec(model)?;
    if !spec.multilingual {
        return None;
    }
    let name = spec.file.trim_end_matches(".bin");
    Some(format!("{name} — multilingual, {} MB", rounded_mb(spec.bytes)))
}

fn whisper_dir(config: &Path) -> PathBuf {
    config.join("whisper")
}

fn model_path(config: &Path, model: &str) -> PathBuf {
    let file = model_spec(model).map(|spec| spec.file).unwrap_or("ggml-base.en.bin");
    whisper_dir(config).join(file)
}

fn model_present(config: &Path, model: &str) -> bool {
    model_path(config, model).is_file()
}

/// Start a background download when the selected model or binary is missing.
/// Unit tests skip this so `cargo test` does not fetch hundreds of megabytes.
pub fn prefetch(config: std::path::PathBuf, model: String) {
    if cfg!(test) || normalize_model(&model) == "off" {
        return;
    }
    std::thread::Builder::new()
        .name("bolo-whisper".into())
        .spawn(move || {
            let _ = tauri::async_runtime::block_on(ensure_assets(&config, &model));
        })
        .ok();
}

pub async fn ensure_assets(config: &Path, model: &str) -> Result<(), String> {
    let model = normalize_model(model);
    if model == "off" {
        return Ok(());
    }
    let dir = whisper_dir(config);
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let model_file = model_path(config, model);
    if !model_file.exists() {
        download_model(model, &model_file).await?;
    }
    if find_binary(&dir).is_none() && std::env::var("BOLO_WHISPER_BIN").is_err() {
        download_binary(&dir).await?;
    }
    Ok(())
}

pub async fn transcribe_encoded(config: &Path, model: &str, language: &str, bytes: &[u8]) -> Result<String, String> {
    let model = normalize_model(model);
    if model == "off" {
        return Err("On-device transcription is turned off in Settings.".into());
    }
    if bytes.is_empty() {
        return Err("There was no audio for the on-device model.".into());
    }
    let dir = whisper_dir(config);
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let model_file = model_path(config, model);
    if !model_file.exists() {
        prefetch(config.to_path_buf(), model.to_string());
        return Err(format!(
            "The {} model isn't on this PC yet. Bolo is downloading it for the next take.",
            model_label(model)
        ));
    }
    let binary = match binary_path(&dir) {
        Some(path) => path,
        None => {
            prefetch(config.to_path_buf(), model.to_string());
            return Err(
                "whisper.cpp isn't on this PC yet. Bolo is downloading it for the next take.".into(),
            );
        }
    };
    let temp = dir.join(format!(
        "chunk-{}-{}.audio",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    fs::write(&temp, bytes).map_err(|err| format!("couldn't store the chunk: {err}"))?;
    let result = run_cli(&binary, &model_file, &temp, &cli_language_args(model, language));
    let _ = fs::remove_file(&temp);
    result
}

fn binary_path(dir: &Path) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("BOLO_WHISPER_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    find_binary(dir)
}

fn find_binary(dir: &Path) -> Option<PathBuf> {
    let names = if cfg!(windows) {
        ["whisper-cli.exe", "main.exe"]
    } else {
        ["whisper-cli", "main"]
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if names.iter().any(|name| file_name.eq_ignore_ascii_case(name)) {
                return Some(path);
            }
        }
    }
    None
}

/// Windows `CREATE_NO_WINDOW`. Without it, whisper-cli opens a black console on every take.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn run_cli(binary: &Path, model: &Path, audio: &Path, language_args: &[&str]) -> Result<String, String> {
    let mut command = std::process::Command::new(binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.stdin(std::process::Stdio::null());
    command
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(audio)
        .arg("-nt")
        .arg("--no-prints")
        .arg("-np");
    for arg in language_args {
        command.arg(arg);
    }
    let output = command
        .output()
        .map_err(|err| format!("couldn't start whisper.cpp ({err})"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = parse_whisper_output(&stdout);
    if !text.is_empty() {
        return Ok(text);
    }
    let fallback = parse_whisper_output(&stderr);
    if !fallback.is_empty() {
        return Ok(fallback);
    }
    if output.status.success() {
        return Err("whisper.cpp returned no words.".into());
    }
    Err(format!(
        "whisper.cpp failed ({}). {}",
        output.status,
        compact(&stderr, 180)
    ))
}

pub fn parse_whisper_output(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("whisper_")
            || lower.starts_with("ggml_")
            || lower.starts_with("main:")
            || lower.starts_with("system_info")
            || lower.contains("load time")
            || lower.contains("mel time")
            || lower.contains("sample time")
            || lower.contains("encode time")
            || lower.contains("decode time")
        {
            continue;
        }
        let spoken = strip_timestamp(line);
        if !spoken.is_empty() {
            lines.push(spoken.to_string());
        }
    }
    lines.join(" ")
}

fn strip_timestamp(line: &str) -> &str {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return rest[end + 1..].trim();
        }
    }
    line
}

fn compact(body: &str, max: usize) -> String {
    let one = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        one
    } else {
        let cut: String = one.chars().take(max).collect();
        format!("{cut}…")
    }
}

async fn download_binary(dir: &Path) -> Result<(), String> {
    let url = if cfg!(windows) {
        format!("https://github.com/ggml-org/whisper.cpp/releases/download/{RELEASE}/whisper-bin-x64.zip")
    } else {
        // Current whisper.cpp releases publish Windows binaries. A Linux
        // whisper-cli on PATH or BOLO_WHISPER_BIN is used when present.
        return Err("whisper.cpp builds are downloaded on Windows. On this system, set BOLO_WHISPER_BIN.".into());
    };
    let bytes = fetch(&url).await?;
    verify_whisper_zip(&bytes)?;
    extract_zip(&bytes, dir)
}

pub fn verify_whisper_zip(bytes: &[u8]) -> Result<(), String> {
    let got = sha256_hex(bytes);
    if got != WHISPER_BIN_SHA256 {
        return Err("The whisper.cpp download didn't match its hash, so it was not saved.".into());
    }
    Ok(())
}

async fn download_model(model: &str, dest: &Path) -> Result<(), String> {
    let spec = model_spec(model).ok_or_else(|| format!("No download for {model}."))?;
    let bytes = fetch(spec.url).await?;
    verify_download(model, &bytes)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let temp = dest.with_extension("part");
    fs::write(&temp, &bytes).map_err(|err| err.to_string())?;
    fs::rename(&temp, dest).map_err(|err| err.to_string())?;
    Ok(())
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15 * 60))
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("Bolo/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| err.to_string())?;
    let response = client.get(url).send().await.map_err(|err| format!("download failed: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("download returned {}", response.status()));
    }
    response.bytes().await.map(|bytes| bytes.to_vec()).map_err(|err| err.to_string())
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|err| format!("whisper.cpp zip: {err}"))?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|err| err.to_string())?;
        let name = file.name().to_string();
        if name.contains("..") {
            continue;
        }
        let out = dest.join(&name);
        if file.is_dir() {
            fs::create_dir_all(&out).map_err(|err| err.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let mut output = fs::File::create(&out).map_err(|err| err.to_string())?;
        std::io::copy(&mut file, &mut output).map_err(|err| err.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if name.ends_with("whisper-cli") || name.ends_with("main") {
                let _ = fs::set_permissions(&out, fs::Permissions::from_mode(0o755));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whisper_cpp_text_and_ignores_timings() {
        let raw = "\
whisper_init_from_file_with_params_no_state: loading model\n\
[00:00:00.000 --> 00:00:01.200]   ship the notes\n\
whisper_print_timings: load time = 10 ms\n";
        assert_eq!(parse_whisper_output(raw), "ship the notes");
        assert_eq!(normalize_model("small"), "small.en");
        assert_eq!(normalize_model("base"), "base.en");
        assert_eq!(normalize_model("off"), "off");
        assert_eq!(normalize_model(""), "base.en");
        assert_eq!(normalize_model("ggml-base"), "ggml-base");
        assert_eq!(normalize_model("ggml-small.bin"), "ggml-small");
        assert!(is_known_model("ggml-base"));
        assert!(is_known_model("ggml-small"));
        assert!(!is_known_model("ggml-tiny"));
        assert_eq!(model_label("base.en"), "whisper.cpp base.en");
        assert_eq!(model_label("ggml-base"), "whisper.cpp ggml-base");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn multilingual_models_are_pinned_to_the_published_hash_and_size() {
        let base = model_spec("ggml-base").unwrap();
        assert_eq!(base.file, "ggml-base.bin");
        assert!(base.multilingual);
        assert_eq!(base.bytes, 147_951_465);
        assert_eq!(base.sha256, "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe");
        assert_eq!(multilingual_choice_label("ggml-base").as_deref(), Some("ggml-base — multilingual, 148 MB"));
        let small = model_spec("ggml-small").unwrap();
        assert_eq!(small.file, "ggml-small.bin");
        assert_eq!(small.bytes, 487_601_967);
        assert_eq!(small.sha256, "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b");
        assert_eq!(multilingual_choice_label("ggml-small").as_deref(), Some("ggml-small — multilingual, 488 MB"));
        let english = model_spec("base.en").unwrap();
        assert!(!english.multilingual);
        assert_eq!(english.bytes, 147_964_211);
        assert_eq!(english.sha256, "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002");
        let err = verify_download("ggml-base", b"not the model").unwrap_err();
        assert!(err.contains("didn't match its hash"), "{err}");
        assert!(verify_whisper_zip(b"not a zip").is_err());
        assert_eq!(WHISPER_BIN_SHA256.len(), 64);
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
        let src = include_str!("offline.rs");
        assert!(src.contains("Command::new"), "whisper.cpp is started with Command");
        assert!(src.contains("creation_flags(CREATE_NO_WINDOW)"));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        fn walk_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() { walk_rs(&path, out); }
                else if path.extension().is_some_and(|ext| ext == "rs") { out.push(path); }
            }
        }
        let mut files = Vec::new();
        walk_rs(&root, &mut files);
        for entry in files {
            let text = std::fs::read_to_string(&entry).unwrap_or_default();
            if text.contains("std::process::Command::new") || text.contains("process::Command::new") {
                assert!(
                    text.contains("CREATE_NO_WINDOW"),
                    "{} starts a process without CREATE_NO_WINDOW",
                    entry.display()
                );
            }
        }
        assert!(err.contains("ggml-base.bin"), "{err}");
    }

    #[test]
    fn installed_english_model_runs_while_multilingual_downloads() {
        let dir = std::env::temp_dir().join(format!("bolo-offline-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for language in ["ur", "hi", "auto", "mixed"] {
            assert_eq!(resolve_fallback(&dir, "base.en", language).unwrap(), "base.en", "{language}");
            assert!(multilingual_download_wanted(&dir, "base.en", language), "{language}");
            assert_ne!(resolve_fallback(&dir, "base.en", language).unwrap(), URDU_OFFLINE_NOTICE);
        }
        assert!(!multilingual_download_wanted(&dir, "base.en", "en"));
        assert!(!multilingual_download_wanted(&dir, "off", "ur"));
        assert_eq!(resolve_fallback(&dir, "base.en", "en").unwrap(), "base.en");
        assert_eq!(resolve_fallback(&dir, "small.en", "en").unwrap(), "small.en");
        assert_eq!(resolve_fallback(&dir, "ggml-base", "ur").unwrap(), "ggml-base");
        assert!(cli_language_args("base.en", "ur").is_empty());
        assert_eq!(cli_language_args("ggml-base", "ur"), vec!["-l", "ur"]);
        assert_eq!(cli_language_args("ggml-small", "hi"), vec!["-l", "hi"]);
        assert!(cli_language_args("ggml-base", "auto").is_empty());
        assert!(cli_language_args("ggml-base", "mixed").is_empty());
        fs::create_dir_all(dir.join("whisper")).unwrap();
        fs::write(dir.join("whisper").join("ggml-base.bin"), b"present").unwrap();
        assert!(!multilingual_download_wanted(&dir, "base.en", "auto"));
        assert_eq!(resolve_fallback(&dir, "small.en", "ur").unwrap(), "ggml-base");
        fs::write(dir.join("whisper").join("ggml-small.bin"), b"present").unwrap();
        assert_eq!(resolve_fallback(&dir, "base.en", "hi").unwrap(), "ggml-small");
        assert_eq!(resolve_fallback(&dir, "ggml-base", "ur").unwrap(), "ggml-base");
        let _ = fs::remove_dir_all(&dir);
    }
}
