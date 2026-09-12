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
import { wszManifest, WSZ_GENERATION } from "./wsz";

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
    const on = await invoke<SkinOnDisk>("read_skin", { id });
    const resolve = (file: string) => convertFileSrc(`${on.dir}/${file}`);
    const written = JSON.parse(on.manifest) as { generator?: number };
    // A manifest is not the skin, it is what this importer made of it, so an
    // importer that has learned something rebuilds it from the art rather
    // than leaving a person with what an older one managed (D107). The same
    // path catches a manifest that no longer parses at all.
    const parsed =
      written.generator === WSZ_GENERATION ? parseSkin(written) : await rebuild(id, on, "an older import");
    for (const w of parsed.warnings) console.warn(`${id}: ${w}`);
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

/**
 * What the importer said about a skin when it wrote its manifest (D110): the
 * art it could not find, and the classic controls this app does not use. Read
 * from the manifest rather than recomputed, so the library can say it when a
 * person picks the skin and not only the once at import. Eyewall has none.
 */
export async function skinNotes(id: string): Promise<string[]> {
  if (id === "eyewall") return [];
  try {
    const on = await invoke<SkinOnDisk>("read_skin", { id });
    const notes = (JSON.parse(on.manifest) as { notes?: unknown }).notes;
    return Array.isArray(notes) ? notes.filter((n): n is string => typeof n === "string") : [];
  } catch {
    return [];
  }
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
