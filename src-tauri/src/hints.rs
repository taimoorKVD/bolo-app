//! Words Whisper is told about on a fresh install. They are hints in the
//! prompt, not rows on the My words screen, so the screen stays the names
//! the person actually uses. They are not learned from anyone's voice.

/// Brands, tools, and apps people dictate.
const BRANDS: &[&str] = &[
    "WhatsApp", "YouTube", "GitHub", "LinkedIn", "Gmail", "Google", "Microsoft", "Slack", "Zoom",
    "Figma", "Notion", "ChatGPT", "Chrome", "Outlook", "Excel", "PowerPoint", "Windows",
];

/// Software words that are easy for Whisper to misspell.
const TECH: &[&str] = &[
    "API", "SSH", "EasySSH", "HTTP", "JSON", "URL", "SQL", "PDF", "HTML", "CSS", "JavaScript", "TypeScript", "Python", "React",
];

/// Names and places that come up in Pakistani English and Urdu dictation.
const NAMES: &[&str] = &[
    "Lahore", "Karachi", "Islamabad", "Rawalpindi", "Pakistan", "Ahmed", "Fatima", "Hassan", "Ayesha",
    "Ali", "Sara", "Usman", "Zainab",
];

/// Urdu words written in English letters, kept as themselves so Whisper does
/// not bend them into English look-alikes.
const ROMAN: &[&str] = &[
    "inshallah", "mashallah", "shukriya", "theek", "bhai", "yaar", "mujhe", "kal", "aaj",
];

/// Priority order for the Whisper prompt. Product names stay in the visible
/// seed list; this pack only fills whatever room is left.
pub fn starter_hints(language: &str) -> Vec<String> {
    let lang = language.trim().to_ascii_lowercase();
    let urdu = lang.is_empty() || lang == "auto" || lang == "ur";
    let mut out = Vec::new();
    let order: &[&[&str]] = if urdu {
        &[&BRANDS, &ROMAN, &NAMES, &TECH]
    } else {
        &[&BRANDS, &TECH, &NAMES, &ROMAN]
    };
    for group in order {
        for word in *group {
            if !out.iter().any(|have: &String| have.eq_ignore_ascii_case(word)) {
                out.push((*word).to_string());
            }
        }
    }
    out
}

/// Brands and addresses the Urdu translation must not drop or respell.
pub fn must_keep_english(word: &str) -> bool {
    let bare = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '@' && c != '.');
    if bare.contains('@') || (bare.contains('.') && bare.chars().any(|c| c.is_ascii_alphabetic())) {
        return true;
    }
    BRANDS.iter().chain(TECH.iter()).any(|hint| hint.eq_ignore_ascii_case(bare))
}

pub fn translation_keeps_english(original: &str, candidate: &str) -> bool {
    let lower = candidate.to_lowercase();
    original.split_whitespace().all(|token| {
        let bare = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '@' && c != '.');
        if !must_keep_english(bare) {
            return true;
        }
        lower.contains(&bare.to_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_pack_fits_the_prompt_and_is_not_a_screen_seed() {
        for language in ["en", "ur", "auto", ""] {
            let hints = starter_hints(language);
            assert!(hints.iter().any(|word| word == "GitHub"));
            assert!(hints.iter().any(|word| word == "Lahore"));
            assert!(hints.iter().any(|word| word == "WhatsApp"));
            assert!(hints.len() < 80, "{}", hints.len());
            let mut words = crate::vocab::protected(&[]);
            for hint in &hints {
                if !words.iter().any(|have| have.eq_ignore_ascii_case(hint)) {
                    words.push(hint.clone());
                }
            }
            let prompt = crate::transcribe::whisper_prompt_with(&words, &[]).unwrap();
            assert!(prompt.chars().count() <= 800, "{} {}", language, prompt.chars().count());
            assert!(prompt.contains("Bolo"));
            assert!(prompt.contains("GitHub") || prompt.contains("WhatsApp"));
        }
        let screen = crate::memory::screen_words(&crate::memory::Memory::default());
        assert!(screen.iter().all(|row| row.seed));
        assert!(!screen.iter().any(|row| row.word == "GitHub"));
        assert!(screen.iter().any(|row| row.word == "Bolo"));
    }

    #[test]
    fn translation_must_keep_brands_and_addresses() {
        assert!(translation_keeps_english(
            "report WhatsApp pe bhej do",
            "Send the report on WhatsApp"
        ));
        assert!(!translation_keeps_english("report WhatsApp pe bhej do", "Send the report"));
        assert!(translation_keeps_english(
            "mail taimoorhussain.com",
            "Email taimoorhussain.com"
        ));
        assert!(must_keep_english("API"));
        assert!(!must_keep_english("bhej"));
    }
}
