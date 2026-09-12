import { describe, expect, it } from "vitest";
import eyewall from "../../skins/eyewall/manifest.json";
import { colorsWorn } from "./theme";
import { madeManifest, MAKER_VERSION, nameFrom, PICTURE_OPACITY } from "./madeskin";
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
    const { skin } = parseSkin(madeManifest(made()));
    const bands = WINDOWS.map((w) => {
      const b = elementsOf(skin, w, false).elements[0] as { sprite: { rect: number[] }; opacity: number; stretch?: string };
      expect(b.opacity).toBe(PICTURE_OPACITY);
      return b;
    });
    expect(bands.map((b) => b.sprite.rect[1])).toEqual([0, 116, 232]);
    // The playlist grows, so its third of the picture has to grow with it.
    expect(bands[2].stretch).toBe("xy");
    expect(bands[0].stretch).toBeUndefined();
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
