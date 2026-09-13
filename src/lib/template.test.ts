import { describe, expect, it } from "vitest";
import eyewall from "../../skins/eyewall/manifest.json";
import { remade } from "./madeskin";
import { checkSheetBounds, elementsOf, parseSkin, sprites, TOKENS, WINDOWS } from "./skin";
import { colorsFor } from "./theme";
import { TEMPLATE_SCALE, templateManifest, templateParts, templateReadme } from "./template";

const palette = colorsFor("eyewall") as Record<(typeof TOKENS)[number], string>;

describe("a skin to paint (#146)", () => {
  it("is a manifest the validator accepts, with no warnings", () => {
    const { warnings } = parseSkin(templateManifest(palette));
    expect(warnings).toEqual([]);
  });

  it("is Eyewall's layout, in full colour, wearing its own six colours", () => {
    const { skin } = parseSkin(templateManifest(palette, "Storm Paint"));
    expect(skin.name).toBe("Storm Paint");
    expect(skin.art).toBe("final");
    expect(skin.colors).toBe("own");
    for (const t of TOKENS) expect(skin.palette[t]).toBe(palette[t]);
    const ew = parseSkin(eyewall).skin;
    for (const w of WINDOWS) {
      const names = elementsOf(skin, w, false).elements.map((e) => e.name);
      expect(names).toEqual(elementsOf(ew, w, false).elements.map((e) => e.name));
    }
  });

  it("draws from one sheet at double size, and every rectangle fits it", () => {
    const { skin } = parseSkin(templateManifest(palette));
    expect(skin.sheets).toEqual({ chrome: { [TEMPLATE_SCALE]: "chrome.png" } });
    // Eyewall's 2x sheet is 550 x 490; the painted one is the same size.
    expect(checkSheetBounds(skin, { "chrome.png": { w: 550, h: 490 } })).toEqual([]);
  });

  // D124 rebuilds a made skin from Eyewall's layout on every wear; a painted
  // skin's layout is the person's, and must never be.
  it("is never rebuilt as a made skin or re-imported as a .wsz", () => {
    const m = templateManifest(palette);
    expect(m.maker).toBeUndefined();
    expect(m.generator).toBeUndefined();
    expect(remade(JSON.parse(JSON.stringify(m)))).toBeNull();
  });

  it("numbers every rectangle of the sheet once, with every element that draws from it", () => {
    const parts = templateParts();
    const skin = parseSkin(eyewall).skin;
    const rects = new Set(sprites(skin).map((s) => s.ref.rect.join(",")));
    expect(parts).toHaveLength(rects.size);
    expect(parts.map((p) => p.n)).toEqual(parts.map((_, i) => i + 1));
    // Reading order: by row, then along it.
    for (let i = 1; i < parts.length; i++) {
      const [a, b] = [parts[i - 1].rect, parts[i].rect];
      expect(a[1] < b[1] || (a[1] === b[1] && a[0] <= b[0])).toBe(true);
    }
    // A shared rectangle lists everyone who uses it, in words a person reads.
    const shared = parts.find((p) => p.names.length > 1)!;
    expect(shared).toBeDefined();
    for (const n of shared.names) expect(n).not.toMatch(/windows\.|elements/);
  });

  it("gives a rectangle worn in two colours the first one's", () => {
    const parts = templateParts();
    const skin = parseSkin(eyewall).skin;
    for (const p of parts) {
      const first = sprites(skin).find((s) => s.ref.rect.join(",") === p.rect.join(","))!;
      expect(p.tint).toBe(first.ref.tint);
    }
  });

  it("says how to paint it and how to bring it back", () => {
    const text = templateReadme();
    expect(text).toContain("chrome.png");
    expect(text).toContain("guide.png");
    expect(text).toContain("manifest.json");
    expect(text).toContain("Import skin");
    expect(text).toContain("\r\n");
  });
});
