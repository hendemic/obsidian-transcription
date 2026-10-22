// Pure helpers for recognising audio embeds, from DOM elements and from note text.

/** Audio extensions Obsidian can play (plus opus/oga, which Chromium decodes). */
export const AUDIO_EXTENSIONS: readonly string[] = [
	"m4a",
	"mp3",
	"wav",
	"webm",
	"ogg",
	"oga",
	"opus",
	"flac",
	"3gp",
];

export function isAudioPath(path: string): boolean {
	const clean = normalizeLinkTarget(path);
	const dot = clean.lastIndexOf(".");
	if (dot < 0 || clean.slice(dot).includes("/")) return false;
	return AUDIO_EXTENSIONS.includes(clean.slice(dot + 1).toLowerCase());
}

/**
 * Reduce a link target to the bare path: drops `|alias/size`, and `#heading`/`#^block` subpaths.
 */
export function normalizeLinkTarget(target: string): string {
	let t = target.trim();
	const pipe = t.indexOf("|");
	if (pipe >= 0) t = t.slice(0, pipe);
	const hash = t.indexOf("#");
	if (hash >= 0) t = t.slice(0, hash);
	return t.trim();
}

export interface ElementLike {
	getAttribute(name: string): string | null;
}

/** The normalized `src` of a rendered `.internal-embed` element, or null if it is not an audio embed. */
export function audioEmbedSrc(el: ElementLike): string | null {
	const src = el.getAttribute("src");
	if (!src) return null;
	const path = normalizeLinkTarget(safeDecode(src));
	return isAudioPath(path) ? path : null;
}

function safeDecode(s: string): string {
	// Rendered src attributes are normally plain paths; decode only if it is percent-encoded.
	if (!/%[0-9a-fA-F]{2}/.test(s)) return s;
	try {
		return decodeURIComponent(s);
	} catch {
		return s;
	}
}

export interface EmbedMatch {
	/** 0-based line index within the note. */
	line: number;
	/** Normalized path as written in the link. */
	path: string;
	/** The matched link text, e.g. `![[a.webm|x]]`. */
	raw: string;
}

const WIKI_EMBED = /!\[\[([^\]\n]+?)\]\]/g;
const MD_EMBED = /!\[[^\]\n]*\]\(\s*(<[^>\n]+>|(?:[^()\n]|\([^()\n]*\))+?)\s*\)/g;

/** Blank out inline code spans (a backtick run closes on a run of equal length), keeping string length. */
function maskInlineCode(text: string): string {
	if (!text.includes("`")) return text;
	const runs = [...text.matchAll(/`+/g)];
	let out = text;
	for (let i = 0; i < runs.length; i++) {
		const open = runs[i];
		if (!open) continue;
		const close = runs.findIndex((r, j) => j > i && r[0].length === open[0].length);
		if (close < 0) continue;
		const end = runs[close];
		if (!end) continue;
		const from = open.index ?? 0;
		const to = (end.index ?? 0) + end[0].length;
		out = out.slice(0, from) + " ".repeat(to - from) + out.slice(to);
		i = close;
	}
	return out;
}

/** Audio-or-not embeds found in a single line of text (wikilink and markdown forms). */
export function parseEmbedsInLine(text: string): { path: string; raw: string }[] {
	const found: { path: string; raw: string; index: number }[] = [];
	for (const m of text.matchAll(WIKI_EMBED)) {
		found.push({ path: normalizeLinkTarget(m[1] ?? ""), raw: m[0], index: m.index ?? 0 });
	}
	for (const m of text.matchAll(MD_EMBED)) {
		let target = m[1] ?? "";
		if (target.startsWith("<") && target.endsWith(">")) target = target.slice(1, -1);
		found.push({
			path: normalizeLinkTarget(safeDecode(target)),
			raw: m[0],
			index: m.index ?? 0,
		});
	}
	return found.sort((a, b) => a.index - b.index).map(({ path, raw }) => ({ path, raw }));
}

/** All audio embeds in a note, in document order, skipping fenced code blocks. */
export function findAudioEmbeds(content: string): EmbedMatch[] {
	const out: EmbedMatch[] = [];
	const lines = content.split(/\r?\n/);
	let fence: { char: string; len: number } | null = null;
	lines.forEach((text, line) => {
		const fm = /^\s{0,3}(`{3,}|~{3,})(.*)$/.exec(text);
		if (fence !== null) {
			if (fm) {
				const marker = fm[1] ?? "";
				if (marker[0] === fence.char && marker.length >= fence.len && (fm[2] ?? "").trim() === "") {
					fence = null;
				}
			}
			return;
		}
		if (fm) {
			const marker = fm[1] ?? "";
			// A backtick fence's info string cannot contain backticks (that would be inline code).
			if (!(marker[0] === "`" && (fm[2] ?? "").includes("`"))) {
				fence = { char: marker[0] ?? "`", len: marker.length };
				return;
			}
		}
		for (const e of parseEmbedsInLine(maskInlineCode(text))) {
			if (isAudioPath(e.path)) out.push({ line, path: e.path, raw: e.raw });
		}
	});
	return out;
}

/** Index of `target` among `candidates` that share its src (used to map a DOM embed to the Nth link in the note). */
export function occurrenceIndex<T extends ElementLike>(candidates: readonly T[], target: T): number {
	const src = audioEmbedSrc(target);
	let n = 0;
	for (const c of candidates) {
		if (c === target) return n;
		if (audioEmbedSrc(c) === src) n++;
	}
	return 0;
}
