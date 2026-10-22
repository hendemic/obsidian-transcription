import { existsSync } from "node:fs";
import { App, Platform, PluginSettingTab, Setting } from "obsidian";
import type LocalWhisperTranscribePlugin from "./main";
import { formatBytes } from "./ui";
import { clampPauseGapSeconds } from "./protocol";
import { LINUX_BACKENDS, type LinuxBackend } from "./settings-model";

export class LocalWhisperSettingTab extends PluginSettingTab {
	private statusEl: HTMLElement | null = null;
	private progressText = "";

	constructor(
		app: App,
		private readonly plugin: LocalWhisperTranscribePlugin,
	) {
		super(app, plugin);
	}

	display(): void {
		const { containerEl } = this;
		containerEl.empty();
		const settings = this.plugin.settings;

		new Setting(containerEl)
			.setName("Language")
			.setDesc('Spoken language as a code such as "en" or "de". Use "auto" to detect it.')
			.addText((t) =>
				t.setValue(settings.language).onChange(async (v) => {
					settings.language = v.trim() || "auto";
					await this.plugin.saveSettings();
				}),
			);

		new Setting(containerEl)
			.setName("Initial prompt")
			.setDesc("Names or terms to bias the transcript toward, for example people and places you mention.")
			.addTextArea((t) =>
				t.setValue(settings.initialPrompt).onChange(async (v) => {
					settings.initialPrompt = v;
					await this.plugin.saveSettings();
				}),
			);

		new Setting(containerEl)
			.setName("Pause gap (seconds)")
			.setDesc("Start a new paragraph when the silence between two phrases is at least this long (0.1 to 60).")
			.addText((t) =>
				t.setValue(String(settings.pauseGapSeconds)).onChange(async (v) => {
					const n = Number(v);
					if (Number.isFinite(n) && n > 0) {
						settings.pauseGapSeconds = clampPauseGapSeconds(n);
						await this.plugin.saveSettings();
					}
				}),
			);

		if (Platform.isLinux) {
			new Setting(containerEl)
				.setName("GPU backend")
				.setDesc("Automatic mode uses the NVIDIA build only when its runtime libraries are installed, otherwise the generic GPU build. CPU is much slower and only needed without a usable GPU.")
				.addDropdown((d) => {
					for (const b of LINUX_BACKENDS) d.addOption(b, b);
					d.setValue(settings.linuxBackend).onChange(async (v) => {
						settings.linuxBackend = v as LinuxBackend;
						await this.plugin.saveSettings();
						void this.refreshStatus();
					});
				});
		}

		new Setting(containerEl)
			.setName("Sidecar path override")
			.setDesc("Absolute path to a sidecar binary, for development. Leave empty to use the downloaded one.")
			.addText((t) =>
				t.setValue(settings.sidecarPath).onChange(async (v) => {
					settings.sidecarPath = v.trim();
					await this.plugin.saveSettings();
					void this.refreshStatus();
				}),
			);

		new Setting(containerEl).setName("Models and engine").setHeading();
		this.statusEl = containerEl.createDiv();
		void this.refreshStatus();
	}

	hide(): void {
		this.statusEl = null;
	}

	private async refreshStatus(): Promise<void> {
		const el = this.statusEl;
		if (!el) return;
		let rows: { label: string; present: boolean; size: number }[];
		let error: string | null = null;
		try {
			const plan = await this.plugin.assets.plan();
			rows = plan.map((a) => ({ label: a.label, present: existsSync(a.dest), size: a.sizeBytes }));
		} catch (e) {
			rows = [];
			error = e instanceof Error ? e.message : String(e);
		}
		if (this.statusEl !== el) return;
		el.empty();
		if (error) el.createEl("p", { text: error });
		const downloading = this.plugin.assets.downloading;
		for (const r of rows) {
			const state = r.present ? "present" : downloading ? "downloading" : "missing";
			el.createDiv({ text: `${r.label}: ${state} (${formatBytes(r.size)})` });
		}
		if (this.progressText) el.createDiv({ text: this.progressText });
		const anyMissing = rows.some((r) => !r.present);
		const setting = new Setting(el)
			.setName("Download")
			.setDesc(
				`Files are saved to ${this.plugin.assets.dataDir()}, outside your vault. See the plugin README for sources.`,
			);
		if (downloading) {
			setting.addButton((b) => b.setButtonText("Cancel").onClick(() => this.plugin.assets.cancelDownload()));
		} else {
			setting.addButton((b) =>
				b
					.setButtonText(anyMissing ? "Download" : "Everything is downloaded")
					.setCta()
					.setDisabled(!anyMissing)
					.onClick(() => void this.startDownload()),
			);
		}
	}

	private async startDownload(): Promise<void> {
		const assets = this.plugin.assets;
		this.progressText = "";
		let lastRender = 0;
		const done = assets
			.downloadMissing((p) => {
				const pct = p.total ? ` ${Math.floor((p.received / p.total) * 100)}%` : "";
				this.progressText = `Downloading ${p.asset.label} (${p.index + 1}/${p.count})${pct}`;
				if (Date.now() - lastRender > 250) {
					lastRender = Date.now();
					void this.refreshStatus();
				}
			})
			.catch((e: unknown) => {
				this.progressText = `Download failed: ${e instanceof Error ? e.message : String(e)}`;
			});
		void this.refreshStatus();
		await done;
		if (!this.progressText.startsWith("Download failed")) this.progressText = "";
		await this.refreshStatus();
	}
}
