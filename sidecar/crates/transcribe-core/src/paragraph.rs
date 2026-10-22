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
}
