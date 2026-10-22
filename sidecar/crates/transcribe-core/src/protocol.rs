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
}
