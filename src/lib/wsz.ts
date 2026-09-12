// `.wsz` → `hp-skin/1` (D6, D17, v0.5). A classic Winamp 2 skin is a zip of
// BMPs and two text files, and every sprite's position inside those BMPs is
// conventional: the format declares nothing, so the offsets live here as a
// table. That is what makes the importer a bounded problem rather than an
// open-ended one (skin-manifest.md).
//
// This half is pure: file names in, a manifest out. The zip, the disk and the
// window that shows the refusal are the importer's; `parseSkin` validates what
// comes out of here exactly as it validates Eyewall, so an import that would
// half-load is refused before anything is written.
//
// The sprite rectangles below are transcribed from Webamp
// (https://github.com/captbaritone/webamp), MIT, Copyright (c) 2015 Jordan
// Eldredge — see THIRD-PARTY.md. The window positions are the classic
// layout, the same coordinates `docs/skin-manifest.md` uses in its example.
import { SkinError } from "./skin";
import { colorsFor, viscolor as themeRamp } from "./theme";

/** x, y, w, h inside a sheet, as the classic format lays them out. */
type Rect = [number, number, number, number];

// ---- the sheets we map, and the sprites we take from each ----

/** A sheet name in the manifest, and the classic file it comes from. Every
 * name is matched case-insensitively: skins in the wild ship `MAIN.BMP`,
 * `main.bmp` and `Main.Bmp`, sometimes in the same zip. */
export const SHEET_FILES: Record<string, string[]> = {
  main: ["main.bmp"],
  titlebar: ["titlebar.bmp"],
  cbuttons: ["cbuttons.bmp"],
  posbar: ["posbar.bmp"],
  volume: ["volume.bmp"],
  playpaus: ["playpaus.bmp"],
  shufrep: ["shufrep.bmp"],
  text: ["text.bmp"],
  // NUMS_EX carries the same digits plus the blank the shade strip wants;
  // skins that ship both mean the extended one.
  numbers: ["nums_ex.bmp", "numbers.bmp"],
  eqmain: ["eqmain.bmp"],
  pledit: ["pledit.bmp"],
};

/** Without these three there is no window to draw, and a skin that lacks them
 * is not a skin (skin-manifest.md: refuse rather than half-load). */
const REQUIRED_SHEETS = ["main", "titlebar", "cbuttons"] as const;

const MAIN_SP = {
  background: [0, 0, 275, 116] as Rect,
};

const TITLEBAR_SP = {
  bar: [27, 15, 275, 14] as Rect,
  barActive: [27, 0, 275, 14] as Rect,
  shadeBar: [27, 42, 275, 14] as Rect,
  shadeBarActive: [27, 29, 275, 14] as Rect,
  options: [0, 0, 9, 9] as Rect,
  optionsDown: [0, 9, 9, 9] as Rect,
  minimize: [9, 0, 9, 9] as Rect,
  minimizeDown: [9, 9, 9, 9] as Rect,
  shade: [0, 18, 9, 9] as Rect,
  shadeDown: [9, 18, 9, 9] as Rect,
  shadeOn: [0, 27, 9, 9] as Rect,
  shadeOnDown: [9, 27, 9, 9] as Rect,
  close: [18, 0, 9, 9] as Rect,
  closeDown: [18, 9, 9, 9] as Rect,
};

const CBUTTONS_SP = {
  prev: [0, 0, 23, 18] as Rect,
  prevDown: [0, 18, 23, 18] as Rect,
  play: [23, 0, 23, 18] as Rect,
  playDown: [23, 18, 23, 18] as Rect,
  pause: [46, 0, 23, 18] as Rect,
  pauseDown: [46, 18, 23, 18] as Rect,
  stop: [69, 0, 23, 18] as Rect,
  stopDown: [69, 18, 23, 18] as Rect,
  next: [92, 0, 23, 18] as Rect,
  nextDown: [92, 18, 22, 18] as Rect,
  eject: [114, 0, 22, 16] as Rect,
  ejectDown: [114, 16, 22, 16] as Rect,
};

const POSBAR_SP = {
  track: [0, 0, 248, 10] as Rect,
  thumb: [248, 0, 29, 10] as Rect,
  thumbActive: [278, 0, 29, 10] as Rect,
};

// VOLUME.BMP is 28 stacked frames of the level bar, 15 apart. The renderer
// draws a fill from the start to the value rather than picking a frame, so
// the fullest frame is the fill and it is stretched; the thumb is the
// classic's own.
const VOLUME_SP = {
  fill: [0, 405, 68, 13] as Rect,
  thumb: [15, 422, 14, 11] as Rect,
  thumbActive: [0, 422, 14, 11] as Rect,
};

const PLAYPAUS_SP = {
  playing: [0, 0, 9, 9] as Rect,
  paused: [9, 0, 9, 9] as Rect,
  stopped: [18, 0, 9, 9] as Rect,
};

const SHUFREP_SP = {
  shuffle: [28, 0, 47, 15] as Rect,
  shuffleDown: [28, 15, 47, 15] as Rect,
  shuffleOn: [28, 30, 47, 15] as Rect,
  shuffleOnDown: [28, 45, 47, 15] as Rect,
  repeat: [0, 0, 28, 15] as Rect,
  repeatDown: [0, 15, 28, 15] as Rect,
  repeatOn: [0, 30, 28, 15] as Rect,
  repeatOnDown: [0, 45, 28, 15] as Rect,
  eq: [0, 61, 23, 12] as Rect,
  eqOn: [0, 73, 23, 12] as Rect,
  eqDown: [46, 61, 23, 12] as Rect,
  playlist: [23, 61, 23, 12] as Rect,
  playlistOn: [23, 73, 23, 12] as Rect,
  playlistDown: [69, 61, 23, 12] as Rect,
};

const EQMAIN_SP = {
  background: [0, 0, 275, 116] as Rect,
  bar: [0, 149, 275, 14] as Rect,
  barActive: [0, 134, 275, 14] as Rect,
  close: [0, 116, 9, 9] as Rect,
  closeDown: [0, 125, 9, 9] as Rect,
  on: [10, 119, 26, 12] as Rect,
  onDown: [128, 119, 26, 12] as Rect,
  onSelected: [69, 119, 26, 12] as Rect,
  onSelectedDown: [187, 119, 26, 12] as Rect,
  presets: [224, 164, 44, 12] as Rect,
  presetsDown: [224, 176, 44, 12] as Rect,
  graph: [0, 294, 113, 19] as Rect,
  thumb: [0, 164, 11, 11] as Rect,
  thumbActive: [0, 176, 11, 11] as Rect,
};

const PLEDIT_SP = {
  topLeft: [0, 0, 25, 20] as Rect,
  topTitle: [26, 0, 100, 20] as Rect,
  topTile: [127, 0, 25, 20] as Rect,
  topRight: [153, 0, 25, 20] as Rect,
  topLeftIdle: [0, 21, 25, 20] as Rect,
  topTitleIdle: [26, 21, 100, 20] as Rect,
  leftTile: [0, 42, 12, 29] as Rect,
  rightTile: [31, 42, 20, 29] as Rect,
  bottomLeft: [0, 72, 125, 38] as Rect,
  bottomRight: [126, 72, 150, 38] as Rect,
  shadeBar: [72, 57, 25, 14] as Rect,
  close: [52, 42, 9, 9] as Rect,
  collapse: [62, 42, 9, 9] as Rect,
  // The menu glyphs a press shows, from the popups the classic opened.
  addDir: [0, 130, 22, 18] as Rect,
  removeSelected: [54, 149, 22, 18] as Rect,
  loadList: [204, 149, 22, 18] as Rect,
};

// ---- where each element sits in the window ----
//
// The classic layout, in logical pixels at 1x. These are positions in the
// window, not offsets in a sheet: the format never declared them either.
const L = {
  titleBar: [0, 0, 275, 14] as Rect,
  options: [6, 3, 9, 9] as Rect,
  minimize: [244, 3, 9, 9] as Rect,
  shade: [254, 3, 9, 9] as Rect,
  close: [264, 3, 9, 9] as Rect,
  // The clock is four digits with the colon painted into MAIN.BMP between
  // them, so it is two elements, not one string (D104). Each pair is two
  // 9-wide glyphs 3 apart, which is the font's tracking.
  clockMinutes: [48, 26, 21, 13] as Rect,
  clockSeconds: [78, 26, 21, 13] as Rect,
  state: [26, 28, 9, 9] as Rect,
  title: [111, 27, 153, 6] as Rect,
  kbps: [111, 43, 15, 6] as Rect,
  khz: [156, 43, 10, 6] as Rect,
  vis: [24, 43, 76, 16] as Rect,
  volume: [107, 57, 68, 13] as Rect,
  eqButton: [219, 58, 23, 12] as Rect,
  plButton: [242, 58, 23, 12] as Rect,
  seek: [16, 72, 248, 10] as Rect,
  prev: [16, 88, 23, 18] as Rect,
  play: [39, 88, 23, 18] as Rect,
  pause: [62, 88, 23, 18] as Rect,
  stop: [85, 88, 23, 18] as Rect,
  next: [108, 88, 22, 18] as Rect,
  eject: [136, 89, 22, 16] as Rect,
  shuffle: [164, 89, 47, 15] as Rect,
  repeat: [210, 89, 28, 15] as Rect,
  // The equalizer.
  eqOn: [14, 18, 26, 12] as Rect,
  eqPresets: [217, 18, 44, 12] as Rect,
  eqGraph: [86, 17, 113, 19] as Rect,
  eqPreamp: [21, 38, 14, 63] as Rect,
  eqBandX: 78,
  eqBandStep: 18,
  eqBandY: 38,
  eqBandW: 14,
  eqBandH: 63,
  // The playlist, at its base size. Its edges tile, so these carry `stretch`,
  // and its bottom-right block carries the corner (D103).
  plTopHeight: 20,
  plLeftWidth: 12,
  plRightWidth: 20,
  plBottomHeight: 38,
  plBottomLeftWidth: 125,
  plBottomRightWidth: 150,
  // All at the base size, 275 x 116; the anchors carry them from there.
  /** The bar's menu buttons, 22 x 18, twelve up from the bottom. */
  plAdd: [14, 86, 22, 18] as Rect,
  plRemove: [43, 86, 22, 18] as Rect,
  plList: [231, 86, 22, 18] as Rect,
  /** The running time, in the bottom-right block. */
  plStatus: [132, 88, 60, 10] as Rect,
  /** The link field, over the bar's left half while it is open. */
  plUrl: [12, 86, 219, 18] as Rect,
  /** The title bar's own buttons. */
  plShade: [254, 3, 9, 9] as Rect,
};

/** The 5 x 6 font in TEXT.BMP: three rows of 31 glyphs, in this order. The
 * classic sheet has no glyph for several ASCII characters; a space stands in
 * for each, which is what the classic player drew too. */
const TEXT_MAP =
  'abcdefghijklmnopqrstuvwxyz"@   ' + "0123456789….:()-'!_+\\/[]^&%,=$#" + "ÅÖÄ?*" + " ".repeat(26);

/** The 9 x 13 digits in NUMBERS.BMP / NUMS_EX.BMP. */
const NUMBERS_MAP = "0123456789";

// ---- the two text files ----

/**
 * `PLEDIT.TXT`, the playlist's colours: an INI-ish file whose `[Text]`
 * section names `Normal`, `Current`, `NormalBG`, `SelectedBG` and a few more,
 * each `#RRGGBB`. Keys are matched case-insensitively, since skins spell them
 * every way. Anything unreadable is simply absent; a missing colour falls
 * back rather than failing, because half the skins in the wild are missing
 * something (skin-manifest.md).
 */
export function parsePledit(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split(/\r?\n/)) {
    const m = /^\s*([A-Za-z]+)\s*=\s*#?([0-9A-Fa-f]{6})\s*$/.exec(line);
    if (m) out[m[1].toLowerCase()] = `#${m[2].toUpperCase()}`;
  }
  return out;
}

/**
 * `VISCOLOR.TXT`, the visualizer's ramp: lines of `r,g,b`, anything after
 * them ignored, comments after `//`.
 *
 * The format wants 24 and the skins in the wild ship 23, 24 or 25 - of
 * thirteen real skins, seven were not 24 (D106). So take what is there: the
 * first 24, and when the file is short, repeat its last colour to fill. Only
 * a file with no colours at all is not a ramp, and then the caller keeps the
 * theme's.
 */
export function parseViscolor(text: string): string[] | null {
  const out: string[] = [];
  for (const line of text.split(/\r?\n/)) {
    const m = /^\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})/.exec(line);
    if (!m) continue;
    const hex = [1, 2, 3]
      .map((i) => Math.min(255, Number(m[i])).toString(16).padStart(2, "0").toUpperCase())
      .join("");
    out.push(`#${hex}`);
    if (out.length === 24) break;
  }
  if (out.length === 0) return null;
  while (out.length < 24) out.push(out[out.length - 1]);
  return out;
}

/**
 * The six tokens, from what a classic skin knows about colour (D101). Under
 * O19's answer the theme paints a skin the renderer can tint, and these only
 * reach the screen for an imported skin, whose own pixels the theme cannot
 * touch: the playlist's rows sit on `void`, in `filament`, with the playing
 * row in `strike` and the selected one in `arc`. A skin that ships no
 * `PLEDIT.TXT` gets the boot theme's six, so it still loads.
 */
export function paletteFrom(pledit: Record<string, string>): Record<string, string> {
  const fallback = colorsFor("eyewall");
  const pick = (key: string, role: string) => pledit[key] ?? fallback[role];
  return {
    ground: pick("normalbg", "ground"),
    surface: pick("normalbg", "surface"),
    text: pick("normal", "text"),
    accent: pick("selectedbg", "accent"),
    alert: pick("current", "alert"),
    warn: pick("current", "warn"),
  };
}

// ---- the manifest ----

/**
 * What this importer is up to. A manifest is not a skin, it is what this code
 * made of one, so it carries the generation that wrote it and is rebuilt when
 * this number moves on (D107). Bump it whenever the mapping changes what it
 * writes for the same art.
 */
export const WSZ_GENERATION = 4;

export type WszInput = {
  /** Every path in the zip, in any case and at any depth. */
  files: string[];
  /** The skin's name, normally the zip's own. */
  name: string;
  /** `PLEDIT.TXT` and `VISCOLOR.TXT`, when the zip has them. */
  pledit?: string;
  viscolor?: string;
  /** Each sheet's real size in pixels, by file name, when they have been
   * measured. A classic skin's sheets are conventionally sized and often are
   * not: the format never declared a size, so an author who needed no volume
   * thumb simply stopped the file short. Given sizes, the art that is not
   * there is left out of the manifest rather than refusing the skin (D106). */
  sizes?: Record<string, [number, number]>;
};

/** A sprite reference in the manifest's shape. Tint is the renderer's word for
 * `art: mask`; a classic skin is `art: final`, so it is ignored, and the
 * default keeps the JSON small. */
const sp = (sheet: string, rect: Rect) => ({ sheet, rect });

/**
 * Map one classic skin into `hp-skin/1`. Returns the manifest as plain JSON
 * (hand it to `parseSkin` to validate, and write it beside the art), plus the
 * warnings a person should see: the parts of their skin this app has no place
 * for yet.
 *
 * Throws `SkinError` when the zip is not a skin at all.
 */
export function wszManifest(input: WszInput): { manifest: Record<string, unknown>; warnings: string[] } {
  const warnings: string[] = [];
  // Basename, lower case: a zip may nest its files in a folder, and the
  // importer writes them out flat and lower case so the manifest's names are
  // right on a case-sensitive disk too.
  const have = new Set(input.files.map((f) => f.split(/[\\/]/).pop()!.toLowerCase()).filter(Boolean));
  const sheets: Record<string, string> = {};
  for (const [name, candidates] of Object.entries(SHEET_FILES)) {
    const file = candidates.find((c) => have.has(c));
    if (file) sheets[name] = file;
  }
  const missing = REQUIRED_SHEETS.filter((s) => !(s in sheets));
  if (missing.length) {
    throw new SkinError(
      `this zip is missing ${missing.map((m) => SHEET_FILES[m][0].toUpperCase()).join(", ")}: it is not a Winamp skin`,
    );
  }
  const has = (sheet: string) => sheet in sheets;

  const pledit = input.pledit ? parsePledit(input.pledit) : {};
  if (!input.pledit) warnings.push("no PLEDIT.TXT: the playlist's colours come from the theme");
  const ramp = input.viscolor ? parseViscolor(input.viscolor) : null;
  if (input.viscolor && !ramp) warnings.push("VISCOLOR.TXT has no colours in it: the analyser keeps the theme's ramp");

  const fonts: Record<string, unknown> = {};
  if (has("text")) fonts.chrome = { type: "bitmap", sheet: "text", glyphSize: [5, 6], map: TEXT_MAP };
  if (has("numbers")) {
    fonts.time = { type: "bitmap", sheet: "numbers", glyphSize: [9, 13], map: NUMBERS_MAP, tracking: 3 };
  }

  const windows: Record<string, WindowJson> = {
    main: mainWindow(sheets, fonts, warnings),
    equalizer: eqWindow(sheets, warnings),
    playlist: playlistWindow(sheets, fonts, warnings),
  };
  if (input.sizes) prune(windows, sheets, input.sizes, warnings);

  return {
    manifest: {
      format: "hp-skin/1",
      // Not part of hp-skin/1 - an unknown key is ignored by the validator -
      // and the whole point of it: an imported skin's manifest is derived
      // from its art, so a better importer rewrites it rather than leaving a
      // person with what an older one could manage (D107).
      generator: WSZ_GENERATION,
      name: input.name,
      author: "",
      authoredScale: 1,
      art: "final",
      glow: "baked",
      sheets,
      palette: paletteFrom(pledit),
      viscolor: ramp ?? themeRamp("eyewall"),
      visualizer: { component: "spectrum-bars" },
      fonts,
      windows,
    },
    warnings,
  };
}

// ---- what the sheets actually have (D106) ----

type ElementJson = Record<string, unknown>;
type SetJson = { size: number[]; elements: Record<string, ElementJson>; [k: string]: unknown };
type WindowJson = { elements: Record<string, ElementJson>; shade: SetJson; [k: string]: unknown };

/** Which keys of an element hold a sprite, and whether the element can go on
 * without that one. A button with no art is nothing to look at; a slider with
 * a track but no thumb is still a slider. */
const SPRITE_KEYS: Record<string, { required: string[]; optional: string[] }> = {
  image: { required: ["sprite"], optional: ["inactive"] },
  nineslice: { required: ["sprite"], optional: [] },
  button: { required: ["sprite"], optional: ["hover", "active", "inactive"] },
  toggle: { required: ["sprite"], optional: ["hover", "active", "inactive"] },
  slider: { required: [], optional: ["track", "fill", "thumb"] },
};

const fitsIn = (size: [number, number] | undefined, rect: Rect) =>
  !size || (rect[0] >= 0 && rect[1] >= 0 && rect[0] + rect[2] <= size[0] && rect[1] + rect[3] <= size[1]);

/**
 * Drop the art a skin's sheets do not actually have (D106).
 *
 * Every rectangle in the table is where the classic format puts a sprite, but
 * plenty of skins ship a shorter file: no volume thumb, an equalizer sheet
 * that stops above the sliders, a seek bar five pixels tall. The renderer
 * refuses a manifest whose rect leaves its sheet, and rightly — so the
 * manifest must not claim what is not there. An element that loses art it
 * cannot do without is left out, and the window carries on with the rest.
 */
function prune(
  windows: Record<string, WindowJson>,
  sheets: Record<string, string>,
  sizes: Record<string, [number, number]>,
  warnings: string[],
) {
  const has = (ref: unknown) => {
    const r = ref as { sheet: string; rect: Rect } | undefined;
    if (!r) return true;
    return fitsIn(sizes[sheets[r.sheet]], r.rect);
  };

  for (const [wname, win] of Object.entries(windows)) {
    const lost: string[] = [];
    for (const set of [win as unknown as SetJson, win.shade]) {
      for (const [name, el] of Object.entries(set.elements)) {
        const keys = SPRITE_KEYS[el.type as string];
        if (!keys) continue;
        for (const k of keys.optional) if (k in el && !has(el[k])) delete el[k];
        const on = el.on as Record<string, unknown> | undefined;
        if (on) {
          for (const k of ["hover", "active", "inactive"]) if (k in on && !has(on[k])) delete on[k];
          // A toggle with no `on` art cannot show its other state; it keeps
          // the art it has and stops being a toggle, so a click still works.
          if (!has(on.sprite)) {
            delete el.on;
            if (el.action) {
              el.type = "button";
              delete el.bind;
              delete el.when;
            }
          }
        }
        const gone =
          keys.required.some((k) => !has(el[k])) ||
          (el.type === "slider" && !["track", "fill", "thumb"].some((k) => k in el)) ||
          (el.type === "toggle" && !el.on);
        if (gone) {
          // The curve's box is required (D99); without art it is a plain box
          // and the window draws the curve on the window's own ground.
          if (name === "eqCurveWell") {
            set.elements[name] = { type: "slot", rect: el.rect };
          } else if (name === "titlebar") {
            // Every set needs its drag handle, and a skin without one is not
            // a skin this app can show (skin-manifest.md).
            throw new SkinError(`${wname}: the title bar's art is not in the sheet`);
          } else {
            delete set.elements[name];
            lost.push(name);
          }
        }
      }
    }
    if (lost.length) {
      const shown = lost.slice(0, 6).join(", ");
      warnings.push(
        `${wname}: this skin's sheets stop short of ${shown}${lost.length > 6 ? ` and ${lost.length - 6} more` : ""}`,
      );
    }
  }
}

/** The title bar's four buttons, which every window set repeats. */
function titleButtons(shadeOn: boolean) {
  const s = TITLEBAR_SP;
  return {
    // The classic options menu is where Double Size lived, so its button is
    // this app's 2x (D101). Nothing else in a classic title bar is free.
    zoom: { type: "button", rect: L.options, sprite: sp("titlebar", s.options), active: sp("titlebar", s.optionsDown), action: "zoom" },
    minimize: { type: "button", rect: L.minimize, sprite: sp("titlebar", s.minimize), active: sp("titlebar", s.minimizeDown), action: "minimize" },
    shade: {
      type: "toggle",
      rect: L.shade,
      sprite: sp("titlebar", shadeOn ? s.shadeOn : s.shade),
      active: sp("titlebar", shadeOn ? s.shadeOnDown : s.shadeDown),
      on: { sprite: sp("titlebar", s.shadeOn), active: sp("titlebar", s.shadeOnDown) },
      action: "shade",
    },
    close: { type: "button", rect: L.close, sprite: sp("titlebar", s.close), active: sp("titlebar", s.closeDown), action: "close" },
  };
}

/** A state lamp: the classic draws one indicator, and the window's own
 * background stands in for "not this state", which is how the classic hid the
 * other two. */
function lamp(state: string, rect: Rect) {
  return {
    type: "toggle",
    rect: L.state,
    sprite: sp("main", [L.state[0], L.state[1], L.state[2], L.state[3]] as Rect),
    on: { sprite: sp("playpaus", rect) },
    bind: "playState",
    when: state,
  };
}

function mainWindow(sheets: Record<string, string>, fonts: Record<string, unknown>, warnings: string[]) {
  const has = (s: string) => s in sheets;
  const buttons = titleButtons(false);
  const els: Record<string, ElementJson> = {
    backdrop: { type: "image", rect: [0, 0, 275, 116], sprite: sp("main", MAIN_SP.background) },
    titlebar: {
      type: "image",
      rect: L.titleBar,
      sprite: sp("titlebar", TITLEBAR_SP.barActive),
      inactive: sp("titlebar", TITLEBAR_SP.bar),
      role: "drag",
    },
    ...buttons,
  };

  if (fonts.time) {
    els.clockMinutes = { type: "text", rect: L.clockMinutes, font: "time", bind: "elapsedMinutes" };
    els.clockSeconds = { type: "text", rect: L.clockSeconds, font: "time", bind: "elapsedSeconds" };
  }
  if (fonts.chrome) {
    els.trackTitle = { type: "text", rect: L.title, font: "chrome", bind: "trackTitle", overflow: "scroll" };
    els.kbps = { type: "text", rect: L.kbps, font: "chrome", bind: "kbps" };
    els.khz = { type: "text", rect: L.khz, font: "chrome", bind: "khz" };
  } else {
    // `trackTitle` is where the window puts a track it cannot open, so the
    // format requires it (D99). Without TEXT.BMP there is no font to draw it
    // in; the theme's face stands in until bitmap fonts render.
    els.trackTitle = { type: "text", rect: L.title, font: "system", bind: "trackTitle", overflow: "scroll" };
    fonts.system = { type: "system", size: 6, case: "none", tracking: 0 };
    warnings.push("no TEXT.BMP: the title is drawn in the theme's face");
  }

  els.vis = { type: "visualizer", rect: L.vis };

  if (has("playpaus")) {
    els.tagPlay = lamp("playing", PLAYPAUS_SP.playing);
    els.tagPause = lamp("paused", PLAYPAUS_SP.paused);
    els.tagStop = lamp("stopped", PLAYPAUS_SP.stopped);
  }
  if (has("posbar")) {
    els.seek = {
      type: "slider",
      rect: L.seek,
      orientation: "horizontal",
      bind: "position",
      track: sp("posbar", POSBAR_SP.track),
      thumb: sp("posbar", POSBAR_SP.thumb),
    };
  }
  if (has("volume")) {
    els.volume = {
      type: "slider",
      rect: L.volume,
      orientation: "horizontal",
      bind: "volume",
      fill: sp("volume", VOLUME_SP.fill),
      thumb: sp("volume", VOLUME_SP.thumb),
    };
  }

  const c = CBUTTONS_SP;
  els.prev = { type: "button", rect: L.prev, sprite: sp("cbuttons", c.prev), active: sp("cbuttons", c.prevDown), action: "prev" };
  els.play = { type: "button", rect: L.play, sprite: sp("cbuttons", c.play), active: sp("cbuttons", c.playDown), action: "play" };
  els.pause = { type: "button", rect: L.pause, sprite: sp("cbuttons", c.pause), active: sp("cbuttons", c.pauseDown), action: "pause" };
  els.stop = { type: "button", rect: L.stop, sprite: sp("cbuttons", c.stop), active: sp("cbuttons", c.stopDown), action: "stop" };
  els.next = { type: "button", rect: L.next, sprite: sp("cbuttons", c.next), active: sp("cbuttons", c.nextDown), action: "next" };
  els.eject = { type: "button", rect: L.eject, sprite: sp("cbuttons", c.eject), active: sp("cbuttons", c.ejectDown), action: "eject" };

  if (has("shufrep")) {
    const s = SHUFREP_SP;
    els.eqButton = {
      type: "toggle",
      rect: L.eqButton,
      sprite: sp("shufrep", s.eq),
      active: sp("shufrep", s.eqDown),
      on: { sprite: sp("shufrep", s.eqOn) },
      action: "eq",
      bind: "eqOpen",
      when: "on",
    };
    els.plButton = {
      type: "toggle",
      rect: L.plButton,
      sprite: sp("shufrep", s.playlist),
      active: sp("shufrep", s.playlistDown),
      on: { sprite: sp("shufrep", s.playlistOn) },
      action: "playlist",
      bind: "plOpen",
      when: "on",
    };
    els.shuffleButton = {
      type: "toggle",
      rect: L.shuffle,
      sprite: sp("shufrep", s.shuffle),
      active: sp("shufrep", s.shuffleDown),
      on: { sprite: sp("shufrep", s.shuffleOn), active: sp("shufrep", s.shuffleOnDown) },
      action: "shuffle",
      bind: "shuffle",
      when: "on",
    };
    els.repeatButton = {
      type: "toggle",
      rect: L.repeat,
      sprite: sp("shufrep", s.repeat),
      active: sp("shufrep", s.repeatDown),
      on: { sprite: sp("shufrep", s.repeatOn), active: sp("shufrep", s.repeatOnDown) },
      action: "repeat",
      bind: "repeatOn",
      when: "on",
    };
  } else {
    warnings.push("no SHUFREP.BMP: no shuffle, repeat, EQ or playlist buttons on Main");
  }

  const shadeButtons = titleButtons(true);
  return {
    size: [275, 116],
    resizable: false,
    elements: els,
    shade: {
      size: [275, 14],
      elements: {
        titlebar: {
          type: "image",
          rect: L.titleBar,
          sprite: sp("titlebar", TITLEBAR_SP.shadeBarActive),
          inactive: sp("titlebar", TITLEBAR_SP.shadeBar),
          role: "drag",
        },
        zoom: shadeButtons.zoom,
        minimize: shadeButtons.minimize,
        shade: shadeButtons.shade,
        close: shadeButtons.close,
      },
    },
  };
}

function eqWindow(sheets: Record<string, string>, warnings: string[]) {
  const e = EQMAIN_SP;
  if (!("eqmain" in sheets)) {
    // Every window set is required (skin-manifest.md), so a skin without an
    // equalizer gets its title bar from the main sheet and nothing else.
    warnings.push("no EQMAIN.BMP: the equalizer wears Main's title bar and has no controls");
  }
  const sheet = "eqmain" in sheets ? "eqmain" : "titlebar";
  const bar = "eqmain" in sheets ? e.barActive : TITLEBAR_SP.barActive;
  const barIdle = "eqmain" in sheets ? e.bar : TITLEBAR_SP.bar;
  const els: Record<string, ElementJson> = {};
  if ("eqmain" in sheets) {
    els.backdrop = { type: "image", rect: [0, 0, 275, 116], sprite: sp("eqmain", e.background) };
  }
  els.titlebar = { type: "image", rect: L.titleBar, sprite: sp(sheet, bar), inactive: sp(sheet, barIdle), role: "drag" };

  if ("eqmain" in sheets) {
    els.close = { type: "button", rect: L.close, sprite: sp("eqmain", e.close), active: sp("eqmain", e.closeDown), action: "close" };
    els.eqOnButton = {
      type: "toggle",
      rect: L.eqOn,
      sprite: sp("eqmain", e.on),
      active: sp("eqmain", e.onDown),
      on: { sprite: sp("eqmain", e.onSelected), active: sp("eqmain", e.onSelectedDown) },
      action: "eqOn",
      bind: "eqOn",
      when: "on",
    };
    els.eqPresetButton = {
      type: "toggle",
      rect: L.eqPresets,
      sprite: sp("eqmain", e.presets),
      active: sp("eqmain", e.presetsDown),
      on: { sprite: sp("eqmain", e.presetsDown) },
      action: "eqPresets",
      bind: "eqMenu",
      when: "open",
    };
    els.eqCurveWell = { type: "image", rect: L.eqGraph, sprite: sp("eqmain", e.graph) };
    const slider = (rect: Rect, bind: string) => ({
      type: "slider",
      rect,
      orientation: "vertical",
      bind,
      origin: 0.5,
      thumb: sp("eqmain", e.thumb),
    });
    els.eqPre = slider(L.eqPreamp, "eqPre");
    for (let i = 0; i < 10; i++) {
      els[`eqBand${i + 1}`] = slider(
        [L.eqBandX + i * L.eqBandStep, L.eqBandY, L.eqBandW, L.eqBandH] as Rect,
        `eqBand${i + 1}`,
      );
    }
  } else {
    // The curve's box is required (D99); with no equalizer art it is the
    // window's own middle, and the window draws the curve on the ground.
    els.eqCurveWell = { type: "slot", rect: L.eqGraph };
  }

  return {
    size: [275, 116],
    resizable: false,
    elements: els,
    shade: {
      size: [275, 14],
      elements: {
        titlebar: { type: "image", rect: L.titleBar, sprite: sp(sheet, bar), inactive: sp(sheet, barIdle), role: "drag" },
        shade: titleButtons(true).shade,
        close: { type: "button", rect: L.close, sprite: sp("titlebar", TITLEBAR_SP.close), active: sp("titlebar", TITLEBAR_SP.closeDown), action: "close" },
      },
    },
  };
}

function playlistWindow(sheets: Record<string, string>, fonts: Record<string, unknown>, warnings: string[]) {
  const p = PLEDIT_SP;
  const hasArt = "pledit" in sheets;
  if (!hasArt) warnings.push("no PLEDIT.BMP: the playlist wears Main's title bar over the theme's ground");
  const top = L.plTopHeight;
  const els: Record<string, ElementJson> = {};

  if (hasArt) {
    els.topLeft = { type: "image", rect: [0, 0, 25, top], sprite: sp("pledit", p.topLeft) };
    els.topRight = { type: "image", rect: [250, 0, 25, top], anchor: "right", sprite: sp("pledit", p.topRight) };
    els.titlebar = {
      type: "image",
      rect: [25, 0, 225, top],
      stretch: "x",
      sprite: sp("pledit", p.topTitle),
      inactive: sp("pledit", p.topTitleIdle),
      role: "drag",
    };
    els.leftTile = { type: "image", rect: [0, top, 12, 58], stretch: "y", sprite: sp("pledit", p.leftTile) };
    els.rightTile = { type: "image", rect: [255, top, 20, 58], anchor: "right", stretch: "y", sprite: sp("pledit", p.rightTile) };
    els.bottomLeft = { type: "image", rect: [0, 78, 125, 38], anchor: "bottom", sprite: sp("pledit", p.bottomLeft) };
    // The classic's bottom-right block carries the running time and the grip,
    // and wants the corner rather than an edge (D103).
    els.bottomRight = {
      type: "image",
      rect: [125, 78, 150, 38],
      anchor: "bottom-right",
      sprite: sp("pledit", p.bottomRight),
    };
    els.shade = {
      type: "toggle",
      rect: L.plShade,
      anchor: "right",
      sprite: sp("pledit", p.collapse),
      on: { sprite: sp("pledit", p.collapse) },
      action: "shade",
    };
  } else {
    els.titlebar = {
      type: "image",
      rect: L.titleBar,
      sprite: sp("titlebar", TITLEBAR_SP.barActive),
      inactive: sp("titlebar", TITLEBAR_SP.bar),
      role: "drag",
      stretch: "x",
    };
  }

  const listTop = hasArt ? top : 14;
  const listHeight = 116 - listTop - (hasArt ? L.plBottomHeight : 13);
  els.list = {
    type: "list",
    // Between the two tiled edges, 12 and 20 wide, as the classic's rows sit.
    rect: [hasArt ? L.plLeftWidth : 4, listTop, hasArt ? 275 - L.plLeftWidth - L.plRightWidth : 267, listHeight],
    stretch: "xy",
    rowHeight: 13,
    font: fonts.chrome ? "chrome" : "system",
    tint: "text",
    current: "alert",
    selected: "accent",
  };
  if (!fonts.chrome && !fonts.system) fonts.system = { type: "system", size: 6, case: "none", tracking: 0 };

  if (hasArt) {
    // The classic's bottom bar is five buttons that opened menus. This app
    // has no menus, so each maps to the one thing its menu was mostly for,
    // and a press shows that menu item's own glyph (D103). The select and
    // misc menus have nothing here to be, and stay as the art they are drawn
    // into.
    const barButton = (rect: Rect, patch: Rect, glyph: Rect, action: string, anchor?: string) => ({
      type: "button",
      rect,
      ...(anchor ? { anchor } : {}),
      sprite: sp("pledit", patch),
      active: sp("pledit", glyph),
      action,
    });
    els.addButton = barButton(L.plAdd, [14, 80, 22, 18], p.addDir, "add", "bottom");
    els.removeButton = barButton(L.plRemove, [43, 80, 22, 18], p.removeSelected, "remove", "bottom");
    els.libraryButton = barButton(L.plList, [232, 80, 22, 18], p.loadList, "library", "bottom-right");
    // No URL button: the classic's Add was a menu, and a link is added from
    // the library window. Nothing of the skin is lost, so this is a line in
    // the docs rather than a warning on every import.
    els.listStatus = { type: "slot", rect: L.plStatus, anchor: "bottom-right" };
    els.urlField = { type: "slot", rect: L.plUrl, anchor: "bottom", stretch: "x" };
  } else {
    els.listStatus = { type: "slot", rect: [150, 103, 110, 12], anchor: "bottom", stretch: "x" };
    els.urlField = { type: "slot", rect: [4, 103, 267, 12], anchor: "bottom", stretch: "x" };
  }

  return {
    size: [275, 116],
    resizable: true,
    resizeStep: [25, 29],
    minSize: [275, 116],
    elements: els,
    shade: {
      size: [275, 14],
      elements: {
        titlebar: {
          type: "image",
          rect: L.titleBar,
          stretch: "x",
          sprite: hasArt ? sp("pledit", p.shadeBar) : sp("titlebar", TITLEBAR_SP.shadeBarActive),
          role: "drag",
        },
        shade: titleButtons(true).shade,
      },
    },
  };
}
