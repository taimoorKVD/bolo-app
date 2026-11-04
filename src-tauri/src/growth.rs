//! Local growth: a share line, quiet milestone thank-yous, and a character card.
//!
//! Nothing here phones home. Counters stay in the settings file. The share
//! text is never appended to a dictation.

use serde::Serialize;

pub const SHARE_URL: &str = "https://apps.microsoft.com/detail/9NMW6C99BF04?cid=share-app";
pub const SITE_HOST: &str = "bolo.thebetawebsite.com";
pub const MILESTONES: [u32; 3] = [100, 500, 1000];

/// Urdu, Roman Urdu, and English, plus the Store link. Not a signature.
pub fn share_message() -> String {
    format!(
        "بولو سے بولو، ٹائپنگ خود ہو جائے۔\nBolo se bolo, typing khud ho jaye.\nSpeak, and Bolo types it for you.\n{SHARE_URL}"
    )
}

pub fn whatsapp_links(message: &str) -> (String, String) {
    let text = percent_encode(message);
    (
        format!("whatsapp://send?text={text}"),
        format!("https://web.whatsapp.com/send?text={text}"),
    )
}

/// The earliest milestone the person has reached and not dismissed.
pub fn pending_milestone(successes: u32, dismissed: &[u32]) -> Option<u32> {
    MILESTONES.into_iter().find(|mark| successes >= *mark && !dismissed.contains(mark))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeChoice {
    None,
    Rating,
    Milestone(u32),
}

/// One card on Home. A milestone waits while a rating ask is already open,
/// and a new rating ask waits while a milestone is still undismissed.
pub fn choose_home_card(
    dictating: bool,
    overlay_visible: bool,
    rating_open: bool,
    rating_eligible: bool,
    successes: u32,
    dismissed: &[u32],
) -> HomeChoice {
    if dictating || overlay_visible {
        return HomeChoice::None;
    }
    if rating_open {
        return HomeChoice::Rating;
    }
    if let Some(mark) = pending_milestone(successes, dismissed) {
        return HomeChoice::Milestone(mark);
    }
    if rating_eligible {
        HomeChoice::Rating
    } else {
        HomeChoice::None
    }
}

pub fn milestone_copy(mark: u32) -> (&'static str, &'static str) {
    match mark {
        100 => (
            "100 dictations",
            "Shukriya. A hundred times you talked and Bolo typed.",
        ),
        500 => (
            "500 dictations",
            "Five hundred times. That's a lot of typing you didn't have to do.",
        ),
        _ => (
            "1,000 dictations",
            "A thousand times. Bolo has been listening for a while.",
        ),
    }
}

pub fn dismiss_milestone(dismissed: &mut Vec<u32>, mark: u32) {
    if MILESTONES.contains(&mark) && !dismissed.contains(&mark) {
        dismissed.push(mark);
    }
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

const CARD_W: u32 = 560;
const CARD_H: u32 = 460;
const FOOTER_TOP: u32 = 392;

/// A dark card: the character picture, then the site address. No network.
pub fn compose_character_card(png: &[u8]) -> Result<Vec<u8>, String> {
    let source = image::load_from_memory(png).map_err(|_| "That picture couldn't be read.".to_string())?;
    let source = source.to_rgba8();
    if source.width() == 0 || source.height() == 0 {
        return Err("That picture couldn't be read.".into());
    }
    let max_w = CARD_W - 80;
    let max_h = FOOTER_TOP - 48;
    let scale = (max_w as f32 / source.width() as f32).min(max_h as f32 / source.height() as f32).min(2.0);
    let dw = ((source.width() as f32) * scale).round().max(1.0) as u32;
    let dh = ((source.height() as f32) * scale).round().max(1.0) as u32;
    let scaled = image::imageops::resize(&source, dw, dh, image::imageops::FilterType::Triangle);
    let bg = image::Rgba([16, 15, 12, 255]);
    let mut card = image::RgbaImage::from_pixel(CARD_W, CARD_H, bg);
    let x0 = (CARD_W - dw) / 2;
    let y0 = (FOOTER_TOP - dh) / 2;
    image::imageops::overlay(&mut card, &scaled, x0.into(), y0.into());
    let gold = image::Rgba([201, 162, 39, 255]);
    for x in 80..CARD_W - 80 {
        card.put_pixel(x, FOOTER_TOP - 16, gold);
    }
    draw_text(&mut card, SITE_HOST, FOOTER_TOP, gold);
    let mut bytes = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut bytes);
    card.write_with_encoder(image::codecs::png::PngEncoder::new(&mut cursor))
        .map_err(|_| "Couldn't prepare the picture.".to_string())?;
    Ok(bytes)
}

pub fn write_png(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    std::fs::write(path, bytes).map_err(|err| err.to_string())
}

fn draw_text(card: &mut image::RgbaImage, text: &str, top: u32, color: image::Rgba<u8>) {
    let scale = 3u32;
    let advance = 6 * scale;
    let width = text.chars().count() as u32 * advance;
    let mut x = CARD_W.saturating_sub(width) / 2;
    for ch in text.chars() {
        let bits = glyph(ch);
        for (row, bits) in bits.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = x + col * scale + dx;
                            let py = top + row as u32 * scale + dy;
                            if px < card.width() && py < card.height() {
                                card.put_pixel(px, py, color);
                            }
                        }
                    }
                }
            }
        }
        x += advance;
    }
}

fn glyph(ch: char) -> [u8; 7] {
    match ch {
        'a' => [0b01110, 0b00001, 0b01111, 0b10001, 0b01111, 0, 0],
        'b' => [0b11110, 0b10001, 0b11110, 0b10001, 0b11110, 0, 0],
        'c' => [0b01110, 0b10001, 0b10000, 0b10001, 0b01110, 0, 0],
        'd' => [0b01111, 0b10001, 0b10001, 0b10001, 0b01111, 0, 0],
        'e' => [0b11111, 0b10000, 0b11110, 0b10000, 0b11111, 0, 0],
        'f' => [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0, 0],
        'g' => [0b01110, 0b10000, 0b10011, 0b10001, 0b01110, 0, 0],
        'h' => [0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0, 0],
        'i' => [0b01110, 0b00100, 0b00100, 0b00100, 0b01110, 0, 0],
        'j' => [0b00111, 0b00010, 0b00010, 0b10010, 0b01100, 0, 0],
        'k' => [0b10001, 0b10010, 0b11100, 0b10010, 0b10001, 0, 0],
        'l' => [0b10000, 0b10000, 0b10000, 0b10000, 0b11111, 0, 0],
        'm' => [0b10001, 0b11011, 0b10101, 0b10001, 0b10001, 0, 0],
        'n' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0, 0],
        'o' => [0b01110, 0b10001, 0b10001, 0b10001, 0b01110, 0, 0],
        'p' => [0b11110, 0b10001, 0b11110, 0b10000, 0b10000, 0, 0],
        'q' => [0b01110, 0b10001, 0b10001, 0b10011, 0b01111, 0, 0],
        'r' => [0b11110, 0b10001, 0b11110, 0b10010, 0b10001, 0, 0],
        's' => [0b01111, 0b10000, 0b01110, 0b00001, 0b11110, 0, 0],
        't' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0],
        'u' => [0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0, 0],
        'v' => [0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0, 0],
        'w' => [0b10001, 0b10001, 0b10101, 0b11011, 0b10001, 0, 0],
        'x' => [0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0, 0],
        'y' => [0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0, 0],
        'z' => [0b11111, 0b00010, 0b00100, 0b01000, 0b11111, 0, 0],
        '.' => [0, 0, 0, 0, 0b00100, 0, 0],
        _ => [0; 7],
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HomeCard {
    Rating {
        title: String,
        body: String,
        rate_label: String,
        later_label: String,
        feedback_label: String,
        rtl: bool,
    },
    Milestone {
        count: u32,
        title: String,
        body: String,
    },
}

#[tauri::command]
pub fn home_card(app: tauri::AppHandle) -> Option<HomeCard> {
    use tauri::Manager;
    let version = env!("CARGO_PKG_VERSION");
    let state = app.state::<crate::dictation::AppState>();
    let dictating = !crate::dictation::is_idle(&state);
    let overlay_visible = app
        .get_webview_window("overlay")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    let settings = state.settings.lock().unwrap_or_else(|err| err.into_inner());
    let rating_open = crate::rating::prompt_is_open(&settings.rating, version);
    let rating_eligible = crate::rating::should_show(
        crate::store::now_ms(),
        version,
        &settings.rating,
        false,
        false,
    ) && !rating_open;
    let choice = choose_home_card(
        dictating,
        overlay_visible,
        rating_open,
        rating_eligible,
        settings.rating.successful_dictations,
        &settings.rating.dismissed_milestones,
    );
    let language = crate::rating::language_for(&settings.language, &settings.output);
    drop(settings);
    match choice {
        HomeChoice::None => None,
        HomeChoice::Milestone(count) => {
            let (title, body) = milestone_copy(count);
            Some(HomeCard::Milestone {
                count,
                title: title.into(),
                body: body.into(),
            })
        }
        HomeChoice::Rating => {
            if !rating_open {
                with_rating(&app, |rating| crate::rating::begin_prompt(rating, version));
            }
            let copy = crate::rating::copy_for(language);
            Some(HomeCard::Rating {
                title: copy.title.into(),
                body: copy.body.into(),
                rate_label: copy.rate.into(),
                later_label: copy.later.into(),
                feedback_label: copy.feedback.into(),
                rtl: language == crate::rating::RatingLanguage::Urdu,
            })
        }
    }
}

#[tauri::command]
pub fn dismiss_home_milestone(app: tauri::AppHandle, count: u32) {
    with_rating(&app, |rating| dismiss_milestone(&mut rating.dismissed_milestones, count));
}

#[tauri::command]
pub fn copy_share_message() -> Result<(), String> {
    crate::insert::set_clipboard_text(&share_message()).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn open_share_whatsapp() -> Result<(), String> {
    let (app_link, web_link) = whatsapp_links(&share_message());
    if open::that(app_link).is_ok() {
        return Ok(());
    }
    open::that(web_link).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn save_character_card(app: tauri::AppHandle, bytes: Vec<u8>) -> Result<bool, String> {
    let png = compose_character_card(&bytes)?;
    let path = pick_save_path(&app)?;
    let Some(path) = path else {
        return Ok(false);
    };
    write_png(&path, &png)?;
    Ok(true)
}

fn with_rating(app: &tauri::AppHandle, update: impl FnOnce(&mut crate::rating::RatingState)) {
    use tauri::Manager;
    let state = app.state::<crate::dictation::AppState>();
    let settings = {
        let mut settings = state.settings.lock().unwrap_or_else(|err| err.into_inner());
        update(&mut settings.rating);
        settings.clone()
    };
    if let Err(err) = crate::store::save_settings(&state.config_dir, &settings) {
        crate::logbuf::log_line(&state.config_dir, "error", &format!("growth: {err}"));
    }
}

fn pick_save_path(app: &tauri::AppHandle) -> Result<Option<std::path::PathBuf>, String> {
    #[cfg(windows)]
    {
        return pick_save_path_windows(app);
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("Saving the picture works in the Windows app.".into())
    }
}

#[cfg(windows)]
fn pick_save_path_windows(app: &tauri::AppHandle) -> Result<Option<std::path::PathBuf>, String> {
    use tauri::Manager;
    let window = app.get_webview_window("settings").ok_or_else(|| "Bolo's window isn't open.".to_string())?;
    let foreign = window.hwnd().map_err(|err| err.to_string())?;
    let hwnd = windows::Win32::Foundation::HWND(foreign.0);
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let outcome = unsafe { show_save_dialog(hwnd) };
        let _ = tx.send(outcome);
    })
    .map_err(|err| err.to_string())?;
    rx.recv_timeout(std::time::Duration::from_secs(180))
        .map_err(|_| "The save dialog didn't finish.".to_string())?
}

#[cfg(windows)]
unsafe fn show_save_dialog(hwnd: windows::Win32::Foundation::HWND) -> Result<Option<std::path::PathBuf>, String> {
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{IFileSaveDialog, IModalWindow, FileSaveDialog, SIGDN_FILESYSPATH};
    let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).map_err(|err| err.to_string())?;
    dialog.SetTitle(&windows_core::HSTRING::from("Save your Bolo character")).map_err(|err| err.to_string())?;
    dialog.SetFileName(&windows_core::HSTRING::from("bolo-character.png")).map_err(|err| err.to_string())?;
    dialog.SetDefaultExtension(&windows_core::HSTRING::from("png")).map_err(|err| err.to_string())?;
    let modal: IModalWindow = dialog.cast().map_err(|err| err.to_string())?;
    if let Err(err) = modal.Show(Some(hwnd)) {
        if err.code().0 as u32 == 0x8007_04C7 {
            return Ok(None);
        }
        return Err(err.to_string());
    }
    let item = dialog.GetResult().map_err(|err| err.to_string())?;
    let wide = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|err| err.to_string())?;
    let path = {
        let mut len = 0usize;
        while !wide.0.is_null() && *wide.0.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(wide.0, len))
    };
    CoTaskMemFree(Some(wide.0 as *const core::ffi::c_void));
    Ok(Some(std::path::PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_share_message_is_urdu_and_english_with_the_store_link() {
        let message = share_message();
        assert!(message.contains("بولو سے بولو"));
        assert!(message.contains("Bolo se bolo, typing khud ho jaye."));
        assert!(message.contains("Speak, and Bolo types it for you."));
        assert!(message.contains(SHARE_URL));
        assert!(message.contains("cid=share-app"));
        assert!(!message.contains("cid=inapp_rate"));
        assert!(!message.to_lowercase().contains("signature"));
        let (app_link, web_link) = whatsapp_links(&message);
        assert!(app_link.starts_with("whatsapp://send?text="));
        assert!(web_link.starts_with("https://web.whatsapp.com/send?text="));
        assert!(app_link.contains("cid%3Dshare-app") || app_link.contains("cid=share-app"));
        assert!(app_link.contains("%D8") || app_link.contains("بولو"));
    }

    #[test]
    fn milestones_show_once_in_order_and_can_be_dismissed() {
        assert_eq!(pending_milestone(99, &[]), None);
        assert_eq!(pending_milestone(100, &[]), Some(100));
        assert_eq!(pending_milestone(1000, &[]), Some(100), "the earliest unshown thank-you comes first");
        let mut dismissed = vec![100];
        assert_eq!(pending_milestone(1000, &dismissed), Some(500));
        dismiss_milestone(&mut dismissed, 500);
        dismiss_milestone(&mut dismissed, 500);
        assert_eq!(pending_milestone(1000, &dismissed), Some(1000));
        dismiss_milestone(&mut dismissed, 1000);
        assert_eq!(pending_milestone(1000, &dismissed), None);
        dismiss_milestone(&mut dismissed, 42);
        assert!(!dismissed.contains(&42));
    }

    #[test]
    fn a_milestone_and_the_rating_card_never_show_together() {
        let dismissed = [];
        assert_eq!(choose_home_card(true, false, false, true, 100, &dismissed), HomeChoice::None);
        assert_eq!(choose_home_card(false, true, false, true, 100, &dismissed), HomeChoice::None);
        assert_eq!(
            choose_home_card(false, false, true, true, 100, &dismissed),
            HomeChoice::Rating,
            "an open rating ask stays; the thank-you waits"
        );
        assert_eq!(
            choose_home_card(false, false, false, true, 100, &dismissed),
            HomeChoice::Milestone(100),
            "a new rating ask waits until the thank-you is dismissed"
        );
        assert_eq!(choose_home_card(false, false, false, true, 20, &[100, 500, 1000]), HomeChoice::Rating);
        assert_eq!(choose_home_card(false, false, false, false, 50, &[]), HomeChoice::None);
    }

    #[test]
    fn the_character_card_is_a_png_with_the_site_address_drawn_on_it() {
        let tiny = image::RgbaImage::from_pixel(4, 4, image::Rgba([220, 80, 60, 255]));
        let mut raw = Vec::new();
        tiny.write_with_encoder(image::codecs::png::PngEncoder::new(std::io::Cursor::new(&mut raw))).unwrap();
        let card = compose_character_card(&raw).unwrap();
        assert!(card.starts_with(b"\x89PNG"));
        let decoded = image::load_from_memory(&card).unwrap().to_rgba8();
        assert_eq!(decoded.width(), CARD_W);
        assert!(decoded.height() > 4);
        let gold = decoded.pixels().skip((FOOTER_TOP as usize) * CARD_W as usize).any(|px| px[0] > 160 && px[1] > 120 && px[2] < 90);
        assert!(gold, "the site address is drawn under the character");
        let dir = std::env::temp_dir().join(format!("bolo-share-png-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("card.png");
        write_png(&path, &card).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), card);
        let _ = std::fs::remove_dir_all(dir);
    }
}
