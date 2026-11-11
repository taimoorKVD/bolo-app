//! Provider failure classification and multi-key continuity.
//!
//! Phase 1 splits Groq HTTP and network errors into transient rate limits,
//! daily quota exhaustion, revoked keys, and outages. Phase 2 keeps several
//! keys (secrets in Credential Manager, metadata on disk) and switches per
//! dictation with cooldowns and automatic switch-back. Accuracy prompts and
//! single-key behavior stay unchanged.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::credentials;
use crate::transcribe::{self, FailKind, GroqFailure};

/// Short 429 waits stay on the same key. Longer waits look like a daily reset.
pub const SHORT_RETRY_MS: u64 = 8_000;
/// A retry-after at or above this is treated as quota, not a burst limit.
pub const QUOTA_RETRY_MS: u64 = 60_000;
/// Default cooldown when Groq reports a daily/TPD limit without a parseable wait.
pub const DEFAULT_QUOTA_COOLDOWN_SECS: u64 = 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Per-minute / burst limit. Retry the same key.
    TransientRateLimit,
    /// Daily or tokens-per-day style limit. Candidate for cooldown + switch.
    QuotaExhausted { explicit: bool },
    /// 401 / 403. Key is dead.
    Revoked,
    /// 5xx, DNS, TLS, timeout. Never mark the key exhausted.
    Outage { offline_hint: bool },
    Other,
}

impl FailureClass {
    pub fn fail_kind(self) -> FailKind {
        match self {
            FailureClass::TransientRateLimit => FailKind::RateLimit,
            FailureClass::QuotaExhausted { .. } => FailKind::QuotaExhausted,
            FailureClass::Revoked => FailKind::Rejected,
            FailureClass::Outage { .. } => FailKind::Unavailable,
            FailureClass::Other => FailKind::Other,
        }
    }

    pub fn explicit_quota(self) -> bool {
        matches!(self, FailureClass::QuotaExhausted { explicit: true })
    }

    pub fn offline_hint(self) -> bool {
        matches!(self, FailureClass::Outage { offline_hint: true })
    }
}

/// Classify an HTTP status and Groq error body. Prefer explicit daily / TPD
/// wording, then a long retry-after, then short burst limits.
pub fn classify_http(status: u16, body: &str) -> FailureClass {
    match status {
        401 | 403 => FailureClass::Revoked,
        429 => classify_429(body),
        500..=599 => FailureClass::Outage { offline_hint: false },
        _ => FailureClass::Other,
    }
}

/// Classify a transport failure (connect / DNS / TLS / timeout).
pub fn classify_network(message: &str) -> FailureClass {
    FailureClass::Outage {
        offline_hint: looks_offline(message),
    }
}

pub fn classify_429(body: &str) -> FailureClass {
    let lower = body.to_ascii_lowercase();
    let wait = transcribe::retry_after_ms(body);
    let explicit = explicit_quota_message(&lower);
    if explicit {
        return FailureClass::QuotaExhausted { explicit: true };
    }
    if let Some(ms) = wait {
        if ms >= QUOTA_RETRY_MS {
            return FailureClass::QuotaExhausted { explicit: false };
        }
        if ms <= SHORT_RETRY_MS {
            return FailureClass::TransientRateLimit;
        }
        // Between short and quota thresholds: treat as quota candidate so a
        // second identical response can cool the key down.
        return FailureClass::QuotaExhausted { explicit: false };
    }
    if quota_candidate_message(&lower) {
        return FailureClass::QuotaExhausted { explicit: false };
    }
    FailureClass::TransientRateLimit
}

pub fn explicit_quota_message(lower: &str) -> bool {
    const MARKERS: &[&str] = &[
        "tokens per day",
        "tokens-per-day",
        "tpd",
        "requests per day",
        "requests-per-day",
        "rpd",
        "daily rate limit",
        "daily limit",
        "daily quota",
        "exceeded your daily",
        "quota exceeded",
        "quota_exceeded",
        "usage limit",
        "used up your",
    ];
    MARKERS.iter().any(|marker| lower.contains(marker))
}

fn quota_candidate_message(lower: &str) -> bool {
    lower.contains("quota")
        || lower.contains("rate_limit_exceeded")
        || (lower.contains("rate limit") && lower.contains("day"))
}

pub fn looks_offline(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "no such host",
        "name or service not known",
        "temporary failure in name resolution",
        "dns error",
        "getaddrinfo",
        "network is unreachable",
        "no route to host",
        "could not connect",
        "connection refused",
        "failed to lookup",
        "offline",
        "not connected",
        "tls handshake",
        "certificate",
        "ssl",
        "timed out",
        "timeout",
        "error sending request",
    ];
    // "timed out" / "tls" alone can also be a provider outage; they still
    // count as outage (never exhaust). offline_hint is true when the wording
    // points at local connectivity so the router skips key churn.
    const LOCAL: &[&str] = &[
        "no such host",
        "name or service not known",
        "temporary failure in name resolution",
        "dns error",
        "getaddrinfo",
        "network is unreachable",
        "no route to host",
        "offline",
        "not connected",
    ];
    let _ = MARKERS;
    LOCAL.iter().any(|marker| lower.contains(marker))
        || (lower.contains("dns") && lower.contains("error"))
}

pub fn classify_to_failure(status: u16, body: &str, using_builtin: bool) -> GroqFailure {
    let class = classify_http(status, body);
    let message = crate::cleanup::groq_error_message(status, body, using_builtin);
    GroqFailure {
        message,
        kind: class.fail_kind(),
        status: Some(status),
        explicit_quota: class.explicit_quota(),
        retry_after_ms: transcribe::retry_after_ms(body),
        offline_hint: class.offline_hint(),
    }
}

pub fn network_to_failure(chain: String) -> GroqFailure {
    let class = classify_network(&chain);
    GroqFailure {
        message: format!("couldn't reach Groq: {chain}"),
        kind: class.fail_kind(),
        status: None,
        explicit_quota: false,
        retry_after_ms: None,
        offline_hint: class.offline_hint(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyState {
    Ok,
    Cooling,
    Dead,
}

impl Default for KeyState {
    fn default() -> Self {
        KeyState::Ok
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyKind {
    User,
    Builtin,
    /// Bolo-managed Worker access (install token). Scaffolded in phase 3.
    Managed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyMeta {
    pub id: String,
    pub provider: String,
    pub kind: KeyKind,
    /// Last four characters only. Never the secret.
    pub hint: Option<String>,
    pub state: KeyState,
    /// Unix ms when a Cooling key may be tried again.
    pub cooldown_until_ms: Option<u64>,
    pub quota_hits: u32,
    pub failure_count: u32,
    pub last_ok_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl KeyMeta {
    pub fn user(id: impl Into<String>, hint: Option<String>) -> Self {
        Self {
            id: id.into(),
            provider: "groq".into(),
            kind: KeyKind::User,
            hint,
            state: KeyState::Ok,
            cooldown_until_ms: None,
            quota_hits: 0,
            failure_count: 0,
            last_ok_ms: None,
            last_error: None,
        }
    }

    pub fn builtin() -> Self {
        Self {
            id: "builtin".into(),
            provider: "groq".into(),
            kind: KeyKind::Builtin,
            hint: None,
            state: KeyState::Ok,
            cooldown_until_ms: None,
            quota_hits: 0,
            failure_count: 0,
            last_ok_ms: None,
            last_error: None,
        }
    }

    #[allow(dead_code)]
    pub fn managed(hint: Option<String>) -> Self {
        Self {
            id: "managed".into(),
            provider: "bolo".into(),
            kind: KeyKind::Managed,
            hint,
            state: KeyState::Ok,
            cooldown_until_ms: None,
            quota_hits: 0,
            failure_count: 0,
            last_ok_ms: None,
            last_error: None,
        }
    }

    pub fn is_ready(&self, now_ms: u64) -> bool {
        match self.state {
            KeyState::Ok => true,
            KeyState::Dead => false,
            KeyState::Cooling => self
                .cooldown_until_ms
                .map(|until| until <= now_ms)
                .unwrap_or(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStore {
    pub keys: Vec<KeyMeta>,
    /// Preferred user key id when several are ready.
    pub preferred: Option<String>,
}

impl ProviderStore {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("providers.json")
    }

    pub fn load(config_dir: &Path) -> Self {
        let path = Self::path(config_dir);
        let mut store = match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => ProviderStore::default(),
        };
        store.reconcile_with_credentials();
        store.refresh_cooldowns(now_ms());
        store
    }

    pub fn save(&self, config_dir: &Path) -> anyhow::Result<()> {
        fs::create_dir_all(config_dir)?;
        let path = Self::path(config_dir);
        let raw = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, raw)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Ensure metadata matches Credential Manager / builtin availability.
    /// Secrets are never written here.
    pub fn reconcile_with_credentials(&mut self) {
        let user_configured = credentials::load_api_key()
            .ok()
            .and_then(|key| key.map(|mut value| {
                let hint = hint_of(&value);
                crate::cleanup::wipe(&mut value);
                hint
            }));
        if let Some(hint) = user_configured {
            if let Some(existing) = self.keys.iter_mut().find(|key| key.id == "default") {
                existing.hint = Some(hint);
                existing.kind = KeyKind::User;
                existing.provider = "groq".into();
            } else {
                self.keys.insert(0, KeyMeta::user("default", Some(hint)));
            }
            if self.preferred.is_none() {
                self.preferred = Some("default".into());
            }
        } else {
            self.keys.retain(|key| key.id != "default" || key.kind != KeyKind::User);
        }

        for id in BACKUP_IDS {
            match credentials::load_provider_key(id) {
                Ok(Some(mut value)) => {
                    let hint = hint_of(&value);
                    crate::cleanup::wipe(&mut value);
                    if let Some(existing) = self.keys.iter_mut().find(|key| key.id == *id) {
                        existing.hint = Some(hint);
                        existing.kind = KeyKind::User;
                        existing.provider = "groq".into();
                    } else {
                        self.keys.push(KeyMeta::user(id, Some(hint)));
                    }
                }
                Ok(None) => self.keys.retain(|key| key.id != *id),
                Err(_) => {}
            }
        }

        if crate::builtin::available() {
            if !self.keys.iter().any(|key| key.id == "builtin") {
                self.keys.push(KeyMeta::builtin());
            }
        } else {
            self.keys.retain(|key| key.id != "builtin");
        }
    }

    pub fn refresh_cooldowns(&mut self, now_ms: u64) {
        for key in &mut self.keys {
            if key.state == KeyState::Cooling {
                if key.is_ready(now_ms) {
                    key.state = KeyState::Ok;
                    key.cooldown_until_ms = None;
                    key.quota_hits = 0;
                }
            }
        }
    }

    pub fn ready_ids(&self, now_ms: u64) -> Vec<String> {
        let mut ready: Vec<&KeyMeta> = self.keys.iter().filter(|key| key.is_ready(now_ms)).collect();
        ready.sort_by_key(|key| {
            let preferred = self.preferred.as_deref() == Some(key.id.as_str());
            // preferred user first, then other users, then builtin.
            match (preferred, key.kind) {
                (true, _) => 0u8,
                (false, KeyKind::Managed) => 0,
                (false, KeyKind::User) => 1,
                (false, KeyKind::Builtin) => 2,
            }
        });
        ready.into_iter().map(|key| key.id.clone()).collect()
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut KeyMeta> {
        self.keys.iter_mut().find(|key| key.id == id)
    }

    pub fn record_success(&mut self, id: &str) {
        let now = now_ms();
        if let Some(key) = self.get_mut(id) {
            key.state = KeyState::Ok;
            key.cooldown_until_ms = None;
            key.quota_hits = 0;
            key.failure_count = 0;
            key.last_ok_ms = Some(now);
            key.last_error = None;
        }
        self.preferred = Some(id.to_string());
    }

    /// Apply a classified failure. Returns whether the router should try the
    /// next key (true) or stop / fall offline (false for hard reject with no
    /// remaining keys is decided by the caller).
    pub fn record_failure(&mut self, id: &str, failure: &GroqFailure) -> RouterAction {
        let now = now_ms();
        let Some(key) = self.get_mut(id) else {
            return RouterAction::Switch;
        };
        key.failure_count = key.failure_count.saturating_add(1);
        key.last_error = Some(compact_error(&failure.message));

        match failure.kind {
            FailKind::RateLimit => RouterAction::RetrySame,
            FailKind::QuotaExhausted => {
                key.quota_hits = key.quota_hits.saturating_add(1);
                let confirm = failure.explicit_quota || key.quota_hits >= 2;
                if confirm {
                    let wait = failure
                        .retry_after_ms
                        .filter(|ms| *ms >= QUOTA_RETRY_MS)
                        .unwrap_or(DEFAULT_QUOTA_COOLDOWN_SECS * 1000);
                    key.state = KeyState::Cooling;
                    key.cooldown_until_ms = Some(now.saturating_add(wait));
                    key.quota_hits = 0;
                    RouterAction::Switch
                } else {
                    // One ambiguous quota-class response: do not cool down yet.
                    RouterAction::RetrySame
                }
            }
            FailKind::Rejected => {
                // One 401 used to mark the key dead for the rest of the
                // session, so later takes never called Groq at all. Try again
                // next take. This take still moves on to the next key.
                RouterAction::Switch
            }
            FailKind::Unavailable => {
                if failure.offline_hint {
                    RouterAction::Offline
                } else {
                    RouterAction::Outage
                }
            }
            FailKind::Other => RouterAction::Switch,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterAction {
    RetrySame,
    Switch,
    Outage,
    Offline,
}

fn hint_of(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<String>().chars().rev().collect();
    format!("····{tail}")
}

fn compact_error(message: &str) -> String {
    let message = crate::builtin::redact_secrets(message);
    let one = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= 160 {
        one
    } else {
        one.chars().take(160).collect()
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

/// Loaded secrets for one dictation pass. Drop wipes them.
pub struct KeyMaterial {
    pub id: String,
    pub builtin: bool,
    value: String,
}

impl KeyMaterial {
    pub fn expose(&self) -> &str {
        &self.value
    }
}

impl Drop for KeyMaterial {
    fn drop(&mut self) {
        crate::cleanup::wipe(&mut self.value);
    }
}

/// Resolve ready keys for this take. Secrets come from Credential Manager or
/// the built-in key; metadata decides order and cooldowns.
pub fn load_ready_keys(config_dir: &Path) -> anyhow::Result<(ProviderStore, Vec<KeyMaterial>)> {
    let mut store = ProviderStore::load(config_dir);
    let now = now_ms();
    store.refresh_cooldowns(now);
    let ids = store.ready_ids(now);
    let mut materials = Vec::new();
    for id in ids {
        match load_secret_for(&id) {
            Ok(Some(material)) => materials.push(material),
            Ok(None) => {
                if let Some(meta) = store.get_mut(&id) {
                    if meta.kind == KeyKind::User {
                        meta.state = KeyState::Dead;
                        meta.last_error = Some("key missing from Credential Manager".into());
                    }
                }
            }
            Err(err) => {
                if let Some(meta) = store.get_mut(&id) {
                    meta.last_error = Some(compact_error(&err.to_string()));
                }
            }
        }
    }
    let _ = store.save(config_dir);
    if materials.is_empty() {
        let reason = store
            .keys
            .iter()
            .find_map(|key| key.last_error.clone())
            .unwrap_or_else(|| "Add a Groq API key in Bolo settings.".into());
        anyhow::bail!(crate::builtin::redact_secrets(&reason));
    }
    Ok((store, materials))
}

fn load_secret_for(id: &str) -> anyhow::Result<Option<KeyMaterial>> {
    if id == "builtin" {
        let Some(value) = crate::builtin::decode() else {
            return Ok(None);
        };
        return Ok(Some(KeyMaterial {
            id: id.to_string(),
            builtin: true,
            value,
        }));
    }
    // "default" and any extra user ids share the same helper.
    let secret = if id == "default" {
        credentials::load_api_key()?
    } else {
        credentials::load_provider_key(id)?
    };
    Ok(secret.map(|value| KeyMaterial {
        id: id.to_string(),
        builtin: false,
        value,
    }))
}

/// Persist metadata after a take. Best-effort; never blocks dictation on disk errors.
pub fn save_store(config_dir: &Path, store: &ProviderStore) {
    let _ = store.save(config_dir);
}

/// Backup slots. The primary key stays `default`.
pub const BACKUP_IDS: [&str; 3] = ["backup-1", "backup-2", "backup-3"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Accepted,
    Rejected,
    Unreachable,
}

/// What a Groq models check means. The body is ignored so a key cannot leak out of it.
pub fn classify_probe(status: Option<u16>) -> Probe {
    match status {
        Some(200..=299) => Probe::Accepted,
        Some(401 | 403) => Probe::Rejected,
        _ => Probe::Unreachable,
    }
}

pub fn probe_message(probe: Probe) -> Option<&'static str> {
    match probe {
        Probe::Accepted => None,
        Probe::Rejected => Some("Groq did not accept that key. Check it and paste it again."),
        Probe::Unreachable => {
            Some("Bolo could not confirm the key with Groq just now. It is saved on this PC and will be tried on the next take.")
        }
    }
}

pub async fn probe_key(client: &reqwest::Client, key: &str) -> Probe {
    match client
        .get("https://api.groq.com/openai/v1/models")
        .bearer_auth(key)
        .send()
        .await
    {
        Ok(response) => classify_probe(Some(response.status().as_u16())),
        Err(_) => classify_probe(None),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderKeyRow {
    pub id: String,
    pub hint: Option<String>,
    pub state: String,
    pub kind: String,
    pub preferred: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderKeyStatus {
    pub keys: Vec<ProviderKeyRow>,
    pub note: Option<String>,
}

pub fn rows_from(store: &ProviderStore) -> Vec<ProviderKeyRow> {
    store
        .keys
        .iter()
        .filter(|key| key.kind != KeyKind::Managed)
        .map(|key| ProviderKeyRow {
            id: key.id.clone(),
            hint: key.hint.clone(),
            state: match key.state {
                KeyState::Ok => "ok",
                KeyState::Cooling => "cooling",
                KeyState::Dead => "dead",
            }
            .into(),
            kind: match key.kind {
                KeyKind::User => "user",
                KeyKind::Builtin => "builtin",
                KeyKind::Managed => "managed",
            }
            .into(),
            preferred: store.preferred.as_deref() == Some(key.id.as_str()),
            last_error: key.last_error.as_ref().map(|err| crate::builtin::redact_secrets(err)),
        })
        .collect()
}

pub fn list_rows(config_dir: &Path) -> Vec<ProviderKeyRow> {
    rows_from(&ProviderStore::load(config_dir))
}

pub fn next_backup_id(store: &ProviderStore) -> Result<String, String> {
    for id in BACKUP_IDS {
        let taken = store.keys.iter().any(|key| key.id == id && key.kind == KeyKind::User);
        if !taken {
            return Ok(id.to_string());
        }
    }
    Err("You already have three backup keys. Remove one to add another.".into())
}

fn backup_id(id: &str) -> bool {
    BACKUP_IDS.contains(&id)
}

/// Save a user key. `new` takes the next backup slot. Secrets stay in the
/// credential store; this file keeps the hint and the slot only.
pub fn store_user_key(config_dir: &Path, requested: &str, key: &str) -> Result<(), String> {
    let requested = requested.trim();
    let id = if requested.is_empty() || requested == "new" {
        next_backup_id(&ProviderStore::load(config_dir))?
    } else if requested == "default" || backup_id(requested) {
        requested.to_string()
    } else {
        return Err("That key slot is not available.".into());
    };
    if id == "default" {
        credentials::set_api_key(key).map_err(|err| crate::builtin::redact_secrets(&err.to_string()))?;
        let status = credentials::status();
        sync_primary_key(config_dir, status.configured, status.hint);
        return Ok(());
    }
    credentials::set_provider_key(&id, key).map_err(|err| crate::builtin::redact_secrets(&err.to_string()))?;
    let mut store = ProviderStore::load(config_dir);
    let hint = Some(hint_of(key));
    if let Some(existing) = store.get_mut(&id) {
        existing.hint = hint;
        existing.kind = KeyKind::User;
        existing.provider = "groq".into();
        existing.state = KeyState::Ok;
        existing.cooldown_until_ms = None;
        existing.quota_hits = 0;
        existing.last_error = None;
    } else {
        store.keys.push(KeyMeta::user(&id, hint));
    }
    store.save(config_dir).map_err(|err| crate::builtin::redact_secrets(&err.to_string()))?;
    Ok(())
}

pub fn revoke_user_key(config_dir: &Path, id: &str) -> Result<(), String> {
    let id = id.trim();
    if id == "builtin" || id == "managed" {
        return Err("That one belongs to Bolo. Remove a key you added.".into());
    }
    if id == "default" {
        credentials::clear_api_key().map_err(|err| crate::builtin::redact_secrets(&err.to_string()))?;
        sync_primary_key(config_dir, false, None);
        return Ok(());
    }
    if !backup_id(id) {
        return Err("That key slot is not available.".into());
    }
    credentials::clear_provider_key(id).map_err(|err| crate::builtin::redact_secrets(&err.to_string()))?;
    let mut store = ProviderStore::load(config_dir);
    store.keys.retain(|key| key.id != id);
    if store.preferred.as_deref() == Some(id) {
        store.preferred = store.keys.iter().find(|key| key.kind == KeyKind::User).map(|key| key.id.clone());
    }
    store.save(config_dir).map_err(|err| crate::builtin::redact_secrets(&err.to_string()))?;
    Ok(())
}

/// Notify the store that the user saved or cleared the primary key.
pub fn sync_primary_key(config_dir: &Path, configured: bool, hint: Option<String>) {
    let mut store = ProviderStore::load(config_dir);
    if configured {
        if let Some(existing) = store.get_mut("default") {
            existing.hint = hint;
            existing.state = KeyState::Ok;
            existing.cooldown_until_ms = None;
            existing.quota_hits = 0;
            existing.kind = KeyKind::User;
        } else {
            store.keys.insert(0, KeyMeta::user("default", hint));
        }
        store.preferred = Some("default".into());
    } else {
        store.keys.retain(|key| key.id != "default");
        if store.preferred.as_deref() == Some("default") {
            store.preferred = store
                .keys
                .iter()
                .find(|key| key.kind == KeyKind::User)
                .map(|key| key.id.clone());
        }
    }
    store.reconcile_with_credentials();
    let _ = store.save(config_dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory vault used by router unit tests. Never touches Credential Manager.
    struct MemoryVault {
        secrets: HashMap<String, String>,
    }

    impl MemoryVault {
        fn new() -> Self {
            Self {
                secrets: HashMap::new(),
            }
        }
    }

    /// Programmed backend responses keyed by key id. Each call pops the next result.
    struct MockProvider {
        scripts: HashMap<String, Vec<Result<(), GroqFailure>>>,
        calls: Vec<String>,
    }

    impl MockProvider {
        fn new() -> Self {
            Self {
                scripts: HashMap::new(),
                calls: Vec::new(),
            }
        }

        fn push(&mut self, id: &str, result: Result<(), GroqFailure>) {
            self.scripts.entry(id.to_string()).or_default().push(result);
        }

        fn call(&mut self, id: &str) -> Result<(), GroqFailure> {
            self.calls.push(id.to_string());
            let queue = self
                .scripts
                .get_mut(id)
                .and_then(|items| if items.is_empty() { None } else { Some(items.remove(0)) });
            queue.unwrap_or(Ok(()))
        }
    }

    fn failure(kind: FailKind, explicit: bool, retry_ms: Option<u64>, offline: bool) -> GroqFailure {
        GroqFailure {
            message: format!("{kind:?}"),
            kind,
            status: None,
            explicit_quota: explicit,
            retry_after_ms: retry_ms,
            offline_hint: offline,
        }
    }

    fn run_router(
        store: &mut ProviderStore,
        order: &[String],
        mock: &mut MockProvider,
    ) -> Result<String, RouterAction> {
        let mut offline = false;
        let mut outages = 0usize;
        for id in order {
            // Up to two attempts on the same key for RetrySame (quota confirm / burst).
            for _ in 0..3 {
                match mock.call(id) {
                    Ok(()) => {
                        store.record_success(id);
                        return Ok(id.clone());
                    }
                    Err(err) => match store.record_failure(id, &err) {
                        RouterAction::RetrySame => continue,
                        RouterAction::Switch => break,
                        RouterAction::Outage => {
                            outages += 1;
                            break;
                        }
                        RouterAction::Offline => {
                            offline = true;
                            break;
                        }
                    },
                }
            }
            if offline {
                return Err(RouterAction::Offline);
            }
        }
        if offline || outages > 0 {
            return Err(if offline {
                RouterAction::Offline
            } else {
                RouterAction::Outage
            });
        }
        Err(RouterAction::Switch)
    }

    #[test]
    fn short_429_is_transient_rate_limit() {
        let body = r#"{"error":{"message":"Please try again in 3.2s. Need more?"}}"#;
        assert_eq!(classify_http(429, body), FailureClass::TransientRateLimit);
        assert_eq!(classify_429("rate limited"), FailureClass::TransientRateLimit);
    }

    #[test]
    fn daily_tpd_message_is_explicit_quota() {
        let body = r#"{"error":{"message":"Rate limit reached for model whisper-large-v3-turbo in organization org_x on tokens per day (TPD): Limit 100000"}}"#;
        assert_eq!(
            classify_http(429, body),
            FailureClass::QuotaExhausted { explicit: true }
        );
        let long = r#"{"error":{"message":"Please try again in 7m12.5s. Need more"}}"#;
        assert_eq!(
            classify_http(429, long),
            FailureClass::QuotaExhausted { explicit: false }
        );
    }

    #[test]
    fn auth_and_server_errors_classify_cleanly() {
        assert_eq!(classify_http(401, "{}"), FailureClass::Revoked);
        assert_eq!(classify_http(403, "{}"), FailureClass::Revoked);
        assert_eq!(
            classify_http(503, "nope"),
            FailureClass::Outage { offline_hint: false }
        );
        assert_eq!(
            classify_network("error sending request: dns error: No such host is known."),
            FailureClass::Outage { offline_hint: true }
        );
        assert_eq!(
            classify_network("error sending request: connection timed out"),
            FailureClass::Outage { offline_hint: false }
        );
    }

    #[test]
    fn ambiguous_quota_needs_two_hits_before_switch() {
        let mut store = ProviderStore {
            keys: vec![KeyMeta::user("a", Some("····aaa1".into()))],
            preferred: Some("a".into()),
        };
        let first = failure(FailKind::QuotaExhausted, false, None, false);
        assert_eq!(store.record_failure("a", &first), RouterAction::RetrySame);
        assert_eq!(store.keys[0].state, KeyState::Ok);
        assert_eq!(store.keys[0].quota_hits, 1);

        let second = failure(FailKind::QuotaExhausted, false, None, false);
        assert_eq!(store.record_failure("a", &second), RouterAction::Switch);
        assert_eq!(store.keys[0].state, KeyState::Cooling);
        assert!(store.keys[0].cooldown_until_ms.is_some());
    }

    #[test]
    fn explicit_quota_switches_immediately() {
        let mut store = ProviderStore {
            keys: vec![KeyMeta::user("a", None)],
            preferred: None,
        };
        let hit = failure(FailKind::QuotaExhausted, true, Some(3_600_000), false);
        assert_eq!(store.record_failure("a", &hit), RouterAction::Switch);
        assert_eq!(store.keys[0].state, KeyState::Cooling);
    }

    #[test]
    fn outage_never_marks_key_exhausted() {
        let mut store = ProviderStore {
            keys: vec![KeyMeta::user("a", None)],
            preferred: None,
        };
        let outage = failure(FailKind::Unavailable, false, None, false);
        assert_eq!(store.record_failure("a", &outage), RouterAction::Outage);
        assert_eq!(store.keys[0].state, KeyState::Ok);

        let offline = failure(FailKind::Unavailable, false, None, true);
        assert_eq!(store.record_failure("a", &offline), RouterAction::Offline);
        assert_eq!(store.keys[0].state, KeyState::Ok);
    }

    #[test]
    fn revoked_marks_dead_and_router_falls_to_next_key() {
        let mut store = ProviderStore {
            keys: vec![
                KeyMeta::user("a", None),
                KeyMeta::user("b", None),
            ],
            preferred: Some("a".into()),
        };
        let mut mock = MockProvider::new();
        mock.push("a", Err(failure(FailKind::Rejected, false, None, false)));
        mock.push("b", Ok(()));
        let order = store.ready_ids(now_ms());
        let winner = run_router(&mut store, &order, &mut mock).unwrap();
        assert_eq!(winner, "b");
        assert_eq!(
            store.keys.iter().find(|k| k.id == "a").unwrap().state,
            KeyState::Ok,
            "a rejected key is tried again on the next take"
        );
        assert_eq!(store.preferred.as_deref(), Some("b"));
        assert_eq!(mock.calls, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn cooldown_expires_and_preferred_key_is_used_again() {
        let mut store = ProviderStore {
            keys: vec![
                KeyMeta {
                    cooldown_until_ms: Some(1),
                    state: KeyState::Cooling,
                    ..KeyMeta::user("a", None)
                },
                KeyMeta::user("b", None),
            ],
            preferred: Some("a".into()),
        };
        store.refresh_cooldowns(now_ms());
        assert_eq!(store.keys[0].state, KeyState::Ok);
        let order = store.ready_ids(now_ms());
        assert_eq!(order[0], "a");
    }

    #[test]
    fn single_key_success_path_preserves_preference() {
        let mut store = ProviderStore {
            keys: vec![KeyMeta::user("default", Some("····wxyz".into()))],
            preferred: Some("default".into()),
        };
        let mut mock = MockProvider::new();
        mock.push("default", Ok(()));
        let order = store.ready_ids(now_ms());
        let winner = run_router(&mut store, &order, &mut mock).unwrap();
        assert_eq!(winner, "default");
        assert_eq!(store.keys[0].state, KeyState::Ok);
        assert_eq!(store.keys[0].quota_hits, 0);
    }

    #[test]
    fn mock_server_quota_then_backup_key() {
        // Tiny HTTP mock: proves classification of a real status line + body
        // without touching Credential Manager or Groq.
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = r#"{"error":{"message":"Rate limit reached on tokens per day (TPD): Limit 1"}}"#;
            let response = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });
        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        stream
            .write_all(b"GET /v1/audio/transcriptions HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        server.join().unwrap();
        let status = raw
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        // Accept both CRLF and LF body separators from the OS stack.
        let body = if body.is_empty() {
            raw.split("\n\n").nth(1).unwrap_or("").trim().to_string()
        } else {
            body.trim().to_string()
        };
        assert_eq!(status, 429);
        assert_eq!(
            classify_http(status, &body),
            FailureClass::QuotaExhausted { explicit: true }
        );

        let mut store = ProviderStore {
            keys: vec![KeyMeta::user("primary", None), KeyMeta::user("backup", None)],
            preferred: Some("primary".into()),
        };
        let mut mock = MockProvider::new();
        mock.push(
            "primary",
            Err(failure(FailKind::QuotaExhausted, true, Some(3_600_000), false)),
        );
        mock.push("backup", Ok(()));
        let order = store.ready_ids(now_ms());
        assert_eq!(run_router(&mut store, &order, &mut mock).unwrap(), "backup");
        assert_eq!(
            store.keys.iter().find(|k| k.id == "primary").unwrap().state,
            KeyState::Cooling
        );
    }

    #[test]
    fn providers_json_round_trip_has_no_secrets() {
        let dir = std::env::temp_dir().join(format!("bolo-provider-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let store = ProviderStore {
            keys: vec![KeyMeta::user("default", Some("····abcd".into()))],
            preferred: Some("default".into()),
        };
        store.save(&dir).unwrap();
        let raw = fs::read_to_string(ProviderStore::path(&dir)).unwrap();
        assert!(raw.contains("····abcd"));
        assert!(!raw.contains("gsk_"));
        assert!(!raw.to_ascii_lowercase().contains("apikey"));
        let loaded: ProviderStore = serde_json::from_str(&raw).unwrap();
        assert_eq!(loaded.keys[0].id, "default");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_results_and_backup_slots_never_store_a_secret() {
        assert_eq!(classify_probe(Some(200)), Probe::Accepted);
        assert_eq!(classify_probe(Some(401)), Probe::Rejected);
        assert_eq!(classify_probe(Some(403)), Probe::Rejected);
        assert_eq!(classify_probe(Some(429)), Probe::Unreachable);
        assert_eq!(classify_probe(None), Probe::Unreachable);
        assert!(probe_message(Probe::Rejected).unwrap().contains("did not accept"));
        let mut store = ProviderStore {
            keys: vec![
                KeyMeta::user("backup-1", Some("····aaaa".into())),
                KeyMeta::user("backup-2", Some("····bbbb".into())),
                KeyMeta::user("backup-3", Some("····cccc".into())),
            ],
            preferred: None,
        };
        assert!(next_backup_id(&store).unwrap_err().contains("three backup"));
        store.keys.pop();
        assert_eq!(next_backup_id(&store).unwrap(), "backup-3");
        store.keys[0].last_error = Some("rejected gsk_test_fixture_key_value_xxxxxxxx".into());
        let raw = serde_json::to_string(&rows_from(&store)).unwrap();
        assert!(raw.contains("····aaaa"));
        assert!(!raw.contains("gsk_"), "{raw}");
        assert!(!raw.to_ascii_lowercase().contains("bearer"));
    }

    #[test]
    fn memory_vault_never_holds_production_targets() {
        let mut vault = MemoryVault::new();
        vault
            .secrets
            .insert("test-only".into(), "gsk_test_not_real_0000000000".into());
        assert!(vault.secrets.contains_key("test-only"));
        assert!(!vault.secrets.contains_key(credentials::TARGET));
    }
}
