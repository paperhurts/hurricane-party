// The skin that ships. Eyewall is bundled by Vite from skins/eyewall/, so it
// is present with zero network and zero file access (D11, D29): the manifest
// is a JSON import and the sheets are asset imports, which Vite inlines as
// data: URLs at this size. An imported skin (#107) arrives by another road
// into the same `loadSkin`.
//
// The manifest is validated at import, not at mount, so a broken default
// skin is a build that does not start rather than three windows with no
// chrome. The test suite parses the same file, so CI sees it first.

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
