import { describe, expect, it } from "vitest";
import tokens from "../../design/tokens.json";
import { TOKENS } from "./skin";
import { contrast } from "./palette";
import {
  colorsFor,
  colorsShown,
  colorsWorn,
  kaleidoscopeFor,
  rampWorn,
  typeFor,
  typeScale,
  viscolor,
  visualizerFor,
  WEARABLE,
  type ThemeName,
} from "./theme";

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

describe("what a classic window paints while wearing a skin (D101)", () => {
  const palette = Object.fromEntries(TOKENS.map((t, i) => [t, `#00000${i}`])) as Record<string, string>;

  it("a mask skin follows the theme, so one grey sheet wears any of them", () => {
    expect(colorsWorn({ art: "mask", palette })).toEqual(colorsFor("eyewall"));
    // The theme as it is worn: legible (#147), which for Purricane moves its
    // pale accents off its near-white ground.
    expect(colorsWorn({ art: "mask", palette }, "purricane")).toEqual(colorsShown("purricane"));
  });

  // Every imported .wsz is `final`: its PLEDIT.TXT colours are derived,
  // written and validated, and until D101's condition ran they were never
  // shown — the rows wore Eyewall's cyan and magenta over someone else's art.
  it("a final skin brings the colours beside its pixels", () => {
    expect(colorsWorn({ art: "final", palette })).toEqual(palette);
  });
});

// #147: a theme a person can pick is worn legible, and brings its analyser.
describe("wearing a theme", () => {
  it("wears Eyewall exactly as the tokens write it", () => {
    expect(colorsShown("eyewall")).toEqual(colorsFor("eyewall"));
  });

  it.each(WEARABLE)("%s is legible: words 4.5:1 on ground and surface, the rest 3:1 on the ground", (name) => {
    const c = colorsShown(name);
    expect(contrast(c.text, c.ground)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(c.text, c.surface)).toBeGreaterThanOrEqual(4.5);
    for (const t of ["accent", "alert", "warn"] as const) expect(contrast(c[t], c.ground)).toBeGreaterThanOrEqual(3);
  });

  it("keeps Purricane's ground and surface as written, and its accent's hue", () => {
    const raw = colorsFor("purricane");
    const worn = colorsShown("purricane");
    expect(worn.ground).toBe(raw.ground);
    expect(worn.surface).toBe(raw.surface);
    // The pastel mint is about 2:1 on the pink window (D131); it deepens, and
    // stays a green-blue.
    expect(worn.accent).not.toBe(raw.accent);
    const rgb = (h: string) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
    const [r, g, bl] = rgb(worn.accent);
    expect(g).toBeGreaterThan(r);
    expect(bl).toBeGreaterThan(r);
  });

  it("sets Comic Sans larger, and leaves Eyewall's sizes alone", () => {
    expect(typeScale("eyewall")).toBe(1);
    expect(typeScale("purricane")).toBeGreaterThan(1);
  });

  const mask = { art: "mask", palette: {}, viscolor: ["a"], visualizer: { component: "spectrum-bars" } };
  const own = { ...mask, art: "final" };

  it("gives a mask skin the theme's analyser and ramp, and a skin with its own colours its own", () => {
    expect(visualizerFor(mask, "eyewall")).toBe("spectrum-bars");
    expect(visualizerFor(mask, "purricane")).toBe("kaleidoscope");
    expect(visualizerFor(own, "purricane")).toBe("spectrum-bars");
    expect(rampWorn(mask, "eyewall")).toEqual(viscolor("eyewall"));
    expect(rampWorn(own, "purricane")).toEqual(["a"]);
    // Purricane has no ramp in the tokens, so one is made from its colours.
    expect(rampWorn(mask, "purricane")).toHaveLength(24);
    expect(colorsWorn({ art: "mask", palette: {} }, "purricane")).toEqual(colorsShown("purricane"));
  });

  it("caps the kaleidoscope where the tokens could ask for more", () => {
    const k = kaleidoscopeFor("purricane");
    expect([6, 8]).toContain(k.segments);
    expect(k.degPerSec).toBeLessThanOrEqual(10);
    expect(k.maxBloomHz).toBeLessThanOrEqual(3);
  });
});
