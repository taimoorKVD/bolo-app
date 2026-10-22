use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn init(dir: &Path) {
    let path = dir.join("bolo.log");
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > 1_000_000 {
            let _ = fs::rename(&path, dir.join("bolo.log.1"));
        }
    }
}

pub fn log_line(dir: &Path, level: &str, message: &str) {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let line = format!("{millis} {level} {message}");
    eprintln!("{line}");
    let _ = fs::create_dir_all(dir);
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("bolo.log"))
    {
        let _ = writeln!(file, "{line}");
    }
}
