//! The generic transcription driver.

use std::io::{self, Write};

use crate::protocol::{error_message, write_line, Message};
use crate::{Options, Paragrapher, Pcm16kMono, Progress, Transcriber};

/// Why [`run`] failed. An error line has already been written when possible.
#[derive(Debug, thiserror::Error)]
pub enum RunError<E: std::error::Error + Send + Sync + 'static> {
    /// The engine failed; an error line describing it was written.
    #[error("transcription failed")]
    Transcribe(#[source] E),
    /// Writing to the output failed.
    #[error("failed to write protocol output")]
    Io(#[source] io::Error),
}

/// Drives one transcription and writes protocol lines to `out`.
///
/// Progress lines are written (and flushed) as the engine reports them, with
/// consecutive duplicates collapsed and percentages capped at 100. On success
/// a result line follows; on engine failure an error line is written instead.
pub fn run<T: Transcriber, P: Paragrapher>(
    engine: &T,
    paragrapher: &P,
    pcm: &Pcm16kMono,
    opts: &Options,
    mut out: impl Write,
) -> Result<(), RunError<T::Error>> {
    let mut last: Option<Progress> = None;
    let mut write_error: Option<io::Error> = None;

    let outcome = engine.transcribe(pcm, opts, &mut |progress| {
        if write_error.is_some() {
            return;
        }
        let progress = match progress {
            Progress::Transcribing { percent } => Progress::Transcribing {
                percent: percent.min(100),
            },
            other => other,
        };
        if last == Some(progress) {
            return;
        }
        last = Some(progress);
        if let Err(e) = write_line(&mut out, &Message::Progress(progress)) {
            write_error = Some(e);
        }
    });

    match outcome {
        Ok(segments) => {
            if let Some(e) = write_error {
                return Err(RunError::Io(e));
            }
            let paragraphs = paragrapher.paragraphs(&segments);
            write_line(&mut out, &Message::Result { paragraphs }).map_err(RunError::Io)
        }
        Err(e) => {
            // Best effort: the engine error is the one worth reporting even if
            // the output channel is also broken.
            let _ = write_line(&mut out, &Message::error(error_message(&e)));
            Err(RunError::Transcribe(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use super::*;
    use crate::{PauseGap, Segment};

    #[derive(Debug, thiserror::Error)]
    #[error("fake engine failed")]
    struct FakeError;

    /// Reports `progress` in order, then returns `result`.
    struct Fake {
        progress: Vec<Progress>,
        result: Result<Vec<Segment>, ()>,
        seen_opts: RefCell<Option<Options>>,
    }

    impl Fake {
        fn ok(progress: Vec<Progress>, segments: Vec<Segment>) -> Self {
            Self {
                progress,
                result: Ok(segments),
                seen_opts: RefCell::new(None),
            }
        }

        fn failing(progress: Vec<Progress>) -> Self {
            Self {
                progress,
                result: Err(()),
                seen_opts: RefCell::new(None),
            }
        }
    }

    impl Transcriber for Fake {
        type Error = FakeError;

        fn transcribe(
            &self,
            _pcm: &Pcm16kMono,
            opts: &Options,
            progress: &mut dyn FnMut(Progress),
        ) -> Result<Vec<Segment>, FakeError> {
            *self.seen_opts.borrow_mut() = Some(opts.clone());
            for p in &self.progress {
                progress(*p);
            }
            self.result.clone().map_err(|()| FakeError)
        }
    }

    fn seg(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment {
            start: Duration::from_millis(start_ms),
            end: Duration::from_millis(end_ms),
            text: text.into(),
        }
    }

    fn run_to_string<T: Transcriber>(engine: &T) -> (Result<(), RunError<T::Error>>, String) {
        let mut out = Vec::new();
        let result = run(
            engine,
            &PauseGap::default(),
            &Pcm16kMono(vec![0.0; 16]),
            &Options::default(),
            &mut out,
        );
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn writes_progress_then_result() {
        let engine = Fake::ok(
            vec![
                Progress::Loading,
                Progress::Transcribing { percent: 0 },
                Progress::Transcribing { percent: 50 },
            ],
            vec![seg(0, 1000, " Hello"), seg(5000, 6000, "again ")],
        );
        let (result, out) = run_to_string(&engine);
        assert!(result.is_ok());
        assert_eq!(
            out,
            concat!(
                "{\"type\":\"progress\",\"stage\":\"loading\"}\n",
                "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":0}\n",
                "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":50}\n",
                "{\"type\":\"result\",\"paragraphs\":[\"Hello\",\"again\"]}\n",
            )
        );
    }

    #[test]
    fn collapses_duplicate_progress_and_caps_percent() {
        let engine = Fake::ok(
            vec![
                Progress::Loading,
                Progress::Loading,
                Progress::Transcribing { percent: 7 },
                Progress::Transcribing { percent: 7 },
                Progress::Transcribing { percent: 250 },
            ],
            vec![],
        );
        let (result, out) = run_to_string(&engine);
        assert!(result.is_ok());
        assert_eq!(
            out,
            concat!(
                "{\"type\":\"progress\",\"stage\":\"loading\"}\n",
                "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":7}\n",
                "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":100}\n",
                "{\"type\":\"result\",\"paragraphs\":[]}\n",
            )
        );
    }

    #[test]
    fn writes_error_line_on_failure() {
        let engine = Fake::failing(vec![Progress::Loading]);
        let (result, out) = run_to_string(&engine);
        assert!(matches!(result, Err(RunError::Transcribe(FakeError))));
        assert_eq!(
            out,
            concat!(
                "{\"type\":\"progress\",\"stage\":\"loading\"}\n",
                "{\"type\":\"error\",\"message\":\"fake engine failed\"}\n",
            )
        );
    }

    #[test]
    fn passes_options_through() {
        let engine = Fake::ok(vec![], vec![]);
        let opts = Options {
            language: Some("de".into()),
            initial_prompt: Some("Obsidian".into()),
        };
        run(
            &engine,
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &opts,
            io::sink(),
        )
        .unwrap();
        assert_eq!(engine.seen_opts.borrow().as_ref(), Some(&opts));
    }

    /// Counts lines written and flushes; each line is a single `write` call.
    #[derive(Default)]
    struct FlushCounter {
        lines: usize,
        flushes: usize,
    }

    impl Write for FlushCounter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.lines += buf.iter().filter(|&&b| b == b'\n').count();
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn every_line_is_flushed() {
        let engine = Fake::ok(vec![Progress::Loading], vec![seg(0, 1, "x")]);
        let mut out = FlushCounter::default();
        run(
            &engine,
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &Options::default(),
            &mut out,
        )
        .unwrap();
        assert_eq!(out.lines, 2);
        assert_eq!(out.flushes, 2);
    }

    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn output_failure_is_reported_as_io_error() {
        let engine = Fake::ok(vec![Progress::Loading], vec![seg(0, 1, "x")]);
        let result = run(
            &engine,
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &Options::default(),
            BrokenPipe,
        );
        assert!(matches!(result, Err(RunError::Io(_))));
    }

    // --- Adversarial cases added by the test pass. -------------------------

    use std::rc::Rc;

    fn lines(out: &str) -> Vec<&str> {
        out.split_terminator('\n').collect()
    }

    fn assert_every_line_is_json(out: &str) {
        assert!(out.is_empty() || out.ends_with('\n'), "{out:?}");
        for l in lines(out) {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            assert!(v.is_object(), "{l}");
        }
    }

    #[test]
    fn engine_emitting_no_progress_writes_only_result() {
        let engine = Fake::ok(vec![], vec![seg(0, 1000, "Only.")]);
        let (result, out) = run_to_string(&engine);
        assert!(result.is_ok());
        assert_eq!(out, "{\"type\":\"result\",\"paragraphs\":[\"Only.\"]}\n");
    }

    #[test]
    fn engine_failing_without_progress_writes_only_error() {
        let engine = Fake::failing(vec![]);
        let (result, out) = run_to_string(&engine);
        assert!(matches!(result, Err(RunError::Transcribe(FakeError))));
        assert_eq!(
            out,
            "{\"type\":\"error\",\"message\":\"fake engine failed\"}\n"
        );
    }

    #[test]
    fn error_path_writes_no_result_line() {
        let engine = Fake::failing(vec![
            Progress::Loading,
            Progress::Transcribing { percent: 99 },
        ]);
        let (_, out) = run_to_string(&engine);
        assert!(!out.contains("\"result\""), "{out}");
        assert_eq!(
            lines(&out).last().unwrap(),
            &"{\"type\":\"error\",\"message\":\"fake engine failed\"}"
        );
        assert_every_line_is_json(&out);
    }

    #[test]
    fn result_with_no_segments_is_an_empty_paragraph_list() {
        let engine = Fake::ok(vec![Progress::Loading], vec![]);
        let (result, out) = run_to_string(&engine);
        assert!(result.is_ok());
        assert_eq!(
            lines(&out).last().unwrap(),
            &"{\"type\":\"result\",\"paragraphs\":[]}"
        );
    }

    #[test]
    fn percent_101_and_255_are_capped_to_100() {
        for raw in [101u8, 200, 255] {
            let engine = Fake::ok(vec![Progress::Transcribing { percent: raw }], vec![]);
            let (_, out) = run_to_string(&engine);
            assert_eq!(
                lines(&out)[0],
                "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":100}",
                "raw {raw}"
            );
        }
    }

    #[test]
    fn capped_percent_collapses_with_a_real_100() {
        // 100, then 150 (capped to 100) are the same message after capping.
        let engine = Fake::ok(
            vec![
                Progress::Transcribing { percent: 100 },
                Progress::Transcribing { percent: 150 },
                Progress::Transcribing { percent: 255 },
            ],
            vec![],
        );
        let (_, out) = run_to_string(&engine);
        assert_eq!(
            out,
            concat!(
                "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":100}\n",
                "{\"type\":\"result\",\"paragraphs\":[]}\n",
            )
        );
    }

    #[test]
    fn non_consecutive_duplicates_are_not_collapsed() {
        let engine = Fake::ok(
            vec![
                Progress::Loading,
                Progress::Transcribing { percent: 5 },
                Progress::Loading,
                Progress::Transcribing { percent: 5 },
            ],
            vec![],
        );
        let (_, out) = run_to_string(&engine);
        assert_eq!(
            out,
            concat!(
                "{\"type\":\"progress\",\"stage\":\"loading\"}\n",
                "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":5}\n",
                "{\"type\":\"progress\",\"stage\":\"loading\"}\n",
                "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":5}\n",
                "{\"type\":\"result\",\"paragraphs\":[]}\n",
            )
        );
    }

    #[test]
    fn progress_order_is_preserved_even_when_decreasing() {
        // run() relays, it does not reorder or enforce monotonicity.
        let engine = Fake::ok(
            vec![
                Progress::Transcribing { percent: 60 },
                Progress::Transcribing { percent: 30 },
                Progress::Transcribing { percent: 90 },
            ],
            vec![],
        );
        let (_, out) = run_to_string(&engine);
        let percents: Vec<u64> = lines(&out)
            .iter()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| v["percent"].as_u64())
            .collect();
        assert_eq!(percents, vec![60, 30, 90]);
    }

    #[test]
    fn many_progress_updates_all_appear_before_result() {
        let progress: Vec<Progress> = (0..=100)
            .map(|percent| Progress::Transcribing { percent })
            .collect();
        let engine = Fake::ok(progress, vec![seg(0, 1, "x")]);
        let (_, out) = run_to_string(&engine);
        let ls = lines(&out);
        assert_eq!(ls.len(), 102);
        assert!(ls[101].starts_with("{\"type\":\"result\""));
        assert_every_line_is_json(&out);
    }

    /// A writer whose buffer the engine can inspect while it runs.
    #[derive(Clone, Default)]
    struct Shared(Rc<RefCell<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Records what had been written to `out` each time it reported progress.
    struct Peeking {
        out: Shared,
        seen: RefCell<Vec<String>>,
    }

    impl Transcriber for Peeking {
        type Error = FakeError;

        fn transcribe(
            &self,
            _: &Pcm16kMono,
            _: &Options,
            progress: &mut dyn FnMut(Progress),
        ) -> Result<Vec<Segment>, FakeError> {
            progress(Progress::Loading);
            self.seen
                .borrow_mut()
                .push(String::from_utf8(self.out.0.borrow().clone()).unwrap());
            progress(Progress::Transcribing { percent: 50 });
            self.seen
                .borrow_mut()
                .push(String::from_utf8(self.out.0.borrow().clone()).unwrap());
            Ok(vec![])
        }
    }

    #[test]
    fn progress_is_streamed_while_the_engine_runs() {
        let out = Shared::default();
        let engine = Peeking {
            out: out.clone(),
            seen: RefCell::new(Vec::new()),
        };
        run(
            &engine,
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &Options::default(),
            out.clone(),
        )
        .unwrap();
        let seen = engine.seen.borrow();
        assert_eq!(seen[0], "{\"type\":\"progress\",\"stage\":\"loading\"}\n");
        assert_eq!(
            seen[1],
            concat!(
                "{\"type\":\"progress\",\"stage\":\"loading\"}\n",
                "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":50}\n",
            )
        );
    }

    /// Records the segments it was given.
    struct RecordingParagrapher(RefCell<Option<Vec<Segment>>>);

    impl Paragrapher for RecordingParagrapher {
        fn paragraphs(&self, segments: &[Segment]) -> Vec<String> {
            *self.0.borrow_mut() = Some(segments.to_vec());
            vec!["p\"1\"".into(), "p\n2".into()]
        }
    }

    #[test]
    fn engine_segments_go_to_the_paragrapher_and_its_output_is_the_result() {
        let segments = vec![seg(0, 10, " a "), seg(5000, 6000, "b")];
        let engine = Fake::ok(vec![], segments.clone());
        let paragrapher = RecordingParagrapher(RefCell::new(None));
        let mut out = Vec::new();
        run(
            &engine,
            &paragrapher,
            &Pcm16kMono::default(),
            &Options::default(),
            &mut out,
        )
        .unwrap();
        assert_eq!(paragrapher.0.borrow().as_ref(), Some(&segments));
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\"type\":\"result\",\"paragraphs\":[\"p\\\"1\\\"\",\"p\\n2\"]}\n"
        );
    }

    #[test]
    fn paragrapher_is_not_called_on_engine_failure() {
        let paragrapher = RecordingParagrapher(RefCell::new(None));
        let _ = run(
            &Fake::failing(vec![]),
            &paragrapher,
            &Pcm16kMono::default(),
            &Options::default(),
            io::sink(),
        );
        assert!(paragrapher.0.borrow().is_none());
    }

    #[test]
    fn custom_pause_gap_is_honoured() {
        let engine = Fake::ok(vec![], vec![seg(0, 1000, "a"), seg(1300, 2000, "b")]);
        let mut out = Vec::new();
        run(
            &engine,
            &PauseGap::new(Duration::from_millis(300)),
            &Pcm16kMono::default(),
            &Options::default(),
            &mut out,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\"type\":\"result\",\"paragraphs\":[\"a\",\"b\"]}\n"
        );
    }

    /// Passes the pcm it received back as the text of one segment.
    struct EchoLen;

    impl Transcriber for EchoLen {
        type Error = FakeError;
        fn transcribe(
            &self,
            pcm: &Pcm16kMono,
            _: &Options,
            _: &mut dyn FnMut(Progress),
        ) -> Result<Vec<Segment>, FakeError> {
            Ok(vec![seg(0, 1, &pcm.0.len().to_string())])
        }
    }

    #[test]
    fn pcm_is_passed_to_the_engine_unchanged() {
        let mut out = Vec::new();
        run(
            &EchoLen,
            &PauseGap::default(),
            &Pcm16kMono(vec![0.25; 12_345]),
            &Options::default(),
            &mut out,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\"type\":\"result\",\"paragraphs\":[\"12345\"]}\n"
        );
    }

    /// An engine error with a nested source and awkward characters.
    #[derive(Debug, thiserror::Error)]
    #[error("load \"model\"\nfailed")]
    struct NastyError(#[source] NastyCause);

    #[derive(Debug, thiserror::Error)]
    #[error("caf\u{e9} \u{1F399} \\ bad magic")]
    struct NastyCause;

    struct FailsNastily;

    impl Transcriber for FailsNastily {
        type Error = NastyError;
        fn transcribe(
            &self,
            _: &Pcm16kMono,
            _: &Options,
            _: &mut dyn FnMut(Progress),
        ) -> Result<Vec<Segment>, NastyError> {
            Err(NastyError(NastyCause))
        }
    }

    #[test]
    fn error_line_includes_source_chain_and_is_escaped_on_one_line() {
        let (result, out) = run_to_string(&FailsNastily);
        assert!(matches!(result, Err(RunError::Transcribe(_))));
        assert_eq!(
            out,
            "{\"type\":\"error\",\"message\":\"load \\\"model\\\"\\nfailed: caf\u{e9} \u{1F399} \\\\ bad magic\"}\n"
        );
    }

    /// Accepts `ok_lines` lines, then fails every write.
    struct FailAfter {
        ok_lines: usize,
        written: Vec<u8>,
        attempts_after_failure: usize,
    }

    impl FailAfter {
        fn new(ok_lines: usize) -> Self {
            Self {
                ok_lines,
                written: Vec::new(),
                attempts_after_failure: 0,
            }
        }
    }

    impl Write for FailAfter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let lines = self.written.iter().filter(|&&b| b == b'\n').count();
            if lines >= self.ok_lines {
                self.attempts_after_failure += 1;
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn out_failing_mid_progress_stops_writing_and_returns_io_error() {
        let engine = Fake::ok(
            vec![
                Progress::Loading,
                Progress::Transcribing { percent: 10 },
                Progress::Transcribing { percent: 20 },
                Progress::Transcribing { percent: 30 },
            ],
            vec![seg(0, 1, "x")],
        );
        let mut out = FailAfter::new(1);
        let result = run(
            &engine,
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &Options::default(),
            &mut out,
        );
        assert!(matches!(result, Err(RunError::Io(_))), "{result:?}");
        assert_eq!(
            String::from_utf8(out.written.clone()).unwrap(),
            "{\"type\":\"progress\",\"stage\":\"loading\"}\n"
        );
        // After the first failure run() must not keep hammering a dead pipe,
        // and must not try to write a result line.
        assert_eq!(out.attempts_after_failure, 1);
    }

    #[test]
    fn out_failing_only_on_result_line_returns_io_error() {
        let engine = Fake::ok(vec![Progress::Loading], vec![seg(0, 1, "x")]);
        let mut out = FailAfter::new(1);
        let result = run(
            &engine,
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &Options::default(),
            &mut out,
        );
        assert!(matches!(result, Err(RunError::Io(_))), "{result:?}");
    }

    #[test]
    fn engine_error_wins_over_broken_output() {
        // Both the engine and the pipe fail: the engine error is what matters.
        let engine = Fake::failing(vec![Progress::Loading]);
        let result = run(
            &engine,
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &Options::default(),
            BrokenPipe,
        );
        assert!(
            matches!(result, Err(RunError::Transcribe(FakeError))),
            "{result:?}"
        );
    }

    #[test]
    fn engine_error_line_is_attempted_after_a_progress_write_failure() {
        // The error line is best effort; with a writer that recovers it lands.
        struct FailFirst {
            failed: bool,
            written: Vec<u8>,
        }
        impl Write for FailFirst {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                if !self.failed {
                    self.failed = true;
                    return Err(io::ErrorKind::WouldBlock.into());
                }
                self.written.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut out = FailFirst {
            failed: false,
            written: Vec::new(),
        };
        let result = run(
            &Fake::failing(vec![Progress::Loading]),
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &Options::default(),
            &mut out,
        );
        assert!(matches!(result, Err(RunError::Transcribe(FakeError))));
        assert_eq!(
            String::from_utf8(out.written).unwrap(),
            "{\"type\":\"error\",\"message\":\"fake engine failed\"}\n"
        );
    }

    #[test]
    fn flush_failure_is_an_io_error() {
        struct FailFlush;
        impl Write for FailFlush {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }
        let result = run(
            &Fake::ok(vec![], vec![]),
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &Options::default(),
            FailFlush,
        );
        assert!(matches!(result, Err(RunError::Io(_))), "{result:?}");
    }

    #[test]
    fn io_error_preserves_the_underlying_kind() {
        let result = run(
            &Fake::ok(vec![], vec![]),
            &PauseGap::default(),
            &Pcm16kMono::default(),
            &Options::default(),
            BrokenPipe,
        );
        let Err(RunError::Io(e)) = result else {
            panic!("expected Io error, got {result:?}");
        };
        assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn run_error_display_includes_engine_error_via_chain() {
        let (result, _) = run_to_string(&Fake::failing(vec![]));
        let e = result.unwrap_err();
        assert_eq!(
            error_message(&e),
            "transcription failed: fake engine failed"
        );
    }

    #[test]
    fn output_is_only_json_lines_in_every_path() {
        let cases: Vec<Fake> = vec![
            Fake::ok(vec![], vec![]),
            Fake::ok(vec![Progress::Loading], vec![seg(0, 1, "a\nb \"c\"")]),
            Fake::failing(vec![]),
            Fake::failing(vec![Progress::Transcribing { percent: 255 }]),
        ];
        for engine in &cases {
            let (_, out) = run_to_string(engine);
            assert_every_line_is_json(&out);
        }
    }
}
