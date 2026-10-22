# Local Whisper Transcribe

Transcribe audio recordings in your notes with Whisper, entirely on your machine. Transcripts are split into paragraphs where the topic changes.

Desktop only: macOS (Apple Silicon) and Linux (x64).

## Use

Right-click an audio embed in a note, or an audio file in the file explorer, and choose **Transcribe**. The first time, the plugin asks to download what it needs (about 350 MB).

## Settings

- **Language:** `auto`, or a code such as `en`.
- **Initial prompt:** names or terms the transcript should spell correctly.
- **GPU backend (Linux):** Auto uses CUDA when available, otherwise Vulkan. CUDA needs NVIDIA driver 550+ and the CUDA 12 runtime; with only the driver, choose Vulkan.

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

The transcription engine is a native program the plugin runs locally. The plugin also runs `nvidia-smi` and `ldconfig -p` on Linux to detect CUDA, and `unzip` on macOS to extract the Core ML encoder.
