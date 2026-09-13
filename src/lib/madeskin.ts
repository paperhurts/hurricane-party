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

/**
 * The picture sheet's height, at 1x, for a picture of this size.
 *
 * The picture is kept at the windows' width and its own proportions, so the
 * playlist can uncover more of it as it grows (D122). The first made skin
 * cropped it to exactly three windows and the playlist stretched that crop,
 * throwing away a tall picture's lower half. At least three windows tall, so
 * every window's third exists in the sheet — a picture shorter than that
 * fills what it can from the top and leaves the rest clear — and at most
 * `PICTURE_MAX_H`.
 */
export function pictureHeightFor(width: number, height: number): number {
  if (!(width > 0 && height > 0)) return PICTURE_H;
  return Math.min(PICTURE_MAX_H, Math.max(PICTURE_H, Math.round((height * PICTURE_W) / width)));
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
  /** The picture sheet's height at 1x, from `pictureHeightFor`. */
  pictureHeight?: number;
}): Record<string, unknown> {
  const sheetH = Math.max(PICTURE_H, Math.min(PICTURE_MAX_H, made.pictureHeight ?? PICTURE_H));
  const m = JSON.parse(JSON.stringify(eyewall)) as Record<string, any>;
  m.name = made.name;
  m.author = "";
  m.maker = MAKER_VERSION;
  // Mask art asking for its own colours (D122): the picture's, not the theme's.
  m.colors = "own";
  m.palette = { ...made.palette };
  m.viscolor = [...made.viscolor];
  m.sheets = {
    ...m.sheets,
    picture: { "1": "picture.png", "2": "picture@2x.png", art: "final" },
  };
  // Before the backdrops go in: the picture's own opacity is not lifted.
  liftOpacities(m.windows);
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
          sprite: { sheet: "picture", rect: [0, band * 116, PICTURE_W, sheetH - band * 116] },
          opacity: PICTURE_OPACITY,
        }
      : {
          type: "image",
          rect: [0, 0, 275, 116],
          sprite: { sheet: "picture", rect: [0, band * 116, PICTURE_W, 116] },
          opacity: PICTURE_OPACITY,
        };
    // First, so it is drawn under everything else in the window.
    w.elements = { backdrop, ...w.elements };
  }
  return m;
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
 * its own proportions, from the top.
 *
 * Always fitted by width, never cropped at the sides. The first version scaled
 * a picture that was too short for three windows up to their height instead,
 * which made a square picture wider than the windows and cut its sides off —
 * the owner's picture lost the balloon its skeleton was reaching for. So a
 * short or wide picture covers as far down the windows as it reaches and the
 * sheet is clear below it, where the ground shows, exactly as it does past
 * the end of a tall picture in a grown playlist.
 */
export async function backdropPng(bitmap: ImageBitmap, scale: 1 | 2): Promise<Uint8Array> {
  const W = PICTURE_W * scale;
  const H = pictureHeightFor(bitmap.width, bitmap.height) * scale;
  const c = new OffscreenCanvas(W, H);
  const g = c.getContext("2d")!;
  const k = W / bitmap.width;
  g.imageSmoothingQuality = "high";
  // From the top: the equalizer's third ends where the playlist's begins, and
  // a picture taller than the cap loses its bottom, never its middle.
  g.drawImage(bitmap, 0, 0, W, bitmap.height * k);
  const blob = await c.convertToBlob({ type: "image/png" });
  return new Uint8Array(await blob.arrayBuffer());
}

/** A picture file's name without its folder or extension, for the skin's. */
export function nameFrom(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? "";
  return base.replace(/\.[^.]+$/, "").trim() || "My skin";
}
