/// <reference types="node" />
// The reference is for this file alone: the app's tsconfig has no node
// types, and the test reads the shipped PNGs' headers straight off disk.
import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";
import { describe, expect, it } from "vitest";
import tokens from "../../design/tokens.json";
import eyewall from "../../skins/eyewall/manifest.json";
import {
  checkSheetBounds,
  elementsOf,
  parseSkin,
  placeRect,
  type Rect,
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

/**
 * Every pixel's alpha from one of the shipped sheets. A minimal PNG reader,
 * test-only: the sheets are what tools/chrome-sheet.ps1 writes, 8-bit RGBA
 * and not interlaced, and anything else is refused rather than misread. The
 * art is generated, so its pixels are the thing worth checking; a sheet can
 * be the right size and still carry the wrong glow (#117).
 */
function alphaOf(file: string): (x: number, y: number) => number {
  const b = readFileSync(new URL(file, SKIN_DIR));
  const idat: Buffer[] = [];
  let w = 0;
  let h = 0;
  for (let p = 8; p < b.length; ) {
    const len = b.readUInt32BE(p);
    const type = b.toString("latin1", p + 4, p + 8);
    const data = b.subarray(p + 8, p + 8 + len);
    if (type === "IHDR") {
      w = data.readUInt32BE(0);
      h = data.readUInt32BE(4);
      if (data[8] !== 8 || data[9] !== 6 || data[12] !== 0) throw new Error(`${file}: not 8-bit RGBA, non-interlaced`);
    } else if (type === "IDAT") idat.push(data);
    else if (type === "IEND") break;
    p += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = w * 4;
  const px = Buffer.alloc(h * stride);
  for (let y = 0; y < h; y++) {
    const filter = raw[y * (stride + 1)];
    const line = y * (stride + 1) + 1;
    for (let x = 0; x < stride; x++) {
      const a = x >= 4 ? px[y * stride + x - 4] : 0;
      const up = y > 0 ? px[(y - 1) * stride + x] : 0;
      const ul = x >= 4 && y > 0 ? px[(y - 1) * stride + x - 4] : 0;
      let v = raw[line + x];
      if (filter === 1) v += a;
      else if (filter === 2) v += up;
      else if (filter === 3) v += (a + up) >> 1;
      else if (filter === 4) {
        const pa = Math.abs(up - ul);
        const pb = Math.abs(a - ul);
        const pc = Math.abs(a + up - 2 * ul);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? up : ul;
      }
      px[y * stride + x] = v & 0xff;
    }
  }
  return (x, y) => px[(y * w + x) * 4 + 3];
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

  it("puts shade on every title bar (#8); minimise, 2x and close are Main's (D63, D86, D96)", () => {
    const { skin } = parseSkin(eyewall);
    for (const w of WINDOWS) {
      for (const shaded of [false, true]) {
        const els = elementsOf(skin, w, shaded).elements;
        const names = els.map((e) => e.name);
        // Shade is per window: collapse the EQ and keep the playlist open.
        expect(names).toContain("shade");
        // 2x is one app-wide setting, so it has one home (D96).
        expect(names.includes("zoom")).toBe(w === "main");
        // A satellite refuses to close (D63), so it is offered no way to.
        expect(names.includes("minimize")).toBe(w === "main");
        expect(names.includes("close")).toBe(w === "main");
        // The rightmost title-bar button sits flush at the same edge on all
        // three, so the buttons line up down a bonded stack.
        const right = Math.max(...els.filter((e) => e.type === "button" || e.type === "toggle")
          .filter((e) => e.rect[1] < 14).map((e) => e.rect[0] + e.rect[2]));
        expect(right).toBe(271);
      }
    }
  });

  it("draws Main's interior: clock, tags, visualizer, strip, seek, transport, volume", () => {
    const { skin } = parseSkin(eyewall);
    const els = elementsOf(skin, "main", false).elements;
    const by = (n: string) => els.find((e) => e.name === n);

    // Every rectangle here was measured off the CSS chrome that shipped
    // (D90), so the window looks the same drawn from sprites.
    expect(by("clock")).toMatchObject({ type: "text", rect: [4, 31, 56, 17], bind: "elapsed", glow: true });
    expect(by("vis")).toMatchObject({ type: "visualizer", rect: [65, 18, 206, 41] });
    expect(by("trackTitle")).toMatchObject({ type: "text", bind: "trackTitle", overflow: "scroll" });
    expect(by("seek")).toMatchObject({ type: "slider", bind: "position", orientation: "horizontal" });
    expect(by("volume")).toMatchObject({ type: "slider", bind: "volume" });

    // The five transport controls, in the order a hand reaches for them.
    for (const [name, action] of [
      ["prev", "prev"],
      ["play", "play"],
      ["pause", "pause"],
      ["stop", "stop"],
      ["next", "next"],
    ] as const) {
      expect(by(name)).toMatchObject({ action });
    }
    // The three that latch read the transport rather than their own click.
    for (const [name, when] of [
      ["play", "playing"],
      ["pause", "paused"],
      ["stop", "stopped"],
    ] as const) {
      expect(by(name)).toMatchObject({ type: "toggle", bind: "playState", when });
    }
    // The tags light on the same state, and STOP lights a different colour.
    expect(by("tagPlay")).toMatchObject({ value: "PLAY", lit: { bind: "playState", when: "playing", tint: "arc" } });
    expect(by("tagStop")).toMatchObject({ lit: { when: "stopped", tint: "strike" } });
    // A literal with a hole in it, so "VOL" and the number are one element.
    expect(by("volLabel")).toMatchObject({ value: "VOL {}", bind: "volumePercent" });
  });

  it("keeps the latched glow on the transport and off the title bar's toggles (#117)", () => {
    // The pixel just inside the ring, top left: no glyph ever reaches it, so
    // it is transparent unless something filled the box.
    const { skin } = parseSkin(eyewall);
    const inside = (alpha: (x: number, y: number) => number, rect: Rect, k: number) =>
      alpha((rect[0] + 1) * k, (rect[1] + 1) * k);
    for (const [file, k] of [
      ["chrome.png", 1],
      ["chrome@2x.png", 2],
    ] as const) {
      const alpha = alphaOf(file);
      let titleToggles = 0;
      for (const w of WINDOWS) {
        for (const shaded of [false, true]) {
          for (const e of elementsOf(skin, w, shaded).elements) {
            if (e.type !== "toggle") continue;
            if (e.action === "shade" || e.action === "zoom") {
              // A title-bar toggle's "on" is its other glyph (1x, the up
              // arrow), drawn as plainly as its "off".
              titleToggles++;
              expect(inside(alpha, e.sprite.rect, k), `${w} ${e.name} off at ${k}x`).toBe(0);
              expect(inside(alpha, e.on.sprite.rect, k), `${w} ${e.name} on at ${k}x`).toBe(0);
            } else if (e.bind === "playState") {
              // The control that proves the check can see a glow at all:
              // a transport button latched on is filled.
              expect(inside(alpha, e.on.sprite.rect, k), `${e.name} lit at ${k}x`).toBeGreaterThan(0);
              expect(inside(alpha, e.sprite.rect, k), `${e.name} unlit at ${k}x`).toBe(0);
            }
          }
        }
      }
      expect(titleToggles).toBeGreaterThan(0);
    }
  });

  it("draws the EQ's interior: eleven centred vertical sliders, their scale, the switch, the lamp (D98)", () => {
    const { skin } = parseSkin(eyewall);
    const els = elementsOf(skin, "equalizer", false).elements;
    const by = (n: string) => els.find((e) => e.name === n);
    const sliders = els.filter((e) => e.type === "slider");
    expect(sliders.map((s) => s.type === "slider" && s.bind)).toEqual([
      "eqPre",
      ...Array.from({ length: 10 }, (_, i) => `eqBand${i + 1}`),
    ]);
    for (const s of sliders) {
      if (s.type !== "slider") continue;
      // Centred on 0 dB, so the fill runs from the line, the wheel nudges
      // and a double press returns there.
      expect(s).toMatchObject({ orientation: "vertical", origin: 0.5 });
      // Past 8 dB either way the thumb turns strike; while the EQ is off
      // the whole slider dims, and the dim wins.
      expect(s.hot).toEqual({ beyond: 0.34, tint: "strike" });
      // Past 8 dB, not at it: a band at exactly 8 (two presets have one)
      // stays arc, as the CSS thumb did with `Math.abs(db) > 8`.
      expect(8 / 24).toBeLessThan(s.hot!.beyond);
      expect(8.5 / 24).toBeGreaterThan(s.hot!.beyond);
      // An odd height too, so 0 dB lands on a pixel's middle, where the tick is.
      expect(s.rect[3] % 2).toBe(1);
      expect(s.lit).toMatchObject({ bind: "eqOn", when: "off", tint: "filament" });
      // An odd width, so a 9-pixel thumb sits on whole pixels at 1x.
      expect(s.rect[2] % 2).toBe(1);
    }
    // The scale under them, centred, "1k" left in lower case.
    expect(by("eqBand5Label")).toMatchObject({ type: "text", value: "1k", align: "center" });
    expect(by("eqPreLabel")).toMatchObject({ value: "PRE", opacity: 0.65 });
    // The switch clicks and lights; the lamp only lights.
    expect(by("eqOnButton")).toMatchObject({ type: "toggle", action: "eqOn", bind: "eqOn", when: "on" });
    expect(by("eqClipLamp")).toMatchObject({ type: "toggle", action: null, bind: "eqClip", when: "on" });
    expect(by("eqPresetButton")).toMatchObject({ action: "eqPresets", bind: "eqMenu", when: "open" });
    // The curve is the window's to draw, in the box the skin gives it.
    expect(by("eqCurveWell")).toMatchObject({ type: "image", rect: [4, 42, 60, 55] });
  });

  it("reuses one ring and one solid at the strengths each box wants (D93)", () => {
    const { skin } = parseSkin(eyewall);
    const els = elementsOf(skin, "main", false).elements;
    const frame = els.find((e) => e.name === "frame");
    const strip = els.find((e) => e.name === "stripFrame");
    expect(frame).toMatchObject({ type: "nineslice", fill: true, opacity: 0.3 });
    expect(strip).toMatchObject({ type: "nineslice", fill: false, rect: [4, 63, 267, 17], opacity: 0.14 });
    // The same eight-by-eight sprite, drawn at two strengths.
    expect(frame?.type === "nineslice" && frame.sprite.rect).toEqual(
      strip?.type === "nineslice" && strip.sprite.rect,
    );
    for (const name of ["stripWell", "seekWell", "volWell"]) {
      expect(els.find((e) => e.name === name)).toMatchObject({ type: "image", sprite: { tint: "well" } });
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
  it("a toggle that neither clicks nor watches a binding", () =>
    refuse((m) => {
      delete m.windows.main.elements.play.action;
      delete m.windows.main.elements.play.bind;
    }, /action.*indicator|indicator/));
  it("a text with nothing to say", () =>
    refuse((m) => {
      delete m.windows.main.elements.tagPlay.value;
      delete m.windows.main.elements.tagPlay.bind;
    }, /bind.*value|value/));
  it("a slider with no track, fill or thumb", () =>
    refuse((m) => {
      delete m.windows.main.elements.seek.fill;
      delete m.windows.main.elements.seek.thumb;
    }, /track.*fill.*thumb/));
  it("a slider with no binding", () =>
    refuse((m) => delete m.windows.main.elements.seek.bind, /bind.*required/));
  it("an opacity outside 0..1", () =>
    refuse((m) => (m.windows.main.elements.frame.opacity = 1.5), /between 0 and 1/));
  it("a lit block with no binding", () =>
    refuse((m) => delete m.windows.main.elements.tagPlay.lit.bind, /lit\.bind/));
  it("a slider origin outside 0..1", () =>
    refuse((m) => (m.windows.equalizer.elements.eqBand3.origin = 1.5), /origin.*between 0 and 1/));
  it("a hot zone on a slider with no centre to measure it from", () =>
    refuse((m) => delete m.windows.equalizer.elements.eqBand3.origin, /hot.*origin/));
  it("a text alignment the renderer does not know", () =>
    refuse((m) => (m.windows.equalizer.elements.eqPreLabel.align = "justify"), /align/));
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
