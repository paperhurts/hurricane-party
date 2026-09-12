import { describe, expect, it } from "vitest";
import { elementsOf, parseSkin, placeRect, SkinError, type Element } from "./skin";
import { colorsFor } from "./theme";
import { parsePledit, parseViscolor, paletteFrom, wszManifest } from "./wsz";

/** The fixture skin's own colours: a made-up skin's PLEDIT.TXT, never the
 * theme's, which is the whole point of the tests below. */
// tokens-exempt: the input under test is a classic skin's own colour file
const SKIN_COLOURS = { normal: "#00FF00", current: "#FFFFFF", bg: "#000000", selected: "#0000FF" }; // tokens-exempt: as above

/** Every file a well-stocked classic skin ships, in the mixed case they
 * arrive in and nested in a folder, as zips from the wild are. */
const FULL = [
  "MyCoolSkin/MAIN.BMP",
  "MyCoolSkin/TITLEBAR.BMP",
  "MyCoolSkin/cbuttons.bmp",
  "MyCoolSkin/PosBar.bmp",
  "MyCoolSkin/VOLUME.BMP",
  "MyCoolSkin/PLAYPAUS.BMP",
  "MyCoolSkin/SHUFREP.BMP",
  "MyCoolSkin/TEXT.BMP",
  "MyCoolSkin/NUMBERS.BMP",
  "MyCoolSkin/EQMAIN.BMP",
  "MyCoolSkin/PLEDIT.BMP",
  "MyCoolSkin/PLEDIT.TXT",
  "MyCoolSkin/VISCOLOR.TXT",
  "MyCoolSkin/README.TXT",
];

const PLEDIT_TXT = [
  "[Text]",
  `Normal=${SKIN_COLOURS.normal}`,
  `Current=${SKIN_COLOURS.current}`,
  `NormalBG=${SKIN_COLOURS.bg}`,
  `SelectedBG=${SKIN_COLOURS.selected}`,
  `mbFG=${SKIN_COLOURS.normal}`,
].join("\r\n");

/** A ramp whose every step is computable, so the test never spells a colour. */
const rampStep = (i: number) => [i, i * 2, i * 3];
const hexOf = (rgb: number[]) =>
  "#" + rgb.map((n) => n.toString(16).padStart(2, "0").toUpperCase()).join("");
const VISCOLOR_TXT = Array.from({ length: 24 }, (_, i) => `${rampStep(i).join(",")} // ${i}`).join("\n");

const make = (files: string[] = FULL, extra: { pledit?: string; viscolor?: string } = {}) =>
  wszManifest({
    files,
    name: "My Cool Skin",
    pledit: files.some((f) => /pledit\.txt$/i.test(f)) ? (extra.pledit ?? PLEDIT_TXT) : undefined,
    viscolor: files.some((f) => /viscolor\.txt$/i.test(f)) ? (extra.viscolor ?? VISCOLOR_TXT) : undefined,
  });

// eslint has no home here; `any` is the manifest before it is parsed.
const elementsOfWindow = (manifest: Record<string, any>, w: string) => manifest.windows[w].elements;

describe("a classic skin becomes an hp-skin/1 manifest", () => {
  it("passes the validator that Eyewall passes, with nothing to warn about", () => {
    const { manifest, warnings } = make();
    expect(warnings).toEqual([]);
    const { skin, warnings: parseWarnings } = parseSkin(manifest);
    expect(parseWarnings).toEqual([]);
    expect(skin.name).toBe("My Cool Skin");
    // Its pixels are its own, and the renderer adds no halo to them (D73).
    expect(skin.art).toBe("final");
    expect(skin.glow).toBe("baked");
  });

  it("finds its sheets whatever the case and however deep in the zip", () => {
    const { manifest } = make();
    expect(manifest.sheets).toMatchObject({
      main: "main.bmp",
      titlebar: "titlebar.bmp",
      cbuttons: "cbuttons.bmp",
      posbar: "posbar.bmp",
      pledit: "pledit.bmp",
    });
    // NUMS_EX is the same digits plus a blank, so it wins when both are there.
    const { manifest: ex } = make([...FULL, "MyCoolSkin/NUMS_EX.BMP"]);
    expect((ex.sheets as Record<string, string>).numbers).toBe("nums_ex.bmp");
  });

  it("puts Main's controls where the classic put them", () => {
    const { manifest } = make();
    const els = elementsOfWindow(manifest, "main");
    expect(els.titlebar).toMatchObject({ rect: [0, 0, 275, 14], role: "drag" });
    expect(els.play).toMatchObject({ rect: [39, 88, 23, 18], action: "play" });
    expect(els.play.sprite).toEqual({ sheet: "cbuttons", rect: [23, 0, 23, 18] });
    expect(els.eject).toMatchObject({ rect: [136, 89, 22, 16], action: "eject" });
    expect(els.seek).toMatchObject({ rect: [16, 72, 248, 10], bind: "position" });
    expect(els.vis).toMatchObject({ rect: [24, 43, 76, 16] });
    // Four digits with the colon painted into the window between them, so
    // the clock is two elements, each two glyphs 3 apart (D104).
    expect(els.clockMinutes).toMatchObject({ rect: [48, 26, 21, 13], font: "time", bind: "elapsedMinutes" });
    expect(els.clockSeconds).toMatchObject({ rect: [78, 26, 21, 13], font: "time", bind: "elapsedSeconds" });
    // The two the play order drives, which the classic had and this app
    // gained at v0.4b (D97).
    expect(els.shuffleButton).toMatchObject({ action: "shuffle", bind: "shuffle", when: "on" });
    expect(els.repeatButton).toMatchObject({ action: "repeat", bind: "repeatOn", when: "on" });
    // And the two that show and hide a window, lit while it is on screen
    // (D109). Eyewall draws neither, so nothing bound them until a .wsz did.
    expect(els.eqButton).toMatchObject({ action: "eq", bind: "eqOpen", when: "on" });
    expect(els.plButton).toMatchObject({ action: "playlist", bind: "plOpen", when: "on" });
  });

  it("makes the windowshade strip's painted controls answer a press (D111)", () => {
    const { skin } = parseSkin(make().manifest);
    const shade = elementsOf(skin, "main", true).elements;
    const by = (n: string) => shade.find((e) => e.name === n);
    // The classic painted these into the strip and hit-tested rectangles over
    // them, so each draws the strip's own pixels and has no pressed art.
    expect(by("shadePlay")).toMatchObject({ type: "button", rect: [176, 2, 10, 10], action: "play" });
    expect((by("shadePlay") as { sprite: { sheet: string; rect: number[] } }).sprite).toEqual({
      sheet: "titlebar",
      rect: [203, 31, 10, 10],
      tint: "text",
    });
    expect(by("shadePrev")).toMatchObject({ action: "prev" });
    expect(by("shadeStop")).toMatchObject({ action: "stop" });
    expect(by("shadeEject")).toMatchObject({ action: "eject" });
    // The seek bar is the one control here the classic gave its own art.
    expect(by("shadeSeek")).toMatchObject({ type: "slider", rect: [226, 4, 17, 7], bind: "position" });
    // And the strip shows what it always showed: the time, and the analyser.
    expect(by("clockMinutes")).toMatchObject({ font: "chrome", bind: "elapsedMinutes" });
    expect(by("vis")).toMatchObject({ type: "visualizer", rect: [79, 5, 38, 5] });
  });

  it("declares its fonts as glyph grids, the clock's digits 3 apart", () => {
    const { skin } = parseSkin(make().manifest);
    expect(skin.fonts.time).toMatchObject({ type: "bitmap", sheet: "numbers", glyphSize: [9, 13], tracking: 3 });
    expect(skin.fonts.chrome).toMatchObject({ type: "bitmap", sheet: "text", glyphSize: [5, 6], tracking: 0 });
    const chrome = skin.fonts.chrome;
    if (chrome.type !== "bitmap") throw new Error("chrome font is not a bitmap font");
    // Three rows of 31, the classic sheet's own grid: letters, then digits
    // and punctuation, then the three Nordic vowels and two marks.
    expect(chrome.map).toHaveLength(93);
    expect(chrome.map.slice(0, 26)).toBe("abcdefghijklmnopqrstuvwxyz");
    expect(chrome.map.slice(31, 41)).toBe("0123456789");
  });

  it("hides the state lamps it is not showing behind the window's own pixels", () => {
    const { manifest } = make();
    const els = elementsOfWindow(manifest, "main");
    // One lamp per state, all in the same 9x9 box: the one whose state holds
    // shows its art, and the others show the background from MAIN.BMP, which
    // is what the classic drew there.
    for (const [name, when] of [
      ["tagPlay", "playing"],
      ["tagPause", "paused"],
      ["tagStop", "stopped"],
    ]) {
      expect(els[name], name).toMatchObject({
        rect: [26, 28, 9, 9],
        bind: "playState",
        when,
        sprite: { sheet: "main", rect: [26, 28, 9, 9] },
      });
      expect(els[name].on.sprite.sheet).toBe("playpaus");
    }
  });

  it("gives the equalizer its eleven sliders, centred on 0 dB", () => {
    const { skin } = parseSkin(make().manifest);
    const els = skin.windows.equalizer.full.elements;
    const sliders = els.filter((e: Element) => e.type === "slider");
    expect(sliders.map((s) => s.type === "slider" && s.bind)).toEqual([
      "eqPre",
      ...Array.from({ length: 10 }, (_, i) => `eqBand${i + 1}`),
    ]);
    for (const s of sliders) {
      expect(s).toMatchObject({ orientation: "vertical", origin: 0.5 });
    }
    // The bands step 18 apart, as the classic window does.
    const band1 = els.find((e: Element) => e.name === "eqBand1")!;
    const band2 = els.find((e: Element) => e.name === "eqBand2")!;
    expect(band2.rect[0] - band1.rect[0]).toBe(18);
    expect(els.find((e: Element) => e.name === "eqOnButton")).toMatchObject({ action: "eqOn", bind: "eqOn" });
  });

  it("gives the playlist its tiled frame, its bar and the corner that holds both edges", () => {
    const { skin } = parseSkin(make().manifest);
    const els = skin.windows.playlist.full.elements;
    const by = (n: string) => els.find((e: Element) => e.name === n);
    // The frame: corners at their size, the title and the edges tiling, and
    // the bottom-right block on the corner rather than an edge (D103).
    expect(by("topRight")).toMatchObject({ rect: [250, 0, 25, 20], anchor: "right" });
    expect(by("titlebar")).toMatchObject({ rect: [25, 0, 225, 20], stretch: "x", role: "drag" });
    expect(by("leftTile")).toMatchObject({ rect: [0, 20, 12, 58], stretch: "y" });
    expect(by("rightTile")).toMatchObject({ anchor: "right", stretch: "y" });
    expect(by("bottomLeft")).toMatchObject({ rect: [0, 78, 125, 38], anchor: "bottom" });
    expect(by("bottomRight")).toMatchObject({ rect: [125, 78, 150, 38], anchor: "bottom-right" });
    // The rows sit between the two tiled edges.
    expect(by("list")).toMatchObject({ rect: [12, 20, 243, 58], stretch: "xy", rowHeight: 13 });

    // The classic's menu buttons become the one thing each menu was for; at
    // rest they are the window's own pixels, and a press shows the menu
    // item's glyph.
    expect(by("addButton")).toMatchObject({ rect: [14, 86, 22, 18], anchor: "bottom", action: "add" });
    expect(by("removeButton")).toMatchObject({ rect: [43, 86, 22, 18], anchor: "bottom", action: "remove" });
    expect(by("libraryButton")).toMatchObject({ anchor: "bottom-right", action: "library" });
    const add = by("addButton")!;
    if (add.type !== "button") throw new Error("addButton is not a button");
    expect(add.sprite).toEqual({ sheet: "pledit", rect: [14, 80, 22, 18], tint: "text" });

    // Widened and heightened on the D30 grid, the corner pieces stay in their
    // corners and the rows take the slack.
    const grown: [number, number] = [325, 174];
    expect(placeRect(by("bottomRight")!, [275, 116], grown)).toMatchObject({ x: 175, y: 136 });
    expect(placeRect(by("libraryButton")!, [275, 116], grown)).toMatchObject({ x: 281, y: 144 });
    expect(placeRect(by("list")!, [275, 116], grown)).toMatchObject({ x: 12, y: 20, w: 293, h: 116 });
  });

  it("takes the playlist's colours from PLEDIT.TXT and the ramp from VISCOLOR.TXT", () => {
    const { skin } = parseSkin(make().manifest);
    // D101: the theme paints what it can tint; a skin that brings its own
    // pixels brings the colours they sit beside.
    expect(skin.palette).toMatchObject({
      ground: SKIN_COLOURS.bg,
      text: SKIN_COLOURS.normal,
      alert: SKIN_COLOURS.current,
      accent: SKIN_COLOURS.selected,
    });
    expect(skin.viscolor).toHaveLength(24);
    expect(skin.viscolor[0]).toBe(hexOf(rampStep(0)));
    expect(skin.viscolor[2]).toBe(hexOf(rampStep(2)));
  });

  it("keeps the theme's colours when the skin ships neither text file", () => {
    const files = FULL.filter((f) => !/\.txt$/i.test(f));
    const { manifest, warnings } = make(files);
    const { skin } = parseSkin(manifest);
    expect(warnings).toContain("no PLEDIT.TXT: the playlist's colours come from the theme");
    // Eyewall's own six, so the skin still loads and reads.
    expect(skin.palette.text).toBe(colorsFor("eyewall").text);
    expect(skin.viscolor).toHaveLength(24);
  });

  it("loads a skin with only the three sheets it cannot do without", () => {
    const { manifest, warnings } = make(["main.bmp", "titlebar.bmp", "cbuttons.bmp"]);
    const { skin, warnings: parseWarnings } = parseSkin(manifest);
    expect(parseWarnings).toEqual([]);
    // It says what is missing rather than dropping it silently.
    expect(warnings.join(" ")).toMatch(/SHUFREP/);
    expect(warnings.join(" ")).toMatch(/EQMAIN/);
    expect(warnings.join(" ")).toMatch(/PLEDIT\.BMP/);
    // The transport still works, and every window still has its title bar and
    // the boxes the windows fill (D99).
    const main = skin.windows.main.full.elements.map((e: Element) => e.name);
    expect(main).toEqual(expect.arrayContaining(["titlebar", "play", "vis", "trackTitle"]));
    const pl = skin.windows.playlist.full.elements.map((e: Element) => e.name);
    expect(pl).toEqual(expect.arrayContaining(["titlebar", "list", "listStatus", "urlField"]));
  });

  it("leaves out the art a skin's sheets stop short of (D106)", () => {
    // Real skins ship short sheets: no volume thumb, an equalizer that stops
    // above the sliders, a seek bar five pixels tall. The manifest must not
    // claim what is not there, or the renderer refuses the whole skin.
    const { manifest, warnings } = wszManifest({
      files: FULL.map((f) => f.split("/").pop()!.toLowerCase()),
      name: "Short Sheets",
      sizes: {
        "main.bmp": [275, 116],
        "titlebar.bmp": [344, 87],
        "cbuttons.bmp": [136, 36],
        // No thumb art (the real Pip-Boy skins), and a seek bar half height
        // (Super Mario Land): the volume keeps its fill, the seek goes.
        "volume.bmp": [68, 422],
        "posbar.bmp": [308, 5],
        // An equalizer that stops above its sliders (Disgaea, Etna).
        "eqmain.bmp": [275, 163],
        "playpaus.bmp": [48, 9],
        "shufrep.bmp": [92, 85],
        "text.bmp": [155, 18],
        "numbers.bmp": [108, 13],
        "pledit.bmp": [280, 186],
      },
    });
    const { skin, warnings: parseWarnings } = parseSkin(manifest);
    expect(parseWarnings).toEqual([]);

    const main = skin.windows.main.full.elements;
    const byName = (els: Element[], n: string) => els.find((e) => e.name === n);
    // The volume keeps its fill and loses only the thumb.
    const volume = byName(main, "volume")!;
    if (volume.type !== "slider") throw new Error("volume is not a slider");
    expect(volume.fill).toBeDefined();
    expect(volume.thumb).toBeUndefined();
    // The seek bar had nothing left, so it is not there at all.
    expect(byName(main, "seek")).toBeUndefined();
    // Everything whose art is where the format says is untouched.
    expect(byName(main, "play")).toBeDefined();
    expect(byName(main, "titlebar")).toBeDefined();

    // The equalizer keeps its background, title bar and switch, and loses the
    // controls the sheet stops short of.
    const eq = skin.windows.equalizer.full.elements;
    expect(byName(eq, "backdrop")).toBeDefined();
    expect(byName(eq, "eqOnButton")).toBeDefined();
    expect(byName(eq, "eqPresetButton")).toBeUndefined();
    expect(byName(eq, "eqBand1")).toBeUndefined();
    // The curve's box is required (D99), so it stays as a box with no art.
    expect(byName(eq, "eqCurveWell")).toMatchObject({ type: "slot", rect: [86, 17, 113, 19] });

    // And it says what it left out, per window.
    expect(warnings.join(" ")).toMatch(/main: this skin's sheets stop short of .*seek/);
    expect(warnings.join(" ")).toMatch(/equalizer: this skin's sheets stop short of/);
  });

  it("keeps every rectangle when the sheets have not been measured", () => {
    // The pure mapping is still pure: with no sizes, nothing is dropped.
    const { manifest } = make();
    const main = (manifest.windows as any).main.elements;
    expect(main.seek).toBeDefined();
    expect(main.volume.thumb).toBeDefined();
  });

  it("refuses a skin whose title bar art is not in the sheet", () => {
    // Every element set needs its drag handle (skin-manifest.md), so this one
    // is a refusal rather than a window nobody can move.
    expect(() =>
      wszManifest({
        files: ["main.bmp", "titlebar.bmp", "cbuttons.bmp"],
        name: "No Title Bar",
        sizes: { "main.bmp": [275, 116], "titlebar.bmp": [344, 10], "cbuttons.bmp": [136, 36] },
      }),
    ).toThrow(/title bar/);
  });

  it("refuses a zip that is not a skin", () => {
    expect(() => make(["readme.txt", "cover.jpg"])).toThrow(SkinError);
    expect(() => make(["main.bmp", "titlebar.bmp"])).toThrow(/CBUTTONS\.BMP/);
  });
});

describe("the two text files", () => {
  it("reads PLEDIT.TXT whatever the case and with or without the hash", () => {
    const p = parsePledit(
      `[Text]\nnormal=${SKIN_COLOURS.normal.slice(1).toLowerCase()}\r\nCURRENT = ${SKIN_COLOURS.current}\nrubbish\nSelectedBG=${SKIN_COLOURS.selected}`,
    );
    expect(p).toEqual({
      normal: SKIN_COLOURS.normal,
      current: SKIN_COLOURS.current,
      selectedbg: SKIN_COLOURS.selected,
    });
  });

  it("falls back for each colour the file leaves out", () => {
    const only = paletteFrom({ normal: SKIN_COLOURS.normal });
    expect(only.text).toBe(SKIN_COLOURS.normal);
    expect(only.ground).toBe(colorsFor("eyewall").ground);
  });

  it("reads a 24-colour ramp, comments and all", () => {
    const ramp = parseViscolor("// the ramp\n" + VISCOLOR_TXT + "\n255,255,255 // a 25th nobody asked for");
    expect(ramp).toHaveLength(24);
    expect(ramp![23]).toBe(hexOf(rampStep(23)));
  });

  it("fills a short ramp with its own last colour, and trims a long one", () => {
    // Of thirteen real skins, seven did not ship 24 (D106): 23 and 25 are
    // both normal, and a ramp is too visible to throw away over one line.
    const short = parseViscolor(Array.from({ length: 23 }, (_, i) => `${i},0,0`).join("\n"))!;
    expect(short).toHaveLength(24);
    expect(short[23]).toBe(short[22]);
    const long = parseViscolor(Array.from({ length: 25 }, (_, i) => `${i},0,0`).join("\n"))!;
    expect(long).toHaveLength(24);
    expect(long[0]).toBe(hexOf([0, 0, 0]));
  });

  it("is not a ramp with no colours at all", () => {
    expect(parseViscolor("// just a comment\nand some words")).toBeNull();
  });
});
