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
    let message = crate::builtin::redact_secrets(message);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use zeroize::Zeroize;

    #[test]
    fn log_line_never_writes_a_groq_key() {
        let dir = std::env::temp_dir().join(format!("bolo-log-redact-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let fixture = "gsk_test_fixture_key_value_xxxxxxxx";
        log_line(&dir, "info", &format!("before {fixture} after"));
        let log = fs::read_to_string(dir.join("bolo.log")).unwrap();
        assert!(!log.contains(fixture), "{log}");
        assert!(!log.contains("gsk_"), "{log}");
        assert!(log.contains("[redacted]"), "{log}");
        if let Some(mut secret) = crate::builtin::decode() {
            assert!(!log.contains(&secret), "{log}");
            secret.zeroize();
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
