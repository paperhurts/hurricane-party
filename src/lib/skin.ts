// hp-skin/1: the native skin format (docs/skin-manifest.md, D36). This module
// is the pure half of the renderer: parse and validate a manifest, refuse it
// rather than half-load it, and answer the geometry questions the windows ask.
// Nothing here touches the DOM, so all of it is under test; the half that
// decodes sheets and slices sprites is skinsheet.ts.
//
// Both importers (.wsz, .wal; v0.5) map *into* this shape, so a gap found
// here is a gap they would inherit. Four surfaced while Eyewall was written
// (D92): `tint` on a sprite for `art: mask`, a `system` font, a sheet per
// scale, and `anchor`/`stretch` for a resizable window.

export const SKIN_FORMAT = "hp-skin/1";

export const TOKENS = ["void", "well", "filament", "arc", "strike", "ember"] as const;
export type Token = (typeof TOKENS)[number];

export const WINDOWS = ["main", "equalizer", "playlist"] as const;
export type WindowName = (typeof WINDOWS)[number];

/** What a `button`'s `action` may name. The app defines the list; a skin picks
 * from it (D23's rule, applied to chrome). Unknown is a hard failure. */
export const ACTIONS = [
  "minimize",
  "shade",
  "zoom",
  "close",
  "play",
  "pause",
  "stop",
  "prev",
  "next",
  "eject",
  "eq",
  "playlist",
  "shuffle",
  "repeat",
  // The equalizer's own: its on switch, and its preset menu.
  "eqOn",
  "eqPresets",
] as const;
export type Action = (typeof ACTIONS)[number];

/** What a `text`, `slider` or `toggle` may `bind` to. Unknown renders empty
 * and warns. `playState` is one of "playing", "paused", "stopped" — the
 * values a `when` compares against. */
export const BINDS = [
  "windowTitle",
  "trackTitle",
  "elapsed",
  "remaining",
  "kbps",
  "khz",
  "position",
  "volume",
  "volumePercent",
  "balance",
  "playState",
  // The equalizer (D21). `eqOn` is "on" or "off", `eqMenu` "open" or
  // "closed", `eqClip` "on" while the lamp is lit; `eqTrim` is the readout's
  // words; the eleven gains are 0..1 fractions, 0.5 being 0 dB.
  "eqOn",
  "eqPreset",
  "eqMenu",
  "eqTrim",
  "eqClip",
  "eqPre",
  "eqBand1",
  "eqBand2",
  "eqBand3",
  "eqBand4",
  "eqBand5",
  "eqBand6",
  "eqBand7",
  "eqBand8",
  "eqBand9",
  "eqBand10",
] as const;
export type Bind = (typeof BINDS)[number];

/** The visualizer components the app has (D20). An unknown one falls back
 * to the first with a warning: a skin naming a future component still loads. */
export const VISUALIZERS = ["spectrum-bars", "oscilloscope"] as const;

export type Scale = 1 | 2;

/** x, y, w, h in logical skin pixels, always at 1x (D40: logical is the
 * source of truth; a sheet's own scale multiplies at slice time). */
export type Rect = [number, number, number, number];

export type SpriteRef = { sheet: string; rect: Rect; tint: Token };

export type Font =
  | { type: "system"; size: number; case: "upper" | "none"; tracking: number }
  | { type: "bitmap"; sheet: string; glyphSize: [number, number]; map: string };

type Placed = {
  name: string;
  rect: Rect;
  /** Which edge the rect is measured from when the window is bigger than its
   * base size. Absent means the left/top edge, as it always did. */
  anchor?: "right" | "bottom";
  /** Which axes the rect grows along with the window. */
  stretch?: "x" | "y" | "xy";
};

export type Element =
  | (Placed & {
      type: "nineslice";
      /** `rect: "fill"` in the manifest: the frame tracks the whole window and
       * `rect` is ignored. Otherwise the nine slices fill `rect`, which is how
       * a box inside the window (the title strip, the seek bar) gets an edge. */
      fill: boolean;
      sprite: SpriteRef;
      insets: [number, number, number, number];
      opacity: number;
    })
  | (Placed & {
      type: "image";
      sprite: SpriteRef;
      inactive?: SpriteRef;
      role?: "drag";
      /** Over the tint, so one full-alpha sprite serves every strength a skin
       * wants: the same 8x8 ring is the window's frame at 0.3 and a control's
       * edge at 0.14. */
      opacity: number;
    })
  | (Placed & {
      type: "button";
      sprite: SpriteRef;
      hover?: SpriteRef;
      active?: SpriteRef;
      inactive?: SpriteRef;
      action: Action;
    })
  | (Placed & {
      type: "toggle";
      sprite: SpriteRef;
      hover?: SpriteRef;
      active?: SpriteRef;
      inactive?: SpriteRef;
      on: { sprite: SpriteRef; hover?: SpriteRef; active?: SpriteRef; inactive?: SpriteRef };
      /** Clickable when present; the app decides the on state from it. */
      action: Action | null;
      /** State-driven instead: `on` art shows while `bind` equals `when`.
       * A toggle with a bind and no action is an indicator. */
      bind: Bind | null;
      when: string | null;
    })
  | (Placed & {
      type: "text";
      font: string;
      bind: Bind | null;
      /** A literal. `{}` in it is replaced by the bound value, so one element
       * can read "VOL 80". Without a bind it is a static label. */
      value: string | null;
      tint: Token;
      /** 0..1 over the tint, for chrome that reads as dimmed rather than as
       * another colour. Sprite art carries this in its alpha; text cannot. */
      opacity: number;
      /** The theme's static glow, for text a `.wsz` would bake into its
       * glyph art (the clock). Ignored when the skin is `glow: "baked"`. */
      glow: boolean;
      inactive?: { tint: Token };
      /** A second appearance, shown while `bind` equals `when`: the PLAY tag
       * lighting while the transport is playing. */
      lit?: { bind: Bind | null; when: string; tint: Token; opacity: number; glow: boolean };
      overflow: "clip" | "scroll";
      /** Where the line sits in its box. Labels under the EQ's sliders are
       * centred; everything before them read left. */
      align: "left" | "center" | "right";
    })
  | (Placed & {
      type: "slider";
      /** All three are optional and at least one is required: track plus fill
       * plus thumb is a seek bar, fill alone is a level meter. */
      track?: SpriteRef;
      fill?: SpriteRef;
      thumb?: SpriteRef;
      orientation: "horizontal" | "vertical";
      bind: Bind | null;
      /** A centred control, 0..1: the fill runs from here to the value rather
       * than from the start, the wheel nudges it, and a double-click returns
       * it here. The EQ's gains sit at 0.5, which is 0 dB. Null for a seek
       * bar or a level, which run from nothing. */
      origin: number | null;
      /** A second look for the fill and thumb while `bind` equals `when`: the
       * EQ's sliders dim while the EQ is off. */
      lit?: { bind: Bind | null; when: string; tint: Token; opacity: number };
      /** The thumb takes `tint` once the value is more than `beyond` from the
       * origin: a band pushed past 8 dB either way turns `strike`. */
      hot?: { beyond: number; tint: Token };
    })
  | (Placed & { type: "visualizer" })
  | (Placed & { type: "list"; rowHeight: number });

export type ElementSet = { size: [number, number]; elements: Element[] };

export type SkinWindow = {
  size: [number, number];
  resizable: boolean;
  resizeStep?: [number, number];
  minSize?: [number, number];
  full: ElementSet;
  shade: ElementSet;
};

export type Skin = {
  name: string;
  author: string;
  authoredScale: Scale;
  /** Sheet name -> file per scale. A string in the manifest is the file at
   * `authoredScale`; an object lists one per scale. */
  sheets: Record<string, Partial<Record<Scale, string>>>;
  art: "final" | "mask";
  glow: "baked" | "renderer";
  palette: Record<Token, string>;
  viscolor: string[];
  visualizer: { component: string; options: Record<string, unknown> };
  fonts: Record<string, Font>;
  windows: Record<WindowName, SkinWindow>;
  seam: {
    thickness: number;
    hoverThickness: number;
    color: Token;
    discharge: { durationMs: number; peakThickness: number };
  };
};

export class SkinError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SkinError";
  }
}

/** Classic geometry the format fixes (skin-manifest.md "Windows", D30). */
export const CLASSIC_SIZE: [number, number] = [275, 116];
export const SHADE_SIZE: [number, number] = [275, 14];
export const PLAYLIST_STEP: [number, number] = [25, 29];

/** A decoded sheet must fit in memory and in a texture. */
export const MAX_SHEET_SIDE = 4096;
export const MAX_SHEET_BYTES = 64 * 1024 * 1024;

// ---- validation ----

type Obj = Record<string, unknown>;

function isObj(v: unknown): v is Obj {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function fail(path: string, what: string): never {
  throw new SkinError(`${path}: ${what}`);
}

function str(o: Obj, key: string, path: string): string {
  const v = o[key];
  if (typeof v !== "string" || v === "") fail(`${path}.${key}`, "must be a non-empty string");
  return v;
}

function num(v: unknown, path: string): number {
  if (typeof v !== "number" || !Number.isFinite(v)) fail(path, "must be a number");
  return v;
}

function int(v: unknown, path: string, min = 0): number {
  const n = num(v, path);
  if (!Number.isInteger(n) || n < min) fail(path, `must be an integer >= ${min}`);
  return n;
}

function pair(v: unknown, path: string, min = 0): [number, number] {
  if (!Array.isArray(v) || v.length !== 2) fail(path, "must be [w, h]");
  return [int(v[0], `${path}[0]`, min), int(v[1], `${path}[1]`, min)];
}

function oneOf<T extends string>(v: unknown, allowed: readonly T[], path: string): T {
  if (typeof v !== "string" || !(allowed as readonly string[]).includes(v)) {
    fail(path, `must be one of ${allowed.join(", ")}`);
  }
  return v as T;
}

// A literal colour is expected in a skin file; this checks its shape.
const HEX = /^#[0-9a-fA-F]{6}$/; // tokens-exempt: a pattern, not a colour

function hex(v: unknown, path: string): string {
  if (typeof v !== "string" || !HEX.test(v)) fail(path, "must be #RRGGBB");
  return v;
}

function rect(v: unknown, path: string): Rect {
  if (!Array.isArray(v) || v.length !== 4) fail(path, "must be [x, y, w, h]");
  const r: Rect = [
    int(v[0], `${path}[0]`),
    int(v[1], `${path}[1]`),
    int(v[2], `${path}[2]`, 1),
    int(v[3], `${path}[3]`, 1),
  ];
  return r;
}

function sprite(v: unknown, sheets: Skin["sheets"], path: string): SpriteRef {
  if (!isObj(v)) fail(path, "must be a sprite {sheet, rect}");
  const sheet = str(v, "sheet", path);
  if (!(sheet in sheets)) fail(`${path}.sheet`, `names no sheet (have ${Object.keys(sheets).join(", ")})`);
  const tint = v.tint === undefined ? "filament" : oneOf(v.tint, TOKENS, `${path}.tint`);
  return { sheet, rect: rect(v.rect, `${path}.rect`), tint };
}

function optSprite(o: Obj, key: string, sheets: Skin["sheets"], path: string): SpriteRef | undefined {
  return o[key] === undefined ? undefined : sprite(o[key], sheets, `${path}.${key}`);
}

function placed(o: Obj, path: string): Placed & { name: string } {
  const p: Placed = { name: path.slice(path.lastIndexOf(".") + 1), rect: rect(o.rect, `${path}.rect`) };
  if (o.anchor !== undefined) p.anchor = oneOf(o.anchor, ["right", "bottom"] as const, `${path}.anchor`);
  if (o.stretch !== undefined) p.stretch = oneOf(o.stretch, ["x", "y", "xy"] as const, `${path}.stretch`);
  return p;
}

function element(
  name: string,
  v: unknown,
  skin: Pick<Skin, "sheets" | "fonts">,
  path: string,
  warnings: string[],
): Element {
  if (!isObj(v)) fail(path, "must be an element object");
  const type = oneOf(
    v.type,
    ["nineslice", "image", "button", "toggle", "text", "slider", "visualizer", "list"] as const,
    `${path}.type`,
  );
  // An unknown binding is a soft failure by spec: the element renders empty
  // and the skin still loads, so a manifest written for a later version
  // degrades rather than dying.
  const bindOf = (o: Obj, where: string, required: boolean): Bind | null => {
    if (o.bind === undefined) {
      if (required) fail(`${where}.bind`, "is required");
      return null;
    }
    const b = str(o, "bind", where);
    if ((BINDS as readonly string[]).includes(b)) return b as Bind;
    warnings.push(`${where}.bind: "${b}" is not a binding this app has; it renders empty`);
    return null;
  };
  const unit = (x: unknown, where: string): number => {
    const n = num(x, where);
    if (n < 0 || n > 1) fail(where, "must be between 0 and 1");
    return n;
  };
  const opacityOf = (o: Obj, where: string): number => {
    if (o.opacity === undefined) return 1;
    const n = num(o.opacity, `${where}.opacity`);
    if (n < 0 || n > 1) fail(`${where}.opacity`, "must be between 0 and 1");
    return n;
  };
  const boolOf = (o: Obj, key: string, where: string): boolean => {
    const b = o[key];
    if (b === undefined) return false;
    if (typeof b !== "boolean") fail(`${where}.${key}`, "must be true or false");
    return b;
  };
  switch (type) {
    case "nineslice": {
      const ins = v.insets;
      if (!Array.isArray(ins) || ins.length !== 4) fail(`${path}.insets`, "must be [top, right, bottom, left]");
      const fill = v.rect === "fill";
      return {
        name,
        rect: fill ? [0, 0, 0, 0] : rect(v.rect, `${path}.rect`),
        type,
        fill,
        sprite: sprite(v.sprite, skin.sheets, `${path}.sprite`),
        insets: [
          int(ins[0], `${path}.insets[0]`),
          int(ins[1], `${path}.insets[1]`),
          int(ins[2], `${path}.insets[2]`),
          int(ins[3], `${path}.insets[3]`),
        ],
        opacity: opacityOf(v, path),
      };
    }
    case "image": {
      const e: Element = {
        ...placed(v, path),
        type,
        sprite: sprite(v.sprite, skin.sheets, `${path}.sprite`),
        inactive: optSprite(v, "inactive", skin.sheets, path),
        opacity: opacityOf(v, path),
      };
      if (v.role !== undefined) e.role = oneOf(v.role, ["drag"] as const, `${path}.role`);
      return e;
    }
    case "button":
      return {
        ...placed(v, path),
        type,
        sprite: sprite(v.sprite, skin.sheets, `${path}.sprite`),
        hover: optSprite(v, "hover", skin.sheets, path),
        active: optSprite(v, "active", skin.sheets, path),
        inactive: optSprite(v, "inactive", skin.sheets, path),
        action: oneOf(v.action, ACTIONS, `${path}.action`),
      };
    case "toggle": {
      if (!isObj(v.on)) fail(`${path}.on`, "must be the on-state sprites {sprite, hover?, active?, inactive?}");
      const on = v.on;
      const e: Element = {
        ...placed(v, path),
        type,
        sprite: sprite(v.sprite, skin.sheets, `${path}.sprite`),
        hover: optSprite(v, "hover", skin.sheets, path),
        active: optSprite(v, "active", skin.sheets, path),
        inactive: optSprite(v, "inactive", skin.sheets, path),
        on: {
          sprite: sprite(on.sprite, skin.sheets, `${path}.on.sprite`),
          hover: optSprite(on, "hover", skin.sheets, `${path}.on`),
          active: optSprite(on, "active", skin.sheets, `${path}.on`),
          inactive: optSprite(on, "inactive", skin.sheets, `${path}.on`),
        },
        action: v.action === undefined ? null : oneOf(v.action, ACTIONS, `${path}.action`),
        bind: bindOf(v, path, false),
        when: v.when === undefined ? null : str(v, "when", path),
      };
      if (!e.action && !(e.bind && e.when)) {
        fail(path, 'needs an "action" to be clickable, or "bind" and "when" to be an indicator');
      }
      return e;
    }
    case "text": {
      const font = str(v, "font", path);
      if (!(font in skin.fonts)) fail(`${path}.font`, `names no font (have ${Object.keys(skin.fonts).join(", ")})`);
      const e: Element = {
        ...placed(v, path),
        type,
        font,
        bind: bindOf(v, path, false),
        value: v.value === undefined ? null : str(v, "value", path),
        tint: v.tint === undefined ? "filament" : oneOf(v.tint, TOKENS, `${path}.tint`),
        opacity: opacityOf(v, path),
        glow: boolOf(v, "glow", path),
        overflow: v.overflow === undefined ? "clip" : oneOf(v.overflow, ["clip", "scroll"] as const, `${path}.overflow`),
        align: v.align === undefined ? "left" : oneOf(v.align, ["left", "center", "right"] as const, `${path}.align`),
      };
      // Declared, not resolved: a bind this app does not have is a soft
      // failure that renders empty, so the element still has something to
      // say and the skin still loads.
      if (v.bind === undefined && e.value === null) {
        fail(path, 'needs a "bind", a literal "value", or both');
      }
      if (v.inactive !== undefined) {
        if (!isObj(v.inactive)) fail(`${path}.inactive`, "must be {tint}");
        e.inactive = { tint: oneOf(v.inactive.tint, TOKENS, `${path}.inactive.tint`) };
      }
      if (v.lit !== undefined) {
        if (!isObj(v.lit)) fail(`${path}.lit`, "must be {bind, when, tint?, opacity?, glow?}");
        const l = v.lit;
        e.lit = {
          bind: bindOf(l, `${path}.lit`, true),
          when: str(l, "when", `${path}.lit`),
          tint: l.tint === undefined ? e.tint : oneOf(l.tint, TOKENS, `${path}.lit.tint`),
          opacity: opacityOf(l, `${path}.lit`),
          glow: boolOf(l, "glow", `${path}.lit`),
        };
      }
      return e;
    }
    case "slider": {
      const e: Element = {
        ...placed(v, path),
        type,
        track: optSprite(v, "track", skin.sheets, path),
        fill: optSprite(v, "fill", skin.sheets, path),
        thumb: optSprite(v, "thumb", skin.sheets, path),
        orientation: oneOf(v.orientation, ["horizontal", "vertical"] as const, `${path}.orientation`),
        bind: bindOf(v, path, true),
        origin: v.origin === undefined ? null : unit(v.origin, `${path}.origin`),
      };
      if (!e.track && !e.fill && !e.thumb) fail(path, 'needs at least one of "track", "fill" or "thumb"');
      if (v.lit !== undefined) {
        if (!isObj(v.lit)) fail(`${path}.lit`, "must be {bind, when, tint?, opacity?}");
        const l = v.lit;
        e.lit = {
          bind: bindOf(l, `${path}.lit`, true),
          when: str(l, "when", `${path}.lit`),
          tint: l.tint === undefined ? "filament" : oneOf(l.tint, TOKENS, `${path}.lit.tint`),
          opacity: opacityOf(l, `${path}.lit`),
        };
      }
      if (v.hot !== undefined) {
        if (!isObj(v.hot)) fail(`${path}.hot`, "must be {beyond, tint}");
        if (e.origin === null) fail(`${path}.hot`, 'needs an "origin" to measure "beyond" from');
        e.hot = { beyond: unit(v.hot.beyond, `${path}.hot.beyond`), tint: oneOf(v.hot.tint, TOKENS, `${path}.hot.tint`) };
      }
      return e;
    }
    case "visualizer":
      return { ...placed(v, path), type };
    case "list":
      // The playlist's rows. Parsed so a spec-valid skin is never refused;
      // drawn by the playlist's own PR.
      return { ...placed(v, path), type, rowHeight: int(v.rowHeight, `${path}.rowHeight`, 1) };
  }
}

function elementSet(
  v: unknown,
  size: [number, number],
  skin: Pick<Skin, "sheets" | "fonts">,
  path: string,
  warnings: string[],
): ElementSet {
  if (!isObj(v)) fail(path, "must be an object of elements");
  const elements: Element[] = [];
  for (const [name, e] of Object.entries(v)) {
    elements.push(element(name, e, skin, `${path}.${name}`, warnings));
  }
  const bar = elements.find((e) => e.name === "titlebar");
  if (!bar || bar.type !== "image" || bar.role !== "drag") {
    fail(path, 'needs a "titlebar" image with role "drag": it is the move handle');
  }
  return { size, elements };
}

function font(v: unknown, sheets: Skin["sheets"], path: string): Font {
  if (!isObj(v)) fail(path, "must be a font object");
  const type = oneOf(v.type, ["system", "bitmap"] as const, `${path}.type`);
  if (type === "system") {
    return {
      type,
      size: int(v.size, `${path}.size`, 1),
      case: v.case === undefined ? "none" : oneOf(v.case, ["upper", "none"] as const, `${path}.case`),
      tracking: v.tracking === undefined ? 0 : num(v.tracking, `${path}.tracking`),
    };
  }
  const sheet = str(v, "sheet", path);
  if (!(sheet in sheets)) fail(`${path}.sheet`, "names no sheet");
  return { type, sheet, glyphSize: pair(v.glyphSize, `${path}.glyphSize`, 1), map: str(v, "map", path) };
}

function sheetsOf(v: unknown, authored: Scale, path: string): Skin["sheets"] {
  if (!isObj(v) || Object.keys(v).length === 0) fail(path, "must name at least one sheet");
  const out: Skin["sheets"] = {};
  for (const [name, f] of Object.entries(v)) {
    if (typeof f === "string" && f !== "") {
      out[name] = { [authored]: f };
    } else if (isObj(f)) {
      const per: Partial<Record<Scale, string>> = {};
      for (const [k, file] of Object.entries(f)) {
        if (k !== "1" && k !== "2") fail(`${path}.${name}`, 'scales are "1" and "2"');
        if (typeof file !== "string" || file === "") fail(`${path}.${name}.${k}`, "must be a file name");
        per[k === "1" ? 1 : 2] = file;
      }
      if (!per[1] && !per[2]) fail(`${path}.${name}`, "lists no file");
      out[name] = per;
    } else {
      fail(`${path}.${name}`, 'must be a file name or {"1": file, "2": file}');
    }
  }
  return out;
}

function windowOf(
  name: WindowName,
  v: unknown,
  skin: Pick<Skin, "sheets" | "fonts">,
  warnings: string[],
): SkinWindow {
  const path = `windows.${name}`;
  if (!isObj(v)) fail(path, "is missing; all three classic windows are required");
  const size = pair(v.size, `${path}.size`, 1);
  if (size[0] !== CLASSIC_SIZE[0] || size[1] !== CLASSIC_SIZE[1]) {
    fail(`${path}.size`, `must be [${CLASSIC_SIZE}]; the classic windows are that size`);
  }
  if (typeof v.resizable !== "boolean") fail(`${path}.resizable`, "must be declared, true or false (D35)");
  const w: SkinWindow = {
    size,
    resizable: v.resizable,
    full: elementSet(v.elements, size, skin, `${path}.elements`, warnings),
    shade: (() => {
      if (!isObj(v.shade)) fail(`${path}.shade`, "is required: the strip is a layout of its own, not a clipped window");
      const s = pair(v.shade.size, `${path}.shade.size`, 1);
      if (s[0] !== SHADE_SIZE[0] || s[1] !== SHADE_SIZE[1]) fail(`${path}.shade.size`, `must be [${SHADE_SIZE}]`);
      return elementSet(v.shade.elements, s, skin, `${path}.shade.elements`, warnings);
    })(),
  };
  if (v.resizable) {
    const step = pair(v.resizeStep, `${path}.resizeStep`, 1);
    if (name === "playlist" && (step[0] !== PLAYLIST_STEP[0] || step[1] !== PLAYLIST_STEP[1])) {
      fail(`${path}.resizeStep`, `must be [${PLAYLIST_STEP}] (D30)`);
    }
    w.resizeStep = step;
    w.minSize = v.minSize === undefined ? size : pair(v.minSize, `${path}.minSize`, 1);
  } else if (name !== "playlist" && v.resizeStep !== undefined) {
    warnings.push(`${path}.resizeStep: ignored, the window is not resizable`);
  }
  return w;
}

/**
 * Parse a manifest. Throws `SkinError` on the first structural problem, so a
 * broken skin is refused whole; returns warnings for the soft cases the spec
 * lists (an unknown `bind`, a field that does nothing here). Unknown keys are
 * ignored, so an `hp-skin/2` degrades rather than dying.
 */
export function parseSkin(json: unknown): { skin: Skin; warnings: string[] } {
  if (!isObj(json)) throw new SkinError("manifest: must be a JSON object");
  const m = json;
  const warnings: string[] = [];
  if (m.format !== SKIN_FORMAT) fail("format", `must be "${SKIN_FORMAT}"`);
  const name = str(m, "name", "manifest");
  const author = typeof m.author === "string" ? m.author : "";
  const authoredScale = oneOf(String(m.authoredScale ?? 1), ["1", "2"] as const, "authoredScale") === "2" ? 2 : 1;
  const sheets = sheetsOf(m.sheets, authoredScale, "sheets");
  const art = m.art === undefined ? "final" : oneOf(m.art, ["final", "mask"] as const, "art");
  const glow = m.glow === undefined ? "baked" : oneOf(m.glow, ["baked", "renderer"] as const, "glow");

  if (!isObj(m.palette)) fail("palette", "must declare all six tokens (D71)");
  const palette = {} as Record<Token, string>;
  for (const t of TOKENS) palette[t] = hex(m.palette[t], `palette.${t}`);

  if (!Array.isArray(m.viscolor) || m.viscolor.length !== 24) fail("viscolor", "must be exactly 24 entries");
  const viscolor = m.viscolor.map((c, i) => hex(c, `viscolor[${i}]`));

  if (!isObj(m.visualizer)) fail("visualizer", "must name a component (D20)");
  let component = str(m.visualizer, "component", "visualizer");
  if (!(VISUALIZERS as readonly string[]).includes(component)) {
    warnings.push(`visualizer.component: "${component}" is not a component this app has; showing ${VISUALIZERS[0]}`);
    component = VISUALIZERS[0];
  }
  const visualizer = { component, options: isObj(m.visualizer.options) ? m.visualizer.options : {} };

  const fonts: Record<string, Font> = {};
  if (m.fonts !== undefined) {
    if (!isObj(m.fonts)) fail("fonts", "must be an object");
    for (const [k, f] of Object.entries(m.fonts)) fonts[k] = font(f, sheets, `fonts.${k}`);
  }

  if (!isObj(m.windows)) fail("windows", "must describe main, equalizer and playlist");
  const partial = { sheets, fonts };
  const windows = {} as Record<WindowName, SkinWindow>;
  for (const w of WINDOWS) windows[w] = windowOf(w, m.windows[w], partial, warnings);

  let seam: Skin["seam"] = {
    thickness: 1,
    hoverThickness: 2,
    color: "arc",
    discharge: { durationMs: 120, peakThickness: 4 },
  };
  if (m.seam !== undefined) {
    if (!isObj(m.seam)) fail("seam", "must be an object");
    const s = m.seam;
    const d = isObj(s.discharge) ? s.discharge : {};
    seam = {
      thickness: s.thickness === undefined ? 1 : int(s.thickness, "seam.thickness", 1),
      hoverThickness: s.hoverThickness === undefined ? 2 : int(s.hoverThickness, "seam.hoverThickness", 1),
      color: s.color === undefined ? "arc" : oneOf(s.color, TOKENS, "seam.color"),
      discharge: {
        durationMs: d.durationMs === undefined ? 120 : int(d.durationMs, "seam.discharge.durationMs", 0),
        peakThickness: d.peakThickness === undefined ? 4 : int(d.peakThickness, "seam.discharge.peakThickness", 1),
      },
    };
  }

  return {
    skin: { name, author, authoredScale, sheets, art, glow, palette, viscolor, visualizer, fonts, windows, seam },
    warnings,
  };
}

// ---- geometry ----

/** The file to decode for a sheet at a device pixel ratio: the 2x file when
 * the screen can show it and the skin has one, else whatever the skin has.
 * The returned scale multiplies rects into that file's pixels. */
export function sheetFor(skin: Skin, name: string, dpr: number): { file: string; scale: Scale } {
  const per = skin.sheets[name];
  if (!per) throw new SkinError(`sheet "${name}" is not in this skin`);
  if (dpr >= 2 && per[2]) return { file: per[2], scale: 2 };
  if (per[1]) return { file: per[1], scale: 1 };
  return { file: per[2]!, scale: 2 };
}

/** Every sprite reference in the skin, with the path that names it. */
export function sprites(skin: Skin): { path: string; ref: SpriteRef }[] {
  const out: { path: string; ref: SpriteRef }[] = [];
  const add = (path: string, ref: SpriteRef | undefined) => {
    if (ref) out.push({ path, ref });
  };
  for (const w of WINDOWS) {
    for (const [setName, set] of [
      ["elements", skin.windows[w].full],
      ["shade.elements", skin.windows[w].shade],
    ] as const) {
      for (const e of set.elements) {
        const p = `windows.${w}.${setName}.${e.name}`;
        switch (e.type) {
          case "nineslice":
            add(`${p}.sprite`, e.sprite);
            break;
          case "image":
            add(`${p}.sprite`, e.sprite);
            add(`${p}.inactive`, e.inactive);
            break;
          case "button":
            add(`${p}.sprite`, e.sprite);
            add(`${p}.hover`, e.hover);
            add(`${p}.active`, e.active);
            add(`${p}.inactive`, e.inactive);
            break;
          case "toggle":
            add(`${p}.sprite`, e.sprite);
            add(`${p}.hover`, e.hover);
            add(`${p}.active`, e.active);
            add(`${p}.inactive`, e.inactive);
            add(`${p}.on.sprite`, e.on.sprite);
            add(`${p}.on.hover`, e.on.hover);
            add(`${p}.on.active`, e.on.active);
            add(`${p}.on.inactive`, e.on.inactive);
            break;
          case "slider":
            add(`${p}.track`, e.track);
            add(`${p}.fill`, e.fill);
            add(`${p}.thumb`, e.thumb);
            break;
          default:
            break;
        }
      }
    }
  }
  return out;
}

/**
 * Every sprite rect must lie inside its sheet, at every scale the sheet is
 * supplied at. Out of bounds is a hard failure, not a clamp. `sizes` maps a
 * file name to its decoded pixel size.
 */
export function checkSheetBounds(skin: Skin, sizes: Record<string, { w: number; h: number }>): string[] {
  const problems: string[] = [];
  for (const [name, per] of Object.entries(skin.sheets)) {
    for (const scale of [1, 2] as const) {
      const file = per[scale];
      if (!file) continue;
      const size = sizes[file];
      if (!size) {
        problems.push(`sheet "${name}" at ${scale}x: ${file} was not decoded`);
        continue;
      }
      if (size.w > MAX_SHEET_SIDE || size.h > MAX_SHEET_SIDE || size.w * size.h * 4 > MAX_SHEET_BYTES) {
        problems.push(`sheet "${name}" at ${scale}x: ${file} is ${size.w}x${size.h}, over the cap`);
        continue;
      }
      for (const { path, ref } of sprites(skin)) {
        if (ref.sheet !== name) continue;
        const [x, y, w, h] = ref.rect;
        if ((x + w) * scale > size.w || (y + h) * scale > size.h) {
          problems.push(`${path}: [${ref.rect}] at ${scale}x leaves ${file} (${size.w}x${size.h})`);
        }
      }
    }
  }
  return problems;
}

/**
 * Where an element sits in a window that may be bigger than the skin's base
 * size (the playlist, D30). Left/top-anchored rects stay put; a `right` or
 * `bottom` anchor moves with that edge; `stretch` grows the rect along the
 * axes named. All in logical px; the webview's zoom does the rest (D76).
 */
export function placeRect(
  e: Placed,
  base: [number, number],
  current: [number, number],
): { x: number; y: number; w: number; h: number } {
  const dx = current[0] - base[0];
  const dy = current[1] - base[1];
  let [x, y, w, h] = e.rect;
  if (e.anchor === "right") x += dx;
  if (e.anchor === "bottom") y += dy;
  if (e.stretch === "x" || e.stretch === "xy") w += dx;
  if (e.stretch === "y" || e.stretch === "xy") h += dy;
  return { x, y, w, h };
}

/**
 * What a button's tooltip says. A skin names the action; the app says it in
 * words, and a toggle says what a press will do from where it stands now.
 * An indicator has no action and so no tooltip: it shows, it does not offer.
 */
export function actionTitle(action: Action | null, on: boolean): string {
  switch (action) {
    case "minimize":
      return "Minimise";
    case "shade":
      return on ? "Expand" : "Shade";
    case "zoom":
      return on ? "Normal size" : "Double size";
    case "close":
      return "Close";
    case "play":
      return "Play";
    case "pause":
      return "Pause";
    case "stop":
      return "Stop";
    case "prev":
      return "Previous";
    case "next":
      return "Next";
    case "eject":
      return "Open";
    case "eq":
      return "Equalizer";
    case "playlist":
      return "Playlist";
    case "shuffle":
      return on ? "Play in order" : "Play in a random order";
    case "repeat":
      return "Repeat";
    case "eqOn":
      return on ? "Turn the EQ off" : "Turn the EQ on";
    case "eqPresets":
      return "Presets";
    case null:
      return "";
  }
}

/** The element set a window shows in its current state. */
export function elementsOf(skin: Skin, window: WindowName, shaded: boolean): ElementSet {
  const w = skin.windows[window];
  return shaded ? w.shade : w.full;
}
