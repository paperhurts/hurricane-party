// The skins that ship. Eyewall and Purricane (D132) are bundled by Vite from
// skins/, so they are present with zero network and zero file access (D11,
// D29): the manifest is a JSON import and the sheets are asset imports. An
// imported skin (#107) arrives by another road into the same `loadSkin`.
//
// The manifest is validated at import, not at mount, so a broken default
// skin is a build that does not start rather than three windows with no
// chrome. The test suite parses the same file, so CI sees it first.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import manifest from "../../skins/eyewall/manifest.json";
import chrome1 from "../../skins/eyewall/chrome.png";
import chrome2 from "../../skins/eyewall/chrome@2x.png";
import purricaneManifest from "../../skins/purricane/manifest.json";
import purricane1 from "../../skins/purricane/chrome.png";
import purricane2 from "../../skins/purricane/chrome@2x.png";
import { parseSkin, type Skin, type WindowName } from "./skin";
import { wszManifest, WSZ_GENERATION } from "./wsz";
import {
  canMove,
  PICTURE_H,
  PICTURE_W,
  pictureFor,
  pictureOf,
  remade,
  type Picture,
  type PicturePlace,
} from "./madeskin";

const FILES: Record<string, string> = {
  "chrome.png": chrome1,
  "chrome@2x.png": chrome2,
};

const parsed = parseSkin(manifest);
for (const w of parsed.warnings) console.warn(`eyewall: ${w}`);

export const EYEWALL: Skin = parsed.skin;

/** A sheet file name from the Eyewall manifest to the URL Vite gave it. */
export function eyewallFile(file: string): string {
  const url = FILES[file];
  if (!url) throw new Error(`eyewall: manifest names ${file}, which is not bundled`);
  return url;
}

const purricaneParsed = parseSkin(purricaneManifest);
for (const w of purricaneParsed.warnings) console.warn(`purricane: ${w}`);

/** Purricane's own skin (D132): the designer's layout, which the Purricane
 * theme wears and which wears the Purricane theme. */
export const PURRICANE: Skin = purricaneParsed.skin;

const PURRICANE_FILES: Record<string, string> = {
  "chrome.png": purricane1,
  "chrome@2x.png": purricane2,
};

/** The skins inside the app, by id. Nothing on disk can take these ids
 * (`skins.rs` SHIPPED), so an id here is always this skin. */
const SHIPPED: Record<string, Omit<Wearable, "id" | "instead">> = {
  eyewall: { skin: EYEWALL, resolve: eyewallFile },
  purricane: {
    skin: PURRICANE,
    resolve: (file) => {
      const url = PURRICANE_FILES[file];
      if (!url) throw new Error(`purricane: manifest names ${file}, which is not bundled`);
      return url;
    },
  },
};

/** Whether a skin id is one that ships rather than one on disk. */
export function isShipped(id: string): boolean {
  return Object.hasOwn(SHIPPED, id);
}

/** A skin ready to load: its art, where each sheet's file lives, and why it
 * is not the skin that was asked for, when it is not. */
export type Wearable = {
  id: string;
  skin: Skin;
  resolve: (file: string) => string;
  /** The skin the person chose, and what went wrong reading it. */
  instead?: { id: string; reason: string };
};

/**
 * The skin the person is wearing (#107). Eyewall is bundled and always the
 * fallback: a skin that will not parse, or a folder that has gone from disk,
 * leaves the windows dressed rather than bare, and says why in the console.
 * An imported skin's sheets come over the asset protocol, which is scoped to
 * the app's own data directories (`tauri.conf.json`).
 */
export async function currentSkin(): Promise<Wearable> {
  let id = "eyewall";
  try {
    id = await invoke<string>("get_skin");
  } catch {
    // No backend (a browser, a test): the shipped skin is the only one there.
    return { id, skin: EYEWALL, resolve: eyewallFile };
  }
  if (isShipped(id)) return { id, ...SHIPPED[id] };
  try {
    const on = await invoke<SkinOnDisk>("read_skin", { id });
    const own = (file: string) => convertFileSrc(`${on.dir}/${file}`);
    const written = JSON.parse(on.manifest) as { generator?: number };
    // A manifest is not the skin, it is what this importer made of it, so an
    // importer that has learned something rebuilds it from the art rather
    // than leaving a person with what an older one managed (D107). The same
    // path catches a manifest that no longer parses at all.
    // Only a manifest the .wsz importer wrote carries `generator` (D107). A
    // made skin or a hand-written one has none and is worn as it is: rebuilding
    // it would re-import art it never came from (#131).
    const stale = typeof written.generator === "number" && written.generator !== WSZ_GENERATION;
    // A made skin is made again from Eyewall's current layout every time it is
    // worn, so it has whatever Eyewall has now (D124), and the folder is
    // brought up to date when that changed anything.
    const again = remade(written as Record<string, any>);
    if (again) {
      const json = JSON.stringify(again, null, 1);
      if (json !== on.manifest) {
        await invoke("write_skin_manifest", { id, json }).catch(() => {});
        console.info(`${id}: made skin brought up to Eyewall's current layout`);
      }
    }
    const parsed = again ? parseSkin(again) : stale ? await rebuild(id, on, "an older import") : parseSkin(written);
    for (const w of parsed.warnings) console.warn(`${id}: ${w}`);
    // And it wears Eyewall's sheets as they are now, not the copies made with
    // it (D126): a layout from today on a sheet from the day it was made
    // reads sprites that have since moved or changed strength. The copies stay
    // in the folder, so the folder is still a whole skin wherever it goes.
    // Its picture's address carries the sheet's shape (D127): the same file
    // name redrawn taller must not be answered from the webview's cache of
    // the shorter one, which would refuse the skin for sprites past its end.
    const shape = again ? pictureOf(again) : null;
    const resolve = again
      ? (file: string) =>
          FILES[file] ?? (file.startsWith("picture") ? `${own(file)}?sheet=${shape!.sheet}.${shape!.top}` : own(file))
      : own;
    return { id, skin: parsed.skin, resolve };
  } catch (e) {
    const reason = e instanceof Error ? e.message : String(e);
    console.error(`skin "${id}" could not be worn; wearing Eyewall instead:`, e);
    return { id: "eyewall", skin: EYEWALL, resolve: eyewallFile, instead: { id, reason } };
  }
}

/** What Rust hands back for a skin on disk: its manifest, its folder, and
 * everything the importer would need to write the manifest again (D107). */
type SkinOnDisk = {
  manifest: string;
  dir: string;
  files: string[];
  pledit: string | null;
  viscolor: string | null;
};

/**
 * Map a skin's art again and write the result beside it. What the importer
 * writes depends on the importer, and this app's has changed twice already
 * while real skins were on disk; regenerating costs one decode of each sheet
 * and saves a person re-importing everything they own (D107).
 */
async function rebuild(id: string, on: SkinOnDisk, why: string) {
  // The name it was imported under, when the old manifest still has it.
  const was = (JSON.parse(on.manifest) as { name?: string }).name;
  const sizes = await measureSheets(on.dir, on.files);
  const built = wszManifest({
    files: on.files,
    name: was || id,
    pledit: on.pledit ?? undefined,
    viscolor: on.viscolor ?? undefined,
    sizes,
  });
  const parsed = parseSkin(built.manifest);
  // Only once it parses: a folder keeps the manifest it had until there is a
  // better one to replace it with.
  await invoke("write_skin_manifest", { id, json: JSON.stringify(built.manifest, null, 1) }).catch(() => {});
  console.info(`${id}: manifest rebuilt (${why})`);
  return parsed;
}

/**
 * How big each of a skin's sheets really is, in pixels, by file name (D106).
 * The classic format never declared a sheet's size, and plenty of skins ship
 * a short one - no volume thumb, an equalizer sheet that stops above the
 * sliders - so the importer has to look before it can write a manifest that
 * only claims art the skin has. The webview's own decoder is the authority,
 * since it is the one that will draw them.
 *
 * A file that will not decode is left out rather than guessed at; the caller
 * then keeps every rectangle for it, and `parseSkin` refuses the skin with
 * the reason, which is the honest outcome for art nothing can read.
 */
export async function measureSheets(dir: string, files: string[]): Promise<Record<string, [number, number]>> {
  return measure(
    dir,
    files.filter((f) => f.endsWith(".bmp")),
  );
}

/**
 * The size of every sheet a native manifest names, in the form the bounds
 * check takes (#146). A painted or zipped `hp-skin/1` skin is checked against
 * its own art before it is worn, as an import is, so a rectangle past the end
 * of a sheet is a refusal with a reason rather than a skin that falls back
 * to Eyewall the moment it is picked.
 */
export async function sheetSizes(dir: string, files: string[]): Promise<Record<string, { w: number; h: number }>> {
  const sizes = await measure(dir, files);
  return Object.fromEntries(Object.entries(sizes).map(([f, [w, h]]) => [f, { w, h }]));
}

async function measure(dir: string, files: string[]): Promise<Record<string, [number, number]>> {
  const sizes: Record<string, [number, number]> = {};
  await Promise.all(
    files
      .map(
        (f) =>
          new Promise<void>((done) => {
            const img = new Image();
            img.crossOrigin = "anonymous";
            img.onload = () => {
              sizes[f] = [img.naturalWidth, img.naturalHeight];
              done();
            };
            img.onerror = () => done();
            img.src = convertFileSrc(`${dir}/${f}`);
          }),
      ),
  );
  return sizes;
}

/**
 * What the importer said about a skin when it wrote its manifest (D110): the
 * art it could not find, and the classic controls this app does not use. Read
 * from the manifest rather than recomputed, so the library can say it when a
 * person picks the skin and not only the once at import. Eyewall has none.
 */
export async function skinNotes(id: string): Promise<string[]> {
  if (isShipped(id)) return [];
  try {
    const on = await invoke<SkinOnDisk>("read_skin", { id });
    const notes = (JSON.parse(on.manifest) as { notes?: unknown }).notes;
    return Array.isArray(notes) ? notes.filter((n): n is string => typeof n === "string") : [];
  } catch {
    return [];
  }
}

/**
 * Where a made skin's picture sits, when it could sit anywhere else (D127).
 * Null for Eyewall, an import, and a made skin whose picture looks the same
 * at every place, so the library offers the choice only where it does
 * something.
 *
 * A skin made before D127 recorded nothing, and a picture shorter than the
 * windows filled the top of a sheet with no room around it — which is most
 * of the skins the owner had made. So the first time one is asked about, its
 * sheet is given that room here, once: it looks exactly as it did, and can
 * move from then on. A picture that really is three windows tall is measured
 * and left as it is.
 *
 * `redrawn` says the sheet changed under the manifest. Ask before telling the
 * windows to wear a skin, never after: the first try told them first, and a
 * window that read the skin while its sheets were half rewritten refused it
 * and put the owner back in Eyewall. When a window may already be wearing it
 * (the library opening), tell them again once this says `redrawn`.
 */
export function readyPicture(id: string): Promise<{ at: PicturePlace | null; redrawn: boolean }> {
  // One at a time per skin. Two at once was how the first try broke: the
  // second saw the first's new 1x sheet beside its old 2x one, took the skin
  // for done and wrote the new manifest before the 2x sheet existed.
  const running = readying.get(id);
  if (running) return running;
  const run = readyOnce(id).finally(() => readying.delete(id));
  readying.set(id, run);
  return run;
}

const readying = new Map<string, Promise<{ at: PicturePlace | null; redrawn: boolean }>>();

async function readyOnce(id: string): Promise<{ at: PicturePlace | null; redrawn: boolean }> {
  const none = { at: null, redrawn: false };
  if (isShipped(id)) return none;
  try {
    const on = await invoke<SkinOnDisk>("read_skin", { id });
    const written = JSON.parse(on.manifest) as Record<string, any>;
    if (typeof written.maker !== "number") return none;
    let p = pictureOf(written);
    let redrawn = false;
    // No room around it: made before D127 (the windows may already have
    // written it a record that says so), or a picture exactly this tall.
    if (p.sheet === PICTURE_H && p.top === 0) {
      p = await giveRoom(id, on, written);
      redrawn = p.top > 0;
    }
    return { at: canMove(p) ? p.at : null, redrawn };
  } catch (e) {
    console.warn(`${id}: where its picture sits could not be read:`, e);
    return none;
  }
}

/** Move a made skin's picture (D127). Only the manifest changes; the windows
 * wear it when they next read it, which `set_skin` asks them to. */
export async function placePicture(id: string, at: PicturePlace): Promise<void> {
  const on = await invoke<SkinOnDisk>("read_skin", { id });
  const written = JSON.parse(on.manifest) as Record<string, any>;
  const again = remade(written, { ...pictureOf(written), at });
  if (!again) throw new Error(`${id} is not a skin made from a picture`);
  // The same validator every skin goes through, before it is written.
  parseSkin(again);
  await invoke("write_skin_manifest", { id, json: JSON.stringify(again, null, 1) });
}

/**
 * Redraw a pre-D127 made skin's picture sheets with room above and below the
 * picture, and record where it is. The picture's height is not in the old
 * manifest, but it is in the sheet: the maker drew it from the top and left
 * the rest clear, so it ends at the last row with anything in it.
 */
async function giveRoom(id: string, on: SkinOnDisk, written: Record<string, any>): Promise<Picture> {
  // Never from this window's cache: a sheet already redrawn must be seen as
  // redrawn, or it would be given its room a second time.
  const fresh = (file: string) => `${convertFileSrc(`${on.dir}/${file}`)}?read=${Date.now()}`;
  const [one, two] = await Promise.all([picture(fresh("picture.png")), picture(fresh("picture@2x.png"))]);
  const rows = usedRows(one);
  const tall = one.naturalHeight;
  let p: Picture;
  if (tall > PICTURE_H && tall < 2 * PICTURE_H && two.naturalHeight === 2 * tall) {
    // Already redrawn, and the record of it lost: a window that read the old
    // manifest wrote it back. The room is in the sheet's own height.
    p = { sheet: tall, top: tall - PICTURE_H, height: 2 * PICTURE_H - tall, at: "top" };
  } else if (tall === PICTURE_H && two.naturalHeight === 2 * PICTURE_H && rows > 0 && rows < PICTURE_H) {
    p = pictureFor(PICTURE_W, rows);
    for (const [img, scale] of [
      [one, 1],
      [two, 2],
    ] as const) {
      const c = new OffscreenCanvas(PICTURE_W * scale, p.sheet * scale);
      c.getContext("2d")!.drawImage(img, 0, p.top * scale);
      const bytes = new Uint8Array(await (await c.convertToBlob({ type: "image/png" })).arrayBuffer());
      await invoke("write_skin_picture", bytes, { headers: { "x-hp-skin": id, "x-hp-scale": String(scale) } });
    }
  } else {
    // Exactly three windows tall, or two sheets that do not agree: left
    // alone. Nothing is written from a sheet this cannot account for.
    return { sheet: PICTURE_H, top: 0, height: PICTURE_H, at: "top" };
  }
  const again = remade(written, p);
  if (!again) return p;
  parseSkin(again);
  await invoke("write_skin_manifest", { id, json: JSON.stringify(again, null, 1) });
  console.info(`${id}: picture given room to move (${rows} rows, now in a ${p.sheet}-row sheet)`);
  return p;
}

function picture(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    // The asset protocol is another origin; a CORS-clean image is one a
    // canvas can read back (skinsheet.ts).
    img.crossOrigin = "anonymous";
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error(`picture sheet failed to decode: ${url.slice(0, 64)}`));
    img.src = url;
  });
}

/** Rows from the top of a 1x picture sheet to the last with any pixel in it. */
function usedRows(img: HTMLImageElement): number {
  const c = new OffscreenCanvas(img.naturalWidth, img.naturalHeight);
  const g = c.getContext("2d")!;
  g.drawImage(img, 0, 0);
  const { data, width, height } = g.getImageData(0, 0, img.naturalWidth, img.naturalHeight);
  for (let y = height - 1; y >= 0; y--) {
    for (let x = 0; x < width; x++) if (data[(y * width + x) * 4 + 3] > 0) return y + 1;
  }
  return 0;
}

/** The three classic windows' labels are not the manifest's names. */
export function windowNameOf(label: string): WindowName {
  switch (label) {
    case "main":
      return "main";
    case "eq":
      return "equalizer";
    case "playlist":
      return "playlist";
    default:
      throw new Error(`no classic window is labelled "${label}"`);
  }
}
