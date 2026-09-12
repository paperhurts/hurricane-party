// The skin that ships. Eyewall is bundled by Vite from skins/eyewall/, so it
// is present with zero network and zero file access (D11, D29): the manifest
// is a JSON import and the sheets are asset imports, which Vite inlines as
// data: URLs at this size. An imported skin (#107) arrives by another road
// into the same `loadSkin`.
//
// The manifest is validated at import, not at mount, so a broken default
// skin is a build that does not start rather than three windows with no
// chrome. The test suite parses the same file, so CI sees it first.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import manifest from "../../skins/eyewall/manifest.json";
import chrome1 from "../../skins/eyewall/chrome.png";
import chrome2 from "../../skins/eyewall/chrome@2x.png";
import { parseSkin, type Skin, type WindowName } from "./skin";

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
  if (id === "eyewall") return { id, skin: EYEWALL, resolve: eyewallFile };
  try {
    const on = await invoke<{ manifest: string; dir: string }>("read_skin", { id });
    const parsed = parseSkin(JSON.parse(on.manifest));
    for (const w of parsed.warnings) console.warn(`${id}: ${w}`);
    return { id, skin: parsed.skin, resolve: (file) => convertFileSrc(`${on.dir}/${file}`) };
  } catch (e) {
    const reason = e instanceof Error ? e.message : String(e);
    console.error(`skin "${id}" could not be worn; wearing Eyewall instead:`, e);
    return { id: "eyewall", skin: EYEWALL, resolve: eyewallFile, instead: { id, reason } };
  }
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
  const sizes: Record<string, [number, number]> = {};
  await Promise.all(
    files
      .filter((f) => f.endsWith(".bmp"))
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
