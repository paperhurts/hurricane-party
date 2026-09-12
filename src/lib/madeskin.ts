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

/** How much of the picture shows through. The six colours are made legible
 * against the ground, not against a photograph, so the picture stays a wash
 * over the ground rather than competing with the words on it. */
export const PICTURE_OPACITY = 0.28;

const WINDOW_BAND: Record<string, number> = { main: 0, equalizer: 1, playlist: 2 };

/** The hp-skin/1 manifest for a made skin: Eyewall's, renamed, repainted, and
 * with the picture laid in behind each window's chrome. */
export function madeManifest(made: {
  name: string;
  palette: Record<Token, string>;
  viscolor: string[];
}): Record<string, unknown> {
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
  for (const [win, band] of Object.entries(WINDOW_BAND)) {
    const w = m.windows[win];
    const backdrop = {
      type: "image",
      rect: [0, 0, 275, 116],
      // The playlist grows, and its third of the picture grows with it.
      ...(w.resizable ? { stretch: "xy" } : {}),
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

/** The backdrop sheet at one scale: the picture cropped to cover the three
 * stacked windows, centred, the way a wallpaper fills a screen. */
export async function backdropPng(bitmap: ImageBitmap, scale: 1 | 2): Promise<Uint8Array> {
  const W = PICTURE_W * scale;
  const H = PICTURE_H * scale;
  const c = new OffscreenCanvas(W, H);
  const g = c.getContext("2d")!;
  const k = Math.max(W / bitmap.width, H / bitmap.height);
  const w = bitmap.width * k;
  const h = bitmap.height * k;
  g.imageSmoothingQuality = "high";
  g.drawImage(bitmap, (W - w) / 2, (H - h) / 2, w, h);
  const blob = await c.convertToBlob({ type: "image/png" });
  return new Uint8Array(await blob.arrayBuffer());
}

/** A picture file's name without its folder or extension, for the skin's. */
export function nameFrom(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? "";
  return base.replace(/\.[^.]+$/, "").trim() || "My skin";
}
