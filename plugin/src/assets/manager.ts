// Obsidian-free orchestration of asset status and downloads.
import { existsSync } from "node:fs";
import { chmod } from "node:fs/promises";
import { homedir } from "node:os";
import { SIDECAR_CHECKSUMS_ASSET, releaseAssetUrl } from "./config";
import { downloadVerified, fetchText } from "./download";
import {
	buildAssetPlan,
	findAsset,
	missingAssets,
	parseSha256Sums,
	resolveDataDir,
	type AssetSpec,
} from "./plan";
import type { LocalWhisperSettings } from "../settings-model";

export interface DownloadProgress {
	asset: AssetSpec;
	index: number;
	count: number;
	received: number;
	total: number | null;
}

export interface ResolvedPaths {
	sidecar: string;
	model: string;
	vad: string;
	paragraphModel: string;
	paragraphTokenizer: string;
}

export class AssetManager {
	private abort: AbortController | null = null;

	constructor(
		private readonly pluginVersion: string,
		private readonly getSettings: () => LocalWhisperSettings,
	) {}

	get downloading(): boolean {
		return this.abort !== null;
	}

	dataDir(): string {
		return resolveDataDir(process.platform, process.env, homedir());
	}

	async plan(): Promise<AssetSpec[]> {
		const settings = this.getSettings();
		// Only Linux has more than one build; macOS ignores the backend.
		const backend = settings.linuxBackend;
		return buildAssetPlan({
			platform: process.platform,
			arch: process.arch,
			backend,
			dataDir: this.dataDir(),
			pluginVersion: this.pluginVersion,
			sidecarOverridden: settings.sidecarPath !== "",
		});
	}

	async missing(): Promise<AssetSpec[]> {
		return missingAssets(await this.plan(), existsSync);
	}

	/** Paths to hand to the sidecar client. Throws if the plan cannot be built. Does not check existence. */
	async paths(): Promise<ResolvedPaths> {
		const plan = await this.plan();
		const settings = this.getSettings();
		const model = findAsset(plan, "model")?.dest;
		const vad = findAsset(plan, "vad")?.dest;
		const paragraphModel = findAsset(plan, "paragraph-model")?.dest;
		const paragraphTokenizer = findAsset(plan, "paragraph-tokenizer")?.dest;
		const sidecar = settings.sidecarPath !== "" ? settings.sidecarPath : findAsset(plan, "sidecar")?.dest;
		if (!model || !vad || !paragraphModel || !paragraphTokenizer || !sidecar) {
			throw new Error("Asset plan is incomplete");
		}
		return { sidecar, model, vad, paragraphModel, paragraphTokenizer };
	}

	cancelDownload(): void {
		this.abort?.abort();
	}

	/** Download and verify everything missing. Only call after the user has agreed. */
	async downloadMissing(onProgress: (p: DownloadProgress) => void): Promise<void> {
		if (this.abort) throw new Error("A download is already in progress");
		const controller = new AbortController();
		this.abort = controller;
		try {
			const todo = await this.missing();
			let sums: Map<string, string> | null = null;
			for (const [index, asset] of todo.entries()) {
				let sha256 = asset.sha256;
				if (sha256 === null && asset.releaseAssetName) {
					sums ??= parseSha256Sums(
						await fetchText(releaseAssetUrl(this.pluginVersion, SIDECAR_CHECKSUMS_ASSET), controller.signal),
					);
					sha256 = sums.get(asset.releaseAssetName) ?? null;
					if (!sha256) throw new Error(`No checksum for ${asset.releaseAssetName} in ${SIDECAR_CHECKSUMS_ASSET}`);
				}
				if (!sha256) throw new Error(`No checksum available for ${asset.label}`);
				await downloadVerified({
					url: asset.url,
					destPath: asset.dest,
					sha256,
					signal: controller.signal,
					onProgress: (received, total) =>
						onProgress({ asset, index, count: todo.length, received, total }),
				});
				if (asset.executable) await chmod(asset.dest, 0o755);
			}
		} finally {
			this.abort = null;
		}
	}
}
