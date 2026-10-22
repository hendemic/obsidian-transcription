# Local Whisper Transcribe

Transcribe audio recordings in your notes with Whisper, entirely on your machine. Transcripts are split into paragraphs where the topic changes.

Desktop only: macOS (Apple Silicon) and Linux (x64, glibc 2.39+, e.g. Ubuntu 24.04 or newer).

## Install

Not in the community plugin list yet. To install by hand:

1. Download `main.js`, `manifest.json` and `styles.css` from the [latest release](https://github.com/hendemic/obsidian-transcription/releases/latest).
2. Put them in `<your vault>/.obsidian/plugins/local-whisper-transcribe/`.
3. In Obsidian, open **Settings → Community plugins** and enable **Local Whisper Transcribe**.

## Use

Right-click an audio embed in a note, or an audio file in the file explorer, and choose **Transcribe**. The first time, the plugin asks to download what it needs (about 350 MB).

## Settings

- **Language:** `auto`, or a code such as `en`.
- **Initial prompt:** names or terms the transcript should spell correctly.
- **GPU backend (Linux):** GPU (Vulkan, for AMD, NVIDIA and Intel graphics) or CPU.

## Downloads and privacy

Nothing is downloaded until you confirm. Every file is checked against a SHA-256 checksum. Your audio is never uploaded.

| File | Source | Size |
| --- | --- | --- |
| Whisper base model | [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) | 148 MB |
| Core ML encoder (macOS only) | [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) | 38 MB |
| Paragraph model and tokenizer | [sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2](https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2) | 127 MB |
| Voice detection model | [ggml-org/whisper-vad](https://huggingface.co/ggml-org/whisper-vad) | 1 MB |
| Transcription engine | [this repo's releases](https://github.com/hendemic/obsidian-transcription/releases) | 35 MB |

Files are stored outside your vault, in `~/Library/Application Support/local-whisper-transcribe/` (macOS) or `~/.local/share/local-whisper-transcribe/` (Linux). Delete that folder to remove them.

The transcription engine is a native program the plugin runs locally. On macOS the plugin also runs `unzip` to extract the Core ML encoder.
