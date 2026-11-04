//! Groq API key storage.
//!
//! On Windows the key is a Generic credential in Credential Manager, persisted
//! for the local machine (not the enterprise roaming store). The target name is
//! fixed so a save and a later read cannot drift. A keyring 3 credential written
//! by older builds is still read and copied onto the new target.
//!
//! The key is never written into settings.json and is never logged. A save is
//! successful only after the same value is read back.

use crate::cleanup;
use anyhow::{anyhow, Context};
use serde::Serialize;
use zeroize::Zeroize;

#[cfg(not(windows))]
const SERVICE: &str = "com.bolo.dictation";
#[cfg(not(windows))]
const SERVICE_CADENCE: &str = "com.cadence.dictation";
const ACCOUNT: &str = "groq-api-key";
/// Current Credential Manager target.
pub(crate) const TARGET: &str = "com.bolo.dictation/groq-api-key";
/// The target used while the product was named Cadence. Still read, and copied
/// onto `TARGET`, so a saved Groq key survives the rename. It is deleted only
/// when the user removes the key.
pub(crate) const CADENCE_TARGET: &str = "com.cadence.dictation/groq-api-key";
/// keyring 3 on Windows stores `user.service`.
pub(crate) const LEGACY_TARGET: &str = "groq-api-key.com.cadence.dictation";

/// Which keyring service a target belongs to. Windows Credential Manager uses
/// the target string itself; this mapping is for the non-Windows store and tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn service_for_target(target: &str) -> Option<&'static str> {
    if target == TARGET {
        Some("com.bolo.dictation")
    } else if target == CADENCE_TARGET || target == LEGACY_TARGET {
        Some("com.cadence.dictation")
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    /// The user saved their own key in the credential store.
    pub configured: bool,
    /// Last four characters of the user's own key. Never set for the built-in key.
    pub hint: Option<String>,
    pub error: Option<String>,
    /// This build compiled in a built-in key. The secret itself is not included.
    pub builtin: bool,
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
    validate_key(key).map_err(|err| anyhow!(err))?;
    write_target(TARGET, key)?;
    match read_target(TARGET)? {
        Some(saved) if saved == key => Ok(()),
        Some(_) => Err(anyhow!(
            "Credential Manager returned a different API key than the one just saved. It was not kept."
        )),
        None => Err(anyhow!(
            "The API key could not be read back from Credential Manager. Nothing was kept."
        )),
    }
}

pub fn clear_api_key() -> anyhow::Result<()> {
    delete_target(TARGET)?;
    delete_target(CADENCE_TARGET)?;
    delete_target(LEGACY_TARGET)?;
    Ok(())
}

pub fn load_api_key() -> anyhow::Result<Option<String>> {
    if let Some(key) = read_target(TARGET)? {
        return Ok(Some(key));
    }
    // Cadence stored the key under either of these. Copy it onto the Bolo
    // target and leave the old entry in place until the user clicks Remove.
    for legacy in [CADENCE_TARGET, LEGACY_TARGET] {
        if let Some(key) = read_target(legacy)? {
            if write_target(TARGET, &key).is_ok() && read_target(TARGET)?.as_deref() == Some(key.as_str()) {
                return Ok(Some(key));
            }
            return Ok(Some(key));
        }
    }
    Ok(None)
}

pub fn status() -> ApiKeyStatus {
    let builtin = crate::builtin::available();
    match load_api_key() {
        Ok(Some(mut key)) => {
            let hint = hint_of(&key);
            key.zeroize();
            ApiKeyStatus {
                configured: true,
                hint: Some(hint),
                error: None,
                builtin,
            }
        }
        Ok(None) => ApiKeyStatus {
            configured: false,
            hint: None,
            error: None,
            builtin,
        },
        Err(err) => ApiKeyStatus {
            configured: false,
            hint: None,
            error: Some(err.to_string()),
            builtin,
        },
    }
}

fn hint_of(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<String>().chars().rev().collect();
    format!("····{tail}")
}

/// Read the key for one request and wipe it when the guard drops.
/// The user's own key wins. Otherwise the key compiled into this build.
pub struct SecretKey {
    value: String,
    builtin: bool,
}

impl SecretKey {
    pub fn load() -> anyhow::Result<Self> {
        let own = load_api_key()?;
        match crate::builtin::select_key_source(own.is_some(), crate::builtin::available()) {
            crate::builtin::KeyChoice::Own => {
                let Some(value) = own else {
                    anyhow::bail!("Add a Groq API key in Bolo settings.");
                };
                Ok(Self {
                    value,
                    builtin: false,
                })
            }
            crate::builtin::KeyChoice::Builtin => {
                let value = crate::builtin::decode().context("built-in Groq key could not be read")?;
                Ok(Self {
                    value,
                    builtin: true,
                })
            }
            crate::builtin::KeyChoice::Required => {
                anyhow::bail!("Add a Groq API key in Bolo settings.")
            }
        }
    }

    pub fn expose(&self) -> &str {
        &self.value
    }

    pub fn is_builtin(&self) -> bool {
        self.builtin
    }
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        cleanup::wipe(&mut self.value);
    }
}

/// Decode a credential blob. keyring 3 writes UTF-16LE. Bolo writes UTF-8.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_secret(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    if bytes.len() % 2 == 0 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        if let Ok(text) = String::from_utf16(&units) {
            let text = text.trim_matches('\0').trim().to_string();
            if !text.is_empty()
                && text.is_ascii()
                && !text.chars().any(|ch| ch.is_control())
            {
                return Some(text);
            }
        }
    }
    let text = String::from_utf8(bytes.to_vec()).ok()?;
    let text = text.trim_matches('\0').trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Process-wide lock for tests that share the isolated credential files.
#[cfg(test)]
pub(crate) static CRED_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Where keys are stored when this process must not touch Credential Manager.
/// Tests always get the isolated folder. A real launch uses the OS store
/// unless `BOLO_CONFIG_DIR` or `BOLO_CREDENTIALS_DIR` is set (smoke tests).
pub(crate) fn credential_store_dir() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        let dir = crate::isolate::isolated_config_dir().join("credentials");
        if crate::isolate::is_inside_real_config(&dir) {
            panic!("test credential store resolved inside the real user config");
        }
        return Some(dir);
    }
    if let Some(dir) = std::env::var_os("BOLO_CREDENTIALS_DIR") {
        let dir = std::path::PathBuf::from(dir);
        if !dir.as_os_str().is_empty() {
            return Some(dir);
        }
    }
    if let Some(dir) = std::env::var_os("BOLO_CONFIG_DIR") {
        let dir = std::path::PathBuf::from(dir);
        if !dir.as_os_str().is_empty() {
            return Some(dir.join("credentials"));
        }
    }
    None
}

fn write_target(target: &str, secret: &str) -> anyhow::Result<()> {
    write_blob(target, secret.as_bytes())
}

fn write_blob(target: &str, secret: &[u8]) -> anyhow::Result<()> {
    if let Some(dir) = credential_store_dir() {
        return file_write(&dir, target, secret);
    }
    platform_write_blob(target, secret)
}

fn read_target(target: &str) -> anyhow::Result<Option<String>> {
    if let Some(dir) = credential_store_dir() {
        return file_read(&dir, target);
    }
    platform_read_target(target)
}

fn delete_target(target: &str) -> anyhow::Result<()> {
    if let Some(dir) = credential_store_dir() {
        return file_delete(&dir, target);
    }
    platform_delete_target(target)
}

fn file_name(target: &str) -> String {
    target
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn file_write(dir: &std::path::Path, target: &str, secret: &[u8]) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).context("couldn't create the isolated credential folder")?;
    let path = dir.join(file_name(target));
    std::fs::write(&path, secret).with_context(|| format!("couldn't write isolated credential {target}"))?;
    Ok(())
}

fn file_read(dir: &std::path::Path, target: &str) -> anyhow::Result<Option<String>> {
    let path = dir.join(file_name(target));
    match std::fs::read(&path) {
        Ok(bytes) => Ok(decode_secret(&bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context("couldn't read the isolated credential"),
    }
}

fn file_delete(dir: &std::path::Path, target: &str) -> anyhow::Result<()> {
    let path = dir.join(file_name(target));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).context("couldn't remove the isolated credential"),
    }
}

fn refuse_real_store_from_tests() -> anyhow::Result<()> {
    if cfg!(test) {
        anyhow::bail!("a test tried to use the real Groq key store");
    }
    Ok(())
}

#[cfg(windows)]
fn platform_write_blob(target: &str, secret: &[u8]) -> anyhow::Result<()> {
    refuse_real_store_from_tests()?;
    use windows::Win32::Security::Credentials::{CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};
    use windows::core::PWSTR;

    let mut target_w = wide(target);
    let mut user_w = wide(ACCOUNT);
    let mut comment_w = wide("Bolo Groq API key");
    let mut alias_w = wide("");
    let mut blob = secret.to_vec();
    let credential = CREDENTIALW {
        Flags: Default::default(),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target_w.as_mut_ptr()),
        Comment: PWSTR(comment_w.as_mut_ptr()),
        LastWritten: Default::default(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR(alias_w.as_mut_ptr()),
        UserName: PWSTR(user_w.as_mut_ptr()),
    };
    let result = unsafe { CredWriteW(&credential, 0) };
    blob.zeroize();
    result.with_context(|| format!("Credential Manager rejected target {target}"))?;
    Ok(())
}

#[cfg(windows)]
fn platform_read_target(target: &str) -> anyhow::Result<Option<String>> {
    refuse_real_store_from_tests()?;
    use windows::Win32::Security::Credentials::{CredFree, CredReadW, CRED_TYPE_GENERIC};
    use windows::core::PCWSTR;

    let target_w = wide(target);
    let mut raw = std::ptr::null_mut();
    let read = unsafe { CredReadW(PCWSTR(target_w.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw) };
    if let Err(err) = read {
        if is_missing(&err) {
            return Ok(None);
        }
        return Err(err).with_context(|| format!("couldn't read credential {target}"));
    }
    if raw.is_null() {
        return Ok(None);
    }
    let secret = unsafe {
        let credential = &*raw;
        let len = credential.CredentialBlobSize as usize;
        let bytes = if credential.CredentialBlob.is_null() || len == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(credential.CredentialBlob, len)
        };
        let secret = decode_secret(bytes);
        CredFree(raw as *const _);
        secret
    };
    Ok(secret)
}

#[cfg(windows)]
fn platform_delete_target(target: &str) -> anyhow::Result<()> {
    refuse_real_store_from_tests()?;
    use windows::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
    use windows::core::PCWSTR;

    let target_w = wide(target);
    match unsafe { CredDeleteW(PCWSTR(target_w.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(()),
        Err(err) if is_missing(&err) => Ok(()),
        Err(err) => Err(err).with_context(|| format!("couldn't delete credential {target}")),
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn is_missing(err: &windows::core::Error) -> bool {
    let code = err.code().0 as u32;
    // HRESULT_FROM_WIN32(ERROR_NOT_FOUND) and ERROR_FILE_NOT_FOUND.
    code == 0x8007_0490 || code == 0x8007_0002 || code == 1168
}

#[cfg(not(windows))]
fn platform_write_blob(target: &str, secret: &[u8]) -> anyhow::Result<()> {
    refuse_real_store_from_tests()?;
    let secret = std::str::from_utf8(secret).context("credential blob was not utf-8")?;
    let service = service_for_target(target).unwrap_or(SERVICE);
    let entry = keyring::Entry::new(service, ACCOUNT).context("couldn't open the credential store")?;
    entry
        .set_password(secret)
        .context("couldn't save the API key to the credential store")?;
    Ok(())
}

#[cfg(not(windows))]
fn platform_read_target(target: &str) -> anyhow::Result<Option<String>> {
    refuse_real_store_from_tests()?;
    let Some(service) = service_for_target(target) else {
        return Ok(None);
    };
    // Silence the unused const when both services are referenced via service_for_target.
    let _ = (SERVICE, SERVICE_CADENCE);
    let entry = keyring::Entry::new(service, ACCOUNT).context("couldn't open the credential store")?;
    match entry.get_password() {
        Ok(key) if key.trim().is_empty() => Ok(None),
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err).context("couldn't read the API key"),
    }
}

#[cfg(not(windows))]
fn platform_delete_target(target: &str) -> anyhow::Result<()> {
    refuse_real_store_from_tests()?;
    let Some(service) = service_for_target(target) else {
        return Ok(());
    };
    let entry = keyring::Entry::new(service, ACCOUNT).context("couldn't open the credential store")?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err).context("couldn't remove the API key"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        CRED_TEST_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    #[test]
    fn rejects_blank_and_short_keys() {
        assert!(validate_key("").is_err());
        assert!(validate_key("gsk_short").is_err());
        assert!(validate_key("gsk_has a space in the token!!").is_err());
        assert!(validate_key("gsk_12345678901234567890").is_ok());
    }

    #[test]
    fn bolo_still_reads_the_cadence_targets() {
        assert_eq!(TARGET, "com.bolo.dictation/groq-api-key");
        assert_eq!(CADENCE_TARGET, "com.cadence.dictation/groq-api-key");
        assert_eq!(LEGACY_TARGET, "groq-api-key.com.cadence.dictation");
        assert_eq!(service_for_target(TARGET), Some("com.bolo.dictation"));
        assert_eq!(service_for_target(CADENCE_TARGET), Some("com.cadence.dictation"));
        assert_eq!(service_for_target(LEGACY_TARGET), Some("com.cadence.dictation"));
        assert_eq!(service_for_target("other"), None);
    }

    #[test]
    fn hint_shows_only_the_last_four() {
        assert_eq!(hint_of("gsk_example_secret_wxyz"), "····wxyz");
    }

    #[test]
    fn builtin_status_payload_has_no_key_material() {
        let reported = ApiKeyStatus {
            configured: false,
            hint: None,
            error: None,
            builtin: true,
        };
        let json = serde_json::to_string(&reported).unwrap();
        assert!(reported.hint.is_none());
        assert!(!json.contains("gsk_"));
        assert!(!json.to_ascii_lowercase().contains("apikey"));
        assert!(!json.to_ascii_lowercase().contains("builtinkey"));
        assert!(json.contains("\"builtin\":true"));
        assert!(json.contains("\"hint\":null"));
    }

    #[test]
    fn decodes_utf8_and_keyring_utf16() {
        let utf8 = b"gsk_12345678901234567890";
        assert_eq!(decode_secret(utf8).as_deref(), Some("gsk_12345678901234567890"));
        let utf16: Vec<u8> = "gsk_12345678901234567890"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        assert_eq!(decode_secret(&utf16).as_deref(), Some("gsk_12345678901234567890"));
        assert_eq!(decode_secret(&[]), None);
    }

    /// Round trip stays in the temp credential folder. It must not touch
    /// Credential Manager or the user's Groq key.
    #[test]
    fn credential_round_trip_stays_in_the_isolated_store() {
        let _serial = lock();
        clear_api_key().expect("clear");
        let probe = "gsk_bolo_selftest_key_123456";
        set_api_key(probe).expect("save probe key");
        let dir = credential_store_dir().expect("isolated store");
        assert!(dir.starts_with(crate::isolate::isolated_config_dir()));
        assert!(!crate::isolate::is_inside_real_config(&dir));
        assert!(dir.join(file_name(TARGET)).is_file());
        assert_eq!(load_api_key().expect("read").as_deref(), Some(probe));
        let reported = status();
        assert!(reported.configured);
        assert_eq!(reported.hint.as_deref(), Some("····3456"));
        clear_api_key().expect("clear probe key");
        assert_eq!(load_api_key().expect("read after clear"), None);
        assert!(!status().configured);
    }

    /// A blob written the way keyring 3 writes Windows passwords must still load.
    #[test]
    fn reads_a_legacy_utf16_blob_from_the_isolated_store() {
        let _serial = lock();
        let target = format!("com.cadence.dictation/selftest-utf16-{}", std::process::id());
        let secret = "gsk_legacy_utf16_key_123456";
        let blob: Vec<u8> = secret.encode_utf16().flat_map(|unit| unit.to_le_bytes()).collect();
        write_blob(&target, &blob).expect("write utf16 blob");
        let loaded = read_target(&target).expect("read utf16 blob");
        delete_target(&target).expect("delete utf16 blob");
        let dir = credential_store_dir().expect("isolated store");
        assert!(!dir.join(file_name(&target)).exists());
        assert!(!crate::isolate::is_inside_real_config(&dir));
        assert_eq!(loaded.as_deref(), Some(secret));
    }

    /// A key saved by Cadence must load, and a copy must land on the Bolo
    /// target, without deleting the original entry. Both files are in the
    /// isolated store.
    #[test]
    fn copies_a_cadence_credential_forward_without_touching_the_real_key() {
        let _serial = lock();
        clear_api_key().expect("clear before carryover");
        let probe = "gsk_cadence_carry_forward_key_99";
        write_target(CADENCE_TARGET, probe).expect("write cadence target");
        assert_eq!(read_target(TARGET).expect("read new target"), None);
        assert_eq!(load_api_key().expect("load").as_deref(), Some(probe));
        assert_eq!(read_target(TARGET).expect("copied").as_deref(), Some(probe));
        assert_eq!(read_target(CADENCE_TARGET).expect("old entry").as_deref(), Some(probe));
        let dir = credential_store_dir().expect("isolated store");
        assert!(dir.starts_with(std::env::temp_dir()));
        assert!(!crate::isolate::is_inside_real_config(&dir.join(file_name(TARGET))));
        clear_api_key().expect("clear probe");
    }

    #[test]
    fn tests_cannot_overwrite_the_real_config_or_groq_key() {
        let _serial = lock();
        let real = crate::isolate::real_user_config_dir();
        let legacy = crate::isolate::real_legacy_config_dir();
        let settings_before = std::fs::metadata(real.join("settings.json")).ok().map(|meta| meta.len());
        let legacy_before = std::fs::metadata(legacy.join("settings.json")).ok().map(|meta| meta.len());
        let log_existed = real.join("bolo.log").exists();
        assert!(crate::store::save_settings(&real, &crate::store::Settings::default()).is_err());
        assert!(crate::store::save_history(&real, &[]).is_err());
        assert!(crate::store::save_settings(&legacy, &crate::store::Settings::default()).is_err());
        crate::logbuf::log_line(&real, "info", "must not land in the user profile");
        assert_eq!(
            std::fs::metadata(real.join("settings.json")).ok().map(|meta| meta.len()),
            settings_before
        );
        assert_eq!(
            std::fs::metadata(legacy.join("settings.json")).ok().map(|meta| meta.len()),
            legacy_before
        );
        if !log_existed {
            assert!(!real.join("bolo.log").exists(), "a test created the real bolo.log");
        }
        let probe = "gsk_isolation_guard_key_1234567890";
        set_api_key(probe).expect("save probe");
        let dir = credential_store_dir().expect("store");
        assert!(dir.starts_with(crate::isolate::isolated_config_dir()));
        assert!(!crate::isolate::is_inside_real_config(&dir));
        assert_eq!(load_api_key().expect("load").as_deref(), Some(probe));
        assert!(dir.join(file_name(TARGET)).is_file());
        clear_api_key().ok();
    }
}
