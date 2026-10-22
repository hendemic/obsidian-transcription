//! Slow end-to-end tests with a real Whisper model. Ignored by default.
//!
//! Run them on a machine where loading a model is acceptable (not the
//! development VM; see "Harness: user-only checks" in `CLAUDE.md`):
//!
//! ```sh
//! # from sidecar/; CPU build, no GPU features
//! curl -L -o /tmp/ggml-tiny.en.bin \
//!   https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin
//! WHISPER_TEST_MODEL=/tmp/ggml-tiny.en.bin cargo test -p transcribe-cli --test real_model -- --ignored
//! ```
//!
//! Optionally set `WHISPER_TEST_VAD_MODEL` to a ggml Silero VAD model
//! (e.g. `ggml-silero-v5.1.2.bin` from
//! https://huggingface.co/ggml-org/whisper-vad) to also run the VAD test.
//! Without the env vars each test prints why it is skipping and passes.
//!
//! The fixture `tests/fixtures/jfk-ask-not.wav` is described in
//! `tests/fixtures/README.md`.
//!
//! Deliberately not covered: exact transcript text (tiny.en punctuation and
//! casing vary between whisper.cpp versions) and paragraph splitting on
//! natural speech. Paragraphs are only checked across a long, unambiguous
//! pause between two copies of the fixture.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_transcribe-cli");

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jfk-ask-not.wav")
}

fn env_path(var: &str) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ => {
            eprintln!(
                "SKIPPED: set {var} to a model path to run this test \
                 (see the header of tests/real_model.rs)"
            );
            None
        }
    }
}

/// Reads a 16-bit PCM, mono, 16 kHz WAV file into f32 samples in [-1, 1).
fn read_wav_16k_mono(path: &Path) -> Vec<f32> {
    let data = std::fs::read(path).unwrap();
    assert_eq!(&data[0..4], b"RIFF");
    assert_eq!(&data[8..12], b"WAVE");
    let mut pos = 12;
    let mut format_ok = false;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &data[pos + 8..pos + 8 + size];
        match id {
            b"fmt " => {
                let format = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes([body[14], body[15]]);
                assert_eq!((format, channels, rate, bits), (1, 1, 16_000, 16));
                format_ok = true;
            }
            b"data" => {
                assert!(format_ok, "fmt chunk must precede data");
                return body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
                    .collect();
            }
            _ => {}
        }
        pos += 8 + size + (size & 1);
    }
    panic!("no data chunk in {}", path.display());
}

struct Run {
    code: Option<i32>,
    lines: Vec<Value>,
    stderr: String,
}

fn transcribe(args: &[&std::ffi::OsStr], samples: &[f32]) -> Run {
    let mut child = Command::new(BIN)
        .arg("transcribe")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let writer = thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    });
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stdout.is_empty() || stdout.ends_with('\n'), "{stdout:?}");
    let lines = stdout
        .split_terminator('\n')
        .map(|l| {
            serde_json::from_str(l)
                .unwrap_or_else(|e| panic!("non-JSON stdout line ({e}): {l:?}\nstderr: {stderr}"))
        })
        .collect();
    Run {
        code: out.status.code(),
        lines,
        stderr,
    }
}

/// Checks the success contract and returns the paragraphs.
fn assert_success(run: &Run) -> Vec<String> {
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let (last, progress) = run.lines.split_last().expect("some output");
    assert_eq!(last["type"], "result", "{last}");
    assert_eq!(
        progress.first().map(|p| &p["stage"]),
        Some(&Value::from("loading"))
    );
    let mut previous = 0;
    for p in progress {
        assert_eq!(p["type"], "progress", "{p}");
        if p["stage"] == "transcribing" {
            let percent = p["percent"].as_u64().expect("percent");
            assert!(percent <= 100, "{p}");
            assert!(percent >= previous, "progress went backwards: {p}");
            previous = percent;
        } else {
            assert_eq!(p["stage"], "loading", "{p}");
        }
    }
    let paragraphs: Vec<String> = serde_json::from_value(last["paragraphs"].clone()).unwrap();
    for p in &paragraphs {
        assert!(!p.trim().is_empty(), "empty paragraph in {paragraphs:?}");
        assert_eq!(p, p.trim(), "untrimmed paragraph {p:?}");
    }
    paragraphs
}

fn assert_expected_words(paragraphs: &[String]) {
    let text = paragraphs.join(" ").to_lowercase();
    for word in ["fellow", "americans"] {
        assert!(text.contains(word), "expected {word:?} in {text:?}");
    }
}

#[test]
fn fixture_is_a_short_16k_mono_wav() {
    // Runs without a model: guards the fixture the slow tests depend on.
    let samples = read_wav_16k_mono(&fixture());
    assert!(samples.len() >= 16_000, "{}", samples.len());
    assert!(samples.len() <= 5 * 16_000, "{}", samples.len());
    assert!(samples.iter().any(|s| s.abs() > 0.1), "fixture is silent");
}

#[test]
#[ignore = "slow: loads a real Whisper model (WHISPER_TEST_MODEL)"]
fn transcribes_fixture_with_tiny_en() {
    let Some(model) = env_path("WHISPER_TEST_MODEL") else {
        return;
    };
    let samples = read_wav_16k_mono(&fixture());
    assert!(samples.len() <= 5 * 16_000, "fixture must be at most 5 s");
    let run = transcribe(
        &[
            "--model".as_ref(),
            model.as_os_str(),
            "--language".as_ref(),
            "en".as_ref(),
        ],
        &samples,
    );
    let paragraphs = assert_success(&run);
    assert_expected_words(&paragraphs);
}

#[test]
#[ignore = "slow: loads a real Whisper model (WHISPER_TEST_MODEL)"]
fn silence_gives_no_hallucinated_paragraphs_or_an_empty_result() {
    let Some(model) = env_path("WHISPER_TEST_MODEL") else {
        return;
    };
    // 2 s of digital silence: must succeed; any paragraphs must be non-empty.
    let run = transcribe(&["--model".as_ref(), model.as_os_str()], &[0.0; 32_000]);
    assert_success(&run);
}

#[test]
#[ignore = "slow: loads a real Whisper model (WHISPER_TEST_MODEL)"]
fn empty_audio_gives_an_empty_result() {
    let Some(model) = env_path("WHISPER_TEST_MODEL") else {
        return;
    };
    let run = transcribe(&["--model".as_ref(), model.as_os_str()], &[]);
    let paragraphs = assert_success(&run);
    assert!(paragraphs.is_empty(), "{paragraphs:?}");
}

#[test]
#[ignore = "slow: loads a real Whisper model (WHISPER_TEST_MODEL)"]
fn unknown_language_is_an_error_line() {
    let Some(model) = env_path("WHISPER_TEST_MODEL") else {
        return;
    };
    let run = transcribe(
        &[
            "--model".as_ref(),
            model.as_os_str(),
            "--language".as_ref(),
            "xx-nope".as_ref(),
        ],
        &[0.0; 16_000],
    );
    assert_eq!(run.code, Some(1), "stderr: {}", run.stderr);
    let last = run.lines.last().unwrap();
    assert_eq!(last["type"], "error");
}

#[test]
#[ignore = "slow: loads real Whisper and Silero VAD models (WHISPER_TEST_MODEL, WHISPER_TEST_VAD_MODEL)"]
fn transcribes_fixture_with_vad_and_keeps_original_timeline() {
    let (Some(model), Some(vad)) = (
        env_path("WHISPER_TEST_MODEL"),
        env_path("WHISPER_TEST_VAD_MODEL"),
    ) else {
        return;
    };
    // Pad the fixture with 6 s of leading silence, which VAD removes before
    // decoding. The expected words must survive splicing.
    let mut samples = vec![0.0f32; 6 * 16_000];
    samples.extend(read_wav_16k_mono(&fixture()));
    let run = transcribe(
        &[
            "--model".as_ref(),
            model.as_os_str(),
            "--vad-model".as_ref(),
            vad.as_os_str(),
            "--language".as_ref(),
            "en".as_ref(),
        ],
        &samples,
    );
    let paragraphs = assert_success(&run);
    assert_expected_words(&paragraphs);
    assert!(
        !run.stderr.contains("no --vad-model"),
        "VAD should be on: {}",
        run.stderr
    );
}

/// The fixture twice, separated by `gap_secs` of digital silence.
fn fixture_twice(gap_secs: usize) -> Vec<f32> {
    let clip = read_wav_16k_mono(&fixture());
    let mut samples = clip.clone();
    samples.extend(std::iter::repeat_n(0.0f32, gap_secs * 16_000));
    samples.extend(clip);
    samples
}

fn assert_two_paragraphs_of_the_clip(paragraphs: &[String]) {
    assert_eq!(paragraphs.len(), 2, "{paragraphs:?}");
    for p in paragraphs {
        assert!(p.to_lowercase().contains("americans"), "{paragraphs:?}");
    }
}

#[test]
#[ignore = "slow: loads a real Whisper model (WHISPER_TEST_MODEL)"]
fn long_pause_between_sentences_starts_a_paragraph() {
    // Regression: segment timestamps abut, so pauses must come from word timing.
    let Some(model) = env_path("WHISPER_TEST_MODEL") else {
        return;
    };
    let run = transcribe(
        &[
            "--model".as_ref(),
            model.as_os_str(),
            "--language".as_ref(),
            "en".as_ref(),
        ],
        &fixture_twice(3),
    );
    assert_two_paragraphs_of_the_clip(&assert_success(&run));
}

#[test]
#[ignore = "slow: loads real Whisper and Silero VAD models (WHISPER_TEST_MODEL, WHISPER_TEST_VAD_MODEL)"]
fn long_pause_removed_by_vad_still_starts_a_paragraph() {
    let (Some(model), Some(vad)) = (
        env_path("WHISPER_TEST_MODEL"),
        env_path("WHISPER_TEST_VAD_MODEL"),
    ) else {
        return;
    };
    let run = transcribe(
        &[
            "--model".as_ref(),
            model.as_os_str(),
            "--vad-model".as_ref(),
            vad.as_os_str(),
            "--language".as_ref(),
            "en".as_ref(),
        ],
        &fixture_twice(3),
    );
    assert_two_paragraphs_of_the_clip(&assert_success(&run));
}
