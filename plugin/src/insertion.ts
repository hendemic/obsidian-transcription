// Pure helpers that turn a transcript into note text.
import { findAudioEmbeds, normalizeLinkTarget } from "./embeds";

export interface EmbedLocator {
	/** Path as written in the clicked embed's link (any form; it is normalized). */
	path: string;
	/** 0-based line of the clicked embed, if it could be determined. */
	line?: number;
	/** Nth (0-based) embed of this path in the note, used when `line` is unusable. */
	occurrence?: number;
}

/**
 * Insert the paragraphs on the lines after the located embed, separated by blank lines.
 * Returns null when no matching embed exists in the note. Line endings are preserved.
 */
export function insertTranscript(
	content: string,
	locator: EmbedLocator,
	paragraphs: readonly string[],
): string | null {
	const paras = cleanParagraphs(paragraphs);
	const target = normalizeLinkTarget(locator.path);
	const matches = findAudioEmbeds(content).filter((m) => m.path === target);
	if (matches.length === 0) return null;

	let chosen = locator.line === undefined ? undefined : matches.find((m) => m.line === locator.line);
	if (!chosen && locator.occurrence !== undefined) chosen = matches[locator.occurrence];
	chosen ??= matches[0];
	if (!chosen) return null;
	return insertAfterLine(content, chosen.line, paras);
}

/** Append the transcript at the end of the note (fallback when the embed vanished). */
export function appendTranscript(content: string, paragraphs: readonly string[]): string {
	const paras = cleanParagraphs(paragraphs);
	if (paras.length === 0) return content;
	const eol = content.includes("\r\n") ? "\r\n" : "\n";
	const base = content.replace(/(\r?\n)*$/, "");
	const prefix = base.length > 0 ? base + eol + eol : "";
	return prefix + paras.join(eol + eol) + eol;
}

function cleanParagraphs(paragraphs: readonly string[]): string[] {
	return paragraphs.map((p) => p.trim()).filter((p) => p.length > 0);
}

function insertAfterLine(content: string, lineIndex: number, paras: string[]): string {
	// Alternating [line, separator, line, separator, ..., lastLine].
	const parts = content.split(/(\r\n|\n)/);
	const lineCount = Math.ceil(parts.length / 2);
	const fallbackEol = content.includes("\r\n") ? "\r\n" : "\n";
	const sepIdx = lineIndex * 2 + 1;
	const eol = parts[sepIdx] || fallbackEol;
	const next = lineIndex + 1 < lineCount ? parts[(lineIndex + 1) * 2] : undefined;
	const needsTrailingBlank = next !== undefined && next.trim() !== "";

	if (paras.length === 0) return content;
	const block = eol + eol + paras.join(eol + eol) + eol + (needsTrailingBlank ? eol : "");
	const before = parts.slice(0, lineIndex * 2 + 1).join("");
	const after = parts.slice(sepIdx + 1).join("");
	return before + block + after;
}

/** Content for a new transcript note: the audio embed followed by the transcript. */
export function buildTranscriptNote(embedLink: string, paragraphs: readonly string[]): string {
	return `${embedLink}\n\n${cleanParagraphs(paragraphs).join("\n\n")}\n`;
}

/** `<folder>/<basename> transcript.md`, or with ` 2`, ` 3`, ... if that path is taken. */
export function uniqueTranscriptPath(
	folderPath: string,
	basename: string,
	exists: (path: string) => boolean,
): string {
	const dir = folderPath === "" || folderPath === "/" ? "" : `${folderPath}/`;
	let candidate = `${dir}${basename} transcript.md`;
	for (let n = 2; exists(candidate); n++) candidate = `${dir}${basename} transcript ${n}.md`;
	return candidate;
}
