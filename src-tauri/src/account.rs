//! Bolo managed-account scaffolding (phase 3).
//!
//! One-time email OTP sign-up, per-install device key material in Credential
//! Manager, short-lived access tokens, and a client for the local/mock Worker.
//! Private keys and tokens never go in settings.json or logs.
//!
//! `managed` calls this client when a session is configured. Dictation tries
//! the managed API first, then the provider router and on-device speech.
//! Without a session, nothing here runs.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::credentials;

/// Credential Manager target for this install's device private key material.
pub const DEVICE_KEY_TARGET: &str = "com.bolo.dictation/account/device-key";
/// Short-lived access token (raw; hash lives on the server).
pub const ACCESS_TOKEN_TARGET: &str = "com.bolo.dictation/account/access-token";
/// Refresh token (rotated on every use server-side).
pub const REFRESH_TOKEN_TARGET: &str = "com.bolo.dictation/account/refresh-token";

const SESSION_FILE: &str = "bolo-account.json";
const DEFAULT_API_BASE: &str = "http://127.0.0.1:8787";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSessionMeta {
    pub email: String,
    pub user_id: String,
    pub install_id: String,
    pub token_public_id: String,
    pub access_last4: Option<String>,
    pub public_key_fingerprint: String,
    pub api_base_url: String,
    pub signed_in_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceKeyPublic {
    /// Opaque public key material sent to `/v1/installs/register`.
    pub public_key: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct DeviceKeyPair {
    pub private_key_hex: String,
    pub public: DeviceKeyPublic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub kill_switch: bool,
    #[serde(default)]
    pub open_router_enabled: bool,
    #[serde(default)]
    pub paid_usage_enabled: bool,
    #[serde(default)]
    pub authenticated: bool,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub remaining_daily_minutes: Option<f64>,
    #[serde(default)]
    pub remaining_monthly_minutes: Option<f64>,
    #[serde(default)]
    pub reconnect_required: Option<bool>,
    #[serde(default)]
    pub pool_full: Option<bool>,
    #[serde(default)]
    pub sign_in_required: Option<bool>,
    /// Placeholder user notices (admin portal). Shown as text only.
    #[serde(default)]
    pub notices: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedFailure {
    KillSwitch,
    PoolFull,
    FairUse,
    Reissued,
    Revoked,
    Auth,
    Network(String),
    Other(String),
}

impl ManagedFailure {
    pub fn from_code(code: &str) -> Self {
        match code {
            "kill_switch" => ManagedFailure::KillSwitch,
            "pool_full" => ManagedFailure::PoolFull,
            "fair_use_daily" | "fair_use_monthly" => ManagedFailure::FairUse,
            "reissued" => ManagedFailure::Reissued,
            "revoked" | "refresh_reuse" | "device_revoked" | "registration_ticket_required"
            | "registration_ticket_invalid" | "registration_ticket_expired" | "device_proof_required"
            | "device_proof_invalid" => ManagedFailure::Revoked,
            "invalid_token" | "token_expired" | "missing_token" => ManagedFailure::Auth,
            other => ManagedFailure::Other(other.to_string()),
        }
    }
}

pub fn session_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SESSION_FILE)
}

pub fn load_session_meta(config_dir: &Path) -> Option<AccountSessionMeta> {
    let raw = fs::read_to_string(session_path(config_dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn save_session_meta(config_dir: &Path, meta: &AccountSessionMeta) -> anyhow::Result<()> {
    fs::create_dir_all(config_dir)?;
    let raw = serde_json::to_string_pretty(meta)?;
    // Refuse to persist anything that looks like a token/secret field slipped in.
    let lower = raw.to_ascii_lowercase();
    if lower.contains("accesstoken")
        || lower.contains("refreshtoken")
        || lower.contains("privatekey")
        || lower.contains("\"gsk_")
    {
        anyhow::bail!("refusing to write account session with secret-like fields");
    }
    fs::write(session_path(config_dir), raw)?;
    Ok(())
}

pub fn clear_session_meta(config_dir: &Path) {
    let _ = fs::remove_file(session_path(config_dir));
}

pub fn api_base_url(config_dir: &Path) -> String {
    load_session_meta(config_dir)
        .map(|m| m.api_base_url)
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
}

/// Device identity is an Ed25519 key. The 32-byte seed (hex) stays in
/// Credential Manager; the server only sees the SPKI public key (base64) and
/// a signature, so a copied install ID alone cannot register.
pub fn generate_device_keypair() -> DeviceKeyPair {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("os random");
    let private_key_hex = hex_encode(&bytes);
    bytes.fill(0);
    DeviceKeyPair { public: public_from_private(&private_key_hex), private_key_hex }
}

const ED25519_SPKI_PREFIX: [u8; 12] = [0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];

fn ed25519_pair(private_key_hex: &str) -> Option<ring::signature::Ed25519KeyPair> {
    let seed = hex_decode(private_key_hex.trim())?;
    if seed.len() != 32 {
        return None;
    }
    ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed).ok()
}

/// Rebuild the public identity from the stored private key (re-registration).
pub fn public_from_private(private_key_hex: &str) -> DeviceKeyPublic {
    use ring::signature::KeyPair;
    let public_key = match ed25519_pair(private_key_hex) {
        Some(pair) => {
            let mut spki = ED25519_SPKI_PREFIX.to_vec();
            spki.extend_from_slice(pair.public_key().as_ref());
            base64_encode(&spki)
        }
        None => String::new(),
    };
    let fingerprint = public_fingerprint(&public_key);
    DeviceKeyPublic { public_key, fingerprint }
}

/// Same as the Worker's `fingerprintPublicKey`.
pub fn public_fingerprint(public_key: &str) -> String {
    hex_encode(&Sha256::digest(public_key.trim().as_bytes()))[..16].to_string()
}

/// Must match `registrationMessage` in the Worker.
pub fn registration_message(user_id: &str, public_key: &str, signed_at_ms: u64) -> String {
    format!("bolo-register-v1\n{user_id}\n{public_key}\n{signed_at_ms}")
}

pub fn sign_registration(private_key_hex: &str, user_id: &str, public_key: &str, signed_at_ms: u64) -> Option<String> {
    let pair = ed25519_pair(private_key_hex)?;
    let sig = pair.sign(registration_message(user_id, public_key, signed_at_ms).as_bytes());
    Some(base64_encode(sig.as_ref()))
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

/// Persist device private key (Credential Manager). Never logs the value.
pub fn store_device_private_key(private_key_hex: &str) -> anyhow::Result<()> {
    credentials::set_account_secret(DEVICE_KEY_TARGET, private_key_hex)
}

pub fn load_device_private_key() -> anyhow::Result<Option<String>> {
    credentials::load_account_secret(DEVICE_KEY_TARGET)
}

pub fn store_access_token(token: &str) -> anyhow::Result<()> {
    credentials::set_account_secret(ACCESS_TOKEN_TARGET, token)
}

pub fn load_access_token() -> anyhow::Result<Option<String>> {
    credentials::load_account_secret(ACCESS_TOKEN_TARGET)
}

pub fn store_refresh_token(token: &str) -> anyhow::Result<()> {
    credentials::set_account_secret(REFRESH_TOKEN_TARGET, token)
}

pub fn load_refresh_token() -> anyhow::Result<Option<String>> {
    credentials::load_account_secret(REFRESH_TOKEN_TARGET)
}

pub fn clear_account_secrets() -> anyhow::Result<()> {
    credentials::clear_account_secret(DEVICE_KEY_TARGET)?;
    credentials::clear_account_secret(ACCESS_TOKEN_TARGET)?;
    credentials::clear_account_secret(REFRESH_TOKEN_TARGET)?;
    Ok(())
}

pub fn access_last4(token: &str) -> String {
    token.chars().rev().take(4).collect::<String>().chars().rev().collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyOtpResponse {
    ok: bool,
    user_id: String,
    email: String,
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    registration_ticket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterResponse {
    ok: bool,
    install_id: String,
    session: SessionPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPayload {
    user_id: String,
    email: String,
    install_id: String,
    token_public_id: String,
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshResponse {
    ok: bool,
    session: SessionPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiError {
    error: String,
    code: String,
}

/// HTTP client for the managed Worker. Uses the existing reqwest stack.
pub struct AccountClient {
    pub base_url: String,
    client: reqwest::Client,
}

impl AccountClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn request_otp(&self, email: &str) -> Result<(), ManagedFailure> {
        let url = format!("{}/v1/auth/request-otp", self.base_url);
        let res = self
            .client
            .post(url)
            .json(&serde_json::json!({ "email": email }))
            .send()
            .await
            .map_err(|e| ManagedFailure::Network(e.to_string()))?;
        if res.status().is_success() {
            Ok(())
        } else {
            Err(Self::map_error(res).await)
        }
    }

    pub async fn verify_otp(&self, email: &str, code: &str) -> Result<(String, String, Option<String>), ManagedFailure> {
        let url = format!("{}/v1/auth/verify-otp", self.base_url);
        let res = self
            .client
            .post(url)
            .json(&serde_json::json!({ "email": email, "code": code }))
            .send()
            .await
            .map_err(|e| ManagedFailure::Network(e.to_string()))?;
        if !res.status().is_success() {
            return Err(Self::map_error(res).await);
        }
        let body: VerifyOtpResponse = res
            .json()
            .await
            .map_err(|e| ManagedFailure::Other(e.to_string()))?;
        Ok((body.user_id, body.email, body.registration_ticket))
    }

    /// Register or silently re-register this PC. Signs with the device key
    /// (the key never leaves the PC). `registration_ticket` comes from a fresh
    /// OTP sign-in and is required by production for a brand-new device.
    pub async fn register_install(
        &self,
        user_id: &str,
        device_name: &str,
        app_version: &str,
        private_key_hex: &str,
        registration_ticket: Option<&str>,
    ) -> Result<SessionPayload, ManagedFailure> {
        let public = public_from_private(private_key_hex);
        if public.public_key.is_empty() {
            return Err(ManagedFailure::Other("device_key_invalid".into()));
        }
        let signed_at_ms = now_ms();
        let signature = sign_registration(private_key_hex, user_id, &public.public_key, signed_at_ms)
            .ok_or_else(|| ManagedFailure::Other("device_key_invalid".into()))?;
        let mut body = serde_json::json!({
            "userId": user_id,
            "deviceName": device_name,
            "appVersion": app_version,
            "publicKey": public.public_key,
            "signature": signature,
            "signedAtMs": signed_at_ms,
        });
        if let Some(ticket) = registration_ticket {
            body["registrationTicket"] = serde_json::Value::String(ticket.to_string());
        }
        let url = format!("{}/v1/installs/register", self.base_url);
        let res = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ManagedFailure::Network(e.to_string()))?;
        if !res.status().is_success() {
            return Err(Self::map_error(res).await);
        }
        let body: RegisterResponse = res
            .json()
            .await
            .map_err(|e| ManagedFailure::Other(e.to_string()))?;
        Ok(body.session)
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<SessionPayload, ManagedFailure> {
        let url = format!("{}/v1/auth/refresh", self.base_url);
        let res = self
            .client
            .post(url)
            .json(&serde_json::json!({ "refreshToken": refresh_token }))
            .send()
            .await
            .map_err(|e| ManagedFailure::Network(e.to_string()))?;
        if !res.status().is_success() {
            return Err(Self::map_error(res).await);
        }
        let body: RefreshResponse = res
            .json()
            .await
            .map_err(|e| ManagedFailure::Other(e.to_string()))?;
        Ok(body.session)
    }

    pub async fn status(&self, access_token: Option<&str>) -> Result<StatusResponse, ManagedFailure> {
        let url = format!("{}/v1/status", self.base_url);
        let mut req = self.client.get(url);
        if let Some(token) = access_token {
            req = req.bearer_auth(token);
        }
        let res = req
            .send()
            .await
            .map_err(|e| ManagedFailure::Network(e.to_string()))?;
        if !res.status().is_success() {
            return Err(Self::map_error(res).await);
        }
        res.json()
            .await
            .map_err(|e| ManagedFailure::Other(e.to_string()))
    }

    async fn map_error(res: reqwest::Response) -> ManagedFailure {
        let status = res.status();
        if let Ok(err) = res.json::<ApiError>().await {
            return ManagedFailure::from_code(&err.code);
        }
        ManagedFailure::Other(format!("http {status}"))
    }
}

/// Apply a fresh session to Credential Manager + metadata file.
pub fn persist_session(
    config_dir: &Path,
    session: &SessionPayload,
    public_fingerprint: &str,
    api_base_url: &str,
) -> anyhow::Result<AccountSessionMeta> {
    store_access_token(&session.access_token)?;
    store_refresh_token(&session.refresh_token)?;
    let meta = AccountSessionMeta {
        email: session.email.clone(),
        user_id: session.user_id.clone(),
        install_id: session.install_id.clone(),
        token_public_id: session.token_public_id.clone(),
        access_last4: Some(access_last4(&session.access_token)),
        public_key_fingerprint: public_fingerprint.to_string(),
        api_base_url: api_base_url.trim_end_matches('/').to_string(),
        signed_in_at_ms: now_ms(),
    };
    save_session_meta(config_dir, &meta)?;
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static DIR_LOCK: Mutex<()> = Mutex::new(());

    fn temp_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join("bolo-account-tests").join(name);
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn device_keypair_is_ed25519_with_server_fingerprint() {
        let kp = generate_device_keypair();
        assert_eq!(kp.private_key_hex.len(), 64);
        assert_eq!(kp.public.fingerprint.len(), 16);
        assert!(kp.public.public_key.starts_with("MCowBQYDK2VwAyEA"));
        assert_eq!(kp.public.fingerprint, public_fingerprint(&kp.public.public_key));
        assert!(!kp.public.public_key.contains(&kp.private_key_hex));
    }

    #[test]
    fn registration_signature_matches_the_worker_vector() {
        // RFC 8032 test 1 seed; expected values computed with Node's WebCrypto,
        // which the Worker uses to verify.
        let seed = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
        let public = public_from_private(seed);
        assert_eq!(public.public_key, "MCowBQYDK2VwAyEA11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=");
        assert_eq!(public.fingerprint, "a00579fb9f411e66");
        let sig = sign_registration(seed, "u1", &public.public_key, 1_700_000_000_000).unwrap();
        assert_eq!(sig, "8NKyw/JYIv67DYQT2ejQfiMznjmdPRlK2FlbVPknZy2hDBfR1hvKI2Mm5hs+cyECXYT1jNg1Mpqn9iNhmZ9VCg==");
        assert!(sign_registration("not-hex", "u1", "x", 1).is_none());
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn session_meta_round_trip_has_no_secrets() {
        let _g = DIR_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = temp_dir("meta");
        let meta = AccountSessionMeta {
            email: "user@example.com".into(),
            user_id: "u1".into(),
            install_id: "i1".into(),
            token_public_id: "tok_abcd".into(),
            access_last4: Some("f00d".into()),
            public_key_fingerprint: "deadbeefdeadbeef".into(),
            api_base_url: "http://127.0.0.1:8787".into(),
            signed_in_at_ms: 1,
        };
        save_session_meta(&dir, &meta).unwrap();
        let loaded = load_session_meta(&dir).unwrap();
        assert_eq!(loaded, meta);
        let raw = fs::read_to_string(session_path(&dir)).unwrap();
        assert!(!raw.to_ascii_lowercase().contains("access_token"));
        assert!(!raw.to_ascii_lowercase().contains("refresh"));
        assert!(!raw.contains("private"));
    }

    #[test]
    fn managed_failure_codes_map_cleanly() {
        assert_eq!(ManagedFailure::from_code("kill_switch"), ManagedFailure::KillSwitch);
        assert_eq!(ManagedFailure::from_code("pool_full"), ManagedFailure::PoolFull);
        assert_eq!(ManagedFailure::from_code("fair_use_daily"), ManagedFailure::FairUse);
        assert_eq!(ManagedFailure::from_code("reissued"), ManagedFailure::Reissued);
        assert_eq!(ManagedFailure::from_code("refresh_reuse"), ManagedFailure::Revoked);
    }

    #[test]
    fn access_last4_never_returns_full_token() {
        let token = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let last = access_last4(token);
        assert_eq!(last.len(), 4);
        assert_ne!(last, token);
    }

    #[test]
    fn account_credential_targets_are_namespaced() {
        assert!(DEVICE_KEY_TARGET.starts_with("com.bolo.dictation/account/"));
        assert!(ACCESS_TOKEN_TARGET.starts_with("com.bolo.dictation/account/"));
        assert!(REFRESH_TOKEN_TARGET.starts_with("com.bolo.dictation/account/"));
        assert_eq!(
            credentials::service_for_target(DEVICE_KEY_TARGET),
            Some("com.bolo.dictation")
        );
    }
}
