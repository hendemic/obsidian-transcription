// Settings defaults and merging of stored data (docs/plan.md, "Plugin" > "Settings").
// Spec: language (auto by default), initial prompt, pause gap (seconds, default 1.8), Linux backend
// (vulkan / cpu), sidecar path override.
import { describe, expect, it } from "vitest";
import { DEFAULT_SETTINGS, LINUX_BACKENDS, mergeSettings } from "./settings-model";

describe("DEFAULT_SETTINGS", () => {
	it("matches the spec defaults", () => {
		expect(DEFAULT_SETTINGS).toEqual({
			language: "auto",
			initialPrompt: "",
			pauseGapSeconds: 1.8,
			linuxBackend: "vulkan",
			sidecarPath: "",
		});
	});

	it("offers exactly the Vulkan and CPU Linux backends", () => {
		expect([...LINUX_BACKENDS].sort()).toEqual(["cpu", "vulkan"]);
	});
});

describe("mergeSettings", () => {
	it.each([
		["undefined", undefined],
		["null", null],
		["a string", "settings"],
		["a number", 42],
		["a boolean", true],
		["an empty object", {}],
		["an array", ["auto", "x"]],
	])("returns the defaults for %s", (_n, stored) => {
		expect(mergeSettings(stored)).toEqual(DEFAULT_SETTINGS);
	});

	it("returns a fresh object, so mutating it does not change the defaults", () => {
		const s = mergeSettings(null);
		s.language = "de";
		s.pauseGapSeconds = 9;
		expect(DEFAULT_SETTINGS.language).toBe("auto");
		expect(DEFAULT_SETTINGS.pauseGapSeconds).toBe(1.8);
		expect(mergeSettings(null)).not.toBe(mergeSettings(null));
	});

	it("keeps valid stored values", () => {
		const stored = {
			language: "de",
			initialPrompt: "Anke, Jörg",
			pauseGapSeconds: 2.5,
			linuxBackend: "cpu",
			sidecarPath: "/opt/transcribe-cli",
		};
		expect(mergeSettings(stored)).toEqual(stored);
	});

	it("fills missing fields from the defaults (partial data from an older version)", () => {
		expect(mergeSettings({ language: "fr" })).toEqual({ ...DEFAULT_SETTINGS, language: "fr" });
	});

	it("drops unknown keys", () => {
		expect(mergeSettings({ obsolete: 1, language: "en" })).toEqual({ ...DEFAULT_SETTINGS, language: "en" });
	});

	it("discards values of the wrong type field by field", () => {
		expect(
			mergeSettings({
				language: 5,
				initialPrompt: ["x"],
				pauseGapSeconds: "2",
				linuxBackend: true,
				sidecarPath: { p: 1 },
			}),
		).toEqual(DEFAULT_SETTINGS);
	});

	it("keeps good fields next to bad ones", () => {
		expect(mergeSettings({ language: null, pauseGapSeconds: 3 })).toEqual({ ...DEFAULT_SETTINGS, pauseGapSeconds: 3 });
	});

	describe("language", () => {
		it("falls back to auto for an empty or whitespace-only value", () => {
			expect(mergeSettings({ language: "" }).language).toBe("auto");
			expect(mergeSettings({ language: "  \t" }).language).toBe("auto");
		});

		it("trims the value", () => {
			expect(mergeSettings({ language: " en " }).language).toBe("en");
		});
	});

	describe("pause gap", () => {
		it.each([0, -1, Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY])(
			"falls back to the default for %s",
			(pauseGapSeconds) => {
				expect(mergeSettings({ pauseGapSeconds }).pauseGapSeconds).toBe(1.8);
			},
		);

		it("keeps small and fractional positive values", () => {
			expect(mergeSettings({ pauseGapSeconds: 0.25 }).pauseGapSeconds).toBe(0.25);
		});

		it("survives a JSON round trip (NaN and Infinity become null on save)", () => {
			const saved = JSON.parse(JSON.stringify({ pauseGapSeconds: Number.NaN })) as unknown;
			expect(mergeSettings(saved).pauseGapSeconds).toBe(1.8);
		});
	});

	describe("Linux backend", () => {
		it.each(LINUX_BACKENDS)("keeps %s", (linuxBackend) => {
			expect(mergeSettings({ linuxBackend }).linuxBackend).toBe(linuxBackend);
		});

		it.each(["CUDA", "rocm", "metal", "", " cuda"])("rejects %j", (linuxBackend) => {
			expect(mergeSettings({ linuxBackend }).linuxBackend).toBe("vulkan");
		});

		it.each(["auto", "cuda"])("moves the removed %j setting to Vulkan", (linuxBackend) => {
			expect(mergeSettings({ linuxBackend }).linuxBackend).toBe("vulkan");
		});
	});

	describe("prompt and sidecar path", () => {
		it("keeps the prompt verbatim, including surrounding whitespace and newlines", () => {
			expect(mergeSettings({ initialPrompt: " Names:\nAnke " }).initialPrompt).toBe(" Names:\nAnke ");
		});

		it("trims the sidecar path, so a whitespace-only path means no override", () => {
			expect(mergeSettings({ sidecarPath: "  /opt/x  " }).sidecarPath).toBe("/opt/x");
			expect(mergeSettings({ sidecarPath: "   " }).sidecarPath).toBe("");
		});

		it("keeps spaces inside the sidecar path", () => {
			expect(mergeSettings({ sidecarPath: "/Users/me/My Tools/transcribe-cli" }).sidecarPath).toBe(
				"/Users/me/My Tools/transcribe-cli",
			);
		});
	});

	it("ignores values inherited through the prototype", () => {
		const proto = { language: "xx", linuxBackend: "cpu" };
		const stored = Object.create(proto) as object;
		const merged = mergeSettings(JSON.parse(JSON.stringify(stored)));
		expect(merged).toEqual(DEFAULT_SETTINGS);
	});

	it("is not polluted by a __proto__ key in saved JSON", () => {
		const stored = JSON.parse('{"__proto__":{"language":"xx"},"pauseGapSeconds":2}') as unknown;
		expect(mergeSettings(stored)).toEqual({ ...DEFAULT_SETTINGS, pauseGapSeconds: 2 });
		expect(({} as Record<string, unknown>).language).toBeUndefined();
	});
});
