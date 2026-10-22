// Asset planning: data directory, Linux backend auto-detection, release variants, missing assets
// (docs/plan.md, "Plugin" > "Asset manager").
//
// Deliberately not covered:
// - Running `nvidia-smi` (manager.ts, needs the real binary); resolveLinuxBackend gets its result as input.
// - Downloads, extraction and chmod (download.ts / manager.ts): network and filesystem side effects.
import { describe, expect, it } from "vitest";
import {
	COREML_ENCODER,
	MODEL,
	PARAGRAPH_MODEL,
	PARAGRAPH_TOKENIZER,
	PLUGIN_ID,
	SIDECAR_ASSET_NAMES,
	VAD_MODEL,
} from "./config";
import {
	buildAssetPlan,
	findAsset,
	isCudaUsable,
	missingAssets,
	parseSha256Sums,
	resolveDataDir,
	resolveLinuxBackend,
	sidecarVariant,
	type PlanInput,
} from "./plan";

describe("isCudaUsable", () => {
	const both = [
		"\tlibcudart.so.12 (libc6,x86-64) => /usr/local/cuda/lib64/libcudart.so.12",
		"\tlibcublas.so.12 (libc6,x86-64) => /usr/local/cuda/lib64/libcublas.so.12",
	].join("\n");

	it("is true with nvidia-smi and both CUDA 12 libs", () => {
		expect(isCudaUsable(true, both)).toBe(true);
	});

	it("is false without nvidia-smi even when the libs exist", () => {
		expect(isCudaUsable(false, both)).toBe(false);
	});

	it("is false when ldconfig output is unavailable", () => {
		expect(isCudaUsable(true, null)).toBe(false);
	});

	it("is false when libcublas is missing", () => {
		expect(isCudaUsable(true, "\tlibcudart.so.12 (libc6,x86-64) => /x/libcudart.so.12")).toBe(false);
	});

	it("is false when libcudart is missing", () => {
		expect(isCudaUsable(true, "\tlibcublas.so.12 (libc6,x86-64) => /x/libcublas.so.12")).toBe(false);
	});

	it("rejects other CUDA major versions", () => {
		const v11 = "\tlibcudart.so.11.0 (libc6,x86-64) => /x\n\tlibcublas.so.11 (libc6,x86-64) => /y";
		expect(isCudaUsable(true, v11)).toBe(false);
	});
});

describe("resolveLinuxBackend", () => {
	it("auto picks cuda when CUDA is usable", () => {
		expect(resolveLinuxBackend("auto", true)).toBe("cuda");
	});

	it("auto picks vulkan when CUDA is not usable", () => {
		expect(resolveLinuxBackend("auto", false)).toBe("vulkan");
	});

	it("auto never picks cpu", () => {
		expect(resolveLinuxBackend("auto", false)).not.toBe("cpu");
		expect(resolveLinuxBackend("auto", true)).not.toBe("cpu");
	});

	it.each([
		["cuda", false],
		["cuda", true],
		["vulkan", true],
		["vulkan", false],
		["cpu", true],
		["cpu", false],
	] as const)("explicit %s is kept regardless of nvidia-smi (%s)", (setting, nv) => {
		expect(resolveLinuxBackend(setting, nv)).toBe(setting);
	});
});

describe("resolveDataDir", () => {
	it("uses Application Support on macOS", () => {
		expect(resolveDataDir("darwin", {}, "/Users/me")).toBe(`/Users/me/Library/Application Support/${PLUGIN_ID}`);
	});

	it("ignores XDG_DATA_HOME on macOS", () => {
		expect(resolveDataDir("darwin", { XDG_DATA_HOME: "/xdg" }, "/Users/me")).toBe(
			"/Users/me/Library/Application Support/local-whisper-transcribe",
		);
	});

	it("uses XDG_DATA_HOME on Linux when set", () => {
		expect(resolveDataDir("linux", { XDG_DATA_HOME: "/data/me" }, "/home/me")).toBe(
			"/data/me/local-whisper-transcribe",
		);
	});

	it("defaults to ~/.local/share on Linux when XDG_DATA_HOME is unset", () => {
		expect(resolveDataDir("linux", {}, "/home/me")).toBe("/home/me/.local/share/local-whisper-transcribe");
	});

	it("defaults to ~/.local/share when XDG_DATA_HOME is explicitly undefined", () => {
		expect(resolveDataDir("linux", { XDG_DATA_HOME: undefined }, "/home/me")).toBe(
			"/home/me/.local/share/local-whisper-transcribe",
		);
	});

	it("defaults to ~/.local/share when XDG_DATA_HOME is empty (XDG spec)", () => {
		expect(resolveDataDir("linux", { XDG_DATA_HOME: "" }, "/home/me")).toBe(
			"/home/me/.local/share/local-whisper-transcribe",
		);
	});

	it("ignores a relative XDG_DATA_HOME (XDG spec: relative paths are invalid)", () => {
		expect(resolveDataDir("linux", { XDG_DATA_HOME: "data" }, "/home/me")).toBe(
			"/home/me/.local/share/local-whisper-transcribe",
		);
		expect(resolveDataDir("linux", { XDG_DATA_HOME: "~/data" }, "/home/me")).toBe(
			"/home/me/.local/share/local-whisper-transcribe",
		);
	});

	it("does not double slashes for trailing slashes in XDG_DATA_HOME or home", () => {
		expect(resolveDataDir("linux", { XDG_DATA_HOME: "/data/" }, "/home/me")).toBe("/data/local-whisper-transcribe");
		expect(resolveDataDir("linux", {}, "/home/me/")).toBe("/home/me/.local/share/local-whisper-transcribe");
		expect(resolveDataDir("darwin", {}, "/Users/me/")).toBe(
			"/Users/me/Library/Application Support/local-whisper-transcribe",
		);
	});

	it("keeps spaces in paths", () => {
		expect(resolveDataDir("linux", { XDG_DATA_HOME: "/my data" }, "/home/me")).toBe(
			"/my data/local-whisper-transcribe",
		);
	});

	it.each(["win32", "android", "freebsd", ""])("throws for unsupported platform %j", (p) => {
		expect(() => resolveDataDir(p, {}, "/home/me")).toThrow(/Unsupported platform/);
	});

	it("never places the data directory inside a vault-relative path", () => {
		for (const dir of [
			resolveDataDir("linux", {}, "/home/me"),
			resolveDataDir("linux", { XDG_DATA_HOME: "rel" }, "/home/me"),
			resolveDataDir("darwin", {}, "/Users/me"),
		]) {
			expect(dir.startsWith("/")).toBe(true);
		}
	});
});

describe("sidecarVariant", () => {
	it("maps macOS arm64 to the macos-arm64 build, whatever the backend", () => {
		expect(sidecarVariant("darwin", "arm64", "vulkan")).toBe("macos-arm64");
		expect(sidecarVariant("darwin", "arm64", "cuda")).toBe("macos-arm64");
	});

	it("has no build for Intel Macs", () => {
		expect(sidecarVariant("darwin", "x64", "vulkan")).toBeNull();
	});

	it("maps Linux x64 cuda and vulkan", () => {
		expect(sidecarVariant("linux", "x64", "cuda")).toBe("linux-x64-cuda");
		expect(sidecarVariant("linux", "x64", "vulkan")).toBe("linux-x64-vulkan");
	});

	it("maps Linux x64 cpu to the CPU build", () => {
		expect(sidecarVariant("linux", "x64", "cpu")).toBe("linux-x64-cpu");
	});

	it("has no CPU build for Intel Macs or Linux arm64", () => {
		expect(sidecarVariant("darwin", "x64", "cpu")).toBeNull();
		expect(sidecarVariant("linux", "arm64", "cpu")).toBeNull();
	});

	it("has no build for Linux arm64 or Windows", () => {
		expect(sidecarVariant("linux", "arm64", "vulkan")).toBeNull();
		expect(sidecarVariant("win32", "x64", "cuda")).toBeNull();
	});
});

describe("buildAssetPlan", () => {
	const linux: PlanInput = {
		platform: "linux",
		arch: "x64",
		backend: "vulkan",
		dataDir: "/d",
		pluginVersion: "0.1.0",
		sidecarOverridden: false,
	};
	const mac: PlanInput = { ...linux, platform: "darwin", arch: "arm64" };
	const ids = (input: PlanInput): string[] => buildAssetPlan(input).map((a) => a.id);

	it("plans model, VAD, paragraph model and sidecar on Linux, without the Core ML encoder", () => {
		expect(ids(linux).sort()).toEqual(["model", "paragraph-model", "paragraph-tokenizer", "sidecar", "vad"]);
	});

	it("plans model, VAD, paragraph model, Core ML encoder and sidecar on macOS", () => {
		expect(ids(mac).sort()).toEqual([
			"coreml",
			"model",
			"paragraph-model",
			"paragraph-tokenizer",
			"sidecar",
			"vad",
		]);
	});

	it("puts models under <dataDir>/models with pinned checksums", () => {
		const plan = buildAssetPlan(linux);
		expect(findAsset(plan, "model")).toMatchObject({
			dest: `/d/models/${MODEL.fileName}`,
			url: MODEL.url,
			sha256: MODEL.sha256,
		});
		expect(findAsset(plan, "vad")).toMatchObject({
			dest: `/d/models/${VAD_MODEL.fileName}`,
			sha256: VAD_MODEL.sha256,
		});
		expect(findAsset(plan, "paragraph-model")).toMatchObject({
			dest: `/d/models/${PARAGRAPH_MODEL.fileName}`,
			url: PARAGRAPH_MODEL.url,
			sha256: PARAGRAPH_MODEL.sha256,
		});
		expect(findAsset(plan, "paragraph-tokenizer")).toMatchObject({
			dest: `/d/models/${PARAGRAPH_TOKENIZER.fileName}`,
			sha256: PARAGRAPH_TOKENIZER.sha256,
		});
	});

	it("places the Core ML encoder next to the model, extracted from a zip", () => {
		const coreml = findAsset(buildAssetPlan(mac), "coreml");
		expect(coreml?.dest).toBe("/d/models/ggml-base-encoder.mlmodelc");
		expect(coreml?.archivePath).toBe(`/d/models/${COREML_ENCODER.fileName}`);
		// whisper.cpp looks for `<model stem>-encoder.mlmodelc` next to the model file.
		const model = findAsset(buildAssetPlan(mac), "model");
		expect(coreml?.dest).toBe(model?.dest.replace(/\.bin$/, "-encoder.mlmodelc"));
	});

	it("plans the sidecar for the resolved Linux backend", () => {
		expect(findAsset(buildAssetPlan({ ...linux, backend: "cuda" }), "sidecar")?.releaseAssetName).toBe(
			SIDECAR_ASSET_NAMES["linux-x64-cuda"],
		);
		expect(findAsset(buildAssetPlan(linux), "sidecar")?.releaseAssetName).toBe(
			SIDECAR_ASSET_NAMES["linux-x64-vulkan"],
		);
	});

	it("fetches the sidecar from the release tagged v<version>, executable, checksum from SHA256SUMS", () => {
		const s = findAsset(buildAssetPlan({ ...linux, pluginVersion: "1.2.3" }), "sidecar");
		expect(s?.url).toMatch(/\/releases\/download\/v1\.2\.3\/transcribe-cli-linux-x64-vulkan$/);
		expect(s?.executable).toBe(true);
		expect(s?.sha256).toBeNull();
	});

	it("versions the sidecar location so a plugin update fetches a matching build", () => {
		const a = findAsset(buildAssetPlan({ ...linux, pluginVersion: "0.1.0" }), "sidecar")?.dest;
		const b = findAsset(buildAssetPlan({ ...linux, pluginVersion: "0.2.0" }), "sidecar")?.dest;
		expect(a).not.toBe(b);
		expect(a?.startsWith("/d/")).toBe(true);
	});

	it("does not plan the sidecar when the path is overridden", () => {
		expect(ids({ ...linux, sidecarOverridden: true })).not.toContain("sidecar");
		expect(ids({ ...mac, sidecarOverridden: true })).not.toContain("sidecar");
	});

	it("still plans the models when the sidecar is overridden", () => {
		expect(ids({ ...linux, sidecarOverridden: true }).sort()).toEqual([
			"model",
			"paragraph-model",
			"paragraph-tokenizer",
			"vad",
		]);
	});

	it("allows the CPU backend when the sidecar is overridden", () => {
		expect(() => buildAssetPlan({ ...linux, backend: "cpu", sidecarOverridden: true })).not.toThrow();
	});

	it("plans the downloadable CPU sidecar for the cpu backend on Linux x64", () => {
		const s = findAsset(buildAssetPlan({ ...linux, backend: "cpu", pluginVersion: "0.3.0" }), "sidecar");
		expect(s?.releaseAssetName).toBe("transcribe-cli-linux-x64-cpu");
		expect(s?.releaseAssetName).toBe(SIDECAR_ASSET_NAMES["linux-x64-cpu"]);
		expect(s?.url).toMatch(/\/releases\/download\/v0\.3\.0\/transcribe-cli-linux-x64-cpu$/);
		expect(s?.executable).toBe(true);
		expect(s?.sha256).toBeNull();
		expect(s?.dest).toBe("/d/bin/0.3.0/transcribe-cli-linux-x64-cpu");
	});

	it("gives each Linux backend a different sidecar destination", () => {
		const dests = (["cuda", "vulkan", "cpu"] as const).map(
			(backend) => findAsset(buildAssetPlan({ ...linux, backend }), "sidecar")?.dest,
		);
		expect(new Set(dests).size).toBe(3);
	});

	it("refuses an unsupported arch with the cpu backend too", () => {
		expect(() => buildAssetPlan({ ...linux, arch: "arm64", backend: "cpu" })).toThrow(/linux\/arm64/);
	});

	it("refuses an unsupported platform/arch without an override", () => {
		expect(() => buildAssetPlan({ ...linux, arch: "arm64" })).toThrow(/linux\/arm64/);
		expect(() => buildAssetPlan({ ...mac, arch: "x64" })).toThrow(/darwin\/x64/);
	});

	it("gives every asset a positive size and a unique destination", () => {
		const plan = buildAssetPlan(mac);
		for (const a of plan) expect(a.sizeBytes).toBeGreaterThan(0);
		expect(new Set(plan.map((a) => a.dest)).size).toBe(plan.length);
	});

	it("pins well-formed lower-case SHA-256 hashes for the models", () => {
		for (const a of buildAssetPlan(mac).filter((x) => x.sha256 !== null)) {
			expect(a.sha256).toMatch(/^[0-9a-f]{64}$/);
		}
	});
});

describe("missingAssets", () => {
	const linuxPlan = buildAssetPlan({
		platform: "linux",
		arch: "x64",
		backend: "cuda",
		dataDir: "/d",
		pluginVersion: "0.1.0",
		sidecarOverridden: false,
	});
	const macPlan = buildAssetPlan({
		platform: "darwin",
		arch: "arm64",
		backend: "vulkan",
		dataDir: "/d",
		pluginVersion: "0.1.0",
		sidecarOverridden: false,
	});

	it("lists everything when nothing exists", () => {
		expect(missingAssets(linuxPlan, () => false)).toEqual(linuxPlan);
	});

	it("lists nothing when everything exists", () => {
		expect(missingAssets(macPlan, () => true)).toEqual([]);
	});

	it("lists only the absent ones", () => {
		const present = new Set([findAsset(linuxPlan, "model")?.dest]);
		expect(missingAssets(linuxPlan, (p) => present.has(p)).map((a) => a.id).sort()).toEqual([
			"paragraph-model",
			"paragraph-tokenizer",
			"sidecar",
			"vad",
		]);
	});

	it("treats the Core ML encoder as missing when only its zip exists", () => {
		const coreml = findAsset(macPlan, "coreml");
		const present = new Set(macPlan.filter((a) => a.id !== "coreml").map((a) => a.dest));
		if (coreml?.archivePath) present.add(coreml.archivePath);
		expect(missingAssets(macPlan, (p) => present.has(p)).map((a) => a.id)).toEqual(["coreml"]);
	});

	it("does not ask about the sidecar when it is overridden", () => {
		const plan = buildAssetPlan({
			platform: "linux",
			arch: "x64",
			backend: "cpu",
			dataDir: "/d",
			pluginVersion: "0.1.0",
			sidecarOverridden: true,
		});
		const asked: string[] = [];
		missingAssets(plan, (p) => {
			asked.push(p);
			return false;
		});
		expect(asked.some((p) => p.includes("/bin/"))).toBe(false);
	});
});

describe("findAsset", () => {
	it("returns undefined for an asset not in the plan", () => {
		const plan = buildAssetPlan({
			platform: "linux",
			arch: "x64",
			backend: "vulkan",
			dataDir: "/d",
			pluginVersion: "0.1.0",
			sidecarOverridden: false,
		});
		expect(findAsset(plan, "coreml")).toBeUndefined();
	});
});

describe("parseSha256Sums", () => {
	const h = (c: string): string => c.repeat(64);

	it("parses text-mode and binary-mode lines", () => {
		const m = parseSha256Sums(`${h("a")}  transcribe-cli-linux-x64-cuda\n${h("b")} *transcribe-cli-macos-arm64\n`);
		expect(m.get("transcribe-cli-linux-x64-cuda")).toBe(h("a"));
		expect(m.get("transcribe-cli-macos-arm64")).toBe(h("b"));
	});

	it("lower-cases upper-case hex", () => {
		expect(parseSha256Sums(`${h("A")}  x`).get("x")).toBe(h("a"));
	});

	it("handles CRLF and trailing whitespace without polluting the name", () => {
		const m = parseSha256Sums(`${h("c")}  main.js  \r\n${h("d")}  styles.css\r\n`);
		expect([...m.keys()].sort()).toEqual(["main.js", "styles.css"]);
	});

	it("keeps spaces inside names", () => {
		expect(parseSha256Sums(`${h("e")}  my file.bin`).get("my file.bin")).toBe(h("e"));
	});

	it("skips malformed lines: short hash, non-hex, missing name, comments", () => {
		const text = [
			`${"a".repeat(63)}  short`,
			`${"g".repeat(64)}  nonhex`,
			`${h("a")}`,
			"# comment",
			"",
			`${"a".repeat(65)}  long`,
		].join("\n");
		expect(parseSha256Sums(text).size).toBe(0);
	});

	// BUG (minor): a hash followed only by whitespace ("<hex>  ") is parsed as an entry whose name is " ",
	// because `\s+` backtracks to leave one space for the lazy name group.
	it("skips a line with a hash and only trailing whitespace", () => {
		expect(parseSha256Sums(`${h("a")}  `).size).toBe(0);
	});

	it("does not accept a hash glued to the name without whitespace", () => {
		expect(parseSha256Sums(`${h("a")}x`).size).toBe(0);
	});

	it("does not match a name that merely shares a prefix", () => {
		const m = parseSha256Sums(`${h("a")}  transcribe-cli-linux-x64-cuda.sig`);
		expect(m.get("transcribe-cli-linux-x64-cuda")).toBeUndefined();
	});

	it("returns an empty map for empty input", () => {
		expect(parseSha256Sums("").size).toBe(0);
	});
});
