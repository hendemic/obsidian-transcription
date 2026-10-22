# Plan: local Whisper transcription for Obsidian

An Obsidian plugin that adds **Transcribe** to the right-click menu of an embedded audio recording. It transcribes the audio locally with Whisper on the GPU and inserts the transcript as plain text, broken into paragraphs at natural pauses, under the embed. Primary use: spoken journal entries, where accuracy matters more than speed.

## Scope

- **Platforms:** macOS (Apple Silicon) and Linux (x64). Desktop only.
- **Out of scope for now:** Windows, mobile, speaker diarization, live/streaming transcription, ROCm builds, WhisperKit, MLX.

## Architecture

```
Obsidian plugin (TypeScript)
  right-click on audio embed / file → "Transcribe"
  → decode audio (Chromium decodeAudioData) → resample to 16 kHz mono f32 (OfflineAudioContext)
  → spawn sidecar, write PCM to stdin, read JSON lines from stdout
  → insert paragraphs under the embed

Rust sidecar (CLI binary)
  Transcriber trait  → WhisperCpp (whisper-rs; GPU backend chosen by cargo feature)
  sentences()        → split at sentence ends, measure the pause after each
  Paragrapher trait  → ContentGate<SentenceEmbedder> (OnnxEmbedder, ONNX Runtime), or PauseGap
  run<T: Transcriber, P: Paragrapher>(…) → JSON lines on stdout
```

Why this split:

- Obsidian plugins must be JS/TS. A separate process (not a napi addon) avoids Electron ABI breakage on Obsidian updates, and a GPU crash can't take Obsidian down.
- Audio decoding happens in the plugin because Obsidian's recorder writes webm/Opus, which Rust's symphonia can't decode. Chromium decodes webm, m4a, mp3, wav, ogg and flac, so the sidecar only ever sees one input format and there's no ffmpeg dependency.
- whisper.cpp selects its GPU backend at compile time, so hardware variety comes from separate builds. The traits sit at the engine and pipeline level, where a second implementation is realistic (see [Future work](#future-work)).

### Repository layout

```
plugin/                    Obsidian plugin (TS, esbuild, based on the official sample plugin)
sidecar/                   Cargo workspace
  crates/transcribe-core/  traits, domain types, sentences(), PauseGap, protocol types, run()
  crates/engine-whispercpp/ WhisperCpp: impl Transcriber (whisper-rs)
  crates/embed-onnx/       OnnxEmbedder: impl SentenceEmbedder (ONNX Runtime, statically linked)
  crates/transcribe-cli/   the sidecar binary
.github/workflows/         CI and release builds
docs/
```

### Rust core (`transcribe-core`)

```rust
pub struct Pcm16kMono(pub Vec<f32>);

pub struct Segment { pub start: Duration, pub end: Duration, pub text: String, pub words: Vec<Word> }
pub struct Word { pub start: Duration, pub end: Duration, pub text: String } // one decoder token
pub struct Silence { pub start: Duration, pub end: Duration }                // a gap VAD found
pub struct Transcript { pub segments: Vec<Segment>, pub silences: Vec<Silence> }

pub struct Options {
    pub language: Option<String>,      // None = auto-detect
    pub initial_prompt: Option<String>, // names/terms to bias toward
}

pub enum Progress { Loading, Transcribing { percent: u8 } }

pub trait Transcriber {
    type Error: std::error::Error + Send + Sync + 'static;
    fn transcribe(
        &self,
        pcm: &Pcm16kMono,
        opts: &Options,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Transcript, Self::Error>;
}

pub struct Sentence { pub text: String, pub pause_after: Duration }
pub fn sentences(transcript: &Transcript) -> Vec<Sentence>;

pub trait Paragrapher {
    fn paragraphs(&self, sentences: &[Sentence]) -> Vec<String>;
}

/// Starts a new paragraph after a sentence followed by a pause of at least `min_gap`.
pub struct PauseGap { pub min_gap: Duration }
```

`run<T: Transcriber, P: Paragrapher>(engine, paragrapher, pcm, opts, out: impl Write)` drives a transcription and writes protocol messages. It has no whisper.cpp dependency, so it's tested with a fake `Transcriber`.

`sentences` rules: a sentence ends at sentence-ending punctuation (`.?!…` and CJK, Arabic and Devanagari forms, optionally followed by closing quotes), or before a word that follows a pause of at least 3 s when Whisper left a stretch unpunctuated. Paragraph breaks are only considered at sentence ends, so a pause to think mid-sentence never splits a paragraph. The pause after a sentence is the larger of the gap between its last spoken word and the next sentence's first (word timing) and the VAD silence around that gap. Segment timestamps are not used: whisper.cpp segments abut, with the silence absorbed into one of them, so the gap between segments is almost always zero. Segments are trimmed and joined with a single space.

`PauseGap` rules: join sentences within a paragraph with a single space, drop empty sentences, never produce an empty paragraph or one containing a line break. Default `min_gap` is 1.8 s (a plugin setting).

`ContentGate` rules (the default when the paragraph model is installed): each sentence boundary gets a topic-drift score, one minus the cosine similarity between the mean embedding of the two sentences before it and the two after it. Drift and pause are ranked within the transcript so both adapt to the speaker. Strong drift (top 15% and at least 0.75) breaks whatever the pause; weak drift (bottom 40% or under 0.5) never breaks, however long the pause, so a pause to think or to land a point stays inside the paragraph; borderline drift breaks only when the pause is also in the speaker's top 30% and at least 0.5 s. Paragraphs have at least two sentences. The embedding model is paraphrase-multilingual-MiniLM-L12-v2 (int8 ONNX, about 50 languages, mean pooling), chosen over SaT/wtpsplit (400 MB+) and the English-only all-MiniLM-L6-v2 after comparing them on test recordings.

### whisper.cpp engine (`engine-whispercpp`)

- `WhisperCpp::new(model_path, config)` loads the model once; `impl Transcriber`.
- Cargo features passed through to whisper-rs: `metal`, `coreml`, `cuda`, `vulkan`. No feature = CPU (used for development and CI tests).
- Model: **ggml base** (148 MB) for 0.1.0, the model it was tested with. large-v3 (f16 or q8_0) remains the target for accuracy; switching is a change to `plugin/src/assets/config.ts`.
- macOS: Metal plus the Core ML encoder (`<model stem>-encoder.mlmodelc`, next to the model file) so the encoder runs on the Neural Engine.
- **Silero VAD on by default** (whisper.cpp's built-in VAD with the ggml Silero model). large-v3 hallucinates and repeats during silence, and journal recordings have pauses.
- Pass `initial_prompt` and `language` through. Map whisper.cpp's progress callback to `Progress::Transcribing`.
- Token timestamps on: each segment's tokens become `Word`s (special tokens dropped, tokens merged where a character is split across them), and the gaps between VAD speech spans become `Silence`s.

### Sidecar protocol (`transcribe-cli`)

This is the contract between the plugin and the sidecar. Plugin and sidecar work can proceed in parallel against it.

```
transcribe-cli probe
  → stdout: {"version":"0.1.0","backend":"metal+coreml"|"cuda"|"vulkan"|"cpu"}

transcribe-cli transcribe --model <path> [--vad-model <path>] [--language <code>]
                          [--prompt <text>] [--pause-gap-ms <n>]
                          [--paragraph-model <path> --paragraph-tokenizer <path>]
  stdin:  raw little-endian f32 PCM, 16 kHz mono, until EOF
  stdout: one JSON object per line
    {"type":"progress","stage":"loading"}
    {"type":"progress","stage":"transcribing","percent":42}
    {"type":"result","paragraphs":["…","…"]}
    {"type":"error","message":"…"}
  exit: 0 on result, 1 on error (an error line is written first when possible)
```

The Core ML encoder is found by whisper.cpp next to the model, so it needs no flag. With the paragraph model, paragraphs break by content (`ContentGate`) and `--pause-gap-ms` only applies if the model fails to load or run, which is reported on stderr but never fails the transcription. Diagnostics go to stderr only. Cancellation is the plugin killing the process.

### Plugin

- **Menu entry points:**
  - Right-click on a rendered audio embed in Live Preview and Reading view: `registerDomEvent(document, "contextmenu", …)`, match `.internal-embed` elements whose `src` has an audio extension, show `new Menu()` with **Transcribe**.
  - `file-menu` event for audio files in the file explorer (writes to a new note next to the audio, see below).
- **Decoding:** read the file with `vault.readBinary`, `AudioContext.decodeAudioData`, then render through a 16 kHz mono `OfflineAudioContext` to get `Float32Array` PCM.
- **Run:** spawn the sidecar with `child_process.spawn`, stream PCM to stdin, parse stdout line by line, show progress in a `Notice` with a Cancel button. Allow only one transcription at a time.
- **Insertion:** with `vault.process`, insert the paragraphs (separated by blank lines, plain text, no callout or heading) on the lines after the embed that was clicked. If the clicked embed can't be mapped to a line, use the first occurrence of that embed in the note. From the file explorer, create `<audio name> transcript.md` in the same folder, embedding the audio followed by the transcript.
- **Settings:** language (auto by default), initial prompt, pause gap (seconds), Linux backend (vulkan / cpu), sidecar path override (for development), and model status with a download button.
- **Asset manager:**
  - Data directory outside the vault so 3 GB models don't sync: `~/Library/Application Support/<plugin-id>/` on macOS, `$XDG_DATA_HOME/<plugin-id>/` (default `~/.local/share`) on Linux.
  - Downloads, with progress and SHA-256 verification: the Whisper ggml model, the Silero VAD model and the paragraph model (int8 ONNX export of paraphrase-multilingual-MiniLM-L12-v2, with its tokenizer.json) from Hugging Face; on macOS also the Core ML encoder (zip, extracted); the sidecar build for this OS/backend from this repo's GitHub releases. Mark the sidecar executable.
  - Linux uses the Vulkan build (AMD, NVIDIA and Intel GPUs) unless the user picks CPU. There is no CUDA or ROCm build: Vulkan needs no extra runtime install and treats all vendors the same.
  - Downloads happen only after the user clicks download in settings or confirms a prompt on first use. The plugin README discloses them (required by Obsidian's community plugin rules).
- **Plugin id:** Obsidian disallows "obsidian" in plugin ids. Working name: **Local Whisper Transcribe** (`local-whisper-transcribe`).

### CI and releases

- CI on push: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (CPU build); plugin typecheck, lint, test, build.
- Release workflow on tag `X.Y.Z` (no "v", as Obsidian requires): build `transcribe-cli` for `macos-arm64` (`metal,coreml`), `linux-x64-vulkan`, `linux-x64-cpu` (built on Ubuntu 24.04, so glibc 2.39+); attach them with SHA-256 sums, plus `main.js`, `manifest.json` and `styles.css`, to the GitHub release.

### Testing

- Rust unit tests: `PauseGap` (gap thresholds, empty segments, whitespace), protocol serialization, `run()` with a fake `Transcriber` (progress, result and error paths), PCM reading from stdin (partial reads, odd byte counts).
- Rust integration test, marked slow/ignored by default: CPU build, `ggml-tiny.en` model, short committed WAV fixture, checks that the expected words appear.
- Plugin unit tests (vitest): embed detection from DOM and from link text, insertion of paragraphs after the right line, protocol line parsing, release variant selection.
- Manual (user-only): GPU builds on macOS and on a Linux machine with an NVIDIA or AMD GPU. The development VM has no GPU, so agents skip GPU and real-model tests there (see `## Harness: user-only checks` in `CLAUDE.md`).

## Work items

Suggested breakdown for `/harness:workstream docs/plan.md`. Sizes are judgments.

1. **Scaffold** (small): cargo workspace with the three crates, plugin from the official sample plugin template, CI workflow, and the harness quality gates and testing layout sections added to the existing `CLAUDE.md` (keep its user-only checks section).
2. **Core crate** (medium): domain types, `Transcriber` and `Paragrapher` traits, `PauseGap`, protocol types, generic `run()`. Depends on 1.
3. **whisper.cpp engine** (large): `WhisperCpp` with feature flags, VAD, prompt/language, progress mapping. Depends on 2.
4. **Sidecar CLI** (small): `probe` and `transcribe` subcommands (clap), stdin PCM reader, wiring engine + `PauseGap` into `run()`. Depends on 2 and 3.
5. **Rust tests** (test): unit and slow integration tests described above. Depends on 2–4.
6. **Plugin menu and settings** (medium): embed and file-menu detection, settings tab. Depends on 1.
7. **Plugin transcription flow** (medium): decode/resample, sidecar client against the protocol above, progress notice and cancel, insertion. Depends on 6 (not on the Rust work, thanks to the protocol contract).
8. **Asset manager** (medium): data directory, downloads with checksums, backend detection, first-use prompt. Depends on 6.
9. **Release workflow** (medium): build matrix, checksums, release assets. Depends on 4.

## Future work

- **If macOS is too slow:** add an MLX engine via `mlx-rs` (or WhisperKit) as a second `impl Transcriber`, built only for macOS. Nothing outside the engine crate and the CLI wiring should change.
- ROCm build for AMD on Linux if Vulkan underperforms.
- Windows support.
