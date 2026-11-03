//! Words Bolo must always spell right: a built-in seed list plus the user's
//! own "My words". Used three ways: first in the Whisper prompt, as a list the
//! cleanup model must keep exactly, and for a careful local fix-up of
//! sound-alike mishearings ("Volo" -> "Bolo", "taimoor hussain dot com" ->
//! "taimoorhussain.com").

use std::sync::Mutex;

/// Always known, even on a fresh install.
pub const SEED_WORDS: &[&str] = &[
    "Bolo",
    "Bolo.com",
    "Kingdom Vision",
    "Taimoor Hussain",
    "taimoorhussain.com",
    "thebetawebsite.com",
];

const TLDS: &[&str] = &["com", "net", "org", "io", "ai", "pk", "co", "app", "dev"];

static CURRENT: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// My words first, then the seeds, without duplicates (ignoring case).
pub fn protected(my_words: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let seeds = SEED_WORDS.iter().map(|s| s.to_string());
    for word in my_words.iter().cloned().chain(seeds) {
        let word = word.trim().to_string();
        if !word.is_empty() && !out.iter().any(|w| w.eq_ignore_ascii_case(&word)) {
            out.push(word);
        }
    }
    out
}

/// Remember the list for this take (cleanup and post-correction read it).
pub fn set_current(words: Vec<String>) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = words;
}

/// The list for this take; the seeds before any take has set it.
pub fn current() -> Vec<String> {
    let words = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if words.is_empty() {
        protected(&[])
    } else {
        words
    }
}

/// A rough "how it sounds" spelling: lower case, letters, digits and dots,
/// common sound-alike letters merged, doubled letters collapsed.
pub fn canon(input: &str) -> String {
    let mut s: String = input
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.')
        .collect();
    if s.starts_with("wr") {
        s.remove(0);
    }
    for (from, to) in [("ph", "f"), ("ck", "k"), ("oo", "u"), ("ou", "u"), ("ee", "i")] {
        s = s.replace(from, to);
    }
    let mut out = String::new();
    for c in s.chars() {
        let c = match c {
            'v' | 'w' => 'b',
            'z' => 's',
            'y' => 'i',
            'q' => 'k',
            other => other,
        };
        if out.chars().last() != Some(c) {
            out.push(c);
        }
    }
    out
}

pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            row[j] = (prev[j] + 1).min(row[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = row;
    }
    prev[b.len()]
}

/// True when `heard` is very likely a mishearing of `word`. Short words must
/// sound identical; longer ones may differ by about one letter in five, with
/// the same first sound and nearly the same length.
pub fn close(heard: &str, word: &str) -> bool {
    let h = canon(heard);
    let w = canon(word);
    if h.is_empty() || w.is_empty() {
        return false;
    }
    let n = w.chars().count();
    if n <= 5 {
        return h == w;
    }
    if h.chars().next() != w.chars().next() {
        return false;
    }
    let m = h.chars().count();
    if m.abs_diff(n) > 2 {
        return false;
    }
    edit_distance(&h, &w) <= n / 5
}

/// Looser check used to let the cleanup model fix a homophone
/// ("roots" -> "wrote"): same idea, about one letter in two.
pub fn sounds_alike(a: &str, b: &str) -> bool {
    let x = canon(a);
    let y = canon(b);
    if x.is_empty() || y.is_empty() {
        return false;
    }
    let n = x.chars().count().max(y.chars().count());
    edit_distance(&x, &y) <= (n / 2).max(1)
}

#[derive(Clone)]
struct Tok {
    start: usize,
    end: usize,
    lead: String,
    core: String,
    tail: String,
}

fn tokens(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let push = |s: usize, e: usize, out: &mut Vec<Tok>| {
        let raw = &text[s..e];
        let core_start = raw.find(|c: char| c.is_alphanumeric()).unwrap_or(raw.len());
        let core_end = raw
            .rfind(|c: char| c.is_alphanumeric())
            .map(|i| i + raw[i..].chars().next().map_or(1, char::len_utf8))
            .unwrap_or(core_start);
        out.push(Tok {
            start: s,
            end: e,
            lead: raw[..core_start].to_string(),
            core: raw[core_start..core_end.max(core_start)].to_string(),
            tail: raw[core_end.max(core_start)..].to_string(),
        });
    };
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                push(s, i, &mut out);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        push(s, text.len(), &mut out);
    }
    out
}

fn squash(word: &str) -> String {
    word.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Fix list words that were misheard, and spoken "dot com" web addresses.
pub fn correct(text: &str, words: &[String]) -> String {
    let toks = tokens(text);
    if toks.is_empty() {
        return text.to_string();
    }
    let mut list: Vec<&String> = words.iter().filter(|w| !w.trim().is_empty()).collect();
    list.sort_by_key(|w| std::cmp::Reverse(w.split_whitespace().count()));
    // (first token, last token, replacement core)
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if let Some((last, core)) = spoken_domain(&toks, i, &list) {
            edits.push((i, last, core));
            i = last + 1;
            continue;
        }
        if let Some((last, core)) = list_match(&toks, i, &list) {
            edits.push((i, last, core));
            i = last + 1;
            continue;
        }
        i += 1;
    }
    if edits.is_empty() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut at = 0;
    for (first, last, core) in edits {
        out.push_str(&text[at..toks[first].start]);
        out.push_str(&toks[first].lead);
        out.push_str(&core);
        out.push_str(&toks[last].tail);
        at = toks[last].end;
    }
    out.push_str(&text[at..]);
    out
}

/// "<name> dot com" (name up to three spoken words) starting at token `i`.
fn spoken_domain(toks: &[Tok], i: usize, list: &[&String]) -> Option<(usize, String)> {
    for k in (1..=3).rev() {
        let dot = i + k;
        let tld = dot + 1;
        if tld >= toks.len() || !toks[dot].core.eq_ignore_ascii_case("dot") {
            continue;
        }
        let ext = toks[tld].core.to_ascii_lowercase();
        if !TLDS.contains(&ext.as_str()) {
            continue;
        }
        // Punctuation inside the name ("Hussain, dot com") means it isn't one.
        if toks[i..dot].iter().enumerate().any(|(j, t)| (j > 0 && !t.lead.is_empty()) || !t.tail.is_empty()) {
            continue;
        }
        if !toks[dot].lead.is_empty() || !toks[dot].tail.is_empty() || !toks[tld].lead.is_empty() {
            continue;
        }
        let base: String = toks[i..dot].iter().map(|t| t.core.to_lowercase()).collect();
        if !base.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            continue;
        }
        if let Some(target) = known_domain(&base, &ext, list) {
            // "to the beta website dot com": start at "the", not "to".
            let shorter: String = toks[i + 1..dot].iter().map(|t| t.core.to_lowercase()).collect();
            if k > 1 && known_domain(&shorter, &ext, list).is_some() {
                return None;
            }
            return Some((tld, target));
        }
        // Not a known site: only a single, non-tiny spoken word becomes one.
        if k == 1 && base.chars().count() >= 3 && !COMMON.contains(&base.as_str()) {
            return Some((tld, format!("{base}.{ext}")));
        }
    }
    None
}

const COMMON: &[&str] = &[
    "the", "this", "that", "and", "our", "your", "their", "his", "her", "its", "was", "dot", "one", "new", "old",
];

fn known_domain(base: &str, ext: &str, list: &[&String]) -> Option<String> {
    if base.is_empty() {
        return None;
    }
    let heard = format!("{base}.{ext}");
    list.iter()
        .map(|word| squash(word))
        .find(|target| target.contains('.') && (target.eq_ignore_ascii_case(&heard) || close(&heard, target)))
}

/// The first spoken piece must sound like the start of the word, so a
/// neighbour such as "to" is never swallowed into it.
fn starts_like(first: &str, target: &str) -> bool {
    let f = canon(first);
    let t = canon(target);
    if f.is_empty() || f.chars().count() > t.chars().count() {
        return false;
    }
    if t.starts_with(&f) {
        return true;
    }
    let n = f.chars().count();
    n >= 3
        && (n - 1..=n + 1).any(|len| {
            let prefix: String = t.chars().take(len).collect();
            edit_distance(&f, &prefix) <= n / 3
        })
}

/// A list word misheard at token `i` (spanning one to three tokens).
fn list_match(toks: &[Tok], i: usize, list: &[&String]) -> Option<(usize, String)> {
    for word in list {
        let parts = word.split_whitespace().count().max(1);
        let target = squash(word);
        let mut sizes = vec![parts, parts + 1];
        if parts > 1 {
            sizes.push(parts - 1);
        }
        for size in sizes {
            if size == 0 || size > 3 || i + size > toks.len() {
                continue;
            }
            let span = &toks[i..i + size];
            if span.iter().enumerate().any(|(j, t)| (j > 0 && !t.lead.is_empty()) || (j + 1 < size && !t.tail.is_empty())) {
                continue;
            }
            if size > 1 && !starts_like(&span[0].core, &target) {
                continue;
            }
            let heard_spaced = span.iter().map(|t| t.core.as_str()).collect::<Vec<_>>().join(" ");
            let heard = squash(&heard_spaced);
            if heard.is_empty() || heard_spaced == word.as_str() {
                continue;
            }
            let same_letters = heard.eq_ignore_ascii_case(&target);
            // Case-only changes for a single plain word are left alone
            // ("bolo" is also Roman Urdu for "speak").
            if same_letters && parts == 1 && !target.contains('.') {
                continue;
            }
            if same_letters || close(&heard, &target) {
                return Some((i + size - 1, word.trim().to_string()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeds() -> Vec<String> {
        protected(&[])
    }

    #[test]
    fn my_words_come_before_the_seeds() {
        let list = protected(&["Priya".into(), "bolo".into()]);
        assert_eq!(list[0], "Priya");
        assert_eq!(list[1], "bolo");
        assert!(!list.iter().skip(2).any(|w| w.eq_ignore_ascii_case("bolo")));
        assert!(list.iter().any(|w| w == "thebetawebsite.com"));
    }

    #[test]
    fn fixes_volo_to_bolo() {
        assert_eq!(correct("Open Volo and start dictating.", &seeds()), "Open Bolo and start dictating.");
        assert_eq!(correct("Visit Volo.com today", &seeds()), "Visit Bolo.com today");
    }

    #[test]
    fn fixes_garbled_names_and_domains() {
        assert_eq!(correct("Send it to taimurhussain.com", &seeds()), "Send it to taimoorhussain.com");
        assert_eq!(correct("Send it to Taimur Hussain.com.", &seeds()), "Send it to taimoorhussain.com.");
        assert_eq!(correct("This is Taimur Husain from Kingdom vision.", &seeds()), "This is Taimoor Hussain from Kingdom Vision.");
        assert_eq!(correct("Tymoor Husain called", &seeds()), "Taimoor Hussain called");
        assert_eq!(correct("Email hello at taimoorhussain dot com", &seeds()), "Email hello at taimoorhussain.com");
        assert_eq!(correct("go to the beta website dot com now", &seeds()), "go to thebetawebsite.com now");
        assert_eq!(correct("Open bolo dot com.", &seeds()), "Open Bolo.com.");
    }

    #[test]
    fn spoken_dot_com_for_an_unknown_site() {
        assert_eq!(correct("check example dot com please", &seeds()), "check example.com please");
        // Two plain words before "dot com" are not glued together.
        assert_eq!(correct("we like the dot com era", &seeds()), "we like the dot com era");
    }

    #[test]
    fn leaves_real_words_alone() {
        let list = seeds();
        for text in [
            "Play polo on Sunday.",
            "I went solo.",
            "bolo kya haal hai",
            "Bowl of soup",
            "The kingdom fell.",
            "I wrote the email yesterday.",
            "Timer is set.",
            "Bolo and Kingdom Vision",
            "Hussain is here and Taimur called.",
            "We saw a bolero at the gala.",
            "Go to Tim's house to see the website.",
            "Vision and the kingdom of heaven",
        ] {
            assert_eq!(correct(text, &list), text, "{text}");
        }
    }

    #[test]
    fn keeps_spacing_and_punctuation() {
        assert_eq!(correct("(Volo),\n  next", &seeds()), "(Bolo),\n  next");
    }

    #[test]
    fn homophones_sound_alike() {
        assert!(sounds_alike("roots", "wrote"));
        assert!(sounds_alike("their", "there"));
        assert!(!sounds_alike("email", "yesterday"));
    }
}