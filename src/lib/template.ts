// A skin to paint (#146): Eyewall's layout, its chrome already in colour, and
// a guide that says what every part of the sheet is. The other half of v0.5's
// headline beside making a skin from a picture (D110, D122).
//
// Eyewall's own sheets are masks the renderer tints (D73), which is nothing a
// person can paint over: a mask is white shapes. So the template is the same
// sheet with every sprite already tinted the colour it wears, and a manifest
// that says `art: final`, so what is painted is exactly what is worn. Nothing
// here is drawn from scratch; the geometry is the manifest's, as it is for
// `tools/chrome-sheet.ps1`.

import eyewall from "../../skins/eyewall/manifest.json";
import { parseSkin, sprites, type Token } from "./skin";

/** The scale the template's sheet is painted at. 2x, because the classic
 * windows are usually doubled (D76), and a 1x sheet doubled is soft; on a
 * screen at 1x the renderer takes the 2x file down. */
export const TEMPLATE_SCALE = 2;

/** The manifest a painted skin starts from: Eyewall's layout under a name of
 * its own, in full colour, wearing its own six colours. No `maker` stamp, so
 * it is never rebuilt from Eyewall (D124) — a painted skin's layout is the
 * person's — and no `generator`, so it is never re-imported (D107). */
export function templateManifest(palette: Record<Token, string>, name = "My skin"): Record<string, unknown> {
  const m = JSON.parse(JSON.stringify(eyewall)) as Record<string, any>;
  m.name = name;
  m.author = "";
  m.art = "final";
  m.colors = "own";
  m.palette = { ...palette };
  m.authoredScale = TEMPLATE_SCALE;
  m.sheets = { chrome: { [String(TEMPLATE_SCALE)]: "chrome.png" } };
  delete m.maker;
  delete m.generator;
  return m;
}

/** One rectangle of the sheet, with every element that draws from it and
 * the colour the first of them wears. */
export type Part = {
  n: number;
  rect: [number, number, number, number];
  tint: Token;
  names: string[];
};

/**
 * The sheet's parts, numbered in reading order. Many elements share one
 * rectangle — every well is one solid — so a part lists them all. Two of
 * Eyewall's rectangles are worn in two colours (a lit toggle's hover and
 * press); in full colour a pixel has one, and the first element's wins.
 */
export function templateParts(manifest: unknown = eyewall): Part[] {
  const skin = parseSkin(manifest).skin;
  const byRect = new Map<string, Part>();
  for (const { path, ref } of sprites(skin)) {
    if (ref.sheet !== "chrome") continue;
    const key = ref.rect.join(",");
    // windows.main.elements.playButton.hover -> main playButton hover
    const label = path
      .replace(/^windows\./, "")
      .replace(/\.(shade\.)?elements\./, (_m, shade) => (shade ? " shade " : " "))
      .replace(/\./g, " ");
    const part = byRect.get(key);
    if (part) {
      if (!part.names.includes(label)) part.names.push(label);
    } else {
      byRect.set(key, { n: 0, rect: [...ref.rect] as Part["rect"], tint: ref.tint, names: [label] });
    }
  }
  const parts = [...byRect.values()].sort((a, b) => a.rect[1] - b.rect[1] || a.rect[0] - b.rect[0]);
  parts.forEach((p, i) => (p.n = i + 1));
  return parts;
}

/** The note that goes in the folder. Windows line endings, for Notepad. */
export function templateReadme(): string {
  const lines = [
    "A skin to paint for hurricane-party",
    "",
    "chrome.png is every piece of the three classic windows' chrome, already in",
    "Eyewall's colours. Paint over it: whatever you leave in a part is exactly",
    "what that part looks like. Keep every part where it is and the size it is;",
    "the layout in manifest.json points at those rectangles.",
    "",
    "guide.png is the same sheet, bigger, with every part outlined and numbered,",
    "and a list of what each number is. Do not paint the guide; it is not used.",
    "",
    "Transparent pixels are see-through. The ground colour behind the windows,",
    "the words, the playlist rows and the analyser come from the six colours in",
    'manifest.json\'s "palette", which you can change too.',
    "",
    "chrome.png is at double size, because the windows usually are. Some parts",
    'draw at less than full strength: an "opacity" beside them in manifest.json',
    "says how much. Set it to 1 if you would rather paint the strength yourself.",
    "",
    "When it is ready, open hurricane-party's library, press Import skin..., and",
    "pick this folder's manifest.json. To share the skin, zip this folder; a zip",
    "imports the same way.",
    "",
  ];
  return lines.join("\r\n");
}

// ---- drawing, in the webview ----

function decode(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("the Eyewall sheet did not decode"));
    img.src = url;
  });
}

async function png(c: OffscreenCanvas): Promise<Uint8Array> {
  return new Uint8Array(await (await c.convertToBlob({ type: "image/png" })).arrayBuffer());
}

/**
 * The sheet to paint: each part cut from Eyewall's mask sheet and filled with
 * the colour it wears. `sheetUrl` is Eyewall's sheet at `TEMPLATE_SCALE`.
 */
export async function paintableSheet(
  sheetUrl: string,
  parts: Part[],
  palette: Record<Token, string>,
): Promise<{ canvas: OffscreenCanvas; bytes: Uint8Array }> {
  const img = await decode(sheetUrl);
  const s = TEMPLATE_SCALE;
  const out = new OffscreenCanvas(img.naturalWidth, img.naturalHeight);
  const g = out.getContext("2d")!;
  const cell = new OffscreenCanvas(1, 1);
  for (const p of parts) {
    const [x, y, w, h] = p.rect.map((v) => v * s);
    cell.width = w;
    cell.height = h;
    const c = cell.getContext("2d")!;
    c.clearRect(0, 0, w, h);
    c.globalCompositeOperation = "source-over";
    c.drawImage(img, x, y, w, h, 0, 0, w, h);
    // The mask's alpha, the part's colour.
    c.globalCompositeOperation = "source-in";
    c.fillStyle = palette[p.tint];
    c.fillRect(0, 0, w, h);
    g.drawImage(cell, x, y);
  }
  return { canvas: out, bytes: await png(out) };
}

/**
 * The guide: the painted sheet at twice its size over the ground colour,
 * every part outlined and numbered, and the list of numbers beside it.
 */
export async function guideSheet(
  painted: OffscreenCanvas,
  parts: Part[],
  palette: Record<Token, string>,
): Promise<Uint8Array> {
  const zoom = 2;
  const k = TEMPLATE_SCALE * zoom;
  const listW = 420;
  const lineH = 13;
  const w = painted.width * zoom + listW;
  const h = Math.max(painted.height * zoom, 16 + parts.length * lineH);
  const c = new OffscreenCanvas(w, h);
  const g = c.getContext("2d")!;
  g.fillStyle = palette.ground;
  g.fillRect(0, 0, w, h);
  g.imageSmoothingEnabled = false;
  g.drawImage(painted, 0, 0, painted.width * zoom, painted.height * zoom);

  g.font = "10px ui-monospace, Consolas, monospace";
  g.textBaseline = "top";
  for (const p of parts) {
    const [x, y, pw, ph] = p.rect.map((v) => v * k);
    g.strokeStyle = palette.alert;
    g.lineWidth = 1;
    g.strokeRect(x + 0.5, y + 0.5, pw - 1, ph - 1);
    const label = String(p.n);
    const tw = g.measureText(label).width + 3;
    g.fillStyle = palette.alert;
    g.fillRect(x, y, tw, 11);
    g.fillStyle = palette.ground;
    g.fillText(label, x + 1.5, y + 1);
  }

  const lx = painted.width * zoom + 12;
  g.fillStyle = palette.text;
  for (const p of parts) {
    const more = p.names.length > 1 ? ` (+${p.names.length - 1})` : "";
    let line = `${String(p.n).padStart(2, " ")}  ${p.names[0]}${more}`;
    if (line.length > 62) line = `${line.slice(0, 61)}…`;
    g.fillText(line, lx, 8 + (p.n - 1) * lineH);
  }
  return png(c);
}
