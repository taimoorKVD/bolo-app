//! On-device fallback through a whisper.cpp binary.
//!
//! Groq stays the default. A 429, a 5xx, or a network failure transcribes with
//! the selected ggml model instead. The next take tries Groq again. The model
//! and the Windows whisper.cpp build download on first use into the config
//! directory. Tests never download.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

const RELEASE: &str = "v1.8.2";

pub fn normalize_model(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "groq" => "off",
        "small" | "small.en" | "ggml-small.en" => "small.en",
        "base" | "base.en" | "ggml-base.en" => "base.en",
        _ => "base.en",
    }
}

pub fn model_label(model: &str) -> &'static str {
    match normalize_model(model) {
        "off" => "Off",
        "small.en" => "whisper.cpp small.en",
        _ => "whisper.cpp base.en",
    }
}

fn model_url(model: &str) -> Option<&'static str> {
    match normalize_model(model) {
        "base.en" => Some("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"),
        "small.en" => Some("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin"),
        _ => None,
    }
}

fn whisper_dir(config: &Path) -> PathBuf {
    config.join("whisper")
}

fn model_path(config: &Path, model: &str) -> PathBuf {
    whisper_dir(config).join(format!("ggml-{}.bin", normalize_model(model)))
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
        if let Some(url) = model_url(model) {
            download_file(url, &model_file).await?;
        }
    }
    if find_binary(&dir).is_none() && std::env::var("BOLO_WHISPER_BIN").is_err() {
        download_binary(&dir).await?;
    }
    Ok(())
}

pub async fn transcribe_encoded(config: &Path, model: &str, bytes: &[u8]) -> Result<String, String> {
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
    let result = run_cli(&binary, &model_file, &temp);
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

fn run_cli(binary: &Path, model: &Path, audio: &Path) -> Result<String, String> {
    let output = std::process::Command::new(binary)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(audio)
        .arg("-nt")
        .arg("--no-prints")
        .arg("-np")
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
    extract_zip(&bytes, dir)
}

async fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let bytes = fetch(url).await?;
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
        assert_eq!(normalize_model("off"), "off");
        assert_eq!(normalize_model(""), "base.en");
        assert_eq!(model_label("base.en"), "whisper.cpp base.en");
    }
}
