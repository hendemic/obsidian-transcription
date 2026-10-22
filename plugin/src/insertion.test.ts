// Transcript insertion into note text (docs/plan.md, "Plugin" > "Insertion").
// Spec: paragraphs separated by blank lines, plain text, on the lines after the clicked embed; fall back to
// the first occurrence when the clicked embed can't be mapped; from the file explorer, a new
// `<audio name> transcript.md` in the same folder with the embed followed by the transcript.
//
// Deliberately not covered:
// - `vault.process` / `vault.create` wiring in flow.ts: needs a running Obsidian.
// - Lone "\r" (classic Mac) line endings: Obsidian normalizes them away.
import { describe, expect, it } from "vitest";
import { appendTranscript, buildTranscriptNote, insertTranscript, uniqueTranscriptPath } from "./insertion";

const P = ["First paragraph.", "Second paragraph."];

describe("insertTranscript", () => {
	it("inserts after an embed on the first line, followed by a blank line before existing text", () => {
		expect(insertTranscript("![[a.webm]]\nNext line", { path: "a.webm" }, P)).toBe(
			"![[a.webm]]\n\nFirst paragraph.\n\nSecond paragraph.\n\nNext line",
		);
	});

	it("inserts after an embed in the middle of the note", () => {
		const note = "# Day\n\n![[a.webm]]\n\nLater thoughts.\n";
		expect(insertTranscript(note, { path: "a.webm" }, P)).toBe(
			"# Day\n\n![[a.webm]]\n\nFirst paragraph.\n\nSecond paragraph.\n\nLater thoughts.\n",
		);
	});

	it("inserts after an embed on the last line that ends with a newline", () => {
		expect(insertTranscript("Intro\n![[a.webm]]\n", { path: "a.webm" }, P)).toBe(
			"Intro\n![[a.webm]]\n\nFirst paragraph.\n\nSecond paragraph.\n",
		);
	});

	it("inserts after an embed on the last line without a trailing newline", () => {
		expect(insertTranscript("Intro\n![[a.webm]]", { path: "a.webm" }, P)).toBe(
			"Intro\n![[a.webm]]\n\nFirst paragraph.\n\nSecond paragraph.\n",
		);
	});

	it("handles a note that is only the embed", () => {
		expect(insertTranscript("![[a.webm]]", { path: "a.webm" }, ["Hi."])).toBe("![[a.webm]]\n\nHi.\n");
	});

	it("keeps exactly one blank line when there is already a blank line below the embed", () => {
		expect(insertTranscript("![[a.webm]]\n\nAfter", { path: "a.webm" }, ["T."])).toBe(
			"![[a.webm]]\n\nT.\n\nAfter",
		);
	});

	it("separates existing text directly below the embed with a blank line", () => {
		const out = insertTranscript("![[a.webm]]\nDirectly below", { path: "a.webm" }, ["T."]);
		expect(out).toBe("![[a.webm]]\n\nT.\n\nDirectly below");
	});

	it("does not touch other lines of the note", () => {
		const note = "line 0\n![[a.webm]]\nline 2\nline 3";
		const out = insertTranscript(note, { path: "a.webm" }, ["T."]) ?? "";
		expect(out.startsWith("line 0\n![[a.webm]]\n")).toBe(true);
		expect(out.endsWith("line 2\nline 3")).toBe(true);
	});

	it("inserts after the whole line when the embed shares it with text", () => {
		expect(insertTranscript("Morning ![[a.webm]] walk\nNext", { path: "a.webm" }, ["T."])).toBe(
			"Morning ![[a.webm]] walk\n\nT.\n\nNext",
		);
	});

	it("emits paragraphs as plain text, not as a callout or heading", () => {
		const out = insertTranscript("![[a.webm]]", { path: "a.webm" }, ["Hello there."]) ?? "";
		expect(out).toBe("![[a.webm]]\n\nHello there.\n");
		expect(out).not.toMatch(/^(>|#)/m);
	});

	it("trims paragraphs and drops empty and whitespace-only ones", () => {
		expect(insertTranscript("![[a.webm]]", { path: "a.webm" }, ["  A.  ", "", "   ", "\tB.\n"])).toBe(
			"![[a.webm]]\n\nA.\n\nB.\n",
		);
	});

	it("leaves the note unchanged when every paragraph is empty", () => {
		const note = "![[a.webm]]\nText";
		expect(insertTranscript(note, { path: "a.webm" }, [])).toBe(note);
		expect(insertTranscript(note, { path: "a.webm" }, ["  ", ""])).toBe(note);
	});

	it("does not interpret $ patterns or markdown in the transcript", () => {
		const paras = ["Cost was $& and $1 and $$.", "*not* a list"];
		expect(insertTranscript("![[a.webm]]", { path: "a.webm" }, paras)).toBe(
			"![[a.webm]]\n\nCost was $& and $1 and $$.\n\n*not* a list\n",
		);
	});

	it("returns null when the embed is not in the note", () => {
		expect(insertTranscript("![[b.webm]]\n[[a.webm]]", { path: "a.webm" }, P)).toBeNull();
		expect(insertTranscript("", { path: "a.webm" }, P)).toBeNull();
	});

	it("returns null when the only match is inside a code fence", () => {
		expect(insertTranscript("```\n![[a.webm]]\n```", { path: "a.webm" }, P)).toBeNull();
	});

	it("matches the clicked path regardless of alias/heading in either the locator or the note", () => {
		expect(insertTranscript("![[a.webm|my alias]]", { path: "a.webm#t=1" }, ["T."])).toBe(
			"![[a.webm|my alias]]\n\nT.\n",
		);
	});

	it("does not match a different file whose name contains the target", () => {
		expect(insertTranscript("![[old a.webm]]", { path: "a.webm" }, P)).toBeNull();
		expect(insertTranscript("![[folder/a.webm]]", { path: "a.webm" }, P)).toBeNull();
	});

	it("finds a percent-encoded markdown embed from a decoded locator path", () => {
		expect(insertTranscript("![](my%20rec.m4a)", { path: "my rec.m4a" }, ["T."])).toBe("![](my%20rec.m4a)\n\nT.\n");
	});

	describe("duplicate embeds", () => {
		const note = "![[a.webm]]\nfirst\n![[a.webm]]\nsecond\n![[a.webm]]\nthird";

		it("uses the first occurrence without a line or occurrence", () => {
			expect(insertTranscript(note, { path: "a.webm" }, ["T."])).toBe(
				"![[a.webm]]\n\nT.\n\nfirst\n![[a.webm]]\nsecond\n![[a.webm]]\nthird",
			);
		});

		it("uses the line locator when it points at a matching embed", () => {
			expect(insertTranscript(note, { path: "a.webm", line: 4 }, ["T."])).toBe(
				"![[a.webm]]\nfirst\n![[a.webm]]\nsecond\n![[a.webm]]\n\nT.\n\nthird",
			);
		});

		it("prefers the line over a conflicting occurrence", () => {
			expect(insertTranscript(note, { path: "a.webm", line: 2, occurrence: 0 }, ["T."])).toBe(
				"![[a.webm]]\nfirst\n![[a.webm]]\n\nT.\n\nsecond\n![[a.webm]]\nthird",
			);
		});

		it("uses the occurrence when no line is given", () => {
			expect(insertTranscript(note, { path: "a.webm", occurrence: 1 }, ["T."])).toBe(
				"![[a.webm]]\nfirst\n![[a.webm]]\n\nT.\n\nsecond\n![[a.webm]]\nthird",
			);
		});

		it("falls back to the occurrence when the line does not hold the embed", () => {
			expect(insertTranscript(note, { path: "a.webm", line: 1, occurrence: 2 }, ["T."])).toBe(
				"![[a.webm]]\nfirst\n![[a.webm]]\nsecond\n![[a.webm]]\n\nT.\n\nthird",
			);
		});

		it("falls back to the first occurrence when the line is out of range", () => {
			const out = insertTranscript(note, { path: "a.webm", line: 999 }, ["T."]);
			expect(out?.startsWith("![[a.webm]]\n\nT.\n\nfirst")).toBe(true);
		});

		it("falls back to the first occurrence when the occurrence is out of range or negative", () => {
			for (const occurrence of [3, 99, -1]) {
				const out = insertTranscript(note, { path: "a.webm", occurrence }, ["T."]);
				expect(out?.startsWith("![[a.webm]]\n\nT.\n\nfirst")).toBe(true);
			}
		});

		it("does not count a fenced copy of the embed as an occurrence", () => {
			const fenced = "```\n![[a.webm]]\n```\n![[a.webm]]\nx\n![[a.webm]]";
			expect(insertTranscript(fenced, { path: "a.webm", occurrence: 1 }, ["T."])).toBe(
				"```\n![[a.webm]]\n```\n![[a.webm]]\nx\n![[a.webm]]\n\nT.\n",
			);
		});

		it("ignores a line locator that points at a fenced copy", () => {
			const fenced = "```\n![[a.webm]]\n```\n![[a.webm]]";
			expect(insertTranscript(fenced, { path: "a.webm", line: 1 }, ["T."])).toBe(
				"```\n![[a.webm]]\n```\n![[a.webm]]\n\nT.\n",
			);
		});

		it("handles two copies on the same line", () => {
			expect(insertTranscript("![[a.webm]] ![[a.webm]]\nx", { path: "a.webm", occurrence: 1 }, ["T."])).toBe(
				"![[a.webm]] ![[a.webm]]\n\nT.\n\nx",
			);
		});

		it("ignores other files' embeds when counting occurrences", () => {
			const mixed = "![[b.webm]]\n![[a.webm]]\n![[b.webm]]\n![[a.webm]]";
			expect(insertTranscript(mixed, { path: "a.webm", occurrence: 1 }, ["T."])).toBe(
				"![[b.webm]]\n![[a.webm]]\n![[b.webm]]\n![[a.webm]]\n\nT.\n",
			);
		});
	});

	describe("line endings", () => {
		it("preserves CRLF line endings throughout", () => {
			expect(insertTranscript("# T\r\n![[a.webm]]\r\nNext\r\n", { path: "a.webm" }, P)).toBe(
				"# T\r\n![[a.webm]]\r\n\r\nFirst paragraph.\r\n\r\nSecond paragraph.\r\n\r\nNext\r\n",
			);
		});

		it("uses CRLF for a last-line embed without a trailing newline in a CRLF note", () => {
			expect(insertTranscript("# T\r\n![[a.webm]]", { path: "a.webm" }, ["T."])).toBe(
				"# T\r\n![[a.webm]]\r\n\r\nT.\r\n",
			);
		});

		it("never introduces a bare LF into a CRLF note", () => {
			const out = insertTranscript("x\r\n![[a.webm]]\r\n\r\ny", { path: "a.webm" }, P) ?? "";
			expect(out.replace(/\r\n/g, "")).not.toContain("\n");
		});

		it("does not insert CR into an LF note", () => {
			const out = insertTranscript("x\n![[a.webm]]\ny", { path: "a.webm" }, P) ?? "";
			expect(out).not.toContain("\r");
		});

		it("keeps a paragraph that contains a newline intact", () => {
			// The sidecar's PauseGap joins segments with a single space, so paragraphs contain no newlines.
			// Here we only check that insertion does not crash and keeps the text.
			const out = insertTranscript("![[a.webm]]", { path: "a.webm" }, ["a\nb"]) ?? "";
			expect(out).toContain("a\nb");
		});
	});
});

describe("appendTranscript", () => {
	it("appends after a blank line at the end of the note", () => {
		expect(appendTranscript("Note text", P)).toBe("Note text\n\nFirst paragraph.\n\nSecond paragraph.\n");
	});

	it("collapses trailing newlines before appending", () => {
		expect(appendTranscript("Note text\n\n\n", ["T."])).toBe("Note text\n\nT.\n");
	});

	it("writes only the transcript into an empty note", () => {
		expect(appendTranscript("", ["T."])).toBe("T.\n");
		expect(appendTranscript("\n\n", ["T."])).toBe("T.\n");
	});

	it("preserves CRLF line endings", () => {
		expect(appendTranscript("a\r\nb\r\n", P)).toBe("a\r\nb\r\n\r\nFirst paragraph.\r\n\r\nSecond paragraph.\r\n");
	});

	it("trims and drops empty paragraphs", () => {
		expect(appendTranscript("x", [" A ", "", "  "])).toBe("x\n\nA\n");
	});

	// Spec deviation (minor): insertTranscript leaves the note unchanged when there is nothing to insert, but
	// appendTranscript still adds blank lines. flow.ts only guards with `paragraphs.every(p => p.trim() === "")`,
	// so today this is unreachable from the UI; the inconsistency is still a trap for callers.
	it.fails("leaves the note unchanged when there is nothing to append", () => {
		expect(appendTranscript("Note text", [])).toBe("Note text");
		expect(appendTranscript("Note text", ["  "])).toBe("Note text");
	});
});

describe("buildTranscriptNote", () => {
	it("puts the embed first, then a blank line, then paragraphs separated by blank lines", () => {
		expect(buildTranscriptNote("![[rec.webm]]", P)).toBe(
			"![[rec.webm]]\n\nFirst paragraph.\n\nSecond paragraph.\n",
		);
	});

	it("trims and drops empty paragraphs", () => {
		expect(buildTranscriptNote("![[rec.webm]]", ["", " A ", "\n"])).toBe("![[rec.webm]]\n\nA\n");
	});

	it("keeps a markdown-style embed link verbatim", () => {
		expect(buildTranscriptNote("![](my%20rec.m4a)", ["T."])).toBe("![](my%20rec.m4a)\n\nT.\n");
	});

	it("produces a note in which the embed is findable and insertion-compatible", () => {
		const note = buildTranscriptNote("![[Audio/rec 1.webm]]", ["T."]);
		expect(insertTranscript(note, { path: "Audio/rec 1.webm" }, ["U."])).toBe(
			"![[Audio/rec 1.webm]]\n\nU.\n\nT.\n",
		);
	});
});

describe("uniqueTranscriptPath", () => {
	const none = (): boolean => false;

	it("names the note `<basename> transcript.md` in the audio's folder", () => {
		expect(uniqueTranscriptPath("Journal/Audio", "Recording 1", none)).toBe("Journal/Audio/Recording 1 transcript.md");
	});

	it("uses the vault root for an empty folder path", () => {
		expect(uniqueTranscriptPath("", "rec", none)).toBe("rec transcript.md");
	});

	it("uses the vault root for '/' (Obsidian's root folder path)", () => {
		expect(uniqueTranscriptPath("/", "rec", none)).toBe("rec transcript.md");
	});

	it("appends 2, 3, ... until a free path is found", () => {
		const taken = new Set(["A/rec transcript.md", "A/rec transcript 2.md", "A/rec transcript 3.md"]);
		expect(uniqueTranscriptPath("A", "rec", (p) => taken.has(p))).toBe("A/rec transcript 4.md");
	});

	it("does not skip numbers that are free", () => {
		const taken = new Set(["rec transcript.md", "rec transcript 3.md"]);
		expect(uniqueTranscriptPath("", "rec", (p) => taken.has(p))).toBe("rec transcript 2.md");
	});

	it("checks the exact candidate paths it may return", () => {
		const asked: string[] = [];
		const result = uniqueTranscriptPath("F", "x", (p) => {
			asked.push(p);
			return asked.length < 2;
		});
		expect(asked).toEqual(["F/x transcript.md", "F/x transcript 2.md"]);
		expect(result).toBe("F/x transcript 2.md");
	});

	it("keeps unicode and dots in the basename", () => {
		expect(uniqueTranscriptPath("", "録音 v1.2", none)).toBe("録音 v1.2 transcript.md");
	});
});
