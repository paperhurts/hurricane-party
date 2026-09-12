import { describe, expect, it } from "vitest";
import { TOKENS } from "./skin";
import { chromaOf, contrast, EDGE_CONTRAST, paletteFromPixels, rampFrom, TEXT_CONTRAST } from "./palette";

/** A picture made of flat patches, each a share of the frame: what a photo
 * reduces to once its colours are clustered. `alpha` is 255 unless given. */
function picture(patches: { rgb: [number, number, number]; share: number; alpha?: number }[]): Uint8ClampedArray {
  const size = 64 * 64;
  const px = new Uint8ClampedArray(size * 4);
  let at = 0;
  for (const p of patches) {
    const n = Math.round(p.share * size);
    for (let i = 0; i < n && at < size; i++, at++) {
      px.set([...p.rgb, p.alpha ?? 255], at * 4);
    }
  }
  // Whatever the shares leave over is the first patch, so the frame is full.
  for (; at < size; at++) px.set([...patches[0].rgb, patches[0].alpha ?? 255], at * 4);
  return px;
}

/** What every made palette owes a person, whatever picture it came from. */
function legible(p: Record<string, string>) {
  for (const t of TOKENS) expect(p[t], t).toMatch(/^#[0-9A-F]{6}$/);
  expect(contrast(p.text, p.ground), "text on ground").toBeGreaterThanOrEqual(TEXT_CONTRAST);
  expect(contrast(p.text, p.surface), "text on surface").toBeGreaterThanOrEqual(TEXT_CONTRAST);
  expect(contrast(p.accent, p.ground), "accent on ground").toBeGreaterThanOrEqual(EDGE_CONTRAST);
  expect(contrast(p.alert, p.ground), "alert on ground").toBeGreaterThanOrEqual(EDGE_CONTRAST);
  expect(contrast(p.warn, p.ground), "warn on ground").toBeGreaterThanOrEqual(EDGE_CONTRAST);
}

const lum = (h: string) => contrast(h, "#000000"); // tokens-exempt: black is the reference for "how light is this", not a colour drawn

describe("a palette made from a picture (#131)", () => {
  it("a dark picture keeps its dark field and gets light words", () => {
    const { palette } = paletteFromPixels(
      picture([
        { rgb: [12, 14, 28], share: 0.7 }, // night sky
        { rgb: [60, 220, 240], share: 0.15 }, // neon cyan
        { rgb: [240, 60, 170], share: 0.1 }, // neon pink
        { rgb: [230, 230, 235], share: 0.05 }, // a streetlight
      ]),
    );
    legible(palette);
    expect(lum(palette.ground)).toBeLessThan(2);
    expect(lum(palette.text)).toBeGreaterThan(lum(palette.ground));
  });

  // Dark-only is about what ships, not what people make: a bright photo gets a
  // bright skin, with dark words, and nothing drags it into the dark.
  it("a bright picture stays bright and gets dark words", () => {
    const { palette } = paletteFromPixels(
      picture([
        { rgb: [250, 244, 230], share: 0.65 }, // paper
        { rgb: [210, 40, 40], share: 0.15 }, // a red stamp
        { rgb: [40, 90, 200], share: 0.12 }, // blue ink
        { rgb: [30, 30, 30], share: 0.08 }, // type
      ]),
    );
    legible(palette);
    expect(lum(palette.ground)).toBeGreaterThan(10);
    expect(lum(palette.text)).toBeLessThan(lum(palette.ground));
  });

  it("a mid-grey picture with one flat colour still gets six readable roles", () => {
    const { palette } = paletteFromPixels(picture([{ rgb: [128, 128, 128], share: 1 }]));
    legible(palette);
    // Six roles, not six copies of grey: the picture had nothing to offer, so
    // the colours that stand off it are made.
    expect(new Set(TOKENS.map((t) => palette[t])).size).toBeGreaterThanOrEqual(5);
  });

  // Found on the repo's own capybara art: a subject on a transparent ground
  // made its orange coat the ground, and every accent went dark to stand off
  // it. A loud picture gets a calm ground so its loud colours stay loud.
  it("a picture that is mostly one vivid colour gets a calm ground and keeps its colours bright", () => {
    const { palette } = paletteFromPixels(
      picture([
        { rgb: [224, 121, 60], share: 0.55 }, // an orange coat
        { rgb: [240, 200, 40], share: 0.2 }, // a yellow jacket
        { rgb: [60, 180, 230], share: 0.15 }, // water
        { rgb: [230, 40, 40], share: 0.1 }, // a red board
      ]),
    );
    legible(palette);
    expect(chromaOf(palette.ground)).toBeLessThan(0.06);
    expect(chromaOf(palette.accent)).toBeGreaterThan(0.1);
    expect(chromaOf(palette.alert)).toBeGreaterThan(0.08);
  });

  it("no two of the words, accent, alert and warning are the same colour", () => {
    const { palette } = paletteFromPixels(
      picture([
        { rgb: [30, 14, 12], share: 0.45 }, // a dark field
        { rgb: [232, 25, 27], share: 0.25 }, // a red cooler
        { rgb: [231, 206, 147], share: 0.2 }, // a cream jacket
        { rgb: [150, 90, 50], share: 0.1 }, // brown fur
      ]),
    );
    legible(palette);
    const roles = [palette.text, palette.accent, palette.alert, palette.warn];
    expect(new Set(roles).size).toBe(4);
  });

  it("the ground is the colour the picture has most of", () => {
    const { palette } = paletteFromPixels(
      picture([
        { rgb: [30, 110, 50], share: 0.6 }, // a lawn
        { rgb: [240, 200, 40], share: 0.4 }, // a yellow ball
      ]),
    );
    const g = parseInt(palette.ground.slice(1), 16);
    // Green dominates the ground's channels, as the lawn does the picture.
    expect((g >> 8) & 255).toBeGreaterThan((g >> 16) & 255);
    legible(palette);
  });

  it("ignores transparent pixels: a cut-out's empty corners are not its colour", () => {
    const cutout = paletteFromPixels(
      picture([
        { rgb: [255, 255, 255], share: 0.8, alpha: 0 }, // empty, and would read as white
        { rgb: [150, 60, 200], share: 0.2 }, // the purple subject
      ]),
    );
    const subjectOnly = paletteFromPixels(picture([{ rgb: [150, 60, 200], share: 1 }]));
    expect(cutout.palette.ground).toBe(subjectOnly.palette.ground);
  });

  it("the same picture always makes the same skin", () => {
    const px = picture([
      { rgb: [20, 20, 40], share: 0.5 },
      { rgb: [200, 120, 30], share: 0.3 },
      { rgb: [90, 200, 120], share: 0.2 },
    ]);
    expect(paletteFromPixels(px)).toEqual(paletteFromPixels(px));
  });

  it("an empty picture falls back to a dark, legible default rather than failing", () => {
    const { palette } = paletteFromPixels(new Uint8ClampedArray(0));
    legible(palette);
  });

  it("the analyser's ramp is 24 steps from the surface to the alert", () => {
    const { palette, viscolor } = paletteFromPixels(
      picture([
        { rgb: [10, 10, 20], share: 0.7 },
        { rgb: [80, 230, 255], share: 0.2 },
        { rgb: [255, 80, 200], share: 0.1 },
      ]),
    );
    expect(viscolor).toHaveLength(24);
    for (const c of viscolor) expect(c).toMatch(/^#[0-9A-F]{6}$/);
    expect(viscolor[23]).toBe(palette.alert);
    expect(rampFrom(palette)).toEqual(viscolor);
  });
});
