//! Domain types and the `Transcriber` trait.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Sample rate of [`Pcm16kMono`], in Hz.
pub const SAMPLE_RATE: u32 = 16_000;

/// Mono PCM audio at 16 kHz, as `f32` samples nominally in `[-1.0, 1.0]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pcm16kMono(pub Vec<f32>);

impl Pcm16kMono {
    /// Length of the audio.
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.0.len() as f64 / f64::from(SAMPLE_RATE))
    }
}

/// A piece of transcribed text with its position in the original audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub start: Duration,
    pub end: Duration,
    pub text: String,
}

/// Per-transcription options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Language code such as `"en"`. `None` = auto-detect.
    pub language: Option<String>,
    /// Names and terms to bias the decoder toward.
    pub initial_prompt: Option<String>,
}

/// Progress reported by a [`Transcriber`] while it works.
///
/// Serializes as the `stage`/`percent` fields of a protocol progress line,
/// e.g. `{"stage":"transcribing","percent":42}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "lowercase")]
pub enum Progress {
    /// Loading a model or preparing to run inference.
    Loading,
    /// Inference is running; `percent` is in `0..=100`.
    Transcribing { percent: u8 },
}

/// A speech-to-text engine.
pub trait Transcriber {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Transcribes `pcm`, calling `progress` as work advances.
    ///
    /// Returned segments are in chronological order, with timestamps relative
    /// to the start of `pcm`.
    fn transcribe(
        &self,
        pcm: &Pcm16kMono,
        opts: &Options,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Segment>, Self::Error>;
}
