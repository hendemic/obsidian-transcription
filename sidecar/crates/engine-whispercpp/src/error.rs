use std::path::PathBuf;

use whisper_rs::WhisperError;

/// Errors from [`crate::WhisperCpp`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("model file not found: {}", .0.display())]
    ModelNotFound(PathBuf),

    #[error("failed to load model {}", .path.display())]
    ModelLoad {
        path: PathBuf,
        #[source]
        source: WhisperError,
    },

    #[error("VAD is enabled but no VAD model path was configured")]
    VadModelMissing,

    #[error("VAD model file not found: {}", .0.display())]
    VadModelNotFound(PathBuf),

    #[error("failed to load VAD model {}", .path.display())]
    VadModelLoad {
        path: PathBuf,
        #[source]
        source: WhisperError,
    },

    #[error("path is not valid UTF-8: {}", .0.display())]
    NonUtf8Path(PathBuf),

    #[error("voice activity detection failed")]
    Vad(#[source] WhisperError),

    #[error("failed to create whisper state")]
    CreateState(#[source] WhisperError),

    #[error("whisper inference failed")]
    Inference(#[source] WhisperError),

    #[error("failed to read segment text")]
    SegmentText(#[source] WhisperError),

    #[error("unknown language code {0:?}")]
    UnknownLanguage(String),

    #[error("{0} must not contain NUL characters")]
    NulCharacter(&'static str),

    #[error("the transcription thread panicked")]
    WorkerPanicked,
}
