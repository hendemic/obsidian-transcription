// whisper.cpp reports progress once per 30 s window of audio, which with large models means long
// gaps and big jumps. This fills the gaps by extrapolating from the measured rate.

export interface ProgressEstimate {
	/** Whole percent, never decreasing; 100 once the engine has reported it. */
	percent: number;
	/** Seconds until transcription finishes, or null while there is nothing to base it on. */
	secondsLeft: number | null;
}

/**
 * Turns the engine's coarse percent reports into a smoothly advancing estimate.
 * Times are milliseconds from any monotonic clock.
 */
export class ProgressEstimator {
	private start: number | null = null;
	private finishedAt: number | null = null;
	private lastPercent = 0;
	private lastAt = 0;
	private largestStep = 0;
	private shown = 0;

	/**
	 * @param expectedSeconds How long transcription is expected to take, e.g. from the previous run.
	 *   Used only until the engine reports real progress.
	 */
	constructor(private readonly expectedSeconds: number | null = null) {}

	report(percent: number, at: number): void {
		if (this.start === null) {
			this.start = at;
			this.lastAt = at;
		}
		if (percent > this.lastPercent) {
			this.largestStep = Math.max(this.largestStep, percent - this.lastPercent);
			this.lastPercent = percent;
			this.lastAt = at;
		}
		if (percent >= 100 && this.finishedAt === null) this.finishedAt = at;
	}

	/** Seconds from the first report to 100%, once finished. */
	get elapsedSeconds(): number | null {
		return this.start !== null && this.finishedAt !== null ? (this.finishedAt - this.start) / 1000 : null;
	}

	estimate(at: number): ProgressEstimate {
		if (this.lastPercent >= 100) return { percent: 100, secondsLeft: 0 };
		const rate = this.rate();
		if (this.start === null || rate === null) return { percent: Math.floor(this.lastPercent), secondsLeft: null };

		const extrapolated = this.lastPercent + rate * (at - this.lastAt);
		// Stay short of where the next report is likely to land, so the bar never jumps backwards.
		const ceiling = Math.min(99, this.largestStep > 0 ? this.lastPercent + this.largestStep * 0.95 : 95);
		this.shown = Math.max(this.shown, Math.min(extrapolated, ceiling));
		// When a report is overdue (a slow window), the windows after it are still to come, so time
		// left must not fall to zero while waiting.
		const projected = (100 - extrapolated) / rate;
		const afterNextReport = this.largestStep > 0 ? (100 - this.lastPercent - this.largestStep) / rate : 0;
		return {
			percent: Math.floor(this.shown),
			secondsLeft: Math.max(0, projected, afterNextReport) / 1000,
		};
	}

	/** Percent per millisecond. */
	private rate(): number | null {
		if (this.start !== null && this.lastPercent > 0 && this.lastAt > this.start) {
			return this.lastPercent / (this.lastAt - this.start);
		}
		return this.expectedSeconds && this.expectedSeconds > 0 ? 100 / (this.expectedSeconds * 1000) : null;
	}
}

export function formatTimeLeft(seconds: number | null): string {
	if (seconds === null) return "estimating time left";
	if (seconds < 5) return "almost done";
	if (seconds < 60) return `about ${Math.ceil(seconds / 5) * 5} s left`;
	if (seconds < 300) {
		const rounded = Math.round(seconds / 10) * 10;
		const min = Math.floor(rounded / 60);
		const sec = rounded % 60;
		return sec === 0 ? `about ${min} min left` : `about ${min} min ${sec} s left`;
	}
	return `about ${Math.round(seconds / 60)} min left`;
}
