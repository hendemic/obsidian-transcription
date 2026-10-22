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
	constructor(public message: string) {}
}
export class Menu {}
