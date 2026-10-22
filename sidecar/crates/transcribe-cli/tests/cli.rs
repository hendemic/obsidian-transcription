//! End-to-end tests of the `transcribe-cli` binary against the sidecar
//! protocol in `docs/plan.md` ("Sidecar protocol"). No real Whisper model is
//! needed: every transcribe case here fails before inference. Model files
//! used are nonexistent paths, a directory, an empty file and a garbage file
//! (whisper.cpp rejects them by magic number, no weights are loaded).
//!
//! Deliberately not covered:
//! - `--help` / `--version`: clap prints human text to stdout and exits 0.
//!   They are user-invoked, not part of the plugin protocol; noted in the
//!   test report as a deviation from "stdout carries only protocol lines".
//! - Successful transcription: needs a real model, see `real_model.rs`.
//! - Process cancellation (plugin kills the sidecar): nothing to assert on
//!   the sidecar side.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_transcribe-cli");

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Output {
    /// Parses stdout as JSON lines, asserting that every line is a JSON
    /// object and that the output ends with a newline.
    fn json_lines(&self) -> Vec<Value> {
        assert!(
            self.stdout.is_empty() || self.stdout.ends_with('\n'),
            "stdout does not end with a newline: {:?}",
            self.stdout
        );
        self.stdout
            .split_terminator('\n')
            .map(|line| {
                let value: Value = serde_json::from_str(line).unwrap_or_else(|e| {
                    panic!(
                        "stdout line is not JSON ({e}): {line:?}\nstderr: {}",
                        self.stderr
                    )
                });
                assert!(value.is_object(), "not a JSON object: {line:?}");
                value
            })
            .collect()
    }

    /// Asserts the protocol's failure contract: exit code 1, stdout only
    /// JSON lines, the last line an error with a non-empty message, and no
    /// result line. Returns the error message.
    fn assert_protocol_error(&self) -> String {
        let lines = self.json_lines();
        assert_eq!(
            self.code,
            Some(1),
            "exit code; stdout: {:?}; stderr: {:?}",
            self.stdout,
            self.stderr
        );
        let last = lines.last().unwrap_or_else(|| {
            panic!("no stdout lines; stderr: {:?}", self.stderr);
        });
        assert_eq!(last["type"], "error", "{last}");
        let errors = lines.iter().filter(|l| l["type"] == "error").count();
        assert_eq!(
            errors, 1,
            "exactly one error line expected: {:?}",
            self.stdout
        );
        assert!(
            lines.iter().all(|l| l["type"] != "result"),
            "{:?}",
            self.stdout
        );
        for line in &lines[..lines.len() - 1] {
            assert_eq!(line["type"], "progress", "{line}");
        }
        let message = last["message"].as_str().expect("message is a string");
        assert!(!message.trim().is_empty(), "empty error message");
        assert_eq!(
            last.as_object().unwrap().len(),
            2,
            "error line has only type and message: {last}"
        );
        message.to_string()
    }
}

fn run_cli<I, S>(args: I, stdin: &[u8]) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_cli_env(args, stdin, &[])
}

fn run_cli_env<I, S>(args: I, stdin: &[u8], env: &[(&str, &str)]) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env_remove("TRANSCRIBE_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn transcribe-cli");
    let mut child_stdin = child.stdin.take().unwrap();
    let data = stdin.to_vec();
    // Write from a thread so a large stdin cannot deadlock against a full
    // stdout pipe; ignore EPIPE when the CLI exits without reading.
    let writer = thread::spawn(move || {
        let _ = child_stdin.write_all(&data);
    });
    let out = child.wait_with_output().expect("wait for transcribe-cli");
    writer.join().unwrap();
    Output {
        code: out.status.code(),
        stdout: String::from_utf8(out.stdout).expect("stdout is UTF-8"),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn pcm(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// A fresh path under cargo's per-target temp dir.
fn temp_path(name: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("cli-tests-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn file_with(name: &str, contents: &[u8]) -> PathBuf {
    let path = temp_path(name);
    std::fs::write(&path, contents).unwrap();
    path
}

const NO_MODEL: &str = "/nonexistent/dir/ggml-model.bin";

// --- probe -------------------------------------------------------------------

#[test]
fn probe_prints_version_and_backend_and_exits_0() {
    let out = run_cli(["probe"], b"");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let lines = out.json_lines();
    assert_eq!(lines.len(), 1, "{:?}", out.stdout);
    let probe = lines[0].as_object().unwrap();
    assert_eq!(probe.len(), 2, "only version and backend: {:?}", out.stdout);
    assert_eq!(probe["version"], env!("CARGO_PKG_VERSION"));
    let backend = probe["backend"].as_str().unwrap();
    assert!(
        ["metal+coreml", "cuda", "vulkan", "cpu"].contains(&backend)
            || backend
                .split('+')
                .all(|p| ["metal", "coreml", "cuda", "vulkan"].contains(&p)),
        "{backend}"
    );
}

#[test]
#[cfg(not(any(
    feature = "metal",
    feature = "coreml",
    feature = "cuda",
    feature = "vulkan"
)))]
fn probe_on_cpu_build_is_exact() {
    let out = run_cli(["probe"], b"");
    assert_eq!(
        out.stdout,
        format!(
            "{{\"version\":\"{}\",\"backend\":\"cpu\"}}\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn probe_version_matches_the_documented_protocol_version() {
    let out = run_cli(["probe"], b"");
    assert_eq!(out.json_lines()[0]["version"], "0.1.0");
}

#[test]
fn probe_writes_nothing_to_stderr() {
    let out = run_cli(["probe"], b"");
    assert_eq!(out.stderr, "");
}

#[test]
fn probe_ignores_stdin() {
    let out = run_cli(["probe"], &[1, 2, 3]);
    assert_eq!(out.code, Some(0));
    assert_eq!(out.json_lines().len(), 1);
}

#[test]
fn probe_with_extra_argument_is_an_arg_error() {
    let out = run_cli(["probe", "--x"], b"");
    out.assert_protocol_error();
    assert_eq!(out.json_lines().len(), 1);
}

// --- argument errors ---------------------------------------------------------

fn arg_error_cases() -> Vec<Vec<&'static str>> {
    vec![
        vec![],
        vec!["bogus"],
        vec!["probe", "extra"],
        vec!["transcribe"],
        vec!["transcribe", "--model"],
        vec!["transcribe", "--model", "m.bin", "--unknown"],
        vec!["transcribe", "--model", "m.bin", "--model", "n.bin"],
        vec!["transcribe", "--model", "m.bin", "--pause-gap-ms"],
        vec!["transcribe", "--model", "m.bin", "--pause-gap-ms", "-1"],
        vec!["transcribe", "--model", "m.bin", "--pause-gap-ms", "1.5"],
        vec!["transcribe", "--model", "m.bin", "--pause-gap-ms", "abc"],
        vec!["transcribe", "--model", "m.bin", "--pause-gap-ms", ""],
        vec![
            "transcribe",
            "--model",
            "m.bin",
            "--pause-gap-ms",
            "18446744073709551616",
        ],
        vec!["transcribe", "--vad-model", "v.bin"],
        vec!["transcribe", "positional", "--model", "m.bin"],
    ]
}

#[test]
fn arg_errors_write_one_json_error_line_and_exit_1() {
    for args in arg_error_cases() {
        let out = run_cli(&args, &pcm(&[0.0; 16]));
        let message = out.assert_protocol_error();
        assert_eq!(out.json_lines().len(), 1, "args {args:?}: {:?}", out.stdout);
        assert!(
            message.starts_with("invalid arguments"),
            "args {args:?}: {message}"
        );
    }
}

#[test]
fn arg_error_messages_contain_no_usage_block() {
    for args in arg_error_cases() {
        let out = run_cli(&args, b"");
        let message = out.assert_protocol_error();
        assert!(!message.contains("Usage:"), "args {args:?}: {message}");
        assert!(!message.contains('\n'), "args {args:?}: {message:?}");
    }
}

#[test]
fn arg_errors_are_explained_on_stderr() {
    let out = run_cli(["transcribe"], b"");
    assert!(out.stderr.contains("--model"), "{}", out.stderr);
}

#[test]
fn missing_model_flag_names_the_flag() {
    let out = run_cli(["transcribe"], b"");
    let message = out.assert_protocol_error();
    assert!(message.contains("--model"), "{message}");
}

#[test]
fn unknown_flag_is_named_in_the_message() {
    let out = run_cli(["transcribe", "--model", "m.bin", "--frobnicate"], b"");
    let message = out.assert_protocol_error();
    assert!(message.contains("--frobnicate"), "{message}");
}

#[test]
fn no_subcommand_error_says_a_command_is_missing() {
    let out = run_cli(Vec::<&str>::new(), b"");
    let message = out.assert_protocol_error();
    assert!(
        message.to_lowercase().contains("command") || message.contains("required"),
        "{message}"
    );
}

#[test]
fn arg_error_messages_contain_no_help_hint() {
    for args in arg_error_cases() {
        let out = run_cli(&args, b"");
        let message = out.assert_protocol_error();
        assert!(!message.contains("--help"), "args {args:?}: {message}");
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_language_argument_is_an_arg_error() {
    use std::os::unix::ffi::OsStrExt;
    let args: Vec<&OsStr> = vec![
        OsStr::new("transcribe"),
        OsStr::new("--model"),
        OsStr::new(NO_MODEL),
        OsStr::new("--language"),
        OsStr::from_bytes(b"e\xffn"),
    ];
    let out = run_cli(&args, b"");
    out.assert_protocol_error();
}

#[cfg(unix)]
#[test]
fn non_utf8_model_path_is_reported_as_json() {
    use std::os::unix::ffi::OsStrExt;
    let args: Vec<&OsStr> = vec![
        OsStr::new("transcribe"),
        OsStr::new("--model"),
        OsStr::from_bytes(b"/nonexistent/mod\xffel.bin"),
    ];
    let out = run_cli(&args, &pcm(&[0.0; 4]));
    out.assert_protocol_error();
}

// --- model errors ------------------------------------------------------------

#[test]
fn missing_model_file_writes_error_line_and_exits_1() {
    let out = run_cli(["transcribe", "--model", NO_MODEL], &pcm(&[0.0; 1600]));
    let message = out.assert_protocol_error();
    assert!(message.contains(NO_MODEL), "{message}");
}

#[test]
fn missing_model_reports_loading_before_the_error() {
    let out = run_cli(["transcribe", "--model", NO_MODEL], &pcm(&[0.0; 160]));
    let lines = out.json_lines();
    assert_eq!(lines.len(), 2, "{:?}", out.stdout);
    assert_eq!(lines[0]["type"], "progress");
    assert_eq!(lines[0]["stage"], "loading");
    assert_eq!(lines[1]["type"], "error");
}

#[test]
fn missing_model_with_empty_stdin_is_still_an_error() {
    let out = run_cli(["transcribe", "--model", NO_MODEL], b"");
    out.assert_protocol_error();
}

#[test]
fn missing_model_with_all_flags_is_still_a_clean_error() {
    let out = run_cli(
        [
            "transcribe",
            "--model",
            NO_MODEL,
            "--vad-model",
            "/nonexistent/vad.bin",
            "--language",
            "en",
            "--prompt",
            "Obsidian, \"Whisper\"\nnewline",
            "--pause-gap-ms",
            "18446744073709551615",
        ],
        &pcm(&[0.0; 160]),
    );
    out.assert_protocol_error();
}

#[test]
fn model_path_with_quotes_and_unicode_round_trips_through_json() {
    let path = "/nonexistent/日本 \"model\" \\ 🎙.bin";
    let out = run_cli(["transcribe", "--model", path], &pcm(&[0.0; 16]));
    let message = out.assert_protocol_error();
    assert!(message.contains(path), "{message}");
}

#[test]
fn model_path_with_newline_stays_on_one_line() {
    let path = "/nonexistent/a\nb.bin";
    let out = run_cli(["transcribe", "--model", path], &pcm(&[0.0; 16]));
    let message = out.assert_protocol_error();
    assert!(message.contains(path), "{message:?}");
}

#[test]
fn model_path_that_is_a_directory_is_an_error() {
    let dir = temp_path("model-dir");
    std::fs::create_dir_all(&dir).unwrap();
    let out = run_cli(
        [
            OsStr::new("transcribe"),
            OsStr::new("--model"),
            dir.as_os_str(),
        ],
        &pcm(&[0.0; 16]),
    );
    out.assert_protocol_error();
}

#[test]
fn empty_model_file_is_an_error_not_a_crash() {
    let model = file_with("empty.bin", b"");
    let out = run_cli(
        [
            OsStr::new("transcribe"),
            OsStr::new("--model"),
            model.as_os_str(),
        ],
        &pcm(&[0.0; 16]),
    );
    out.assert_protocol_error();
}

#[test]
fn garbage_model_file_is_an_error_not_a_crash() {
    let model = file_with("garbage.bin", &[0xAB; 4096]);
    let out = run_cli(
        [
            OsStr::new("transcribe"),
            OsStr::new("--model"),
            model.as_os_str(),
        ],
        &pcm(&[0.0; 1600]),
    );
    let message = out.assert_protocol_error();
    assert!(
        message.contains(&*model.to_string_lossy()),
        "error should name the model: {message}"
    );
}

/// A file with the ggml magic ("lmgg") followed by zeroed hyperparameters,
/// like the first bytes of a truncated download.
fn truncated_ggml_model(name: &str) -> PathBuf {
    file_with(name, &[0x6C, 0x6D, 0x67, 0x67, 0, 0, 0, 0])
}

#[test]
fn truncated_model_with_short_audio_is_an_error() {
    let model = truncated_ggml_model("truncated-short.bin");
    let out = run_cli(
        [
            OsStr::new("transcribe"),
            OsStr::new("--model"),
            model.as_os_str(),
        ],
        &pcm(&[0.0; 16]),
    );
    out.assert_protocol_error();
}

#[test]
fn truncated_model_with_one_second_of_audio_is_an_error_not_an_abort() {
    let model = truncated_ggml_model("truncated-1s.bin");
    let out = run_cli(
        [
            OsStr::new("transcribe"),
            OsStr::new("--model"),
            model.as_os_str(),
        ],
        &pcm(&[0.0; 16_000]),
    );
    out.assert_protocol_error();
}

#[test]
fn whisper_logs_never_reach_stdout_even_at_trace_level() {
    let model = file_with("garbage-trace.bin", &[0x00; 1024]);
    let out = run_cli_env(
        [
            OsStr::new("transcribe"),
            OsStr::new("--model"),
            model.as_os_str(),
        ],
        &pcm(&[0.0; 16]),
        &[("TRANSCRIBE_LOG", "trace")],
    );
    out.assert_protocol_error();
}

#[test]
fn invalid_log_level_env_is_ignored() {
    let out = run_cli_env(
        ["transcribe", "--model", NO_MODEL],
        &pcm(&[0.0; 16]),
        &[("TRANSCRIBE_LOG", "\u{1F4A5}not-a-level")],
    );
    out.assert_protocol_error();
}

#[test]
fn missing_vad_model_file_is_an_error() {
    let model = file_with("garbage-for-vad.bin", &[0xAB; 64]);
    let out = run_cli(
        [
            OsStr::new("transcribe"),
            OsStr::new("--model"),
            model.as_os_str(),
            OsStr::new("--vad-model"),
            OsStr::new("/nonexistent/silero.bin"),
        ],
        &pcm(&[0.0; 16]),
    );
    out.assert_protocol_error();
}

#[test]
fn without_vad_model_a_warning_goes_to_stderr_only() {
    let out = run_cli(["transcribe", "--model", NO_MODEL], &pcm(&[0.0; 16]));
    assert!(
        out.stderr.to_lowercase().contains("vad"),
        "stderr: {}",
        out.stderr
    );
    assert!(
        !out.stdout.to_lowercase().contains("warning"),
        "{}",
        out.stdout
    );
}

// --- stdin errors ------------------------------------------------------------

#[test]
fn odd_length_stdin_writes_error_line_and_exits_1() {
    for len in [1usize, 2, 3, 5, 7, 4 * 1000 + 1] {
        let out = run_cli(["transcribe", "--model", NO_MODEL], &vec![0u8; len]);
        let message = out.assert_protocol_error();
        assert!(
            !message.contains("model file not found"),
            "len {len}: PCM error should be reported, got {message}"
        );
    }
}

#[test]
fn large_odd_length_stdin_does_not_deadlock() {
    // ~4 MB, one byte too many.
    let mut data = vec![0u8; 4 * 1024 * 1024];
    data.push(0);
    let out = run_cli(["transcribe", "--model", NO_MODEL], &data);
    out.assert_protocol_error();
}

#[test]
fn nan_in_stdin_writes_error_line_and_exits_1() {
    let out = run_cli(
        ["transcribe", "--model", NO_MODEL],
        &pcm(&[0.0, 0.1, f32::NAN, 0.2]),
    );
    let message = out.assert_protocol_error();
    assert!(!message.contains("model file not found"), "{message}");
}

#[test]
fn infinity_in_stdin_writes_error_line_and_exits_1() {
    let out = run_cli(
        ["transcribe", "--model", NO_MODEL],
        &pcm(&[f32::NEG_INFINITY]),
    );
    out.assert_protocol_error();
}

#[test]
fn closed_stdin_is_treated_as_empty_audio() {
    let out = Command::new(BIN)
        .args(["transcribe", "--model", NO_MODEL])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let out = Output {
        code: out.status.code(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    };
    let message = out.assert_protocol_error();
    assert!(message.contains("model file not found"), "{message}");
}

// --- exit codes --------------------------------------------------------------

#[test]
fn failures_never_exit_with_clap_or_panic_codes() {
    let cases: Vec<(Vec<&str>, Vec<u8>)> = vec![
        (vec![], vec![]),
        (vec!["transcribe"], vec![]),
        (vec!["transcribe", "--model", NO_MODEL], vec![0; 3]),
        (vec!["transcribe", "--model", NO_MODEL], pcm(&[f32::NAN])),
        (vec!["transcribe", "--model", NO_MODEL], pcm(&[0.0])),
    ];
    for (args, stdin) in cases {
        let out = run_cli(&args, &stdin);
        assert_eq!(out.code, Some(1), "args {args:?}: {}", out.stderr);
        assert!(!out.stderr.contains("panicked"), "{}", out.stderr);
    }
}
