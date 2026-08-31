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
  "authoredScale": 1,          // 1 or 2. the scale the art is drawn at (O3)

  "sheets": {                  // logical name -> file, relative to the skin root
    "chrome":  "chrome.png",
    "buttons": "buttons.png",
    "numbers": "numbers.png",
    "text":    "text.png",
    "pledit":  "pledit.png"
  },

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
    "chrome": { "type": "bitmap", "sheet": "text", "glyphSize": [5, 6], "map": " ABCDEFG..." },
    "time":   { "type": "bitmap", "sheet": "numbers", "glyphSize": [9, 13], "map": "0123456789 -" }
  },

  "windows": { "main": { … }, "equalizer": { … }, "playlist": { … } },

  "seam": { … },

  "regions": { … }             // optional, best-effort, v0.5+
}
```

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
| `size` | Base size in logical px at `authoredScale`. Main and EQ are `[275,116]` and immutable |
| `resizable` | **The capability flag D35 checks at hover time.** Determines whether a shared edge offers a splitter cursor or a move cursor |
| `resizeStep` | Quantization for the splitter. Playlist is `[25,29]`; omit when not resizable |
| `shade` | The windowshade layout — a separate element set at `[275,14]`, not a clipped version of the full one |

**`shade` is not optional for these three.** The Main shade doubles as the always-on-top mini-player and is, per the design brief, the single most important screen in the project. A skin that omits it fails validation rather than rendering a clipped main window.

### Focus is a group property

Every element that varies with focus declares an `inactive` variant. The renderer resolves focus **per bond group, not per window** — when any window in a connected component has OS focus, every member renders active. Getting this wrong looks broken immediately, and it's a renderer rule rather than a manifest one; the manifest's only job is supplying both sets of art.

---

## Elements

Every element is an absolute rectangle in window space. Origin is the window's top-left, units are logical px at `authoredScale`.

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
| `image` | Static sprite | `rect`, `sprite` |
| `nineslice` | Stretchable frame. `rect: "fill"` tracks the window | `sprite`, `insets` |
| `button` | Clickable. `action` names an app command | `rect`, `sprite`, `action` |
| `toggle` | Two-state button | `rect`, `sprite`, `on`, `action` |
| `slider` | Continuous control | `rect`, `track`, `thumb`, `bind` |
| `text` | Bitmap or system text | `rect`, `font`, `bind` |
| `list` | Playlist rows | `rect`, `rowHeight`, row color bindings |
| `visualizer` | Where the component from `visualizer` draws | `rect` |

`action` and `bind` are drawn from **fixed vocabularies the app defines** — the same discipline as the companion pack's seven behavior states (D23). A skin selects from the list; it cannot extend it. An unknown `action` fails validation; an unknown `bind` renders empty and warns.

### Glow is baked, not computed

Per `theme.md`: for the three classic windows, the glow is **pre-rendered into the sprite art**. No CSS filters anywhere near the 60 Hz visualizer, and it means native and imported skins take an identical rendering path. The modern decorated windows compute glow in CSS, but they aren't described by this manifest at all.

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
| `PLEDIT.BMP` + `PLEDIT.TXT` | `windows.playlist` frame and row color bindings |
| `EQMAIN.BMP` | `windows.equalizer` |
| `VISCOLOR.TXT` | `viscolor` (24 entries — the format's own count) |
| `REGION.TXT` | `regions`, best-effort |

Sprite coordinates in `.wsz` are **conventional, not declared** — the offsets live in the importer as a constant table, which is exactly why the format is bounded and a weekend-to-a-fortnight problem rather than an open-ended one.

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

- **The conventional `.wsz` sprite offset table** hasn't been transcribed yet. It's mechanical, it's public in Webamp's source, and it's a v0.5 task — but it's the thing that makes the mapping table above real rather than aspirational
- **Fixed rectangles for the Eyewall default skin** still have to be derived from the Pass 1 prototypes, which are flexbox. That's a v0.4 task and the first real test of whether this schema is expressive enough
