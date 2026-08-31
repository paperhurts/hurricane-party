# hurricane-party — Purricane

*(Working name. Alternates: Kaleidokitten, Category Cute. "Landfall Kitten" was mine and it's the worst of the four.)*

I gave this three sentences in the theme spec and filed it under Pass 3 as "variations on locked foundations." That was wrong on two counts: it isn't a variation, and I ignored the word **kaleidoscope** entirely, which turns out to be the load-bearing part.

---

## What I missed

**Kaleidoscope isn't a color palette. It's a visualizer directive.**

In Eyewall, the analyser is spectrum bars on a radar ramp. In Purricane, **the analyser is a kaleidoscope** — radial mirrored segments driven by the same FFT data, rotating slowly, blooming on beat.

That's the whole theme. The kittens are what happens *around* the kaleidoscope. I described the accessory and skipped the centerpiece.

Which also means this theme proves something structurally important: **the visualizer is a themeable component, not a fixed widget.** Eyewall draws bars, Purricane draws a mandala, and a future theme could draw an oscilloscope or a Vectrex wireframe. That's a real capability the native skin manifest has to support, and it only surfaced because you pushed back.

---

## Resolving the light-mode tension

Eyewall's thesis is that the screen is the only light source in the room. Purricane can't violate that or the whole design philosophy is decorative.

**Resolution: Purricane is still emissive, just high-key.** Think a lit aquarium, or a Lite-Brite in a dark room — not a white webpage. The field is bright but *saturated*, and it's still glowing rather than reflecting.

That keeps the thesis intact. It's not light mode, it's a different color temperature of the same emission. The joke stands: the brightest thing we ship is a theme, not a mode.

## Palette

| Token | Hex | Role |
|---|---|---|
| `--sugar` | `#FFF4FB` | Field. Near-white with a pink cast — bright, not neutral |
| `--floss` | `#FF9FD6` | Hot pink. Primary accent |
| `--mint` | `#7FFFD4` | Aquamarine. Secondary |
| `--butter` | `#FFE97F` | Pale yellow. Tertiary, warmth |
| `--periwinkle` | `#A99FFF` | Violet. The callback to paperhurts' base — same hue family as `--void`, opposite end of the value scale |
| `--ink` | `#4A2A5C` | Deep violet. Text |

This is the one theme where text is dark on light rather than bright on dark. That's fine; don't fight it. Everything *else* still glows.

Typeface is **Comic Sans**, per the original brief, and it's the correct call. "So ugly it's cute" is precisely this theme's thesis. Use it at a slightly larger size than Iosevka would sit at — Comic Sans has a small x-height relative to its cap height and looks cramped at 11px.

---

## The kaleidoscope

Radial symmetry driven by live FFT data.

- **6 or 8 mirrored segments**, user-selectable. Even counts read as more symmetric and more kawaii; odd counts read as psychedelic
- **Rotation rate is a slow constant**, not audio-driven. Audio drives the *pattern*; a rotation that speeds up with volume becomes nauseating fast
- **Low frequencies drive radial extent** — bass pushes the mandala outward from center
- **High frequencies drive detail density** near the rim
- **Beat flag triggers a bloom** — a fast scale-and-fade pulse, roughly 150ms, not a color change
- **Hue drifts slowly** across the palette, cycling on the order of a minute or two

### Accessibility — this one is real, not boilerplate

Rotating high-contrast radial patterns are a genuine photosensitivity and migraine risk in a way that spectrum bars simply are not. Kaleidoscope visualizers are a known trigger category.

Non-negotiable:

- **Cap rotation speed** and never let audio increase it
- **Cap flash frequency below 3 Hz.** The beat bloom must not fire faster than that regardless of BPM — clamp it, and let fast tracks skip blooms rather than stacking them
- **`prefers-reduced-motion` produces a static mandala** that responds to audio in amplitude only. No rotation, no bloom
- **Ship a "calm" toggle in the theme itself**, discoverable without digging through settings

That isn't hedging. A visualizer that hurts someone is a bug, and this specific kind of visualizer is the one most likely to.

---

## The kittens

Small animated cats living on the desktop, outside the app window. Interactive, and they dance to the beat.

### They should treat the windows as furniture

This is the thing that makes them yours rather than a generic desktop pet.

The bonded window group is **terrain.** Kittens sit on top edges, walk along the glowing seam between bonded windows, curl up asleep on the playlist, bat at the seek bar as it moves.

And the payoff: **when you double-click a seam to break a bond, any kitten sitting on it has to jump off.** The signature interaction and the signature creature, connected. Nobody else's desktop pet reacts to your window manager because nobody else's window manager has a seam worth sitting on.

### Behavior states

| State | Trigger |
|---|---|
| Idle | Default. Sit, blink, tail flick, occasional grooming |
| Sleep | No audio playing for a while. Curls up, ideally on a window |
| Dance | Playing. Beat flag from the viz stream drives the bounce |
| Walk | Periodic wander between perches |
| Startle | Bond breaks under them, or the window they're on moves |
| Pet | Clicked. Purr animation, hearts, brief affection |
| Carry | Dragged. Dangles, complains, resettles when dropped |

### Palette coupling

**The kittens share the kaleidoscope's drifting hue.** On a bass drop, the mandala and the cats shift color together. That's the detail that makes it feel like one designed thing rather than a mascot bolted onto a visualizer.

### Population

Default two. Range one to five. Five is already a lot of moving things on a desktop; don't offer more just because the code allows it.

---

## Architecture — unchanged, and now more clearly justified

**The kittens are an external process consuming the public viz API.** The kaleidoscope is *in* the player as a themed visualizer component; the kittens are *outside* it, talking over the same pipe an LED wall would use.

Four reasons, which hold up better now that the pets have a real behavior spec:

1. **Dogfooding.** If your kittens dance correctly through the public protocol, the protocol is proven for everyone else
2. **They're the reference client.** A charming working example beats a 40-line demo
3. **Containment.** Sprite animation, pathfinding, per-pixel hit testing, and multi-monitor wandering are a real subsystem with nothing to do with playing audio
4. **Isolation.** A kitten crash doesn't stop the music. During a hurricane that's the correct priority

The one thing this costs: the kittens need window *geometry* to treat windows as furniture, which the viz stream doesn't carry. So the control channel gains one event:

```jsonc
{"event":"layout_changed", "windows":[
  {"id":"main",     "x":420, "y":300, "w":550, "h":232},
  {"id":"playlist", "x":420, "y":532, "w":550, "h":232}
], "bonds":[
  {"a":"main", "b":"playlist", "edge":"bottom", "span":[420, 970]}
]}
```

Useful beyond kittens — any external overlay, LED positioning, or second-screen tool wants it. Add it to the public protocol at v1.0.

### Technical notes

- Transparent, always-on-top, undecorated windows — one per kitten
- Windows: `WS_EX_LAYERED` for per-pixel alpha, toggling `WS_EX_TRANSPARENT` so a kitten is click-through while idle and clickable when you want to pet it. **This toggle is the fiddly part**; budget for it
- Screen bounds change when displays connect and disconnect. Kittens must not strand offscreen
- Beat detection is free from the viz stream's `flags` bit

---

## One thing the theme should not do

**Don't let it rewrite error copy.**

Kawaii empty states, kawaii idle text, kawaii tooltips — yes, all of it. But when a download fails at 2am with a storm outside, the message says what broke and what to do. It does not say `oopsie! (=^･ω･^=)`.

Themes change how the app looks and feels. They don't change how clearly it tells you the truth.

---

## Companions are skinnable

Yes — and this is worth building properly, because it's the difference between "a cute easter egg" and "a thing people make stuff for."

Define a companion as **a sprite sheet plus a declarative manifest.** Kittens become one shipped pack. Unicorns, space marines, tardigrades, Roombas, tiny hurricanes — anyone can make those and nobody has to touch the code.

### The line that keeps this safe

Same line as `.wal`, and for the same reason: **art comes in, code does not.**

- The **behavior vocabulary is fixed by the app.** Idle, sleep, dance, walk, startle, pet, carry. Seven states, defined once
- A pack supplies **art for those states**, plus a few numbers
- A pack **cannot define new behaviors**

That last constraint is doing all the work. Let packs invent behaviors and they need triggers; triggers need conditions; conditions need logic — and you've built a scripting language and reinvented MAKI in the pet system. Fixed vocabulary, open art. That's the whole discipline.

### `companion.json`

```jsonc
{
  "format": "hp-companion/1",
  "name": "Space Marine",
  "author": "…",
  "sprite": "marine.png",
  "frameSize": [32, 32],
  "anchor": [16, 31],        // feet — where it contacts a window edge
  "palette": "fixed",        // "theme" = tint with the theme's hue drift
                             // "fixed" = the pack's own colors

  "states": {
    "idle":    {"frames": [0,1,2,1],    "fps": 4,  "loop": true},
    "sleep":   {"frames": [8,9],        "fps": 1,  "loop": true},
    "dance":   {"frames": [16,17,18,19],"syncTo": "beat"},
    "walk":    {"frames": [24,25,26,27],"fps": 8,  "loop": true},
    "startle": {"frames": [32,33,34],   "fps": 12, "loop": false, "then": "idle"},
    "pet":     {"frames": [40,41,42],   "fps": 6,  "loop": false, "then": "idle"},
    "carry":   {"frames": [48,49],      "fps": 3,  "loop": true}
  },

  "walkPxPerSec": 24,
  "defaultCount": 2
}
```

Three fields carry most of the design:

- **`anchor`** is the contact point, at the feet. It's what lets a 32px kitten and a 64px space marine both perch correctly on a bonded window seam without the app knowing anything about either
- **`"syncTo": "beat"`** makes `dance` advance on beat flags from the viz stream rather than on a frame rate. That's the difference between dancing and merely animating, and it's one field
- **`palette`** decides whether the companion drifts hue with the kaleidoscope or keeps its own colors. Kittens want `theme`. Unicorns emphatically want `fixed`

### Validation and failure

Packs are untrusted input from the internet, even without code in them. So:

- Validate the manifest against a schema and **refuse to load rather than half-load.** A partially-valid pack is a support burden
- Cap sprite sheet dimensions and frame counts — a 16k×16k PNG is a denial of service dressed as a unicorn
- Missing optional states fall back to `idle`. Missing `idle` is a hard failure
- Unknown keys are ignored, not errors, so `hp-companion/2` degrades gracefully

### Two tiers of "make it yours"

Worth stating plainly, because it's the whole extensibility story:

| | For | Runs where |
|---|---|---|
| **Companion packs** | The easy 90%. New art, existing behaviors | Data only. No execution anywhere |
| **The public viz API** | Everything else. Your own overlay, LED wall, second-screen toy, a whole different pet app | The author's own process |

**Nothing executes inside hurricane-party in either case.** That's how you get real extensibility without ever building a plugin loader — and it's why the control API being public matters more than it first looked.

---

## Scope

**v0.7**, after the protocol freezes at v1.0. Not because it's low value — it's the most distinctive thing in the project — but because it's the *reward* for a stable API. Build it first and it drags the protocol into a kitten-shaped form that serves nobody else.

Split it:

| | Ships in | Where it lives |
|---|---|---|
| Kaleidoscope visualizer + Purricane palette | **v0.5**, with the skin system | In the player |
| Kittens | **v0.7**, after protocol freeze | External process |

The theme is usable and complete without the cats. The cats are the encore.
