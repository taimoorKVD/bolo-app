//! Phrases every build scores before release. Text only: no audio, no key,
//! no user data. Word error rate is printed for the release script.

#[cfg(test)]
mod tests {
    use crate::accuracy::{self, wer};
    use crate::vocab;

    #[test]
    fn release_phrases_score_zero_word_error() {
        let words = vocab::protected(&[]);
        let cases = [
            ("i goes to office yesterday", "I went to office yesterday."),
            ("she don't like the update", "She doesn't like the update."),
            ("we was late for the meeting", "We were late for the meeting."),
            ("this is a apple and a url", "This is an apple and a URL."),
            ("send it on whats app", "Send it on WhatsApp."),
            ("push the branch to git hub", "Push the branch to GitHub."),
            ("Open Volo and start dictating.", "Open Bolo and start dictating."),
            ("hello at taimoorhussain dot com", "hello@taimoorhussain.com"),
            ("Send it to taimoorhussain.com", "Send it to taimoorhussain.com."),
            ("go to google.com today", "Go to google.com today."),
            ("the total is 1500 rupees", "The total is 1500 rupees."),
            ("I roots the email yesterday.", "I wrote the email yesterday."),
            ("their going home now", "They're going home now."),
            ("bolo kya haal hai", "bolo kya haal hai"),
            ("میں Bolo office میں ہوں", "میں Bolo office میں ہوں"),
        ];
        let mut total = 0.0;
        for (before, after) in cases {
            let got = accuracy::polish_text(before, &[], &words, true);
            let score = wer(after, &got);
            assert_eq!(got, after, "{before}");
            assert_eq!(score, 0.0, "{before}");
            total += score;
        }
        let mean = total / cases.len() as f64;
        eprintln!("REGRESSION phrases={} mean_wer={mean:.3}", cases.len());
        assert_eq!(mean, 0.0);
    }
}
