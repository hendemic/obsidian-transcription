import { describe, expect, it } from "vitest";
import { formatTimeLeft, ProgressEstimator } from "./progress-estimate";

describe("ProgressEstimator", () => {
	it("has no estimate before any report or prior expectation", () => {
		expect(new ProgressEstimator().estimate(0)).toEqual({ percent: 0, secondsLeft: null });
	});

	it("has no time left estimate at 0% without a prior expectation", () => {
		const e = new ProgressEstimator();
		e.report(0, 0);
		expect(e.estimate(5_000)).toEqual({ percent: 0, secondsLeft: null });
	});

	it("extrapolates between reports at the measured rate", () => {
		const e = new ProgressEstimator();
		e.report(0, 0);
		e.report(20, 10_000); // 2% per second
		expect(e.estimate(15_000)).toEqual({ percent: 30, secondsLeft: 35 });
	});

	it("stays short of the next likely report", () => {
		const e = new ProgressEstimator();
		e.report(0, 0);
		e.report(20, 10_000);
		// Far past where 40% should have arrived: capped at 20 + 0.95 * 20.
		expect(e.estimate(60_000).percent).toBe(39);
	});

	it("never goes backwards when a report lands below the shown estimate", () => {
		const e = new ProgressEstimator();
		e.report(0, 0);
		e.report(20, 10_000);
		const before = e.estimate(19_000).percent;
		e.report(30, 30_000); // slower window: rate drops to 1% per second
		expect(e.estimate(30_000).percent).toBeGreaterThanOrEqual(before);
	});

	it("uses the prior expectation until real progress arrives", () => {
		const e = new ProgressEstimator(100);
		e.report(0, 0);
		expect(e.estimate(20_000)).toEqual({ percent: 20, secondsLeft: 80 });
	});

	it("caps the prior-based estimate below 100", () => {
		const e = new ProgressEstimator(10);
		e.report(0, 0);
		expect(e.estimate(60_000)).toEqual({ percent: 95, secondsLeft: 0 });
	});

	it("reports 100 and the elapsed time once finished", () => {
		const e = new ProgressEstimator();
		e.report(0, 1_000);
		e.report(100, 31_000);
		expect(e.estimate(31_000)).toEqual({ percent: 100, secondsLeft: 0 });
		expect(e.elapsedSeconds).toBe(30);
	});

	it("has no elapsed time before finishing", () => {
		const e = new ProgressEstimator();
		e.report(0, 0);
		e.report(50, 1_000);
		expect(e.elapsedSeconds).toBeNull();
	});
});

describe("formatTimeLeft", () => {
	it.each([
		[null, "estimating time left"],
		[3, "almost done"],
		[12, "about 15 s left"],
		[45, "about 45 s left"],
		[90, "about 2 min left"],
		[600, "about 10 min left"],
	])("formats %j as %j", (seconds, text) => {
		expect(formatTimeLeft(seconds)).toBe(text);
	});
});
