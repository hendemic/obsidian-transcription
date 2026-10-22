// Sidecar protocol: JSON lines on stdout, plus argument building. See docs/plan.md.

export type SidecarMessage =
	| { type: "progress"; stage: "loading" }
	| { type: "progress"; stage: "transcribing"; percent: number }
	| { type: "result"; paragraphs: string[] }
	| { type: "error"; message: string };

export class ProtocolError extends Error {
	constructor(
		message: string,
		public readonly line: string,
	) {
		super(message);
		this.name = "ProtocolError";
	}
}

/** Parse one stdout line. Returns null for blank lines; throws ProtocolError for malformed ones. */
export function parseLine(line: string): SidecarMessage | null {
	const trimmed = line.trim();
	if (trimmed === "") return null;
	let raw: unknown;
	try {
		raw = JSON.parse(trimmed);
	} catch {
		throw new ProtocolError("not valid JSON", line);
	}
	if (typeof raw !== "object" || raw === null) throw new ProtocolError("not a JSON object", line);
	const obj = raw as Record<string, unknown>;
	switch (obj.type) {
		case "progress":
			if (obj.stage === "loading") return { type: "progress", stage: "loading" };
			if (obj.stage === "transcribing" && typeof obj.percent === "number") {
				return { type: "progress", stage: "transcribing", percent: obj.percent };
			}
			throw new ProtocolError("unknown progress message", line);
		case "result":
			if (Array.isArray(obj.paragraphs) && obj.paragraphs.every((p) => typeof p === "string")) {
				return { type: "result", paragraphs: obj.paragraphs };
			}
			throw new ProtocolError("result without a paragraphs string array", line);
		case "error":
			if (typeof obj.message === "string") return { type: "error", message: obj.message };
			throw new ProtocolError("error without a message", line);
		default:
			throw new ProtocolError("unknown message type", line);
	}
}

/** Accumulates stream chunks (strings or bytes, possibly split mid-line or mid-character) into whole lines. */
export class LineBuffer {
	private pending = "";
	private readonly decoder = new TextDecoder("utf-8");

	push(chunk: string | Uint8Array): string[] {
		this.pending +=
			typeof chunk === "string" ? chunk : this.decoder.decode(chunk, { stream: true });
		const parts = this.pending.split("\n");
		this.pending = parts.pop() ?? "";
		return parts.map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));
	}

	/** The trailing unterminated line, if any (call once at end of stream). */
	flush(): string[] {
		const rest = this.pending + this.decoder.decode();
		this.pending = "";
		return rest === "" ? [] : [rest];
	}
}

export interface TranscribeArgsInput {
	modelPath: string;
	vadModelPath?: string;
	/** "auto" or empty means auto-detect (flag omitted). */
	language?: string;
	prompt?: string;
	pauseGapSeconds?: number;
}

export function buildTranscribeArgs(input: TranscribeArgsInput): string[] {
	const args = ["transcribe", "--model", input.modelPath];
	if (input.vadModelPath) args.push("--vad-model", input.vadModelPath);
	const language = input.language?.trim();
	if (language && language.toLowerCase() !== "auto") args.push("--language", language);
	const prompt = input.prompt?.trim();
	if (prompt) args.push("--prompt", prompt);
	const gap = input.pauseGapSeconds;
	if (gap !== undefined && Number.isFinite(gap) && gap > 0) {
		args.push("--pause-gap-ms", String(Math.round(gap * 1000)));
	}
	return args;
}
