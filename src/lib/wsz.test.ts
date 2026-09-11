import { describe, expect, it } from "vitest";
import { parseSkin, SkinError, type Element } from "./skin";
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
    expect(els.clock).toMatchObject({ rect: [36, 26, 63, 13], font: "time", bind: "elapsed" });
    // The two the play order drives, which the classic had and this app
    // gained at v0.4b (D97).
    expect(els.shuffleButton).toMatchObject({ action: "shuffle", bind: "shuffle", when: "on" });
    expect(els.repeatButton).toMatchObject({ action: "repeat", bind: "repeatOn", when: "on" });
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

  it("takes the playlist's colours from PLEDIT.TXT and the ramp from VISCOLOR.TXT", () => {
    const { skin } = parseSkin(make().manifest);
    // D101: the theme paints what it can tint; a skin that brings its own
    // pixels brings the colours they sit beside.
    expect(skin.palette).toMatchObject({
      void: SKIN_COLOURS.bg,
      filament: SKIN_COLOURS.normal,
      strike: SKIN_COLOURS.current,
      arc: SKIN_COLOURS.selected,
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
    expect(skin.palette.filament).toBe(colorsFor("eyewall").filament);
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
    expect(only.filament).toBe(SKIN_COLOURS.normal);
    expect(only.void).toBe(colorsFor("eyewall").void);
  });

  it("reads a 24-colour ramp, comments and all", () => {
    const ramp = parseViscolor("// the ramp\n" + VISCOLOR_TXT + "\n255,255,255 // a 25th nobody asked for");
    expect(ramp).toHaveLength(24);
    expect(ramp![23]).toBe(hexOf(rampStep(23)));
  });

  it("is not a ramp with 23 colours", () => {
    expect(parseViscolor(Array.from({ length: 23 }, () => "1,2,3").join("\n"))).toBeNull();
  });
});
