import { describe, expect, it } from "vitest";
import { bassOf, parseVisualsFrame, Pulse, rgbOf, silentFrame, VISUALS_BANDS, VISUALS_WAVE } from "./visualsframe";

function frameBytes(beat: boolean, bands: number[], waveAt = 128): Uint8Array {
  const b = new Uint8Array(4 + bands.length + VISUALS_WAVE);
  b.set([beat ? 1 : 0, 255, 51, 0]);
  b.set(bands, 4);
  b.fill(waveAt, 4 + bands.length);
  return b;
}

describe("the visuals window's frames (D169)", () => {
  it("reads flags, levels, bands and the wave", () => {
    const f = parseVisualsFrame(frameBytes(true, [10, 20, 30], 200).buffer as ArrayBuffer)!;
    expect(f.beat).toBe(true);
    expect(f.peak).toBe(1);
    expect(f.rms).toBeCloseTo(0.2);
    expect([...f.bands]).toEqual([10, 20, 30]);
    expect(f.wave.length).toBe(VISUALS_WAVE);
    expect(f.wave[0]).toBe(200);
  });

  it("refuses bytes too short to be a frame", () => {
    expect(parseVisualsFrame(new Uint8Array(4 + VISUALS_WAVE))).toBeNull();
  });

  it("starts silent: no bands, a flat wave", () => {
    const f = silentFrame();
    expect(f.bands.length).toBe(VISUALS_BANDS);
    expect(f.wave.every((w) => w === 128)).toBe(true);
    expect(bassOf(f)).toBe(0);
  });

  it("hears the bass in the lowest bands", () => {
    const f = parseVisualsFrame(frameBytes(false, [255, 255, 0, 0, 0, 0, 0, 0]))!;
    expect(bassOf(f, 2)).toBe(1);
    expect(bassOf(f, 4)).toBe(0.5);
  });

  it("pulses on a beat at most three times a second, and fades", () => {
    const p = new Pulse();
    expect(p.beat(0)).toBe(true);
    expect(p.value(0)).toBe(1);
    expect(p.beat(0.2)).toBe(false);
    expect(p.value(0.2)).toBeGreaterThan(0);
    expect(p.value(0.4)).toBe(0);
    expect(p.beat(0.34)).toBe(true);
    // A beat every 50 ms for a second makes at most three pulses.
    const q = new Pulse();
    let pulses = 0;
    for (let t = 0; t < 1; t += 0.05) if (q.beat(t)) pulses++;
    expect(pulses).toBeLessThanOrEqual(3);
  });

  it("reads a ramp colour", () => {
    expect(rgbOf("#ff8000")).toEqual([1, 128 / 255, 0]); // tokens-exempt: the parser's test input, not a colour shown
    expect(rgbOf("nonsense")).toEqual([0, 0, 0]);
  });
});
