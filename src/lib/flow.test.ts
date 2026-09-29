import { describe, expect, it } from "vitest";
import { rampAt } from "./flow";
import { autoGain } from "./visualsframe";

describe("the flow's arithmetic (D169)", () => {
  it("lifts a quiet waveform toward full size, up to sixteen times", () => {
    expect(autoGain(0.8)).toBeCloseTo(1);
    expect(autoGain(0.2)).toBeCloseTo(4);
    expect(autoGain(0.016)).toBe(16);
    expect(autoGain(0)).toBe(16);
  });

  it("walks the ramp, blending between neighbours and wrapping around", () => {
    const ramp: [number, number, number][] = [
      [0, 0, 0],
      [1, 1, 1],
    ];
    expect(rampAt(ramp, 0)).toEqual([0, 0, 0]);
    expect(rampAt(ramp, 0.5)).toEqual([0.5, 0.5, 0.5]);
    expect(rampAt(ramp, 1.5)).toEqual([0.5, 0.5, 0.5]);
    expect(rampAt(ramp, -0.5)).toEqual([0.5, 0.5, 0.5]);
    expect(rampAt([], 3)).toEqual([1, 1, 1]);
  });
});
