// Network and filesystem glue for downloads: stream to a temp file, verify, rename.
import { createWriteStream } from "node:fs";
import { mkdir, rename, rm } from "node:fs/promises";
import type { IncomingMessage } from "node:http";
import { get } from "node:https";
import { dirname } from "node:path";
import { pipeline } from "node:stream/promises";
import { verifySha256 } from "./checksum";

const MAX_REDIRECTS = 10;

export type ProgressFn = (received: number, total: number | null) => void;

function openStream(url: string, signal?: AbortSignal, redirects = 0): Promise<IncomingMessage> {
	return new Promise((resolve, reject) => {
		if (redirects > MAX_REDIRECTS) {
			reject(new Error("Too many redirects"));
			return;
		}
		const req = get(url, { headers: { "User-Agent": "local-whisper-transcribe" }, signal }, (res) => {
			const status = res.statusCode ?? 0;
			if (status >= 300 && status < 400 && res.headers.location) {
				res.resume();
				const next = new URL(res.headers.location, url).toString();
				openStream(next, signal, redirects + 1).then(resolve, reject);
			} else if (status === 200) {
				resolve(res);
			} else {
				res.resume();
				reject(new Error(`Download failed: HTTP ${status} for ${url}`));
			}
		});
		req.on("error", reject);
	});
}

export async function fetchText(url: string, signal?: AbortSignal): Promise<string> {
	const res = await openStream(url, signal);
	res.setEncoding("utf8");
	let body = "";
	for await (const chunk of res) body += chunk as string;
	return body;
}

/**
 * Download to `<dest>.part`, verify the SHA-256, then rename onto `dest`.
 * The temp file is removed on any failure, including a checksum mismatch.
 */
export async function downloadVerified(opts: {
	url: string;
	destPath: string;
	sha256: string;
	onProgress?: ProgressFn;
	signal?: AbortSignal;
}): Promise<void> {
	const tmp = `${opts.destPath}.part`;
	await mkdir(dirname(opts.destPath), { recursive: true });
	try {
		const res = await openStream(opts.url, opts.signal);
		const lengthHeader = res.headers["content-length"];
		const total = lengthHeader ? Number(lengthHeader) : null;
		let received = 0;
		res.on("data", (chunk: Uint8Array) => {
			received += chunk.length;
			opts.onProgress?.(received, total && Number.isFinite(total) ? total : null);
		});
		await pipeline(res, createWriteStream(tmp), { signal: opts.signal });
		await verifySha256(tmp, opts.sha256);
		await rename(tmp, opts.destPath);
	} catch (e) {
		await rm(tmp, { force: true });
		throw e;
	}
}
