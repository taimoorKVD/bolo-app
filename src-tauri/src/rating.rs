//! In-app Microsoft Store rating prompt.
//!
//! The card only offers the Store's own rating dialog (or the Store review
//! page on the direct-download build). It never asks whether the user is
//! happy first, and it never offers a reward.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DAY_MS: u64 = 86_400_000;
pub const MIN_AGE_MS: u64 = 7 * DAY_MS;
pub const MIN_ACTIVE_DAYS: u32 = 3;
pub const MIN_SUCCESSES: u32 = 20;
pub const MAX_PROMPTS: u32 = 3;
pub const NOT_NOW_MS: u64 = 90 * DAY_MS;
pub const QUIET_MS: u64 = 10 * 60 * 1000;
pub const PRODUCT_ID: &str = "9NMW6C99BF04";
pub const REVIEW_URI: &str = "ms-windows-store://review/?ProductId=9NMW6C99BF04";
pub const REVIEW_WEB: &str = "https://apps.microsoft.com/detail/9NMW6C99BF04?cid=inapp_rate";
pub const FEEDBACK_ADDRESS: &str = "taimoor@kingdom-vision.co.uk";

const SESSION_FILE: &str = "rating-session";

/// True for the Microsoft Store (MSIX) build, which is compiled without the
/// in-app updater. The direct-download exe keeps the updater feature.
pub fn is_store_build() -> bool {
    !cfg!(feature = "updater")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct RatingState {
    pub first_launch_unix_ms: u64,
    pub active_days: u32,
    pub last_active_day: String,
    pub successful_dictations: u32,
    /// Newest first. `"ok"`, `"error"`, or `"fixed"`. At most three.
    pub recent_outcomes: Vec<String>,
    pub last_not_now_unix_ms: u64,
    pub prompt_count: u32,
    pub prompted_versions: Vec<String>,
    /// Version whose card is on the Home page and has not been answered.
    pub open_prompt_version: String,
    pub rated: bool,
    pub last_error_unix_ms: u64,
    pub last_update_unix_ms: u64,
    pub last_crash_unix_ms: u64,
    pub last_network_unix_ms: u64,
    pub last_seen_version: String,
}

impl Default for RatingState {
    fn default() -> Self {
        Self {
            first_launch_unix_ms: 0,
            active_days: 0,
            last_active_day: String::new(),
            successful_dictations: 0,
            recent_outcomes: Vec::new(),
            last_not_now_unix_ms: 0,
            prompt_count: 0,
            prompted_versions: Vec::new(),
            open_prompt_version: String::new(),
            rated: false,
            last_error_unix_ms: 0,
            last_update_unix_ms: 0,
            last_crash_unix_ms: 0,
            last_network_unix_ms: 0,
            last_seen_version: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatingLanguage {
    English,
    RomanUrdu,
    Urdu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingCopy {
    pub title: &'static str,
    pub body: &'static str,
    pub rate: &'static str,
    pub later: &'static str,
    pub feedback: &'static str,
    pub thanks: &'static str,
}

pub fn language_for(app_language: &str, output: &str) -> RatingLanguage {
    let lang = app_language.trim().to_ascii_lowercase();
    let keep_urdu = output.trim().eq_ignore_ascii_case("same");
    match lang.as_str() {
        "ur" | "urdu" if keep_urdu => RatingLanguage::Urdu,
        "ur" | "urdu" | "roman-urdu" | "ur-latn" => RatingLanguage::RomanUrdu,
        _ => RatingLanguage::English,
    }
}

pub fn copy_for(language: RatingLanguage) -> RatingCopy {
    match language {
        RatingLanguage::English => RatingCopy {
            title: "Enjoying Bolo?",
            body: "If Bolo saves you some typing, a quick rating on Microsoft Store helps other people find it. It takes about ten seconds.",
            rate: "Rate Bolo",
            later: "Not now",
            feedback: "Send private feedback to the developer",
            thanks: "Shukriya! Thank you for rating Bolo.",
        },
        RatingLanguage::RomanUrdu => RatingCopy {
            title: "Bolo pasand aa raha hai?",
            body: "Agar Bolo se aap ki typing bachti hai, to Microsoft Store par ek rating dein.\nIs se doosre log bhi Bolo dhoond paate hain. Sirf das second lagte hain.",
            rate: "Rating dein",
            later: "Abhi nahi",
            feedback: "Developer ko private feedback bhejein",
            thanks: "Shukriya! Rating ke liye bohat meherbani.",
        },
        RatingLanguage::Urdu => RatingCopy {
            title: "کیا بولو آپ کو پسند آ رہا ہے؟",
            body: "اگر بولو سے آپ کی ٹائپنگ بچتی ہے تو مائیکروسافٹ اسٹور پر ایک ریٹنگ دیں۔ اس سے دوسرے لوگ بھی بولو تک پہنچ پاتے ہیں۔ صرف دس سیکنڈ لگتے ہیں۔",
            rate: "ریٹنگ دیں",
            later: "ابھی نہیں",
            feedback: "ڈویلپر کو نجی رائے بھیجیں",
            thanks: "شکریہ! ریٹنگ کے لیے بہت مہربانی۔",
        },
    }
}

pub fn feedback_mailto(version: &str) -> String {
    let subject = format!("Bolo feedback (version {version})").replace(' ', "%20");
    format!("mailto:{FEEDBACK_ADDRESS}?subject={subject}")
}

/// What the Store dialog (or the review-page open) came back as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(all(windows, not(feature = "updater"))), allow(dead_code))]
pub enum ReviewResult {
    /// They rated, or updated an existing rating. Never ask again.
    Rated,
    /// Not now, or the Store dialog was canceled. Wait 90 days.
    AskLater,
    /// The review page opened. This version is used up; we don't know the stars.
    PageOpened,
    /// Don't count the ask. Try again on a later day.
    Network,
    /// Logged locally. Wait 90 days.
    Other,
}

pub fn should_show(now_ms: u64, version: &str, state: &RatingState, dictating: bool, overlay_visible: bool) -> bool {
    if state.rated || dictating || overlay_visible {
        return false;
    }
    if state.open_prompt_version == version && !state.open_prompt_version.is_empty() {
        return true;
    }
    can_start(now_ms, version, state)
}

fn can_start(now_ms: u64, version: &str, state: &RatingState) -> bool {
    if state.rated || version.is_empty() {
        return false;
    }
    if state.prompt_count >= MAX_PROMPTS {
        return false;
    }
    if state.prompted_versions.iter().any(|asked| asked == version) {
        return false;
    }
    if state.first_launch_unix_ms == 0 || now_ms.saturating_sub(state.first_launch_unix_ms) < MIN_AGE_MS {
        return false;
    }
    if state.active_days < MIN_ACTIVE_DAYS || state.successful_dictations < MIN_SUCCESSES {
        return false;
    }
    if state.recent_outcomes.len() < 3 || state.recent_outcomes.iter().take(3).any(|outcome| outcome != "ok") {
        return false;
    }
    if state.last_not_now_unix_ms > 0 && now_ms.saturating_sub(state.last_not_now_unix_ms) < NOT_NOW_MS {
        return false;
    }
    if within(now_ms, state.last_error_unix_ms) || within(now_ms, state.last_update_unix_ms) || within(now_ms, state.last_crash_unix_ms) {
        return false;
    }
    if state.last_network_unix_ms > 0 && day_key(state.last_network_unix_ms) == day_key(now_ms) {
        return false;
    }
    true
}

fn within(now_ms: u64, event_ms: u64) -> bool {
    event_ms > 0 && now_ms.saturating_sub(event_ms) < QUIET_MS
}

/// Record that the Home card is on screen. Counts once per version.
pub fn begin_prompt(state: &mut RatingState, version: &str) {
    if version.is_empty() || state.open_prompt_version == version {
        return;
    }
    state.open_prompt_version = version.to_string();
    if !state.prompted_versions.iter().any(|asked| asked == version) {
        state.prompted_versions.push(version.to_string());
        state.prompt_count = state.prompt_count.saturating_add(1);
    }
}

pub fn apply_review(state: &mut RatingState, version: &str, result: ReviewResult, now_ms: u64) {
    match result {
        ReviewResult::Rated => {
            state.rated = true;
            state.open_prompt_version.clear();
        }
        ReviewResult::AskLater | ReviewResult::Other => {
            state.last_not_now_unix_ms = now_ms;
            state.open_prompt_version.clear();
        }
        ReviewResult::PageOpened => {
            state.open_prompt_version.clear();
        }
        ReviewResult::Network => {
            if state.prompted_versions.iter().any(|asked| asked == version) && state.prompt_count > 0 {
                state.prompt_count -= 1;
            }
            state.prompted_versions.retain(|asked| asked != version);
            state.open_prompt_version.clear();
            state.last_network_unix_ms = now_ms;
        }
    }
}

pub fn note_startup(state: &mut RatingState, version: &str, now_ms: u64, oldest_history_ms: Option<u64>, crashed: bool) {
    if state.first_launch_unix_ms == 0 {
        state.first_launch_unix_ms = oldest_history_ms.filter(|stamp| *stamp > 0 && *stamp <= now_ms).unwrap_or(now_ms);
    }
    note_activity(state, now_ms);
    if crashed {
        state.last_crash_unix_ms = now_ms;
    }
    if !state.last_seen_version.is_empty() && state.last_seen_version != version {
        state.last_update_unix_ms = now_ms;
    }
    state.last_seen_version = version.to_string();
}

pub fn note_activity(state: &mut RatingState, now_ms: u64) {
    let day = day_key(now_ms);
    if state.last_active_day != day {
        state.active_days = state.active_days.saturating_add(1);
        state.last_active_day = day;
    }
}

pub fn note_success(state: &mut RatingState, now_ms: u64) {
    note_activity(state, now_ms);
    state.successful_dictations = state.successful_dictations.saturating_add(1);
    push_outcome(state, "ok");
}

pub fn note_error(state: &mut RatingState, now_ms: u64) {
    state.last_error_unix_ms = now_ms;
    push_outcome(state, "error");
}

pub fn note_fixed(state: &mut RatingState) {
    if state.recent_outcomes.first().map(String::as_str) == Some("ok") {
        state.recent_outcomes[0] = "fixed".into();
    } else {
        push_outcome(state, "fixed");
    }
}

fn push_outcome(state: &mut RatingState, outcome: &str) {
    state.recent_outcomes.insert(0, outcome.to_string());
    state.recent_outcomes.truncate(3);
}

pub fn day_key(unix_ms: u64) -> String {
    let (year, month, day) = civil_from_days((unix_ms / DAY_MS) as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's civil_from_days. `days` is days since 1970-01-01 UTC.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

pub fn session_was_unclean(dir: &Path) -> bool {
    dir.join(SESSION_FILE).is_file()
}

pub fn session_mark_running(dir: &Path) {
    let _ = std::fs::write(dir.join(SESSION_FILE), b"running");
}

pub fn session_mark_clean(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(SESSION_FILE));
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingCard {
    pub title: String,
    pub body: String,
    pub rate_label: String,
    pub later_label: String,
    pub feedback_label: String,
    pub rtl: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateResponse {
    /// `rated`, `later`, `opened`, `retry`, or `failed`.
    pub status: String,
    pub message: String,
}

fn card_for(language: RatingLanguage) -> RatingCard {
    let copy = copy_for(language);
    RatingCard {
        title: copy.title.into(),
        body: copy.body.into(),
        rate_label: copy.rate.into(),
        later_label: copy.later.into(),
        feedback_label: copy.feedback.into(),
        rtl: language == RatingLanguage::Urdu,
    }
}

fn with_rating(app: &tauri::AppHandle, update: impl FnOnce(&mut RatingState)) {
    use tauri::Manager;
    let state = app.state::<crate::dictation::AppState>();
    let settings = {
        let mut settings = state.settings.lock().unwrap_or_else(|err| err.into_inner());
        update(&mut settings.rating);
        settings.clone()
    };
    if let Err(err) = crate::store::save_settings(&state.config_dir, &settings) {
        crate::logbuf::log_line(&state.config_dir, "error", &format!("rating: {err}"));
    }
}

fn current_copy(app: &tauri::AppHandle) -> RatingCopy {
    use tauri::Manager;
    let state = app.state::<crate::dictation::AppState>();
    let settings = state.settings.lock().unwrap_or_else(|err| err.into_inner());
    copy_for(language_for(&settings.language, &settings.output))
}

#[tauri::command]
pub fn rating_card(app: tauri::AppHandle) -> Option<RatingCard> {
    use tauri::Manager;
    let version = env!("CARGO_PKG_VERSION");
    let state = app.state::<crate::dictation::AppState>();
    let dictating = !crate::dictation::is_idle(&state);
    let overlay_visible = app
        .get_webview_window("overlay")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    let settings = state.settings.lock().unwrap_or_else(|err| err.into_inner());
    let language = language_for(&settings.language, &settings.output);
    let now = crate::store::now_ms();
    if !should_show(now, version, &settings.rating, dictating, overlay_visible) {
        return None;
    }
    let needs_begin = settings.rating.open_prompt_version != version;
    drop(settings);
    if needs_begin {
        with_rating(&app, |rating| begin_prompt(rating, version));
    }
    Some(card_for(language))
}

#[tauri::command]
pub fn rating_not_now(app: tauri::AppHandle) {
    let version = env!("CARGO_PKG_VERSION");
    let now = crate::store::now_ms();
    with_rating(&app, |rating| apply_review(rating, version, ReviewResult::AskLater, now));
}

#[tauri::command]
pub fn rating_feedback(app: tauri::AppHandle) -> Result<(), String> {
    let link = feedback_mailto(env!("CARGO_PKG_VERSION"));
    open::that(link).map_err(|err| err.to_string())?;
    let _ = app;
    Ok(())
}

#[tauri::command]
pub fn rate_bolo(app: tauri::AppHandle) -> Result<RateResponse, String> {
    let version = env!("CARGO_PKG_VERSION");
    let now = crate::store::now_ms();
    let result = if is_store_build() {
        request_store_review(&app)
    } else {
        match open_review_page() {
            Ok(()) => Ok(ReviewResult::PageOpened),
            Err(err) => {
                use tauri::Manager;
                let state = app.state::<crate::dictation::AppState>();
                crate::logbuf::log_line(&state.config_dir, "warn", &format!("rating page: {err}"));
                Ok(ReviewResult::Other)
            }
        }
    }?;
    with_rating(&app, |rating| apply_review(rating, version, result, now));
    let copy = current_copy(&app);
    let (status, message) = match result {
        ReviewResult::Rated => ("rated", copy.thanks),
        ReviewResult::AskLater => ("later", ""),
        ReviewResult::PageOpened => ("opened", ""),
        ReviewResult::Network => ("retry", ""),
        ReviewResult::Other => ("failed", ""),
    };
    Ok(RateResponse { status: status.into(), message: message.into() })
}

fn open_review_page() -> Result<(), String> {
    let uri = format!("ms-windows-store://review/?ProductId={PRODUCT_ID}");
    let web = format!("https://apps.microsoft.com/detail/{PRODUCT_ID}?cid=inapp_rate");
    debug_assert_eq!(uri, REVIEW_URI);
    debug_assert_eq!(web, REVIEW_WEB);
    if open::that(uri).is_ok() {
        return Ok(());
    }
    open::that(web).map_err(|err| err.to_string())
}

#[cfg(all(windows, not(feature = "updater")))]
fn request_store_review(app: &tauri::AppHandle) -> Result<ReviewResult, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let app = app.clone();
    app.run_on_main_thread(move || {
        if let Err(err) = start_store_dialog(&app, tx.clone()) {
            let _ = tx.send(Err(err));
        }
    })
    .map_err(|err| err.to_string())?;
    rx.recv_timeout(std::time::Duration::from_secs(180))
        .map_err(|_| "The rating dialog didn't finish.".to_string())?
}

#[cfg(all(windows, not(feature = "updater")))]
fn start_store_dialog(app: &tauri::AppHandle, tx: std::sync::mpsc::Sender<Result<ReviewResult, String>>) -> Result<(), String> {
    use tauri::Manager;
    use windows::Services::Store::StoreContext;
    use windows::Win32::UI::Shell::IInitializeWithWindow;
    let window = app.get_webview_window("settings").ok_or_else(|| "Bolo's window isn't open.".to_string())?;
    // Tauri's window handle is the windows 0.62 HWND. This crate uses 0.61, so
    // rebuild the handle from the raw pointer. Both are `HWND(pub *mut c_void)`.
    let foreign = window.hwnd().map_err(|err| err.to_string())?;
    let hwnd = windows::Win32::Foundation::HWND(foreign.0);
    let context = StoreContext::GetDefault().map_err(|err| err.to_string())?;
    let init: IInitializeWithWindow = context.cast().map_err(|err| err.to_string())?;
    unsafe { init.Initialize(hwnd) }.map_err(|err| err.to_string())?;
    let operation = context.RequestRateAndReviewAppAsync().map_err(|err| err.to_string())?;
    let watched = operation.clone();
    operation
        .SetCompleted(&windows_future::AsyncOperationCompletedHandler::new(move |_info, _status| {
            let outcome = match watched.GetResults() {
                Ok(result) => Ok(map_store_result(&result)),
                Err(err) => Err(err.to_string()),
            };
            let _ = tx.send(outcome);
            Ok(())
        }))
        .map_err(|err| err.to_string())?;
    Ok(())
}

#[cfg(all(windows, not(feature = "updater")))]
fn map_store_result(result: &windows::Services::Store::StoreRateAndReviewResult) -> ReviewResult {
    use windows::Services::Store::StoreRateAndReviewStatus;
    let updated = result.WasUpdated().unwrap_or(false);
    let status = result.Status().unwrap_or(StoreRateAndReviewStatus::Error);
    if status == StoreRateAndReviewStatus::Succeeded || updated {
        ReviewResult::Rated
    } else if status == StoreRateAndReviewStatus::CanceledByUser {
        ReviewResult::AskLater
    } else if status == StoreRateAndReviewStatus::NetworkError {
        ReviewResult::Network
    } else {
        if let Ok(err) = result.ExtendedError() {
            eprintln!("rating store error: {err:?}");
        }
        ReviewResult::Other
    }
}

#[cfg(not(all(windows, not(feature = "updater"))))]
fn request_store_review(_app: &tauri::AppHandle) -> Result<ReviewResult, String> {
    Err("The Store rating dialog is only in the Microsoft Store build.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(now: u64) -> RatingState {
        let mut state = RatingState::default();
        state.first_launch_unix_ms = now - MIN_AGE_MS - DAY_MS;
        state.active_days = MIN_ACTIVE_DAYS;
        state.last_active_day = "1970-01-01".into();
        state.successful_dictations = MIN_SUCCESSES;
        state.recent_outcomes = vec!["ok".into(), "ok".into(), "ok".into()];
        state
    }

    #[test]
    fn a_steady_user_is_asked_once_per_version() {
        let now = 50 * DAY_MS;
        let mut state = ready(now);
        assert!(should_show(now, "0.1.6", &state, false, false));
        assert!(!should_show(now, "0.1.6", &state, true, false), "not while dictating");
        assert!(!should_show(now, "0.1.6", &state, false, true), "not over the overlay");
        begin_prompt(&mut state, "0.1.6");
        assert_eq!(state.prompt_count, 1);
        assert!(should_show(now, "0.1.6", &state, false, false), "the open card stays up");
        begin_prompt(&mut state, "0.1.6");
        assert_eq!(state.prompt_count, 1, "showing it again does not count twice");
        apply_review(&mut state, "0.1.6", ReviewResult::AskLater, now);
        assert!(!should_show(now, "0.1.6", &state, false, false));
        assert!(!should_show(now + DAY_MS, "0.1.7", &state, false, false), "Not now waits 90 days");
        assert!(should_show(now + NOT_NOW_MS, "0.1.7", &state, false, false));
    }

    #[test]
    fn timing_gates_are_all_required() {
        let now = 50 * DAY_MS;
        let mut young = ready(now);
        young.first_launch_unix_ms = now - MIN_AGE_MS + 1;
        assert!(!should_show(now, "0.1.6", &young, false, false), "not before 7 days");
        young.first_launch_unix_ms = now - MIN_AGE_MS;
        assert!(can_start(now, "0.1.6", &young), "7 days is enough");

        let mut days = ready(now);
        days.active_days = 2;
        assert!(!can_start(now, "0.1.6", &days));
        let mut takes = ready(now);
        takes.successful_dictations = 19;
        assert!(!can_start(now, "0.1.6", &takes));
        let mut streak = ready(now);
        streak.recent_outcomes = vec!["ok".into(), "fixed".into(), "ok".into()];
        assert!(!can_start(now, "0.1.6", &streak), "a Fix last breaks the streak");
        streak.recent_outcomes = vec!["ok".into(), "error".into(), "ok".into()];
        assert!(!can_start(now, "0.1.6", &streak));
        streak.recent_outcomes = vec!["ok".into(), "ok".into()];
        assert!(!can_start(now, "0.1.6", &streak), "three clean takes");

        let mut capped = ready(now);
        capped.prompt_count = MAX_PROMPTS;
        assert!(!can_start(now, "0.1.6", &capped));
        let mut asked = ready(now);
        asked.prompted_versions = vec!["0.1.6".into()];
        assert!(!can_start(now, "0.1.6", &asked));
        assert!(can_start(now, "0.1.7", &asked));

        let mut quiet = ready(now);
        quiet.last_error_unix_ms = now - QUIET_MS + 1;
        assert!(!can_start(now, "0.1.6", &quiet));
        quiet.last_error_unix_ms = now - QUIET_MS;
        quiet.last_update_unix_ms = now - 1_000;
        assert!(!can_start(now, "0.1.6", &quiet), "a fresh update waits");
        quiet.last_update_unix_ms = 0;
        quiet.last_crash_unix_ms = now - 1_000;
        assert!(!can_start(now, "0.1.6", &quiet), "a crash restart waits");
        quiet.last_crash_unix_ms = now - QUIET_MS;
        assert!(can_start(now, "0.1.6", &quiet));
    }

    #[test]
    fn rated_is_final_and_a_network_blip_does_not_count() {
        let now = 50 * DAY_MS;
        let mut state = ready(now);
        begin_prompt(&mut state, "0.1.6");
        apply_review(&mut state, "0.1.6", ReviewResult::Rated, now);
        assert!(state.rated);
        assert!(!should_show(now + NOT_NOW_MS * 2, "9.0.0", &state, false, false));

        let mut flaky = ready(now);
        begin_prompt(&mut flaky, "0.1.6");
        apply_review(&mut flaky, "0.1.6", ReviewResult::Network, now);
        assert_eq!(flaky.prompt_count, 0);
        assert!(flaky.prompted_versions.is_empty());
        assert!(!can_start(now + 1_000, "0.1.6", &flaky), "at most one network retry a day");
        assert!(can_start(now + DAY_MS, "0.1.6", &flaky));
    }

    #[test]
    fn three_prompts_is_the_lifetime_cap() {
        let now = 50 * DAY_MS;
        let mut state = ready(now);
        for (index, version) in ["0.1.6", "0.1.7", "0.1.8"].iter().enumerate() {
            assert!(can_start(now + (index as u64) * NOT_NOW_MS, version, &state), "{version}");
            begin_prompt(&mut state, version);
            apply_review(&mut state, version, ReviewResult::AskLater, now + (index as u64) * NOT_NOW_MS);
        }
        assert_eq!(state.prompt_count, 3);
        assert!(!can_start(now + 4 * NOT_NOW_MS, "0.1.9", &state));
    }

    #[test]
    fn activity_success_and_fix_last_update_the_counters() {
        let mut state = RatingState::default();
        let day = 10 * DAY_MS + 3_600_000;
        note_activity(&mut state, day);
        note_activity(&mut state, day + 1_000);
        assert_eq!(state.active_days, 1);
        note_activity(&mut state, day + DAY_MS);
        assert_eq!(state.active_days, 2);
        note_success(&mut state, day + DAY_MS);
        note_success(&mut state, day + DAY_MS);
        assert_eq!(state.successful_dictations, 2);
        assert_eq!(state.recent_outcomes, vec!["ok".to_string(), "ok".to_string()]);
        note_fixed(&mut state);
        assert_eq!(state.recent_outcomes[0], "fixed");
        note_error(&mut state, day + DAY_MS + 5_000);
        assert_eq!(state.last_error_unix_ms, day + DAY_MS + 5_000);
        assert_eq!(state.recent_outcomes[0], "error");
    }

    #[test]
    fn startup_records_the_first_day_an_update_and_a_crash() {
        let now = 80 * DAY_MS;
        let mut state = RatingState::default();
        note_startup(&mut state, "0.1.6", now, Some(now - 30 * DAY_MS), false);
        assert_eq!(state.first_launch_unix_ms, now - 30 * DAY_MS);
        assert_eq!(state.active_days, 1);
        assert_eq!(state.last_update_unix_ms, 0, "the first seen version is not an update");
        note_startup(&mut state, "0.1.7", now + 1_000, None, true);
        assert_eq!(state.last_update_unix_ms, now + 1_000);
        assert_eq!(state.last_crash_unix_ms, now + 1_000);
        assert_eq!(state.first_launch_unix_ms, now - 30 * DAY_MS);
    }

    #[test]
    fn copy_follows_the_app_language_and_is_not_a_gate() {
        assert_eq!(language_for("auto", "english"), RatingLanguage::English);
        assert_eq!(language_for("en", "same"), RatingLanguage::English);
        assert_eq!(language_for("ur", "english"), RatingLanguage::RomanUrdu);
        assert_eq!(language_for("ur", "same"), RatingLanguage::Urdu);
        assert_eq!(language_for("roman-urdu", "english"), RatingLanguage::RomanUrdu);
        for language in [RatingLanguage::English, RatingLanguage::RomanUrdu, RatingLanguage::Urdu] {
            let copy = copy_for(language);
            assert_ne!(copy.rate, "Yes");
            assert_ne!(copy.later, "No");
            let blob = format!("{} {} {} {}", copy.title, copy.body, copy.rate, copy.thanks);
            let lower = blob.to_lowercase();
            assert!(!lower.contains("reward"));
            assert!(!lower.contains("unlock"));
            assert!(!copy.feedback.to_lowercase().contains("review"));
        }
        assert_eq!(copy_for(RatingLanguage::English).title, "Enjoying Bolo?");
        assert_eq!(copy_for(RatingLanguage::English).rate, "Rate Bolo");
        assert_eq!(copy_for(RatingLanguage::English).later, "Not now");
        assert_eq!(copy_for(RatingLanguage::RomanUrdu).rate, "Rating dein");
        assert_eq!(copy_for(RatingLanguage::Urdu).later, "ابھی نہیں");
        let mailto = feedback_mailto("0.1.6");
        assert!(mailto.starts_with("mailto:taimoor@kingdom-vision.co.uk?subject="));
        assert!(mailto.contains("0.1.6"));
        assert!(!mailto.contains("log"));
        assert!(!mailto.contains("history"));
        assert_eq!(REVIEW_URI, "ms-windows-store://review/?ProductId=9NMW6C99BF04");
        assert!(REVIEW_WEB.contains(PRODUCT_ID));
        assert_eq!(is_store_build(), !cfg!(feature = "updater"));
        assert_eq!(day_key(0), "1970-01-01");
        assert_eq!(day_key(DAY_MS), "1970-01-02");
    }
}
