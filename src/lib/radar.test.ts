import { describe, expect, it } from "vitest";
import { age, alertsStale, frameAt, issued, readout, STALE_MS, stackTop, type RadarStatus } from "./radar";

const site = { id: "KJAX", name: "Jacksonville", state: "FL", lat: 30.48, lon: -81.7, region: "CONUS" };
const at = Date.parse("2026-09-13T18:44:00Z");
const status = (over: Partial<RadarStatus>): RadarStatus => ({
  site,
  frames: [],
  last_attempt_ms: null,
  last_ok_ms: null,
  last_error: null,
  alerts: [],
  alerts_ms: null,
  frame_size: [550, 1044],
  site_y: 188,
  ...over,
});
const tz = "America/New_York";

describe("the radar readout never lets cached data look current (#85)", () => {
  it("says how old a fresh loop is, and that it is live", () => {
    const r = readout(status({ frames: [{ time_ms: at - 2 * 60000, path: "a" }], last_attempt_ms: at, last_ok_ms: at }), at, tz);
    expect(r.state).toBe("live");
    expect(r.text).toBe("RADAR · KJAX · 14:42 EDT · 2m old · LIVE");
    expect(r.warn).toBe(false);
    expect(r.short).toBe("2M OLD");
  });

  it("goes stale past the line, in the warning colour", () => {
    const old = at - (4 * 60 + 12) * 60000;
    const r = readout(status({ frames: [{ time_ms: old, path: "a" }], last_attempt_ms: at, last_ok_ms: at }), at, tz);
    expect(STALE_MS).toBe(3600 * 1000);
    expect(r.state).toBe("stale");
    expect(r.text).toContain("4h 12m old · STALE");
    expect(r.short).toBe("4H12M OLD");
    expect(r.warn).toBe(true);
  });

  it("says OFFLINE when the last fetch failed, however young the loop", () => {
    const r = readout(
      status({ frames: [{ time_ms: at - 5 * 60000, path: "a" }], last_attempt_ms: at, last_ok_ms: at - 600000, last_error: "no route" }),
      at,
      tz,
    );
    expect(r.state).toBe("offline");
    expect(r.text).toMatch(/· OFFLINE$/);
    expect(r.warn).toBe(true);
  });

  it("says what is missing when there is nothing to draw", () => {
    expect(readout(null, at).text).toBe("RADAR · PICK YOUR RADAR IN THE LIBRARY");
    expect(readout(status({}), at).text).toBe("RADAR · KJAX · NO DATA · FETCHING");
    expect(readout(status({ last_attempt_ms: at, last_error: "offline" }), at)).toMatchObject({ state: "offline", warn: true });
  });

  it("counts minutes and hours", () => {
    expect(age(20000)).toBe("just now");
    expect(age(59 * 60000)).toBe("59m old");
    expect(age(125 * 60000)).toBe("2h 5m old");
  });

  it("calls alerts stale on the same line, and gives their issue time", () => {
    expect(alertsStale(status({ alerts_ms: at - 60000 }), at)).toBe(false);
    expect(alertsStale(status({ alerts_ms: at - STALE_MS - 1 }), at)).toBe(true);
    expect(alertsStale(status({}), at)).toBe(true);
    const a = { event: "Hurricane Warning", severity: "Extreme", headline: null, sent: "2026-09-13T14:05:00-04:00", expires: null };
    expect(issued(a, tz)).toBe("issued 14:05 EDT");
    expect(issued({ ...a, sent: null })).toBe("");
  });
});

describe("the loop", () => {
  it("steps through the frames slowly, holds the newest, and agrees across windows", () => {
    expect(frameAt(0, 0, false)).toBe(-1);
    expect(frameAt(123456, 5, true)).toBe(4);
    expect(frameAt(0, 5, false)).toBe(0);
    expect(frameAt(1000, 5, false)).toBe(2);
    // The newest holds for the hold steps, then it starts again.
    expect(frameAt(4500, 5, false)).toBe(4);
    expect(frameAt(5000, 5, false)).toBe(4);
    expect(frameAt(5500, 5, false)).toBe(0);
  });

  it("puts the windows in their classic order in the picture", () => {
    expect([stackTop("main"), stackTop("equalizer"), stackTop("playlist")]).toEqual([0, 116, 232]);
  });
});
