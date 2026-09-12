import { describe, expect, it } from "vitest";
import tokens from "../../design/tokens.json";
import { TOKENS } from "./skin";
import { colorsFor, typeFor, viscolor, type ThemeName } from "./theme";

// Issue #28 was a theme that filled its own set of colour names: every
// `var(--surface)` under Purricane resolved to nothing, because `applyTheme`
// sets what a theme declares and nothing clears what the last one left. Equal
// key sets make that impossible, so the key sets are what these tests check.
const names = Object.keys(tokens.themes) as ThemeName[];

describe("every theme", () => {
  it.each(names)("%s fills all six roles and nothing else", (name) => {
    const c = colorsFor(name);
    expect(Object.keys(c).sort()).toEqual([...TOKENS].sort());
    for (const t of TOKENS) expect(c[t]).toMatch(/^#[0-9A-Fa-f]{6}$/);
  });

  it.each(names)("%s resolves a chrome and a UI face", (name) => {
    const t = typeFor(name);
    expect(t.chrome).toBeTruthy();
    expect(t.ui).toBeTruthy();
  });

  // Cone has no palette of its own and `extends` Eyewall, so the ramp walk is
  // what gives it one. Purricane draws a kaleidoscope and has no ramp yet.
  it("inherits the radar ramp through extends", () => {
    expect(viscolor("eyewall")).toHaveLength(24);
    expect(viscolor("cone")).toEqual(viscolor("eyewall"));
  });

  // A theme may keep its own words for the six — Eyewall's void and filament,
  // Purricane's sugar and ink — as documentation beside the roles (D108).
  // Nothing reads them, so the only thing to get wrong is the set.
  it.each(names)("%s labels either all six roles or none", (name) => {
    const own = (tokens.themes as Record<string, { names?: Record<string, string> }>)[name].names;
    if (!own) return;
    const labelled = Object.keys(own).filter((k) => k !== "$comment");
    expect(labelled.sort()).toEqual([...TOKENS].sort());
  });
});
