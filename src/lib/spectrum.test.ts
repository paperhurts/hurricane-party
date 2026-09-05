import { describe, expect, it } from "vitest";
import { bandEdges, Levels, reduceBands } from "./spectrum";

describe("bandEdges", () => {
  const cases: [bars: number, bins: number, rate: number][] = [
    [19, 1024, 44100],
    [19, 512, 44100],
    [32, 1024, 48000],
    [24, 1024, 44100],
    [64, 1024, 48000],
    [8, 128, 22050],
  ];

  it.each(cases)("%i bars over %i bins at %i Hz: contiguous, non-empty, in range", (bars, bins, rate) => {
    const e = bandEdges(bars, bins, rate);
    expect(e.length).toBe(bars * 2);
    expect(e[0]).toBeGreaterThanOrEqual(1); // never DC
    for (let i = 0; i < bars; i++) {
      const lo = e[i * 2];
      const hi = e[i * 2 + 1];
      expect(hi).toBeGreaterThan(lo); // at least one bin
      if (i > 0) expect(lo).toBe(e[i * 2 - 1]); // contiguous
    }
    expect(e[bars * 2 - 1]).toBeLessThanOrEqual(bins);
  });

  it("spaces the bands wider as frequency rises", () => {
    const e = bandEdges(19, 1024, 44100);
    const width = (i: number) => e[i * 2 + 1] - e[i * 2];
    expect(width(18)).toBeGreaterThan(width(9));
    expect(width(9)).toBeGreaterThanOrEqual(width(0));
  });

  it("stops at fMax", () => {
    // 1024 bins at 44.1 kHz: 21.5 Hz per bin, so 16 kHz is bin 743.
    const e = bandEdges(19, 1024, 44100, 50, 16000);
    expect(e[e.length - 1]).toBeLessThanOrEqual(743);
  });

  it("refuses shapes that cannot give every bar a bin", () => {
    expect(() => bandEdges(0, 1024, 44100)).toThrow(RangeError);
    expect(() => bandEdges(200, 100, 44100)).toThrow(RangeError);
  });
});

describe("reduceBands", () => {
  it("takes the loudest bin in each band, scaled to 0..1", () => {
    const edges = new Uint16Array([1, 3, 3, 6]);
    const data = new Uint8Array([255, 10, 51, 0, 255, 102, 255]);
    const v = reduceBands(data, edges);
    expect(v.length).toBe(2);
    expect(v[0]).toBeCloseTo(51 / 255);
    expect(v[1]).toBeCloseTo(1); // bin 4, not bin 6 (outside the band)
  });

  it("reuses the output buffer", () => {
    const edges = new Uint16Array([1, 2]);
    const out = new Float32Array(1);
    expect(reduceBands(new Uint8Array([0, 128]), edges, out)).toBe(out);
  });
});

describe("Levels", () => {
  it("rises instantly and falls no faster than `fall`", () => {
    const l = new Levels(1, { fall: 0.1, hold: 0 });
    l.step([0.8]);
    expect(l.bars[0]).toBeCloseTo(0.8);
    l.step([0]);
    expect(l.bars[0]).toBeCloseTo(0.7);
    l.step([0.2]);
    expect(l.bars[0]).toBeCloseTo(0.6);
    l.step([0.9]);
    expect(l.bars[0]).toBeCloseTo(0.9);
  });

  it("holds the peak, then lets it fall with gravity, never below the bar", () => {
    const l = new Levels(1, { fall: 1, hold: 3, gravity: 0.1 });
    l.step([0.5]);
    expect(l.peaks[0]).toBeCloseTo(0.5);
    // Three frames of hold at zero input.
    l.step([0]);
    l.step([0]);
    l.step([0]);
    expect(l.peaks[0]).toBeCloseTo(0.5);
    // Released: falls 0.1, then 0.2, accelerating.
    l.step([0]);
    expect(l.peaks[0]).toBeCloseTo(0.4);
    l.step([0]);
    expect(l.peaks[0]).toBeCloseTo(0.2);
    // A bar underneath stops the fall.
    l.step([0.15]);
    expect(l.peaks[0]).toBeCloseTo(0.15);
  });

  it("clamps input to 0..1 and reports settled only at rest", () => {
    const l = new Levels(2, { fall: 1, hold: 0, gravity: 1 });
    expect(l.settled()).toBe(true);
    l.step([2, -1]);
    expect(l.bars[0]).toBe(1);
    expect(l.bars[1]).toBe(0);
    expect(l.settled()).toBe(false);
    l.step([0, 0]);
    l.step([0, 0]);
    expect(l.settled()).toBe(true);
  });
});
