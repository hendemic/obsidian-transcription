import { Plugin } from "obsidian";
import { LocalWhisperSettingTab } from "./settings";

export default class LocalWhisperTranscribePlugin extends Plugin {
	onload(): void {
		this.addSettingTab(new LocalWhisperSettingTab(this.app, this));
	}
}
