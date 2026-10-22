//! Grouping sentences into paragraphs.

use std::time::Duration;

use crate::sentence::clean_text;
use crate::Sentence;

/// Turns sentences into paragraphs of plain text.
pub trait Paragrapher {
    /// Groups `sentences`, in order, into paragraphs. Paragraphs are never
    /// empty and never contain a line break.
    fn paragraphs(&self, sentences: &[Sentence]) -> Vec<String>;
}

/// Starts a new paragraph after a sentence followed by a pause of at least `min_gap`.
///
/// Sentences within a paragraph are joined with a single space. Sentences
/// that are empty after trimming are dropped, but a long pause after one
/// still ends the paragraph before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PauseGap {
    pub min_gap: Duration,
}

impl PauseGap {
    /// Default minimum pause that starts a new paragraph.
    pub const DEFAULT_MIN_GAP: Duration = Duration::from_millis(1800);

    pub fn new(min_gap: Duration) -> Self {
        Self { min_gap }
    }
}

impl Default for PauseGap {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MIN_GAP)
    }
}

impl Paragrapher for PauseGap {
    fn paragraphs(&self, sentences: &[Sentence]) -> Vec<String> {
        assemble(
            sentences
                .iter()
                .map(|s| (s.text.as_str(), s.pause_after >= self.min_gap)),
        )
    }
}

/// Joins sentences into paragraphs, ending a paragraph after each sentence
/// whose flag is set. Sentences are cleaned and joined with a single space;
/// empty ones are dropped, but a break after one still applies.
pub(crate) fn assemble<'a>(sentences: impl IntoIterator<Item = (&'a str, bool)>) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = String::new();

    for (text, break_after) in sentences {
        let text = clean_text(text);
        if !text.is_empty() {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(&text);
        }
        if break_after && !current.is_empty() {
            paragraphs.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str, pause_ms: u64) -> Sentence {
        Sentence {
            text: text.to_string(),
            pause_after: Duration::from_millis(pause_ms),
        }
    }

    fn gap(min_gap_ms: u64) -> PauseGap {
        PauseGap::new(Duration::from_millis(min_gap_ms))
    }

    #[test]
    fn default_min_gap_is_1_8_seconds() {
        assert_eq!(PauseGap::default().min_gap, Duration::from_millis(1800));
    }

    #[test]
    fn no_sentences_gives_no_paragraphs() {
        assert!(PauseGap::default().paragraphs(&[]).is_empty());
    }

    #[test]
    fn short_pauses_join_with_single_space() {
        let sentences = [s("Hello.", 500), s("World.", 0)];
        assert_eq!(
            PauseGap::default().paragraphs(&sentences),
            vec!["Hello. World."]
        );
    }

    #[test]
    fn pause_at_threshold_starts_new_paragraph() {
        let sentences = [s("One.", 1800), s("Two.", 0)];
        assert_eq!(
            PauseGap::default().paragraphs(&sentences),
            vec!["One.", "Two."]
        );
    }

    #[test]
    fn pause_just_below_threshold_stays_in_paragraph() {
        let sentences = [
            Sentence {
                text: "One.".into(),
                pause_after: Duration::from_millis(1800) - Duration::from_nanos(1),
            },
            s("Two.", 0),
        ];
        assert_eq!(
            PauseGap::default().paragraphs(&sentences),
            vec!["One. Two."]
        );
    }

    #[test]
    fn threshold_follows_configured_min_gap() {
        let sentences = [s("One.", 500), s("Two.", 0)];
        assert_eq!(gap(500).paragraphs(&sentences), vec!["One.", "Two."]);
        assert_eq!(gap(501).paragraphs(&sentences), vec!["One. Two."]);
    }

    #[test]
    fn zero_min_gap_puts_every_sentence_in_its_own_paragraph() {
        let sentences = [s("One.", 0), s("Two.", 0), s("Three.", 0)];
        assert_eq!(
            gap(0).paragraphs(&sentences),
            vec!["One.", "Two.", "Three."]
        );
    }

    #[test]
    fn huge_min_gap_gives_one_paragraph() {
        let sentences = [s("One.", 3_600_000), s("Two.", 86_400_000), s("Three.", 0)];
        assert_eq!(
            PauseGap::new(Duration::MAX).paragraphs(&sentences),
            vec!["One. Two. Three."]
        );
    }

    #[test]
    fn empty_sentences_are_dropped() {
        let sentences = [s("One.", 0), s("  ", 0), s("\u{200B}", 0), s("Two.", 0)];
        assert_eq!(
            PauseGap::default().paragraphs(&sentences),
            vec!["One. Two."]
        );
    }

    #[test]
    fn long_pause_after_an_empty_sentence_still_breaks() {
        let sentences = [s("One.", 0), s(" ", 5000), s("Two.", 0)];
        assert_eq!(
            PauseGap::default().paragraphs(&sentences),
            vec!["One.", "Two."]
        );
    }

    #[test]
    fn long_pause_before_any_text_does_not_create_an_empty_paragraph() {
        let sentences = [s(" ", 5000), s("Hello.", 0)];
        assert_eq!(PauseGap::default().paragraphs(&sentences), vec!["Hello."]);
    }

    #[test]
    fn paragraphs_are_trimmed_and_never_contain_a_line_break() {
        // Sentences built by hand may not be clean; the plugin separates
        // paragraphs with blank lines, so a line break would split one.
        let sentences = [
            s("\n One \n\n two. ", 0),
            s("\tThree.\r\n", 5000),
            s(" Four.", 0),
        ];
        let paragraphs = PauseGap::default().paragraphs(&sentences);
        assert_eq!(paragraphs, vec!["One two. Three.", "Four."]);
    }

    #[test]
    fn many_paragraphs_keep_order_and_count() {
        let sentences: Vec<Sentence> = (0..50).map(|i| s(&format!("p{i}."), 5000)).collect();
        let paragraphs = PauseGap::default().paragraphs(&sentences);
        assert_eq!(paragraphs.len(), 50);
        for (i, p) in paragraphs.iter().enumerate() {
            assert_eq!(p, &format!("p{i}."));
        }
    }

    #[test]
    fn mixed_paragraphs_from_a_realistic_journal() {
        let sentences = [
            s("Today I went for a walk.", 300),
            s("It was cold.", 3000),
            s("Later, I called Anna.", 100),
            s("We talked about the trip.", 2200),
            s("Goodnight.", 0),
        ];
        assert_eq!(
            PauseGap::default().paragraphs(&sentences),
            vec![
                "Today I went for a walk. It was cold.",
                "Later, I called Anna. We talked about the trip.",
                "Goodnight.",
            ]
        );
    }
}
