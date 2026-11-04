//! Personal memory, kept only on this PC in `memory.json`.
//!
//! Learned words live in `settings.dictionary` (as in 0.1.3, so older builds
//! still read them). This file adds what 0.1.3 did not have: names seen once
//! and waiting for a second take, words the user told Bolo to forget, the
//! user's own fixes (a replacement map), and a few recent sentences used as a
//! style reference. A missing or unreadable file starts empty.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const VERSION: u32 = 1;
const MAX_CORRECTIONS: usize = 100;
const MAX_STYLE: usize = 5;
const MAX_PENDING: usize = 200;
const MAX_BLOCKED: usize = 300;
const MAX_MY_WORDS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Correction {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub created_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Memory {
    pub version: u32,
    /// Plain names seen once mid-sentence; learned on the second take.
    pub pending: Vec<String>,
    /// Words the user forgot. They are never learned again automatically.
    pub blocked: Vec<String>,
    pub corrections: Vec<Correction>,
    pub style: Vec<String>,
    /// "My words": names and terms the user added or fixed. Always first in
    /// the Whisper prompt and kept exactly by cleanup.
    pub my_words: Vec<String>,
    /// How often each My word showed up in a take. Most-used lead the prompt.
    pub uses: Vec<WordUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordUse {
    pub word: String,
    pub uses: u32,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            version: VERSION,
            pending: Vec::new(),
            blocked: Vec::new(),
            corrections: Vec::new(),
            style: Vec::new(),
            my_words: Vec::new(),
            uses: Vec::new(),
        }
    }
}

pub fn path(config: &Path) -> PathBuf {
    config.join("memory.json")
}

/// Missing file: defaults (the 0.1.3 upgrade path). Corrupt file: kept aside
/// as `memory.bad.json` and replaced with defaults, never a crash.
pub fn load(config: &Path) -> Memory {
    let file = path(config);
    let Ok(raw) = std::fs::read_to_string(&file) else {
        return Memory::default();
    };
    match serde_json::from_str::<Memory>(&raw) {
        Ok(memory) => migrate(memory),
        Err(_) => {
            let _ = std::fs::copy(&file, config.join("memory.bad.json"));
            Memory::default()
        }
    }
}

fn migrate(mut memory: Memory) -> Memory {
    if memory.version < VERSION {
        memory.version = VERSION;
    }
    memory.corrections.retain(|c| !c.from.trim().is_empty() && !c.to.trim().is_empty());
    memory.corrections.truncate(MAX_CORRECTIONS);
    memory.style.truncate(MAX_STYLE);
    memory.pending.truncate(MAX_PENDING);
    memory.my_words.truncate(MAX_MY_WORDS);
    memory.uses.truncate(MAX_MY_WORDS);
    memory
}

pub fn save(config: &Path, memory: &Memory) -> Result<(), String> {
    crate::store::write_atomic(&path(config), memory).map_err(|e| e.to_string())
}

static STATE: Mutex<Option<(PathBuf, Memory)>> = Mutex::new(None);

pub fn init(config: &Path) {
    let memory = load(config);
    crate::cleanup::set_style(memory.style.clone());
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some((config.to_path_buf(), memory));
}

/// A copy of the current memory (defaults before `init`).
pub fn snapshot() -> Memory {
    STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|(_, m)| m.clone())
        .unwrap_or_default()
}

/// Change memory and save it. Returns the closure's result.
pub fn update<T>(change: impl FnOnce(&mut Memory) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_mut() {
        Some((dir, memory)) => {
            let before = memory.clone();
            let out = change(memory);
            if memory.style != before.style {
                crate::cleanup::set_style(memory.style.clone());
            }
            if *memory != before {
                if let Err(err) = save(dir, memory) {
                    crate::logbuf::log_line(dir, "error", &format!("memory save: {err}"));
                }
            }
            out
        }
        None => {
            let mut scratch = Memory::default();
            change(&mut scratch)
        }
    }
}

/// Words the user's fixes taught, for the Whisper prompt.
pub fn prompt_terms(memory: &Memory) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for c in memory.corrections.iter() {
        let to = c.to.trim();
        if !to.is_empty() && to.chars().count() <= 40 && !terms.iter().any(|t| t.eq_ignore_ascii_case(to)) {
            terms.push(to.to_string());
        }
    }
    terms
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '_'
}

/// Replace each fix's `from` with its `to`, whole words only, ignoring case.
pub fn apply_corrections(text: &str, corrections: &[Correction]) -> String {
    let mut out = text.to_string();
    // Longer phrases first so "my sequel" wins over "sequel".
    let mut sorted: Vec<&Correction> = corrections.iter().collect();
    sorted.sort_by_key(|c| std::cmp::Reverse(c.from.chars().count()));
    for c in sorted {
        let from = c.from.trim();
        if from.is_empty() || from == c.to {
            continue;
        }
        out = replace_words(&out, from, c.to.trim());
    }
    out
}

fn replace_words(text: &str, from: &str, to: &str) -> String {
    let lower_text = text.to_lowercase();
    let lower_from = from.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII; fall back to exact match then.
    let (hay, needle) = if lower_text.len() == text.len() && lower_from.len() == from.len() {
        (lower_text, lower_from)
    } else {
        (text.to_string(), from.to_string())
    };
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut search = 0;
    while let Some(found) = hay[search..].find(&needle) {
        let start = search + found;
        let end = start + needle.len();
        let before_ok = text[..start].chars().next_back().is_none_or(|c| !is_word_char(c));
        let after_ok = text[end..].chars().next().is_none_or(|c| !is_word_char(c));
        if before_ok && after_ok {
            out.push_str(&text[cursor..start]);
            out.push_str(to);
            cursor = end;
            search = end;
        } else {
            search = start + text[start..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        }
        if search >= hay.len() {
            break;
        }
    }
    out.push_str(&text[cursor..]);
    out
}

fn bare(word: &str) -> String {
    word.trim_matches(|c: char| !is_word_char(c)).to_string()
}

/// Compare what Bolo wrote with what the user typed and return the changed
/// span as one fix. Spans longer than five words are not useful as a
/// replacement (the user rewrote the sentence), so they give nothing.
pub fn diff_fix(old: &str, new: &str) -> Option<(String, String)> {
    let a: Vec<String> = old.split_whitespace().map(bare).filter(|w| !w.is_empty()).collect();
    let b: Vec<String> = new.split_whitespace().map(bare).filter(|w| !w.is_empty()).collect();
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let from = &a[pre..a.len() - suf];
    let to = &b[pre..b.len() - suf];
    if from.is_empty() || to.is_empty() || from.len() > 5 || to.len() > 5 {
        return None;
    }
    let from = from.join(" ");
    let to = to.join(" ");
    if from == to {
        return None;
    }
    // A one-word fix of a very common word would rewrite every sentence.
    if !from.contains(' ') && (from.chars().count() <= 3 || crate::cleanup::is_common_word(&from)) && !from.eq_ignore_ascii_case(&to) {
        return None;
    }
    Some((from, to))
}

pub fn add_correction(memory: &mut Memory, from: &str, to: &str, now: u64) {
    memory.corrections.retain(|c| !c.from.eq_ignore_ascii_case(from));
    memory.corrections.insert(0, Correction { from: from.into(), to: to.into(), created_unix_ms: now });
    memory.corrections.truncate(MAX_CORRECTIONS);
}

/// Keep a short recent sentence as a style sample.
pub fn add_style(memory: &mut Memory, text: &str) {
    let text = text.trim();
    let words = text.split_whitespace().count();
    if !(4..=40).contains(&words) || !text.is_ascii() {
        return;
    }
    memory.style.retain(|s| s != text);
    memory.style.insert(0, text.to_string());
    memory.style.truncate(MAX_STYLE);
}

pub fn forget_word(memory: &mut Memory, dictionary: &mut Vec<String>, word: &str) {
    dictionary.retain(|w| !w.eq_ignore_ascii_case(word));
    memory.pending.retain(|w| !w.eq_ignore_ascii_case(word));
    memory.my_words.retain(|w| !w.eq_ignore_ascii_case(word));
    memory.uses.retain(|w| !w.word.eq_ignore_ascii_case(word));
    if !memory.blocked.iter().any(|w| w.eq_ignore_ascii_case(word)) {
        memory.blocked.push(word.to_string());
        if memory.blocked.len() > MAX_BLOCKED {
            memory.blocked.remove(0);
        }
    }
}

/// Pin a word or short name to "My words" (newest first). Returns true when
/// it was added. Sentences, common English, and very long text are ignored
/// so junk never reaches the Whisper prompt. A repeat bumps its use count.
pub fn add_my_word(memory: &mut Memory, word: &str) -> bool {
    let word = word.trim().trim_matches(|c: char| !c.is_alphanumeric());
    if word.is_empty()
        || word.chars().count() > 40
        || word.split_whitespace().count() > 4
        || !word.chars().any(char::is_alphabetic)
    {
        return false;
    }
    if let Some(existing) = memory.my_words.iter().find(|w| w.eq_ignore_ascii_case(word)).cloned() {
        bump(memory, &existing);
        return false;
    }
    if !worth_prompt(word) {
        return false;
    }
    memory.blocked.retain(|w| !w.eq_ignore_ascii_case(word));
    memory.my_words.insert(0, word.to_string());
    memory.my_words.truncate(MAX_MY_WORDS);
    bump(memory, word);
    true
}

fn worth_prompt(word: &str) -> bool {
    if word.contains('.') || word.contains('@') {
        return true;
    }
    word.split_whitespace().any(|part| part.chars().count() >= 3 && !crate::cleanup::is_common_word(part))
}

/// Spoken this many times, the screen says "Used a lot". Fewer than that, "Ready".
pub const USED_A_LOT: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordRow {
    pub word: String,
    pub uses: u32,
    pub seed: bool,
    pub tag: &'static str,
}

/// Seeds plus words the user added, most-used first. A seed is never absent.
pub fn screen_words(memory: &Memory) -> Vec<WordRow> {
    let mut rows: Vec<(usize, String, bool)> = Vec::new();
    for (index, seed) in crate::vocab::SEED_WORDS.iter().enumerate() {
        rows.push((index, (*seed).to_string(), true));
    }
    for (index, word) in memory.my_words.iter().enumerate() {
        if rows.iter().any(|(_, have, _)| have.eq_ignore_ascii_case(word)) {
            continue;
        }
        rows.push((crate::vocab::SEED_WORDS.len() + index, word.clone(), false));
    }
    rows.sort_by(|a, b| use_of(memory, &b.1).cmp(&use_of(memory, &a.1)).then(a.0.cmp(&b.0)));
    rows.into_iter()
        .map(|(_, word, seed)| {
            let uses = use_of(memory, &word);
            WordRow {
                tag: if uses >= USED_A_LOT { "Used a lot" } else { "Ready" },
                word,
                uses,
                seed,
            }
        })
        .collect()
}

/// Drop a word the user added. Seeds stay, whatever the user clicks.
pub fn dismiss_user_word(memory: &mut Memory, dictionary: &mut Vec<String>, word: &str) -> Result<(), String> {
    let word = word.trim();
    if word.is_empty() {
        return Err("Type a word first.".into());
    }
    if crate::vocab::is_seed(word) {
        return Err("Bolo always spells that one.".into());
    }
    let known = memory.my_words.iter().any(|entry| entry.eq_ignore_ascii_case(word))
        || dictionary.iter().any(|entry| entry.eq_ignore_ascii_case(word));
    if !known {
        return Err("That word isn't in your list.".into());
    }
    forget_word(memory, dictionary, word);
    Ok(())
}

/// Most-used first. Equal counts keep list order (newest first).
pub fn rank_my_words(memory: &Memory) -> Vec<String> {
    let mut indexed: Vec<(usize, String)> = memory.my_words.iter().cloned().enumerate().collect();
    indexed.sort_by(|a, b| use_of(memory, &b.1).cmp(&use_of(memory, &a.1)).then(a.0.cmp(&b.0)));
    indexed.into_iter().map(|(_, word)| word).collect()
}

pub fn use_of(memory: &Memory, word: &str) -> u32 {
    memory.uses.iter().find(|stat| stat.word.eq_ignore_ascii_case(word)).map(|stat| stat.uses).unwrap_or(0)
}

/// Count My words that actually appeared in this take.
pub fn note_uses(memory: &mut Memory, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let mut heard: Vec<String> = memory.my_words.iter().filter(|word| contains_phrase(text, word)).cloned().collect();
    for seed in crate::vocab::SEED_WORDS {
        if contains_phrase(text, seed) && !heard.iter().any(|word| word.eq_ignore_ascii_case(seed)) {
            heard.push((*seed).to_string());
        }
    }
    for word in heard {
        bump(memory, &word);
    }
}

fn bump(memory: &mut Memory, word: &str) {
    if let Some(stat) = memory.uses.iter_mut().find(|stat| stat.word.eq_ignore_ascii_case(word)) {
        stat.uses = stat.uses.saturating_add(1);
        return;
    }
    memory.uses.push(WordUse { word: word.to_string(), uses: 1 });
    memory.uses.truncate(MAX_MY_WORDS);
}

fn contains_phrase(text: &str, phrase: &str) -> bool {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '@').to_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    let phrase: Vec<String> = phrase.split_whitespace().map(|word| word.to_lowercase()).collect();
    if phrase.is_empty() {
        return false;
    }
    if phrase.len() == 1 {
        return words.iter().any(|word| word == &phrase[0]);
    }
    words.windows(phrase.len()).any(|window| window == phrase.as_slice())
}

/// Fix last: save wrong->right immediately, pin the right spelling, and keep
/// the mishearing out of the prompt. No twice-seen wait.
pub fn record_fix(memory: &mut Memory, dictionary: &mut Vec<String>, from: &str, to: &str, now: u64) -> Vec<String> {
    add_correction(memory, from, to, now);
    for word in from.split_whitespace().map(bare) {
        if word.is_empty() || to.to_lowercase().contains(&word.to_lowercase()) {
            continue;
        }
        dictionary.retain(|entry| !entry.eq_ignore_ascii_case(&word));
        memory.pending.retain(|entry| !entry.eq_ignore_ascii_case(&word));
        memory.my_words.retain(|entry| !entry.eq_ignore_ascii_case(&word));
        memory.uses.retain(|entry| !entry.word.eq_ignore_ascii_case(&word));
        if crate::vocab::SEED_WORDS.iter().any(|seed| seed.eq_ignore_ascii_case(&word)) {
            continue;
        }
        if word.chars().count() >= 4
            && !crate::cleanup::is_common_word(&word)
            && !memory.blocked.iter().any(|entry| entry.eq_ignore_ascii_case(&word))
        {
            memory.blocked.push(word);
            if memory.blocked.len() > MAX_BLOCKED {
                memory.blocked.remove(0);
            }
        }
    }
    add_my_word(memory, to);
    learn_from_fix(memory, to, dictionary)
}

pub fn forget_everything(memory: &mut Memory, dictionary: &mut Vec<String>) {
    dictionary.clear();
    *memory = Memory::default();
}

/// Learn names from a take, with the pending list kept across restarts.
pub fn learn(memory: &mut Memory, text: &str, dictionary: &mut Vec<String>) -> Vec<String> {
    let blocked = memory.blocked.clone();
    let added = crate::cleanup::learn_terms_with(text, dictionary, &mut memory.pending, &blocked);
    memory.pending.truncate(MAX_PENDING);
    added
}

/// Words the user typed in a fix that should be spelled their way.
pub fn learn_from_fix(memory: &mut Memory, to: &str, dictionary: &mut Vec<String>) -> Vec<String> {
    let mut added = Vec::new();
    for word in to.split_whitespace().map(bare) {
        if word.chars().count() < 2 || word.chars().count() > 64 || dictionary.len() >= 200 {
            continue;
        }
        let shaped = word.chars().any(|c| c.is_uppercase() || c.is_ascii_digit());
        if !shaped || crate::cleanup::is_common_word(&word) {
            continue;
        }
        if dictionary.iter().any(|w| w.eq_ignore_ascii_case(&word)) {
            continue;
        }
        memory.blocked.retain(|w| !w.eq_ignore_ascii_case(&word));
        dictionary.push(word.clone());
        added.push(word);
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bolo-memory-{name}-{}-{}",
            std::process::id(),
            crate::store::now_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn my_words_take_short_fixes_and_forget_removes_them() {
        let mut memory = Memory::default();
        assert!(add_my_word(&mut memory, "Taimoor Hussain"));
        assert!(!add_my_word(&mut memory, "taimoor hussain"));
        assert!(!add_my_word(&mut memory, "this is a whole long sentence here"));
        assert!(!add_my_word(&mut memory, "42"));
        assert!(add_my_word(&mut memory, "Bolo.com."));
        assert_eq!(memory.my_words, vec!["Bolo.com", "Taimoor Hussain"]);
        let mut dictionary = vec!["Bolo.com".to_string()];
        forget_word(&mut memory, &mut dictionary, "bolo.com");
        assert_eq!(memory.my_words, vec!["Taimoor Hussain"]);
        let old: Memory = serde_json::from_str("{\"version\":1}").unwrap();
        assert!(old.my_words.is_empty());
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = temp("missing");
        let memory = load(&dir);
        assert_eq!(memory, Memory::default());
        assert_eq!(memory.version, VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_kept_aside_and_defaults_load() {
        let dir = temp("corrupt");
        std::fs::write(path(&dir), "{not json").unwrap();
        assert_eq!(load(&dir), Memory::default());
        assert!(dir.join("memory.bad.json").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_and_load_round_trip_and_old_version_upgrades() {
        let dir = temp("round");
        let mut memory = Memory::default();
        add_correction(&mut memory, "sequel", "SQL", 5);
        add_style(&mut memory, "Let's ship the build on Friday.");
        memory.pending.push("Taimoor".into());
        save(&dir, &memory).unwrap();
        assert_eq!(load(&dir), memory);
        std::fs::write(path(&dir), r#"{"version":0,"corrections":[{"from":"a b","to":"AB"},{"from":"","to":"x"}]}"#).unwrap();
        let old = load(&dir);
        assert_eq!(old.version, VERSION);
        assert_eq!(old.corrections.len(), 1);
        assert!(old.style.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrections_replace_whole_words_ignoring_case() {
        let fixes = vec![
            Correction { from: "sequel".into(), to: "SQL".into(), created_unix_ms: 0 },
            Correction { from: "my sequel".into(), to: "MySQL".into(), created_unix_ms: 0 },
            Correction { from: "kubernetes".into(), to: "Kubernetes".into(), created_unix_ms: 0 },
        ];
        assert_eq!(
            apply_corrections("Move my sequel and the Sequel table to kubernetes.", &fixes),
            "Move MySQL and the SQL table to Kubernetes."
        );
        assert_eq!(apply_corrections("sequels stay", &fixes), "sequels stay");
        assert_eq!(apply_corrections("", &fixes), "");
    }

    #[test]
    fn diff_finds_the_changed_words_only() {
        assert_eq!(
            diff_fix("Please ask tie more about it.", "Please ask Taimoor about it."),
            Some(("tie more".into(), "Taimoor".into()))
        );
        assert_eq!(diff_fix("same words", "same words"), None);
        assert_eq!(diff_fix("a b c d e f g h", "one two three four five six seven eight"), None);
        assert_eq!(diff_fix("I saw the cat", "I saw a cat"), None);
    }

    #[test]
    fn forget_blocks_relearning_and_fix_unblocks() {
        let mut memory = Memory::default();
        let mut dict = vec!["Kaju".to_string()];
        forget_word(&mut memory, &mut dict, "kaju");
        assert!(dict.is_empty());
        let added = learn(&mut memory, "We met Kaju today. Then Kaju came again.", &mut dict);
        assert!(added.is_empty(), "{added:?}");
        let added = learn_from_fix(&mut memory, "Kaju", &mut dict);
        assert_eq!(added, vec!["Kaju".to_string()]);
        assert!(memory.blocked.is_empty());
    }

    #[test]
    fn pending_names_survive_as_data_and_learn_on_second_take() {
        let mut memory = Memory::default();
        let mut dict = Vec::new();
        assert!(learn(&mut memory, "I spoke with Zubair yesterday.", &mut dict).is_empty());
        assert_eq!(memory.pending, vec!["Zubair".to_string()]);
        // A restart: memory comes back from disk.
        let json = serde_json::to_string(&memory).unwrap();
        let mut memory: Memory = serde_json::from_str(&json).unwrap();
        assert_eq!(learn(&mut memory, "Call Zubair again.", &mut dict), vec!["Zubair".to_string()]);
    }

    #[test]
    fn style_keeps_a_few_recent_plain_sentences() {
        let mut memory = Memory::default();
        add_style(&mut memory, "ok");
        add_style(&mut memory, "یہ اردو ہے اور لمبا جملہ ہے");
        assert!(memory.style.is_empty());
        for n in 0..8 {
            add_style(&mut memory, &format!("This is sentence number {n} today."));
        }
        assert_eq!(memory.style.len(), MAX_STYLE);
        assert!(memory.style[0].contains('7'));
    }

    #[test]
    fn forget_everything_clears_all() {
        let mut memory = Memory::default();
        add_correction(&mut memory, "x y", "XY", 1);
        add_style(&mut memory, "One two three four five.");
        let mut dict = vec!["GitHub".to_string()];
        forget_everything(&mut memory, &mut dict);
        assert!(dict.is_empty());
        assert_eq!(memory, Memory::default());
    }

    #[test]
    fn prompt_terms_are_newest_first_and_unique() {
        let mut memory = Memory::default();
        add_correction(&mut memory, "a", "Alpha", 1);
        add_correction(&mut memory, "b", "Beta", 2);
        add_correction(&mut memory, "c", "alpha", 3);
        assert_eq!(prompt_terms(&memory), vec!["alpha".to_string(), "Beta".to_string()]);
    }

    #[test]
    fn my_words_screen_orders_by_use_and_will_not_drop_a_seed() {
        let mut memory = Memory::default();
        let mut dictionary = Vec::new();
        assert!(add_my_word(&mut memory, "Priya"));
        assert!(add_my_word(&mut memory, "Acme Studio"));
        memory.uses = vec![
            WordUse { word: "Priya".into(), uses: 9 },
            WordUse { word: "Bolo".into(), uses: USED_A_LOT },
            WordUse { word: "Acme Studio".into(), uses: 1 },
        ];
        let rows = screen_words(&memory);
        assert_eq!(rows[0].word, "Priya");
        assert_eq!(rows[0].tag, "Used a lot");
        assert!(!rows[0].seed);
        let bolo = rows.iter().find(|row| row.word == "Bolo").expect("seed");
        assert!(bolo.seed);
        assert_eq!(bolo.tag, "Used a lot");
        let priya = rows.iter().position(|row| row.word == "Priya").unwrap();
        let bolo_at = rows.iter().position(|row| row.word == "Bolo").unwrap();
        let kingdom = rows.iter().position(|row| row.word == "Kingdom Vision").unwrap();
        assert!(priya < bolo_at && bolo_at < kingdom);
        assert!(rows.iter().any(|row| row.word == "Acme Studio" && row.tag == "Ready" && !row.seed));
        for seed in crate::vocab::SEED_WORDS {
            assert!(rows.iter().any(|row| row.word == *seed && row.seed), "{seed}");
        }

        dismiss_user_word(&mut memory, &mut dictionary, "Acme Studio").unwrap();
        assert!(!screen_words(&memory).iter().any(|row| row.word == "Acme Studio"));
        let err = dismiss_user_word(&mut memory, &mut dictionary, "Bolo").unwrap_err();
        assert!(err.contains("always spells"));
        assert!(screen_words(&memory).iter().any(|row| row.word == "Bolo" && row.seed));
        assert!(dismiss_user_word(&mut memory, &mut dictionary, "Priya").is_ok());
        assert!(!screen_words(&memory).iter().any(|row| row.word == "Priya"));

        assert!(add_my_word(&mut memory, "Northwind"));
        let again = screen_words(&memory);
        assert!(again.iter().any(|row| row.word == "Northwind" && !row.seed));
        assert!(again.len() >= crate::vocab::SEED_WORDS.len());
    }
}
