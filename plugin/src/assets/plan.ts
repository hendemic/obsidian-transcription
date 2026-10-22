// Pure logic: where assets live, which are needed on this platform, and which are missing.
import {
	COREML_ENCODER,
	MODEL,
	SIDECAR_APPROX_SIZE_BYTES,
	SIDECAR_ASSET_NAMES,
	type AssetId,
	type SidecarVariant,
	PLUGIN_ID,
	VAD_MODEL,
	releaseAssetUrl,
} from "./config";
import type { LinuxBackend } from "../settings-model";

/** Node `process.platform` value; "darwin" and "linux" are supported. */
export type Platform = string;
export type Env = Record<string, string | undefined>;
export type ResolvedBackend = "cuda" | "vulkan" | "cpu";

/** Data directory outside the vault, so large models do not sync. */
export function resolveDataDir(platform: Platform, env: Env, home: string): string {
	if (platform === "darwin") {
		return joinPath(home, "Library", "Application Support", PLUGIN_ID);
	}
	if (platform === "linux") {
		const xdg = env.XDG_DATA_HOME;
		// The XDG spec says relative paths are invalid and must be ignored.
		const base = xdg && xdg.startsWith("/") ? xdg : joinPath(home, ".local", "share");
		return joinPath(base, PLUGIN_ID);
	}
	throw new Error(`Unsupported platform: ${platform}`);
}

function joinPath(...parts: string[]): string {
	return parts.map((p, i) => (i === 0 ? p.replace(/\/+$/, "") : p.replace(/^\/+|\/+$/g, ""))).join("/");
}

/**
 * Whether the CUDA build is usable: `nvidia-smi` is present AND `ldconfig -p` lists the CUDA 12
 * runtime libraries. `ldconfigOutput` is null when `ldconfig` could not be run (assume not usable).
 */
export function isCudaUsable(hasNvidiaSmi: boolean, ldconfigOutput: string | null): boolean {
	if (!hasNvidiaSmi || ldconfigOutput === null) return false;
	return /\blibcudart\.so\.12\b/.test(ldconfigOutput) && /\blibcublas\.so\.12\b/.test(ldconfigOutput);
}

/** "auto" picks CUDA when `cudaUsable` (see isCudaUsable), else Vulkan. CPU only when chosen explicitly. */
export function resolveLinuxBackend(setting: LinuxBackend, cudaUsable: boolean): ResolvedBackend {
	if (setting === "auto") return cudaUsable ? "cuda" : "vulkan";
	return setting;
}

/**
 * Release variant for this machine, or null when there is no published build
 * (unsupported platform/arch).
 */
export function sidecarVariant(
	platform: Platform,
	arch: string,
	backend: ResolvedBackend,
): SidecarVariant | null {
	if (platform === "darwin") return arch === "arm64" ? "macos-arm64" : null;
	if (platform === "linux" && arch === "x64") {
		return backend === "cuda" ? "linux-x64-cuda" : backend === "vulkan" ? "linux-x64-vulkan" : "linux-x64-cpu";
	}
	return null;
}

export interface AssetSpec {
	id: AssetId;
	label: string;
	url: string;
	/** Final location (for the Core ML encoder: the extracted directory). */
	dest: string;
	/** Download target when it differs from `dest` (the Core ML zip). */
	archivePath?: string;
	sizeBytes: number;
	/** Hex SHA-256, or null when it must be read from the release's SHA256SUMS. */
	sha256: string | null;
	/** Release asset name, for sidecar checksum lookup. */
	releaseAssetName?: string;
	executable?: boolean;
}

export interface PlanInput {
	platform: Platform;
	arch: string;
	backend: ResolvedBackend;
	dataDir: string;
	pluginVersion: string;
	/** When true the sidecar comes from the settings override and is not planned for download. */
	sidecarOverridden: boolean;
}

export function buildAssetPlan(input: PlanInput): AssetSpec[] {
	const models = `${input.dataDir}/models`;
	const plan: AssetSpec[] = [
		{
			id: "model",
			label: MODEL.label,
			url: MODEL.url,
			dest: `${models}/${MODEL.fileName}`,
			sizeBytes: MODEL.sizeBytes,
			sha256: MODEL.sha256,
		},
		{
			id: "vad",
			label: VAD_MODEL.label,
			url: VAD_MODEL.url,
			dest: `${models}/${VAD_MODEL.fileName}`,
			sizeBytes: VAD_MODEL.sizeBytes,
			sha256: VAD_MODEL.sha256,
		},
	];
	if (input.platform === "darwin") {
		plan.push({
			id: "coreml",
			label: COREML_ENCODER.label,
			url: COREML_ENCODER.url,
			dest: `${models}/${COREML_ENCODER.extractedDirName}`,
			archivePath: `${models}/${COREML_ENCODER.fileName}`,
			sizeBytes: COREML_ENCODER.sizeBytes,
			sha256: COREML_ENCODER.sha256,
		});
	}
	if (!input.sidecarOverridden) {
		const variant = sidecarVariant(input.platform, input.arch, input.backend);
		if (!variant) {
			throw new Error(`No transcription engine build is available for ${input.platform}/${input.arch}.`);
		}
		const name = SIDECAR_ASSET_NAMES[variant];
		plan.push({
			id: "sidecar",
			label: `Transcription engine (${variant})`,
			url: releaseAssetUrl(input.pluginVersion, name),
			dest: `${input.dataDir}/bin/${input.pluginVersion}/${name}`,
			sizeBytes: SIDECAR_APPROX_SIZE_BYTES,
			sha256: null,
			releaseAssetName: name,
			executable: true,
		});
	}
	return plan;
}

/** Assets from the plan that are not present on disk. */
export function missingAssets(plan: readonly AssetSpec[], exists: (path: string) => boolean): AssetSpec[] {
	return plan.filter((a) => !exists(a.dest));
}

export function findAsset(plan: readonly AssetSpec[], id: AssetId): AssetSpec | undefined {
	return plan.find((a) => a.id === id);
}

/** Parse a `sha256sum` style file ("<hex>  <name>" or "<hex> *<name>") into name -> hex. */
export function parseSha256Sums(text: string): Map<string, string> {
	const out = new Map<string, string>();
	for (const line of text.split(/\r?\n/)) {
		const m = /^([0-9a-fA-F]{64})\s+\*?(.+?)\s*$/.exec(line);
		if (m?.[1] && m[2]?.trim()) out.set(m[2], m[1].toLowerCase());
	}
	return out;
}
