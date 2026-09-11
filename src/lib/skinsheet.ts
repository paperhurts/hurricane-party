// The DOM half of the skin renderer: decode a skin's sheets and cut them into
// sprites. A sprite is handed out as a data: URL of exactly its rectangle, so
// the chrome can stretch, tile and mask it with plain CSS (a nine-slice edge
// is `mask-size: 100% 100%` on its own slice; `background-position` on a
// whole sheet could never stretch one region of it). Cut once per window and
// memoised; a window's chrome asks for a few dozen.
//
// The CSP allows `data:` for images and nothing remote (D29), which is the
// whole loading story: the shipped skin's sheets are bundled by Vite, an
// imported one's (#107) come through the asset protocol.

import { checkSheetBounds, type Scale, type Skin, SkinError, sheetFor, type SpriteRef, type Token } from "./skin";

export type Slice = {
  /** A data: URL of just this sprite, at `scale` pixels per logical pixel. */
  url: string;
  /** Logical size, the rect's own. */
  w: number;
  h: number;
  scale: Scale;
  tint: Token;
};

export type LoadedSkin = {
  skin: Skin;
  /** The device pixel ratio the sheets were chosen for. */
  dpr: number;
  /** The sprite at a reference. Memoised by sheet and rect. */
  slice(ref: SpriteRef): Slice;
  /** A sheet's size in logical pixels. A bitmap font needs it: its glyphs are
   * a grid, and how many fit across is the sheet's own width (D104). */
  sheetSize(name: string): { w: number; h: number };
};

type Sheet = { canvas: HTMLCanvasElement; scale: Scale };

function decode(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new SkinError(`sheet failed to decode: ${url.slice(0, 64)}`));
    img.src = url;
  });
}

/**
 * Decode every sheet a skin needs at this device pixel ratio and check every
 * sprite against the pixels that arrived. A skin that fails is refused whole
 * (skin-manifest.md, Validation); nothing is cut before the check passes.
 *
 * `resolve` turns a sheet file name from the manifest into a URL the
 * webview may load.
 */
export async function loadSkin(
  skin: Skin,
  resolve: (file: string) => string,
  dpr: number = window.devicePixelRatio,
): Promise<LoadedSkin> {
  const sheets = new Map<string, Sheet>();
  const sizes: Record<string, { w: number; h: number }> = {};
  for (const [name, per] of Object.entries(skin.sheets)) {
    // Every file the skin lists is decoded and measured, not only the one
    // this screen draws: a rect must lie inside its sheet at every scale
    // (skin-manifest.md), and a short 2x file is a broken skin on a 1x
    // screen too, not a surprise saved for the next monitor. They are small.
    const chosen = sheetFor(skin, name, dpr);
    for (const file of new Set(Object.values(per))) {
      const img = await decode(resolve(file));
      sizes[file] = { w: img.naturalWidth, h: img.naturalHeight };
      if (file === chosen.file) {
        const canvas = document.createElement("canvas");
        canvas.width = img.naturalWidth;
        canvas.height = img.naturalHeight;
        canvas.getContext("2d")!.drawImage(img, 0, 0);
        sheets.set(name, { canvas, scale: chosen.scale });
      }
    }
  }
  const problems = checkSheetBounds(skin, sizes);
  if (problems.length) throw new SkinError(`skin "${skin.name}" refused:\n  ${problems.join("\n  ")}`);

  const cache = new Map<string, Slice>();
  return {
    skin,
    dpr,
    slice(ref) {
      const key = `${ref.sheet}|${ref.rect.join(",")}|${ref.tint}`;
      const hit = cache.get(key);
      if (hit) return hit;
      const sheet = sheets.get(ref.sheet)!;
      const [x, y, w, h] = ref.rect;
      const s = sheet.scale;
      const c = document.createElement("canvas");
      c.width = w * s;
      c.height = h * s;
      c.getContext("2d")!.drawImage(sheet.canvas, x * s, y * s, w * s, h * s, 0, 0, w * s, h * s);
      const out: Slice = { url: c.toDataURL("image/png"), w, h, scale: s, tint: ref.tint };
      cache.set(key, out);
      return out;
    },
    sheetSize(name) {
      const sheet = sheets.get(name);
      if (!sheet) return { w: 0, h: 0 };
      return { w: sheet.canvas.width / sheet.scale, h: sheet.canvas.height / sheet.scale };
    },
  };
}

/** The nine pieces of a nine-slice, as sprite references into the same sheet.
 * Corners keep their size; edges stretch along one axis; the centre along
 * both. `insets` are top, right, bottom, left, in logical px. */
export function nineSliceRefs(
  sprite: SpriteRef,
  insets: [number, number, number, number],
): { piece: NinePiece; ref: SpriteRef }[] {
  const [x, y, w, h] = sprite.rect;
  const [t, r, b, l] = insets;
  const cw = w - l - r;
  const ch = h - t - b;
  const at = (px: number, py: number, pw: number, ph: number): SpriteRef => ({
    sheet: sprite.sheet,
    rect: [x + px, y + py, pw, ph],
    tint: sprite.tint,
  });
  const out: { piece: NinePiece; ref: SpriteRef }[] = [];
  if (t > 0 && l > 0) out.push({ piece: "tl", ref: at(0, 0, l, t) });
  if (t > 0 && cw > 0) out.push({ piece: "t", ref: at(l, 0, cw, t) });
  if (t > 0 && r > 0) out.push({ piece: "tr", ref: at(l + cw, 0, r, t) });
  if (ch > 0 && l > 0) out.push({ piece: "l", ref: at(0, t, l, ch) });
  if (ch > 0 && cw > 0) out.push({ piece: "c", ref: at(l, t, cw, ch) });
  if (ch > 0 && r > 0) out.push({ piece: "r", ref: at(l + cw, t, r, ch) });
  if (b > 0 && l > 0) out.push({ piece: "bl", ref: at(0, t + ch, l, b) });
  if (b > 0 && cw > 0) out.push({ piece: "b", ref: at(l, t + ch, cw, b) });
  if (b > 0 && r > 0) out.push({ piece: "br", ref: at(l + cw, t + ch, r, b) });
  return out;
}

export type NinePiece = "tl" | "t" | "tr" | "l" | "c" | "r" | "bl" | "b" | "br";

/** Where a nine-slice piece sits, as CSS inset/size for an absolutely
 * positioned box filling its parent. */
export function ninePieceStyle(piece: NinePiece, insets: [number, number, number, number]): string {
  const [t, r, b, l] = insets;
  switch (piece) {
    case "tl":
      return `left:0;top:0;width:${l}px;height:${t}px`;
    case "t":
      return `left:${l}px;right:${r}px;top:0;height:${t}px`;
    case "tr":
      return `right:0;top:0;width:${r}px;height:${t}px`;
    case "l":
      return `left:0;top:${t}px;bottom:${b}px;width:${l}px`;
    case "c":
      return `left:${l}px;right:${r}px;top:${t}px;bottom:${b}px`;
    case "r":
      return `right:0;top:${t}px;bottom:${b}px;width:${r}px`;
    case "bl":
      return `left:0;bottom:0;width:${l}px;height:${b}px`;
    case "b":
      return `left:${l}px;right:${r}px;bottom:0;height:${b}px`;
    case "br":
      return `right:0;bottom:0;width:${r}px;height:${b}px`;
  }
}
