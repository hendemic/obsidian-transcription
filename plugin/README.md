# Local Whisper Transcribe

An Obsidian plugin that transcribes audio recordings embedded in your notes, locally, using Whisper large-v3 on your own GPU. Your audio never leaves your machine. The transcript is inserted as plain text, broken into paragraphs where the topic shifts, using pauses to settle borderline cases. It is aimed at spoken journal entries, where accuracy matters more than speed.

Desktop only. Supported: macOS (Apple Silicon) and Linux (x64).

## Usage

- **In a note:** right-click an audio embed (`![[recording.webm]]`) in Live Preview or Reading view and choose **Transcribe**. The transcript is inserted on the lines after the embed.
- **From the file explorer:** right-click an audio file and choose **Transcribe**. A new note `<audio name> transcript.md` is created next to the audio, containing the embed and the transcript.
- A progress notice shows loading and percent complete. Use its **Cancel** button to stop. Only one transcription runs at a time.

Supported audio formats are whatever Obsidian's built-in player decodes: m4a, mp3, wav, webm, ogg, flac, and 3gp.

## Settings

| Setting | Meaning |
| --- | --- |
| Language | `auto` (default) or a code such as `en`. |
| Initial prompt | Names or terms to bias the transcript toward. |
| Pause gap (seconds) | Only used if the paragraph model is missing or fails to load: then a silence at least this long after a sentence starts a new paragraph. Pauses in the middle of a sentence never do. Default 1.8. |
| GPU backend (Linux only) | Auto, CUDA, Vulkan, or CPU. Auto uses CUDA only if `nvidia-smi` is on your PATH and `ldconfig -p` lists both `libcudart.so.12` and `libcublas.so.12`; otherwise (including when `ldconfig` is unavailable) it uses Vulkan. CPU downloads a build without GPU support, which is much slower. |
| Sidecar path override | Use your own `transcribe-cli` binary instead of the downloaded one (for development). |
| Models and engine | Shows what is downloaded and lets you download what is missing. |

**CUDA requirements:** the CUDA build needs an NVIDIA driver 550 or newer **and** the CUDA 12 runtime libraries (`libcudart` and `libcublas`, from the CUDA toolkit or your distro's CUDA runtime packages). Auto mode checks for these libraries and falls back to Vulkan when they are missing. If you only have the driver installed, choose **Vulkan**, which also runs on NVIDIA GPUs and needs no extra install.

## Downloads (network use disclosure)

The plugin does nothing on the network until you click **Download** in its settings, or confirm the prompt shown the first time you use **Transcribe** with files missing. Every download is verified against a SHA-256 checksum before it is used; a file that does not match is deleted.

| File | Source | Size | Platforms |
| --- | --- | --- | --- |
| Whisper large-v3 model (`ggml-large-v3.bin`) | [Hugging Face, ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) | about 3.1 GB | all |
| Silero VAD model (`ggml-silero-v5.1.2.bin`) | [Hugging Face, ggml-org/whisper-vad](https://huggingface.co/ggml-org/whisper-vad) | about 0.9 MB | all |
| Paragraph model (`paragraph-minilm-l12-int8.onnx`, int8 ONNX export of paraphrase-multilingual-MiniLM-L12-v2) and its tokenizer (`paragraph-minilm-l12-tokenizer.json`) | [Hugging Face, sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2](https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2) | about 118 MB + 9 MB | all |
| Core ML encoder (`ggml-large-v3-encoder.mlmodelc.zip`, extracted) | [Hugging Face, ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) | about 1.2 GB | macOS only |
| Transcription engine (`transcribe-cli-<platform>`) | [GitHub releases of hendemic/obsidian-transcription](https://github.com/hendemic/obsidian-transcription/releases), tag `v<plugin version>`, checksums from the release's `SHA256SUMS` | small | macOS arm64, Linux x64 (CUDA, Vulkan or CPU) |

Files are stored **outside your vault**, so they are not synced:

- macOS: `~/Library/Application Support/local-whisper-transcribe/`
- Linux: `$XDG_DATA_HOME/local-whisper-transcribe/` (default `~/.local/share/local-whisper-transcribe/`)

The downloaded engine is a native executable that the plugin runs on your machine to do the transcription. On Linux the plugin also runs `nvidia-smi` and `ldconfig -p` once to detect an NVIDIA GPU and the CUDA runtime. On macOS it runs the system `unzip` to extract the Core ML encoder. Audio is passed to the engine over a pipe; nothing is uploaded.

To remove everything, delete the data directory above.

## Development

```sh
npm install
npm run dev        # watch build
npm run typecheck && npm run lint && npm test && npm run build
```

See `docs/plan.md` in the repository for the architecture and the sidecar protocol.
