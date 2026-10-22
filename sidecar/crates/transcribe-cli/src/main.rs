//! The sidecar binary. See "Sidecar protocol" in `docs/plan.md`.
//!
//! stdout carries only protocol lines; diagnostics go to stderr. Set
//! `TRANSCRIBE_LOG` to `error`, `warn` (default), `info`, `debug` or `trace`
//! to control whisper.cpp's stderr logging.

mod pcm;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use engine_whispercpp::{LevelFilter, VadConfig, WhisperCpp, WhisperCppConfig};
use transcribe_core::protocol::{error_message, write_line, Message, ProbeResponse};
use transcribe_core::{run, Options, PauseGap, Pcm16kMono, Progress, Segment, Transcriber};

#[derive(Debug, Parser)]
#[command(
    name = "transcribe-cli",
    version,
    about = "Local Whisper transcription sidecar"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the version and compiled backend as JSON.
    Probe,
    /// Transcribe raw 16 kHz mono little-endian f32 PCM read from stdin.
    Transcribe(TranscribeArgs),
}

#[derive(Debug, clap::Args)]
struct TranscribeArgs {
    /// Path to the ggml Whisper model.
    #[arg(long)]
    model: PathBuf,
    /// Path to the ggml Silero VAD model. Without it VAD is disabled.
    #[arg(long)]
    vad_model: Option<PathBuf>,
    /// Language code, e.g. "en". Omit (or "auto") to auto-detect.
    #[arg(long)]
    language: Option<String>,
    /// Initial prompt: names and terms to bias the decoder toward.
    #[arg(long)]
    prompt: Option<String>,
    /// Minimum pause, in milliseconds, that starts a new paragraph.
    #[arg(long, default_value_t = 1800)]
    pause_gap_ms: u64,
}

fn main() -> ExitCode {
    engine_whispercpp::log_to_stderr(log_level());

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if !e.use_stderr() => {
            // --help / --version
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            let _ = e.print();
            let message = format!("invalid arguments: {}", summarize_clap_error(&e));
            return fail(&message);
        }
    };

    match cli.command {
        Command::Probe => probe(),
        Command::Transcribe(args) => transcribe(args),
    }
}

/// clap's error text without the "error: " prefix, usage and help hints,
/// on one line.
fn summarize_clap_error(e: &clap::Error) -> String {
    let rendered = e.render().to_string();
    rendered
        .lines()
        .take_while(|line| !line.starts_with("Usage:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim_start_matches("error: ")
        .to_string()
}

fn log_level() -> LevelFilter {
    std::env::var("TRANSCRIBE_LOG")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(LevelFilter::Warn)
}

/// Reports `message` on stderr and as an error line, and returns exit code 1.
fn fail(message: &str) -> ExitCode {
    eprintln!("error: {message}");
    let _ = write_line(&mut io::stdout().lock(), &Message::error(message));
    ExitCode::FAILURE
}

fn probe() -> ExitCode {
    let response = ProbeResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        backend: engine_whispercpp::backend(),
    };
    match write_line(&mut io::stdout().lock(), &response) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: failed to write probe output: {e}");
            ExitCode::FAILURE
        }
    }
}

fn transcribe(args: TranscribeArgs) -> ExitCode {
    let pcm = match pcm::read_pcm_f32le(io::stdin().lock()) {
        Ok(pcm) => pcm,
        Err(e) => return fail(&error_message(&e)),
    };

    let engine = LoadThenTranscribe {
        model: args.model,
        config: WhisperCppConfig {
            vad: vad_config(args.vad_model),
            n_threads: None,
        },
    };
    let paragrapher = PauseGap::new(Duration::from_millis(args.pause_gap_ms));
    let opts = Options {
        language: args.language,
        initial_prompt: args.prompt,
    };

    let stdout = io::stdout().lock();
    match run(&engine, &paragrapher, &pcm, &opts, stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // run() has already written the error line when it could.
            eprintln!("error: {}", error_message(&e));
            let _ = io::stderr().flush();
            ExitCode::FAILURE
        }
    }
}

/// `--vad-model` is optional in the protocol, so its absence explicitly
/// disables VAD (with a warning) instead of tripping the engine's
/// "VAD enabled without a model" error.
fn vad_config(vad_model: Option<PathBuf>) -> VadConfig {
    if vad_model.is_none() {
        eprintln!(
            "warning: no --vad-model given; transcribing without voice activity detection \
             (expect more hallucinated text during silences)"
        );
    }
    VadConfig {
        enabled: vad_model.is_some(),
        model_path: vad_model,
    }
}

/// Loads the model inside `transcribe` so the loading progress line comes
/// first and model-load errors are reported through `run()` as error lines.
struct LoadThenTranscribe {
    model: PathBuf,
    config: WhisperCppConfig,
}

impl Transcriber for LoadThenTranscribe {
    type Error = engine_whispercpp::Error;

    fn transcribe(
        &self,
        pcm: &Pcm16kMono,
        opts: &Options,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Vec<Segment>, Self::Error> {
        progress(Progress::Loading);
        let engine = WhisperCpp::new(&self.model, self.config.clone())?;
        engine.transcribe(pcm, opts, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_transcribe_with_all_flags() {
        let cli = Cli::try_parse_from([
            "transcribe-cli",
            "transcribe",
            "--model",
            "m.bin",
            "--vad-model",
            "v.bin",
            "--language",
            "en",
            "--prompt",
            "Obsidian, Whisper",
            "--pause-gap-ms",
            "2500",
        ])
        .unwrap();
        let Command::Transcribe(args) = cli.command else {
            panic!("expected transcribe");
        };
        assert_eq!(args.model, PathBuf::from("m.bin"));
        assert_eq!(args.vad_model, Some(PathBuf::from("v.bin")));
        assert_eq!(args.language.as_deref(), Some("en"));
        assert_eq!(args.prompt.as_deref(), Some("Obsidian, Whisper"));
        assert_eq!(args.pause_gap_ms, 2500);
    }

    #[test]
    fn transcribe_defaults() {
        let cli =
            Cli::try_parse_from(["transcribe-cli", "transcribe", "--model", "m.bin"]).unwrap();
        let Command::Transcribe(args) = cli.command else {
            panic!("expected transcribe");
        };
        assert_eq!(args.vad_model, None);
        assert_eq!(args.language, None);
        assert_eq!(args.prompt, None);
        assert_eq!(args.pause_gap_ms, 1800);
    }

    #[test]
    fn model_is_required() {
        assert!(Cli::try_parse_from(["transcribe-cli", "transcribe"]).is_err());
    }

    #[test]
    fn clap_errors_are_summarized_on_one_line() {
        let e = Cli::try_parse_from(["transcribe-cli", "transcribe"]).unwrap_err();
        let summary = summarize_clap_error(&e);
        assert!(!summary.contains('\n'), "{summary}");
        assert!(summary.contains("--model"), "{summary}");
        assert!(!summary.starts_with("error:"), "{summary}");
    }

    #[test]
    fn parses_probe() {
        let cli = Cli::try_parse_from(["transcribe-cli", "probe"]).unwrap();
        assert!(matches!(cli.command, Command::Probe));
    }

    #[test]
    fn vad_follows_vad_model_flag() {
        assert!(!vad_config(None).enabled);
        let vad = vad_config(Some("v.bin".into()));
        assert!(vad.enabled);
        assert_eq!(vad.model_path, Some(PathBuf::from("v.bin")));
    }

    #[test]
    fn missing_model_is_reported_as_error_line() {
        let engine = LoadThenTranscribe {
            model: "/nonexistent/model.bin".into(),
            config: WhisperCppConfig {
                vad: vad_config(None),
                n_threads: None,
            },
        };
        let mut out = Vec::new();
        let result = run(
            &engine,
            &PauseGap::default(),
            &Pcm16kMono(vec![0.0; 160]),
            &Options::default(),
            &mut out,
        );
        assert!(result.is_err());
        let out = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "{\"type\":\"progress\",\"stage\":\"loading\"}");
        assert_eq!(
            lines[1],
            "{\"type\":\"error\",\"message\":\"model file not found: /nonexistent/model.bin\"}"
        );
        assert_eq!(lines.len(), 2);
    }
}
