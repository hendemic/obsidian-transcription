// Obsidian-free orchestration of asset status and downloads.
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, rm } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname } from "node:path";
import { promisify } from "node:util";
import { SIDECAR_CHECKSUMS_ASSET, releaseAssetUrl } from "./config";
import { downloadVerified, fetchText } from "./download";
import {
	buildAssetPlan,
	findAsset,
	isCudaUsable,
	missingAssets,
	parseSha256Sums,
	resolveDataDir,
	resolveLinuxBackend,
	type AssetSpec,
} from "./plan";
import type { LocalWhisperSettings } from "../settings-model";

const execFileAsync = promisify(execFile);

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
}

export class AssetManager {
	private cudaUsable: boolean | null = null;
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

	/** Whether `nvidia-smi` runs and the CUDA 12 runtime libs are loadable; cached after the first check. */
	async detectNvidia(): Promise<boolean> {
		if (this.cudaUsable === null) {
			let hasSmi = false;
			try {
				await execFileAsync("nvidia-smi", ["-L"], { timeout: 10_000 });
				hasSmi = true;
			} catch {
				hasSmi = false;
			}
			let ldconfig: string | null = null;
			if (hasSmi) {
				try {
					const { stdout } = await execFileAsync("ldconfig", ["-p"], { timeout: 10_000 });
					ldconfig = String(stdout);
				} catch {
					ldconfig = null; // ldconfig unavailable: fall back to Vulkan
				}
			}
			this.cudaUsable = isCudaUsable(hasSmi, ldconfig);
		}
		return this.cudaUsable;
	}

	async plan(): Promise<AssetSpec[]> {
		const settings = this.getSettings();
		const backend =
			process.platform === "linux"
				? resolveLinuxBackend(settings.linuxBackend, await this.detectNvidia())
				: "cpu"; // unused off Linux; macOS has a single build
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
		const sidecar = settings.sidecarPath !== "" ? settings.sidecarPath : findAsset(plan, "sidecar")?.dest;
		if (!model || !vad || !sidecar) throw new Error("Asset plan is incomplete");
		return { sidecar, model, vad };
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
				const target = asset.archivePath ?? asset.dest;
				await downloadVerified({
					url: asset.url,
					destPath: target,
					sha256,
					signal: controller.signal,
					onProgress: (received, total) =>
						onProgress({ asset, index, count: todo.length, received, total }),
				});
				await this.finalize(asset, target);
			}
		} finally {
			this.abort = null;
		}
	}

	private async finalize(asset: AssetSpec, downloaded: string): Promise<void> {
		if (asset.executable) {
			await chmod(downloaded, 0o755);
		}
		if (asset.id === "coreml") {
			// macOS ships /usr/bin/unzip. Extract next to the model, then drop the archive.
			await execFileAsync("unzip", ["-o", "-q", downloaded, "-d", dirname(downloaded)]);
			await rm(downloaded, { force: true });
			if (!existsSync(asset.dest)) throw new Error("Core ML encoder archive did not contain the expected folder");
		}
	}
}
