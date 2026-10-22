import { MarkdownView, Menu, Plugin, TFile } from "obsidian";
import { AssetManager } from "./assets/manager";
import { audioEmbedSrc, isAudioPath, occurrenceIndex } from "./embeds";
import { TranscribeFlow, type TranscribeRequest } from "./flow";
import type { EmbedLocator } from "./insertion";
import { LocalWhisperSettingTab } from "./settings";
import { DEFAULT_SETTINGS, mergeSettings, type LocalWhisperSettings } from "./settings-model";

/** The CodeMirror view behind an Editor; not part of the public API, so probed defensively. */
interface CmLike {
	posAtDOM(node: Node): number;
	state: { doc: { lineAt(pos: number): { number: number } } };
}

export default class LocalWhisperTranscribePlugin extends Plugin {
	settings: LocalWhisperSettings = { ...DEFAULT_SETTINGS };
	assets!: AssetManager;
	private flow!: TranscribeFlow;

	async onload(): Promise<void> {
		await this.loadSettings();
		this.assets = new AssetManager(this.manifest.version, () => this.settings);
		this.flow = new TranscribeFlow(this.app, this.assets, () => this.settings);
		this.addSettingTab(new LocalWhisperSettingTab(this.app, this));

		// Capture phase so we see the click before CodeMirror or the embed's own handlers.
		this.registerDomEvent(document, "contextmenu", (evt) => this.onContextMenu(evt), { capture: true });

		this.registerEvent(
			this.app.workspace.on("file-menu", (menu, file) => {
				if (file instanceof TFile && isAudioPath(file.path)) {
					menu.addItem((item) =>
						item
							.setTitle("Transcribe")
							.setIcon("audio-lines")
							.onClick(() => void this.flow.run({ kind: "file", audio: file })),
					);
				}
			}),
		);
	}

	onunload(): void {
		this.flow.cancel();
		this.assets.cancelDownload();
	}

	async loadSettings(): Promise<void> {
		this.settings = mergeSettings(await this.loadData());
	}

	async saveSettings(): Promise<void> {
		await this.saveData(this.settings);
	}

	private onContextMenu(evt: MouseEvent): void {
		const target = evt.target;
		if (!(target instanceof Element)) return;
		const embedEl = target.closest(".internal-embed");
		if (!embedEl) return;
		const src = audioEmbedSrc(embedEl);
		if (!src) return;

		const view = this.app.workspace.getActiveViewOfType(MarkdownView);
		const note = view?.file ?? this.app.workspace.getActiveFile();
		if (!note) return;
		const audio = this.app.metadataCache.getFirstLinkpathDest(src, note.path);
		if (!audio) return;

		const request: TranscribeRequest = {
			kind: "embed",
			audio,
			notePath: note.path,
			locator: this.locate(view, embedEl, src),
		};
		evt.preventDefault();
		evt.stopPropagation();
		const menu = new Menu();
		menu.addItem((item) =>
			item
				.setTitle("Transcribe")
				.setIcon("audio-lines")
				.onClick(() => void this.flow.run(request)),
		);
		menu.showAtMouseEvent(evt);
	}

	/** Best-effort mapping of a rendered embed to its place in the note. */
	private locate(view: MarkdownView | null, embedEl: Element, src: string): EmbedLocator {
		const locator: EmbedLocator = { path: src };
		if (!view) return locator;
		const cm = (view.editor as unknown as { cm?: CmLike }).cm;
		if (cm) {
			try {
				locator.line = cm.state.doc.lineAt(cm.posAtDOM(embedEl)).number - 1;
			} catch {
				// Not in the editor DOM (e.g. reading view); fall through to occurrence matching.
			}
		}
		const all = Array.from(view.containerEl.querySelectorAll(".internal-embed"));
		locator.occurrence = occurrenceIndex(all, embedEl);
		return locator;
	}
}
