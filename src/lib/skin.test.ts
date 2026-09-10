/// <reference types="node" />
// The reference is for this file alone: the app's tsconfig has no node
// types, and the test reads the shipped PNGs' headers straight off disk.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import tokens from "../../design/tokens.json";
import eyewall from "../../skins/eyewall/manifest.json";
import {
  checkSheetBounds,
  elementsOf,
  parseSkin,
  placeRect,
  sheetFor,
  SkinError,
  sprites,
  TOKENS,
  WINDOWS,
} from "./skin";

const SKIN_DIR = new URL("../../skins/eyewall/", import.meta.url);

/** Width and height from a PNG's IHDR chunk. Enough to check bounds without
 * decoding, which is the one thing a test runner without a browser can do. */
function pngSize(file: string): { w: number; h: number } {
  const b = readFileSync(new URL(file, SKIN_DIR));
  if (b.toString("latin1", 1, 4) !== "PNG") throw new Error(`${file} is not a PNG`);
  return { w: b.readUInt32BE(16), h: b.readUInt32BE(20) };
}

/** A structurally clone of the shipped manifest to break in one place. */
function broken(mutate: (m: any) => void): unknown {
  const m = JSON.parse(JSON.stringify(eyewall));
  mutate(m);
  return m;
}

describe("the Eyewall manifest", () => {
  it("parses with no errors and no warnings", () => {
    const { skin, warnings } = parseSkin(eyewall);
    expect(warnings).toEqual([]);
    expect(skin.name).toBe("Eyewall");
    expect(skin.art).toBe("mask");
    expect(skin.glow).toBe("renderer");
    expect(skin.authoredScale).toBe(1);
  });

  it("carries the six tokens and the ramp from design/tokens.json, unchanged", () => {
    const { skin } = parseSkin(eyewall);
    const eye = tokens.themes.eyewall;
    for (const t of TOKENS) expect(skin.palette[t]).toBe((eye.colors as Record<string, string>)[t]);
    expect(skin.viscolor).toEqual(eye.visualizer.palette);
    expect(skin.visualizer.component).toBe(eye.visualizer.component);
  });

  it("describes all three windows with a shade strip each", () => {
    const { skin } = parseSkin(eyewall);
    for (const w of WINDOWS) {
      expect(skin.windows[w].size).toEqual([275, 116]);
      expect(skin.windows[w].shade.size).toEqual([275, 14]);
      expect(elementsOf(skin, w, false).elements.some((e) => e.name === "titlebar")).toBe(true);
      expect(elementsOf(skin, w, true).elements.some((e) => e.name === "titlebar")).toBe(true);
    }
    expect(skin.windows.playlist.resizable).toBe(true);
    expect(skin.windows.playlist.resizeStep).toEqual([25, 29]);
    expect(skin.windows.main.resizable).toBe(false);
  });

  it("puts the shade toggle on every title bar (#8) and minimise on Main's only (D86)", () => {
    const { skin } = parseSkin(eyewall);
    for (const w of WINDOWS) {
      for (const shaded of [false, true]) {
        const names = elementsOf(skin, w, shaded).elements.map((e) => e.name);
        expect(names).toContain("shade");
        expect(names).toContain("zoom");
        expect(names.includes("minimize")).toBe(w === "main");
      }
    }
  });

  it("ships a sheet at 1x and 2x, and every sprite lies inside both", () => {
    const { skin } = parseSkin(eyewall);
    const sizes: Record<string, { w: number; h: number }> = {};
    for (const per of Object.values(skin.sheets)) {
      for (const file of Object.values(per)) sizes[file] = pngSize(file);
    }
    expect(Object.keys(sizes).length).toBe(2);
    const one = sheetFor(skin, "chrome", 1);
    const two = sheetFor(skin, "chrome", 2);
    expect(one.scale).toBe(1);
    expect(two.scale).toBe(2);
    expect(sizes[two.file].w).toBe(sizes[one.file].w * 2);
    expect(sizes[two.file].h).toBe(sizes[one.file].h * 2);
    expect(checkSheetBounds(skin, sizes)).toEqual([]);
    expect(sprites(skin).length).toBeGreaterThan(20);
  });
});

describe("parseSkin refuses rather than half-loads", () => {
  const refuse = (mutate: (m: any) => void, message: RegExp) => {
    expect(() => parseSkin(broken(mutate))).toThrow(SkinError);
    expect(() => parseSkin(broken(mutate))).toThrow(message);
  };

  it("the wrong format", () => refuse((m) => (m.format = "hp-skin/9"), /format/));
  it("a missing window", () => refuse((m) => delete m.windows.equalizer, /windows\.equalizer/));
  it("a missing shade strip", () => refuse((m) => delete m.windows.main.shade, /shade.*required/));
  it("a shade strip of the wrong size", () =>
    refuse((m) => (m.windows.main.shade.size = [275, 20]), /shade\.size/));
  it("a window of the wrong size", () => refuse((m) => (m.windows.eq = m.windows.main.size = [300, 116]), /size/));
  it("an undeclared resizable", () => refuse((m) => delete m.windows.playlist.resizable, /resizable/));
  it("the wrong playlist step (D30)", () => refuse((m) => (m.windows.playlist.resizeStep = [10, 10]), /D30/));
  it("an unknown action", () => refuse((m) => (m.windows.main.elements.shade.action = "explode"), /action/));
  it("a sprite naming no sheet", () =>
    refuse((m) => (m.windows.main.elements.shade.sprite.sheet = "nope"), /names no sheet/));
  it("a rect with three numbers", () =>
    refuse((m) => (m.windows.main.elements.shade.sprite.rect = [0, 0, 13]), /rect/));
  it("a rect with a zero size", () =>
    refuse((m) => (m.windows.main.elements.shade.sprite.rect = [0, 0, 0, 9]), /rect\[2\]/));
  it("a text naming no font", () => refuse((m) => (m.windows.main.elements.title.font = "serif"), /names no font/));
  it("a missing token", () => refuse((m) => delete m.palette.ember, /palette\.ember/));
  it("a colour that is not a hex", () => refuse((m) => (m.palette.arc = "cyan"), /RRGGBB/));
  it("a ramp of the wrong length", () => refuse((m) => m.viscolor.pop(), /24/));
  it("a title bar that is not the drag handle", () =>
    refuse((m) => delete m.windows.main.elements.titlebar.role, /titlebar.*drag/));
  it("a tint outside the six", () =>
    refuse((m) => (m.windows.main.elements.titlebar.sprite.tint = "cyan"), /tint/));
});

describe("parseSkin warns on the soft cases", () => {
  it("an unknown bind renders empty", () => {
    const { skin, warnings } = parseSkin(broken((m) => (m.windows.main.elements.title.bind = "mood")));
    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toMatch(/mood/);
    const title = elementsOf(skin, "main", false).elements.find((e) => e.name === "title");
    expect(title?.type === "text" && title.bind).toBeNull();
  });

  it("an unknown visualizer falls back to bars", () => {
    const { skin, warnings } = parseSkin(broken((m) => (m.visualizer.component = "kaleidoscope")));
    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toMatch(/kaleidoscope/);
    expect(skin.visualizer.component).toBe("spectrum-bars");
  });

  it("parses a list element, which the playlist's PR will draw", () => {
    const { skin, warnings } = parseSkin(
      broken((m) => (m.windows.playlist.elements.rows = { type: "list", rect: [12, 20, 243, 58], rowHeight: 10 })),
    );
    expect(warnings).toEqual([]);
    const rows = elementsOf(skin, "playlist", false).elements.find((e) => e.name === "rows");
    expect(rows?.type === "list" && rows.rowHeight).toBe(10);
  });

  it("ignores unknown keys", () => {
    const { warnings } = parseSkin(broken((m) => (m.windows.main.elements.title.glitter = true)));
    expect(warnings).toEqual([]);
  });

  it("defaults tint to filament and art to final", () => {
    const { skin } = parseSkin(
      broken((m) => {
        delete m.art;
        delete m.glow;
        delete m.windows.main.elements.titlebar.sprite.tint;
      }),
    );
    expect(skin.art).toBe("final");
    expect(skin.glow).toBe("baked");
    const bar = elementsOf(skin, "main", false).elements.find((e) => e.name === "titlebar");
    expect(bar?.type === "image" && bar.sprite.tint).toBe("filament");
  });
});

describe("sheets per scale", () => {
  it("a plain file name is the sheet at authoredScale", () => {
    const { skin } = parseSkin(broken((m) => (m.sheets.chrome = "chrome.png")));
    expect(sheetFor(skin, "chrome", 1)).toEqual({ file: "chrome.png", scale: 1 });
    expect(sheetFor(skin, "chrome", 2)).toEqual({ file: "chrome.png", scale: 1 });
  });

  it("a 2x-only skin serves its sheet at every ratio, scaled", () => {
    const { skin } = parseSkin(
      broken((m) => {
        m.authoredScale = 2;
        m.sheets.chrome = "chrome@2x.png";
      }),
    );
    expect(sheetFor(skin, "chrome", 1)).toEqual({ file: "chrome@2x.png", scale: 2 });
    expect(sheetFor(skin, "chrome", 1.5)).toEqual({ file: "chrome@2x.png", scale: 2 });
  });

  it("bounds are checked in the sheet's own pixels", () => {
    const { skin } = parseSkin(eyewall);
    const one = pngSize("chrome.png");
    const tooSmall = checkSheetBounds(skin, { "chrome.png": one, "chrome@2x.png": { w: one.w * 2, h: 100 } });
    expect(tooSmall.length).toBeGreaterThan(0);
    expect(tooSmall.every((p) => p.includes("2x"))).toBe(true);
    const huge = checkSheetBounds(skin, {
      "chrome.png": { w: 5000, h: one.h },
      "chrome@2x.png": { w: one.w * 2, h: one.h * 2 },
    });
    expect(huge).toHaveLength(1);
    expect(huge[0]).toMatch(/over the cap/);
  });
});

describe("placeRect", () => {
  const base: [number, number] = [275, 116];
  it("leaves a plain rect alone in a bigger window", () => {
    expect(placeRect({ name: "a", rect: [4, 0, 216, 14] }, base, [300, 145])).toEqual({ x: 4, y: 0, w: 216, h: 14 });
  });
  it("moves a right-anchored rect with the right edge", () => {
    expect(placeRect({ name: "a", rect: [258, 2, 13, 9], anchor: "right" }, base, [300, 116])).toEqual({
      x: 283,
      y: 2,
      w: 13,
      h: 9,
    });
  });
  it("grows a stretched rect along the axes named", () => {
    expect(placeRect({ name: "a", rect: [0, 0, 275, 14], stretch: "x" }, base, [325, 174])).toEqual({
      x: 0,
      y: 0,
      w: 325,
      h: 14,
    });
    expect(placeRect({ name: "a", rect: [0, 0, 275, 116], stretch: "xy" }, base, [325, 174])).toEqual({
      x: 0,
      y: 0,
      w: 325,
      h: 174,
    });
  });
  it("is the identity at base size", () => {
    expect(placeRect({ name: "a", rect: [258, 2, 13, 9], anchor: "right", stretch: "xy" }, base, base)).toEqual({
      x: 258,
      y: 2,
      w: 13,
      h: 9,
    });
  });
});
