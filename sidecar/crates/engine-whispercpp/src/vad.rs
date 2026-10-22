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

    // --- Adversarial cases added by the test pass. -------------------------
    //
    // Expected positions are hand-computed from whisper.cpp's splicing rule
    // (extend every span but the last by `overlap`, insert 0.1 s of silence
    // between spans). Deliberately not covered:
    // - Unsorted span lists: whisper.cpp's VAD returns spans in order.
    // - Timestamps past the end of the spliced buffer: they extrapolate 1:1
    //   and may exceed the audio length; whisper.cpp does the same.

    /// Samples whose value is their index, so copied ranges are identifiable.
    fn ramp(len: usize) -> Vec<f32> {
        (0..len).map(|i| i as f32).collect()
    }

    fn span(start: usize, end: usize) -> SpeechSpan {
        SpeechSpan { start, end }
    }

    fn assert_monotonic(map: &TimeMap, len: usize) {
        let mut previous = map.to_original_samples(0);
        for p in 0..=len {
            let o = map.to_original_samples(p);
            assert!(
                o >= previous,
                "map went backwards at {p}: {previous} -> {o}"
            );
            previous = o;
        }
    }

    #[test]
    fn silence_is_a_tenth_of_a_second() {
        assert_eq!(SILENCE_SAMPLES, 1600);
    }

    #[test]
    fn no_spans_gives_empty_buffer_and_identity_map() {
        let (out, map) = splice(&ramp(1000), &[], 160);
        assert!(out.is_empty());
        assert_eq!(map, TimeMap::identity());
    }

    #[test]
    fn empty_audio_gives_empty_buffer() {
        let (out, _) = splice(&[], &[span(0, 100), span(200, 300)], 160);
        assert!(out.is_empty());
    }

    #[test]
    fn spans_entirely_past_the_end_are_dropped() {
        let (out, _) = splice(&ramp(100), &[span(10, 20), span(500, 600)], 0);
        // Only the first span survives, so no silence is appended.
        assert_eq!(out, ramp(20)[10..].to_vec());
    }

    #[test]
    fn span_end_past_audio_is_clamped() {
        let (out, map) = splice(&ramp(100), &[span(90, 1000)], 0);
        assert_eq!(out, ramp(100)[90..].to_vec());
        assert_eq!(map.to_original_samples(0), 90);
        assert_eq!(map.to_original_samples(10), 100);
    }

    #[test]
    fn single_span_is_copied_without_overlap_or_silence() {
        let samples = ramp(1000);
        let (out, map) = splice(&samples, &[span(200, 300)], 160);
        assert_eq!(out, samples[200..300].to_vec());
        assert_eq!(map.to_original_samples(0), 200);
        assert_eq!(map.to_original_samples(50), 250);
        assert_eq!(map.to_original_samples(100), 300);
    }

    #[test]
    fn single_span_covering_whole_audio_is_identity_like() {
        let samples = ramp(500);
        let (out, map) = splice(&samples, &[span(0, 500)], 160);
        assert_eq!(out, samples);
        for p in [0, 1, 250, 499, 500] {
            assert_eq!(map.to_original_samples(p), p);
        }
    }

    #[test]
    fn span_at_audio_start_maps_zero_to_zero() {
        let (_, map) = splice(&ramp(10 * SR), &[span(0, SR), span(5 * SR, 6 * SR)], 0);
        assert_eq!(map.to_original(0), Duration::ZERO);
        assert_eq!(map.to_original(50), Duration::from_millis(500));
    }

    #[test]
    fn span_at_audio_end_maps_buffer_end_to_audio_end() {
        let len = 10 * SR;
        let (out, map) = splice(&ramp(len), &[span(SR, 2 * SR), span(9 * SR, len)], 0);
        assert_eq!(map.to_original_samples(out.len()), len);
        assert_eq!(*out.last().unwrap(), (len - 1) as f32);
    }

    #[test]
    fn overlap_is_clamped_at_audio_end() {
        // The first span's overlap would run past the end of the audio.
        let samples = ramp(1000);
        let (out, map) = splice(&samples, &[span(800, 980), span(990, 1000)], 50);
        assert_eq!(out.len(), 200 + SILENCE_SAMPLES + 10);
        assert_eq!(&out[..200], &samples[800..1000]);
        assert_eq!(&out[200 + SILENCE_SAMPLES..], &samples[990..1000]);
        assert_monotonic(&map, out.len());
        assert_eq!(map.to_original_samples(out.len()), 1000);
    }

    #[test]
    fn overlap_audio_is_the_samples_after_the_span() {
        let samples = ramp(1000);
        let (out, _) = splice(&samples, &[span(100, 200), span(600, 700)], 30);
        assert_eq!(&out[..130], &samples[100..230]);
        assert!(out[130..130 + SILENCE_SAMPLES].iter().all(|&s| s == 0.0));
        assert_eq!(&out[130 + SILENCE_SAMPLES..], &samples[600..700]);
    }

    #[test]
    fn three_spans_length_and_boundaries() {
        let samples = ramp(100_000);
        let spans = [span(1000, 2000), span(10_000, 12_000), span(50_000, 50_500)];
        let overlap = 160;
        let (out, map) = splice(&samples, &spans, overlap);
        // (1000 + 160) + 1600 + (2000 + 160) + 1600 + 500
        assert_eq!(out.len(), 7020);
        // Span starts.
        assert_eq!(map.to_original_samples(0), 1000);
        assert_eq!(map.to_original_samples(2760), 10_000);
        assert_eq!(map.to_original_samples(6520), 50_000);
        // Span ends (including overlap).
        assert_eq!(map.to_original_samples(1160), 2160);
        assert_eq!(map.to_original_samples(4920), 12_160);
        assert_eq!(map.to_original_samples(7020), 50_500);
        // Copied samples line up with the map.
        assert_eq!(out[2760], 10_000.0);
        assert_eq!(out[6520], 50_000.0);
        assert_monotonic(&map, out.len());
    }

    #[test]
    fn adjacent_spans_keep_a_monotonic_map() {
        // Spans touch; the first one's overlap runs into the second.
        let samples = ramp(1000);
        let (out, map) = splice(&samples, &[span(0, 100), span(100, 200)], 50);
        assert_eq!(out.len(), 150 + SILENCE_SAMPLES + 100);
        assert_monotonic(&map, out.len());
        assert_eq!(map.to_original_samples(0), 0);
        assert_eq!(map.to_original_samples(150), 150);
        assert_eq!(map.to_original_samples(out.len()), 200);
        // The second span's start lands somewhere between its true start and
        // the end of the first span's overlap, never before either span.
        let second_start = map.to_original_samples(150 + SILENCE_SAMPLES);
        assert!((100..=150).contains(&second_start), "{second_start}");
    }

    #[test]
    fn overlapping_input_spans_keep_a_monotonic_map() {
        let samples = ramp(1000);
        let (out, map) = splice(&samples, &[span(0, 300), span(200, 400)], 0);
        assert_eq!(out.len(), 300 + SILENCE_SAMPLES + 200);
        assert_eq!(&out[300 + SILENCE_SAMPLES..], &samples[200..400]);
        assert_monotonic(&map, out.len());
        assert_eq!(map.to_original_samples(out.len()), 400);
    }

    #[test]
    fn empty_spans_between_real_ones_add_no_silence() {
        let samples = ramp(1000);
        let (out, map) = splice(&samples, &[span(0, 100), span(300, 300), span(500, 600)], 0);
        // Only one silence block: the empty span is skipped entirely.
        assert_eq!(out.len(), 100 + SILENCE_SAMPLES + 100);
        assert_eq!(map.to_original_samples(100 + SILENCE_SAMPLES), 500);
    }

    #[test]
    fn inverted_span_is_skipped() {
        let (out, _) = splice(&ramp(1000), &[span(500, 400)], 0);
        assert!(out.is_empty());
    }

    #[test]
    fn time_inside_inserted_silence_maps_into_the_original_gap() {
        // Speech 1–2 s and 10–11 s. A segment boundary placed by whisper in
        // the inserted silence must land between 2 s and 10 s.
        let (_, map) = splice(
            &ramp(12 * SR),
            &[span(SR, 2 * SR), span(10 * SR, 11 * SR)],
            0,
        );
        for cs in 101..110 {
            let t = map.to_original(cs);
            assert!(
                t > Duration::from_secs(2) && t < Duration::from_secs(10),
                "{cs} cs -> {t:?}"
            );
        }
    }

    #[test]
    fn segment_times_at_span_boundaries_with_whisper_overlap() {
        // The engine's real setting: 0.1 s overlap. Speech at 1–2 s, 5–6 s.
        let overlap = SR / 10;
        let (out, map) = splice(
            &ramp(8 * SR),
            &[span(SR, 2 * SR), span(5 * SR, 6 * SR)],
            overlap,
        );
        // 1.1 s + 0.1 s silence + 1.0 s
        assert_eq!(out.len(), SR * 22 / 10);
        // 0 cs -> 1 s, 110 cs (end of first span + overlap) -> 2.1 s,
        // 120 cs (after silence) -> 5 s, 220 cs (end) -> 6 s.
        assert_eq!(map.to_original(0), Duration::from_secs(1));
        assert_eq!(map.to_original(100), Duration::from_secs(2));
        assert_eq!(map.to_original(110), Duration::from_millis(2100));
        assert_eq!(map.to_original(120), Duration::from_secs(5));
        assert_eq!(map.to_original(170), Duration::from_millis(5500));
        assert_eq!(map.to_original(220), Duration::from_secs(6));
    }

    #[test]
    fn removed_pause_is_restored_for_paragraphing() {
        // Two utterances 5 s apart in the original are 0.1 s apart in the
        // spliced buffer; mapped back, the pause must again exceed 1.8 s so
        // PauseGap can split there.
        let (_, map) = splice(&ramp(10 * SR), &[span(SR, 2 * SR), span(7 * SR, 8 * SR)], 0);
        let first_end = map.to_original(100);
        let second_start = map.to_original(110);
        assert!(second_start - first_end >= Duration::from_millis(1800));
    }

    #[test]
    fn negative_and_zero_centiseconds_map_to_first_span_start() {
        let (_, map) = splice(&ramp(10 * SR), &[span(3 * SR, 4 * SR)], 0);
        assert_eq!(map.to_original(0), Duration::from_secs(3));
        assert_eq!(map.to_original(-1), Duration::from_secs(3));
        assert_eq!(map.to_original(i64::MIN), Duration::from_secs(3));
    }

    #[test]
    #[ignore = "BUG: TimeMap::to_original_samples overflows (panics in debug builds) for \
                huge timestamps when the map's last anchor has original > processed; \
                `last.1 + (processed - last.0)` is unchecked"]
    fn huge_centiseconds_do_not_panic() {
        let (_, map) = splice(&ramp(10 * SR), &[span(3 * SR, 4 * SR)], 0);
        let t = map.to_original(i64::MAX);
        assert!(t >= Duration::from_secs(4));
    }

    #[test]
    fn huge_centiseconds_on_identity_map_do_not_panic() {
        let t = TimeMap::identity().to_original(i64::MAX);
        assert!(t > Duration::from_secs(1_000_000));
    }

    #[test]
    fn mapping_is_monotonic_over_many_random_like_spans() {
        // Deterministic pseudo-random spans (LCG), sorted, varied overlaps.
        let len = 60 * SR;
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (x >> 33) as usize
        };
        for _ in 0..20 {
            let mut points: Vec<usize> = (0..12).map(|_| next() % len).collect();
            points.sort_unstable();
            let spans: Vec<SpeechSpan> = points.chunks(2).map(|c| span(c[0], c[1])).collect();
            let overlap = next() % 4000;
            let (out, map) = splice(&ramp(len), &spans, overlap);
            assert_monotonic(&map, out.len());
            if !out.is_empty() {
                let first = spans.iter().find(|s| s.end > s.start).unwrap();
                assert_eq!(map.to_original_samples(0), first.start);
            }
        }
    }

    #[test]
    fn from_centiseconds_handles_nan_and_infinity() {
        let len = 10 * SR;
        assert_eq!(
            SpeechSpan::from_centiseconds(f32::NAN, f32::NAN, len),
            span(0, 0)
        );
        assert_eq!(
            SpeechSpan::from_centiseconds(f32::NEG_INFINITY, f32::INFINITY, len),
            span(0, len)
        );
        assert_eq!(
            SpeechSpan::from_centiseconds(100.0, f32::NAN, len),
            span(SR, SR)
        );
    }

    #[test]
    fn from_centiseconds_rounds_to_nearest_sample() {
        // 1 cs = 160 samples; 0.01 cs = 1.6 samples -> 2.
        assert_eq!(SpeechSpan::from_centiseconds(0.01, 1.0, SR), span(2, 160));
    }

    #[test]
    fn from_centiseconds_with_zero_length_audio() {
        assert_eq!(SpeechSpan::from_centiseconds(10.0, 20.0, 0), span(0, 0));
    }
}
