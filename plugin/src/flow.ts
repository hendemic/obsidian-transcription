// The transcription flow: assets -> decode -> sidecar -> insert.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { App, Notice, TFile } from "obsidian";
import type { AssetManager } from "./assets/manager";
import { decodeTo16kMono } from "./decode";
import {
	appendTranscript,
	buildTranscriptNote,
	insertTranscript,
	uniqueTranscriptPath,
	type EmbedLocator,
} from "./insertion";
import { buildTranscribeArgs } from "./protocol";
import type { LocalWhisperSettings } from "./settings-model";
import { runSidecar, TranscriptionCancelled, type ChildLike } from "./sidecar-client";
import { createProgressNotice, ensureAssets } from "./ui";

export type TranscribeRequest =
	| { kind: "embed"; audio: TFile; notePath: string; locator: EmbedLocator }
	| { kind: "file"; audio: TFile };

export class TranscribeFlow {
	private controller: AbortController | null = null;

	constructor(
		private readonly app: App,
		private readonly assets: AssetManager,
		private readonly getSettings: () => LocalWhisperSettings,
	) {}

	get busy(): boolean {
		return this.controller !== null;
	}

	cancel(): void {
		this.controller?.abort();
	}

	async run(req: TranscribeRequest): Promise<void> {
		if (this.controller) {
			new Notice("A transcription is already running.");
			return;
		}
		const controller = new AbortController();
		this.controller = controller;
		let progress: ReturnType<typeof createProgressNotice> | null = null;
		try {
			if (!(await ensureAssets(this.app, this.assets))) return;
			const paths = await this.assets.paths();
			if (!existsSync(paths.sidecar)) {
				throw new Error(`Transcription engine not found at ${paths.sidecar}`);
			}

			progress = createProgressNotice("Decoding audio…", () => controller.abort());
			const pcm = await decodeTo16kMono(await this.app.vault.readBinary(req.audio));
			if (controller.signal.aborted) throw new TranscriptionCancelled();

			const settings = this.getSettings();
			const args = buildTranscribeArgs({
				modelPath: paths.model,
				vadModelPath: paths.vad,
				paragraphModelPath: paths.paragraphModel,
				paragraphTokenizerPath: paths.paragraphTokenizer,
				language: settings.language,
				prompt: settings.initialPrompt,
				pauseGapSeconds: settings.pauseGapSeconds,
			});
			const notice = progress;
			const paragraphs = await runSidecar({
				binary: paths.sidecar,
				args,
				pcm,
				signal: controller.signal,
				spawn: (cmd, a) => spawn(cmd, a, { stdio: ["pipe", "pipe", "pipe"] }) as unknown as ChildLike,
				onProgress: (p) =>
					notice.setText(
						p.stage === "loading" ? "Loading model…" : `Transcribing… ${Math.round(p.percent)}%`,
					),
			});
			if (paragraphs.every((p) => p.trim() === "")) {
				new Notice("No speech was found in the recording.");
				return;
			}
			await this.insert(req, paragraphs);
		} catch (e) {
			if (e instanceof TranscriptionCancelled) new Notice("Transcription cancelled.");
			else new Notice(`Transcription failed: ${e instanceof Error ? e.message : String(e)}`, 10000);
		} finally {
			progress?.hide();
			this.controller = null;
		}
	}

	private async insert(req: TranscribeRequest, paragraphs: string[]): Promise<void> {
		const { vault, fileManager, workspace } = this.app;
		if (req.kind === "embed") {
			const note = vault.getAbstractFileByPath(req.notePath);
			if (!(note instanceof TFile)) throw new Error("The note is no longer available.");
			let fellBack = false;
			await vault.process(note, (content) => {
				const updated = insertTranscript(content, req.locator, paragraphs);
				if (updated !== null) return updated;
				fellBack = true;
				return appendTranscript(content, paragraphs);
			});
			new Notice(
				fellBack
					? "Transcribed. The audio embed was not found, so the transcript was added at the end of the note."
					: "Transcription added to the note.",
			);
			return;
		}
		const path = uniqueTranscriptPath(req.audio.parent?.path ?? "", req.audio.basename, (p) =>
			vault.getAbstractFileByPath(p) !== null,
		);
		const embed = `!${fileManager.generateMarkdownLink(req.audio, path)}`;
		const created = await vault.create(path, buildTranscriptNote(embed, paragraphs));
		new Notice(`Transcript saved to ${created.path}`);
		await workspace.getLeaf("tab").openFile(created);
	}
}
