// A skin made from a picture (#131). The maker writes no art: a made skin is
// Eyewall's own mask sheets (D73), painted in six colours found in the picture
// (`palette.ts`), with the picture itself behind the chrome. So a made skin is
// always complete — every button, slider and frame Eyewall has — and the only
// thing that can go wrong with one is its colours, which are made legible.

import eyewall from "../../skins/eyewall/manifest.json";
import type { Token } from "./skin";

/** Bumped when what the maker writes changes, the way `WSZ_GENERATION` is for
 * imports (D107). Its own key: a made skin must never be mistaken for an
 * import and "rebuilt" from art it never came from. */
export const MAKER_VERSION = 1;

/** The three classic windows stacked, which is how they usually sit: one
 * picture across all three rather than the same crop three times. */
export const PICTURE_W = 275;
export const PICTURE_H = 116 * 3;

/** The tallest the picture sheet is kept, at 1x: taller than a playlist is
 * likely to be dragged, so a tall picture is not cut short in use, and short
 * of a sheet that costs real memory to decode. */
export const PICTURE_MAX_H = 1600;

/** Which part of the picture the three stacked windows show (D127). */
export const PLACES = ["top", "middle", "bottom"] as const;
export type PicturePlace = (typeof PLACES)[number];

/**
 * Where a made skin's picture is, at 1x: the sheet's height, the band of it
 * the picture fills, and which part of that the windows show. Written into
 * the manifest as `picture`, so moving the picture is a manifest change and
 * never a redraw: the maker does not keep the picture it was given (D122),
 * only the sheet it drew.
 */
export type Picture = {
  sheet: number;
  top: number;
  height: number;
  at: PicturePlace;
};

/**
 * The sheet for a picture of this size.
 *
 * The picture is kept at the windows' width and its own proportions, so the
 * playlist can uncover more of it as it grows (D122). The first made skin
 * cropped it to exactly three windows and the playlist stretched that crop,
 * throwing away a tall picture's lower half. At most `PICTURE_MAX_H` tall.
 *
 * A picture shorter than the three windows gets clear space above and below
 * it, as much as the windows are taller than it, so they can sit with the
 * picture at their top, their middle or their bottom and always have sheet
 * under them (D127). Before, it filled the top of a sheet exactly three
 * windows tall and could be nowhere else, which left a wide picture's
 * playlist bare.
 */
export function pictureFor(width: number, height: number): Picture {
  if (!(width > 0 && height > 0)) return { sheet: PICTURE_H, top: 0, height: PICTURE_H, at: "top" };
  const h = Math.min(PICTURE_MAX_H, Math.max(1, Math.round((height * PICTURE_W) / width)));
  if (h >= PICTURE_H) return { sheet: h, top: 0, height: h, at: "top" };
  const room = PICTURE_H - h;
  return { sheet: h + 2 * room, top: room, height: h, at: "top" };
}

/** Where in the sheet the top of Main sits, for the picture's place. */
export function windowsTop(p: Picture): number {
  const y =
    p.at === "middle"
      ? p.top + Math.round((p.height - PICTURE_H) / 2)
      : p.at === "bottom"
        ? p.top + p.height - PICTURE_H
        : p.top;
  return Math.max(0, Math.min(p.sheet - PICTURE_H, y));
}

/** Whether the places differ at all: a picture exactly three windows tall
 * looks the same at every one, and so does one made before D127 that was no
 * taller than the windows, whose sheet has no room around it to move in. */
export function canMove(p: Picture): boolean {
  return p.height !== PICTURE_H;
}

/** How much of the picture shows through. The six colours are made legible
 * against the ground, not against a photograph, so the picture stays a wash
 * over the ground rather than competing with the words on it. 0.28 was tried
 * first, on a bright illustration, and lost the quiet chrome in its yellows. */
export const PICTURE_OPACITY = 0.22;

/**
 * The least any chrome is drawn at over a picture. Eyewall quiets its
 * secondary chrome with opacity — control edges at 0.14, frames at 0.3,
 * labels at 0.4 — which reads as calm on a flat ground and as nothing at all
 * over a photograph: the EQ's sliders became faint ticks and its band labels,
 * TRIM, CLIP and the playlist's bar buttons went unreadable. So a made skin
 * squeezes Eyewall's range into this floor and 1, which keeps the order —
 * quiet things stay quieter than loud ones — and loses only the whisper.
 */
export const QUIET_FLOOR = 0.55;

/**
 * How strongly the analyser's well and the EQ graph's paint over the picture.
 * They were solid, and the owner asked for the picture through them: the bars
 * and the curve are drawn at full strength in the accent, so a wash behind
 * them is enough to hold them, and the rest can be the picture.
 */
export const DISPLAY_WELL = 0.35;

/** Eyewall's quietest chrome, which lands on the floor. */
const EYEWALL_QUIETEST = 0.14;

const lift = (o: number) =>
  Math.min(1, Math.max(QUIET_FLOOR, QUIET_FLOOR + (o - EYEWALL_QUIETEST) * ((1 - QUIET_FLOOR) / (1 - EYEWALL_QUIETEST))));

/** Every `opacity` under a window, however deep — labels, lit and hot states
 * carry their own — lifted over the floor. */
function liftOpacities(node: unknown): void {
  if (Array.isArray(node)) {
    node.forEach(liftOpacities);
  } else if (node && typeof node === "object") {
    for (const [k, v] of Object.entries(node as Record<string, unknown>)) {
      if (k === "opacity" && typeof v === "number") (node as Record<string, unknown>)[k] = lift(v);
      else liftOpacities(v);
    }
  }
}

const WINDOW_BAND: Record<string, number> = { main: 0, equalizer: 1, playlist: 2 };

/** The hp-skin/1 manifest for a made skin: Eyewall's, renamed, repainted, and
 * with the picture laid in behind each window's chrome. */
export function madeManifest(made: {
  name: string;
  palette: Record<Token, string>;
  viscolor: string[];
  /** Where the picture is in its sheet, from `pictureFor`. */
  picture?: Picture;
}): Record<string, unknown> {
  const picture = made.picture ?? pictureFor(0, 0);
  const sheetH = picture.sheet;
  const y0 = windowsTop(picture);
  const m = JSON.parse(JSON.stringify(eyewall)) as Record<string, any>;
  m.name = made.name;
  m.author = "";
  m.maker = MAKER_VERSION;
  // Mask art asking for its own colours (D122): the picture's, not the theme's.
  m.colors = "own";
  m.palette = { ...made.palette };
  m.viscolor = [...made.viscolor];
  // Not part of hp-skin/1: the maker's own note of where its picture is, read
  // back by `remade` (D127). The validator ignores keys it does not know.
  m.picture = { ...picture };
  m.sheets = {
    ...m.sheets,
    picture: { "1": "picture.png", "2": "picture@2x.png", art: "final" },
  };
  // Before the backdrops go in: the picture's own opacity is not lifted.
  liftOpacities(m.windows);
  // The two displays let the picture through (after the lift, which would
  // otherwise raise them back up).
  m.windows.main.elements.vis.well = DISPLAY_WELL;
  m.windows.equalizer.elements.eqCurveWell.opacity = DISPLAY_WELL;
  for (const [win, band] of Object.entries(WINDOW_BAND)) {
    const w = m.windows[win];
    // Main and the equalizer never change size, so each takes its third. The
    // playlist grows, and takes everything from its third to the bottom of
    // the picture, revealed rather than stretched as it is dragged taller.
    const backdrop = w.resizable
      ? {
          type: "image",
          rect: [0, 0, 275, 116],
          stretch: "xy",
          fit: "reveal",
          sprite: { sheet: "picture", rect: [0, y0 + band * 116, PICTURE_W, sheetH - y0 - band * 116] },
          opacity: PICTURE_OPACITY,
        }
      : {
          type: "image",
          rect: [0, 0, 275, 116],
          sprite: { sheet: "picture", rect: [0, y0 + band * 116, PICTURE_W, 116] },
          opacity: PICTURE_OPACITY,
        };
    // First, so it is drawn under everything else in the window.
    w.elements = { backdrop, ...w.elements };
  }
  return m;
}

/**
 * A made skin, made again from what its manifest already holds (D124).
 *
 * A made skin copies Eyewall's layout the moment it is made, so it never got
 * what Eyewall gained afterwards — SEL was the first thing that showed it,
 * and the owner had been told to make the same skin again four times in one
 * evening. Everything the maker used is in the manifest it wrote: the name,
 * the six colours, the ramp, and where the picture is (`picture`, D127, or
 * for a skin made before that, where the playlist's backdrop ends). So the
 * skin is rebuilt from Eyewall's current layout each
 * time it is worn — a pure step, no picture decoded — the way an import is
 * rebuilt when its importer improves (D107). The picture itself is not
 * redrawn: it is the same sheet either way.
 *
 * Returns null for anything that is not a made skin.
 */
export function remade(written: Record<string, any>, picture?: Picture): Record<string, unknown> | null {
  if (typeof written?.maker !== "number") return null;
  if (typeof written.name !== "string" || !written.palette || !Array.isArray(written.viscolor)) return null;
  return madeManifest({
    name: written.name,
    palette: written.palette,
    viscolor: written.viscolor,
    picture: picture ?? pictureOf(written),
  });
}

/**
 * Where a made skin's picture is, from its manifest. A skin made before D127
 * recorded nothing, and its sheet is the picture from the top down to where
 * the playlist's backdrop ends, with no room to move in; a record that does
 * not add up is read the same way rather than drawn from outside the sheet.
 */
export function pictureOf(written: Record<string, any>): Picture {
  const band = written.windows?.playlist?.elements?.backdrop?.sprite?.rect;
  const end = Array.isArray(band) && band.length === 4 ? band[1] + band[3] : PICTURE_H;
  const sheet = Math.max(PICTURE_H, Math.min(PICTURE_MAX_H, end));
  const old: Picture = { sheet, top: 0, height: sheet, at: "top" };
  const p = written.picture;
  if (!p || typeof p !== "object") return old;
  const whole = (n: unknown) => typeof n === "number" && Number.isInteger(n);
  if (!whole(p.sheet) || !whole(p.top) || !whole(p.height) || !(PLACES as readonly string[]).includes(p.at)) return old;
  if (p.sheet !== sheet || p.height < 1 || p.top < 0 || p.top + p.height > p.sheet) return old;
  return { sheet: p.sheet, top: p.top, height: p.height, at: p.at };
}

// ---- the picture, in the webview ----

/** Pixels for the palette: the picture shrunk to a small square, which is all
 * clustering needs and keeps a 20 MB photo from costing a second. */
export function pixelsOf(bitmap: ImageBitmap, size = 96): Uint8ClampedArray {
  const c = new OffscreenCanvas(size, size);
  const g = c.getContext("2d")!;
  g.drawImage(bitmap, 0, 0, size, size);
  return g.getImageData(0, 0, size, size).data;
}

/**
 * The backdrop sheet at one scale: the whole picture at the windows' width,
 * its own proportions, where `pictureFor` puts it.
 *
 * Always fitted by width, never cropped at the sides. The first version scaled
 * a picture that was too short for three windows up to their height instead,
 * which made a square picture wider than the windows and cut its sides off —
 * the owner's picture lost the balloon its skeleton was reaching for. So a
 * short or wide picture covers part of the windows, and the sheet is clear
 * around it where the ground shows, exactly as it does past the end of a
 * tall picture in a grown playlist.
 */
export async function backdropPng(bitmap: ImageBitmap, scale: 1 | 2): Promise<Uint8Array> {
  const p = pictureFor(bitmap.width, bitmap.height);
  const W = PICTURE_W * scale;
  const c = new OffscreenCanvas(W, p.sheet * scale);
  const g = c.getContext("2d")!;
  const k = W / bitmap.width;
  g.imageSmoothingQuality = "high";
  // A picture taller than the cap loses its bottom, never its middle.
  g.drawImage(bitmap, 0, p.top * scale, W, bitmap.height * k);
  const blob = await c.convertToBlob({ type: "image/png" });
  return new Uint8Array(await blob.arrayBuffer());
}

/** A picture file's name without its folder or extension, for the skin's. */
export function nameFrom(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? "";
  return base.replace(/\.[^.]+$/, "").trim() || "My skin";
}
