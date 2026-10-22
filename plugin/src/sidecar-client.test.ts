// Sidecar client against the protocol in docs/plan.md ("Sidecar protocol", "Plugin" > "Run"), using fake
// child processes through the ChildLike / SpawnFn seams.
//
// Deliberately not covered:
// - A real child process: flow.ts wires node's `spawn`; a real binary does not exist in this repo yet.
// - Big-endian hosts: all supported platforms (macOS arm64, Linux x64) are little-endian.
import { EventEmitter } from "node:events";
import { describe, expect, it, vi } from "vitest";
import {
	pcmChunks,
	runSidecar,
	TranscriptionCancelled,
	writePcm,
	type ChildLike,
	type RunSidecarOptions,
	type SidecarProgress,
	type SpawnFn,
	type WritableLike,
} from "./sidecar-client";

/** Fake stdin: records writes; `accept` decides the return value of each write (backpressure). */
class FakeStdin extends EventEmitter implements WritableLike {
	chunks: Uint8Array[] = [];
	ended = false;
	accept: (n: number) => boolean = () => true;
	write(chunk: Uint8Array): boolean {
		// Copy, so later mutation of the source cannot hide what was written.
		this.chunks.push(new Uint8Array(chunk));
		return this.accept(this.chunks.length);
	}
	end(): void {
		this.ended = true;
	}
	bytes(): Uint8Array {
		const total = this.chunks.reduce((n, c) => n + c.length, 0);
		const out = new Uint8Array(total);
		let o = 0;
		for (const c of this.chunks) {
			out.set(c, o);
			o += c.length;
		}
		return out;
	}
}

class FakeChild extends EventEmitter implements ChildLike {
	stdin: FakeStdin | null = new FakeStdin();
	stdout = new EventEmitter();
	stderr = new EventEmitter();
	killed: (string | undefined)[] = [];
	/** When true, kill() makes the process close (like SIGTERM on a cooperative process). */
	closeOnKill = true;
	kill(signal?: string): boolean {
		this.killed.push(signal);
		if (this.closeOnKill) queueMicrotask(() => this.emit("close", null, signal ?? "SIGTERM"));
		return true;
	}
	out(s: string | Uint8Array): void {
		this.stdout.emit("data", s);
	}
	err(s: string | Uint8Array): void {
		this.stderr.emit("data", s);
	}
	exit(code: number | null, signal: string | null = null): void {
		this.emit("close", code, signal);
	}
}

function setup(over: Partial<RunSidecarOptions> = {}): {
	child: FakeChild;
	spawn: ReturnType<typeof vi.fn<SpawnFn>>;
	progress: SidecarProgress[];
	promise: Promise<string[]>;
} {
	const child = new FakeChild();
	const spawn = vi.fn<SpawnFn>(() => child);
	const progress: SidecarProgress[] = [];
	const promise = runSidecar({
		binary: "/bin/transcribe-cli",
		args: ["transcribe", "--model", "m"],
		pcm: new Float32Array([0.5, -0.5]),
		spawn,
		onProgress: (p) => progress.push(p),
		...over,
	});
	return { child, spawn, progress, promise };
}

const tick = (): Promise<void> => new Promise((r) => setImmediate(r));

async function rejection(p: Promise<unknown>): Promise<Error> {
	try {
		await p;
	} catch (e) {
		return e as Error;
	}
	throw new Error("expected the promise to reject");
}

describe("runSidecar", () => {
	it("spawns the given binary with the given args", () => {
		const { spawn } = setup();
		expect(spawn).toHaveBeenCalledWith("/bin/transcribe-cli", ["transcribe", "--model", "m"]);
	});

	it("resolves with the result paragraphs on exit 0", async () => {
		const { child, promise } = setup();
		child.out('{"type":"result","paragraphs":["One.","Two."]}\n');
		child.exit(0);
		await expect(promise).resolves.toEqual(["One.", "Two."]);
	});

	it("does not resolve before the process closes", async () => {
		const { child, promise } = setup();
		let done = false;
		void promise.then(() => (done = true));
		child.out('{"type":"result","paragraphs":["x"]}\n');
		await tick();
		expect(done).toBe(false);
		child.exit(0);
		await promise;
		expect(done).toBe(true);
	});

	it("reports progress in order", async () => {
		const { child, progress, promise } = setup();
		child.out('{"type":"progress","stage":"loading"}\n');
		child.out('{"type":"progress","stage":"transcribing","percent":10}\n{"type":"progress","stage":"transc');
		child.out('ribing","percent":55}\n');
		child.out('{"type":"result","paragraphs":[]}\n');
		child.exit(0);
		await promise;
		expect(progress).toEqual([
			{ stage: "loading" },
			{ stage: "transcribing", percent: 10 },
			{ stage: "transcribing", percent: 55 },
		]);
	});

	it("works without an onProgress callback", async () => {
		const { child, promise } = setup({ onProgress: undefined });
		child.out('{"type":"progress","stage":"loading"}\n{"type":"result","paragraphs":["a"]}\n');
		child.exit(0);
		await expect(promise).resolves.toEqual(["a"]);
	});

	it("parses stdout delivered as bytes split inside a UTF-8 character", async () => {
		const { child, promise } = setup();
		const bytes = new TextEncoder().encode('{"type":"result","paragraphs":["Grüße"]}\n');
		const cut = bytes.indexOf(0xc3) + 1;
		child.out(bytes.slice(0, cut));
		child.out(bytes.slice(cut));
		child.exit(0);
		await expect(promise).resolves.toEqual(["Grüße"]);
	});

	it("accepts a final result line without a trailing newline", async () => {
		const { child, promise } = setup();
		child.out('{"type":"result","paragraphs":["tail"]}');
		child.exit(0);
		await expect(promise).resolves.toEqual(["tail"]);
	});

	it("ignores blank lines", async () => {
		const { child, promise } = setup();
		child.out('\n\r\n  \n{"type":"result","paragraphs":["a"]}\n\n');
		child.exit(0);
		await expect(promise).resolves.toEqual(["a"]);
	});

	it("resolves with an empty transcript", async () => {
		const { child, promise } = setup();
		child.out('{"type":"result","paragraphs":[]}\n');
		child.exit(0);
		await expect(promise).resolves.toEqual([]);
	});

	describe("errors", () => {
		it("rejects with the message of an error line", async () => {
			const { child, promise } = setup();
			child.out('{"type":"error","message":"model file is corrupt"}\n');
			child.exit(1);
			const e = await rejection(promise);
			expect(e.message).toBe("model file is corrupt");
			expect(e).not.toBeInstanceOf(TranscriptionCancelled);
		});

		it("rejects on an error line even when the exit code is 0", async () => {
			const { child, promise } = setup();
			child.out('{"type":"error","message":"boom"}\n');
			child.exit(0);
			expect((await rejection(promise)).message).toBe("boom");
		});

		it("rejects on a non-zero exit without a result, including the exit code and stderr tail", async () => {
			const { child, promise } = setup();
			child.err("ggml_cuda_init: failed\n");
			child.err("CUDA error: out of memory\n");
			child.exit(134);
			const e = await rejection(promise);
			expect(e.message).toContain("134");
			expect(e.message).toContain("CUDA error: out of memory");
		});

		it("rejects on exit 0 without a result", async () => {
			const { child, promise } = setup();
			child.out('{"type":"progress","stage":"loading"}\n');
			child.exit(0);
			const e = await rejection(promise);
			expect(e.message).toMatch(/without a result/);
		});

		it("does not include stderr in the message when stderr is empty or blank", async () => {
			const { child, promise } = setup();
			child.err("  \n");
			child.exit(2);
			const e = await rejection(promise);
			expect(e.message).toBe("The transcription engine exited with code 2");
		});

		it("keeps only the tail of very long stderr output", async () => {
			const { child, promise } = setup();
			child.err("HEAD-MARKER " + "x".repeat(20_000));
			child.err("\nfinal diagnostic line");
			child.exit(1);
			const e = await rejection(promise);
			expect(e.message).toContain("final diagnostic line");
			expect(e.message).not.toContain("HEAD-MARKER");
			expect(e.message.length).toBeLessThan(10_000);
		});

		it("decodes stderr delivered as bytes", async () => {
			const { child, promise } = setup();
			child.err(new TextEncoder().encode("error: bad input\n"));
			child.exit(1);
			expect((await rejection(promise)).message).toContain("error: bad input");
		});

		// BUG (minor): each stderr byte chunk is decoded with a fresh TextDecoder, so a multi-byte character
		// split across chunks turns into replacement characters in the error shown to the user.
		it("decodes a UTF-8 character split across stderr chunks", async () => {
			const { child, promise } = setup();
			const bytes = new TextEncoder().encode("Datei nicht gefunden: Grüße.bin\n");
			const cut = bytes.indexOf(0xc3) + 1;
			child.err(bytes.slice(0, cut));
			child.err(bytes.slice(cut));
			child.exit(1);
			expect((await rejection(promise)).message).toContain("Grüße.bin");
		});

		// Minor UX gap: a process killed by a signal (e.g. the OOM killer) closes with code null, and the
		// message reads "exited with code null". The signal is available in the close event but unused.
		it("names the signal when the process is killed externally", async () => {
			const { child, promise } = setup();
			child.exit(null, "SIGKILL");
			const e = await rejection(promise);
			expect(e.message).not.toContain("null");
			expect(e.message).toContain("SIGKILL");
		});

		it("kills the child and rejects on a malformed stdout line", async () => {
			const { child, promise } = setup();
			child.out("whisper_init: loading model from /m\n");
			expect(child.killed).toHaveLength(1);
			const e = await rejection(promise);
			expect(e.message).toMatch(/Unexpected output/);
			expect(e.message).toContain("whisper_init: loading model from /m");
		});

		it("kills on an unknown message type", async () => {
			const { child, promise } = setup();
			child.out('{"type":"warning","message":"x"}\n');
			expect(child.killed).toHaveLength(1);
			await expect(promise).rejects.toThrow(/Unexpected output/);
		});

		it("still rejects on a malformed line if a result arrives afterwards", async () => {
			const child = new FakeChild();
			child.closeOnKill = false;
			const promise = runSidecar({ binary: "b", args: [], pcm: new Float32Array(), spawn: () => child });
			child.out("garbage\n");
			child.out('{"type":"result","paragraphs":["late"]}\n');
			child.exit(0);
			await expect(promise).rejects.toThrow(/Unexpected output/);
		});

		it("does not report progress after a malformed line", async () => {
			const { child, progress, promise } = setup();
			child.out('garbage\n{"type":"progress","stage":"loading"}\n');
			await rejection(promise);
			expect(progress).toEqual([]);
		});

		it("kills on a malformed trailing line flushed at close", async () => {
			const child = new FakeChild();
			child.closeOnKill = false;
			const promise = runSidecar({ binary: "b", args: [], pcm: new Float32Array(), spawn: () => child });
			child.out('{"type":"result","paragraphs":["a"]}\n{"trunc');
			child.exit(0);
			await expect(promise).rejects.toThrow(/Unexpected output/);
		});

		it("rejects when the child emits an error (e.g. ENOENT) and ignores the later close", async () => {
			const { child, promise } = setup();
			child.emit("error", new Error("spawn /bin/transcribe-cli ENOENT"));
			child.exit(-2);
			const e = await rejection(promise);
			expect(e.message).toContain("ENOENT");
		});

		it("rejects when spawn itself throws", async () => {
			const promise = runSidecar({
				binary: "b",
				args: [],
				pcm: new Float32Array(),
				spawn: () => {
					throw new Error("EACCES");
				},
			});
			await expect(promise).rejects.toThrow(/EACCES/);
		});

		it("rejects with a string thrown by spawn", async () => {
			const promise = runSidecar({
				binary: "b",
				args: [],
				pcm: new Float32Array(),
				spawn: () => {
					// eslint-disable-next-line @typescript-eslint/only-throw-error -- spawn may throw a non-Error; that path is under test
					throw "weird";
				},
			});
			await expect(promise).rejects.toThrow(/weird/);
		});
	});

	describe("cancellation", () => {
		it("kills the child and rejects with TranscriptionCancelled on abort", async () => {
			const ac = new AbortController();
			const { child, promise } = setup({ signal: ac.signal });
			child.out('{"type":"progress","stage":"loading"}\n');
			ac.abort();
			expect(child.killed).toHaveLength(1);
			await expect(promise).rejects.toBeInstanceOf(TranscriptionCancelled);
		});

		it("reports cancellation even if the killed process prints an error line or exits non-zero", async () => {
			const ac = new AbortController();
			const child = new FakeChild();
			child.closeOnKill = false;
			const promise = runSidecar({
				binary: "b",
				args: [],
				pcm: new Float32Array(),
				spawn: () => child,
				signal: ac.signal,
			});
			ac.abort();
			child.out('{"type":"error","message":"interrupted"}\n');
			child.err("terminated\n");
			child.exit(143);
			await expect(promise).rejects.toBeInstanceOf(TranscriptionCancelled);
		});

		it("does not spawn at all when the signal is already aborted", async () => {
			const ac = new AbortController();
			ac.abort();
			const { spawn, promise } = setup({ signal: ac.signal });
			await expect(promise).rejects.toBeInstanceOf(TranscriptionCancelled);
			expect(spawn).not.toHaveBeenCalled();
		});

		it("does not kill after the run has finished", async () => {
			const ac = new AbortController();
			const { child, promise } = setup({ signal: ac.signal });
			child.out('{"type":"result","paragraphs":["a"]}\n');
			child.exit(0);
			await promise;
			ac.abort();
			expect(child.killed).toEqual([]);
		});

		it("removes its abort listener when done", async () => {
			const ac = new AbortController();
			const remove = vi.spyOn(ac.signal, "removeEventListener");
			const { child, promise } = setup({ signal: ac.signal });
			child.exit(1);
			await rejection(promise);
			expect(remove).toHaveBeenCalledWith("abort", expect.any(Function));
		});

		it("survives kill() throwing", async () => {
			const ac = new AbortController();
			const child = new FakeChild();
			child.kill = () => {
				throw new Error("ESRCH");
			};
			const promise = runSidecar({
				binary: "b",
				args: [],
				pcm: new Float32Array(),
				spawn: () => child,
				signal: ac.signal,
			});
			expect(() => ac.abort()).not.toThrow();
			child.exit(null, "SIGTERM");
			await expect(promise).rejects.toBeInstanceOf(TranscriptionCancelled);
		});
	});

	describe("PCM on stdin", () => {
		it("writes the PCM as little-endian f32 and ends stdin", async () => {
			const { child } = setup({ pcm: new Float32Array([1, -2, 0.5]) });
			await tick();
			const stdin = child.stdin as FakeStdin;
			expect(Array.from(stdin.bytes())).toEqual([
				0x00, 0x00, 0x80, 0x3f, // 1.0
				0x00, 0x00, 0x00, 0xc0, // -2.0
				0x00, 0x00, 0x00, 0x3f, // 0.5
			]);
			expect(stdin.ended).toBe(true);
		});

		it("writes every sample of a long recording, in order", async () => {
			const n = 16384 * 3 + 7;
			const pcm = new Float32Array(n);
			for (let i = 0; i < n; i++) pcm[i] = i;
			const { child } = setup({ pcm });
			await tick();
			const stdin = child.stdin as FakeStdin;
			const bytes = stdin.bytes();
			expect(bytes.length).toBe(n * 4);
			const view = new DataView(bytes.buffer);
			for (const i of [0, 1, 16383, 16384, 32768, n - 1]) expect(view.getFloat32(i * 4, true)).toBe(i);
			expect(stdin.chunks.length).toBeGreaterThan(1);
			expect(stdin.ended).toBe(true);
		});

		it("ends stdin without writing for an empty recording", async () => {
			const { child } = setup({ pcm: new Float32Array() });
			await tick();
			const stdin = child.stdin as FakeStdin;
			expect(stdin.chunks).toEqual([]);
			expect(stdin.ended).toBe(true);
		});

		it("copes with a child without stdin", async () => {
			const child = new FakeChild();
			child.stdin = null;
			const promise = runSidecar({ binary: "b", args: [], pcm: new Float32Array([1]), spawn: () => child });
			child.out('{"type":"result","paragraphs":["a"]}\n');
			child.exit(0);
			await expect(promise).resolves.toEqual(["a"]);
		});

		it("reports the process failure, not the broken pipe, when stdin errors (EPIPE)", async () => {
			const n = 16384 * 4;
			const child = new FakeChild();
			const stdin = child.stdin as FakeStdin;
			stdin.accept = () => false;
			const promise = runSidecar({ binary: "b", args: [], pcm: new Float32Array(n), spawn: () => child });
			await tick();
			expect(stdin.chunks).toHaveLength(1);
			stdin.emit("error", Object.assign(new Error("write EPIPE"), { code: "EPIPE" }));
			await tick();
			expect(stdin.chunks).toHaveLength(1);
			expect(stdin.ended).toBe(false);
			child.err("failed to load model\n");
			child.exit(1);
			const e = await rejection(promise);
			expect(e.message).toContain("failed to load model");
			expect(e.message).not.toContain("EPIPE");
		});
	});
});

describe("writePcm", () => {
	it("waits for drain before writing more", async () => {
		const stdin = new FakeStdin();
		stdin.accept = () => false;
		const done = writePcm(stdin, new Float32Array(16384 * 3));
		await tick();
		expect(stdin.chunks).toHaveLength(1);
		stdin.emit("drain");
		await tick();
		expect(stdin.chunks).toHaveLength(2);
		stdin.emit("drain");
		await tick();
		expect(stdin.chunks).toHaveLength(3);
		expect(stdin.ended).toBe(false);
		stdin.emit("drain");
		await done;
		expect(stdin.ended).toBe(true);
	});

	it("does not leak drain/close/error listeners per chunk", async () => {
		const stdin = new FakeStdin();
		stdin.accept = () => false;
		const done = writePcm(stdin, new Float32Array(16384 * 5));
		for (let i = 0; i < 5; i++) {
			await tick();
			stdin.emit("drain");
		}
		await done;
		expect(stdin.listenerCount("drain")).toBe(0);
		expect(stdin.listenerCount("close")).toBe(0);
		// One persistent error listener is fine (it guards against an unhandled 'error').
		expect(stdin.listenerCount("error")).toBeLessThanOrEqual(1);
	});

	it("stops without ending when the pipe errors while waiting for drain", async () => {
		const stdin = new FakeStdin();
		stdin.accept = () => false;
		const done = writePcm(stdin, new Float32Array(16384 * 3));
		await tick();
		stdin.emit("error", new Error("EPIPE"));
		await expect(done).resolves.toBeUndefined();
		expect(stdin.chunks).toHaveLength(1);
		expect(stdin.ended).toBe(false);
	});

	it("does not throw an unhandled error when the pipe errors after all writes", async () => {
		const stdin = new FakeStdin();
		await writePcm(stdin, new Float32Array(4));
		expect(() => stdin.emit("error", new Error("EPIPE"))).not.toThrow();
	});

	// BUG (minor): the doc comment says writing "stops quietly if the pipe closes", but a 'close' while
	// waiting for drain only resumes the loop; the next chunk is written to the closed pipe and the writer then
	// waits for a drain that never comes (a leaked pending promise). Only 'error' sets the broken flag.
	it("stops writing when stdin closes while waiting for drain", async () => {
		const stdin = new FakeStdin();
		stdin.accept = () => false;
		void writePcm(stdin, new Float32Array(16384 * 3));
		await tick();
		stdin.emit("close");
		await tick();
		expect(stdin.chunks).toHaveLength(1);
	});
});

describe("pcmChunks", () => {
	it("yields little-endian bytes of a subarray, honouring its byte offset", () => {
		const backing = new Float32Array([9, 9, 1, 9]);
		const view = backing.subarray(2, 3);
		const chunks = [...pcmChunks(view)];
		expect(chunks).toHaveLength(1);
		expect(Array.from(chunks[0] ?? [])).toEqual([0x00, 0x00, 0x80, 0x3f]);
	});

	it("splits into chunks of the requested sample count, with a short last chunk", () => {
		const chunks = [...pcmChunks(new Float32Array(10), 4)];
		expect(chunks.map((c) => c.byteLength)).toEqual([16, 16, 8]);
	});

	it("yields nothing for empty PCM", () => {
		expect([...pcmChunks(new Float32Array())]).toEqual([]);
	});

	it("preserves special float values bit-exactly", () => {
		const pcm = new Float32Array([-0, Number.NaN, Number.POSITIVE_INFINITY]);
		const bytes = Array.from([...pcmChunks(pcm)][0] ?? []);
		expect(bytes.slice(0, 4)).toEqual([0x00, 0x00, 0x00, 0x80]); // -0
		expect(bytes.slice(8, 12)).toEqual([0x00, 0x00, 0x80, 0x7f]); // +Infinity
	});
});
