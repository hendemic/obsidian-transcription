import { App, PluginSettingTab } from "obsidian";
import type LocalWhisperTranscribePlugin from "./main";

export class LocalWhisperSettingTab extends PluginSettingTab {
	constructor(app: App, plugin: LocalWhisperTranscribePlugin) {
		super(app, plugin);
	}

	display(): void {
		this.containerEl.empty();
	}
}
