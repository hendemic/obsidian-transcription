// Plugin settings shape and merging (pure; no Obsidian imports).
import { clampPauseGapSeconds } from "./protocol";

export type LinuxBackend = "auto" | "cuda" | "vulkan" | "cpu";
export const LINUX_BACKENDS: readonly LinuxBackend[] = ["auto", "cuda", "vulkan", "cpu"];

export interface LocalWhisperSettings {
	/** ISO code such as "en", or "auto" to detect. */
	language: string;
	initialPrompt: string;
	pauseGapSeconds: number;
	linuxBackend: LinuxBackend;
	/** Absolute path to a sidecar binary; overrides the downloaded one when non-empty. */
	sidecarPath: string;
}

export const DEFAULT_SETTINGS: LocalWhisperSettings = {
	language: "auto",
	initialPrompt: "",
	pauseGapSeconds: 1.8,
	linuxBackend: "auto",
	sidecarPath: "",
};

/** Merge stored data over the defaults, discarding values of the wrong type. */
export function mergeSettings(stored: unknown): LocalWhisperSettings {
	const out: LocalWhisperSettings = { ...DEFAULT_SETTINGS };
	if (typeof stored !== "object" || stored === null) return out;
	const s = stored as Record<string, unknown>;
	if (typeof s.language === "string" && s.language.trim() !== "") out.language = s.language.trim();
	if (typeof s.initialPrompt === "string") out.initialPrompt = s.initialPrompt;
	if (typeof s.pauseGapSeconds === "number" && Number.isFinite(s.pauseGapSeconds) && s.pauseGapSeconds > 0) {
		out.pauseGapSeconds = clampPauseGapSeconds(s.pauseGapSeconds);
	}
	if (typeof s.linuxBackend === "string" && (LINUX_BACKENDS as readonly string[]).includes(s.linuxBackend)) {
		out.linuxBackend = s.linuxBackend as LinuxBackend;
	}
	if (typeof s.sidecarPath === "string") out.sidecarPath = s.sidecarPath.trim();
	return out;
}
