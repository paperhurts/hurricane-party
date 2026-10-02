import { describe, expect, it } from "vitest";
import captain from "../../skins/companions/captain/companion.json";
import {
  COMPANION_CELL,
  COMPANION_COLUMNS,
  COMPANION_STATES,
  SUGGESTED,
  companionTemplateManifest,
  companionTemplateReadme,
} from "./companiontemplate";

describe("a companion to paint (D163)", () => {
  it("says it is painted, so the import reads its cells", () => {
    expect(companionTemplateManifest()).toEqual({
      format: "hp-companion/1",
      name: "My companion",
      sprite: "sheet.png",
      painted: true,
    });
    expect(companionTemplateManifest("Sir Waddles").name).toBe("Sir Waddles");
  });

  it("is the Cap'n's layout: the same rows in the same order, 64 px cells, and up to 32 across", () => {
    expect(captain.frameSize).toEqual([COMPANION_CELL, COMPANION_CELL]);
    // His sheet is as wide as a pack ever is with eight or fewer a state,
    // 8 across; the template has room for 32, and the import lays a painted
    // pack out no wider than its longest row (D183).
    const his = 8;
    const rows = Object.entries(captain.states).map(([s, v]) => [s, Math.floor(v.frames[0] / his)]);
    expect(rows).toEqual(COMPANION_STATES.map((s, i) => [s, i]));
    expect(COMPANION_COLUMNS).toBe(32);
    for (const s of COMPANION_STATES) expect(SUGGESTED[s]).toBeLessThanOrEqual(his);
  });

  it("has a README that names every row and how to paint in Aseprite, in CRLF", () => {
    const readme = companionTemplateReadme();
    for (const s of COMPANION_STATES) expect(readme).toContain(` ${s} `);
    expect(readme).toContain("2048 x 448");
    expect(readme).toContain("4096 x 896");
    expect(readme).toContain("up to 32");
    expect(readme).toContain("Aseprite");
    expect(readme).toContain("Import companion");
    expect(readme.split("\n").every((l, i, all) => i === all.length - 1 || l.endsWith("\r"))).toBe(true);
  });
});
