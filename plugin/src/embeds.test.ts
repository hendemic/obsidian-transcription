// Embed detection from rendered DOM elements and from note text (docs/plan.md, "Plugin" and "Testing").
//
// Deliberately not covered:
// - Real DOM / Live Preview rendering: `main.ts` needs a running Obsidian; we test the pure helpers it calls.
// - Embeds inside HTML comments, `%% comments %%` and indented (4-space) code blocks: rare in journal notes.
// - Markdown embeds with a title (`![](a.m4a "t")`): Obsidian's handling is unclear, so no expectation is set.
import { describe, expect, it } from "vitest";
import {
	AUDIO_EXTENSIONS,
	audioEmbedSrc,
	findAudioEmbeds,
	isAudioPath,
	normalizeLinkTarget,
	occurrenceIndex,
	parseEmbedsInLine,
	type ElementLike,
} from "./embeds";

/** A fake rendered `.internal-embed` element. */
function el(src: string | null): ElementLike {
	return { getAttribute: (name: string) => (name === "src" ? src : null) };
}

describe("isAudioPath", () => {
	it.each(["m4a", "mp3", "wav", "webm", "ogg", "flac", "3gp"])("accepts Obsidian's audio extension .%s", (ext) => {
		expect(isAudioPath(`rec.${ext}`)).toBe(true);
	});

	it("accepts every listed extension in upper case", () => {
		for (const ext of AUDIO_EXTENSIONS) expect(isAudioPath(`REC.${ext.toUpperCase()}`)).toBe(true);
	});

	it.each(["a.png", "a.md", "a.mp4", "a.pdf", "a.webm.md", "a.wav.png"])("rejects non-audio path %s", (p) => {
		expect(isAudioPath(p)).toBe(false);
	});

	it("rejects a path without an extension", () => {
		expect(isAudioPath("webm")).toBe(false);
		expect(isAudioPath("")).toBe(false);
	});

	it("rejects a path whose only dot is in a folder name", () => {
		expect(isAudioPath("audio.webm/notes")).toBe(false);
	});

	it("rejects a near-miss extension that merely starts or ends like an audio one", () => {
		expect(isAudioPath("a.webmx")).toBe(false);
		expect(isAudioPath("a.xwav")).toBe(false);
	});

	it("ignores an alias and a heading subpath", () => {
		expect(isAudioPath("a.webm|300")).toBe(true);
		expect(isAudioPath("a.webm#t=10")).toBe(true);
	});
});

describe("normalizeLinkTarget", () => {
	it("drops alias, size, heading and block subpaths", () => {
		expect(normalizeLinkTarget("a.webm|my alias")).toBe("a.webm");
		expect(normalizeLinkTarget("a.webm|100x200")).toBe("a.webm");
		expect(normalizeLinkTarget("a.webm#Heading")).toBe("a.webm");
		expect(normalizeLinkTarget("a.webm#^block|x")).toBe("a.webm");
	});

	it("trims surrounding whitespace but keeps inner spaces", () => {
		expect(normalizeLinkTarget("  my folder/rec 1.webm  ")).toBe("my folder/rec 1.webm");
		expect(normalizeLinkTarget("rec.webm | alias")).toBe("rec.webm");
	});
});

describe("audioEmbedSrc (rendered .internal-embed elements)", () => {
	it("returns the src of an audio embed", () => {
		expect(audioEmbedSrc(el("Recording 20240101.webm"))).toBe("Recording 20240101.webm");
	});

	it("keeps folders in the path", () => {
		expect(audioEmbedSrc(el("Journal/Audio/rec.m4a"))).toBe("Journal/Audio/rec.m4a");
	});

	it("recognises an upper-case extension", () => {
		expect(audioEmbedSrc(el("REC.WEBM"))).toBe("REC.WEBM");
		expect(audioEmbedSrc(el("rec.Mp3"))).toBe("rec.Mp3");
	});

	it("returns null for a non-audio embed", () => {
		expect(audioEmbedSrc(el("photo.png"))).toBeNull();
		expect(audioEmbedSrc(el("Other note"))).toBeNull();
		expect(audioEmbedSrc(el("Other note#Heading"))).toBeNull();
	});

	it("returns null when src is missing or empty", () => {
		expect(audioEmbedSrc(el(null))).toBeNull();
		expect(audioEmbedSrc(el(""))).toBeNull();
		expect(audioEmbedSrc(el("   "))).toBeNull();
	});

	it("strips a subpath or alias carried in src", () => {
		expect(audioEmbedSrc(el("rec.webm#t=5"))).toBe("rec.webm");
		expect(audioEmbedSrc(el("rec.webm|alias"))).toBe("rec.webm");
	});

	it("decodes a percent-encoded src (markdown-style embeds)", () => {
		expect(audioEmbedSrc(el("my%20rec.m4a"))).toBe("my rec.m4a");
	});

	it("does not throw on malformed percent-encoding", () => {
		expect(() => audioEmbedSrc(el("bad%E0%A4%A.webm"))).not.toThrow();
		expect(audioEmbedSrc(el("bad%E0%A4%A.webm"))).toBe("bad%E0%A4%A.webm");
	});

	it("handles unicode and emoji file names", () => {
		expect(audioEmbedSrc(el("日記/録音 🎙️.webm"))).toBe("日記/録音 🎙️.webm");
	});
});

describe("parseEmbedsInLine", () => {
	it("returns wikilink and markdown embeds in the order they appear", () => {
		const line = "![](b.mp3) then ![[a.webm]] and ![x](c.wav)";
		expect(parseEmbedsInLine(line).map((e) => e.path)).toEqual(["b.mp3", "a.webm", "c.wav"]);
	});

	it("keeps the raw matched text", () => {
		expect(parseEmbedsInLine("say ![[a.webm|x]] hi")).toEqual([{ path: "a.webm", raw: "![[a.webm|x]]" }]);
	});

	it("unwraps angle-bracketed markdown targets", () => {
		expect(parseEmbedsInLine("![](<my rec.m4a>)")).toEqual([{ path: "my rec.m4a", raw: "![](<my rec.m4a>)" }]);
	});

	it("returns nothing for plain text", () => {
		expect(parseEmbedsInLine("just [[a.webm]] and [a](a.webm)")).toEqual([]);
	});
});

describe("findAudioEmbeds (note text)", () => {
	const paths = (content: string): string[] => findAudioEmbeds(content).map((m) => m.path);

	it("finds a wikilink embed with its 0-based line", () => {
		expect(findAudioEmbeds("# Title\n\n![[a.webm]]\n")).toEqual([{ line: 2, path: "a.webm", raw: "![[a.webm]]" }]);
	});

	it("normalizes aliases, sizes and headings", () => {
		expect(paths("![[a.webm|my alias]]\n![[b.webm|300]]\n![[c.webm#Heading]]")).toEqual([
			"a.webm",
			"b.webm",
			"c.webm",
		]);
	});

	it("keeps folders and spaces", () => {
		expect(paths("![[Journal/2024/Recording 1.webm]]")).toEqual(["Journal/2024/Recording 1.webm"]);
	});

	it("recognises upper-case extensions in text", () => {
		expect(paths("![[A.M4A]]")).toEqual(["A.M4A"]);
	});

	it("decodes percent-encoded markdown embeds", () => {
		expect(paths("![](a%20b.m4a)")).toEqual(["a b.m4a"]);
		expect(paths("![alt text](Folder%20X/rec.mp3)")).toEqual(["Folder X/rec.mp3"]);
	});

	it("ignores links that are not embedded", () => {
		expect(paths("[[a.webm]]\n[a](a.webm)\n[[a.webm|alias]]")).toEqual([]);
	});

	it("ignores non-audio embeds", () => {
		expect(paths("![[photo.png]]\n![[Other note]]\n![](x.pdf)")).toEqual([]);
	});

	it("finds several embeds on one line and across lines, in document order", () => {
		const m = findAudioEmbeds("![[a.webm]] ![[b.webm]]\ntext\n![[a.webm]]");
		expect(m.map((x) => [x.line, x.path])).toEqual([
			[0, "a.webm"],
			[0, "b.webm"],
			[2, "a.webm"],
		]);
	});

	it("counts lines correctly with CRLF endings", () => {
		expect(findAudioEmbeds("a\r\nb\r\n![[x.webm]]\r\n")[0]?.line).toBe(2);
	});

	it("returns nothing for an empty note", () => {
		expect(findAudioEmbeds("")).toEqual([]);
	});

	it("does not match an embed broken across lines", () => {
		expect(paths("![[a.\nwebm]]")).toEqual([]);
	});

	it("does not match an empty wikilink embed", () => {
		expect(paths("![[]]")).toEqual([]);
	});

	describe("code", () => {
		it("skips embeds inside a backtick fence", () => {
			expect(paths("```\n![[a.webm]]\n```\n![[b.webm]]")).toEqual(["b.webm"]);
		});

		it("skips embeds inside a tilde fence", () => {
			expect(paths("~~~\n![[a.webm]]\n~~~\n![[b.webm]]")).toEqual(["b.webm"]);
		});

		it("skips embeds inside a fence with an info string", () => {
			expect(paths("```markdown\n![[a.webm]]\n```")).toEqual([]);
		});

		it("does not close a backtick fence with a tilde line", () => {
			expect(paths("```\n~~~\n![[a.webm]]\n```")).toEqual([]);
		});

		it("treats everything after an unclosed fence as code", () => {
			expect(paths("![[a.webm]]\n```\n![[b.webm]]")).toEqual(["a.webm"]);
		});

		it("reports the right line for an embed after a fence", () => {
			expect(findAudioEmbeds("```\nx\n```\n![[b.webm]]")[0]?.line).toBe(3);
		});

		it("treats a fence indented by four or more spaces as not a fence", () => {
			// A 4-space indent is an indented code line, not a fence opener, so the embed after it counts.
			expect(paths("    ```\n![[a.webm]]")).toEqual(["a.webm"]);
		});

		// BUG: findAudioEmbeds ignores inline code. Obsidian renders `![[a.webm]]` in backticks as literal
		// text, so it should not be found. Consequence: the occurrence index computed from the DOM (which
		// has no embed for it) is off by one against the note text, and the transcript goes under the
		// wrong embed.
		it.fails("skips an embed inside inline code", () => {
			expect(paths("Use `![[a.webm]]` to embed.\n![[a.webm]]")).toEqual(["a.webm"]);
			expect(findAudioEmbeds("Use `![[a.webm]]` to embed.\n![[a.webm]]")[0]?.line).toBe(1);
		});

		// BUG: a closing fence must use at least as many markers as the opener (CommonMark 4.5). A ``` line
		// inside a ```` fence closes it early, so the embed below (still in the code block) is detected.
		it.fails("does not close a four-backtick fence with a three-backtick line", () => {
			expect(paths("````\n```\n![[a.webm]]\n```\n````")).toEqual([]);
		});

		// BUG: a closing fence cannot carry an info string (CommonMark 4.5). Inside a ``` block, a line
		// "```js" is content, but findAudioEmbeds treats it as the close.
		it.fails("does not close a fence with a line that has an info string", () => {
			expect(paths("```\n```js\n![[a.webm]]\n```")).toEqual([]);
		});
	});

	// BUG: CommonMark allows balanced parentheses in a link destination, and duplicate files are often
	// named "rec (1).m4a". The markdown embed regex stops at the first ")".
	it.fails("finds a markdown embed whose file name contains parentheses", () => {
		expect(paths("![](rec%20(1).m4a)")).toEqual(["rec (1).m4a"]);
	});
});

describe("occurrenceIndex", () => {
	it("counts only earlier embeds with the same src", () => {
		const a1 = el("a.webm");
		const b = el("b.webm");
		const a2 = el("a.webm");
		const a3 = el("a.webm");
		const all = [a1, b, a2, a3];
		expect(occurrenceIndex(all, a1)).toBe(0);
		expect(occurrenceIndex(all, b)).toBe(0);
		expect(occurrenceIndex(all, a2)).toBe(1);
		expect(occurrenceIndex(all, a3)).toBe(2);
	});

	it("treats srcs that normalize to the same path as the same embed", () => {
		const a1 = el("a.webm|alias");
		const a2 = el("a.webm#t=3");
		expect(occurrenceIndex([a1, a2], a2)).toBe(1);
	});

	it("ignores non-audio embeds in between", () => {
		const a1 = el("a.webm");
		const img = el("a.png");
		const a2 = el("a.webm");
		expect(occurrenceIndex([a1, img, a2], a2)).toBe(1);
	});

	it("uses element identity, not equal src, to find the target", () => {
		const a1 = el("a.webm");
		const a2 = el("a.webm");
		expect(occurrenceIndex([a1, a2], a2)).toBe(1);
	});

	it("falls back to 0 when the target is not among the candidates", () => {
		expect(occurrenceIndex([el("a.webm"), el("a.webm")], el("a.webm"))).toBe(0);
	});
});
