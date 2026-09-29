import { describe, expect, it } from "vitest";
import { cardioid, EDGE, juliaC } from "./fractal";

describe("the fractal's walk (D170)", () => {
  it("walks the main cardioid's edge: the cusp at a quarter, the neck at minus three quarters", () => {
    const [cx, cy] = cardioid(0);
    expect(cx).toBeCloseTo(0.25);
    expect(cy).toBeCloseTo(0);
    const [nx, ny] = cardioid(Math.PI);
    expect(nx).toBeCloseTo(-0.75);
    expect(ny).toBeCloseTo(0);
  });

  it("keeps c just inside the edge, where the sets stay connected", () => {
    for (const a of [0.5, 1.5, 2.2, 3, 4.5]) {
      const [ex, ey] = cardioid(a);
      const [x, y] = juliaC(a);
      expect(Math.hypot(x, y)).toBeCloseTo(Math.hypot(ex, ey) * EDGE);
    }
  });
});
