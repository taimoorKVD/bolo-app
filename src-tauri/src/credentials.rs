//! Groq API key storage.
//!
//! On Windows, the `keyring` crate writes to Credential Manager. The key is
//! never written into settings.json and is never logged.

use crate::cleanup;
use anyhow::Context;
use serde::Serialize;
use zeroize::Zeroize;

const SERVICE: &str = "com.cadence.dictation";
const ACCOUNT: &str = "groq-api-key";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    pub configured: bool,
    pub hint: Option<String>,
    pub error: Option<String>,
}

pub fn validate_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Enter a Groq API key.".into());
    }
    if key.len() < 20 || key.len() > 256 || key.chars().any(|c| c.is_whitespace()) {
        return Err("That doesn't look like a Groq API key.".into());
    }
    Ok(())
}

pub fn set_api_key(key: &str) -> anyhow::Result<()> {
    let key = key.trim();
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).context("couldn't open the credential store")?;
    entry
        .set_password(key)
        .context("couldn't save the API key to the credential store")?;
    Ok(())
}

pub fn clear_api_key() -> anyhow::Result<()> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).context("couldn't open the credential store")?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err).context("couldn't remove the API key"),
    }
}

pub fn load_api_key() -> anyhow::Result<Option<String>> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).context("couldn't open the credential store")?;
    match entry.get_password() {
        Ok(key) if key.trim().is_empty() => Ok(None),
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err).context("couldn't read the API key"),
    }
}

pub fn status() -> ApiKeyStatus {
    match load_api_key() {
        Ok(Some(mut key)) => {
            let hint = hint_of(&key);
            key.zeroize();
            ApiKeyStatus {
                configured: true,
                hint: Some(hint),
                error: None,
            }
        }
        Ok(None) => ApiKeyStatus {
            configured: false,
            hint: None,
            error: None,
        },
        Err(err) => ApiKeyStatus {
            configured: false,
            hint: None,
            error: Some(err.to_string()),
        },
    }
}

pub fn status_from_key(key: &str) -> ApiKeyStatus {
    ApiKeyStatus {
        configured: true,
        hint: Some(hint_of(key)),
        error: None,
    }
}

fn hint_of(key: &str) -> String {
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("····{tail}")
}

/// Read the key for one request and wipe it when the guard drops.
pub struct SecretKey(String);

impl SecretKey {
    pub fn load() -> anyhow::Result<Self> {
        match load_api_key()? {
            Some(key) => Ok(Self(key)),
            None => anyhow::bail!("Add a Groq API key in Cadence settings."),
        }
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        cleanup::wipe(&mut self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blank_and_short_keys() {
        assert!(validate_key("").is_err());
        assert!(validate_key("gsk_short").is_err());
        assert!(validate_key("gsk_has a space in the token!!").is_err());
        assert!(validate_key("gsk_12345678901234567890").is_ok());
    }

    #[test]
    fn hint_shows_only_the_last_four() {
        assert_eq!(hint_of("gsk_example_secret_wxyz"), "····wxyz");
    }
}
