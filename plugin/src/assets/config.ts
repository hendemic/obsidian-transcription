// Single source of truth for everything the plugin downloads.
// Keep this in sync with the "Downloads and privacy" section of README.md.

export const PLUGIN_ID = "local-whisper-transcribe";

export const RELEASE_REPO = "hendemic/obsidian-transcription";

export type AssetId = "model" | "vad" | "paragraph-model" | "paragraph-tokenizer" | "sidecar";

export interface FileAssetConfig {
	label: string;
	url: string;
	/** File name under the data directory's models/ folder. */
	fileName: string;
	sizeBytes: number;
	/** SHA-256 (hex) from Hugging Face's LFS metadata. */
	sha256: string;
}

/**
 * Close to large-v3 accuracy with a much smaller decoder. q5_0 keeps the download under 600 MB. No
 * Core ML encoder: it is unquantized (1.2 GB), so macOS runs the whole model on Metal instead.
 */
export const MODEL: FileAssetConfig = {
	label: "Whisper large-v3-turbo model (q5_0)",
	url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
	fileName: "ggml-large-v3-turbo-q5_0.bin",
	sizeBytes: 574_041_195,
	sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
};

export const VAD_MODEL: FileAssetConfig = {
	label: "Silero VAD model",
	url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin",
	fileName: "ggml-silero-v5.1.2.bin",
	sizeBytes: 885_098,
	sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
};

const PARAGRAPH_REPO = "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/main";

/**
 * Sentence-embedding model (~50 languages) the engine uses to break paragraphs where the topic
 * shifts. The int8 export runs on any CPU; the "arm64" in its name is only the quantization recipe.
 */
export const PARAGRAPH_MODEL: FileAssetConfig = {
	label: "Paragraph model (multilingual MiniLM)",
	url: `${PARAGRAPH_REPO}/onnx/model_qint8_arm64.onnx`,
	fileName: "paragraph-minilm-l12-int8.onnx",
	sizeBytes: 118_412_398,
	sha256: "783fea82d71a58179b830a4dbd2d58447e640609e98eedf9ffa12622d375a672",
};

export const PARAGRAPH_TOKENIZER: FileAssetConfig = {
	label: "Paragraph model tokenizer",
	url: `${PARAGRAPH_REPO}/tokenizer.json`,
	fileName: "paragraph-minilm-l12-tokenizer.json",
	sizeBytes: 9_081_518,
	sha256: "2c3387be76557bd40970cec13153b3bbf80407865484b209e655e5e4729076b8",
};

export type SidecarVariant = "macos-arm64" | "linux-x64-vulkan" | "linux-x64-cpu";

export const SIDECAR_ASSET_NAMES: Record<SidecarVariant, string> = {
	"macos-arm64": "transcribe-cli-macos-arm64",
	"linux-x64-vulkan": "transcribe-cli-linux-x64-vulkan",
	"linux-x64-cpu": "transcribe-cli-linux-x64-cpu",
};

/**
 * Release assets are fetched from the GitHub release tagged with the bare plugin version (`1.2.3`, no
 * "v"), which is the tag format Obsidian requires for community plugin releases.
 * TODO(release): no release exists yet, so sidecar checksums cannot be pinned here. They are read at
 * download time from the release's `SHA256SUMS` asset instead; a download without a matching entry fails.
 */
export const SIDECAR_CHECKSUMS_ASSET = "SHA256SUMS";
/** Approximate size shown in the UI only; the real size comes from the download. */
export const SIDECAR_APPROX_SIZE_BYTES = 35_000_000;

export function releaseAssetUrl(version: string, assetName: string): string {
	return `https://github.com/${RELEASE_REPO}/releases/download/${version}/${assetName}`;
}
