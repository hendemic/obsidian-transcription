// Minimal mock of the `obsidian` module for vitest. Extend as tests need it.
export class Plugin {}
export class PluginSettingTab {
	containerEl = { empty(): void {} };
	constructor(
		public app: unknown,
		public plugin: unknown,
	) {}
}
export class Notice {
	messageEl = {};
	constructor(public message: string) {}
	hide(): void {}
}
export class Menu {
	addItem(): this {
		return this;
	}
	showAtMouseEvent(): void {}
}
export class Modal {
	contentEl = {};
	constructor(public app: unknown) {}
	open(): void {}
	close(): void {}
}
export class Setting {
	constructor(public containerEl: unknown) {}
}
export class TFile {
	path = "";
	basename = "";
	parent: { path: string } | null = null;
}
export class MarkdownView {}
export const Platform = { isLinux: false, isMacOS: false, isDesktop: true };
