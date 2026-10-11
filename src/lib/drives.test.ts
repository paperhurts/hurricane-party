import { describe, expect, it } from "vitest";
import { onDrive, outRoots, sayOut } from "./drives";

const roots = [
  { id: 1, label: "Default", present: true },
  { id: 2, label: "hp", present: false },
  { id: 3, label: "stick", present: false },
];
const on = (...ids: number[]) => ids.map((root_id) => ({ root_id }));

describe("a track on a drive that is out (D143)", () => {
  it("is out when its root is not there, and in when its root is, or is not listed yet", () => {
    const out = outRoots(roots);
    expect(onDrive({ root_id: 1 }, out)).toBe(true);
    expect(onDrive({ root_id: 2 }, out)).toBe(false);
    expect(onDrive({ root_id: 9 }, out)).toBe(true);
  });

  it("says nothing about a list with nothing out", () => {
    expect(sayOut(on(1, 1), roots, false)).toBeNull();
    expect(sayOut([], roots, true)).toBeNull();
  });

  it("says how many are hidden and on which drives", () => {
    expect(sayOut(on(1, 2), roots, false)).toBe("1 track is hidden: it's on hp, which isn't plugged in.");
    expect(sayOut(on(2, 2, 1), roots, false)).toBe("2 tracks are hidden: they're on hp, which isn't plugged in.");
    expect(sayOut(on(3, 2, 2), roots, false)).toBe("3 tracks are hidden: they're on hp and stick, which aren't plugged in.");
  });

  it("says they will not play while they are shown", () => {
    expect(sayOut(on(2), roots, true)).toBe("1 track on hp won't play until it's plugged in.");
    expect(sayOut(on(2, 3), roots, true)).toBe("2 tracks on hp and stick won't play until they're plugged in.");
    const more = [...roots, { id: 4, label: "backup", present: false }];
    expect(sayOut(on(2, 3, 4), more, true)).toBe("3 tracks on hp, stick and backup won't play until they're plugged in.");
  });
});
