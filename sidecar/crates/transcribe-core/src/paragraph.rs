//! Grouping transcribed segments into paragraphs.

use std::time::Duration;

use crate::Segment;

/// Turns segments into paragraphs of plain text.
pub trait Paragrapher {
    fn paragraphs(&self, segments: &[Segment]) -> Vec<String>;
}

/// Starts a new paragraph when the silence between two segments is at least `min_gap`.
///
/// Rules:
/// - each segment's text is trimmed; segments that are empty after trimming are
///   dropped, and the gap is measured from the end of the last kept segment;
/// - segments within a paragraph are joined with a single space;
/// - an empty paragraph is never produced.
///
/// Overlapping segments count as a zero gap.
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
    fn paragraphs(&self, segments: &[Segment]) -> Vec<String> {
        let mut paragraphs = Vec::new();
        let mut current = String::new();
        let mut last_end: Option<Duration> = None;

        for segment in segments {
            let text = segment.text.trim();
            if text.is_empty() {
                continue;
            }
            if let Some(end) = last_end {
                if segment.start.saturating_sub(end) >= self.min_gap {
                    paragraphs.push(std::mem::take(&mut current));
                }
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(text);
            last_end = Some(last_end.map_or(segment.end, |end| end.max(segment.end)));
        }

        if !current.is_empty() {
            paragraphs.push(current);
        }
        paragraphs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment {
            start: Duration::from_millis(start_ms),
            end: Duration::from_millis(end_ms),
            text: text.to_string(),
        }
    }

    #[test]
    fn default_min_gap_is_1_8_seconds() {
        assert_eq!(PauseGap::default().min_gap, Duration::from_millis(1800));
    }

    #[test]
    fn no_segments_gives_no_paragraphs() {
        assert!(PauseGap::default().paragraphs(&[]).is_empty());
    }

    #[test]
    fn short_gaps_join_with_single_space() {
        let segments = [seg(0, 1000, " Hello"), seg(1500, 2000, "world. ")];
        assert_eq!(
            PauseGap::default().paragraphs(&segments),
            vec!["Hello world."]
        );
    }

    #[test]
    fn gap_at_threshold_starts_new_paragraph() {
        let segments = [seg(0, 1000, "One."), seg(2800, 3000, "Two.")];
        assert_eq!(
            PauseGap::default().paragraphs(&segments),
            vec!["One.", "Two."]
        );
    }

    #[test]
    fn gap_just_below_threshold_stays_in_paragraph() {
        let segments = [seg(0, 1000, "One."), seg(2799, 3000, "Two.")];
        assert_eq!(PauseGap::default().paragraphs(&segments), vec!["One. Two."]);
    }

    #[test]
    fn empty_segments_are_dropped_and_gap_measured_from_last_kept() {
        // The blank segment sits in the middle of the pause; it must neither
        // split the pause in two nor produce an empty paragraph.
        let segments = [
            seg(0, 1000, "One."),
            seg(1500, 2000, "   "),
            seg(3000, 3500, "Two."),
            seg(10_000, 11_000, "\n\t"),
        ];
        assert_eq!(
            PauseGap::default().paragraphs(&segments),
            vec!["One.", "Two."]
        );
    }

    #[test]
    fn only_whitespace_segments_give_no_paragraphs() {
        let segments = [seg(0, 1000, " "), seg(5000, 6000, "")];
        assert!(PauseGap::default().paragraphs(&segments).is_empty());
    }

    #[test]
    fn overlapping_segments_count_as_zero_gap() {
        let segments = [seg(0, 2000, "A"), seg(1500, 2500, "B")];
        assert_eq!(PauseGap::default().paragraphs(&segments), vec!["A B"]);
    }

    // --- Adversarial cases added by the test pass. -------------------------
    //
    // Deliberately not covered:
    // - Zero-width and other non-`White_Space` characters (U+200B, U+FEFF):
    //   `str::trim` keeps them, so a segment made only of them becomes a
    //   visually empty paragraph. Whisper does not emit these in practice;
    //   noted in the test report rather than locked in either way.

    fn gap(min_gap_ms: u64) -> PauseGap {
        PauseGap::new(Duration::from_millis(min_gap_ms))
    }

    #[test]
    fn gap_just_above_threshold_starts_new_paragraph() {
        let segments = [seg(0, 1000, "One."), seg(2801, 3000, "Two.")];
        assert_eq!(
            PauseGap::default().paragraphs(&segments),
            vec!["One.", "Two."]
        );
    }

    #[test]
    fn gap_one_nanosecond_below_threshold_stays_in_paragraph() {
        let segments = [
            Segment {
                start: Duration::ZERO,
                end: Duration::from_secs(1),
                text: "One.".into(),
            },
            Segment {
                start: Duration::from_millis(2800) - Duration::from_nanos(1),
                end: Duration::from_secs(3),
                text: "Two.".into(),
            },
        ];
        assert_eq!(PauseGap::default().paragraphs(&segments), vec!["One. Two."]);
    }

    #[test]
    fn threshold_follows_configured_min_gap_not_default() {
        // 500 ms gap: below the 1.8 s default, at a 500 ms threshold.
        let segments = [seg(0, 1000, "One."), seg(1500, 2000, "Two.")];
        assert_eq!(gap(500).paragraphs(&segments), vec!["One.", "Two."]);
        assert_eq!(gap(501).paragraphs(&segments), vec!["One. Two."]);
    }

    #[test]
    fn zero_min_gap_puts_every_kept_segment_in_its_own_paragraph() {
        // Any gap, including zero (touching segments), is "at least" zero.
        let segments = [
            seg(0, 1000, "One."),
            seg(1000, 2000, "Two."),
            seg(2500, 3000, "Three."),
        ];
        assert_eq!(gap(0).paragraphs(&segments), vec!["One.", "Two.", "Three."]);
    }

    #[test]
    fn zero_min_gap_still_drops_empty_segments() {
        let segments = [
            seg(0, 1000, "One."),
            seg(1000, 1500, " "),
            seg(1500, 2000, "Two."),
        ];
        assert_eq!(gap(0).paragraphs(&segments), vec!["One.", "Two."]);
    }

    #[test]
    fn huge_min_gap_gives_one_paragraph() {
        let segments = [
            seg(0, 1000, "One."),
            seg(3_600_000, 3_601_000, "Two."),
            seg(86_400_000, 86_401_000, "Three."),
        ];
        assert_eq!(
            PauseGap::new(Duration::MAX).paragraphs(&segments),
            vec!["One. Two. Three."]
        );
    }

    #[test]
    fn huge_timestamps_do_not_overflow() {
        let segments = [
            Segment {
                start: Duration::MAX - Duration::from_secs(10),
                end: Duration::MAX - Duration::from_secs(9),
                text: "One.".into(),
            },
            Segment {
                start: Duration::MAX - Duration::from_secs(1),
                end: Duration::MAX,
                text: "Two.".into(),
            },
        ];
        assert_eq!(
            PauseGap::default().paragraphs(&segments),
            vec!["One.", "Two."]
        );
    }

    #[test]
    fn single_segment_gives_single_trimmed_paragraph() {
        assert_eq!(
            PauseGap::default().paragraphs(&[seg(0, 1000, "\t Hello there. \r\n")]),
            vec!["Hello there."]
        );
    }

    #[test]
    fn all_empty_segments_with_long_gaps_give_no_paragraphs() {
        let segments = [
            seg(0, 1000, ""),
            seg(10_000, 11_000, "   "),
            seg(20_000, 21_000, "\n"),
            seg(30_000, 31_000, "\u{3000}\u{00A0}"),
        ];
        assert!(PauseGap::default().paragraphs(&segments).is_empty());
    }

    #[test]
    fn leading_empty_segments_do_not_create_a_paragraph_break() {
        let segments = [seg(0, 1000, " "), seg(5000, 6000, "Hello")];
        assert_eq!(PauseGap::default().paragraphs(&segments), vec!["Hello"]);
    }

    #[test]
    fn unicode_whitespace_is_trimmed() {
        // Ideographic space, no-break space, em space, line separator.
        let segments = [
            seg(0, 1000, "\u{3000}こんにちは\u{00A0}"),
            seg(1100, 2000, "\u{2003}مرحبا\u{2028}"),
        ];
        assert_eq!(
            PauseGap::default().paragraphs(&segments),
            vec!["こんにちは مرحبا"]
        );
    }

    #[test]
    fn interior_whitespace_is_preserved_and_joins_use_one_space() {
        let segments = [seg(0, 1000, "  a  b "), seg(1000, 2000, " c\td ")];
        assert_eq!(PauseGap::default().paragraphs(&segments), vec!["a  b c\td"]);
    }

    #[test]
    fn no_paragraph_has_leading_or_trailing_whitespace() {
        let segments = [
            seg(0, 1000, "\n One \n"),
            seg(1200, 2000, "\n"),
            seg(5000, 6000, " Two\n\n"),
            seg(6100, 6200, " "),
        ];
        for p in PauseGap::default().paragraphs(&segments) {
            assert!(!p.is_empty());
            assert_eq!(p, p.trim(), "{p:?}");
        }
    }

    #[test]
    #[ignore = "SPEC GAP: a segment containing a blank line yields a paragraph containing a \
                blank line, which the plugin's blank-line-separated insertion turns into two \
                paragraphs; PauseGap only trims the ends of each segment"]
    fn paragraph_never_contains_a_blank_line() {
        // The plugin inserts paragraphs separated by blank lines (docs/plan.md,
        // "Insertion"), so a blank line inside one paragraph silently splits it.
        let segments = [seg(0, 1000, "One.\n\nTwo.")];
        for p in PauseGap::default().paragraphs(&segments) {
            assert!(!p.contains("\n\n"), "{p:?}");
        }
    }

    #[test]
    fn empty_segment_with_late_end_does_not_extend_the_gap_origin() {
        // The blank segment ends at 9 s, but only kept segments count: the gap
        // from "One." (ends 1 s) to "Two." (starts 2.5 s) is 1.5 s.
        let segments = [
            seg(0, 1000, "One."),
            seg(1000, 9000, "  "),
            seg(2500, 3000, "Two."),
        ];
        assert_eq!(PauseGap::default().paragraphs(&segments), vec!["One. Two."]);
    }

    #[test]
    fn empty_segment_inside_a_short_gap_does_not_split() {
        // Kept-to-kept gap is 1 s; the empty segment must not be used as an origin.
        let segments = [
            seg(0, 1000, "One."),
            seg(1000, 1000, ""),
            seg(2000, 2500, "Two."),
        ];
        assert_eq!(PauseGap::default().paragraphs(&segments), vec!["One. Two."]);
    }

    #[test]
    fn gap_is_measured_from_latest_end_of_kept_segments() {
        // A long segment followed by a short one nested inside it: the gap to
        // "C" must be measured from A's end (10 s), not B's end (3 s).
        let segments = [
            seg(0, 10_000, "A"),
            seg(2000, 3000, "B"),
            seg(5000, 6000, "C"),
        ];
        assert_eq!(PauseGap::default().paragraphs(&segments), vec!["A B C"]);
    }

    #[test]
    fn out_of_order_segment_counts_as_zero_gap() {
        let segments = [seg(5000, 6000, "Later"), seg(0, 1000, "Earlier")];
        assert_eq!(
            PauseGap::default().paragraphs(&segments),
            vec!["Later Earlier"]
        );
    }

    #[test]
    fn segment_with_end_before_start_does_not_panic() {
        let segments = [seg(1000, 0, "Odd"), seg(5000, 6000, "Next")];
        assert_eq!(
            PauseGap::default().paragraphs(&segments),
            vec!["Odd", "Next"]
        );
    }

    #[test]
    fn many_paragraphs_keep_order_and_count() {
        let segments: Vec<Segment> = (0..50)
            .map(|i| seg(i * 5000, i * 5000 + 1000, &format!("p{i}")))
            .collect();
        let paragraphs = PauseGap::default().paragraphs(&segments);
        assert_eq!(paragraphs.len(), 50);
        for (i, p) in paragraphs.iter().enumerate() {
            assert_eq!(p, &format!("p{i}"));
        }
    }

    #[test]
    fn mixed_paragraphs_from_a_realistic_journal() {
        let segments = [
            seg(0, 2000, " Today I went for a walk."),
            seg(2300, 4000, " It was cold."),
            seg(4000, 4500, " "),
            seg(7000, 9000, " Later, I called Anna."),
            seg(9100, 9800, " We talked about the trip."),
            seg(12_000, 13_000, " Goodnight."),
        ];
        assert_eq!(
            PauseGap::default().paragraphs(&segments),
            vec![
                "Today I went for a walk. It was cold.",
                "Later, I called Anna. We talked about the trip.",
                "Goodnight.",
            ]
        );
    }
}
