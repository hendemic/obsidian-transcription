// Obsidian UI helpers: progress notice, confirmation modal, download runner.
import { App, Modal, Notice, Setting } from "obsidian";
import type { AssetManager } from "./assets/manager";
import type { AssetSpec } from "./assets/plan";

export function formatBytes(n: number): string {
	if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
	if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
	return `${Math.max(1, Math.round(n / 1e3))} KB`;
}

export interface ProgressNotice {
	setText(text: string): void;
	hide(): void;
}

/** A persistent notice with a Cancel button. */
export function createProgressNotice(initial: string, onCancel: () => void): ProgressNotice {
	const frag = createFragment();
	const text = frag.createDiv({ text: initial });
	const button = frag.createEl("button", { text: "Cancel" });
	button.addEventListener("click", () => {
		button.disabled = true;
		onCancel();
	});
	const notice = new Notice(frag, 0);
	return {
		setText: (t) => text.setText(t),
		hide: () => notice.hide(),
	};
}

class ConfirmDownloadModal extends Modal {
	private choice = false;

	constructor(
		app: App,
		private readonly assets: AssetSpec[],
		private readonly resolve: (ok: boolean) => void,
	) {
		super(app);
	}

	onOpen(): void {
		const total = this.assets.reduce((n, a) => n + a.sizeBytes, 0);
		this.setTitle("Download transcription files?");
		this.contentEl.createEl("p", {
			text: `Local Whisper Transcribe needs to download about ${formatBytes(total)} before it can transcribe. Files are stored outside your vault and checked against SHA-256 checksums.`,
		});
		const list = this.contentEl.createEl("ul");
		for (const a of this.assets) list.createEl("li", { text: `${a.label} (${formatBytes(a.sizeBytes)})` });
		new Setting(this.contentEl)
			.addButton((b) => b.setButtonText("Cancel").onClick(() => this.close()))
			.addButton((b) =>
				b
					.setButtonText("Download")
					.setCta()
					.onClick(() => {
						this.choice = true;
						this.close();
					}),
			);
	}

	onClose(): void {
		this.contentEl.empty();
		this.resolve(this.choice);
	}
}

export function confirmDownload(app: App, assets: AssetSpec[]): Promise<boolean> {
	return new Promise((resolve) => new ConfirmDownloadModal(app, assets, resolve).open());
}

/** Run the missing downloads with a progress notice. Returns true on success. */
export async function downloadWithNotice(assets: AssetManager): Promise<boolean> {
	const notice = createProgressNotice("Starting download…", () => assets.cancelDownload());
	try {
		await assets.downloadMissing((p) => {
			const pct = p.total ? ` ${Math.floor((p.received / p.total) * 100)}%` : "";
			notice.setText(`Downloading ${p.asset.label} (${p.index + 1}/${p.count})${pct}`);
		});
		new Notice("Download complete.");
		return true;
	} catch (e) {
		new Notice(`Download failed: ${e instanceof Error ? e.message : String(e)}`);
		return false;
	} finally {
		notice.hide();
	}
}

/** First-use gate: if assets are missing, ask, then download. Returns true when everything is present. */
export async function ensureAssets(app: App, assets: AssetManager): Promise<boolean> {
	const missing = await assets.missing();
	if (missing.length === 0) return true;
	if (!(await confirmDownload(app, missing))) return false;
	return downloadWithNotice(assets);
}
