import { describe, expect, it } from "vitest";
import eyewall from "../../skins/eyewall/manifest.json";
import { colorsWorn } from "./theme";
import {
  DISPLAY_WELL,
  madeManifest,
  MAKER_VERSION,
  nameFrom,
  PICTURE_H,
  PICTURE_MAX_H,
  PICTURE_OPACITY,
  pictureHeightFor,
  QUIET_FLOOR,
} from "./madeskin";
import { elementsOf, parseSkin, TOKENS, WINDOWS } from "./skin";
import { paletteFromPixels } from "./palette";

/** A made palette, from a small synthetic picture: a dark field, two colours. */
function made() {
  const px = new Uint8ClampedArray(32 * 32 * 4);
  for (let i = 0; i < 32 * 32; i++) {
    const rgb = i < 700 ? [14, 12, 24] : i < 900 ? [230, 120, 40] : [70, 190, 230];
    px.set([...rgb, 255], i * 4);
  }
  const { palette, viscolor } = paletteFromPixels(px);
  return { name: "Storm Kitty", palette, viscolor };
}

describe("a skin made from a picture (#131)", () => {
  it("is a manifest the validator accepts, with no warnings", () => {
    const { warnings } = parseSkin(madeManifest(made()));
    expect(warnings).toEqual([]);
  });

  it("is Eyewall's chrome, complete, under the picture's name", () => {
    const { skin } = parseSkin(madeManifest(made()));
    expect(skin.name).toBe("Storm Kitty");
    expect(skin.art).toBe("mask");
    expect(skin.glow).toBe("renderer");
    // Every element Eyewall has, plus the backdrop: nothing is lost by making.
    const ew = parseSkin(eyewall).skin;
    for (const w of WINDOWS) {
      const names = elementsOf(skin, w, false).elements.map((e) => e.name);
      const ewNames = elementsOf(ew, w, false).elements.map((e) => e.name);
      expect(names).toEqual(["backdrop", ...ewNames]);
    }
  });

  // D122: mask art that asks for its own colours. Without it the windows
  // would paint the theme's palette and every made skin would look like
  // Eyewall with a faint photo behind it.
  it("paints the picture's colours, not the theme's", () => {
    const m = made();
    const { skin } = parseSkin(madeManifest(m));
    expect(skin.colors).toBe("own");
    const worn = colorsWorn(skin);
    for (const t of TOKENS) expect(worn[t]).toBe(m.palette[t]);
    expect(skin.viscolor).toEqual(m.viscolor);
  });

  it("draws the picture as a picture, and everything else as tinted mask", () => {
    const { skin } = parseSkin(madeManifest(made()));
    expect(skin.finalSheets).toEqual(["picture"]);
    expect(skin.sheets.picture).toEqual({ 1: "picture.png", 2: "picture@2x.png" });
  });

  it("lays one picture across the three stacked windows, as a wash under the chrome", () => {
    const { skin } = parseSkin(madeManifest({ ...made(), pictureHeight: 900 }));
    const bands = WINDOWS.map((w) => {
      const b = elementsOf(skin, w, false).elements[0] as {
        sprite: { rect: number[] };
        opacity: number;
        stretch?: string;
        fit: string;
      };
      expect(b.opacity).toBe(PICTURE_OPACITY);
      return b;
    });
    expect(bands.map((b) => b.sprite.rect[1])).toEqual([0, 116, 232]);
    // Main and the equalizer take their third and never change size.
    expect(bands[0].sprite.rect[3]).toBe(116);
    expect(bands[0].fit).toBe("stretch");
    // The owner dragged a made playlist taller and the picture stretched,
    // though the picture had more below. The playlist takes everything from
    // its third down, and reveals it as it grows rather than stretching.
    expect(bands[2].sprite.rect[3]).toBe(900 - 232);
    expect(bands[2].fit).toBe("reveal");
    expect(bands[2].stretch).toBe("xy");
  });

  it("keeps a picture at the windows' width and its own height, within bounds", () => {
    // A tall portrait keeps its height, so the playlist has something to reveal.
    expect(pictureHeightFor(1000, 2000)).toBe(550);
    // A wide landscape still gets a sheet three windows tall, so every
    // window's third exists; the picture fills its top at the windows' width
    // and the rest is clear (backdropPng), rather than being scaled up to
    // that height and cropped at the sides.
    expect(pictureHeightFor(4000, 1000)).toBe(PICTURE_H);
    // A very tall strip stops at the cap.
    expect(pictureHeightFor(100, 100000)).toBe(PICTURE_MAX_H);
    // Nothing to measure is not a crash.
    expect(pictureHeightFor(0, 0)).toBe(PICTURE_H);
  });

  // Seen on the first real made skin: Eyewall's quiet chrome (0.14 edges, 0.3
  // frames, 0.4 labels) vanished over a bright picture.
  it("lifts every piece of chrome over the floor, and keeps quiet things quieter than loud ones", () => {
    const opacities = (m: unknown, out: number[] = []): number[] => {
      if (Array.isArray(m)) m.forEach((v) => opacities(v, out));
      else if (m && typeof m === "object") {
        for (const [k, v] of Object.entries(m)) {
          if (k === "opacity" && typeof v === "number") out.push(v);
          else opacities(v, out);
        }
      }
      return out;
    };
    const made_ = madeManifest(made()) as { windows: Record<string, { elements: Record<string, any> }> };
    // The two displays are see-through on purpose (DISPLAY_WELL, its own test); every other piece
    // of chrome is held to the floor and to Eyewall's order.
    delete made_.windows.equalizer.elements.eqCurveWell.opacity;
    for (const w of Object.values(made_.windows)) {
      const { backdrop, ...chrome } = w.elements;
      expect((backdrop as { opacity: number }).opacity).toBe(PICTURE_OPACITY);
      for (const o of opacities(chrome)) expect(o).toBeGreaterThanOrEqual(QUIET_FLOOR);
    }
    // Order is kept: the same walk over Eyewall and over the made skin, pairwise.
    const before = opacities((eyewall as { windows: unknown }).windows);
    const after = opacities(
      Object.fromEntries(
        Object.entries(made_.windows).map(([k, w]) => {
          const { backdrop: _b, ...elements } = w.elements;
          return [k, { ...w, elements }];
        }),
      ),
    );
    expect(after).toHaveLength(before.length);
    for (let i = 0; i < before.length; i++) {
      for (let j = 0; j < before.length; j++) {
        if (before[i] < before[j]) expect(after[i]).toBeLessThanOrEqual(after[j]);
      }
    }
  });

  it("lets the picture show through the analyser and the EQ graph", () => {
    const { skin } = parseSkin(madeManifest(made()));
    const vis = elementsOf(skin, "main", false).elements.find((e) => e.name === "vis") as { well: number };
    const curve = elementsOf(skin, "equalizer", false).elements.find((e) => e.name === "eqCurveWell") as {
      opacity: number;
    };
    expect(vis.well).toBe(DISPLAY_WELL);
    expect(curve.opacity).toBe(DISPLAY_WELL);
    // Eyewall keeps its solid wells.
    const ew = parseSkin(eyewall).skin;
    expect((elementsOf(ew, "main", false).elements.find((e) => e.name === "vis") as { well: number }).well).toBe(1);
  });

  it("refuses a visualizer well outside 0..1", () => {
    const m = madeManifest(made()) as Record<string, any>;
    m.windows.main.elements.vis.well = 2;
    expect(() => parseSkin(m)).toThrow(/well/);
  });

  it("carries a maker stamp and no importer stamp, so it is never rebuilt as a .wsz", () => {
    const m = madeManifest(made());
    expect(m.maker).toBe(MAKER_VERSION);
    expect(m.generator).toBeUndefined();
  });

  it("names itself from the picture's file", () => {
    expect(nameFrom("C:\\pictures\\Storm Kitty.jpeg")).toBe("Storm Kitty");
    expect(nameFrom("/tmp/cat.final.png")).toBe("cat.final");
    expect(nameFrom("")).toBe("My skin");
  });
});

describe("whose colours a skin paints (D122)", () => {
  const palette = Object.fromEntries(TOKENS.map((t) => [t, "#123456"])) as Record<string, string>; // tokens-exempt: a stand-in palette to tell the skin's from the theme's

  it("defaults to D101: a final skin its own, a mask skin the theme's", () => {
    expect(colorsWorn({ art: "final", palette })).toEqual(palette);
    expect(colorsWorn({ art: "mask", palette })).not.toEqual(palette);
  });

  it("follows what the skin says when it says it", () => {
    expect(colorsWorn({ art: "mask", colors: "own", palette })).toEqual(palette);
    expect(colorsWorn({ art: "final", colors: "theme", palette })).not.toEqual(palette);
  });

  it("refuses a colours value that is not one of the two", () => {
    const m = madeManifest(made()) as Record<string, unknown>;
    m.colors = "rainbow";
    expect(() => parseSkin(m)).toThrow(/colors/);
  });

  it("refuses a sheet art value that is not one of the two", () => {
    const m = madeManifest(made()) as Record<string, any>;
    m.sheets.picture.art = "watercolour";
    expect(() => parseSkin(m)).toThrow(/art/);
  });
});
