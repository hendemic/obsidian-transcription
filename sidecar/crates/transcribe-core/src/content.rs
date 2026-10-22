//! Paragraph breaks from what is said, with pauses as a tie-breaker.

use std::cmp::Ordering;
use std::time::Duration;

use crate::paragraph::{assemble, Paragrapher, PauseGap};
use crate::sentence::clean_text;
use crate::Sentence;

/// Turns sentences into embedding vectors.
pub trait SentenceEmbedder {
    type Error: std::error::Error + Send + Sync + 'static;

    /// One vector per sentence, in order, all of the same length.
    fn embed(&self, sentences: &[&str]) -> Result<Vec<Vec<f32>>, Self::Error>;
}

/// Breaks paragraphs where the topic shifts, using pauses only when the shift is borderline.
///
/// Each sentence boundary gets a drift score: how different the two sentences
/// before it are from the two after it. Drift and pause are ranked within the
/// transcript, so both adapt to the speaker and the subject. Then:
/// - strong drift breaks, whatever the pause;
/// - weak drift never breaks, however long the pause (a pause to land a point
///   or to think stays inside the paragraph);
/// - borderline drift breaks only if the pause is also long for this speaker.
///
/// Absolute floors stop short transcripts, where ranks mean little, from
/// breaking on small differences. Paragraphs have at least
/// [`MIN_SENTENCES`](Self::MIN_SENTENCES) sentences; when breaks compete for
/// that, the ones with the higher combined rank win. A paragraph longer than
/// eight sentences is split at its strongest borderline boundary even without
/// a long pause, for readability; weak boundaries are never used for this.
///
/// If the embedder fails, this falls back to `fallback` and logs a warning:
/// a transcript is worth more than its paragraphing.
#[derive(Debug, Clone)]
pub struct ContentGate<E> {
    embedder: E,
    fallback: PauseGap,
}

/// Drift rank at or above which a boundary is a strong topic shift.
const STRONG_RANK: f64 = 0.85;
/// Drift a strong shift must also reach.
const STRONG_DRIFT: f32 = 0.75;
/// Drift rank at or below which a boundary is weak.
const WEAK_RANK: f64 = 0.40;
/// Drift below which a boundary is weak whatever its rank.
const MIN_DRIFT: f32 = 0.5;
/// Pause rank a borderline boundary needs to break.
const PAUSE_RANK: f64 = 0.70;
/// Pause a borderline boundary needs to break whatever its rank.
const MIN_PAUSE: Duration = Duration::from_millis(500);
const MIN_SENTENCES: usize = 2;
/// Paragraphs longer than this are split at their strongest borderline boundary.
const MAX_SENTENCES: usize = 8;

impl<E> ContentGate<E> {
    /// Fewest sentences in a paragraph.
    pub const MIN_SENTENCES: usize = MIN_SENTENCES;

    pub fn new(embedder: E, fallback: PauseGap) -> Self {
        Self { embedder, fallback }
    }
}

impl<E: SentenceEmbedder> Paragrapher for ContentGate<E> {
    fn paragraphs(&self, sentences: &[Sentence]) -> Vec<String> {
        let kept: Vec<(String, Duration)> = sentences
            .iter()
            .map(|s| (clean_text(&s.text), s.pause_after))
            .filter(|(text, _)| !text.is_empty())
            .collect();
        if kept.len() <= Self::MIN_SENTENCES {
            return assemble(kept.iter().map(|(text, _)| (text.as_str(), false)));
        }

        let texts: Vec<&str> = kept.iter().map(|(text, _)| text.as_str()).collect();
        let vectors = match self.embedder.embed(&texts) {
            Ok(v) if v.len() == texts.len() => v,
            Ok(v) => {
                log::warn!(
                    "paragraph model returned {} vectors for {} sentences; using pauses only",
                    v.len(),
                    texts.len()
                );
                return self.fallback.paragraphs(sentences);
            }
            Err(e) => {
                log::warn!("paragraph model failed: {e}; using pauses only");
                return self.fallback.paragraphs(sentences);
            }
        };

        let pauses: Vec<Duration> = kept.iter().map(|(_, pause)| *pause).collect();
        let breaks = decide(&drift(&vectors), &pauses[..pauses.len() - 1]);
        assemble(
            texts
                .iter()
                .enumerate()
                .map(|(i, text)| (*text, breaks.get(i).copied().unwrap_or(false))),
        )
    }
}

/// Topic drift at each boundary between consecutive vectors: one minus the
/// cosine similarity of the mean of the two unit vectors before it and the
/// two after it (fewer at the edges). Zero vectors give zero drift.
fn drift(vectors: &[Vec<f32>]) -> Vec<f32> {
    let unit: Vec<Vec<f32>> = vectors.iter().map(|v| normalized(v)).collect();
    (0..unit.len().saturating_sub(1))
        .map(|i| {
            let left = mean(&unit[i.saturating_sub(1)..=i]);
            let right = mean(&unit[i + 1..(i + 3).min(unit.len())]);
            1.0 - cosine(&left, &right)
        })
        .collect()
}

fn normalized(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter().map(|x| x / norm).collect()
    } else {
        v.to_vec()
    }
}

fn mean(vectors: &[Vec<f32>]) -> Vec<f32> {
    let len = vectors.iter().map(Vec::len).max().unwrap_or(0);
    let n = vectors.len().max(1) as f32;
    (0..len)
        .map(|d| vectors.iter().filter_map(|v| v.get(d)).sum::<f32>() / n)
        .collect()
}

/// Cosine similarity; 1 (no drift) when either vector is zero.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norms =
        a.iter().map(|x| x * x).sum::<f32>().sqrt() * b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norms > 0.0 {
        dot / norms
    } else {
        1.0
    }
}

/// Rank of each value within `values`, from 0 (lowest) to 1 (highest).
fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].partial_cmp(&values[b]).unwrap_or(Ordering::Equal));
    let denominator = values.len().saturating_sub(1).max(1) as f64;
    let mut out = vec![0.0; values.len()];
    for (position, &index) in order.iter().enumerate() {
        out[index] = position as f64 / denominator;
    }
    out
}

/// Whether to break after each boundary, given its drift and the pause there.
fn decide(drift: &[f32], pauses: &[Duration]) -> Vec<bool> {
    let n = drift.len().min(pauses.len());
    let drift_rank = ranks(&drift[..n].iter().map(|&d| f64::from(d)).collect::<Vec<_>>());
    let pause_rank = ranks(
        &pauses[..n]
            .iter()
            .map(Duration::as_secs_f64)
            .collect::<Vec<_>>(),
    );

    let weak: Vec<bool> = (0..n)
        .map(|i| drift_rank[i] <= WEAK_RANK || drift[i] < MIN_DRIFT)
        .collect();
    let score = |i: usize| drift_rank[i] + pause_rank[i];
    let strongest_first =
        |a: &usize, b: &usize| score(*b).partial_cmp(&score(*a)).unwrap_or(Ordering::Equal);

    let mut wanted: Vec<usize> = (0..n)
        .filter(|&i| {
            let strong = drift_rank[i] >= STRONG_RANK && drift[i] >= STRONG_DRIFT;
            let long_pause = pause_rank[i] >= PAUSE_RANK && pauses[i] >= MIN_PAUSE;
            strong || (!weak[i] && long_pause)
        })
        .collect();
    wanted.sort_by(strongest_first);

    // Boundary i ends a paragraph after sentence i; there are n + 1 sentences.
    let sentences = n + 1;
    let mut chosen: Vec<usize> = Vec::new();
    for i in wanted {
        let edges = with_edge(&chosen, i);
        if paragraph_lengths_ok(&edges, sentences) {
            chosen = edges;
        }
    }

    // Split paragraphs that run long at their strongest borderline boundary,
    // until none is too long or none can be split without a weak break.
    loop {
        let split = paragraph_spans(&chosen, sentences)
            .filter(|(start, end)| end - start > MAX_SENTENCES)
            .find_map(|(start, end)| {
                (start..end - 1)
                    .filter(|&i| {
                        !weak[i] && paragraph_lengths_ok(&with_edge(&chosen, i), sentences)
                    })
                    .min_by(strongest_first)
            });
        match split {
            Some(i) => chosen = with_edge(&chosen, i),
            None => break,
        }
    }

    let mut breaks = vec![false; n];
    for i in chosen {
        breaks[i] = true;
    }
    breaks
}

fn with_edge(edges: &[usize], edge: usize) -> Vec<usize> {
    let mut out = edges.to_vec();
    out.push(edge);
    out.sort_unstable();
    out
}

/// `(first sentence, one past the last)` of each paragraph when breaking
/// after each sentence index in sorted `edges`.
fn paragraph_spans(edges: &[usize], sentences: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
    let starts = std::iter::once(0).chain(edges.iter().map(|e| e + 1));
    let ends = edges
        .iter()
        .map(|e| e + 1)
        .chain(std::iter::once(sentences));
    starts.zip(ends)
}

/// Whether breaking after each sentence index in sorted `edges` leaves every
/// paragraph of `sentences` with at least [`MIN_SENTENCES`].
fn paragraph_lengths_ok(edges: &[usize], sentences: usize) -> bool {
    paragraph_spans(edges, sentences).all(|(start, end)| end.saturating_sub(start) >= MIN_SENTENCES)
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

    /// Embeds by the sentence's first word: "Cats" and "Taxes" are unrelated
    /// topics; anything else is halfway between.
    struct Topics;

    #[derive(Debug, thiserror::Error)]
    #[error("embedder broke")]
    struct Broke;

    impl SentenceEmbedder for Topics {
        type Error = Broke;
        fn embed(&self, sentences: &[&str]) -> Result<Vec<Vec<f32>>, Broke> {
            Ok(sentences
                .iter()
                .map(|s| match s.split_whitespace().next() {
                    Some("Cats") => vec![1.0, 0.0],
                    Some("Taxes") => vec![0.0, 1.0],
                    _ => vec![1.0, 1.0],
                })
                .collect())
        }
    }

    struct Failing;

    impl SentenceEmbedder for Failing {
        type Error = Broke;
        fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, Broke> {
            Err(Broke)
        }
    }

    struct WrongCount;

    impl SentenceEmbedder for WrongCount {
        type Error = Broke;
        fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, Broke> {
            Ok(vec![vec![1.0]])
        }
    }

    fn gate<E>(embedder: E) -> ContentGate<E> {
        ContentGate::new(embedder, PauseGap::default())
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn topic_shift_breaks_without_a_pause() {
        let sentences = [
            s("Cats sleep.", 100),
            s("Cats purr.", 100),
            s("Cats hunt.", 100),
            s("Cats nap.", 100),
            s("Taxes are due.", 100),
            s("Taxes are high.", 100),
            s("Taxes are odd.", 100),
            s("Taxes are done.", 0),
        ];
        assert_eq!(
            gate(Topics).paragraphs(&sentences),
            vec![
                "Cats sleep. Cats purr. Cats hunt. Cats nap.",
                "Taxes are due. Taxes are high. Taxes are odd. Taxes are done."
            ]
        );
    }

    #[test]
    fn long_pause_on_the_same_topic_does_not_break() {
        let sentences = [
            s("Cats sleep.", 100),
            s("Cats purr.", 100),
            s("Cats hunt.", 10_000),
            s("Cats nap.", 100),
            s("Cats eat.", 100),
            s("Cats play.", 0),
        ];
        assert_eq!(gate(Topics).paragraphs(&sentences).len(), 1);
    }

    #[test]
    fn strong_drift_breaks_whatever_the_pause() {
        let drift = [0.1, 0.1, 0.9, 0.1, 0.1, 0.1];
        let pauses = [ms(2000), ms(2000), ms(0), ms(2000), ms(2000), ms(2000)];
        assert_eq!(
            decide(&drift, &pauses),
            vec![false, false, true, false, false, false]
        );
    }

    #[test]
    fn weak_drift_never_breaks_however_long_the_pause() {
        let drift = [0.6, 0.6, 0.2, 0.6, 0.6, 0.6];
        let pauses = [ms(100), ms(100), ms(30_000), ms(100), ms(100), ms(100)];
        assert!(decide(&drift, &pauses).iter().all(|b| !b));
    }

    #[test]
    fn borderline_drift_breaks_only_with_a_long_pause() {
        // Boundary 3 is borderline: second-highest drift, but under the strong floor.
        let drift = [0.1, 0.2, 0.3, 0.6, 0.4, 0.9, 0.35, 0.45];
        let short = [ms(100); 8];
        let mut long = short;
        long[3] = ms(1500);
        assert!(!decide(&drift, &short)[3]);
        assert!(decide(&drift, &long)[3]);
    }

    #[test]
    fn borderline_needs_an_absolute_pause_too() {
        // Top-ranked pause, but only 300 ms. Seven sentences, under the length limit.
        let drift = [0.1, 0.2, 0.3, 0.6, 0.4, 0.35];
        let mut pauses = [ms(100); 6];
        pauses[3] = ms(300);
        assert!(!decide(&drift, &pauses)[3]);
    }

    #[test]
    fn strong_rank_needs_an_absolute_drift_too() {
        // Highest drift in the transcript, but everything is on one topic.
        let drift = [0.1, 0.12, 0.3, 0.1, 0.11, 0.1];
        assert!(decide(&drift, &[ms(100); 6]).iter().all(|b| !b));
    }

    #[test]
    fn adjacent_strong_shifts_keep_two_sentences_and_the_stronger_wins() {
        // Breaking at both 5 and 6 would leave sentence 6 alone.
        let mut drift = [0.1; 12];
        drift[5] = 0.92;
        drift[6] = 0.95;
        let breaks = decide(&drift, &[ms(100); 12]);
        assert!(breaks[6] && !breaks[5], "{breaks:?}");
    }

    #[test]
    fn long_paragraph_splits_at_its_strongest_borderline_boundary() {
        // Twelve sentences, no strong shifts or long pauses; boundaries 5 and 7
        // are borderline, 7 more so.
        let mut drift = [0.3; 11];
        drift[5] = 0.6;
        drift[7] = 0.7;
        let breaks = decide(&drift, &[ms(100); 11]);
        let positions: Vec<usize> = (0..breaks.len()).filter(|&i| breaks[i]).collect();
        assert_eq!(positions, vec![7]);
    }

    #[test]
    fn long_paragraph_on_one_topic_is_not_split() {
        let breaks = decide(&[0.3; 11], &[ms(100); 11]);
        assert!(breaks.iter().all(|b| !b), "{breaks:?}");
    }

    #[test]
    fn very_long_paragraph_is_split_repeatedly() {
        // Twenty sentences with borderline boundaries every few sentences.
        let mut drift = [0.3; 19];
        for i in [4, 9, 14] {
            drift[i] = 0.65;
        }
        let breaks = decide(&drift, &[ms(100); 19]);
        let mut start = 0;
        for i in (0..breaks.len()).filter(|&i| breaks[i]) {
            assert!(i + 1 - start <= MAX_SENTENCES, "{breaks:?}");
            start = i + 1;
        }
        assert!(20 - start <= MAX_SENTENCES, "{breaks:?}");
    }

    #[test]
    fn break_leaving_one_sentence_at_either_end_is_rejected() {
        assert!(!paragraph_lengths_ok(&[0], 5));
        assert!(!paragraph_lengths_ok(&[3], 5));
        assert!(paragraph_lengths_ok(&[1], 5));
        assert!(paragraph_lengths_ok(&[1, 3], 6));
        assert!(!paragraph_lengths_ok(&[1, 2], 6));
        assert!(paragraph_lengths_ok(&[], 2));
    }

    #[test]
    fn two_sentences_are_one_paragraph() {
        assert_eq!(
            gate(Failing).paragraphs(&[s("Cats.", 5000), s("Taxes.", 0)]),
            vec!["Cats. Taxes."]
        );
    }

    #[test]
    fn failing_embedder_falls_back_to_pauses() {
        let sentences = [s("A.", 100), s("B.", 3000), s("C.", 100), s("D.", 0)];
        assert_eq!(gate(Failing).paragraphs(&sentences), vec!["A. B.", "C. D."]);
    }

    #[test]
    fn wrong_vector_count_falls_back_to_pauses() {
        let sentences = [s("A.", 100), s("B.", 3000), s("C.", 100), s("D.", 0)];
        assert_eq!(
            gate(WrongCount).paragraphs(&sentences),
            vec!["A. B.", "C. D."]
        );
    }

    #[test]
    fn empty_sentences_are_dropped_before_embedding() {
        let sentences = [s(" ", 0), s("Cats sleep.", 0), s("\u{200B}", 0)];
        assert_eq!(gate(Topics).paragraphs(&sentences), vec!["Cats sleep."]);
    }

    #[test]
    fn drift_is_zero_within_a_topic_and_one_across_unrelated_topics() {
        let (a, b) = (vec![1.0, 0.0], vec![0.0, 3.0]);
        let d = drift(&[a.clone(), a.clone(), a, b.clone(), b.clone(), b]);
        assert!(d[0].abs() < 1e-6, "{d:?}");
        assert!((d[2] - 1.0).abs() < 1e-6, "{d:?}");
        assert!(d[4].abs() < 1e-6, "{d:?}");
    }

    #[test]
    fn zero_vectors_give_zero_drift() {
        assert_eq!(drift(&[vec![0.0, 0.0], vec![1.0, 0.0]]), vec![0.0]);
    }

    #[test]
    fn ranks_run_from_zero_to_one() {
        assert_eq!(ranks(&[3.0, 1.0, 2.0]), vec![1.0, 0.0, 0.5]);
        assert_eq!(ranks(&[5.0]), vec![0.0]);
        assert!(ranks(&[]).is_empty());
    }
}
