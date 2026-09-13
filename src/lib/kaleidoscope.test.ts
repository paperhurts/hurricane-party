import { describe, expect, it } from "vitest";
import { BLOOM_MS, BloomGate, bloomAt, centres, driftHue, hueOf, MAX_BLOOM_HZ, MAX_DEG_PER_SEC, rotationAt } from "./kaleidoscope";
import tokens from "../../design/tokens.json";

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
  it("reads a colour's hue", () => {
    const ramp = tokens.themes.purricane.visualizer.palette;
    // The designer's drift starts at floss pink, 325 degrees.
    expect(hueOf(ramp[0])).toBeCloseTo(325, 0);
    expect(hueOf(tokens.themes.purricane.colors.alert)).toBeCloseTo(246, 0);
    expect(hueOf("not a colour")).toBe(0);
  });

  it("drifts round the hues the short way, and comes back", () => {
    // Four hues over forty seconds: ten each, gliding into the next.
    const hues = [350, 10, 100, 200];
    expect(driftHue(0, hues, 40)).toBeCloseTo(350);
    // Half-way from 350 to 10 is 0, not 180: across the top of the wheel.
    expect(driftHue(5, hues, 40)).toBeCloseTo(0);
    expect(driftHue(15, hues, 40)).toBeCloseTo(55);
    // From 200 back to 350 is 150 forward, not 210 back.
    expect(driftHue(35, hues, 40)).toBeCloseTo(275);
    expect(driftHue(40, hues, 40)).toBeCloseTo(driftHue(0, hues, 40));
    expect(driftHue(3, [], 40)).toBe(0);
  });

  it("puts one mandala in a round badge, filling it, and a band across a wide display", () => {
    expect(centres(34, 34)).toEqual([{ x: 17, y: 17, r: 17 }]);
    const band = centres(410, 80);
    expect(band.length).toBe(3);
    for (const c of band) expect(c.r).toBeCloseTo(80 * 0.55);
    expect(centres(0, 80)).toEqual([]);
  });
});
