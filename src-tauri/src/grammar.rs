//! Local grammar for dictated English.
//!
//! The model cleanup often never lands: its prompt used to forbid word
//! changes, the acceptor rejected any new form, and a fixed 1.2s budget
//! expired while the model was still reasoning. This pass is the reliable
//! layer. It fixes capitalization, punctuation, a/an, and a few agreement
//! forms of words the speaker already said. It does not add content words.

/// How long cleanup may wait, growing with the transcript so a short take
/// stays snappy and a longer one is not cut off mid-sentence.
pub fn cleanup_timeout_ms(chars: usize) -> u64 {
    (800 + (chars as u64) * 3).clamp(800, 2_200)
}

/// True when `a` and `b` are the same word or an agreement form of it
/// (go/goes/went, is/are/was, a/an, don't/doesn't).
pub fn same_lexeme(a: &str, b: &str) -> bool {
    let a = a.to_ascii_lowercase();
    let b = b.to_ascii_lowercase();
    if a == b {
        return true;
    }
    const GROUPS: &[&[&str]] = &[
        &["go", "goes", "went", "going"],
        &["is", "are", "am", "was", "were"],
        &["have", "has", "had"],
        &["do", "does", "did", "don't", "doesn't", "dont", "doesnt"],
        &["a", "an"],
    ];
    GROUPS.iter().any(|group| group.contains(&a.as_str()) && group.contains(&b.as_str()))
}

/// Capitalize, punctuate, and fix agreement. Roman Urdu and Urdu script are
/// left alone so this pass does not invent English.
pub fn polish(text: &str) -> String {
    let text = tidy(text);
    if text.is_empty() || !english_like(&text) {
        return text;
    }
    let mut tokens: Vec<String> = text.split_whitespace().map(|token| token.to_string()).collect();
    contractions(&mut tokens);
    agreement(&mut tokens);
    articles(&mut tokens);
    acronyms(&mut tokens);
    capitalize(&mut tokens);
    terminal(&mut tokens);
    tokens.join(" ")
}

fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn english_like(text: &str) -> bool {
    if crate::transcribe::has_arabic(text) {
        return false;
    }
    if roman_markers(text) >= 2 {
        return false;
    }
    let words: Vec<&str> = text.split_whitespace().filter(|word| word.chars().any(|c| c.is_ascii_alphabetic())).collect();
    words.len() >= 2
}

fn roman_markers(text: &str) -> usize {
    const MARKERS: &[&str] = &[
        "hai", "hain", "kya", "mein", "theek", "thik", "yaar", "bhai", "shukriya", "nahin", "nahi", "aaj",
        "mujhe", "hoon", "hun", "karo", "wala", "wali", "acha", "achha", "gya", "gayi",
    ];
    text.split_whitespace()
        .filter(|word| MARKERS.iter().any(|marker| core(word) == *marker))
        .count()
}

fn contractions(tokens: &mut [String]) {
    for token in tokens.iter_mut() {
        if is_protected_token(token) {
            continue;
        }
        let next = match core(token).as_str() {
            "im" => "I'm",
            "dont" => "don't",
            "doesnt" => "doesn't",
            "cant" => "can't",
            "wont" => "won't",
            "ive" => "I've",
            "id" => "I'd",
            "isnt" => "isn't",
            "wasnt" => "wasn't",
            "arent" => "aren't",
            "werent" => "weren't",
            _ => continue,
        };
        *token = splice(token, next);
    }
}

fn agreement(tokens: &mut [String]) {
    let past = tokens.iter().any(|token| matches!(core(token).as_str(), "yesterday" | "ago" | "earlier"));
    for index in 0..tokens.len() {
        let prev = index.checked_sub(1).map(|i| core(&tokens[i])).unwrap_or_default();
        if !is_subject(&prev) || is_protected_token(&tokens[index]) {
            continue;
        }
        let word = core(&tokens[index]);
        let next = match word.as_str() {
            "goes" | "go" | "went" if past => "went",
            "goes" if matches!(prev.as_str(), "i" | "we" | "they" | "you") => "go",
            "go" if matches!(prev.as_str(), "he" | "she" | "it") && !past => "goes",
            "is" | "are" | "am" | "was" | "were" if past && matches!(prev.as_str(), "we" | "they" | "you") => "were",
            "is" | "are" | "am" | "was" | "were" if past => "was",
            "is" | "are" if prev == "i" => "am",
            "are" if matches!(prev.as_str(), "he" | "she" | "it") => "is",
            "is" | "am" if matches!(prev.as_str(), "we" | "they" | "you") => "are",
            "was" if matches!(prev.as_str(), "we" | "they" | "you") => "were",
            "were" if matches!(prev.as_str(), "he" | "she" | "it" | "i") => "was",
            "don't" | "dont" if matches!(prev.as_str(), "he" | "she" | "it") => "doesn't",
            "doesn't" | "doesnt" if matches!(prev.as_str(), "i" | "we" | "they" | "you") => "don't",
            "has" if matches!(prev.as_str(), "i" | "we" | "they" | "you") => "have",
            "have" if matches!(prev.as_str(), "he" | "she" | "it") => "has",
            _ => continue,
        };
        if next != word {
            tokens[index] = splice(&tokens[index], next);
        }
    }
}

fn articles(tokens: &mut [String]) {
    for index in 0..tokens.len().saturating_sub(1) {
        let word = core(&tokens[index]);
        if word != "a" && word != "an" {
            continue;
        }
        if is_protected_token(&tokens[index + 1]) {
            continue;
        }
        let next = if needs_an(&core(&tokens[index + 1])) { "an" } else { "a" };
        if next != word {
            tokens[index] = splice(&tokens[index], next);
        }
    }
}

fn acronyms(tokens: &mut [String]) {
    for token in tokens.iter_mut() {
        if is_protected_token(token) {
            continue;
        }
        let next = match core(token).as_str() {
            "api" => "API",
            "pdf" => "PDF",
            "sql" => "SQL",
            "url" => "URL",
            "html" => "HTML",
            "css" => "CSS",
            _ => continue,
        };
        *token = splice(token, next);
    }
}

fn capitalize(tokens: &mut [String]) {
    let mut start = true;
    for token in tokens.iter_mut() {
        if start && !is_protected_token(token) {
            cap_first(token);
        }
        let core_word = core(token);
        if core_word == "i" && !is_protected_token(token) {
            *token = splice(token, "I");
        }
        start = token.ends_with(['.', '?', '!']);
    }
}

fn terminal(tokens: &mut [String]) {
    let Some(last) = tokens.last_mut() else {
        return;
    };
    if last.ends_with(['.', '?', '!', ',', ':', ';']) {
        return;
    }
    last.push('.');
}

fn needs_an(next: &str) -> bool {
    if matches!(next, "api" | "url" | "sql" | "html" | "user" | "university" | "one" | "once" | "euro" | "use" | "used") {
        return false;
    }
    if next.starts_with("hour") || next.starts_with("honest") || next.starts_with("honor") {
        return true;
    }
    matches!(next.chars().next(), Some('a' | 'e' | 'i' | 'o' | 'u'))
}

fn is_subject(word: &str) -> bool {
    matches!(word, "i" | "we" | "they" | "you" | "he" | "she" | "it")
}

fn is_protected_token(token: &str) -> bool {
    token.contains('@') || looks_like_domain(token)
}

fn looks_like_domain(token: &str) -> bool {
    let bare = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.');
    let Some((host, tld)) = bare.rsplit_once('.') else {
        return false;
    };
    !host.is_empty()
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && (2..=6).contains(&tld.len())
        && tld.chars().all(|c| c.is_ascii_alphabetic())
}

fn core(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        .to_ascii_lowercase()
}

fn splice(token: &str, replacement: &str) -> String {
    let start = token.find(|c: char| c.is_alphanumeric() || c == '\'').unwrap_or(0);
    let end = token
        .rfind(|c: char| c.is_alphanumeric() || c == '\'')
        .map(|index| index + token[index..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(start);
    format!("{}{}{}", &token[..start], replacement, &token[end..])
}

fn cap_first(token: &mut String) {
    let start = token.find(|c: char| c.is_ascii_alphabetic());
    let Some(start) = start else {
        return;
    };
    let mut chars = token[start..].chars();
    let Some(first) = chars.next() else {
        return;
    };
    if first.is_uppercase() {
        return;
    }
    let mut next = token[..start].to_string();
    next.push_str(&first.to_ascii_uppercase().to_string());
    next.push_str(chars.as_str());
    *token = next;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_examples_before_and_after() {
        let cases = [
            ("i goes to office yesterday", "I went to office yesterday."),
            ("she don't like the update", "She doesn't like the update."),
            ("we was late for the meeting", "We were late for the meeting."),
            ("this is a apple", "This is an apple."),
            ("he have the file", "He has the file."),
            ("send the notes please", "Send the notes please."),
            ("the total is 1500 rupees", "The total is 1500 rupees."),
            ("we need 3 api keys", "We need 3 API keys."),
            ("I wrote the email yesterday.", "I wrote the email yesterday."),
            ("bolo kya haal hai", "bolo kya haal hai"),
            ("hello@taimoorhussain.com", "hello@taimoorhussain.com"),
            ("میں ٹھیک ہوں", "میں ٹھیک ہوں"),
        ];
        for (before, after) in cases {
            assert_eq!(polish(before), after, "{before}");
            assert_eq!(polish(after), after, "idempotent {after}");
        }
    }

    #[test]
    fn timeout_grows_with_length_and_stays_bounded() {
        let short = cleanup_timeout_ms(20);
        let medium = cleanup_timeout_ms(200);
        let long = cleanup_timeout_ms(800);
        eprintln!("GRAMMAR cleanup_timeout_ms short={short} medium={medium} long={long}");
        assert!(short <= 1_200, "{short}");
        assert!(short < medium && medium < long);
        assert!(long <= 2_200);
        assert!(same_lexeme("goes", "went"));
        assert!(same_lexeme("a", "an"));
        assert!(!same_lexeme("email", "yesterday"));
    }
}
