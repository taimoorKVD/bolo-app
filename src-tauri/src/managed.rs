//! Managed Bolo access for live dictation (phase 4).
//!
//! Order of engines once an account is configured: Bolo Worker, then the
//! user's own Groq key (BYOK), then on-device. Without an account this module
//! is inert and the legacy single-key path runs unchanged.
//!
//! Never logs audio, transcripts, or tokens; only short failure classes.

use crate::account::{self, AccountClient, ManagedFailure, StatusResponse};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Status checks run at startup and then about every 15 minutes.
pub const STATUS_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// How long a busy/paused/fair-use answer keeps live takes on the backup path.
const COOLDOWN_MS: u64 = 15 * 60 * 1000;

static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();
static STATE: Mutex<Snapshot> = Mutex::new(Snapshot::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedState {
    /// No account: legacy path only.
    NotConfigured,
    Connected,
    /// Admin reissued this device; reconnecting silently.
    Reconnecting,
    /// Revoked or suspended: the user must sign in again.
    SignInRequired,
    /// Owner kill switch is on.
    Paused,
    FairUse,
    PoolFull,
    Offline,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub state: ManagedState,
    /// Plain message for the overlay/status line, never contains secrets.
    pub message: Option<String>,
    #[serde(skip)]
    pub blocked_until_ms: u64,
}

impl Snapshot {
    const fn new() -> Self {
        Self { state: ManagedState::NotConfigured, message: None, blocked_until_ms: 0 }
    }
}

/// What to do after a managed call fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// Access token expired: rotate with the refresh token, retry once.
    Refresh,
    /// Admin reissued: re-register with the device key, retry once.
    Reregister,
    /// Revoked: stop using managed until the user signs in again.
    SignIn,
    /// Kill switch, fair use, or pool full: use the backup for a while.
    Cooldown(ManagedState),
    /// Network or unknown: use the backup for this take only.
    Fallback,
}

pub fn plan_after(failure: &ManagedFailure) -> Recovery {
    match failure {
        ManagedFailure::Auth => Recovery::Refresh,
        ManagedFailure::Reissued => Recovery::Reregister,
        ManagedFailure::Revoked => Recovery::SignIn,
        ManagedFailure::KillSwitch => Recovery::Cooldown(ManagedState::Paused),
        ManagedFailure::FairUse => Recovery::Cooldown(ManagedState::FairUse),
        ManagedFailure::PoolFull => Recovery::Cooldown(ManagedState::PoolFull),
        ManagedFailure::Other(code) if code == "suspended" => Recovery::SignIn,
        ManagedFailure::Network(_) | ManagedFailure::Other(_) => Recovery::Fallback,
    }
}

/// Map a status response onto a display state.
pub fn state_from_status(status: &StatusResponse) -> ManagedState {
    if status.sign_in_required.unwrap_or(false) {
        return ManagedState::SignInRequired;
    }
    if status.code.as_deref() == Some("reissued") || status.reconnect_required.unwrap_or(false) {
        return ManagedState::Reconnecting;
    }
    if status.kill_switch {
        return ManagedState::Paused;
    }
    if status.pool_full.unwrap_or(false) {
        return ManagedState::PoolFull;
    }
    let daily = status.remaining_daily_minutes.unwrap_or(f64::MAX);
    let monthly = status.remaining_monthly_minutes.unwrap_or(f64::MAX);
    if status.authenticated && (daily <= 0.0 || monthly <= 0.0) {
        return ManagedState::FairUse;
    }
    ManagedState::Connected
}

pub fn message_for(state: ManagedState) -> Option<String> {
    let text = match state {
        ManagedState::NotConfigured | ManagedState::Connected => return None,
        ManagedState::Reconnecting => "Reconnecting to Bolo…",
        ManagedState::SignInRequired => "Sign in to Bolo again to keep using free dictation.",
        ManagedState::Paused => "Bolo's free dictation is paused. Using your backup.",
        ManagedState::FairUse => "You've reached today's free minutes. Using your backup.",
        ManagedState::PoolFull => "Bolo is busy right now. Using your backup.",
        ManagedState::Offline => "Can't reach Bolo. Using your backup.",
    };
    Some(text.to_string())
}

/// Audio minutes from sample counts (interleaved samples / channels / rate).
pub fn audio_minutes(samples: usize, channels: u16, sample_rate: u32) -> f64 {
    if channels == 0 || sample_rate == 0 {
        return 0.0;
    }
    (samples as f64 / channels as f64 / sample_rate as f64 / 60.0).max(0.01)
}

pub fn init(config_dir: &Path) {
    let _ = CONFIG_DIR.set(config_dir.to_path_buf());
}

fn config_dir() -> Option<&'static PathBuf> {
    CONFIG_DIR.get()
}

/// True only when a Bolo account session exists on this device.
pub fn is_configured() -> bool {
    config_dir().map(|dir| account::load_session_meta(dir).is_some()).unwrap_or(false)
}

pub fn snapshot() -> Snapshot {
    STATE.lock().map(|s| s.clone()).unwrap_or_else(|_| Snapshot::new())
}

/// Returns true when the state changed (so callers emit once).
fn set_state(state: ManagedState, blocked_until_ms: u64) -> bool {
    let Ok(mut guard) = STATE.lock() else { return false };
    let changed = guard.state != state;
    guard.state = state;
    guard.message = message_for(state);
    guard.blocked_until_ms = blocked_until_ms;
    changed
}

fn usable_now() -> bool {
    let snap = snapshot();
    if snap.state == ManagedState::SignInRequired {
        return false;
    }
    snap.blocked_until_ms <= account::now_ms()
}

fn client() -> Option<(AccountClient, PathBuf)> {
    let dir = config_dir()?.clone();
    account::load_session_meta(&dir)?;
    Some((AccountClient::new(account::api_base_url(&dir)), dir))
}

async fn refresh_session(client: &AccountClient, dir: &Path) -> Result<(), ManagedFailure> {
    let meta = account::load_session_meta(dir).ok_or(ManagedFailure::Revoked)?;
    let refresh = account::load_refresh_token()
        .map_err(|_| ManagedFailure::Other("credential_read".into()))?
        .ok_or(ManagedFailure::Revoked)?;
    let session = client.refresh(&refresh).await?;
    account::persist_session(dir, &session, &meta.public_key_fingerprint, &client.base_url)
        .map_err(|_| ManagedFailure::Other("credential_write".into()))?;
    Ok(())
}

async fn reregister(client: &AccountClient, dir: &Path) -> Result<(), ManagedFailure> {
    let meta = account::load_session_meta(dir).ok_or(ManagedFailure::Revoked)?;
    let private = account::load_device_private_key()
        .map_err(|_| ManagedFailure::Other("credential_read".into()))?
        .ok_or(ManagedFailure::Revoked)?;
    let public = account::public_from_private(&private);
    let device = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".into());
    let session = client
        .register_install(&meta.user_id, &device, env!("CARGO_PKG_VERSION"), &private, None)
        .await?;
    account::persist_session(dir, &session, &public.fingerprint, &client.base_url)
        .map_err(|_| ManagedFailure::Other("credential_write".into()))?;
    Ok(())
}

async fn recover(client: &AccountClient, dir: &Path, failure: &ManagedFailure) -> bool {
    match plan_after(failure) {
        Recovery::Refresh => match refresh_session(client, dir).await {
            Ok(()) => true,
            Err(ManagedFailure::Reissued) => reregister(client, dir).await.is_ok(),
            Err(other) => {
                apply_failure(&other);
                false
            }
        },
        Recovery::Reregister => {
            set_state(ManagedState::Reconnecting, 0);
            match reregister(client, dir).await {
                Ok(()) => true,
                Err(other) => {
                    apply_failure(&other);
                    false
                }
            }
        }
        _ => {
            apply_failure(failure);
            false
        }
    }
}

fn apply_failure(failure: &ManagedFailure) {
    match plan_after(failure) {
        Recovery::SignIn => {
            set_state(ManagedState::SignInRequired, 0);
        }
        Recovery::Cooldown(state) => {
            set_state(state, account::now_ms() + COOLDOWN_MS);
        }
        Recovery::Fallback if matches!(failure, ManagedFailure::Network(_)) => {
            set_state(ManagedState::Offline, 0);
        }
        _ => {}
    }
}

enum Op<'a> {
    Whisper { audio_b64: &'a str, minutes: f64, task: &'a str, language: Option<&'a str>, prompt: Option<&'a str> },
    Cleanup { text: &'a str, tone: &'a str, mode: &'a str },
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Reply {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    avg_logprob: Option<f32>,
    #[serde(default)]
    no_speech_prob: Option<f32>,
}

#[derive(serde::Deserialize)]
struct ErrReply {
    code: String,
}

fn whisper_body(audio_b64: &str, minutes: f64, task: &str, language: Option<&str>, prompt: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::json!({ "audioBase64": audio_b64, "audioMinutes": minutes, "task": task });
    if let Some(language) = language {
        body["language"] = serde_json::Value::String(language.to_string());
    }
    if let Some(prompt) = prompt.filter(|p| !p.trim().is_empty()) {
        body["prompt"] = serde_json::Value::String(prompt.to_string());
    }
    body
}

async fn call(base: &str, op: &Op<'_>) -> Result<Reply, ManagedFailure> {
    let token = account::load_access_token()
        .map_err(|_| ManagedFailure::Other("credential_read".into()))?
        .ok_or(ManagedFailure::Auth)?;
    let http = crate::transcribe::http_client();
    let req = match op {
        Op::Whisper { audio_b64, minutes, task, language, prompt } => http
            .post(format!("{base}/v1/dictation/transcribe"))
            .header("x-bolo-audio-minutes", format!("{minutes:.3}"))
            .json(&whisper_body(audio_b64, *minutes, task, *language, *prompt)),
        Op::Cleanup { text, tone, mode } => http
            .post(format!("{base}/v1/dictation/cleanup"))
            .json(&serde_json::json!({ "text": text, "tone": tone, "mode": mode })),
    };
    let res = req
        .bearer_auth(token)
        .timeout(Duration::from_secs(45))
        .send()
        .await
        .map_err(|e| ManagedFailure::Network(if e.is_timeout() { "timeout".into() } else { "connect".into() }))?;
    let status = res.status();
    if status.is_success() {
        return res.json::<Reply>().await.map_err(|_| ManagedFailure::Other("bad_reply".into()));
    }
    match res.json::<ErrReply>().await {
        Ok(err) => Err(ManagedFailure::from_code(&err.code)),
        Err(_) => Err(ManagedFailure::Other(format!("http {}", status.as_u16()))),
    }
}

async fn run(op: Op<'_>) -> Option<Reply> {
    if !usable_now() {
        return None;
    }
    let (client, dir) = client()?;
    match call(&client.base_url, &op).await {
        Ok(reply) => {
            set_state(ManagedState::Connected, 0);
            Some(reply)
        }
        Err(failure) => {
            log_class(&dir, &failure);
            if recover(&client, &dir, &failure).await {
                match call(&client.base_url, &op).await {
                    Ok(reply) => {
                        set_state(ManagedState::Connected, 0);
                        return Some(reply);
                    }
                    Err(second) => {
                        log_class(&dir, &second);
                        apply_failure(&second);
                    }
                }
            }
            None
        }
    }
}

fn log_class(dir: &Path, failure: &ManagedFailure) {
    let class = match failure {
        ManagedFailure::Network(kind) => format!("network:{kind}"),
        ManagedFailure::Other(code) => format!("other:{}", code.chars().take(32).collect::<String>()),
        other => format!("{other:?}").to_lowercase(),
    };
    crate::logbuf::log_line(dir, "info", &format!("managed fallback class={class}"));
}

/// Pick between a prompted and an unprompted attempt, like the Groq path:
/// prefer non-empty, non-stock text, then the more confident one.
fn prefer_second(first: &Reply, second: &Reply) -> bool {
    use crate::transcribe::stock_phrase;
    if second.text.trim().is_empty() {
        return false;
    }
    if first.text.trim().is_empty() || (stock_phrase(&first.text) && !stock_phrase(&second.text)) {
        return true;
    }
    second.avg_logprob.unwrap_or(-9.0) > first.avg_logprob.unwrap_or(-9.0) + 0.2
}

fn to_transcript(reply: Reply, tries: u8, fallback_language: Option<String>) -> crate::transcribe::Transcript {
    let doubtful = crate::transcribe::doubtful(&reply.text, reply.avg_logprob, reply.no_speech_prob);
    crate::transcribe::Transcript {
        text: reply.text.trim().to_string(),
        tries,
        language: reply.language.filter(|l| !l.is_empty()).or(fallback_language).unwrap_or_default(),
        doubtful,
    }
}

/// Managed Whisper with the same language, prompt and retry rules as the Groq
/// path. `None` means use the user's own key or on-device instead.
pub async fn whisper(bytes: &[u8], minutes: f64, task: &str, language: &str, prompt: Option<&str>) -> Option<crate::transcribe::Transcript> {
    let audio_b64 = account::base64_encode(bytes);
    let language = crate::transcribe::normalize_language(language);
    let first = run(Op::Whisper { audio_b64: &audio_b64, minutes, task, language: language.as_deref(), prompt }).await?;
    let unsure = crate::transcribe::should_retry(&first.text, first.avg_logprob, first.no_speech_prob)
        || crate::transcribe::stock_phrase(&first.text);
    if unsure && prompt.is_some() {
        if let Some(second) = run(Op::Whisper { audio_b64: &audio_b64, minutes, task, language: language.as_deref(), prompt: None }).await {
            let pick = if prefer_second(&first, &second) { second } else { first };
            return Some(to_transcript(pick, 2, language));
        }
    }
    Some(to_transcript(first, 1, language))
}

/// Managed cleanup, held to the same word-preservation check as Groq cleanup.
pub async fn cleanup(text: &str, tone: &str) -> Option<String> {
    let reply = run(Op::Cleanup { text, tone, mode: "cleanup" }).await?;
    let out = reply.text.trim().to_string();
    (!out.is_empty() && crate::cleanup::accept_cleanup(text, &out)).then_some(out)
}

/// Managed Urdu -> English text translation for the Mixed route.
pub async fn translate_text(text: &str) -> crate::cleanup::CleanOutcome {
    let original = text.trim();
    match run(Op::Cleanup { text: original, tone: "plain", mode: "translate" }).await {
        Some(reply) if crate::cleanup::accept_translation(original, &reply.text) => crate::cleanup::CleanOutcome {
            text: reply.text.trim().to_string(),
            used_model: true,
            note: None,
            why: "managed".into(),
        },
        _ => crate::cleanup::CleanOutcome { text: original.to_string(), used_model: false, note: None, why: "managed_unavailable".into() },
    }
}

/// One status check; refreshes or re-registers silently when needed.
pub async fn check_status() -> ManagedState {
    let Some((client, dir)) = client() else {
        set_state(ManagedState::NotConfigured, 0);
        return ManagedState::NotConfigured;
    };
    for attempt in 0..2 {
        let token = account::load_access_token().ok().flatten();
        let status = match client.status(token.as_deref()).await {
            Ok(status) => status,
            Err(failure) => {
                apply_failure(&failure);
                return snapshot().state;
            }
        };
        if attempt == 0 {
            let failure = match status.code.as_deref() {
                Some("reissued") => Some(ManagedFailure::Reissued),
                Some("token_expired") | Some("invalid_token") => Some(ManagedFailure::Auth),
                _ => None,
            };
            if let Some(failure) = failure {
                if recover(&client, &dir, &failure).await {
                    continue;
                }
                return snapshot().state;
            }
        }
        let state = state_from_status(&status);
        let blocked = match state {
            ManagedState::Paused | ManagedState::PoolFull | ManagedState::FairUse => account::now_ms() + COOLDOWN_MS,
            _ => 0,
        };
        set_state(state, blocked);
        return state;
    }
    snapshot().state
}

/// Status loop: once at startup, then about every 15 minutes. Emits
/// `managed-status` only when the state changes; the overlay/status line is
/// the only UI.
pub fn spawn(app: tauri::AppHandle) {
    use tauri::Emitter;
    std::thread::spawn(move || {
        let mut last: Option<ManagedState> = None;
        loop {
            let state = tauri::async_runtime::block_on(check_status());
            if last != Some(state) {
                let _ = app.emit("managed-status", snapshot());
                last = Some(state);
            }
            std::thread::sleep(STATUS_INTERVAL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> StatusResponse {
        StatusResponse {
            kill_switch: false,
            open_router_enabled: false,
            paid_usage_enabled: false,
            authenticated: true,
            code: None,
            remaining_daily_minutes: Some(10.0),
            remaining_monthly_minutes: Some(100.0),
            reconnect_required: None,
            pool_full: None,
            sign_in_required: None,
            notices: Vec::new(),
        }
    }

    #[test]
    fn recovery_plan_matches_failures() {
        assert_eq!(plan_after(&ManagedFailure::Auth), Recovery::Refresh);
        assert_eq!(plan_after(&ManagedFailure::Reissued), Recovery::Reregister);
        assert_eq!(plan_after(&ManagedFailure::Revoked), Recovery::SignIn);
        assert_eq!(plan_after(&ManagedFailure::PoolFull), Recovery::Cooldown(ManagedState::PoolFull));
        assert_eq!(plan_after(&ManagedFailure::FairUse), Recovery::Cooldown(ManagedState::FairUse));
        assert_eq!(plan_after(&ManagedFailure::KillSwitch), Recovery::Cooldown(ManagedState::Paused));
        assert_eq!(plan_after(&ManagedFailure::Network("x".into())), Recovery::Fallback);
        assert_eq!(plan_after(&ManagedFailure::Other("suspended".into())), Recovery::SignIn);
    }

    #[test]
    fn status_flags_map_to_states() {
        assert_eq!(state_from_status(&status()), ManagedState::Connected);
        let mut s = status();
        s.pool_full = Some(true);
        assert_eq!(state_from_status(&s), ManagedState::PoolFull);
        let mut s = status();
        s.kill_switch = true;
        assert_eq!(state_from_status(&s), ManagedState::Paused);
        let mut s = status();
        s.remaining_daily_minutes = Some(0.0);
        assert_eq!(state_from_status(&s), ManagedState::FairUse);
        let mut s = status();
        s.code = Some("reissued".into());
        assert_eq!(state_from_status(&s), ManagedState::Reconnecting);
        let mut s = status();
        s.sign_in_required = Some(true);
        s.kill_switch = true;
        assert_eq!(state_from_status(&s), ManagedState::SignInRequired);
    }

    #[test]
    fn messages_only_for_degraded_states() {
        assert!(message_for(ManagedState::Connected).is_none());
        assert!(message_for(ManagedState::NotConfigured).is_none());
        assert!(message_for(ManagedState::PoolFull).unwrap().contains("backup"));
    }

    #[test]
    fn minutes_from_samples() {
        let m = audio_minutes(16_000 * 60, 1, 16_000);
        assert!((m - 1.0).abs() < 1e-9);
        assert_eq!(audio_minutes(10, 0, 16_000), 0.0);
        assert!(audio_minutes(1, 1, 16_000) >= 0.01);
    }

    #[test]
    fn not_configured_without_init() {
        // CONFIG_DIR is never set in unit tests, so the legacy path stays on.
        assert!(!is_configured());
    }
}
