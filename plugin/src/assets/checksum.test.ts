// SHA-256 verification of downloaded assets (docs/plan.md, "Asset manager": "SHA-256 verification").
// Uses real temporary files: the point is to lock in hashing of real bytes on disk.
//
// Expected digests are the well-known SHA-256 test vectors (FIPS 180-2 / NIST examples), not computed here.
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { ChecksumMismatchError, sha256File, verifySha256 } from "./checksum";

const EMPTY = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const ABC = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
// SHA-256 of one million 'a' characters (NIST test vector).
const MILLION_A = "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0";

const dir = mkdtempSync(join(tmpdir(), "lwt-checksum-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));

function file(name: string, content: string | Uint8Array): string {
	const p = join(dir, name);
	writeFileSync(p, content);
	return p;
}

describe("sha256File", () => {
	it("hashes an empty file", async () => {
		await expect(sha256File(file("empty", ""))).resolves.toBe(EMPTY);
	});

	it("hashes 'abc'", async () => {
		await expect(sha256File(file("abc", "abc"))).resolves.toBe(ABC);
	});

	it("hashes a file larger than one read chunk (stream reassembly)", async () => {
		await expect(sha256File(file("million", "a".repeat(1_000_000)))).resolves.toBe(MILLION_A);
	});

	it("returns lower-case hex", async () => {
		expect(await sha256File(file("abc2", "abc"))).toMatch(/^[0-9a-f]{64}$/);
	});

	it("rejects for a missing file", async () => {
		await expect(sha256File(join(dir, "does-not-exist"))).rejects.toThrow();
	});
});

describe("verifySha256", () => {
	it("resolves when the hash matches", async () => {
		await expect(verifySha256(file("v1", "abc"), ABC)).resolves.toBeUndefined();
	});

	it("matches upper-case expected hex", async () => {
		await expect(verifySha256(file("v2", "abc"), ABC.toUpperCase())).resolves.toBeUndefined();
	});

	it("matches mixed-case expected hex with surrounding whitespace", async () => {
		const mixed = ABC.slice(0, 32).toUpperCase() + ABC.slice(32);
		await expect(verifySha256(file("v3", "abc"), `  ${mixed}\n`)).resolves.toBeUndefined();
	});

	it("throws ChecksumMismatchError with both hashes on mismatch", async () => {
		const p = file("v4", "abd");
		let thrown: unknown;
		try {
			await verifySha256(p, ABC);
		} catch (e) {
			thrown = e;
		}
		expect(thrown).toBeInstanceOf(ChecksumMismatchError);
		const err = thrown as ChecksumMismatchError;
		expect(err.expected).toBe(ABC);
		expect(err.actual).toMatch(/^[0-9a-f]{64}$/);
		expect(err.actual).not.toBe(ABC);
		expect(err.message).toContain(ABC);
	});

	it("detects a single flipped bit", async () => {
		await expect(verifySha256(file("v5", "abc"), ABC.slice(0, 63) + "e")).rejects.toBeInstanceOf(
			ChecksumMismatchError,
		);
	});

	it("detects a truncated download", async () => {
		await expect(verifySha256(file("v6", "ab"), ABC)).rejects.toBeInstanceOf(ChecksumMismatchError);
	});

	it("does not accept a prefix of the right hash", async () => {
		await expect(verifySha256(file("v7", "abc"), ABC.slice(0, 32))).rejects.toBeInstanceOf(ChecksumMismatchError);
	});

	it("does not accept an empty expected hash", async () => {
		await expect(verifySha256(file("v8", ""), "")).rejects.toBeInstanceOf(ChecksumMismatchError);
	});

	it("reports the expected hash normalized to lower case", async () => {
		const err = (await verifySha256(file("v9", "x"), ABC.toUpperCase()).catch((e: unknown) => e)) as ChecksumMismatchError;
		expect(err.expected).toBe(ABC);
	});

	it("rejects (not as a mismatch) for a missing file", async () => {
		const err = await verifySha256(join(dir, "nope"), ABC).catch((e: unknown) => e);
		expect(err).toBeInstanceOf(Error);
		expect(err).not.toBeInstanceOf(ChecksumMismatchError);
	});
});
