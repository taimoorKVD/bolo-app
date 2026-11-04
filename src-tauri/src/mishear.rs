//! A fixed table of mishearings that are safe to repair on a new install.
//! Whole words or adjacent pairs only. A domain that is not the one in the
//! table is never rewritten.

struct Pair {
    heard: &'static [&'static str],
    write: &'static str,
}

const PAIRS: &[Pair] = &[
    Pair { heard: &["whats", "app"], write: "WhatsApp" },
    Pair { heard: &["whatsapp"], write: "WhatsApp" },
    Pair { heard: &["you", "tube"], write: "YouTube" },
    Pair { heard: &["youtube"], write: "YouTube" },
    Pair { heard: &["git", "hub"], write: "GitHub" },
    Pair { heard: &["github"], write: "GitHub" },
    Pair { heard: &["linked", "in"], write: "LinkedIn" },
    Pair { heard: &["linkedin"], write: "LinkedIn" },
    Pair { heard: &["power", "point"], write: "PowerPoint" },
    Pair { heard: &["powerpoint"], write: "PowerPoint" },
    Pair { heard: &["chat", "gpt"], write: "ChatGPT" },
    Pair { heard: &["chatgpt"], write: "ChatGPT" },
];

/// Repair known brand mishearings. "in" is LinkedIn only when it is not the
/// preposition in "in the" / "in a".
pub fn apply(text: &str) -> String {
    let mut tokens: Vec<String> = text.split_whitespace().map(|token| token.to_string()).collect();
    if tokens.is_empty() {
        return text.to_string();
    }
    let mut index = 0;
    let mut out = Vec::new();
    while index < tokens.len() {
        if let Some((take, replacement)) = match_at(&tokens, index) {
            let tail = trailing(&tokens[index + take - 1]);
            out.push(format!("{replacement}{tail}"));
            index += take;
            continue;
        }
        out.push(tokens[index].clone());
        index += 1;
    }
    out.join(" ")
}

fn match_at(tokens: &[String], index: usize) -> Option<(usize, &'static str)> {
    for pair in PAIRS {
        let n = pair.heard.len();
        if index + n > tokens.len() {
            continue;
        }
        if tokens[index..index + n].iter().any(|token| token.contains('@') || has_dot(&core(token))) {
            continue;
        }
        let same = pair
            .heard
            .iter()
            .zip(&tokens[index..index + n])
            .all(|(want, got)| core(got) == *want);
        if !same {
            continue;
        }
        if pair.write == "LinkedIn" && n == 2 {
            let after = tokens.get(index + 2).map(|token| core(token)).unwrap_or_default();
            if matches!(after.as_str(), "the" | "a" | "an" | "my" | "our" | "this" | "that" | "his" | "her") {
                continue;
            }
        }
        return Some((n, pair.write));
    }
    None
}

fn core(token: &str) -> String {
    token.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase()
}

fn has_dot(core: &str) -> bool {
    core.contains('.')
}

fn trailing(token: &str) -> String {
    token
        .chars()
        .rev()
        .take_while(|c| !c.is_alphanumeric())
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_mishearings_are_fixed_and_other_words_stay() {
        assert_eq!(apply("send it on whats app"), "send it on WhatsApp");
        assert_eq!(apply("open you tube"), "open YouTube");
        assert_eq!(apply("push to git hub"), "push to GitHub");
        assert_eq!(apply("share on linked in"), "share on LinkedIn");
        assert_eq!(apply("linked in the doc"), "linked in the doc");
        assert_eq!(apply("go to google.com"), "go to google.com");
        assert_eq!(apply("mail taimoorhussain.com"), "mail taimoorhussain.com");
        assert_eq!(apply("the roots of the word"), "the roots of the word");
        assert_eq!(apply("chat gpt please."), "ChatGPT please.");
    }
}
