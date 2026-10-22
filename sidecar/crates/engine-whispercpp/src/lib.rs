//! whisper.cpp-backed `Transcriber` implementation.
//!
//! The GPU backend is chosen at compile time with the `metal`, `coreml`,
//! `cuda` and `vulkan` features (forwarded to whisper-rs); no feature means a
//! CPU build. [`backend`] reports what was compiled in.
//!
//! # Voice activity detection
//!
//! Silero VAD (whisper.cpp's built-in implementation, using a ggml Silero
//! model) is on by default: large-v3 hallucinates and repeats during silence.
//! whisper-rs only exposes `whisper_full_with_state`, which does not run VAD
//! (only `whisper_full` does), so the engine runs whisper.cpp's VAD through
//! `WhisperVadContext`, splices the speech spans together the way
//! `whisper_full` would, and maps segment timestamps back to the original
//! audio. See the `vad` module.
//!
//! # Word timing
//!
//! Token-level timestamps are enabled and returned as [`Word`]s, and the gaps
//! between VAD speech spans as [`Silence`]s. Paragraph breaks need both:
//! segment timestamps abut, so they carry no pause information.
//!
//! If VAD is enabled but [`VadConfig::model_path`] is `None`,
//! [`WhisperCpp::new`] fails with [`Error::VadModelMissing`] rather than
//! silently transcribing without VAD: output quality would quietly degrade
//! (repetition loops in pauses) and nobody would know why. Callers that want
//! to run without VAD say so with `VadConfig { enabled: false, .. }`.

mod error;
mod logging;
mod vad;
mod words;

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::thread;

use transcribe_core::{
    Options, Pcm16kMono, Progress, Segment, Silence, Transcriber, Transcript, Word, SAMPLE_RATE,
};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperError,
    WhisperVadContext, WhisperVadContextParams, WhisperVadParams,
};

pub use error::Error;
pub use log::LevelFilter;
pub use logging::log_to_stderr;

use vad::{silences, splice, SpeechSpan, TimeMap};
use words::merge_tokens;

/// The compiled-in backend: `"metal+coreml"`, `"cuda"`, `"vulkan"` or `"cpu"`.
///
/// Feature names are joined with `+` in the order metal, coreml, cuda, vulkan.
pub fn backend() -> String {
    let parts: Vec<&str> = [
        (cfg!(feature = "metal"), "metal"),
        (cfg!(feature = "coreml"), "coreml"),
        (cfg!(feature = "cuda"), "cuda"),
        (cfg!(feature = "vulkan"), "vulkan"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    if parts.is_empty() {
        "cpu".to_string()
    } else {
        parts.join("+")
    }
}

/// Voice activity detection settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VadConfig {
    /// Run VAD before decoding. Default: `true`.
    pub enabled: bool,
    /// Path to the ggml Silero VAD model (e.g. `ggml-silero-v5.1.2.bin`).
    /// Required when `enabled` is true.
    pub model_path: Option<PathBuf>,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model_path: None,
        }
    }
}

/// Engine settings fixed at load time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhisperCppConfig {
    pub vad: VadConfig,
    /// CPU threads for decoding. `None` = available parallelism, capped at 8.
    pub n_threads: Option<NonZeroUsize>,
}

/// Beam width. 5 is whisper.cpp's CLI default and OpenAI's reference setting
/// for best accuracy; wider beams cost a lot more time for little gain.
const BEAM_SIZE: i32 = 5;

/// Checks the ggml file header before whisper.cpp sees the file.
///
/// whisper.cpp reads the header without bounds checks, so a truncated file
/// can crash the process inside model loading on some CPUs, before
/// [`check_model_dimensions`] gets a chance to run. The header is the magic
/// `0x67676d6c` followed by eleven little-endian `i32`s: ten dimensions, which
/// must be positive, and the ftype, which must not be negative.
fn check_model_header(path: &Path) -> Result<(), String> {
    use std::io::Read;

    const MAGIC: u32 = 0x6767_6d6c;
    const FIELDS: [&str; 11] = [
        "n_vocab",
        "n_audio_ctx",
        "n_audio_state",
        "n_audio_head",
        "n_audio_layer",
        "n_text_ctx",
        "n_text_state",
        "n_text_head",
        "n_text_layer",
        "n_mels",
        "ftype",
    ];

    let mut header = [0u8; 4 + 4 * FIELDS.len()];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|e| format!("cannot read model header: {e}"))?;
    let word = |i: usize| <[u8; 4]>::try_from(&header[i * 4..i * 4 + 4]).unwrap();

    if u32::from_le_bytes(word(0)) != MAGIC {
        return Err("not a ggml whisper model (bad magic)".to_string());
    }
    for (i, name) in FIELDS.iter().enumerate() {
        let value = i32::from_le_bytes(word(i + 1));
        let ok = if *name == "ftype" {
            value >= 0
        } else {
            value > 0
        };
        if !ok {
            return Err(format!("{name} is {value}"));
        }
    }
    Ok(())
}

/// Rejects models whose hyperparameters are zero.
///
/// whisper.cpp accepts a file with a valid magic but no tensors ("assuming
/// empty model for testing"), e.g. a truncated download with a zeroed header,
/// and then throws a C++ exception during inference, which aborts the process
/// without an error line. Checking the loaded context's dimensions through
/// whisper-rs's accessors catches it at load time without hand-parsing the
/// file format. Every real Whisper model has all of these above zero.
fn check_model_dimensions(ctx: &WhisperContext) -> Result<(), String> {
    let dimensions = [
        ("n_vocab", ctx.model_n_vocab()),
        ("n_audio_ctx", ctx.model_n_audio_ctx()),
        ("n_audio_state", ctx.model_n_audio_state()),
        ("n_audio_head", ctx.model_n_audio_head()),
        ("n_audio_layer", ctx.model_n_audio_layer()),
        ("n_text_ctx", ctx.model_n_text_ctx()),
        ("n_text_state", ctx.model_n_text_state()),
        ("n_text_head", ctx.model_n_text_head()),
        ("n_text_layer", ctx.model_n_text_layer()),
        ("n_mels", ctx.model_n_mels()),
    ];
    match dimensions.iter().find(|(_, value)| *value <= 0) {
        Some((name, value)) => Err(format!("{name} is {value}")),
        None => Ok(()),
    }
}

/// A loaded whisper.cpp model.
pub struct WhisperCpp {
    ctx: WhisperContext,
    vad: Option<Mutex<WhisperVadContext>>,
    n_threads: i32,
}

impl std::fmt::Debug for WhisperCpp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhisperCpp")
            .field("vad", &self.vad.is_some())
            .field("n_threads", &self.n_threads)
            .finish_non_exhaustive()
    }
}

impl WhisperCpp {
    /// Loads the model at `model_path` (and the VAD model, if enabled).
    ///
    /// On macOS with the `coreml` feature, whisper.cpp looks for the Core ML
    /// encoder (`<model stem>-encoder.mlmodelc`) next to the model file.
    pub fn new(model_path: impl AsRef<Path>, config: WhisperCppConfig) -> Result<Self, Error> {
        logging::install_hooks();
        let model_path = model_path.as_ref();

        let vad_path = match (&config.vad.enabled, &config.vad.model_path) {
            (false, _) => None,
            (true, None) => return Err(Error::VadModelMissing),
            (true, Some(path)) => Some(path.as_path()),
        };

        if !model_path.is_file() {
            return Err(Error::ModelNotFound(model_path.to_path_buf()));
        }
        if let Some(path) = vad_path {
            if !path.is_file() {
                return Err(Error::VadModelNotFound(path.to_path_buf()));
            }
        }

        let n_threads = config
            .n_threads
            .map(NonZeroUsize::get)
            .unwrap_or_else(default_threads);
        let n_threads = i32::try_from(n_threads).unwrap_or(i32::MAX);

        check_model_header(model_path).map_err(|reason| Error::InvalidModel {
            path: model_path.to_path_buf(),
            reason,
        })?;

        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .map_err(|source| Error::ModelLoad {
                path: model_path.to_path_buf(),
                source,
            })?;

        check_model_dimensions(&ctx).map_err(|reason| Error::InvalidModel {
            path: model_path.to_path_buf(),
            reason,
        })?;

        let vad = match vad_path {
            None => None,
            Some(path) => {
                let path_str = path
                    .to_str()
                    .ok_or_else(|| Error::NonUtf8Path(path.to_path_buf()))?;
                let mut params = WhisperVadContextParams::new();
                params.set_n_threads(n_threads);
                let vad = WhisperVadContext::new(path_str, params).map_err(|source| {
                    Error::VadModelLoad {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
                Some(Mutex::new(vad))
            }
        };

        Ok(Self {
            ctx,
            vad,
            n_threads,
        })
    }

    /// Returns the audio to decode, the map from its timeline to `samples`,
    /// and the silences VAD found in `samples`.
    fn speech_only(&self, samples: &[f32]) -> Result<(Vec<f32>, TimeMap, Vec<Silence>), Error> {
        let Some(vad) = &self.vad else {
            return Ok((samples.to_vec(), TimeMap::identity(), Vec::new()));
        };
        let params = WhisperVadParams::default();
        let spans: Vec<SpeechSpan> = {
            let mut vad = vad.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            vad.segments_from_samples(params, samples)
                .map_err(Error::Vad)?
                .map(|s| SpeechSpan::from_centiseconds(s.start, s.end, samples.len()))
                .collect()
        };
        log::info!("VAD found {} speech spans", spans.len());
        // whisper.cpp's default `samples_overlap` is 0.1 s.
        let overlap = SAMPLE_RATE as usize / 10;
        let (audio, time_map) = splice(samples, &spans, overlap);
        Ok((audio, time_map, silences(&spans)))
    }
}

fn default_threads() -> usize {
    thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(4)
        .min(8)
}

/// Validates the language option. `None`, `""` and `"auto"` mean auto-detect;
/// English-only models always get `"en"`.
fn resolve_language(language: Option<&str>, multilingual: bool) -> Result<Option<String>, Error> {
    let language = language
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "auto");
    let Some(code) = language else {
        return Ok((!multilingual).then(|| "en".to_string()));
    };
    if code.contains('\0') {
        return Err(Error::NulCharacter("language"));
    }
    let code = code.to_lowercase();
    if whisper_rs::get_lang_id(&code).is_none() {
        return Err(Error::UnknownLanguage(code));
    }
    Ok(Some(code))
}

/// Validates the initial prompt. Blank prompts are treated as none.
fn resolve_prompt(prompt: Option<&str>) -> Result<Option<&str>, Error> {
    match prompt.filter(|p| !p.trim().is_empty()) {
        Some(p) if p.contains('\0') => Err(Error::NulCharacter("initial prompt")),
        other => Ok(other),
    }
}

enum Event {
    Progress(i32),
    Done(thread::Result<Result<(), WhisperError>>),
}

fn clamp_percent(percent: i32) -> u8 {
    percent.clamp(0, 100) as u8
}

impl Transcriber for WhisperCpp {
    type Error = Error;

    fn transcribe(
        &self,
        pcm: &Pcm16kMono,
        opts: &Options,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Transcript, Error> {
        // Covers state allocation (and, with Core ML, loading the encoder) and the VAD pass.
        progress(Progress::Loading);

        let language = resolve_language(opts.language.as_deref(), self.ctx.is_multilingual())?;
        let prompt = resolve_prompt(opts.initial_prompt.as_deref())?;

        if pcm.0.is_empty() {
            return Ok(Transcript::default());
        }
        let (audio, time_map, silences) = self.speech_only(&pcm.0)?;
        if audio.is_empty() {
            log::info!("no speech detected");
            return Ok(Transcript::default());
        }

        let mut state = self.ctx.create_state().map_err(Error::CreateState)?;

        // Decoding parameters: beam search with whisper.cpp's defaults for
        // everything else, i.e. temperature 0 with fallback in 0.2 steps when
        // a window fails the entropy (2.4) or log-probability (-1.0)
        // thresholds, and no-speech threshold 0.6. These match OpenAI's
        // reference decoding, which favours accuracy over speed.
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: BEAM_SIZE,
            patience: -1.0,
        });
        params.set_n_threads(self.n_threads);
        params.set_translate(false);
        params.set_language(language.as_deref());
        if let Some(prompt) = prompt {
            params.set_initial_prompt(prompt);
        }
        // Never let whisper.cpp print to stdout (print_realtime uses printf).
        params.set_print_realtime(false);
        params.set_print_progress(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        params.set_token_timestamps(true);

        // whisper-rs requires a 'static progress closure, so progress travels
        // over a channel from a scoped worker thread running inference.
        // (whisper-rs leaks the boxed closure, and with it this sender; the
        // loop below therefore ends on `Done`, not on channel close.)
        let (tx, rx) = mpsc::channel::<Event>();
        let progress_tx = tx.clone();
        params.set_progress_callback_safe(move |percent: i32| {
            let _ = progress_tx.send(Event::Progress(percent));
        });

        let outcome = thread::scope(|scope| {
            let state = &mut state;
            let audio = &audio;
            scope.spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    state.full(params, audio)
                }));
                let _ = tx.send(Event::Done(result));
            });
            for event in rx.iter() {
                match event {
                    Event::Progress(percent) => progress(Progress::Transcribing {
                        percent: clamp_percent(percent),
                    }),
                    Event::Done(result) => return result,
                }
            }
            // Unreachable in practice: the worker always sends `Done`.
            Err(Box::new("worker exited without a result") as Box<dyn std::any::Any + Send>)
        });

        match outcome {
            Ok(result) => result.map_err(Error::Inference)?,
            Err(_) => return Err(Error::WorkerPanicked),
        }
        progress(Progress::Transcribing { percent: 100 });

        // Special tokens (timestamps, end of text, ...) have ids from EOT up.
        let eot = self.ctx.token_eot();
        let mut segments = Vec::new();
        for segment in state.as_iter() {
            let text = segment
                .to_str_lossy()
                .map_err(Error::SegmentText)?
                .into_owned();
            let tokens = (0..segment.n_tokens())
                .filter_map(|i| segment.get_token(i))
                .filter(|token| token.token_id() < eot)
                .map(|token| {
                    let data = token.token_data();
                    token.to_bytes().map(|bytes| (bytes, data.t0, data.t1))
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(Error::SegmentText)?;
            let words = merge_tokens(tokens)
                .into_iter()
                .map(|piece| Word {
                    start: time_map.to_original(piece.t0),
                    end: time_map.to_original(piece.t1),
                    text: piece.text,
                })
                .collect();
            segments.push(Segment {
                start: time_map.to_original(segment.start_timestamp()),
                end: time_map.to_original(segment.end_timestamp()),
                text,
                words,
            });
        }
        Ok(Transcript { segments, silences })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engine-header-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn header(fields: [i32; 11]) -> Vec<u8> {
        let mut bytes = 0x6767_6d6c_u32.to_le_bytes().to_vec();
        for f in fields {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        bytes
    }

    // large-v3 hyperparameters, f16.
    const LARGE_V3: [i32; 11] = [51866, 1500, 1280, 20, 32, 448, 1280, 20, 32, 128, 1];

    #[test]
    fn header_of_a_real_model_shape_is_accepted() {
        let path = header_file("ok.bin", &header(LARGE_V3));
        assert_eq!(check_model_header(&path), Ok(()));
    }

    #[test]
    fn truncated_header_is_rejected() {
        let path = header_file("short.bin", &[0x6C, 0x6D, 0x67, 0x67, 0, 0, 0, 0]);
        let err = check_model_header(&path).unwrap_err();
        assert!(err.contains("cannot read model header"), "{err}");
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut bytes = header(LARGE_V3);
        bytes[0] = b'x';
        let path = header_file("magic.bin", &bytes);
        assert!(check_model_header(&path).unwrap_err().contains("bad magic"));
    }

    #[test]
    fn zero_dimension_is_rejected_by_name() {
        let mut fields = LARGE_V3;
        fields[4] = 0;
        let path = header_file("zero.bin", &header(fields));
        assert_eq!(
            check_model_header(&path),
            Err("n_audio_layer is 0".to_string())
        );
    }

    #[test]
    fn f32_ftype_zero_is_accepted_but_negative_is_not() {
        let mut fields = LARGE_V3;
        fields[10] = 0;
        assert_eq!(
            check_model_header(&header_file("f32.bin", &header(fields))),
            Ok(())
        );
        fields[10] = -1;
        assert!(check_model_header(&header_file("neg.bin", &header(fields))).is_err());
    }

    #[test]
    fn cpu_build_reports_cpu_backend() {
        if !cfg!(any(
            feature = "metal",
            feature = "coreml",
            feature = "cuda",
            feature = "vulkan"
        )) {
            assert_eq!(backend(), "cpu");
        }
    }

    #[test]
    fn vad_is_on_by_default() {
        assert!(WhisperCppConfig::default().vad.enabled);
    }

    #[test]
    fn vad_enabled_without_model_is_an_error() {
        let err =
            WhisperCpp::new("/nonexistent/model.bin", WhisperCppConfig::default()).unwrap_err();
        assert!(matches!(err, Error::VadModelMissing), "{err}");
    }

    #[test]
    fn missing_model_file_is_reported() {
        let config = WhisperCppConfig {
            vad: VadConfig {
                enabled: false,
                model_path: None,
            },
            n_threads: None,
        };
        let err = WhisperCpp::new("/nonexistent/model.bin", config).unwrap_err();
        assert!(matches!(err, Error::ModelNotFound(_)), "{err}");
    }

    #[test]
    fn language_resolution() {
        assert_eq!(resolve_language(None, true).unwrap(), None);
        assert_eq!(resolve_language(Some("auto"), true).unwrap(), None);
        assert_eq!(resolve_language(Some("  "), true).unwrap(), None);
        assert_eq!(resolve_language(None, false).unwrap(), Some("en".into()));
        assert_eq!(
            resolve_language(Some("DE"), true).unwrap(),
            Some("de".into())
        );
        assert!(matches!(
            resolve_language(Some("xx-nope"), true),
            Err(Error::UnknownLanguage(_))
        ));
        assert!(matches!(
            resolve_language(Some("e\0n"), true),
            Err(Error::NulCharacter(_))
        ));
    }

    #[test]
    fn prompt_resolution() {
        assert_eq!(resolve_prompt(None).unwrap(), None);
        assert_eq!(resolve_prompt(Some("  ")).unwrap(), None);
        assert_eq!(resolve_prompt(Some("Obsidian")).unwrap(), Some("Obsidian"));
        assert!(matches!(
            resolve_prompt(Some("a\0b")),
            Err(Error::NulCharacter(_))
        ));
    }

    #[test]
    fn percent_is_clamped() {
        assert_eq!(clamp_percent(-5), 0);
        assert_eq!(clamp_percent(42), 42);
        assert_eq!(clamp_percent(1000), 100);
    }
}
