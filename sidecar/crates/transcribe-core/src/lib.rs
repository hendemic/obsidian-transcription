//! Traits, domain types, paragraphing, protocol types and `run()`.
//!
//! This crate has no whisper.cpp dependency: engines implement [`Transcriber`],
//! [`sentences`] splits their output at sentence ends and measures pauses,
//! paragraphing strategies implement [`Paragrapher`], and [`run`] drives one
//! transcription while writing [sidecar protocol](protocol) lines.

mod paragraph;
mod pipeline;
pub mod protocol;
mod sentence;
mod types;

pub use paragraph::{Paragrapher, PauseGap};
pub use pipeline::{run, RunError};
pub use sentence::{sentences, Sentence, UNPUNCTUATED_BREAK};
pub use types::{
    Options, Pcm16kMono, Progress, Segment, Silence, Transcriber, Transcript, Word, SAMPLE_RATE,
};
