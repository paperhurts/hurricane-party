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

  it("is the Cap'n's layout: the same rows in the same order, 64 px cells, 8 across", () => {
    expect(captain.frameSize).toEqual([COMPANION_CELL, COMPANION_CELL]);
    const rows = Object.entries(captain.states).map(([s, v]) => [s, Math.floor(v.frames[0] / COMPANION_COLUMNS)]);
    expect(rows).toEqual(COMPANION_STATES.map((s, i) => [s, i]));
    for (const s of COMPANION_STATES) expect(SUGGESTED[s]).toBeLessThanOrEqual(COMPANION_COLUMNS);
  });

  it("has a README that names every row and how to paint in Aseprite, in CRLF", () => {
    const readme = companionTemplateReadme();
    for (const s of COMPANION_STATES) expect(readme).toContain(` ${s} `);
    expect(readme).toContain("512 x 448");
    expect(readme).toContain("1024 x 896");
    expect(readme).toContain("Aseprite");
    expect(readme).toContain("Import companion");
    expect(readme.split("\n").every((l, i, all) => i === all.length - 1 || l.endsWith("\r"))).toBe(true);
  });
});
