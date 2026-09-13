import { describe, expect, it } from "vitest";
import eyewall from "../../skins/eyewall/manifest.json";
import { colorsWorn } from "./theme";
import {
  canMove,
  DISPLAY_WELL,
  madeManifest,
  remade,
  MAKER_VERSION,
  nameFrom,
  PICTURE_H,
  PICTURE_MAX_H,
  PICTURE_OPACITY,
  pictureFor,
  pictureOf,
  PLACES,
  QUIET_FLOOR,
  windowsTop,
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
    const { skin } = parseSkin(madeManifest({ ...made(), picture: pictureFor(275, 900) }));
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
    expect(pictureFor(1000, 2000)).toEqual({ sheet: 550, top: 0, height: 550, at: "top" });
    // A wide landscape keeps its height too, rather than being scaled up to
    // three windows and cropped at the sides, and gets clear sheet above and
    // below it, as much as the windows are taller than it (D127).
    expect(pictureFor(4000, 1000)).toEqual({ sheet: 627, top: 279, height: 69, at: "top" });
    // A very tall strip stops at the cap.
    expect(pictureFor(100, 100000)).toEqual({ sheet: PICTURE_MAX_H, top: 0, height: PICTURE_MAX_H, at: "top" });
    // Nothing to measure is not a crash.
    expect(pictureFor(0, 0)).toEqual({ sheet: PICTURE_H, top: 0, height: PICTURE_H, at: "top" });
  });

  // The owner's square pictures covered Main and the equalizer and left most
  // of the playlist bare, with nowhere else for them to go (D127).
  it("puts the windows at the top, middle or bottom of a picture", () => {
    const bands = (at: (typeof PLACES)[number], picture = pictureFor(1000, 1000)) => {
      const { skin } = parseSkin(madeManifest({ ...made(), picture: { ...picture, at } }));
      return WINDOWS.map((w) => (elementsOf(skin, w, false).elements[0] as { sprite: { rect: number[] } }).sprite.rect);
    };
    // A square picture is 275 tall at the windows' width, with 73 clear above
    // and below it: the sheet is 421.
    const square = pictureFor(1000, 1000);
    expect(square).toMatchObject({ sheet: 421, top: 73, height: 275 });
    // Top: the picture starts where Main does.
    expect(bands("top").map((r) => r[1])).toEqual([73, 189, 305]);
    // Middle: the picture's centre is the three windows' centre, to a pixel.
    const mid = windowsTop({ ...square, at: "middle" });
    expect(Math.abs(mid + PICTURE_H / 2 - (square.top + square.height / 2))).toBeLessThanOrEqual(0.5);
    expect(bands("middle").map((r) => r[1])).toEqual([mid, mid + 116, mid + 232]);
    // Bottom: it ends where the playlist does at its base height.
    expect(bands("bottom").map((r) => r[1])).toEqual([0, 116, 232]);
    // Every band stays inside the sheet, and the playlist's runs to its end.
    for (const at of PLACES) {
      const b = bands(at);
      for (const r of b) expect(r[1] + r[3]).toBeLessThanOrEqual(square.sheet);
      expect(b[2][1] + b[2][3]).toBe(square.sheet);
    }
    // A tall picture moves the other way: the windows slide down it, and the
    // playlist still has the rest of it below to reveal.
    const tall = pictureFor(275, 900);
    expect(bands("top", tall)[0][1]).toBe(0);
    expect(bands("middle", tall)[0][1]).toBe(276);
    expect(bands("bottom", tall)[0][1]).toBe(552);
    expect(bands("bottom", tall)[2]).toEqual([0, 784, 275, 116]);
  });

  it("offers a place only where the places differ", () => {
    expect(canMove(pictureFor(1000, 1000))).toBe(true);
    expect(canMove(pictureFor(275, 900))).toBe(true);
    // Exactly three windows tall looks the same at every place.
    expect(canMove(pictureFor(275, PICTURE_H))).toBe(false);
  });

  it("reads back where the picture is, and reads a skin made before that as it was drawn", () => {
    const m = madeManifest({ ...made(), picture: { ...pictureFor(4000, 1000), at: "middle" } });
    expect(pictureOf(JSON.parse(JSON.stringify(m)))).toEqual({ sheet: 627, top: 279, height: 69, at: "middle" });
    // Before D127: no record, the picture from the top of a sheet that ends
    // where the playlist's backdrop does, and no room to move a short one.
    const old = madeManifest({ ...made(), picture: pictureFor(0, 0) }) as Record<string, any>;
    delete old.picture;
    expect(pictureOf(old)).toEqual({ sheet: PICTURE_H, top: 0, height: PICTURE_H, at: "top" });
    expect(canMove(pictureOf(old))).toBe(false);
    // A record that does not match the sheet is not trusted.
    const lying = JSON.parse(JSON.stringify(m));
    lying.picture.sheet = 1200;
    expect(pictureOf(lying)).toMatchObject({ top: 0, at: "top" });
  });

  it("moves the picture without touching anything else", () => {
    const m = madeManifest({ ...made(), picture: pictureFor(1000, 1000) }) as Record<string, any>;
    const written = JSON.parse(JSON.stringify(m));
    const moved = remade(written, { ...pictureOf(written), at: "bottom" }) as Record<string, any>;
    expect(moved.picture.at).toBe("bottom");
    expect(moved.windows.main.elements.backdrop.sprite.rect[1]).toBe(0);
    expect(moved.palette).toEqual(m.palette);
    expect(moved.sheets).toEqual(m.sheets);
    // And back.
    expect(remade(moved, { ...pictureOf(moved), at: "top" })).toEqual(m);
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

  // The rail was the one thing the floor could not reach while its 0.22 was
  // baked into the sheet (D126).
  it("lifts the EQ rail over the floor, and keeps it quieter than the fill", () => {
    const { skin } = parseSkin(madeManifest(made()));
    const sliders = skin.windows.equalizer.full.elements.filter((e) => e.type === "slider");
    expect(sliders).toHaveLength(11);
    for (const s of sliders) {
      if (s.type !== "slider") continue;
      expect(s.trackOpacity).toBeGreaterThanOrEqual(QUIET_FLOOR);
      expect(s.trackOpacity).toBeLessThan(1);
    }
  });

  it("is made again, unchanged, from its own manifest", () => {
    const m = madeManifest({ ...made(), picture: { ...pictureFor(1000, 500), at: "middle" } });
    expect(remade(JSON.parse(JSON.stringify(m)))).toEqual(m);
  });

  // SEL was the first thing Eyewall gained that made skins never got: they
  // copied Eyewall's layout when they were made (D124).
  it("picks up what Eyewall has gained since the skin was made", () => {
    const old = madeManifest({ ...made(), picture: pictureFor(275, 500) }) as Record<string, any>;
    delete old.windows.playlist.elements.selectButton;
    delete old.windows.playlist.elements.selectWell;
    const again = remade(old) as Record<string, any>;
    expect(again.windows.playlist.elements.selectButton).toMatchObject({ action: "loadSelected" });
    // And keeps what the maker chose: the colours, and how tall its picture is.
    expect(again.palette).toEqual(old.palette);
    const band = again.windows.playlist.elements.backdrop.sprite.rect;
    expect(band[1] + band[3]).toBe(500);
    expect(() => parseSkin(again)).not.toThrow();
  });

  it("leaves anything that is not a made skin alone", () => {
    expect(remade(eyewall as Record<string, unknown>)).toBeNull();
    expect(remade({ generator: 4, name: "an import" })).toBeNull();
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
