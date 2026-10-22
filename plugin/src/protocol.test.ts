// Sidecar protocol: JSON-line parsing and argument building (docs/plan.md, "Sidecar protocol").
//
// Deliberately not covered:
// - Range checking of `percent` (spec: u8 0..=100). The parser passes the number through; the UI rounds it.
//   No expectation is set for out-of-range values because the spec does not say what the plugin must do.
import { describe, expect, it } from "vitest";
import { buildTranscribeArgs, LineBuffer, parseLine, ProtocolError } from "./protocol";

describe("parseLine", () => {
	it("parses the loading progress message", () => {
		expect(parseLine('{"type":"progress","stage":"loading"}')).toEqual({ type: "progress", stage: "loading" });
	});

	it("parses the transcribing progress message with its percent", () => {
		expect(parseLine('{"type":"progress","stage":"transcribing","percent":42}')).toEqual({
			type: "progress",
			stage: "transcribing",
			percent: 42,
		});
	});

	it("parses percent 0 and 100", () => {
		expect(parseLine('{"type":"progress","stage":"transcribing","percent":0}')).toMatchObject({ percent: 0 });
		expect(parseLine('{"type":"progress","stage":"transcribing","percent":100}')).toMatchObject({ percent: 100 });
	});

	it("parses a result with paragraphs", () => {
		expect(parseLine('{"type":"result","paragraphs":["One.","Two."]}')).toEqual({
			type: "result",
			paragraphs: ["One.", "Two."],
		});
	});

	it("parses a result with no paragraphs", () => {
		expect(parseLine('{"type":"result","paragraphs":[]}')).toEqual({ type: "result", paragraphs: [] });
	});

	it("keeps unicode and escaped characters in paragraphs", () => {
		expect(parseLine('{"type":"result","paragraphs":["Caf\\u00e9 \\"quoted\\" 日本語 🎙️"]}')).toEqual({
			type: "result",
			paragraphs: ['Café "quoted" 日本語 🎙️'],
		});
	});

	it("parses an error message", () => {
		expect(parseLine('{"type":"error","message":"model not found"}')).toEqual({
			type: "error",
			message: "model not found",
		});
	});

	it("parses an error with an empty message", () => {
		expect(parseLine('{"type":"error","message":""}')).toEqual({ type: "error", message: "" });
	});

	it("ignores extra fields and key order", () => {
		expect(parseLine('{"extra":1,"stage":"loading","type":"progress"}')).toEqual({
			type: "progress",
			stage: "loading",
		});
	});

	it("strips extra fields from the returned message", () => {
		expect(parseLine('{"type":"error","message":"x","secret":"y"}')).toEqual({ type: "error", message: "x" });
	});

	it("tolerates surrounding whitespace and a trailing CR", () => {
		expect(parseLine('  {"type":"progress","stage":"loading"}\r')).toEqual({ type: "progress", stage: "loading" });
	});

	it("returns null for blank and whitespace-only lines", () => {
		expect(parseLine("")).toBeNull();
		expect(parseLine("   \t\r")).toBeNull();
	});

	describe("malformed lines throw ProtocolError carrying the line", () => {
		const bad: [string, string][] = [
			["invalid JSON", "{not json"],
			["truncated JSON", '{"type":"result","paragraphs":["a"'],
			["two objects on one line", '{"type":"progress","stage":"loading"}{"type":"progress","stage":"loading"}'],
			["a JSON string", '"progress"'],
			["a JSON number", "42"],
			["JSON null", "null"],
			["a JSON array", '[{"type":"progress","stage":"loading"}]'],
			["unknown type", '{"type":"hello"}'],
			["missing type", '{"stage":"loading"}'],
			["type with wrong case", '{"type":"Progress","stage":"loading"}'],
			["non-string type", '{"type":1}'],
			["unknown progress stage", '{"type":"progress","stage":"decoding"}'],
			["progress without stage", '{"type":"progress"}'],
			["transcribing without percent", '{"type":"progress","stage":"transcribing"}'],
			["transcribing with string percent", '{"type":"progress","stage":"transcribing","percent":"42"}'],
			["transcribing with null percent", '{"type":"progress","stage":"transcribing","percent":null}'],
			["result without paragraphs", '{"type":"result"}'],
			["result with string paragraphs", '{"type":"result","paragraphs":"one"}'],
			["result with non-string paragraph", '{"type":"result","paragraphs":["a",1]}'],
			["result with null paragraph", '{"type":"result","paragraphs":[null]}'],
			["error without message", '{"type":"error"}'],
			["error with non-string message", '{"type":"error","message":{"code":1}}'],
			["plain diagnostic text", "whisper_init_from_file: loading model"],
		];

		it.each(bad)("%s", (_name, line) => {
			let thrown: unknown;
			try {
				parseLine(line);
			} catch (e) {
				thrown = e;
			}
			expect(thrown).toBeInstanceOf(ProtocolError);
			expect((thrown as ProtocolError).line).toBe(line);
		});
	});

	it("does not let a __proto__ key turn into a message", () => {
		expect(() => parseLine('{"__proto__":{"type":"error","message":"x"}}')).toThrow(ProtocolError);
	});
});

describe("LineBuffer", () => {
	const enc = (s: string): Uint8Array => new TextEncoder().encode(s);

	it("returns whole lines and keeps the unterminated rest", () => {
		const b = new LineBuffer();
		expect(b.push("a\nb\nc")).toEqual(["a", "b"]);
		expect(b.push("d\n")).toEqual(["cd"]);
		expect(b.flush()).toEqual([]);
	});

	it("reassembles a line split across many chunks", () => {
		const b = new LineBuffer();
		const line = '{"type":"progress","stage":"loading"}';
		const out: string[] = [];
		for (const ch of line + "\n") out.push(...b.push(ch));
		expect(out).toEqual([line]);
	});

	it("returns several lines from one chunk, including blank ones", () => {
		const b = new LineBuffer();
		expect(b.push("a\n\nb\n")).toEqual(["a", "", "b"]);
	});

	it("strips CR from CRLF lines, even when CR and LF arrive in different chunks", () => {
		const b = new LineBuffer();
		expect(b.push("a\r\nb\r")).toEqual(["a"]);
		expect(b.push("\nc\n")).toEqual(["b", "c"]);
	});

	it("reassembles a multi-byte UTF-8 character split across byte chunks", () => {
		const bytes = enc('{"type":"result","paragraphs":["é 🎙️"]}\n');
		// Split inside the 4-byte emoji: find its first byte (0xF0) and cut after it.
		const cut = bytes.indexOf(0xf0) + 1;
		const b = new LineBuffer();
		const out = [...b.push(bytes.slice(0, cut)), ...b.push(bytes.slice(cut))];
		expect(out).toEqual(['{"type":"result","paragraphs":["é 🎙️"]}']);
	});

	it("reassembles a character delivered one byte at a time", () => {
		const bytes = enc("日本\n");
		const b = new LineBuffer();
		const out: string[] = [];
		for (const byte of bytes) out.push(...b.push(new Uint8Array([byte])));
		expect(out).toEqual(["日本"]);
	});

	it("handles a mix of string and byte chunks", () => {
		const b = new LineBuffer();
		expect(b.push(enc("ab"))).toEqual([]);
		expect(b.push("c\nd")).toEqual(["abc"]);
		expect(b.push(enc("e\n"))).toEqual(["de"]);
	});

	it("flushes the trailing unterminated line once", () => {
		const b = new LineBuffer();
		b.push('{"type":"error","message":"x"}');
		expect(b.flush()).toEqual(['{"type":"error","message":"x"}']);
		expect(b.flush()).toEqual([]);
	});

	it("flushes a dangling partial UTF-8 sequence as a replacement character, not silently", () => {
		const b = new LineBuffer();
		b.push(new Uint8Array([0x61, 0xe6]));
		const rest = b.flush();
		expect(rest).toHaveLength(1);
		expect(rest[0]?.startsWith("a")).toBe(true);
		expect(rest[0]).toContain("�");
	});

	it("returns nothing for an empty chunk", () => {
		const b = new LineBuffer();
		expect(b.push("")).toEqual([]);
		expect(b.push(new Uint8Array())).toEqual([]);
		expect(b.flush()).toEqual([]);
	});

	it("feeds parseLine cleanly for a whole session split at arbitrary byte offsets", () => {
		const session =
			'{"type":"progress","stage":"loading"}\n' +
			'{"type":"progress","stage":"transcribing","percent":50}\r\n' +
			"\n" +
			'{"type":"result","paragraphs":["Grüße aus Köln 🎉"]}\n';
		const bytes = enc(session);
		for (let size = 1; size <= 7; size++) {
			const b = new LineBuffer();
			const msgs = [];
			for (let i = 0; i < bytes.length; i += size) {
				for (const line of b.push(bytes.slice(i, i + size))) {
					const m = parseLine(line);
					if (m) msgs.push(m);
				}
			}
			expect(b.flush()).toEqual([]);
			expect(msgs).toEqual([
				{ type: "progress", stage: "loading" },
				{ type: "progress", stage: "transcribing", percent: 50 },
				{ type: "result", paragraphs: ["Grüße aus Köln 🎉"] },
			]);
		}
	});
});

describe("buildTranscribeArgs", () => {
	it("passes only the subcommand and model when nothing else is set", () => {
		expect(buildTranscribeArgs({ modelPath: "/m/ggml.bin" })).toEqual(["transcribe", "--model", "/m/ggml.bin"]);
	});

	it("passes every flag when everything is set", () => {
		expect(
			buildTranscribeArgs({
				modelPath: "/m/model.bin",
				vadModelPath: "/m/vad.bin",
				language: "de",
				prompt: "Anke, Jörg",
				pauseGapSeconds: 2.5,
			}),
		).toEqual([
			"transcribe",
			"--model",
			"/m/model.bin",
			"--vad-model",
			"/m/vad.bin",
			"--language",
			"de",
			"--prompt",
			"Anke, Jörg",
			"--pause-gap-ms",
			"2500",
		]);
	});

	it("keeps paths with spaces as a single argument", () => {
		const args = buildTranscribeArgs({
			modelPath: "/Users/me/Library/Application Support/x/model.bin",
			vadModelPath: "/a b/vad.bin",
		});
		expect(args).toEqual([
			"transcribe",
			"--model",
			"/Users/me/Library/Application Support/x/model.bin",
			"--vad-model",
			"/a b/vad.bin",
		]);
	});

	it("omits the VAD flag for an empty path", () => {
		expect(buildTranscribeArgs({ modelPath: "m", vadModelPath: "" })).not.toContain("--vad-model");
	});

	describe("language", () => {
		it.each(["auto", "AUTO", "Auto", " auto ", "", "   "])("omits --language for %j", (language) => {
			expect(buildTranscribeArgs({ modelPath: "m", language })).not.toContain("--language");
		});

		it("trims the language code", () => {
			expect(buildTranscribeArgs({ modelPath: "m", language: " en " })).toEqual([
				"transcribe",
				"--model",
				"m",
				"--language",
				"en",
			]);
		});
	});

	describe("prompt", () => {
		it("passes a prompt with spaces and quotes as one argument, unescaped", () => {
			const prompt = `Names: "Zoë" O'Brien, Dr. Ngũgĩ; $HOME && rm -rf / \`x\``;
			const args = buildTranscribeArgs({ modelPath: "m", prompt });
			expect(args.slice(-2)).toEqual(["--prompt", prompt]);
		});

		it("trims the prompt and omits an empty or whitespace-only one", () => {
			expect(buildTranscribeArgs({ modelPath: "m", prompt: "  hi  " }).slice(-1)).toEqual(["hi"]);
			expect(buildTranscribeArgs({ modelPath: "m", prompt: "  \n " })).not.toContain("--prompt");
		});

		it("keeps internal newlines in a multi-line prompt", () => {
			expect(buildTranscribeArgs({ modelPath: "m", prompt: "a\nb" }).slice(-1)).toEqual(["a\nb"]);
		});

		// RISK (depends on the sidecar's clap config, which is not written yet): clap rejects an option value
		// that starts with "-" unless `allow_hyphen_values` is set, so `--prompt "-Anke"` or a prompt like
		// "--help" would make the sidecar fail or print help. Passing `--prompt=<text>` (or `--` handling)
		// would be robust. This test asserts the robust form.
		it.fails("passes a prompt that starts with a hyphen in a form an option parser cannot misread", () => {
			const args = buildTranscribeArgs({ modelPath: "m", prompt: "--help" });
			const i = args.indexOf("--prompt");
			const safe = args.includes("--prompt=--help") || (i >= 0 && !(args[i + 1] ?? "").startsWith("-"));
			expect(safe).toBe(true);
		});
	});

	describe("pause gap", () => {
		it("converts seconds to whole milliseconds", () => {
			const ms = (s: number): string | undefined => {
				const a = buildTranscribeArgs({ modelPath: "m", pauseGapSeconds: s });
				return a[a.indexOf("--pause-gap-ms") + 1];
			};
			expect(ms(1.8)).toBe("1800");
			expect(ms(1)).toBe("1000");
			expect(ms(0.1)).toBe("100");
			expect(ms(2.0004)).toBe("2000");
			expect(ms(2.0006)).toBe("2001");
			expect(ms(0.0015)).toBe("2");
			expect(ms(60)).toBe("60000");
		});

		it("avoids floating-point artefacts such as 1.1 * 1000 = 1100.0000000000002", () => {
			const a = buildTranscribeArgs({ modelPath: "m", pauseGapSeconds: 1.1 });
			expect(a.slice(-2)).toEqual(["--pause-gap-ms", "1100"]);
			const b = buildTranscribeArgs({ modelPath: "m", pauseGapSeconds: 0.29 });
			expect(b.slice(-2)).toEqual(["--pause-gap-ms", "290"]);
		});

		it.each([0, -1, Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY])(
			"omits the flag for an unusable value %s",
			(pauseGapSeconds) => {
				expect(buildTranscribeArgs({ modelPath: "m", pauseGapSeconds })).not.toContain("--pause-gap-ms");
			},
		);

		// BUG (minor): a positive gap below 0.5 ms rounds to "0", which is sent although the input was
		// considered valid (> 0). Either clamp to at least 1 or omit the flag.
		it.fails("never sends a zero gap for a positive setting", () => {
			const a = buildTranscribeArgs({ modelPath: "m", pauseGapSeconds: 0.0001 });
			expect(a[a.indexOf("--pause-gap-ms") + 1]).not.toBe("0");
		});

		// BUG (minor): very large values are formatted by String() in exponent notation ("1e+21"), which the
		// sidecar cannot parse as an integer. mergeSettings accepts any finite positive number.
		it.fails("always formats the gap as a plain integer", () => {
			const a = buildTranscribeArgs({ modelPath: "m", pauseGapSeconds: 1e20 });
			expect(a[a.indexOf("--pause-gap-ms") + 1]).toMatch(/^\d+$/);
		});
	});
});
