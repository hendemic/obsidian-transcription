//! Sidecar protocol messages: one JSON object per line on stdout.
//!
//! ```text
//! {"type":"progress","stage":"loading"}
//! {"type":"progress","stage":"transcribing","percent":42}
//! {"type":"result","paragraphs":["…","…"]}
//! {"type":"error","message":"…"}
//! ```
//!
//! `transcribe-cli probe` prints a single [`ProbeResponse`] instead:
//! `{"version":"0.1.0","backend":"cpu"}`.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use crate::Progress;

/// A line written by `transcribe-cli transcribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Message {
    Progress(Progress),
    Result { paragraphs: Vec<String> },
    Error { message: String },
}

impl Message {
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

/// The output of `transcribe-cli probe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeResponse {
    pub version: String,
    /// `"metal+coreml"`, `"cuda"`, `"vulkan"` or `"cpu"`.
    pub backend: String,
}

/// Writes `value` as one JSON line and flushes.
pub fn write_line<T: Serialize + ?Sized>(out: &mut impl Write, value: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    out.write_all(&line)?;
    out.flush()
}

/// Formats an error and its chain of sources as `"outer: inner: root"`.
pub fn error_message(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        // Skip causes that the outer message already spells out.
        if !message.ends_with(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line<T: Serialize>(value: &T) -> String {
        let mut out = Vec::new();
        write_line(&mut out, value).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn progress_loading_line() {
        assert_eq!(
            line(&Message::Progress(Progress::Loading)),
            "{\"type\":\"progress\",\"stage\":\"loading\"}\n"
        );
    }

    #[test]
    fn progress_transcribing_line() {
        assert_eq!(
            line(&Message::Progress(Progress::Transcribing { percent: 42 })),
            "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":42}\n"
        );
    }

    #[test]
    fn result_line() {
        let msg = Message::Result {
            paragraphs: vec!["One.".into(), "Two \"quoted\".".into()],
        };
        assert_eq!(
            line(&msg),
            "{\"type\":\"result\",\"paragraphs\":[\"One.\",\"Two \\\"quoted\\\".\"]}\n"
        );
    }

    #[test]
    fn empty_result_line() {
        let msg = Message::Result { paragraphs: vec![] };
        assert_eq!(line(&msg), "{\"type\":\"result\",\"paragraphs\":[]}\n");
    }

    #[test]
    fn error_line() {
        assert_eq!(
            line(&Message::error("boom")),
            "{\"type\":\"error\",\"message\":\"boom\"}\n"
        );
    }

    #[test]
    fn probe_line() {
        let probe = ProbeResponse {
            version: "0.1.0".into(),
            backend: "cpu".into(),
        };
        assert_eq!(
            line(&probe),
            "{\"version\":\"0.1.0\",\"backend\":\"cpu\"}\n"
        );
    }

    #[test]
    fn messages_round_trip() {
        for msg in [
            Message::Progress(Progress::Loading),
            Message::Progress(Progress::Transcribing { percent: 100 }),
            Message::Result {
                paragraphs: vec!["a".into()],
            },
            Message::error("e"),
        ] {
            let text = line(&msg);
            assert_eq!(serde_json::from_str::<Message>(&text).unwrap(), msg);
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Inner);

    #[derive(Debug, thiserror::Error)]
    #[error("inner")]
    struct Inner;

    #[test]
    fn error_message_includes_sources() {
        assert_eq!(error_message(&Outer(Inner)), "outer: inner");
    }

    // --- Adversarial cases added by the test pass. -------------------------
    //
    // Expected strings are hand-written JSON (RFC 8259), not produced by
    // serializing the same types.

    /// Asserts `text` is exactly one line: ends with `\n`, contains no other
    /// line break, and parses as a JSON object.
    fn assert_one_json_object_line(text: &str) -> serde_json::Value {
        assert!(text.ends_with('\n'), "{text:?}");
        let body = &text[..text.len() - 1];
        assert!(!body.contains('\n'), "embedded LF in {text:?}");
        assert!(!body.contains('\r'), "embedded CR in {text:?}");
        let value: serde_json::Value = serde_json::from_str(body).unwrap();
        assert!(value.is_object(), "{text:?}");
        value
    }

    #[test]
    fn progress_transcribing_bounds() {
        assert_eq!(
            line(&Message::Progress(Progress::Transcribing { percent: 0 })),
            "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":0}\n"
        );
        assert_eq!(
            line(&Message::Progress(Progress::Transcribing { percent: 100 })),
            "{\"type\":\"progress\",\"stage\":\"transcribing\",\"percent\":100}\n"
        );
    }

    #[test]
    fn probe_line_for_every_documented_backend() {
        for backend in ["metal+coreml", "cuda", "vulkan", "cpu"] {
            let probe = ProbeResponse {
                version: "0.1.0".into(),
                backend: backend.into(),
            };
            assert_eq!(
                line(&probe),
                format!("{{\"version\":\"0.1.0\",\"backend\":\"{backend}\"}}\n")
            );
        }
    }

    #[test]
    fn result_paragraph_newlines_and_tabs_are_escaped() {
        let msg = Message::Result {
            paragraphs: vec!["line1\nline2\r\n\tend".into()],
        };
        assert_eq!(
            line(&msg),
            "{\"type\":\"result\",\"paragraphs\":[\"line1\\nline2\\r\\n\\tend\"]}\n"
        );
    }

    #[test]
    fn result_paragraph_backslashes_and_quotes_are_escaped() {
        let msg = Message::Result {
            paragraphs: vec![r#"C:\path "q" \n"#.into()],
        };
        assert_eq!(
            line(&msg),
            "{\"type\":\"result\",\"paragraphs\":[\"C:\\\\path \\\"q\\\" \\\\n\"]}\n"
        );
    }

    #[test]
    fn control_characters_are_escaped() {
        let msg = Message::error("nul\u{0}bell\u{7}esc\u{1b}del\u{7f}");
        let text = line(&msg);
        // C0 controls must be escaped (RFC 8259 §7); DEL need not be.
        assert_eq!(
            text,
            "{\"type\":\"error\",\"message\":\"nul\\u0000bell\\u0007esc\\u001bdel\u{7f}\"}\n"
        );
        assert_one_json_object_line(&text);
    }

    #[test]
    fn unicode_paragraphs_survive_as_utf8() {
        let paragraphs = vec![
            "café 🎙️ naïve".to_string(),
            "مرحبا بالعالم".to_string(),
            "zero\u{200B}width\u{FEFF}".to_string(),
            "日本語のテキスト".to_string(),
        ];
        let text = line(&Message::Result {
            paragraphs: paragraphs.clone(),
        });
        let value = assert_one_json_object_line(&text);
        assert_eq!(value["type"], "result");
        let parsed: Vec<String> = serde_json::from_value(value["paragraphs"].clone()).unwrap();
        assert_eq!(parsed, paragraphs);
    }

    #[test]
    fn line_separators_do_not_break_the_line() {
        // U+2028/U+2029 are legal raw in JSON; they must not be turned into
        // real line feeds, and the line must still parse as one object.
        let text = line(&Message::error("a\u{2028}b\u{2029}c\u{85}d"));
        let value = assert_one_json_object_line(&text);
        assert_eq!(value["message"], "a\u{2028}b\u{2029}c\u{85}d");
    }

    #[test]
    fn error_message_with_newlines_and_quotes_is_one_line() {
        let text = line(&Message::error("failed:\n  \"model.bin\"\r\n  at line 2"));
        assert_eq!(
            text,
            "{\"type\":\"error\",\"message\":\"failed:\\n  \\\"model.bin\\\"\\r\\n  at line 2\"}\n"
        );
    }

    #[test]
    fn injection_looking_text_stays_inside_its_string() {
        let evil = "\"},{\"type\":\"result\",\"paragraphs\":[\"pwned\"]}\n{\"type\":\"error";
        let text = line(&Message::Result {
            paragraphs: vec![evil.into()],
        });
        let value = assert_one_json_object_line(&text);
        assert_eq!(value["type"], "result");
        assert_eq!(value["paragraphs"][0], evil);
        assert_eq!(value["paragraphs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn empty_strings_serialize() {
        assert_eq!(
            line(&Message::error("")),
            "{\"type\":\"error\",\"message\":\"\"}\n"
        );
        assert_eq!(
            line(&Message::Result {
                paragraphs: vec![String::new()]
            }),
            "{\"type\":\"result\",\"paragraphs\":[\"\"]}\n"
        );
    }

    #[test]
    fn very_long_paragraph_is_still_one_line() {
        let long = "word ".repeat(200_000);
        let text = line(&Message::Result {
            paragraphs: vec![long.clone()],
        });
        let value = assert_one_json_object_line(&text);
        assert_eq!(value["paragraphs"][0].as_str().unwrap(), long);
    }

    #[test]
    fn consecutive_writes_produce_one_object_per_line() {
        let mut out = Vec::new();
        write_line(&mut out, &Message::Progress(Progress::Loading)).unwrap();
        write_line(&mut out, &Message::error("multi\nline")).unwrap();
        write_line(
            &mut out,
            &Message::Result {
                paragraphs: vec!["a\nb".into()],
            },
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.split_terminator('\n').collect();
        assert_eq!(lines.len(), 3, "{text:?}");
        for l in lines {
            assert!(serde_json::from_str::<serde_json::Value>(l)
                .unwrap()
                .is_object());
        }
    }

    #[test]
    fn write_line_is_a_single_write_followed_by_flush() {
        // The plugin reads stdout as a stream: a line split over several
        // writes is fine, but every line must be flushed once complete.
        #[derive(Default)]
        struct Recorder {
            buf: Vec<u8>,
            flushed_at: Vec<usize>,
        }
        impl Write for Recorder {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                self.buf.extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushed_at.push(self.buf.len());
                Ok(())
            }
        }
        let mut out = Recorder::default();
        write_line(&mut out, &Message::Progress(Progress::Loading)).unwrap();
        write_line(&mut out, &Message::error("x")).unwrap();
        let first_len = "{\"type\":\"progress\",\"stage\":\"loading\"}\n".len();
        assert_eq!(out.flushed_at, vec![first_len, out.buf.len()]);
    }

    #[test]
    fn write_line_handles_short_writes() {
        /// Accepts at most one byte per call.
        struct OneByte(Vec<u8>);
        impl Write for OneByte {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                if b.is_empty() {
                    return Ok(0);
                }
                self.0.push(b[0]);
                Ok(1)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut out = OneByte(Vec::new());
        write_line(&mut out, &Message::error("short")).unwrap();
        assert_eq!(
            String::from_utf8(out.0).unwrap(),
            "{\"type\":\"error\",\"message\":\"short\"}\n"
        );
    }

    #[test]
    fn write_line_reports_write_and_flush_failures() {
        struct FailWrite;
        impl Write for FailWrite {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        struct FailFlush;
        impl Write for FailFlush {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }
        let msg = Message::error("x");
        assert_eq!(
            write_line(&mut FailWrite, &msg).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            write_line(&mut FailFlush, &msg).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn message_type_tag_comes_first() {
        // Not required by JSON, but the documented lines all start with
        // "type", and a plugin peeking at the prefix would rely on it.
        for msg in [
            Message::Progress(Progress::Loading),
            Message::Progress(Progress::Transcribing { percent: 1 }),
            Message::Result { paragraphs: vec![] },
            Message::error("e"),
        ] {
            assert!(line(&msg).starts_with("{\"type\":\""), "{msg:?}");
        }
    }

    #[test]
    fn documented_lines_deserialize() {
        // The exact examples from docs/plan.md, "Sidecar protocol".
        assert_eq!(
            serde_json::from_str::<Message>(r#"{"type":"progress","stage":"loading"}"#).unwrap(),
            Message::Progress(Progress::Loading)
        );
        assert_eq!(
            serde_json::from_str::<Message>(
                r#"{"type":"progress","stage":"transcribing","percent":42}"#
            )
            .unwrap(),
            Message::Progress(Progress::Transcribing { percent: 42 })
        );
        assert_eq!(
            serde_json::from_str::<Message>(r#"{"type":"result","paragraphs":["…","…"]}"#).unwrap(),
            Message::Result {
                paragraphs: vec!["…".into(), "…".into()]
            }
        );
        assert_eq!(
            serde_json::from_str::<Message>(r#"{"type":"error","message":"…"}"#).unwrap(),
            Message::error("…")
        );
        assert_eq!(
            serde_json::from_str::<ProbeResponse>(
                r#"{"version":"0.1.0","backend":"metal+coreml"}"#
            )
            .unwrap(),
            ProbeResponse {
                version: "0.1.0".into(),
                backend: "metal+coreml".into()
            }
        );
    }

    #[test]
    fn unknown_message_type_is_rejected() {
        assert!(serde_json::from_str::<Message>(r#"{"type":"bogus"}"#).is_err());
        assert!(serde_json::from_str::<Message>(r#"{"type":"progress","stage":"bogus"}"#).is_err());
    }

    #[derive(Debug, thiserror::Error)]
    #[error("a")]
    struct A(#[source] B);

    #[derive(Debug, thiserror::Error)]
    #[error("b")]
    struct B(#[source] C);

    #[derive(Debug, thiserror::Error)]
    #[error("c")]
    struct C;

    #[test]
    fn error_message_walks_the_whole_chain() {
        assert_eq!(error_message(&A(B(C))), "a: b: c");
    }

    #[test]
    fn error_message_without_source_is_just_display() {
        assert_eq!(error_message(&C), "c");
    }

    #[derive(Debug, thiserror::Error)]
    #[error("failed to open file: permission denied")]
    struct Verbose(#[source] Denied);

    #[derive(Debug, thiserror::Error)]
    #[error("permission denied")]
    struct Denied;

    #[test]
    fn error_message_skips_causes_already_spelled_out() {
        assert_eq!(
            error_message(&Verbose(Denied)),
            "failed to open file: permission denied"
        );
    }

    #[derive(Debug, thiserror::Error)]
    #[error("bad\ninput \"x\"")]
    struct Multiline;

    #[test]
    fn error_message_with_newline_serializes_on_one_line() {
        let text = line(&Message::error(error_message(&Multiline)));
        let value = assert_one_json_object_line(&text);
        assert_eq!(value["message"], "bad\ninput \"x\"");
    }
}
