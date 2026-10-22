// Spawns the sidecar, streams PCM to stdin and reads protocol lines from stdout.
// Structural types let tests substitute a fake child process.
import { LineBuffer, parseLine, ProtocolError } from "./protocol";

export interface WritableLike {
	write(chunk: Uint8Array): boolean;
	end(): void;
	on(event: "error" | "drain" | "close", cb: () => void): unknown;
	off(event: "error" | "drain" | "close", cb: () => void): unknown;
}

export interface ReadableLike {
	on(event: "data", cb: (chunk: string | Uint8Array) => void): unknown;
}

export interface ChildLike {
	stdin: WritableLike | null;
	stdout: ReadableLike | null;
	stderr: ReadableLike | null;
	on(event: "error", cb: (err: Error) => void): unknown;
	on(event: "close", cb: (code: number | null, signal: string | null) => void): unknown;
	kill(signal?: string): boolean;
}

export type SpawnFn = (command: string, args: string[]) => ChildLike;

export type SidecarProgress =
	| { stage: "loading" }
	| { stage: "transcribing"; percent: number };

export class TranscriptionCancelled extends Error {
	constructor() {
		super("Transcription cancelled");
		this.name = "TranscriptionCancelled";
	}
}

export interface RunSidecarOptions {
	binary: string;
	args: string[];
	pcm: Float32Array;
	spawn: SpawnFn;
	onProgress?: (p: SidecarProgress) => void;
	signal?: AbortSignal;
}

const PCM_CHUNK_SAMPLES = 16384; // 64 KiB
const STDERR_LIMIT = 8192;

/** Split PCM into little-endian f32 byte chunks (all supported platforms are little-endian). */
export function* pcmChunks(pcm: Float32Array, chunkSamples = PCM_CHUNK_SAMPLES): Generator<Uint8Array> {
	for (let i = 0; i < pcm.length; i += chunkSamples) {
		const part = pcm.subarray(i, Math.min(i + chunkSamples, pcm.length));
		yield new Uint8Array(part.buffer, part.byteOffset, part.byteLength);
	}
}

/** Write all PCM then end stdin, honouring backpressure. Stops quietly if the pipe closes. */
export async function writePcm(stdin: WritableLike, pcm: Float32Array): Promise<void> {
	let broken = false;
	const markBroken = (): void => {
		broken = true;
	};
	stdin.on("error", markBroken);
	for (const chunk of pcmChunks(pcm)) {
		if (broken) return;
		if (!stdin.write(chunk)) {
			await new Promise<void>((resolve) => {
				const done = (): void => {
					stdin.off("drain", done);
					stdin.off("close", done);
					stdin.off("error", done);
					resolve();
				};
				stdin.on("drain", done);
				stdin.on("close", done);
				stdin.on("error", done);
			});
		}
	}
	if (!broken) stdin.end();
}

/** Run one transcription. Resolves with the paragraphs; rejects on error or cancellation. */
export function runSidecar(opts: RunSidecarOptions): Promise<string[]> {
	return new Promise<string[]>((resolve, reject) => {
		if (opts.signal?.aborted) {
			reject(new TranscriptionCancelled());
			return;
		}
		let child: ChildLike;
		try {
			child = opts.spawn(opts.binary, opts.args);
		} catch (e) {
			reject(new Error(`Could not start the transcription engine: ${errMsg(e)}`));
			return;
		}

		let settled = false;
		let result: string[] | null = null;
		let reportedError: string | null = null;
		let cancelled = false;
		let stderrText = "";
		const stdoutLines = new LineBuffer();

		const finish = (fn: () => void): void => {
			if (settled) return;
			settled = true;
			opts.signal?.removeEventListener("abort", onAbort);
			fn();
		};
		const kill = (): void => {
			try {
				child.kill();
			} catch {
				// already gone
			}
		};
		function onAbort(): void {
			cancelled = true;
			kill();
		}
		opts.signal?.addEventListener("abort", onAbort);

		const handleLine = (line: string): void => {
			if (settled) return;
			try {
				const msg = parseLine(line);
				if (!msg) return;
				if (msg.type === "progress") {
					opts.onProgress?.(
						msg.stage === "loading"
							? { stage: "loading" }
							: { stage: "transcribing", percent: msg.percent },
					);
				} else if (msg.type === "result") {
					result = msg.paragraphs;
				} else {
					reportedError = msg.message;
				}
			} catch (e) {
				const detail = e instanceof ProtocolError ? `${e.message}: ${e.line}` : errMsg(e);
				kill();
				finish(() => reject(new Error(`Unexpected output from the transcription engine (${detail})`)));
			}
		};

		child.stdout?.on("data", (chunk) => stdoutLines.push(chunk).forEach(handleLine));
		child.stderr?.on("data", (chunk) => {
			const text = typeof chunk === "string" ? chunk : new TextDecoder().decode(chunk);
			stderrText = (stderrText + text).slice(-STDERR_LIMIT);
		});
		child.on("error", (err) => {
			finish(() => reject(new Error(`Could not run the transcription engine: ${err.message}`)));
		});
		child.on("close", (code) => {
			stdoutLines.flush().forEach(handleLine);
			finish(() => {
				if (cancelled) reject(new TranscriptionCancelled());
				else if (result) resolve(result);
				else if (reportedError !== null) reject(new Error(reportedError));
				else {
					const tail = stderrText.trim();
					const how = code === 0 ? "finished without a result" : `exited with code ${String(code)}`;
					reject(new Error(`The transcription engine ${how}${tail ? `: ${tail}` : ""}`));
				}
			});
		});

		if (child.stdin) {
			writePcm(child.stdin, opts.pcm).catch(() => {
				// A broken pipe means the process died; the close handler reports why.
			});
		}
	});
}

function errMsg(e: unknown): string {
	return e instanceof Error ? e.message : String(e);
}
