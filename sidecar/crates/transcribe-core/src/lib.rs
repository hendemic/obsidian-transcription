//! Traits, domain types, paragraphing, protocol types and `run()`.
//!
//! This crate has no whisper.cpp dependency: engines implement [`Transcriber`],
//! paragraphing strategies implement [`Paragrapher`], and [`run`] drives one
//! transcription while writing [sidecar protocol](protocol) lines.

mod paragraph;
mod pipeline;
pub mod protocol;
mod types;

pub use paragraph::{Paragrapher, PauseGap};
pub use pipeline::{run, RunError};
pub use types::{Options, Pcm16kMono, Progress, Segment, Transcriber, SAMPLE_RATE};
