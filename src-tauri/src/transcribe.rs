//! Groq Whisper transcription. Audio is sent only to api.groq.com.

use anyhow::Context;
use serde::Deserialize;
use std::sync::OnceLock;
use std::time::Duration;

pub const WHISPER_TURBO: &str = "whisper-large-v3-turbo";
pub const WHISPER_LARGE: &str = "whisper-large-v3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// HTTP 429 with a short retry-after. Retry the same key, then maybe on-device.
    RateLimit,
    /// Daily / tokens-per-day limit (or a long retry-after). Cool the key down.
    QuotaExhausted,
    /// Network failure or HTTP 5xx. A few backoffs, then the on-device model.
    /// Never mark the key exhausted for this.
    Unavailable,
    /// The key was rejected. Retry once, then the on-device model may take the chunk.
    Rejected,
    Other,
}

#[derive(Debug)]
pub struct GroqFailure {
    pub message: String,
    pub kind: FailKind,
    /// True when the body named a daily / TPD / quota limit explicitly.
    pub explicit_quota: bool,
    /// Parsed retry-after from the provider message, when present.
    pub retry_after_ms: Option<u64>,
    /// Transport wording that points at local offline (DNS and similar).
    pub offline_hint: bool,
}

impl std::fmt::Display for GroqFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for GroqFailure {}

pub fn failure_kind(err: &anyhow::Error) -> Option<FailKind> {
    err.downcast_ref::<GroqFailure>().map(|failure| failure.kind)
}

pub fn failure_details(err: &anyhow::Error) -> Option<&GroqFailure> {
    err.downcast_ref::<GroqFailure>()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPlan {
    /// Wait, then try the same request again.
    Backoff { ms: u64 },
    /// Stop retrying. `fallback` means whisper.cpp should take this chunk.
    Stop { fallback: bool },
}

/// What the overlay may say when Groq rejects the key. No settings, no
/// Windows, no "API key" — Bolo retries and falls back on its own.
pub const REJECTED_KEY_MESSAGE: &str =
    "Dictation had a brief problem. Bolo will try again on its own.";

/// `attempt` is how many failures have already been seen, starting at 0.
pub fn retry_plan(kind: FailKind, attempt: u32) -> RetryPlan {
    match kind {
        // A single rejection is often a blip. Retry once, then let the
        // on-device model take the chunk instead of stopping the take.
        FailKind::Rejected => {
            if attempt == 0 {
                RetryPlan::Backoff { ms: 400 }
            } else {
                RetryPlan::Stop { fallback: true }
            }
        }
        FailKind::Other => RetryPlan::Stop { fallback: false },
        // Daily quota: stop this key immediately and let the provider router
        // switch or fall back. Do not burn retries on a spent key.
        FailKind::QuotaExhausted => RetryPlan::Stop { fallback: true },
        FailKind::RateLimit => {
            if attempt == 0 {
                RetryPlan::Backoff { ms: 700 }
            } else {
                RetryPlan::Stop { fallback: true }
            }
        }
        FailKind::Unavailable => {
            if attempt < 2 {
                RetryPlan::Backoff { ms: 250 * 3u64.pow(attempt) }
            } else {
                RetryPlan::Stop { fallback: true }
            }
        }
    }
}

pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(4)
            .tcp_keepalive(Duration::from_secs(30))
            .http2_keep_alive_interval(Duration::from_secs(20))
            .http2_keep_alive_timeout(Duration::from_secs(10))
            .http2_keep_alive_while_idle(true)
            .user_agent(concat!("Bolo/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client")
    })
}

/// Open a connection to Groq before the first take so the upload does not pay
/// for TCP and TLS. The response body is irrelevant and the key is not sent.
pub async fn prewarm(log_dir: &std::path::Path) {
    // One retry: after resume or a network change the first attempt often
    // fails instantly (DNS or Wi-Fi not back yet) and left the next take cold.
    for attempt in 1..=PREWARM_TRIES {
        let started = std::time::Instant::now();
        let result = http_client().get("https://api.groq.com/").send().await;
        let ms = started.elapsed().as_millis();
        match result {
            Ok(response) => {
                crate::logbuf::log_line(
                    log_dir,
                    "info",
                    &format!("groq prewarm ms={ms} attempt={attempt} status={}", response.status().as_u16()),
                );
                return;
            }
            Err(err) => {
                crate::logbuf::log_line(
                    log_dir,
                    if attempt < PREWARM_TRIES { "info" } else { "warn" },
                    &format!("groq prewarm ms={ms} attempt={attempt} error={}", error_chain(&err)),
                );
                if attempt < PREWARM_TRIES {
                    let _ = tauri::async_runtime::spawn_blocking(|| std::thread::sleep(PREWARM_RETRY_DELAY)).await;
                }
            }
        }
    }
}

pub(crate) const PREWARM_TRIES: u32 = 2;
pub(crate) const PREWARM_RETRY_DELAY: Duration = Duration::from_secs(3);

/// An error with its causes. reqwest's own text is only "error sending
/// request for url (...)", which hid whether DNS, connect, or TLS failed.
pub fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !text.is_empty() && !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = cause.source();
    }
    out
}

pub fn normalize_model(model: &str) -> Option<&'static str> {
    match model {
        WHISPER_TURBO => Some(WHISPER_TURBO),
        WHISPER_LARGE => Some(WHISPER_LARGE),
        _ => None,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn whisper_prompt(words: &[String]) -> Option<String> {
    whisper_prompt_with(words, &[])
}

/// Whisper follows the prompt's language. The normal prompt is English
/// (instructions, English terms, recent English takes), which pulled Urdu
/// speech into English look-alike words ("behtari" -> "battery"). Urdu takes
/// get an Urdu-script prompt instead.
pub(crate) const URDU_PROMPT: &str = "یہ اردو میں بولی گئی بات ہے۔ بہتری، بہتر، ایپلیکیشن، اینیمیشن، بیک گراؤنڈ، ویب سائٹ، ڈیزائن۔";

/// Hindi takes that the user asked to keep use a Devanagari prompt. An English
/// or Urdu prompt pulls the same speech into Latin or Nastaliq.
pub(crate) const HINDI_PROMPT: &str = "यह हिंदी में कही गई बात है। आज। कल। ठीक है।";

/// Under this trimmed length, a vocabulary prompt and the previous take are
/// more influential than the audio. Whisper then starts the paste with a
/// prompt word ("Bolo and Easy…") or continues the last sentence.
pub const BRIEF_AUDIO_SECS: f32 = 2.5;

/// Character budget for a normal take's glossary. Whisper reads the prompt as
/// the words spoken just before this clip (about 224 tokens). An 800-character
/// instruction plus the previous paste filled that window, so everyday speech
/// was continued from the prompt instead of heard from the mic.
pub const PROMPT_TERM_BUDGET: usize = 280;

pub fn brief_audio(speech_secs: f32) -> bool {
    speech_secs > 0.0 && speech_secs < BRIEF_AUDIO_SECS
}

/// Short clips get a closed glossary and nothing else. No instruction
/// sentence (it gets echoed), no proper names (they get pasted), no recent
/// transcript (it gets continued).
pub fn whisper_prompt_brief() -> String {
    "EasySSH, SSH, API, HTTP, HTTPS, JSON, URL, SQL.".to_string()
}

/// Rare spellings for a normal-length take.
///
/// The prompt is previous-token context, not an instruction. A sentence such as
/// "Spell these terms exactly…" and the last paste are continued as if the
/// person were still saying them, which is how ordinary speech picked up
/// EasySSH, SSH, and the previous sentence. This list is names and odd
/// spellings only. `recent` contributes proper nouns from the last takes
/// (Priya), never the sentences themselves. Technical acronyms stay on
/// `whisper_prompt_brief`, which is the whole prompt for a short clip.
pub fn whisper_prompt_with(words: &[String], recent: &[String]) -> Option<String> {
    let mut merged = context_terms(recent);
    for word in words {
        let trimmed = word.trim();
        if trimmed.is_empty() || merged.iter().any(|have| have.eq_ignore_ascii_case(trimmed)) {
            continue;
        }
        merged.push(trimmed.to_string());
    }
    if merged.is_empty() {
        return None;
    }
    let terms = select_terms(&merged, PROMPT_TERM_BUDGET);
    if terms.is_empty() {
        return None;
    }
    Some(format!("{}.", terms.join(", ")))
}

/// Capitalized names and addresses from recent pastes. The first word of a
/// sentence is skipped: it is capitalized because it starts the sentence.
fn context_terms(recent: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for text in recent.iter().take(2) {
        let mut boundary = true;
        for token in text.split_whitespace() {
            let ends = token.ends_with(['.', '?', '!', '۔']);
            let mut bare: String = token.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '@').collect();
            if ends {
                bare = bare.trim_end_matches(['.', '?', '!']).to_string();
            }
            if !boundary && name_like(&bare) && !out.iter().any(|have: &String| have.eq_ignore_ascii_case(&bare)) {
                out.push(bare);
            }
            boundary = ends;
            if out.len() >= 6 {
                return out;
            }
        }
    }
    out
}

fn name_like(bare: &str) -> bool {
    if bare.contains('.') || bare.contains('@') {
        return bare.chars().any(|c| c.is_ascii_alphabetic());
    }
    let chars: Vec<char> = bare.chars().collect();
    if chars.len() < 3 || chars.len() > 32 {
        return false;
    }
    let head = chars[0].is_uppercase();
    let tail_lower = chars.iter().skip(1).any(|c| c.is_lowercase());
    let inner_cap = chars.iter().skip(1).any(|c| c.is_uppercase());
    let acronym = chars.len() <= 6 && chars.iter().all(|c| c.is_ascii_uppercase());
    (head && tail_lower) || inner_cap || acronym
}

/// My words (already most-used first) stay ahead of seeds, and seeds keep a
/// reserved slice so a long dictionary cannot push Bolo out of the prompt.
///
/// `prompt_words` emits ranked My words, then the built-in seeds, then the
/// dictionary. A My word can itself be a seed ("Bolo"), so the seed block is
/// the last contiguous run of seeds, not the first seed we see.
fn select_terms(words: &[String], budget: usize) -> Vec<String> {
    let cleaned: Vec<String> = words.iter().map(|word| word.trim().to_string()).filter(|word| !word.is_empty()).collect();
    let is_seed = |term: &str| crate::vocab::SEED_WORDS.iter().any(|seed| seed.eq_ignore_ascii_case(term));
    let mut last_run: Option<(usize, usize)> = None;
    let mut index = 0;
    while index < cleaned.len() {
        if is_seed(&cleaned[index]) {
            let start = index;
            while index < cleaned.len() && is_seed(&cleaned[index]) {
                index += 1;
            }
            last_run = Some((start, index));
        } else {
            index += 1;
        }
    }
    let (leading, seeds, rest): (Vec<String>, Vec<String>, Vec<String>) = match last_run {
        Some((start, end)) => (cleaned[..start].to_vec(), cleaned[start..end].to_vec(), cleaned[end..].to_vec()),
        None => (cleaned, Vec::new(), Vec::new()),
    };
    let mut budget = budget;
    let seed_cost: usize = seeds.iter().map(|term| term.chars().count() + 2).sum();
    let reserve = if seeds.is_empty() { 0 } else { seed_cost.min(budget / 2) };
    let mut hot = budget.saturating_sub(reserve);
    let mut chosen = Vec::new();
    for term in &leading {
        let cost = term.chars().count() + 2;
        if cost > hot {
            continue;
        }
        hot -= cost;
        budget = budget.saturating_sub(cost);
        chosen.push(term.clone());
    }
    for term in seeds.iter().chain(rest.iter()) {
        let cost = term.chars().count() + 2;
        if cost > budget {
            continue;
        }
        budget = budget.saturating_sub(cost);
        chosen.push(term.clone());
    }
    chosen
}

/// Words that make "it's such a …" a real sentence rather than a mis-hearing of SSH.
const REAL_SUCH: &[&str] = &[
    "amazing", "awful", "bad", "beautiful", "big", "blessing", "clear", "cold", "common", "cool", "deal", "fun", "good",
    "great", "happy", "hard", "honor", "honour", "hot", "huge", "idea", "important", "joy", "long", "lovely", "mess",
    "nice", "nightmare", "obvious", "pain", "perfect", "pity", "pleasure", "problem", "rare", "relief", "sad", "shame",
    "shock", "short", "simple", "small", "strange", "strong", "surprise", "sweet", "terrible", "tiny", "waste", "weak",
    "weird", "wonderful",
];

const LIGHT_GLUE: &[&str] = &["and", "so", "um", "uh", "umm", "well", "hey", "ok", "okay", "yeah"];

const FINITE_VERBS: &[&str] = &[
    "is", "are", "was", "were", "be", "been", "am", "have", "has", "had", "do", "does", "did", "can", "could", "will",
    "would", "should", "shall", "may", "might", "go", "goes", "went", "going", "want", "wants", "need", "needs", "think",
    "thinks", "said", "say", "says", "make", "makes", "made", "get", "gets", "got", "ship", "send", "open", "start",
    "let", "lets", "please", "use", "uses", "run", "runs", "deploy", "check", "see", "look", "remember", "know", "note",
    "keep", "hold", "try", "tell", "ask", "call", "put", "take", "give", "come", "mean", "love", "like", "hope", "find",
    "leave", "stay", "stop", "wait", "help", "show", "move", "turn", "read", "write", "set", "add", "change", "fix",
    "work", "works",
];

/// Whisper hears "easy ssh" as ordinary English and then keeps talking:
/// "easy, it's such are", "Bolo and Easy, it's such a truth. Nerd thing, imagination."
/// Restore the technical phrase when the take is that failure, and leave a real
/// sentence ("it's such a good idea", "open easy ssh and log in") alone.
pub fn recover_brief_tech(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(collapsed) = collapse_tech_hallucination(trimmed) {
        return collapsed;
    }
    replace_tech_literals(trimmed)
}

/// Drop a runaway tail when the trimmed audio is too short to have held those words.
pub fn guard_short_take(text: &str, speech_secs: f32) -> String {
    let recovered = recover_brief_tech(text);
    clamp_runaway(&recovered, speech_secs)
}

fn clamp_runaway(text: &str, speech_secs: f32) -> String {
    if !(speech_secs > 0.05 && speech_secs < 6.0) {
        return text.to_string();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    // Conversational speech is about 2–4 words a second. Far above that, the
    // extra words are the model continuing after the person stopped.
    let max_words = ((speech_secs * 4.5) + 3.0).round() as usize;
    let max_words = max_words.max(4);
    if words.len() <= max_words {
        return text.to_string();
    }
    words[..max_words].join(" ")
}

fn collapse_tech_hallucination(text: &str) -> Option<String> {
    let cores = tech_cores(text);
    if cores.is_empty() || cores.len() > 16 {
        return None;
    }
    if let Some(easy_at) = cores.iter().position(|word| word == "easy") {
        let prefix = &cores[..easy_at];
        if prefix.iter().all(|word| is_seed_glue(word) || is_light_glue(word)) {
            if let Some(span) = ssh_mishear_len(&cores[easy_at + 1..]) {
                let tail = &cores[easy_at + 1 + span..];
                if collapse_tail(text, prefix, tail) {
                    return Some("EasySSH".to_string());
                }
            }
        }
    }
    if let Some(span) = ssh_mishear_len(&cores) {
        let tail = &cores[span..];
        if collapse_tail(text, &[], tail) {
            let product = matches!(cores.first().map(String::as_str), Some("easyssh" | "easy-ssh"));
            return Some(if product { "EasySSH" } else { "SSH" }.to_string());
        }
    }
    None
}

fn collapse_tail(original: &str, prefix: &[String], tail: &[String]) -> bool {
    if real_such_clause(tail) {
        return false;
    }
    let leaked = prefix.iter().any(|word| is_seed_glue(word));
    leaked || broken_tail(tail) || extra_sentence_nonsense(original)
}

fn real_such_clause(tail: &[String]) -> bool {
    let rest = match tail.first().map(String::as_str) {
        Some("a" | "an" | "the") => &tail[1..],
        _ => return false,
    };
    rest.first().is_some_and(|word| REAL_SUCH.contains(&word.as_str()))
}

fn broken_tail(tail: &[String]) -> bool {
    let words: Vec<&str> = tail.iter().map(String::as_str).collect();
    matches!(
        words.as_slice(),
        []
            | ["are"]
            | ["a"]
            | ["an"]
            | ["the"]
            | ["truth"]
            | ["thing"]
            | ["a", "truth"]
            | ["a", "thing"]
            | ["the", "truth"]
            | ["the", "thing"]
    )
}

/// A second fragment with no verb ("Nerd thing, imagination.") after the mis-hearing.
fn extra_sentence_nonsense(text: &str) -> bool {
    let parts: Vec<&str> = text.split(['.', '?', '!']).filter(|part| !part.trim().is_empty()).collect();
    if parts.len() < 2 {
        return false;
    }
    let extra = tech_cores(&parts[1..].join(" "));
    !extra.is_empty() && extra.len() <= 8 && !extra.iter().any(|word| FINITE_VERBS.contains(&word.as_str()))
}

fn ssh_mishear_len(cores: &[String]) -> Option<usize> {
    let c = |i: usize| cores.get(i).map(String::as_str);
    if c(0) == Some("ssh") || c(0) == Some("easyssh") || c(0) == Some("easy-ssh") {
        return Some(1);
    }
    if matches!(c(0), Some("s" | "es" | "ess")) && matches!(c(1), Some("s" | "es" | "ess")) && matches!(c(2), Some("h" | "aitch")) {
        return Some(3);
    }
    if matches!(c(0), Some("it's" | "its" | "it")) && c(1) == Some("such") {
        return Some(2);
    }
    None
}

fn is_seed_glue(word: &str) -> bool {
    let flat: String = word.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    matches!(
        flat.as_str(),
        "bolo"
            | "bolocom"
            | "kingdom"
            | "vision"
            | "taimoor"
            | "hussain"
            | "taimoorhussain"
            | "taimoorhussaincom"
            | "thebetawebsite"
            | "thebetawebsitecom"
    )
}

fn is_light_glue(word: &str) -> bool {
    LIGHT_GLUE.contains(&word)
}

fn replace_tech_literals(text: &str) -> String {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return text.to_string();
    }
    let cores: Vec<String> = tokens.iter().copied().map(core_word).collect();
    let mut out = Vec::new();
    let mut index = 0;
    let mut changed = false;
    while index < tokens.len() {
        let Some(span) = literal_tech_span(&cores[index..]) else {
            out.push(tokens[index].to_string());
            index += 1;
            continue;
        };
        let product = literal_includes_easy(&cores[index..index + span]);
        let tail = trailing_punct(tokens[index + span - 1]);
        out.push(format!("{}{tail}", if product { "EasySSH" } else { "SSH" }));
        index += span;
        changed = true;
    }
    if !changed {
        text.to_string()
    } else {
        out.join(" ")
    }
}

/// Joining spans only. "it's such" is handled by the utterance collapse so a
/// real sentence ("it's such a good idea") is not rewritten.
fn literal_tech_span(cores: &[String]) -> Option<usize> {
    let c = |i: usize| cores.get(i).map(String::as_str);
    if matches!(c(0), Some("easyssh" | "easy-ssh")) {
        return Some(1);
    }
    if c(0) == Some("easy") && c(1) == Some("ssh") {
        return Some(2);
    }
    if c(0) == Some("easy")
        && matches!(c(1), Some("s" | "es" | "ess"))
        && matches!(c(2), Some("s" | "es" | "ess"))
        && matches!(c(3), Some("h" | "aitch"))
    {
        return Some(4);
    }
    if matches!(c(0), Some("s" | "es" | "ess")) && matches!(c(1), Some("s" | "es" | "ess")) && matches!(c(2), Some("h" | "aitch")) {
        return Some(3);
    }
    None
}

fn literal_includes_easy(cores: &[String]) -> bool {
    cores.iter().any(|word| word == "easy" || word == "easyssh" || word == "easy-ssh")
}

fn trailing_punct(token: &str) -> String {
    token
        .chars()
        .rev()
        .take_while(|c| !c.is_alphanumeric() && *c != '\'')
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

fn tech_cores(text: &str) -> Vec<String> {
    text.split_whitespace().map(core_word).filter(|word| !word.is_empty()).collect()
}

fn core_word(token: &str) -> String {
    let trimmed = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}' && c != '-');
    trimmed.replace(['\u{2019}', '\u{2018}'], "'").to_ascii_lowercase()
}

/// Drop Whisper prompt echoes that sometimes prepend the pasted transcript.
pub(crate) fn strip_prompt_leak(text: &str) -> String {
    let mut out = text.trim().to_string();
    loop {
        let before = out.clone();
        let trimmed = out.trim_start();
        let next = if let Some(rest) = strip_known_prefix(trimmed) {
            rest
        } else if let Some(rest) = strip_term_instruction(trimmed) {
            rest
        } else if let Some(rest) = strip_glossary_echo(trimmed) {
            rest
        } else {
            break;
        };
        out = next
            .trim_start()
            .trim_matches(|c| c == '"' || c == '\'' || c == '“' || c == '”')
            .trim()
            .to_string();
        if out == before {
            break;
        }
    }
    out
}

fn strip_known_prefix(text: &str) -> Option<String> {
    const PREFIXES: &[&str] = &[
        "Recent dictation:",
        "Dictation transcript.",
        "Dictation transcript:",
    ];
    for prefix in PREFIXES {
        if let Some(rest) = text.strip_prefix(prefix) {
            return Some(rest.to_string());
        }
    }
    None
}

/// The short-clip glossary, echoed back as the transcript.
fn strip_glossary_echo(text: &str) -> Option<String> {
    const MARKERS: &[&str] = &[
        "EasySSH, SSH, API, HTTP, HTTPS, JSON, URL, SQL.",
        "EasySSH, SSH, API, HTTP, HTTPS, JSON, URL, SQL",
    ];
    for marker in MARKERS {
        if let Some(rest) = text.strip_prefix(marker) {
            return Some(rest.to_string());
        }
    }
    None
}

/// "Spell these terms exactly when spoken: Bolo, Tauri." is the prompt, not speech.
/// Drop that sentence, including the term list, when Whisper echoes it.
fn strip_term_instruction(text: &str) -> Option<String> {
    const MARKER: &str = "Spell these terms exactly when spoken:";
    let rest = text.strip_prefix(MARKER)?;
    match rest.find('.') {
        Some(index) => Some(rest[index + 1..].to_string()),
        None => Some(String::new()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    pub tries: u8,
    /// ISO-ish code from verbose_json (`en`, `ur`), or empty when the body omitted it.
    pub language: String,
    /// Whisper's own confidence was low (avg_logprob / no_speech_prob).
    pub doubtful: bool,
}

/// Worth offering Fix last. A low logprob is also retried; no_speech on its
/// own only raises the card, unless the take is a couple of words.
pub fn doubtful(text: &str, avg_logprob: Option<f32>, no_speech_prob: Option<f32>) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    avg_logprob.is_some_and(|value| value < -0.7) || no_speech_prob.is_some_and(|value| value > 0.5)
}

/// First decode is greedy (temperature 0). A mumbled or accented take often
/// sticks to the wrong word there. The retry samples a little (Whisper's own
/// fallback schedule starts at 0.2) and drops the prompt so it is not the same request.
pub const RETRY_TEMPERATURE: f32 = 0.2;
/// "Please try again in 3.2s" / "in 850ms" from a Groq 429 message.
pub fn retry_after_ms(message: &str) -> Option<u64> {
    let at = message.find("try again in ")? + "try again in ".len();
    // Groq writes compound waits such as "1m30s" or "7m12.5s".
    let mut rest = &message[at..];
    let mut total = 0f64;
    let mut parsed = false;
    loop {
        let num: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let Ok(value) = num.parse::<f64>() else { break };
        let unit = &rest[num.len()..];
        let (scale, len) = if unit.starts_with("ms") {
            (1.0, 2)
        } else if unit.starts_with('s') {
            (1000.0, 1)
        } else if unit.starts_with('m') {
            (60_000.0, 1)
        } else if unit.starts_with('h') {
            (3_600_000.0, 1)
        } else {
            break;
        };
        total += value * scale;
        parsed = true;
        rest = &unit[len..];
    }
    parsed.then_some(total as u64)
}

/// Phrases Whisper invents on silence or noise.
pub fn stock_phrase(text: &str) -> bool {
    let t: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    matches!(
        t.as_str(),
        "thank you" | "thanks" | "thank you very much" | "thanks for watching" | "thank you for watching"
            | "you" | "bye" | "bye bye" | "please subscribe" | "subtitles by the amaraorg community" | "شکریہ"
    )
}

pub fn should_retry(
    text: &str,
    avg_logprob: Option<f32>,
    no_speech_prob: Option<f32>,
    compression_ratio: Option<f32>,
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    // OpenAI's decoder treats compression above 2.4 as a loop, and logprob
    // below -0.7 as a decode it would not keep. Both used to be pasted.
    if compression_ratio.is_some_and(|value| value > 2.4) {
        return true;
    }
    if avg_logprob.is_some_and(|value| value < -0.7) {
        return true;
    }
    let words = trimmed.split_whitespace().count();
    no_speech_prob.is_some_and(|value| value > 0.7) && words < 3
}

pub async fn transcribe(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    model: &str,
    language: &str,
    prompt: Option<String>,
    using_builtin: bool,
) -> anyhow::Result<Transcript> {
    let model = normalize_model(model).context("unknown Whisper model")?;
    let first = request_audio(
        client,
        api_key,
        audio.clone(),
        TRANSCRIPTIONS,
        model,
        language,
        prompt.as_deref(),
        0.0,
        using_builtin,
    )
    .await?;
    let mut first = first;
    if stock_phrase(&first.text) && first.no_speech_prob.unwrap_or(0.0) > 0.25 {
        // "Thank you." / "Thanks for watching." on a take that is mostly noise.
        first.text.clear();
    }
    if !should_retry(&first.text, first.avg_logprob, first.no_speech_prob, first.compression_ratio) {
        let doubt = doubtful(&first.text, first.avg_logprob, first.no_speech_prob);
        return Ok(Transcript {
            text: first.text,
            tries: 1,
            language: first.language,
            doubtful: doubt,
        });
    }
    crate::diag::note(format!(
        "whisper retry temp={RETRY_TEMPERATURE} log={:?} compression={:?} no_speech={:?}",
        first.avg_logprob, first.compression_ratio, first.no_speech_prob
    ));
    match request_audio(client, api_key, audio, TRANSCRIPTIONS, model, language, None, RETRY_TEMPERATURE, using_builtin).await {
        Ok(second) => {
            let chosen = prefer_attempt(first, second);
            let doubt = doubtful(&chosen.text, chosen.avg_logprob, chosen.no_speech_prob);
            Ok(Transcript {
                text: chosen.text,
                tries: 2,
                language: chosen.language,
                doubtful: doubt,
            })
        }
        Err(_) if !first.text.trim().is_empty() => Ok(Transcript {
            text: first.text,
            tries: 2,
            language: first.language,
            // The first try already looked bad enough to retry.
            doubtful: true,
        }),
        Err(err) => Err(err),
    }
}

/// Groq translates with `whisper-large-v3` only. Turbo does not translate.
pub async fn translate_audio(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    prompt: Option<&str>,
    using_builtin: bool,
) -> anyhow::Result<Transcript> {
    let first = request_audio(client, api_key, audio.clone(), TRANSLATIONS, WHISPER_LARGE, "", prompt, 0.0, using_builtin).await?;
    if !first.text.trim().is_empty() && !repetitive(&first.text) {
        let doubt = doubtful(&first.text, first.avg_logprob, first.no_speech_prob);
        return Ok(Transcript {
            text: first.text,
            tries: 1,
            language: first.language,
            doubtful: doubt,
        });
    }
    let second = request_audio(client, api_key, audio, TRANSLATIONS, WHISPER_LARGE, "", None, RETRY_TEMPERATURE, using_builtin).await?;
    let language = if second.language.is_empty() {
        first.language
    } else {
        second.language
    };
    let doubt = doubtful(&second.text, second.avg_logprob, second.no_speech_prob);
    Ok(Transcript {
        text: second.text,
        tries: 2,
        language,
        doubtful: doubt,
    })
}

const TRANSCRIPTIONS: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const TRANSLATIONS: &str = "https://api.groq.com/openai/v1/audio/translations";

fn prefer_attempt(first: Attempt, second: Attempt) -> Attempt {
    let primary = first.text.trim();
    let retry = second.text.trim();
    let pick_second = if retry.is_empty() {
        false
    } else if primary.is_empty() || (stock_phrase(primary) && !stock_phrase(retry)) {
        true
    } else if stock_phrase(retry) && !stock_phrase(primary) {
        false
    } else {
        let first_bad = should_retry(primary, first.avg_logprob, first.no_speech_prob, first.compression_ratio);
        let second_bad = should_retry(retry, second.avg_logprob, second.no_speech_prob, second.compression_ratio);
        if first_bad != second_bad {
            first_bad
        } else if first_bad && second_bad && retry.chars().count() > primary.chars().count() * 3 / 2 + 8 {
            // Both weak, and the retry kept talking. Length used to win here,
            // which pasted the hallucination.
            matches!((first.avg_logprob, second.avg_logprob), (Some(a), Some(b)) if b > a + 0.25)
        } else {
            matches!((first.avg_logprob, second.avg_logprob), (Some(a), Some(b)) if b > a + 0.05)
        }
    };
    let fallback_language = if first.language.is_empty() {
        second.language.clone()
    } else {
        first.language.clone()
    };
    let mut chosen = if pick_second { second } else { first };
    if chosen.language.is_empty() {
        chosen.language = fallback_language;
    }
    chosen.text = strip_prompt_leak(&chosen.text);
    chosen
}

struct Attempt {
    text: String,
    avg_logprob: Option<f32>,
    no_speech_prob: Option<f32>,
    compression_ratio: Option<f32>,
    language: String,
}

async fn request_audio(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    endpoint: &str,
    model: &str,
    language: &str,
    prompt: Option<&str>,
    temperature: f32,
    using_builtin: bool,
) -> anyhow::Result<Attempt> {
    let mut attempt = 0u32;
    loop {
        match request_audio_once(client, api_key, audio.clone(), endpoint, model, language, prompt, temperature, using_builtin).await {
            Ok(parsed) => return Ok(parsed),
            Err(failure) if failure.kind == FailKind::RateLimit && attempt < 3 => {
                // Free Groq keys allow ~20 requests a minute. Groq says how long
                // to wait; a few seconds is better than dropping the take.
                match retry_after_ms(&failure.message) {
                    Some(ms) if ms <= 8_000 => {
                        std::thread::sleep(Duration::from_millis(ms + 150));
                        attempt += 1;
                    }
                    _ => match retry_plan(failure.kind, attempt.min(1)) {
                        RetryPlan::Backoff { ms } => {
                            std::thread::sleep(Duration::from_millis(ms));
                            attempt += 1;
                        }
                        RetryPlan::Stop { .. } => return Err(anyhow::Error::from(failure)),
                    },
                }
            }
            Err(failure) => match retry_plan(failure.kind, attempt) {
                RetryPlan::Backoff { ms } => {
                    std::thread::sleep(Duration::from_millis(ms));
                    attempt += 1;
                }
                RetryPlan::Stop { .. } => return Err(anyhow::Error::from(failure)),
            },
        }
    }
}

async fn request_audio_once(
    client: &reqwest::Client,
    api_key: &str,
    audio: Vec<u8>,
    endpoint: &str,
    model: &str,
    language: &str,
    prompt: Option<&str>,
    temperature: f32,
    using_builtin: bool,
) -> Result<Attempt, GroqFailure> {
    let translating = endpoint.ends_with("/translations");
    let model = if translating { WHISPER_LARGE } else { model };
    let part = match audio_part(audio) {
        Ok(part) => part,
        Err(err) => {
            return Err(GroqFailure {
                message: err.to_string(),
                kind: FailKind::Other,
                explicit_quota: false,
                retry_after_ms: None,
                offline_hint: false,
            })
        }
    };
    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string())
        .text("response_format", "verbose_json")
        .text("temperature", temperature_field(temperature));
    if !translating {
        if let Some(code) = normalize_language(language) {
            form = form.text("language", code);
        }
    }
    if let Some(prompt) = prompt {
        form = form.text("prompt", prompt.to_string());
    }
    let response = match client
        .post(endpoint)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            return Err(crate::provider::network_to_failure(error_chain(&err)))
        }
    };
    let status = response.status();
    let body = match response.text().await {
        Ok(body) => body,
        Err(err) => {
            return Err(GroqFailure {
                message: format!("empty transcription response: {err}"),
                kind: if status.is_server_error() {
                    FailKind::Unavailable
                } else {
                    FailKind::Other
                },
                explicit_quota: false,
                retry_after_ms: None,
                offline_hint: false,
            })
        }
    };
    if !status.is_success() {
        let code = status.as_u16();
        return Err(crate::provider::classify_to_failure(code, &body, using_builtin));
    }
    parse_attempt(&body).map_err(|err| GroqFailure {
        message: err.to_string(),
        kind: FailKind::Other,
        explicit_quota: false,
        retry_after_ms: None,
        offline_hint: false,
    })
}

fn temperature_field(temperature: f32) -> String {
    if temperature <= 0.0 {
        "0".to_string()
    } else {
        format!("{temperature:.2}")
    }
}

fn audio_part(bytes: Vec<u8>) -> anyhow::Result<reqwest::multipart::Part> {
    let flac = bytes.starts_with(b"fLaC");
    let (name, mime) = if flac {
        ("dictation.flac", "audio/flac")
    } else {
        ("dictation.wav", "audio/wav")
    };
    reqwest::multipart::Part::bytes(bytes)
        .file_name(name)
        .mime_str(mime)
        .context("couldn't build the audio upload")
}

fn parse_attempt(body: &str) -> anyhow::Result<Attempt> {
    if let Ok(verbose) = serde_json::from_str::<VerboseBody>(body) {
        if let Some(text) = verbose.text {
            let (avg_logprob, no_speech_prob, compression_ratio) = verbose
                .segments
                .as_deref()
                .map(confidence)
                .unwrap_or((None, None, None));
            let text = verbose
                .segments
                .as_deref()
                .map(|segments| filter_hallucinated_segments(&text, segments))
                .unwrap_or(text);
            return Ok(Attempt {
                text: strip_prompt_leak(&text),
                avg_logprob,
                no_speech_prob,
                compression_ratio,
                language: verbose.language.map(|value| language_code(&value)).unwrap_or_default(),
            });
        }
    }
    let parsed: SimpleTranscript =
        serde_json::from_str(body).context("transcription response was not JSON")?;
    Ok(Attempt {
        text: strip_prompt_leak(&parsed.text),
        avg_logprob: None,
        no_speech_prob: None,
        compression_ratio: None,
        language: String::new(),
    })
}

fn confidence(segments: &[VerboseSegment]) -> (Option<f32>, Option<f32>, Option<f32>) {
    let logs: Vec<f32> = segments.iter().filter_map(|segment| segment.avg_logprob).collect();
    let noises: Vec<f32> = segments.iter().filter_map(|segment| segment.no_speech_prob).collect();
    let avg = if logs.is_empty() {
        None
    } else {
        Some(logs.iter().sum::<f32>() / logs.len() as f32)
    };
    let noise = noises.into_iter().reduce(f32::max);
    let compression = segments.iter().filter_map(|segment| segment.compression_ratio).reduce(f32::max);
    (avg, noise, compression)
}

pub fn normalize_language(language: &str) -> Option<String> {
    let language = language.trim().to_ascii_lowercase();
    if language.is_empty() || language == "auto" {
        return None;
    }
    if (2..=3).contains(&language.len()) && language.chars().all(|c| c.is_ascii_lowercase()) {
        Some(language)
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
struct SimpleTranscript {
    text: String,
}

#[derive(Debug, Deserialize)]
struct VerboseBody {
    text: Option<String>,
    language: Option<String>,
    segments: Option<Vec<VerboseSegment>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechRoute {
    /// Keep the transcript in the language that was spoken.
    Transcribe,
    /// `/audio/translations` with whisper-large-v3.
    Translate,
    /// Transcribe, then Llama into English without adding or dropping meaning.
    Mixed,
}

pub fn normalize_output(value: &str) -> &'static str {
    if value.trim().eq_ignore_ascii_case("same") {
        "same"
    } else {
        "english"
    }
}

/// A fixed non-English language other than Urdu can go straight to translation.
pub fn direct_translate(output: &str, setting: &str) -> bool {
    if normalize_output(output) != "english" {
        return false;
    }
    match normalize_language(setting).as_deref() {
        Some(code) if code != "en" && code != "ur" => true,
        _ => false,
    }
}

pub fn speech_route(output: &str, setting: &str, detected: &str, text: &str) -> SpeechRoute {
    if normalize_output(output) != "english" {
        return SpeechRoute::Transcribe;
    }
    let setting_code = normalize_language(setting).unwrap_or_default();
    if setting_code == "en" {
        return SpeechRoute::Transcribe;
    }
    let detected = language_code(detected);
    // English words that were only tagged Urdu. Translating them rewrites the
    // sentence. A real Urdu take has Urdu script; French and the rest still translate.
    if setting_code != "ur" && detected == "ur" && has_latin(text) && !has_arabic(text) {
        return SpeechRoute::Transcribe;
    }
    let urdu = setting_code == "ur" || detected == "ur" || mixed_urdu(text);
    if !urdu {
        if detected.is_empty() || detected == "en" {
            return SpeechRoute::Transcribe;
        }
        return SpeechRoute::Translate;
    }
    if mixed_urdu(text) || (has_latin(text) && (setting_code == "ur" || detected == "ur")) {
        return SpeechRoute::Mixed;
    }
    if setting_code == "ur" || detected == "ur" {
        return SpeechRoute::Translate;
    }
    SpeechRoute::Transcribe
}

pub fn language_code(raw: &str) -> String {
    let name = raw.trim().to_ascii_lowercase();
    match name.as_str() {
        "" => String::new(),
        "english" | "en" => "en".into(),
        "urdu" | "ur" => "ur".into(),
        "hindi" | "hi" => "hi".into(),
        "spanish" | "es" => "es".into(),
        "french" | "fr" => "fr".into(),
        "german" | "de" => "de".into(),
        "portuguese" | "pt" => "pt".into(),
        "italian" | "it" => "it".into(),
        "dutch" | "nl" => "nl".into(),
        "polish" | "pl" => "pl".into(),
        "russian" | "ru" => "ru".into(),
        "ukrainian" | "uk" => "uk".into(),
        "arabic" | "ar" => "ar".into(),
        "hebrew" | "he" => "he".into(),
        "japanese" | "ja" => "ja".into(),
        "korean" | "ko" => "ko".into(),
        "chinese" | "zh" => "zh".into(),
        "turkish" | "tr" => "tr".into(),
        "persian" | "farsi" | "fa" => "fa".into(),
        "punjabi" | "pa" => "pa".into(),
        other => {
            if (2..=3).contains(&other.len()) && other.chars().all(|c| c.is_ascii_lowercase()) {
                other.to_string()
            } else {
                other.to_string()
            }
        }
    }
}

fn mixed_urdu(text: &str) -> bool {
    has_arabic(text) && has_latin(text)
}

pub(crate) fn has_latin(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_alphabetic())
}

/// Latin letters and no other script, or Latin clearly outweighs it.
/// Accented English is often tagged Hindi or Urdu while the letters stay Latin.
pub(crate) fn latin_only(text: &str) -> bool {
    let latin = text.chars().filter(|c| c.is_ascii_alphabetic()).count();
    if latin == 0 {
        return false;
    }
    let other = text.chars().filter(|c| c.is_alphabetic() && !c.is_ascii()).count();
    other == 0 || latin > other.saturating_mul(3)
}

pub(crate) fn has_devanagari(text: &str) -> bool {
    text.chars().any(|c| ('\u{0900}'..='\u{097F}').contains(&c))
}

pub(crate) fn has_arabic(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(
            c,
            '\u{0600}'..='\u{06FF}'
                | '\u{0750}'..='\u{077F}'
                | '\u{08A0}'..='\u{08FF}'
                | '\u{FB50}'..='\u{FDFF}'
                | '\u{FE70}'..='\u{FEFF}'
        )
    })
}

#[derive(Debug, Deserialize)]
struct VerboseSegment {
    #[serde(default)]
    text: Option<String>,
    avg_logprob: Option<f32>,
    no_speech_prob: Option<f32>,
    #[serde(default)]
    compression_ratio: Option<f32>,
}

/// A later segment that is silence, very unsure, or highly repetitive is the
/// model talking after the person stopped. Keep the earlier speech.
fn filter_hallucinated_segments(text: &str, segments: &[VerboseSegment]) -> String {
    let usable: Vec<&VerboseSegment> = segments
        .iter()
        .filter(|segment| segment.text.as_ref().is_some_and(|value| !value.trim().is_empty()))
        .collect();
    if usable.len() < 2 {
        return text.to_string();
    }
    let kept: Vec<&str> = usable
        .iter()
        .filter(|segment| !segment_is_hallucination(segment))
        .filter_map(|segment| segment.text.as_deref())
        .collect();
    if kept.is_empty() || kept.len() == usable.len() {
        return text.to_string();
    }
    let joined = kept.join("").trim().to_string();
    if joined.is_empty() { text.to_string() } else { joined }
}

fn segment_is_hallucination(segment: &VerboseSegment) -> bool {
    segment.compression_ratio.is_some_and(|value| value > 2.4)
        || segment.avg_logprob.is_some_and(|value| value < -1.0)
        || segment.no_speech_prob.is_some_and(|value| value > 0.6)
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// Whisper's looping failure: one short phrase repeated over and over
/// ("So is there any, yeah. So is there any, yeah. ..."), which hides the rest
/// of the speech. True when one 2-6 word phrase appears 3+ times and covers at
/// least half the words.
pub(crate) fn repetitive(text: &str) -> bool {
    let tokens = words(text);
    if tokens.len() < 6 {
        return false;
    }
    for n in 2..=6usize {
        if tokens.len() < n * 3 {
            break;
        }
        let mut counts: std::collections::HashMap<&[String], usize> = std::collections::HashMap::new();
        for gram in tokens.windows(n) {
            *counts.entry(gram).or_default() += 1;
        }
        if let Some(best) = counts.values().max() {
            if *best >= 3 && best * n * 2 >= tokens.len() {
                return true;
            }
        }
    }
    false
}

/// Last resort: drop sentences that repeat the one before them.
pub(crate) fn collapse_repeats(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut last: Vec<String> = Vec::new();
    for piece in text.split_inclusive(['.', '?', '!', '\u{06D4}', '\u{061F}']) {
        let key = words(piece);
        if key.is_empty() {
            out.push(piece);
            continue;
        }
        if !last.is_empty() && last.ends_with(&key) {
            continue;
        }
        last = key;
        out.push(piece);
    }
    out.concat().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Layer(&'static str, Option<Box<Layer>>);
    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref().map(|e| e as _)
        }
    }

    #[test]
    fn error_chain_shows_the_root_cause() {
        let err = Layer("error sending request", Some(Box::new(Layer("dns error", Some(Box::new(Layer("No such host is known.", None)))))));
        assert_eq!(error_chain(&err), "error sending request: dns error: No such host is known.");
        let repeated = Layer("timed out", Some(Box::new(Layer("timed out", None))));
        assert_eq!(error_chain(&repeated), "timed out");
    }
    #[test]
    fn retry_after_is_parsed() {
        assert_eq!(retry_after_ms("Please try again in 3s. Need more"), Some(3000));
        assert_eq!(retry_after_ms("try again in 1.5s"), Some(1500));
        assert_eq!(retry_after_ms("try again in 640ms"), Some(640));
        assert_eq!(retry_after_ms("nope"), None);
    }

    #[test]
    fn stock_phrases_are_recognized() {
        assert!(stock_phrase(" Thank you. "));
        assert!(stock_phrase("Thanks for watching!"));
        assert!(!stock_phrase("Thank you for the report."));
    }

    #[test]
    fn prompt_includes_terms_and_truncates() {
        assert_eq!(whisper_prompt(&[]), None);
        assert_eq!(
            whisper_prompt(&["  Bolo  ".into(), "".into(), "Tauri".into()]).as_deref(),
            Some("Bolo, Tauri.")
        );
        let brief = whisper_prompt_brief();
        assert!(brief.contains("EasySSH") && brief.contains("SSH") && brief.ends_with('.'));
        assert!(!brief.to_lowercase().contains("bolo"));
        assert!(!brief.contains("Spell these"));
        assert!(brief_audio(1.1));
        assert!(!brief_audio(0.0));
        assert!(!brief_audio(BRIEF_AUDIO_SECS));
        assert!(!brief_audio(4.0));
        let huge = vec!["Supercalifragilistic".to_string(); 80];
        let prompt = whisper_prompt(&huge).unwrap();
        assert!(prompt.chars().count() <= PROMPT_TERM_BUDGET + 8, "{}", prompt.chars().count());
        assert!(!prompt.contains("Dictation transcript"));
        assert!(!prompt.contains("Spell these"));
        assert!(prompt.contains("Supercalifragilistic"));
        let with_recent = whisper_prompt_with(
            &["Bolo".into()],
            &["Ship the notes to Priya tomorrow morning before the review.".into()],
        )
        .unwrap();
        assert!(with_recent.contains("Bolo"));
        assert!(with_recent.contains("Priya"));
        assert!(!with_recent.to_lowercase().contains("tomorrow"));
        assert!(!with_recent.to_lowercase().contains("ship the notes"));
        assert!(!with_recent.contains("EasySSH"), "a normal take does not lead with the tech glossary\n{with_recent}");
        assert!(!with_recent.contains("Recent dictation"));
        assert!(!with_recent.contains("Spell these"));
        assert!(with_recent.chars().count() <= PROMPT_TERM_BUDGET + 8);
        assert_eq!(whisper_prompt_with(&[], &["  ".into()]), None);
        assert_eq!(
            whisper_prompt_with(&[], &["Ship the notes to Priya tomorrow.".into()]).as_deref(),
            Some("Priya.")
        );
        assert_eq!(
            strip_prompt_leak("Recent dictation:\"Hey, can you listen to me?\""),
            "Hey, can you listen to me?"
        );
        assert_eq!(
            strip_prompt_leak(
                "Dictation transcript. Spell these terms exactly when spoken: Bolo, Tauri. Ship it tomorrow."
            ),
            "Ship it tomorrow."
        );
        assert_eq!(strip_prompt_leak("Ship it tomorrow."), "Ship it tomorrow.");
        assert_eq!(
            strip_prompt_leak("EasySSH, SSH, API, HTTP, HTTPS, JSON, URL, SQL. Ship it."),
            "Ship it."
        );
    }

    #[test]
    fn short_tech_phrases_stay_words_not_invented_sentences() {
        let cases = [
            ("easy ssh", "EasySSH"),
            ("easy, ssh", "EasySSH"),
            ("EasySSH", "EasySSH"),
            ("easyssh", "EasySSH"),
            ("easy s s h", "EasySSH"),
            ("s s h", "SSH"),
            ("ssh", "SSH"),
            ("easy, it's such are", "EasySSH"),
            ("easy, its such are", "EasySSH"),
            ("easy it’s such", "EasySSH"),
            ("Bolo and Easy, it's such a truth. Nerd thing, imagination.", "EasySSH"),
            ("Bolo and Easy, it's such a truth. Nerd thing, imagination.", "EasySSH"),
            ("it's such", "SSH"),
            ("it's such are", "SSH"),
            ("it's such a truth. Nerd thing, imagination.", "SSH"),
            ("open easy ssh please", "open EasySSH please"),
            ("ssh into the server", "ssh into the server"),
            ("I think it's such a good idea", "I think it's such a good idea"),
            ("Easy, it's such a beautiful day", "Easy, it's such a beautiful day"),
            ("Bolo and Easy, it's such a beautiful day", "Bolo and Easy, it's such a beautiful day"),
            ("It's such a truth. Remember that.", "It's such a truth. Remember that."),
            ("Open Bolo and start dictating.", "Open Bolo and start dictating."),
            ("ship it tomorrow", "ship it tomorrow"),
        ];
        for (heard, expected) in cases {
            assert_eq!(recover_brief_tech(heard), expected, "{heard}");
        }
        let runaway = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda";
        let clamped = guard_short_take(runaway, 0.5);
        assert!(clamped.split_whitespace().count() < runaway.split_whitespace().count(), "{clamped}");
        assert_eq!(guard_short_take("ship the notes to Priya tomorrow", 3.0), "ship the notes to Priya tomorrow");
        assert_eq!(guard_short_take("easy, it's such are", 0.9), "EasySSH");
        let body = r#"{"text":" easy it's such. Nerd thing, imagination.","language":"english","segments":[
            {"text":" easy it's such.","avg_logprob":-0.3,"no_speech_prob":0.05,"compression_ratio":1.2},
            {"text":" Nerd thing, imagination.","avg_logprob":-1.4,"no_speech_prob":0.82,"compression_ratio":2.8}
        ]}"#;
        let attempt = parse_attempt(body).unwrap();
        assert!(!attempt.text.to_lowercase().contains("imagination"), "{}", attempt.text);
        assert!(attempt.text.to_lowercase().contains("such") || attempt.text.contains("EasySSH"));
    }

    #[test]
    fn retries_empty_or_low_confidence_results() {
        assert!(should_retry("", None, None, None));
        assert!(should_retry("   ", Some(-0.2), Some(0.1), None));
        assert!(should_retry("hello there", Some(-1.4), Some(0.2), None));
        assert!(should_retry("hello there", Some(-0.85), Some(0.1), None), "the old -1.0 gate pasted this");
        assert!(should_retry("ship it tomorrow", Some(-0.3), Some(0.05), Some(2.8)));
        assert!(should_retry("hi", Some(-0.2), Some(0.9), None));
        assert!(!should_retry("ship it tomorrow", Some(-0.3), Some(0.05), Some(1.4)));
        assert!(!should_retry("a longer confident sentence here", None, Some(0.8), None));
        let body = r#"{"text":" Ship it. ","language":"english","segments":[{"avg_logprob":-0.2,"no_speech_prob":0.01,"compression_ratio":1.1},{"avg_logprob":-0.4,"no_speech_prob":0.2,"compression_ratio":1.8}]}"#;
        let attempt = parse_attempt(body).unwrap();
        assert_eq!(attempt.text, "Ship it.");
        assert_eq!(attempt.language, "en");
        assert!(attempt.avg_logprob.unwrap() < -0.2);
        assert!((attempt.no_speech_prob.unwrap() - 0.2).abs() < 0.001);
        assert!((attempt.compression_ratio.unwrap() - 1.8).abs() < 0.001);
        let plain = parse_attempt(r#"{"text":"hello"}"#).unwrap();
        assert_eq!(plain.text, "hello");
        assert_eq!(retry_plan(FailKind::RateLimit, 0), RetryPlan::Backoff { ms: 700 });
        assert_eq!(retry_plan(FailKind::RateLimit, 1), RetryPlan::Stop { fallback: true });
        assert_eq!(retry_plan(FailKind::Unavailable, 0), RetryPlan::Backoff { ms: 250 });
        assert_eq!(retry_plan(FailKind::Unavailable, 1), RetryPlan::Backoff { ms: 750 });
        assert_eq!(retry_plan(FailKind::Unavailable, 2), RetryPlan::Stop { fallback: true });
        assert_eq!(retry_plan(FailKind::Rejected, 0), RetryPlan::Backoff { ms: 400 });
        assert_eq!(retry_plan(FailKind::Rejected, 1), RetryPlan::Stop { fallback: true });
        assert!(!REJECTED_KEY_MESSAGE.to_lowercase().contains("settings"));
        assert!(!REJECTED_KEY_MESSAGE.to_lowercase().contains("windows"));
        let failure = GroqFailure {
            message: "Groq is rate limiting requests right now".into(),
            kind: FailKind::RateLimit,
            explicit_quota: false,
            retry_after_ms: None,
            offline_hint: false,
        };
        assert_eq!(retry_plan(FailKind::QuotaExhausted, 0), RetryPlan::Stop { fallback: true });
        let wrapped = anyhow::Error::from(failure);
        assert_eq!(failure_kind(&wrapped), Some(FailKind::RateLimit));
        assert!(plain.avg_logprob.is_none());
        assert_eq!(temperature_field(0.0), "0");
        assert_eq!(temperature_field(RETRY_TEMPERATURE), "0.20");
    }

    #[test]
    fn a_longer_weak_retry_loses_to_the_clearer_take() {
        let clear = Attempt {
            text: "send the invoice friday".into(),
            avg_logprob: Some(-0.35),
            no_speech_prob: Some(0.05),
            compression_ratio: Some(1.2),
            language: "en".into(),
        };
        let longer = Attempt {
            text: "send the invoice friday and then we talked about the whole roadmap for next quarter in detail please".into(),
            avg_logprob: Some(-0.92),
            no_speech_prob: Some(0.4),
            compression_ratio: Some(2.7),
            language: "en".into(),
        };
        assert_eq!(prefer_attempt(clear, longer).text, "send the invoice friday");
        let mumbled = Attempt {
            text: "ship it to maro".into(),
            avg_logprob: Some(-0.95),
            no_speech_prob: Some(0.2),
            compression_ratio: Some(1.3),
            language: "en".into(),
        };
        let recovered = Attempt {
            text: "ship it tomorrow".into(),
            avg_logprob: Some(-0.28),
            no_speech_prob: Some(0.04),
            compression_ratio: Some(1.2),
            language: "en".into(),
        };
        assert_eq!(prefer_attempt(mumbled, recovered).text, "ship it tomorrow");
    }

    #[test]
    fn language_auto_is_omitted() {
        assert_eq!(normalize_language("auto"), None);
        assert_eq!(normalize_language(" EN "), Some("en".into()));
        assert_eq!(normalize_language("english"), None);
        assert_eq!(normalize_model("whisper-large-v3"), Some(WHISPER_LARGE));
        assert_eq!(normalize_model("gpt-4o"), None);
    }

    #[test]
    fn english_output_routes_urdu_and_other_languages() {
        assert_eq!(normalize_output("same"), "same");
        assert_eq!(normalize_output("English"), "english");
        assert!(!direct_translate("same", "es"));
        assert!(direct_translate("english", "es"));
        assert!(!direct_translate("english", "ur"));
        assert!(!direct_translate("english", "auto"));
        assert_eq!(speech_route("same", "ur", "ur", "سلام"), SpeechRoute::Transcribe);
        assert_eq!(speech_route("english", "en", "en", "hello"), SpeechRoute::Transcribe);
        assert_eq!(speech_route("english", "auto", "english", "hello"), SpeechRoute::Transcribe);
        assert_eq!(speech_route("english", "auto", "french", "bonjour"), SpeechRoute::Translate);
        assert_eq!(speech_route("english", "auto", "urdu", "یہ اردو ہے"), SpeechRoute::Translate);
        assert_eq!(
            speech_route("english", "auto", "urdu", "meeting کل ہے"),
            SpeechRoute::Mixed
        );
        assert_eq!(
            speech_route("english", "auto", "urdu", "please send the invoice by Friday"),
            SpeechRoute::Transcribe
        );
        assert_eq!(
            speech_route("english", "ur", "", "kal meeting hai at 5"),
            SpeechRoute::Mixed
        );
        assert_eq!(language_code("Urdu"), "ur");
        assert_eq!(language_code("english"), "en");
    }
}
