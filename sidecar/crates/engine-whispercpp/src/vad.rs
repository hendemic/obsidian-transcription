//! Splicing speech spans found by VAD into one buffer, and mapping timestamps
//! in that buffer back to the original audio.
//!
//! This mirrors what whisper.cpp's `whisper_full` does internally when
//! `params.vad` is set. whisper-rs only exposes `whisper_full_with_state`,
//! which skips that step, so the engine runs whisper.cpp's Silero VAD itself
//! (via `WhisperVadContext`) and uses these helpers to feed only speech to the
//! decoder while keeping segment timestamps on the original timeline. Real
//! timestamps matter here: `PauseGap` paragraphs on the silences VAD removes.

use std::time::Duration;

use transcribe_core::SAMPLE_RATE;

/// Silence inserted between spliced speech spans (whisper.cpp uses 0.1 s).
pub(crate) const SILENCE_SAMPLES: usize = SAMPLE_RATE as usize / 10;

/// A span of speech in the original audio, in samples, `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpeechSpan {
    pub start: usize,
    pub end: usize,
}

impl SpeechSpan {
    /// Converts a VAD segment given in centiseconds to samples, clamped to `len`.
    pub fn from_centiseconds(start_cs: f32, end_cs: f32, len: usize) -> Self {
        let to_samples = |cs: f32| -> usize {
            let samples = (f64::from(cs) * f64::from(SAMPLE_RATE) / 100.0).round();
            if samples.is_nan() || samples <= 0.0 {
                0
            } else {
                (samples as usize).min(len)
            }
        };
        let start = to_samples(start_cs);
        let end = to_samples(end_cs).max(start);
        Self { start, end }
    }
}

/// Piecewise-linear map from positions in the spliced buffer to positions in
/// the original audio, both in samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimeMap {
    /// `(processed, original)` anchors; `processed` strictly increasing,
    /// `original` non-decreasing.
    points: Vec<(usize, usize)>,
}

impl TimeMap {
    /// The identity map (no VAD).
    pub fn identity() -> Self {
        Self { points: Vec::new() }
    }

    fn push(&mut self, processed: usize, original: usize) {
        let original = self
            .points
            .last()
            .map_or(original, |&(_, o)| o.max(original));
        match self.points.last_mut() {
            Some(last) if last.0 == processed => last.1 = original,
            _ => self.points.push((processed, original)),
        }
    }

    /// Maps a sample position in the spliced buffer to the original audio.
    pub fn to_original_samples(&self, processed: usize) -> usize {
        let (Some(&first), Some(&last)) = (self.points.first(), self.points.last()) else {
            return processed;
        };
        if processed <= first.0 {
            return first.1.saturating_sub(first.0 - processed);
        }
        if processed >= last.0 {
            return last.1 + (processed - last.0);
        }
        // First anchor strictly after `processed`; there is one before it.
        let i = self.points.partition_point(|&(p, _)| p <= processed);
        let (p0, o0) = self.points[i - 1];
        let (p1, o1) = self.points[i];
        let fraction = (processed - p0) as f64 / (p1 - p0) as f64;
        o0 + ((o1 - o0) as f64 * fraction).round() as usize
    }

    /// Maps a whisper.cpp timestamp (centiseconds into the spliced buffer) to
    /// a time in the original audio.
    pub fn to_original(&self, centiseconds: i64) -> Duration {
        let cs = usize::try_from(centiseconds).unwrap_or(0);
        let processed = cs.saturating_mul(SAMPLE_RATE as usize / 100);
        let original = self.to_original_samples(processed);
        Duration::from_secs_f64(original as f64 / f64::from(SAMPLE_RATE))
    }
}

/// Copies `spans` of `samples` into one buffer, extending every span but the
/// last by `overlap` samples and separating spans with [`SILENCE_SAMPLES`] of
/// silence, as whisper.cpp does. Returns the buffer and its [`TimeMap`].
///
/// Empty spans are skipped. If no span has any samples the buffer is empty.
pub(crate) fn splice(samples: &[f32], spans: &[SpeechSpan], overlap: usize) -> (Vec<f32>, TimeMap) {
    let spans: Vec<SpeechSpan> = spans
        .iter()
        .map(|s| SpeechSpan {
            start: s.start.min(samples.len()),
            end: s.end.min(samples.len()),
        })
        .filter(|s| s.end > s.start)
        .collect();

    let mut out = Vec::new();
    let mut map = TimeMap::identity();

    for (i, span) in spans.iter().enumerate() {
        let is_last = i + 1 == spans.len();
        let end = if is_last {
            span.end
        } else {
            (span.end + overlap).min(samples.len())
        };

        map.push(out.len(), span.start);
        out.extend_from_slice(&samples[span.start..end]);
        map.push(out.len(), end);

        if let Some(next) = spans.get(i + 1) {
            out.resize(out.len() + SILENCE_SAMPLES, 0.0);
            map.push(out.len(), next.start);
        }
    }

    (out, map)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: usize = SAMPLE_RATE as usize;

    #[test]
    fn identity_map_passes_through() {
        let map = TimeMap::identity();
        assert_eq!(map.to_original_samples(12_345), 12_345);
        assert_eq!(map.to_original(150), Duration::from_millis(1500));
        assert_eq!(map.to_original(-5), Duration::ZERO);
    }

    #[test]
    fn centiseconds_to_span_clamps() {
        assert_eq!(
            SpeechSpan::from_centiseconds(100.0, 250.0, 10 * SR),
            SpeechSpan {
                start: SR,
                end: SR * 5 / 2
            }
        );
        assert_eq!(
            SpeechSpan::from_centiseconds(-3.0, 1e9, 100),
            SpeechSpan { start: 0, end: 100 }
        );
        assert_eq!(
            SpeechSpan::from_centiseconds(50.0, 10.0, 10 * SR),
            SpeechSpan {
                start: SR / 2,
                end: SR / 2
            }
        );
    }

    #[test]
    fn splice_copies_speech_and_inserts_silence() {
        let samples: Vec<f32> = (0..10).map(|i| i as f32).collect();
        let spans = [
            SpeechSpan { start: 1, end: 3 },
            SpeechSpan { start: 6, end: 8 },
        ];
        let (out, _) = splice(&samples, &spans, 0);
        assert_eq!(out.len(), 2 + SILENCE_SAMPLES + 2);
        assert_eq!(&out[..2], &[1.0, 2.0]);
        assert!(out[2..2 + SILENCE_SAMPLES].iter().all(|&s| s == 0.0));
        assert_eq!(&out[2 + SILENCE_SAMPLES..], &[6.0, 7.0]);
    }

    #[test]
    fn splice_skips_empty_spans_and_handles_none() {
        let samples = vec![1.0; 100];
        let (out, map) = splice(&samples, &[SpeechSpan { start: 5, end: 5 }], 0);
        assert!(out.is_empty());
        assert_eq!(map, TimeMap::identity());
    }

    #[test]
    fn timestamps_map_back_across_removed_silence() {
        // Speech at 1–2 s and 10–11 s of a 12 s recording.
        let samples = vec![0.5; 12 * SR];
        let spans = [
            SpeechSpan {
                start: SR,
                end: 2 * SR,
            },
            SpeechSpan {
                start: 10 * SR,
                end: 11 * SR,
            },
        ];
        let (out, map) = splice(&samples, &spans, 0);
        assert_eq!(out.len(), 2 * SR + SILENCE_SAMPLES);

        // Start of the buffer is 1 s in the original.
        assert_eq!(map.to_original(0), Duration::from_secs(1));
        // Half way through the first span.
        assert_eq!(map.to_original(50), Duration::from_millis(1500));
        // End of the first span.
        assert_eq!(map.to_original(100), Duration::from_secs(2));
        // After the inserted 0.1 s of silence we are at 10 s.
        assert_eq!(map.to_original(110), Duration::from_secs(10));
        assert_eq!(map.to_original(160), Duration::from_millis(10_500));
        // End of buffer, and beyond it, extrapolate 1:1.
        assert_eq!(map.to_original(210), Duration::from_secs(11));
        assert_eq!(map.to_original(220), Duration::from_millis(11_100));
    }

    #[test]
    fn overlap_extends_all_but_last_span() {
        let samples = vec![0.0; 1000];
        let spans = [
            SpeechSpan { start: 0, end: 100 },
            SpeechSpan {
                start: 500,
                end: 600,
            },
        ];
        let (out, map) = splice(&samples, &spans, 50);
        assert_eq!(out.len(), 150 + SILENCE_SAMPLES + 100);
        assert_eq!(map.to_original_samples(150), 150);
        assert_eq!(map.to_original_samples(150 + SILENCE_SAMPLES), 500);
        assert_eq!(map.to_original_samples(150 + SILENCE_SAMPLES + 100), 600);
    }

    #[test]
    fn map_stays_monotonic_when_overlap_reaches_next_span() {
        let samples = vec![0.0; 1000];
        let spans = [
            SpeechSpan { start: 0, end: 100 },
            SpeechSpan {
                start: 120,
                end: 200,
            },
        ];
        let (out, map) = splice(&samples, &spans, 50);
        let mut previous = 0;
        for p in 0..=out.len() {
            let o = map.to_original_samples(p);
            assert!(o >= previous, "map went backwards at {p}");
            previous = o;
        }
    }
}
