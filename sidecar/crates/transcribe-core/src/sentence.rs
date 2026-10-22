//! Splitting a transcript into sentences and measuring the pause after each.
//!
//! Paragraph breaks are only considered at sentence ends, so a pause to think
//! mid-sentence never splits a paragraph. Pauses come from word timing and VAD
//! silences, not segment timestamps: whisper.cpp segments (and tokens) abut,
//! with the silence absorbed into one side, so the gap between consecutive
//! segments is almost always zero.

use std::time::Duration;

use crate::{Silence, Transcript};

/// A sentence and the pause that follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    /// Trimmed, non-empty, and without line breaks when produced by [`sentences`].
    pub text: String,
    /// Silence between this sentence's last spoken word and the next
    /// sentence's first. Zero for the last sentence.
    pub pause_after: Duration,
}

/// How far either side of the gap between two words to count VAD silence.
/// whisper.cpp token timestamps are approximate and often give a pause to the
/// punctuation token, so the silence can start before the last word's end or
/// end after the next word's start.
const SILENCE_SLACK: Duration = Duration::from_millis(300);

/// A pause this long ends a sentence even without punctuation, so a stretch
/// Whisper did not punctuate can still be split. Shorter pauses mid-sentence
/// are taken as thinking pauses.
pub const UNPUNCTUATED_BREAK: Duration = Duration::from_secs(3);

/// Characters that end a sentence, including CJK, Arabic and Devanagari forms.
const SENTENCE_END: [char; 9] = ['.', '?', '!', '…', '。', '？', '！', '؟', '।'];

/// Closing punctuation that may follow a sentence end, as in `"Done."`.
const CLOSING: [char; 9] = ['"', '\'', '”', '’', '»', ')', ']', '」', '』'];

/// Zero-width and format characters that `str::trim` keeps but that render as nothing.
const INVISIBLE: [char; 5] = ['\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}'];

/// Splits `transcript` into sentences, in order.
///
/// Rules:
/// - a sentence ends at a word whose text ends with sentence-ending
///   punctuation, optionally followed by closing quotes or brackets, or
///   before a spoken word that follows a pause of at least
///   [`UNPUNCTUATED_BREAK`];
/// - punctuation glued to the end of a sentence (no leading space) stays with
///   it, so a closing quote emitted as its own token is not moved to the next
///   sentence;
/// - words within a segment are concatenated as-is (they carry their own
///   spaces); segments are trimmed and joined with a single space;
/// - text after the last sentence end becomes a final sentence;
/// - sentences that are empty after [`clean_text`] are dropped.
///
/// A segment without words is treated as one word spanning the segment.
pub fn sentences(transcript: &Transcript) -> Vec<Sentence> {
    let mut builder = Builder::new(&transcript.silences);
    for segment in &transcript.segments {
        builder.start_segment();
        if segment.words.is_empty() {
            builder.push(&segment.text, segment.start, segment.end);
        } else {
            for word in &segment.words {
                builder.push(&word.text, word.start, word.end);
            }
        }
    }
    builder.finish()
}

#[derive(Debug, Default)]
struct Open {
    text: String,
    /// Latest end of a spoken word in this sentence.
    end: Option<Duration>,
}

struct Builder<'a> {
    silences: &'a [Silence],
    out: Vec<Sentence>,
    current: Open,
    /// A sentence that has ended, held until the next spoken word so the pause after it can be measured.
    ended: Option<Open>,
    at_segment_start: bool,
}

impl<'a> Builder<'a> {
    fn new(silences: &'a [Silence]) -> Self {
        Self {
            silences,
            out: Vec::new(),
            current: Open::default(),
            ended: None,
            at_segment_start: false,
        }
    }

    fn start_segment(&mut self) {
        self.at_segment_start = true;
    }

    fn push(&mut self, text: &str, start: Duration, end: Duration) {
        let segment_start = std::mem::take(&mut self.at_segment_start);
        let spoken = is_spoken(text);

        if !spoken && !segment_start && !text.starts_with(char::is_whitespace) {
            if let Some(ended) = self.ended.as_mut() {
                ended.text.push_str(text);
                return;
            }
        }

        if spoken {
            if let Some(ended) = self.ended.take() {
                let pause = ended
                    .end
                    .map_or(Duration::ZERO, |e| pause_between(e, start, self.silences));
                self.emit(&ended.text, pause);
            } else if let Some(pause) = self
                .current
                .end
                .map(|e| pause_between(e, start, self.silences))
                .filter(|pause| *pause >= UNPUNCTUATED_BREAK)
            {
                let done = std::mem::take(&mut self.current);
                self.emit(&done.text, pause);
            }
            self.current.end = Some(self.current.end.map_or(end, |e| e.max(end)));
        }

        append(&mut self.current.text, text, segment_start);

        if ends_sentence(text) {
            let done = std::mem::take(&mut self.current);
            match self.ended.as_mut() {
                // Only possible when `done` has no spoken word: keep the punctuation with the sentence before.
                Some(ended) => append(&mut ended.text, &done.text, true),
                None => self.ended = Some(done),
            }
        }
    }

    fn finish(mut self) -> Vec<Sentence> {
        if let Some(ended) = self.ended.take() {
            self.emit(&ended.text, Duration::ZERO);
        }
        let rest = std::mem::take(&mut self.current);
        self.emit(&rest.text, Duration::ZERO);
        self.out
    }

    fn emit(&mut self, text: &str, pause_after: Duration) {
        let text = clean_text(text);
        if !text.is_empty() {
            self.out.push(Sentence { text, pause_after });
        }
    }
}

/// Appends `piece`; at a segment boundary, trims both sides of the join and
/// separates them with one space.
fn append(text: &mut String, piece: &str, segment_start: bool) {
    if !segment_start {
        text.push_str(piece);
        return;
    }
    let piece = piece.trim_start();
    if piece.is_empty() {
        return;
    }
    text.truncate(text.trim_end().len());
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(piece);
}

fn is_spoken(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(|c: char| c.is_whitespace() || INVISIBLE.contains(&c))
        .trim_end_matches(CLOSING)
        .ends_with(SENTENCE_END)
}

/// The pause between a word ending at `end` and the next starting at
/// `next_start`: the larger of the gap in word timing and the VAD silence
/// around it.
fn pause_between(end: Duration, next_start: Duration, silences: &[Silence]) -> Duration {
    let word_gap = next_start.saturating_sub(end);
    let from = end.saturating_sub(SILENCE_SLACK);
    let to = next_start.saturating_add(SILENCE_SLACK);
    let silent = silences
        .iter()
        .map(|s| s.end.min(to).saturating_sub(s.start.max(from)))
        .fold(Duration::ZERO, Duration::saturating_add);
    word_gap.max(silent)
}

/// Trims `text`, removes invisible characters from its ends, and collapses
/// every whitespace run containing a line break to a single space so a
/// paragraph never contains a blank line. Empty if only invisibles remain.
pub(crate) fn clean_text(text: &str) -> String {
    let text = text.trim_matches(|c: char| c.is_whitespace() || INVISIBLE.contains(&c));
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            run.push(c);
            continue;
        }
        flush_run(&mut out, &mut run);
        out.push(c);
    }
    flush_run(&mut out, &mut run);
    if out.chars().all(|c| INVISIBLE.contains(&c)) {
        out.clear();
    }
    out
}

fn flush_run(out: &mut String, run: &mut String) {
    if run.contains(['\n', '\r']) {
        out.push(' ');
    } else {
        out.push_str(run);
    }
    run.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Segment, Word};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn word(start_ms: u64, end_ms: u64, text: &str) -> Word {
        Word {
            start: ms(start_ms),
            end: ms(end_ms),
            text: text.to_string(),
        }
    }

    /// A segment spanning its words, as whisper.cpp reports them.
    fn seg_words(words: Vec<Word>) -> Segment {
        Segment {
            start: words.first().map_or(Duration::ZERO, |w| w.start),
            end: words.last().map_or(Duration::ZERO, |w| w.end),
            text: words.iter().map(|w| w.text.as_str()).collect(),
            words,
        }
    }

    /// A segment without word timing.
    fn seg(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment {
            start: ms(start_ms),
            end: ms(end_ms),
            text: text.to_string(),
            words: Vec::new(),
        }
    }

    fn silence(start_ms: u64, end_ms: u64) -> Silence {
        Silence {
            start: ms(start_ms),
            end: ms(end_ms),
        }
    }

    fn transcript(segments: Vec<Segment>, silences: Vec<Silence>) -> Transcript {
        Transcript { segments, silences }
    }

    fn texts(sentences: &[Sentence]) -> Vec<&str> {
        sentences.iter().map(|s| s.text.as_str()).collect()
    }

    fn pauses(sentences: &[Sentence]) -> Vec<Duration> {
        sentences.iter().map(|s| s.pause_after).collect()
    }

    #[test]
    fn empty_transcript_gives_no_sentences() {
        assert!(sentences(&Transcript::default()).is_empty());
    }

    #[test]
    fn pause_comes_from_word_timing_when_segments_abut() {
        // The original bug: whisper.cpp segment timestamps touch (0..2000,
        // 2000..4000), but the words inside show a 1.2 s pause.
        let t = transcript(
            vec![
                seg_words(vec![
                    word(0, 600, " Hello"),
                    word(600, 800, " there"),
                    word(800, 2000, "."),
                ]),
                seg_words(vec![word(2000, 2400, " Next"), word(2400, 4000, ".")]),
            ],
            vec![],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["Hello there.", "Next."]);
        assert_eq!(pauses(&s), vec![ms(1200), Duration::ZERO]);
    }

    #[test]
    fn pause_inside_absorbing_punctuation_counts_as_word_gap() {
        // As seen in real output: the "." token runs to the next word, but the
        // last spoken word ends 2.1 s before it.
        let t = transcript(
            vec![seg_words(vec![
                word(0, 900, " Done"),
                word(900, 3000, "."),
                word(3000, 3400, " Then"),
                word(3400, 3500, "."),
            ])],
            vec![],
        );
        assert_eq!(pauses(&sentences(&t))[0], ms(2100));
    }

    #[test]
    fn pause_comes_from_vad_silence_when_word_timing_hides_it() {
        // Word timing shows only 100 ms, but VAD found silence from 0.9 s to
        // 1.5 s, which overlaps the boundary's slack window by 600 ms.
        let t = transcript(
            vec![seg_words(vec![
                word(0, 1200, " Done"),
                word(1200, 1300, "."),
                word(1300, 1600, " Then"),
                word(1600, 1700, "."),
            ])],
            vec![silence(900, 1500)],
        );
        assert_eq!(pauses(&sentences(&t))[0], ms(600));
    }

    #[test]
    fn pause_is_the_larger_of_word_gap_and_silence() {
        let words = vec![
            word(0, 1000, " A"),
            word(1000, 1000, "."),
            word(1800, 2000, " B."),
        ];
        let t = transcript(vec![seg_words(words.clone())], vec![silence(1100, 1400)]);
        assert_eq!(pauses(&sentences(&t))[0], ms(800));
        let t = transcript(vec![seg_words(words)], vec![silence(900, 2100)]);
        assert_eq!(pauses(&sentences(&t))[0], ms(1200));
    }

    #[test]
    fn silence_counts_only_near_the_sentence_boundary() {
        // Silences well before the last word and well after the next one are ignored;
        // only the 200 ms inside the slack window counts.
        let t = transcript(
            vec![seg_words(vec![
                word(0, 5000, " Long"),
                word(5000, 5000, "."),
                word(5000, 9000, " Next."),
            ])],
            vec![
                silence(1000, 3000),
                silence(5100, 5300),
                silence(7000, 8000),
            ],
        );
        assert_eq!(pauses(&sentences(&t))[0], ms(200));
    }

    #[test]
    fn pause_inside_a_sentence_is_not_a_sentence_boundary() {
        let t = transcript(
            vec![seg_words(vec![
                word(0, 500, " I"),
                word(500, 900, " think"),
                word(3400, 3900, " so"),
                word(3900, 4000, "."),
            ])],
            vec![silence(950, 3350)],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["I think so."]);
        assert_eq!(pauses(&s), vec![Duration::ZERO]);
    }

    #[test]
    fn very_long_pause_ends_an_unpunctuated_sentence() {
        let t = transcript(
            vec![
                seg_words(vec![word(0, 500, " and so"), word(500, 900, " ask not")]),
                seg_words(vec![
                    word(3900, 4300, " and so"),
                    word(4300, 4700, " ask not"),
                ]),
            ],
            vec![],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["and so ask not", "and so ask not"]);
        assert_eq!(pauses(&s), vec![UNPUNCTUATED_BREAK, Duration::ZERO]);
    }

    #[test]
    fn shorter_pause_does_not_end_an_unpunctuated_sentence() {
        let t = transcript(
            vec![
                seg_words(vec![word(0, 500, " and so")]),
                seg_words(vec![word(3499, 4000, " ask not")]),
            ],
            vec![],
        );
        assert_eq!(texts(&sentences(&t)), vec!["and so ask not"]);
    }

    #[test]
    fn sentence_ends_on_question_exclamation_ellipsis_and_cjk_marks() {
        let t = transcript(
            vec![seg_words(vec![
                word(0, 100, " Why?"),
                word(100, 200, " Yes!"),
                word(200, 300, " Well…"),
                word(300, 400, "はい。"),
                word(400, 500, "本当？"),
            ])],
            vec![],
        );
        assert_eq!(
            texts(&sentences(&t)),
            vec!["Why?", "Yes!", "Well…", "はい。", "本当？"]
        );
    }

    #[test]
    fn closing_quote_token_stays_with_its_sentence() {
        let t = transcript(
            vec![seg_words(vec![
                word(0, 500, " \"Stop"),
                word(500, 600, "."),
                word(600, 700, "\""),
                word(2000, 2500, " She"),
                word(2500, 2900, " left."),
            ])],
            vec![],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["\"Stop.\"", "She left."]);
        // Measured from the last spoken word, not the quote.
        assert_eq!(s[0].pause_after, ms(1500));
    }

    #[test]
    fn opening_quote_after_a_space_starts_the_next_sentence() {
        let t = transcript(
            vec![seg_words(vec![
                word(0, 500, " Fine."),
                word(500, 600, " \""),
                word(600, 900, "Go"),
                word(900, 950, ".\""),
            ])],
            vec![],
        );
        assert_eq!(texts(&sentences(&t)), vec!["Fine.", "\"Go.\""]);
    }

    #[test]
    fn sentence_ending_inside_closing_quote_is_detected() {
        let t = transcript(
            vec![seg(0, 1000, "He said \"no.\""), seg(3000, 4000, "Then")],
            vec![],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["He said \"no.\"", "Then"]);
        assert_eq!(s[0].pause_after, ms(2000));
    }

    #[test]
    fn text_without_final_punctuation_is_the_last_sentence() {
        let t = transcript(
            vec![seg(0, 1000, " One."), seg(1000, 2000, " and then")],
            vec![],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["One.", "and then"]);
        assert_eq!(pauses(&s), vec![Duration::ZERO, Duration::ZERO]);
    }

    #[test]
    fn segments_without_words_are_trimmed_and_joined_with_one_space() {
        let t = transcript(
            vec![
                seg(0, 1000, " Hello"),
                seg(1500, 2000, "world. "),
                seg(2000, 3000, "  a  b "),
                seg(3000, 4000, " c\td."),
            ],
            vec![],
        );
        assert_eq!(texts(&sentences(&t)), vec!["Hello world.", "a  b c\td."]);
    }

    #[test]
    fn words_within_a_segment_keep_their_own_spacing() {
        let t = transcript(
            vec![seg_words(vec![
                word(0, 100, "今日"),
                word(100, 200, "は"),
                word(200, 300, "晴れ"),
                word(300, 400, "。"),
            ])],
            vec![],
        );
        assert_eq!(texts(&sentences(&t)), vec!["今日は晴れ。"]);
    }

    #[test]
    fn unicode_whitespace_is_trimmed() {
        // Ideographic space, no-break space, em space, line separator.
        let t = transcript(
            vec![
                seg(0, 1000, "\u{3000}こんにちは\u{00A0}"),
                seg(1100, 2000, "\u{2003}مرحبا\u{2028}"),
            ],
            vec![],
        );
        assert_eq!(texts(&sentences(&t)), vec!["こんにちは مرحبا"]);
    }

    #[test]
    fn sentence_never_contains_a_line_break() {
        // The plugin separates paragraphs with blank lines (docs/plan.md,
        // "Insertion"), so a line break inside a sentence could split one.
        let t = transcript(
            vec![seg(0, 1000, "One\n\ntwo"), seg(1000, 2000, "\r\nthree.\n")],
            vec![],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["One two three."]);
    }

    #[test]
    fn whitespace_and_invisible_only_segments_give_no_sentences() {
        let t = transcript(
            vec![
                seg(0, 1000, ""),
                seg(1000, 2000, "   \n\t"),
                seg(2000, 3000, "\u{3000}\u{00A0}"),
                seg(3000, 4000, "\u{200B}\u{FEFF} \u{2060}\u{200C}\u{200D}"),
            ],
            vec![silence(0, 4000)],
        );
        assert!(sentences(&t).is_empty());
    }

    #[test]
    fn invisible_only_segment_between_sentences_is_dropped() {
        let t = transcript(
            vec![
                seg(0, 1000, "One."),
                seg(1000, 1500, "\u{200B}\u{FEFF} \u{2060}"),
                seg(3000, 3500, "Two."),
            ],
            vec![],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["One.", "Two."]);
        assert_eq!(s[0].pause_after, ms(2000));
    }

    #[test]
    fn punctuation_only_sentence_end_stays_with_the_sentence_before() {
        let t = transcript(
            vec![
                seg(0, 1000, "Wait."),
                seg(1000, 1200, " ..."),
                seg(3000, 3500, "Okay."),
            ],
            vec![],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["Wait. ...", "Okay."]);
        assert_eq!(s[0].pause_after, ms(2000));
    }

    #[test]
    fn pause_is_measured_from_the_latest_spoken_end_in_the_sentence() {
        // A long segment followed by a short one nested inside it: the pause
        // before "C." starts at A's end (10 s), so it is zero.
        let t = transcript(
            vec![
                seg(0, 10_000, "A"),
                seg(2000, 3000, "B."),
                seg(5000, 6000, "C."),
            ],
            vec![],
        );
        assert_eq!(pauses(&sentences(&t))[0], Duration::ZERO);
    }

    #[test]
    fn out_of_order_and_inverted_timestamps_give_zero_pause() {
        let t = transcript(
            vec![
                seg(5000, 6000, "Later."),
                seg(0, 1000, "Earlier."),
                seg(9000, 8000, "Odd."),
                seg(9500, 9600, "End."),
            ],
            vec![silence(3000, 2000)],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["Later.", "Earlier.", "Odd.", "End."]);
        assert_eq!(s[0].pause_after, Duration::ZERO);
        // "Odd." ends at 8 s (its inverted end); "End." starts at 9.5 s.
        assert_eq!(s[2].pause_after, ms(1500));
    }

    #[test]
    fn huge_timestamps_do_not_overflow() {
        let max = Duration::MAX;
        let t = transcript(
            vec![
                Segment {
                    start: max - Duration::from_secs(10),
                    end: max - Duration::from_secs(9),
                    text: "One.".into(),
                    words: Vec::new(),
                },
                Segment {
                    start: max - Duration::from_secs(1),
                    end: max,
                    text: "Two.".into(),
                    words: Vec::new(),
                },
            ],
            vec![
                Silence {
                    start: Duration::ZERO,
                    end: max,
                },
                Silence {
                    start: max - Duration::from_secs(9),
                    end: max,
                },
            ],
        );
        let s = sentences(&t);
        assert_eq!(texts(&s), vec!["One.", "Two."]);
        assert!(s[0].pause_after >= Duration::from_secs(8));
    }
}
