// Pure logic: where assets live, which are needed on this platform, and which are missing.
import {
	MODEL,
	PARAGRAPH_MODEL,
	PARAGRAPH_TOKENIZER,
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
 * Release variant for this machine, or null when there is no published build
 * (unsupported platform/arch).
 */
export function sidecarVariant(
	platform: Platform,
	arch: string,
	backend: LinuxBackend,
): SidecarVariant | null {
	if (platform === "darwin") return arch === "arm64" ? "macos-arm64" : null;
	if (platform === "linux" && arch === "x64") {
		return backend === "vulkan" ? "linux-x64-vulkan" : "linux-x64-cpu";
	}
	return null;
}

export interface AssetSpec {
	id: AssetId;
	label: string;
	url: string;
	dest: string;
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
	backend: LinuxBackend;
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
		{
			id: "paragraph-model",
			label: PARAGRAPH_MODEL.label,
			url: PARAGRAPH_MODEL.url,
			dest: `${models}/${PARAGRAPH_MODEL.fileName}`,
			sizeBytes: PARAGRAPH_MODEL.sizeBytes,
			sha256: PARAGRAPH_MODEL.sha256,
		},
		{
			id: "paragraph-tokenizer",
			label: PARAGRAPH_TOKENIZER.label,
			url: PARAGRAPH_TOKENIZER.url,
			dest: `${models}/${PARAGRAPH_TOKENIZER.fileName}`,
			sizeBytes: PARAGRAPH_TOKENIZER.sizeBytes,
			sha256: PARAGRAPH_TOKENIZER.sha256,
		},
	];
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
