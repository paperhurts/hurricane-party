import { describe, expect, it } from "vitest";
import { between, placeFor, rowKey } from "./rows";

describe("a row is named by itself (#252)", () => {
  it("tells two copies of one track apart", () => {
    expect(rowKey({ id: 5, item: 1 })).not.toBe(rowKey({ id: 5, item: 2 }));
  });

  it("keys a row with no id of its own by its track", () => {
    expect(rowKey({ id: 5, item: null })).toBe("m5");
    expect(rowKey({ id: 5, item: 1 })).toBe("i1");
  });
});

describe("a drop is named by the rows either side of it", () => {
  it("has nothing above at the top, nothing below at the bottom", () => {
    expect(between([1, 2, 3], 0)).toEqual({ above: null, below: 1 });
    expect(between([1, 2, 3], 1)).toEqual({ above: 1, below: 2 });
    expect(between([1, 2, 3], 3)).toEqual({ above: 3, below: null });
  });

  it("lands before the row below the line, else after the row above it", () => {
    expect(placeFor(1, 3, 4)).toEqual({ before: 4, after: 3 });
    expect(placeFor(5, null, 1)).toEqual({ before: 1, after: null });
    expect(placeFor(1, 5, null)).toEqual({ before: null, after: 5 });
  });

  it("does not move a row dropped just above or just below itself, or alone", () => {
    expect(placeFor(2, 1, 2)).toBeNull();
    expect(placeFor(2, 2, 3)).toBeNull();
    expect(placeFor(1, null, 1)).toBeNull();
    expect(placeFor(1, null, null)).toBeNull();
  });
});

describe("putting a list in order with rows hidden (D143)", () => {
  // A B C D, rows 1 2 3 4, with B on a drive that is out: A C D show.
  const shown = [1, 3, 4];

  it("names the showing rows either side, so the hidden row keeps its place", () => {
    // A dropped between C and D: before D, after C. Rust makes it B C A D.
    const { above, below } = between(shown, 2);
    expect(placeFor(1, above, below)).toEqual({ before: 4, after: 3 });
  });

  it("does not move a row dropped just above the next showing row", () => {
    // A dropped above C, its next showing row: where it already is.
    const { above, below } = between(shown, 1);
    expect(placeFor(1, above, below)).toBeNull();
  });
});
