# hurricane-party — Native Skin Manifest (`hp-skin/1`)

**Locked as D36.** This is the format the default Eyewall skin ships in, and it is the target that the `.wsz` and `.wal` importers both map *into*. It is therefore load-bearing for v0.4 (skin renderer) and v0.5 (importers) simultaneously.

The governing constraint, stated once and applied everywhere below: **art and layout come in, code does not** (D8, D17, D23). There is no expression language here, no conditionals, no scripting hook. If a skin wants behavior the app doesn't already have, the answer is no.

---

## Why this format exists rather than "just CSS"

Because `.wsz` support later has to be a *loader*, not a rewrite (D6, and `architecture.md` is emphatic about it).

Classic Winamp skins are fixed sprite rectangles at conventional offsets on known sheets. If the native format is flexbox and CSS custom properties, then importing a `.wsz` means retrofitting fixed-pixel sprite positioning onto a layout engine that fights it. If the native format is *itself* sprite-rectangle-based, the importer is a coordinate table.

So this schema is deliberately closer to Winamp's model than to the web's — absolute rectangles in a fixed window space — with 9-slice added on top so that native and `.wal` skins get the resizability that classic skins never had.

**Consequence worth naming:** the design prototypes in `design/screens/` are built with flexbox and `gap`. They are visual reference (per the precedence table in `CLAUDE.md`), not the layout model. Deriving fixed rectangles from them is a real step in v0.4, not a copy-paste.

---

## Top-level shape

```jsonc
{
  "format": "hp-skin/1",
  "name": "Eyewall",
  "author": "paperhurts",
  "authoredScale": 1,          // 1 or 2: the scale a sheet named by a plain file is drawn at (O3, D92)

  "sheets": {                  // logical name -> file, relative to the skin root
    "chrome":  { "1": "chrome.png", "2": "chrome@2x.png" },   // one file per scale (D73, D92)
    "buttons": "buttons.png",                                    // or one file, at authoredScale
    "numbers": "numbers.png",
    "text":    "text.png",
    "pledit":  "pledit.png"
  },

  "art":  "mask",              // "final": full-colour pixels drawn as they are;
                               // "mask": alpha, tinted from the palette (D73)
  "glow": "renderer",          // "baked": the halo is in the pixels, none added;
                               // "renderer": painted from `arc`, user-toggleable (D73)

  "palette": {                 // the six tokens. imported from design/tokens.json,
    "void":     "#0C0A14",     // never hardcoded in a component
    "well":     "#05040A",
    "filament": "#E8F4FF",
    "arc":      "#6FE3FF",
    "strike":   "#FF4FD8",
    "ember":    "#FFB347"
  },

  "viscolor": [ /* exactly 24 hex entries, low energy -> peak */ ],

  "visualizer": { "component": "spectrum-bars", "options": { "bars": 19, "peakHold": true } },

  "fonts": {
    "chrome": { "type": "system", "size": 9, "case": "upper", "tracking": 0.08 },   // the theme's typeface (D92)
    "time":   { "type": "bitmap", "sheet": "numbers", "glyphSize": [9, 13], "map": "0123456789 -" }
  },

  "windows": { "main": { … }, "equalizer": { … }, "playlist": { … } },

  "seam": { … },

  "regions": { … }             // optional, best-effort, v0.5+
}
```

### Rects are logical, sheets come per scale (D92)

Every `rect` in a manifest is in logical pixels at 1x, whatever the sheet. A sheet named by a plain file name is drawn at `authoredScale`; one named by `{ "1": file, "2": file }` ships both, and the renderer decodes the one the screen can show (the `2` file when `devicePixelRatio` is 2 or more) and multiplies every rect by that scale when it cuts a sprite. Eyewall ships both (D73); a `.wsz` is a plain 1x file. Round once (D40): logical is the source of truth and the multiply happens at the cut, nowhere else.

### `tint` on a sprite, for `art: mask` (D92)

A sprite reference is `{ "sheet", "rect" }`, plus `"tint"`, a palette token name, when the skin is `art: mask`: the sprite's alpha is the shape and the token is its colour, so a theme reaches every piece of chrome live. Default `filament`. Ignored under `art: final`, and absent from every imported skin. A `text` element carries a `tint` the same way, and an `inactive: { "tint" }` for when the group loses focus.

### Fonts: bitmap or system (D92)

`fonts.<name>` is `{ "type": "bitmap", "sheet", "glyphSize", "map" }`, the classic glyph strip, or `{ "type": "system", "size", "case": "upper" | "none", "tracking": <em> }`, the theme's chrome typeface (`design/tokens.json`, `type.chrome`) at that size. Eyewall's title bar is a system font, the look v0.4b shipped; every `.wsz` font is a bitmap. A skin never names a family: that is the theme's.

### `viscolor` is one array, used twice

Exactly 24 entries, and **the same array drives the spectrum analyser and the Cone radar backdrop's dBZ colormap** (`theme.md`). That isn't a coordination effort — it's one definition consumed in two places, which is why bass energy and heavy precipitation come out the same magenta.

It is also what the control API publishes on `palette_changed` (`control-api.md`), so an LED wall recolors when the skin changes. Validate the length strictly: 24, not "about 24."

### `visualizer` is a component reference, not a widget

Required by **D20**. The manifest names a component the app implements; it never supplies one.

| `component` | Ships in | Notes |
|---|---|---|
| `spectrum-bars` | v0.4 | Eyewall. Bars on the `viscolor` ramp |
| `kaleidoscope` | v0.5 | Purricane. Honors the accessibility clamps in `purricane.md` — those are enforced by the app, not configurable by the skin |
| `oscilloscope` | — | Reserved |

An unknown `component` falls back to `spectrum-bars` with a warning. This is the one place a soft failure is right: a skin that names a visualizer from a future version should still load and look mostly correct.

---

## Windows

Only the three classic windows are skinnable (**O13**). Library, Video, Downloads, Prep, and Settings are normal decorated OS windows with modern chrome, and no `.wsz` contains art for them — don't invent sprite layouts Winamp never had.

```jsonc
"playlist": {
  "size": [275, 116],
  "resizable": true,
  "resizeStep": [25, 29],      // D30. verified against Webamp
  "minSize": [275, 116],
  "shade": { "size": [275, 14], "elements": { … } },
  "elements": { … }
}
```

| Field | Meaning |
|---|---|
| `size` | Base size in logical px at 1x (D92). Main and EQ are `[275,116]` and immutable |
| `resizable` | **The capability flag D35 checks at hover time.** Determines whether a shared edge offers a splitter cursor or a move cursor |
| `resizeStep` | Quantization for the splitter. Playlist is `[25,29]`; omit when not resizable |
| `shade` | The windowshade layout — a separate element set at `[275,14]`, not a clipped version of the full one |

**A window supplies values, never geometry.** The skin says where the clock is, what font it is in and how it is lit; the window hands the shell a `binds` record (what the clock reads) and a `slots` record of content to place inside a named element's box — the analyser goes in whatever rectangle the manifest gave the visualizer named `vis`. Nothing outside the manifest knows a pixel.

**The boxes a window fills are found by name, so those names are part of the format** (D99), like `titlebar`, and a full element set without one is refused rather than drawn with nothing in it:

| Window | Name | Type | What the window puts there |
|---|---|---|---|
| `main` | `vis` | `visualizer` | The analyser |
| `main` | `trackTitle` | `text` | The title, and a track it cannot open |
| `equalizer` | `eqCurveWell` | `image` or `slot` | The response curve and the preset menu |
| `playlist` | `list` | `list` | The rows |
| `playlist` | `listStatus` | `slot` | The count and running time |
| `playlist` | `urlField` | `slot` | The link field |

**Every element set, full and shade, has a `titlebar` image with `"role": "drag"`.** It is the one move handle (D35) and carries the double-click that toggles shade (D60); a set without one is refused.

**`shade` is not optional for these three.** The Main shade doubles as the always-on-top mini-player and is, per the design brief, the single most important screen in the project. A skin that omits it fails validation rather than rendering a clipped main window.

### Focus is a group property

Every element that varies with focus declares an `inactive` variant. The renderer resolves focus **per bond group, not per window** — when any window in a connected component has OS focus, every member renders active. Getting this wrong looks broken immediately, and it's a renderer rule rather than a manifest one; the manifest's only job is supplying both sets of art.

---

## Elements

Every element is an absolute rectangle in window space. Origin is the window's top-left, units are logical px at 1x (D92). In a resizable window (the playlist, D30) an element may add `"anchor": "right" | "bottom" | "bottom-right"` to keep its distance from that edge, or from that corner (D103), instead of from the origin, and `"stretch": "x" | "y" | "xy"` to grow with the window along those axes: the playlist's title bar stretches along x, its shade button anchors right, its rows stretch along both, and its bottom bar anchors to the bottom (D99).

```jsonc
"elements": {
  "frame":  { "type": "nineslice", "rect": "fill",
              "sprite": { "sheet": "chrome", "rect": [0, 0, 64, 64] },
              "insets": [3, 3, 3, 3] },

  "titlebar": { "type": "image", "rect": [0, 0, 275, 14],
                "sprite":   { "sheet": "chrome", "rect": [27, 0, 275, 14] },
                "inactive": { "sheet": "chrome", "rect": [27, 15, 275, 14] },
                "role": "drag" },

  "play":   { "type": "button", "rect": [39, 88, 23, 18],
              "sprite": { "sheet": "buttons", "rect": [23, 0, 23, 18] },
              "hover":  { "sheet": "buttons", "rect": [23, 18, 23, 18] },
              "active": { "sheet": "buttons", "rect": [23, 36, 23, 18] },
              "action": "play" },

  "time":   { "type": "text", "rect": [48, 26, 63, 13], "font": "time", "bind": "elapsed" },

  "title":  { "type": "text", "rect": [111, 27, 154, 6], "font": "chrome",
              "bind": "trackTitle", "overflow": "scroll" },

  "seekbar": { "type": "slider", "rect": [16, 72, 248, 10],
               "track": { "sheet": "chrome", "rect": [0, 68, 248, 10] },
               "thumb": { "sheet": "chrome", "rect": [248, 68, 29, 10] },
               "orientation": "horizontal", "bind": "position" },

  "vis":    { "type": "visualizer", "rect": [24, 43, 76, 16] }
}
```

### Types

| `type` | Purpose | Required |
|---|---|---|
| `image` | Static sprite | `rect`, `sprite`; `inactive`, `role: "drag"` and `opacity` optional |
| `nineslice` | Stretchable frame. `rect: "fill"` tracks the window; a rect of its own edges a control | `sprite`, `insets`; `opacity` optional |
| `button` | Clickable. `action` names an app command | `rect`, `sprite`, `action`; `hover`, `active` (pressed), `inactive`, `label`, `disabled` optional (D99) |
| `toggle` | Two-state button, or an indicator | `rect`, `sprite`, `on` (`{ sprite, hover?, active?, inactive? }`), and either `action` (clickable) or `bind` + `when` (state-driven); `label`, `disabled` optional (D99) |
| `slider` | Continuous control | `rect`, `bind`, and at least one of `track`, `fill`, `thumb`; `origin`, `lit`, `hot` optional (D98) |
| `text` | Bitmap or system text | `rect`, `font`, and either `bind` or a literal `value`; `opacity`, `glow`, `lit`, `overflow`, `align` optional |
| `list` | Playlist rows. The window draws them in this box (D99) | `rect`, `rowHeight`, `font`; `tint`, `opacity`, `current`, `selected` optional |
| `slot` | A box with no art, for something the window draws (D99) | `rect` |
| `visualizer` | Where the component from `visualizer` draws | `rect` |

A `slider`'s three pieces are each optional and at least one is required: `track` under the whole length, `fill` from the start to the value, `thumb` at it. All three is a seek bar; a `fill` alone is a level meter.

**A centred slider (D98).** `origin`, 0..1, makes a slider a centred control: the fill runs from the origin to the value rather than from the start, the wheel nudges it by 1/48 of its range, and a double press returns it to the origin. The EQ's gains sit at `0.5`, which is 0 dB. `lit` is the same shape as on a text, `{ bind, when, tint?, opacity? }`, and gives the fill and thumb a second look while the binding holds; every EQ slider dims while the EQ is off. `hot`, `{ beyond, tint }`, tints the thumb once the value is more than `beyond` from the origin, and needs an origin to measure from. The dim wins over hot. A text's `align` is `left` (the default), `center` or `right`.

`action` and `bind` are drawn from **fixed vocabularies the app defines** — the same discipline as the companion pack's seven behavior states (D23). A skin selects from the list; it cannot extend it. An unknown `action` fails validation; an unknown `bind` renders empty and warns. The lists are `ACTIONS` and `BINDS` in `src/lib/skin.ts`. Today: actions `minimize`, `shade`, `zoom`, `close`, `play`, `pause`, `stop`, `prev`, `next`, `eject`, `eq`, `playlist`, `shuffle`, `repeat`; binds `windowTitle`, `trackTitle`, `elapsed`, `remaining`, `kbps`, `khz`, `position`, `volume`, `volumePercent`, `balance`, `playState` (`"playing"`, `"paused"`, `"stopped"`), and the equalizer's `eqOn` (`"on"`, `"off"`), `eqPreset`, `eqMenu` (`"open"`, `"closed"`), `eqTrim`, `eqClip`, `eqPre` and `eqBand1`–`eqBand10` (0..1, 0.5 being 0 dB). The EQ adds two actions, `eqOn` and `eqPresets`. The playlist (D99) adds the actions `add`, `addUrl`, `remove`, `library`, and the binds `shuffle` and `repeatOn` (`"on"`, `"off"`), `repeatLabel` (`"REP"`, `"1x"`, `"ALL"`) and `plCanRemove` (`"yes"`, `"no"`).

### `opacity`, so one sprite serves every strength (D93)

Mask art carries strength in its own alpha, which would mean a near-identical sprite for every place a skin wants the same shape a little dimmer. An element may instead declare `opacity`, 0..1, over its tint: Eyewall draws one full-alpha `ring` and uses it as the window's frame at `0.3` and as the edge of the title strip, the seek bar and the volume bar at `0.14`, and one full-alpha `solid` as every well. Available on `image`, `nineslice`, `text` and `list`, and on a `label`. An imported skin never sets it — its alpha is already in its pixels.

### A `text` says what it shows (D93)

`bind` names a value the app supplies; `value` is a literal, and a `{}` in it is replaced by the bound value, so `"value": "VOL {}"` with `"bind": "volumePercent"` is one element rather than two. One of the two is required. `glow: true` asks for the theme's static glow, for text a `.wsz` would have baked into glyph art — Eyewall's clock. `lit` is a second appearance chosen by state, `{ bind, when, tint?, opacity?, glow? }`: the PLAY tag lights when `playState` is `"playing"`, and STOP lights `strike` where the others light `arc`.

### A `toggle` that watches instead of clicking (D93)

`action` makes a toggle clickable and the app decides its on state. `bind` and `when` make it state-driven: the on art shows while that binding holds that value. A transport button has both — it plays when clicked and lights while playing. A toggle with a binding and no action is an indicator, which is why the format has no separate type for one. At least one of the two is required.

### A button's words, and when it cannot be pressed (D99)

`label` puts words in a `button` or `toggle`'s box: `{ "font", "value"?, "bind"?, "tint"?, "opacity"?, "hover"?, "on"? }`, with `value` and `bind` working as they do on a `text` (one is required). The words are centred and drawn with the box, so the halo takes them too; `hover` is their colour while the pointer is over the button, `on` their colour while a toggle is on, and both are a token at full strength. Words do not change with focus; the art does. Eyewall's playlist bar is six buttons on one 21 × 13 sprite, each with its own label. A separate `text` over a button still works, and stays right for words that should not answer the pointer (the EQ's preset name).

`disabled`, `{ "bind", "when" }`, makes a button or toggle unpressable while the binding holds that value: drawn at 0.35, with no hover art, no halo and no click. The playlist's REM waits on `plCanRemove`.

### A list, and a slot (D99)

A `list` is the box the playlist's rows are drawn in, and the skin says how they look: `rowHeight` in logical pixels, `font` (a name from `fonts`), `tint` and `opacity` for a row, `current` for the playing row and `selected` for the selected one's highlight. The rows themselves are the window's — pressing, double-pressing, dragging to reorder and the keys are behaviour, not art — and it reads the skin's choices from `--list-fg`, `--list-hi`, `--list-now`, `--list-sel` and `--list-row` on the box. The well and edge around the rows are ordinary `image` and `nineslice` elements, and the edge goes after the list so a selected row never covers it.

A `slot` is a placed box with nothing drawn in it: where the window puts something the skin has no element for. Eyewall's playlist has two, the count and running time, and the link field that lies over the whole bar while it is open. A slot lets the pointer through; what the window puts in it takes the pointer back where it needs it.

### Glow is declared, not assumed

Two top-level fields say what kind of art this is (D73). **`glow`** is `"baked"` — the halo is in the pixels and the renderer adds none — or `"renderer"`, where the renderer paints a halo from the palette's `arc` behind glow-eligible chrome and the user's glow toggle applies: a `glow` checkbox in the library's header until there is a settings window, saved in `settings` and heard by all three classic windows at once (D100). Off, the renderer adds no halo anywhere in a classic window, a text's `glow` included, and the windows drop the halos they draw themselves; a `baked` skin is untouched either way, since its halo is in its pixels. **`art`** is `"final"` — full-colour pixels, drawn as they are — or `"mask"`, alpha masks the renderer tints from the palette, so a theme change reaches the whole chrome rather than only its halo. Both importers produce `art: final, glow: baked`, which is what their source art is, so an imported skin never double-glows and native and imported skins take one rendering path.

What no manifest can change: **no CSS filter on the visualizer surface or any ancestor of it.** That is the 60 Hz path, a filter on a parent runs the child through it every frame, and the analyser's own glow is pre-rendered into its ramp art for that reason. The renderer scopes the toggle per chrome element and does not consult the skin about the exemption — the same status as `prefers-reduced-motion` on the seam below. The modern decorated windows compute glow in CSS and aren't described by this manifest at all.

---

## The bond seam

The signature element, and the one thing here Winamp has no equivalent for — so it has no `.wsz` mapping and always falls back to a palette-drawn default.

```jsonc
"seam": {
  "thickness": 1,
  "hoverThickness": 2,
  "color": "arc",              // palette token, never a hex literal
  "discharge": { "durationMs": 120, "peakThickness": 4 }
}
```

Four states, per the design brief: idle (hairline), hover (brighter, +1px), dragging (tracks the cursor), discharging (fast bloom then out, ~120 ms).

**`prefers-reduced-motion` makes the discharge an instant state change** — the seam still disappears, it just doesn't bloom. That's enforced by the renderer and is not skin-overridable, for the same reason the kaleidoscope's flash clamps aren't.

Animate `opacity` on a pre-composited glow layer. Never animate `box-shadow`.

---

## Importer mappings

Both importers are mappings *into* the above. That is the entire justification for locking this schema before either one is written.

### `.wsz` — full support, v0.5

| Classic file | Maps to |
|---|---|
| `MAIN.BMP` | `windows.main.elements.frame` + backdrop |
| `CBUTTONS.BMP` | The five transport `button` elements, at conventional offsets |
| `TITLEBAR.BMP` | `titlebar` sprite + `inactive` variant, plus the shade-mode strips |
| `NUMBERS.BMP` / `NUMS_EX.BMP` | `fonts.time` |
| `TEXT.BMP` | `fonts.chrome` |
| `VOLUME.BMP` / `BALANCE.BMP` | The volume and balance `slider` elements |
| `POSBAR.BMP` | `seekbar` track and thumb |
| `PLEDIT.BMP` + `PLEDIT.TXT` | `windows.playlist` frame, its bar's buttons, and the row colours |
| `EQMAIN.BMP` | `windows.equalizer` |
| `VISCOLOR.TXT` | `viscolor` (24 entries — the format's own count) |
| `REGION.TXT` | `regions`, best-effort |

Sprite coordinates in `.wsz` are **conventional, not declared** — the offsets live in the importer as a constant table, which is exactly why the format is bounded and a weekend-to-a-fortnight problem rather than an open-ended one. That table is `src/lib/wsz.ts`, transcribed from Webamp with its notice in `THIRD-PARTY.md` (D102), beside the classic window positions. `wszManifest()` turns a zip's file list and its two text files into a manifest this document's own validator then accepts or refuses, so an import that would half-load is refused before anything is written.

**Colour comes from the theme, except where it cannot** (D101). An imported skin is `art: final`, and pixels cannot be tinted, so its `PLEDIT.TXT` becomes the palette the three classic windows use and its `VISCOLOR.TXT` becomes the analyser's ramp. A native `art: mask` skin declares a palette that is validated and ignored: the theme paints it.

Classic skins set `resizable: false` on all three windows (except playlist), so on a `.wsz` most edges offer the move cursor and the interaction degrades gracefully. Same engine, fewer capabilities.

### `.wal` — partial, explicitly, v0.5+

Parse the XML layout, map what corresponds to native concepts, render the PNGs, **ignore the `.maki` bytecode entirely** (D17, and `windows.md` argues it at length).

Modern skins declare `resizable: true`, so on a `.wal` most edges become live splitters and the Excel-splitter model comes fully alive. That's the clean story: same engine, richer skin capabilities.

Document the limitation honestly: *many modern skins load; heavily scripted ones will look right but sit still.*

---

## Validation

Packs are untrusted input from the internet even without code in them. Same rules as `companion.json` (`purricane.md`):

- **Validate against the schema and refuse to load rather than half-load.** A partially-valid skin is a support burden and an unreproducible bug report
- **Cap sheet dimensions and total decoded size.** A 16k × 16k PNG is a denial of service dressed as a skin
- **Every `sprite.rect` must lie inside its sheet.** Out-of-bounds is a hard failure, not a clamp
- **Unknown keys are ignored, not errors**, so `hp-skin/2` degrades rather than dying
- **Missing required elements are a hard failure.** Missing *optional* ones fall back to the default skin's art for that element, so a skin that forgets the balance slider still loads

The asymmetry is deliberate: structural errors fail loudly at load time, missing art falls back quietly at render time. The first is a broken file; the second is an incomplete one, and incomplete skins are the norm.

---

## Open

- ~~**The conventional `.wsz` sprite offset table** hasn't been transcribed yet~~ **Done** (D102): `src/lib/wsz.ts`. Mapped so far: Main in full, the equalizer's switch, preset button, curve box and eleven sliders, and the playlist's frame, rows and the boxes the window fills. Still to come, one PR each: the playlist's bottom bar and its `bottom-right` anchor, **bitmap fonts drawn as glyphs** (a `.wsz`'s clock and title are art, and until then they are the theme's face at the glyph height), and the zip, the import button and the refusal in front of the person who chose the file (#107)
- ~~**Fixed rectangles for the Eyewall default skin** still have to be derived from the Pass 1 prototypes, which are flexbox. That's a v0.4 task and the first real test of whether this schema is expressive enough~~ **Done for the shell chrome in #3's first PR** (D90, D92): `skins/eyewall/` is derived from the CSS chrome that shipped, and it is the template a person copies. The interiors follow, one window per PR
