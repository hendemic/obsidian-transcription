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

	it("keeps time for the remaining windows while a report is overdue", () => {
		const e = new ProgressEstimator();
		e.report(0, 0);
		e.report(25, 10_000); // 2.5% per second, 25% per window
		// The 50% report is overdue, so two more windows (50%) are still to come: 20 s.
		expect(e.estimate(60_000).secondsLeft).toBe(20);
	});

	it("says the last window is nearly done once it is overdue", () => {
		const e = new ProgressEstimator();
		e.report(0, 0);
		e.report(50, 10_000);
		expect(e.estimate(60_000).secondsLeft).toBe(0);
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
		[0.5, "almost done"],
		[7.2, "about 8 s left"],
		[14, "about 14 s left"],
		[16, "about 20 s left"],
		[45, "about 45 s left"],
		[90, "about 1 min 30 s left"],
		[118, "about 2 min left"],
		[125, "about 2 min 10 s left"],
		[400, "about 7 min left"],
		[430, "about 7 min 30 s left"],
	])("formats %j as %j", (seconds, text) => {
		expect(formatTimeLeft(seconds)).toBe(text);
	});
});
