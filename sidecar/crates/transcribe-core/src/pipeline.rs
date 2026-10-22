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
}
