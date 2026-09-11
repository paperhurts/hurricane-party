import { describe, expect, it } from "vitest";
import { endedId, isRepeat, nextRepeat, shuffled, startId, stepId } from "./playorder";

/** A random source that walks a fixed list, so a shuffle is repeatable. */
function seq(...xs: number[]): () => number {
  let i = 0;
  return () => xs[i++ % xs.length];
}

describe("repeat", () => {
  it("cycles off, one, all, off", () => {
    expect(nextRepeat("off")).toBe("one");
    expect(nextRepeat("one")).toBe("all");
    expect(nextRepeat("all")).toBe("off");
  });
  it("knows its own values and nothing else", () => {
    expect(isRepeat("all")).toBe(true);
    expect(isRepeat("twice")).toBe(false);
    expect(isRepeat(null)).toBe(false);
  });
});

describe("shuffled", () => {
  it("is a permutation: every id once, nothing added", () => {
    const ids = [1, 2, 3, 4, 5, 6];
    const s = shuffled(ids, null, seq(0.9, 0.1, 0.5, 0.3, 0.7));
    expect([...s].sort()).toEqual(ids);
  });
  it("leads with `first`, so turning shuffle on keeps the song playing", () => {
    for (const r of [0, 0.25, 0.5, 0.99]) {
      const s = shuffled([1, 2, 3, 4], 3, () => r);
      expect(s[0]).toBe(3);
      expect([...s].sort()).toEqual([1, 2, 3, 4]);
    }
  });
  it("ignores a `first` that is not in the list", () => {
    expect(shuffled([1, 2], 9, () => 0).length).toBe(2);
  });
  it("actually moves things", () => {
    // Fisher-Yates with rng 0 swaps each position with the first.
    expect(shuffled([1, 2, 3, 4], null, () => 0)).not.toEqual([1, 2, 3, 4]);
  });
});

describe("startId: Play with nothing loaded (#116)", () => {
  it("starts on the playlist window's selected row", () => {
    expect(startId([10, 20, 30], 20)).toBe(20);
  });
  it("starts on the first row when nothing is selected", () => {
    expect(startId([10, 20, 30], null)).toBe(10);
  });
  it("ignores a selection that is not in the list showing", () => {
    expect(startId([10, 20, 30], 99)).toBe(10);
  });
  it("has nothing to start on in an empty list", () => {
    expect(startId([], null)).toBeNull();
  });
});

describe("stepId: Next and Previous", () => {
  const o = [10, 20, 30];
  it("walks the order", () => {
    expect(stepId(o, 10, 1, false)).toBe(20);
    expect(stepId(o, 30, -1, false)).toBe(20);
  });
  it("starts from a standing start: Next at the first, Previous at the last", () => {
    expect(stepId(o, null, 1, false)).toBe(10);
    expect(stepId(o, null, -1, false)).toBe(30);
  });
  it("stops at either end without repeat all", () => {
    expect(stepId(o, 30, 1, false)).toBeNull();
    expect(stepId(o, 10, -1, false)).toBeNull();
  });
  it("wraps at either end with repeat all", () => {
    expect(stepId(o, 30, 1, true)).toBe(10);
    expect(stepId(o, 10, -1, true)).toBe(30);
  });
  it("treats a current that left the list as nothing current", () => {
    expect(stepId(o, 99, 1, false)).toBe(10);
  });
  it("has nowhere to go in an empty list", () => {
    expect(stepId([], 10, 1, true)).toBeNull();
  });
});

describe("endedId: a track ending on its own", () => {
  const o = [10, 20, 30];
  it("repeat one plays it again", () => {
    expect(endedId(o, 20, "one")).toBe(20);
  });
  it("repeat all walks on and wraps", () => {
    expect(endedId(o, 20, "all")).toBe(30);
    expect(endedId(o, 30, "all")).toBe(10);
  });
  it("off walks on and stops at the end", () => {
    expect(endedId(o, 20, "off")).toBe(30);
    expect(endedId(o, 30, "off")).toBeNull();
  });
});
