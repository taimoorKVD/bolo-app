//! Built-in Groq key compiled into the binary.
//!
//! `build.rs` reads `BOLO_BUILTIN_GROQ_KEY` and writes XOR bytes to `OUT_DIR`.
//! This module never sees a string literal of the key. An empty slice means
//! this build has no built-in key, and the user must save their own.

use zeroize::Zeroize;

include!(concat!(env!("OUT_DIR"), "/builtin_key.rs"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyChoice {
    Own,
    Builtin,
    Required,
}

/// Own key wins. Otherwise the compiled-in key. Otherwise the user must add one.
pub fn select_key_source(own_key_present: bool, builtin_present: bool) -> KeyChoice {
    if own_key_present {
        KeyChoice::Own
    } else if builtin_present {
        KeyChoice::Builtin
    } else {
        KeyChoice::Required
    }
}

/// Settings opens for a missing key only when this build has nothing to use.
pub fn onboarding_needs_key(own_configured: bool, builtin_present: bool) -> bool {
    matches!(
        select_key_source(own_configured, builtin_present),
        KeyChoice::Required
    )
}

pub fn obfuscate(plain: &[u8], mask: &[u8]) -> Vec<u8> {
    if mask.is_empty() {
        return plain.to_vec();
    }
    plain
        .iter()
        .enumerate()
        .map(|(index, byte)| byte ^ mask[index % mask.len()])
        .collect()
}

pub fn reveal(obfuscated: &[u8], mask: &[u8]) -> Option<String> {
    if obfuscated.is_empty() || mask.is_empty() {
        return None;
    }
    let plain = obfuscate(obfuscated, mask);
    let mut text = match String::from_utf8(plain) {
        Ok(text) => text,
        Err(err) => {
            let mut bytes = err.into_bytes();
            bytes.zeroize();
            return None;
        }
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        text.zeroize();
        return None;
    }
    if trimmed.len() == text.len() {
        return Some(text);
    }
    let owned = trimmed.to_string();
    text.zeroize();
    Some(owned)
}

pub fn available() -> bool {
    match decode() {
        Some(mut key) => {
            let ok = !key.trim().is_empty();
            key.zeroize();
            ok
        }
        None => false,
    }
}

pub fn decode() -> Option<String> {
    reveal(BUILTIN_OBFUSCATED, BUILTIN_MASK)
}

pub fn redact_secret(message: &str, secret: &str) -> String {
    if secret.len() < 8 || !message.contains(secret) {
        return message.to_string();
    }
    message.replace(secret, "[redacted]")
}

pub fn redact_builtin(message: &str) -> String {
    let Some(secret) = decode() else {
        return message.to_string();
    };
    let out = redact_secret(message, &secret);
    let mut secret = secret;
    secret.zeroize();
    out
}

/// Strip the compiled-in key and any `gsk_` token so a log line cannot keep one.
pub fn redact_secrets(message: &str) -> String {
    redact_gsk_tokens(&redact_builtin(message))
}

fn redact_gsk_tokens(message: &str) -> String {
    let bytes = message.as_bytes();
    let mut result = String::with_capacity(message.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"gsk_") {
            let mut end = index + 4;
            while end < bytes.len() && is_key_byte(bytes[end]) {
                end += 1;
            }
            if end > index + 4 {
                result.push_str("[redacted]");
                index = end;
                continue;
            }
        }
        let next = next_char_end(message, index);
        result.push_str(&message[index..next]);
        index = next;
    }
    result
}

fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn next_char_end(message: &str, index: usize) -> usize {
    let mut end = index + 1;
    while end < message.len() && !message.is_char_boundary(end) {
        end += 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "gsk_test_fixture_key_value_xxxxxxxx";

    #[test]
    fn own_key_wins_over_the_builtin() {
        assert_eq!(select_key_source(true, true), KeyChoice::Own);
        assert_eq!(select_key_source(true, false), KeyChoice::Own);
        assert_eq!(select_key_source(false, true), KeyChoice::Builtin);
        assert_eq!(select_key_source(false, false), KeyChoice::Required);
    }

    #[test]
    fn onboarding_asks_only_when_nothing_is_available() {
        assert!(!onboarding_needs_key(true, true));
        assert!(!onboarding_needs_key(true, false));
        assert!(!onboarding_needs_key(false, true));
        assert!(onboarding_needs_key(false, false));
    }

    #[test]
    fn xor_roundtrip_hides_the_plaintext() {
        let mask = b"bolo-mask-bytes!";
        let obfuscated = obfuscate(FIXTURE.as_bytes(), mask);
        assert_ne!(obfuscated, FIXTURE.as_bytes());
        assert!(obfuscated.windows(4).all(|window| window != b"gsk_"));
        assert!(!obfuscated.windows(FIXTURE.len()).any(|window| window == FIXTURE.as_bytes()));
        assert_eq!(reveal(&obfuscated, mask).as_deref(), Some(FIXTURE));
        assert_eq!(reveal(&[], mask), None);
        assert_eq!(reveal(b"abc", &[]), None);
    }

    #[test]
    fn compiled_blob_is_not_a_plaintext_key() {
        assert!(BUILTIN_OBFUSCATED.windows(4).all(|window| window != b"gsk_"));
        if let Some(mut secret) = decode() {
            let bytes = secret.as_bytes();
            assert!(BUILTIN_OBFUSCATED.windows(bytes.len()).all(|window| window != bytes));
            secret.zeroize();
        }
    }

    #[test]
    fn redact_secret_strips_a_fixture() {
        let message = format!("token {FIXTURE} tail");
        let redacted = redact_secret(&message, FIXTURE);
        assert!(!redacted.contains(FIXTURE));
        assert!(redacted.contains("[redacted]"));
        assert_eq!(redact_secret("short", "abcd"), "short");
    }

    #[test]
    fn redact_secrets_strips_gsk_tokens() {
        let message = format!("before {FIXTURE} after");
        let redacted = redact_secrets(&message);
        assert!(!redacted.contains(FIXTURE));
        assert!(!redacted.contains("gsk_"));
        assert!(redacted.contains("[redacted]"));
    }
}
