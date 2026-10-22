import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";

export async function sha256File(path: string): Promise<string> {
	const hash = createHash("sha256");
	for await (const chunk of createReadStream(path)) hash.update(chunk as Uint8Array);
	return hash.digest("hex");
}

export class ChecksumMismatchError extends Error {
	constructor(
		public readonly expected: string,
		public readonly actual: string,
	) {
		super(`Checksum mismatch (expected ${expected}, got ${actual})`);
		this.name = "ChecksumMismatchError";
	}
}

/** Resolves if the file's SHA-256 equals `expected` (case-insensitive); otherwise throws ChecksumMismatchError. */
export async function verifySha256(path: string, expected: string): Promise<void> {
	const actual = await sha256File(path);
	if (actual !== expected.trim().toLowerCase()) {
		throw new ChecksumMismatchError(expected.toLowerCase(), actual);
	}
}
