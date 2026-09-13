import { describe, expect, it } from "vitest";
import { BLOOM_MS, BloomGate, bloomAt, centres, hueAt, MAX_BLOOM_HZ, MAX_DEG_PER_SEC, mix, rotationAt } from "./kaleidoscope";

// docs/purricane.md: "Cap rotation speed and never let audio increase it",
// "Cap flash frequency below 3 Hz", "prefers-reduced-motion produces a static
// mandala". These are the clamps, tested because they are the ones that hurt
// someone when they are wrong.
describe("the kaleidoscope's clamps (#147)", () => {
  it("turns at a slow constant, capped, and not at all when still", () => {
    const quarter = rotationAt(1, 4, false);
    expect(quarter).toBeCloseTo((4 * Math.PI) / 180);
    // A theme asking for 90 degrees a second gets the cap.
    expect(rotationAt(1, 90, false)).toBeCloseTo((MAX_DEG_PER_SEC * Math.PI) / 180);
    expect(rotationAt(12.3, 4, true)).toBe(0);
    expect(rotationAt(1, Number.NaN, false)).toBe(0);
  });

  it("never blooms faster than 3 Hz, however fast the beat", () => {
    const gate = new BloomGate(MAX_BLOOM_HZ);
    let blooms = 0;
    // Ten seconds at 60 frames a second, a hard kick every 100 ms (10 Hz).
    for (let f = 0; f < 600; f++) {
      const now = (f * 1000) / 60;
      const kick = Math.floor(now) % 100 < 17;
      if (gate.feed(kick ? 1 : 0.05, now, false)) blooms++;
    }
    expect(blooms).toBeGreaterThan(0);
    expect(blooms / 10).toBeLessThan(3);
  });

  it("does not bloom faster than the cap even when a theme asks for more", () => {
    const gate = new BloomGate(20);
    const times: number[] = [];
    for (let f = 0; f < 600; f++) {
      const now = (f * 1000) / 60;
      if (gate.feed(f % 4 === 0 ? 1 : 0.05, now, false)) times.push(now);
    }
    for (let i = 1; i < times.length; i++) expect(times[i] - times[i - 1]).toBeGreaterThan(1000 / MAX_BLOOM_HZ);
  });

  it("never blooms when still", () => {
    const gate = new BloomGate();
    for (let f = 0; f < 600; f++) expect(gate.feed(f % 20 === 0 ? 1 : 0, f * 16, true)).toBe(false);
  });

  it("blooms briefly and fades", () => {
    expect(bloomAt(1000, 1000)).toBe(1);
    expect(bloomAt(1000 + BLOOM_MS / 2, 1000)).toBeCloseTo(0.5);
    expect(bloomAt(1000 + BLOOM_MS, 1000)).toBe(0);
    expect(bloomAt(900, 1000)).toBe(0);
  });
});

describe("the kaleidoscope's colour and layout", () => {
  const stops = ["#000000", "#FFFFFF"]; // tokens-exempt: two ends of a ramp to test the mix, not colours anything draws

  it("drifts through the palette, holding each colour, and comes back", () => {
    // Two stops over ten seconds: five seconds each.
    expect(hueAt(0, stops, 10)).toBe(mix(stops[0], stops[1], 0));
    // Held for most of its turn, not half-way to the next.
    expect(hueAt(2.5, stops, 10)).toBe(mix(stops[0], stops[1], 0));
    // Blending over the last part of it.
    expect(hueAt(4.25, stops, 10)).toBe(mix(stops[0], stops[1], 0.5));
    expect(hueAt(10, stops, 10)).toBe(hueAt(0, stops, 10));
    expect(hueAt(3, [], 10)).toBe("transparent");
  });

  it("lays a band of mandalas across a wide, short display, and one in a square", () => {
    const band = centres(410, 80);
    expect(band.length).toBe(3);
    for (const c of band) expect(c.r).toBeCloseTo(80 * 0.62);
    expect(centres(80, 80)).toHaveLength(1);
    expect(centres(0, 80)).toEqual([]);
  });
});
