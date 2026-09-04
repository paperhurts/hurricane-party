# hurricane-party — Theme Spec

**Default theme: "Eyewall"**

---

## The thesis

The eyewall is the bright, violent ring around the calm center of a storm. Glowing outline, dark fill. That's not a metaphor I retrofitted onto your description — it's literally what "bright glowing outlines, deep dark window fill" describes.

And it gives dark-only a *reason* rather than a preference: **during a hurricane this screen is the only light source in the room.** Everything glows because everything is emitting. Nothing is lit from outside. A light theme would be a lie about the conditions the app was built for.

Which makes your light-mode policy coherent instead of stubborn: **no light mode, ever. The brightest thing we ship is a kitten.**

---

## Palette

Anchored on the paperhurts.dev base (`#0c0a14` — near-black with a violet cast, pulled from the live site's theme color).

| Token | Hex | Role |
|---|---|---|
| `--void` | `#0C0A14` | Window fill. The paperhurts base |
| `--well` | `#05040A` | Insets — list backgrounds, seek trough, EQ bed |
| `--filament` | `#E8F4FF` | Bright text. Near-white, cold cast so it reads as emission not paint |
| `--arc` | `#6FE3FF` | **Primary glow.** Outlines, borders, active edges |
| `--strike` | `#FF4FD8` | Magenta. Now-playing, current row, peak indicator |
| `--ember` | `#FFB347` | Amber. Storage pressure, failed jobs, warnings |

Amber rather than red for warnings — red on violet-black muddies into brown at low brightness, and the whole palette is low brightness by construction.

Cyan and magenta together is the CRT phosphor pair, which is where "glowing outline on black" comes from historically. It's the same visual lineage as a spectrum analyzer, which is convenient, since you're building one.

### Visualizer palette — the deliberate choice

**Color the spectrum bars on a weather radar reflectivity ramp.** Low energy green, through yellow, into red, peaking magenta-white.

That's the 24-entry `VISCOLOR` array, and it means the analyser looks like a radar return. Given the app's name and purpose, that's the one place the theme should be literal. Nobody else's player does this because nobody else's player is named after a storm.

Chrome stays cyan/magenta. Radar ramp is visualizer-only. Both are instrument-display languages, so they cohabit without arguing.

---

## Typography

| Role | Face | Why |
|---|---|---|
| Chrome — track title, time, playlist, EQ labels | **Iosevka** | You love it, and it's *narrow*. At 275px, character count is a hard constraint and Iosevka buys you roughly 20% more title before it scrolls. Aesthetics and function agreeing is rare; take it. |
| Modern windows — library, settings, import, downloads | **Iosevka Aile** | The quasi-proportional humanist companion in the same family. One typeface, two registers. Dense tables stay legible without introducing a second family's personality. |
| Kitten theme | **Comic Sans** | Obviously. "So ugly it's cute" is the entire brief for that theme. |

Charter and Crimson Pro are lovely and wrong here — serifs at 11px on dark with a glow halo turn to mush. Save them for doc-md.

---

## How the glow is actually rendered

This matters for performance, and the answer differs by window type.

**Classic chrome (main, EQ, playlist): the sprites carry the shape; the renderer paints the glow from `--arc`** (D73). That is what lets a hand-drawn sheet follow the theme instead of freezing one palette into its pixels. A skin whose halo is already in the art says `"glow": "baked"` in its manifest and gets no second one — every imported `.wsz` is that, so native and imported skins still take one path. The one hard rule is scoped to the 60 Hz path: **no CSS filter on the visualizer surface or any ancestor of it**, whatever the skin declares, because a filter on a parent runs the child through it every frame. The analyser's own glow stays pre-rendered in its ramp art for the same reason.

**Modern windows (library, settings, import): compute it in CSS**, but sparingly:

```css
.edge-active {
  border: 1px solid var(--arc);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--arc) 40%, transparent),
              0 0 12px color-mix(in srgb, var(--arc) 25%, transparent);
}
```

A crisp 1px stroke at full brightness with a soft halo underneath. Not a blurry border — glow reads as glow because there's a *sharp* core inside the diffusion. A single soft `box-shadow` with no hard edge just looks out of focus.

**Never animate `box-shadow`.** Animate `opacity` on a pre-composited glow layer instead.

---

## Signature element: bonds are visible

Your Excel-splitter model is the thing no other player has. Make it the thing people remember.

When two windows magnetize, **the shared edge lights up** — a hairline `--arc` seam running the length of the bond, brighter than any other edge in the interface. You can see at a glance which windows are joined.

- Hover a live splitter: the seam brightens and thickens by a pixel
- Drag it: the seam tracks, both neighbors resize
- Double-click: the seam **discharges** — a fast bloom-then-out, ~120ms, and the windows are free

That's one animation, in one place, doing real work: it tells you the bond existed and that it's now gone. Everything else in the interface stays still. Spend the boldness here and nowhere else.

(Respect `prefers-reduced-motion` — the discharge becomes an instant state change, the seam still disappears.)

---

## The kitten theme — see `purricane.md`

Working name **Purricane.** It has its own document because it isn't a palette swap.

The short version of what this section originally got wrong: **kaleidoscope is a visualizer directive, not a color scheme.** In Eyewall the analyser is spectrum bars on a radar ramp; in Purricane the analyser *is* a kaleidoscope — radial mirrored segments driven by the same FFT data.

Which surfaces a real architectural requirement: **the visualizer is a swappable themed component, not a fixed widget.** The native skin manifest has to support that. Eyewall draws bars, Purricane draws a mandala, and some future theme draws an oscilloscope.

It also resolves cleanly against the only-light-source thesis rather than violating it: Purricane is still emissive, just high-key. A lit aquarium, not a white webpage.

Ships in two pieces — the kaleidoscope visualizer and palette at **v0.5** with the skin system, the kittens at **v0.7** as an external viz-API client. The theme is complete without the cats; the cats are the encore.

---

## Second shipped theme: "Cone"

Live NEXRAD reflectivity as the window backdrop. Named for the cone of uncertainty, which is the right emotional register: you're watching a thing you can't control and don't fully know.

This is the best idea in the project and it's yours. But it has a conflict to resolve first.

### The conflict

**D11 says zero outbound network. A live radar theme needs the network.**

And worse: the app exists for when the network is *gone*. A live radar theme is at its least useful precisely when you most want to look at it.

### The resolution, which is better than the original idea

**Cache aggressively, and make the offline state the feature.**

While you have connectivity — during prep, during the early bands — the theme pulls radar frames on a timer and writes them to disk. When connectivity dies, it doesn't blank or spin. **It loops the last four hours it managed to get, with the timestamp burned into the chrome.**

So during the storm, in the dark, you're watching the last thing the radar saw before you lost contact. Frozen, looping, timestamped, while the music plays.

That's not a degraded fallback. That's the most evocative thing this application could possibly do, and it only works because the app was built offline-first.

### Data sources

| Source | What | Notes |
|---|---|---|
| **IDP-GIS `radar_base_reflectivity_time`** | Time-enabled RIDGE II base reflectivity, REST + WMS | The primary. Four-hour moving window updating every ten minutes, covering CONUS, Alaska, Caribbean, Guam, and Hawaii — a four-hour window is *exactly* the loop you want to cache |
| **`radar.weather.gov/ridge/standard/`** | Per-radar low-bandwidth reflectivity images | Fallback. Explicitly low-bandwidth by design, which matters on a degraded connection |
| **`api.weather.gov/alerts`** | Active watches and warnings | Free, no key. **Requires a `User-Agent` header or you get 403** |
| **`mrms.ncep.noaa.gov/data/`** | GeoTIFF download | If you ever want real dBZ values rather than rendered imagery |

You've already built against api.weather.gov for CoatCheck, so the User-Agent quirk and the rate-limit behavior are solved problems you can lift directly.

### The elegant part

**The dBZ colormap and the `VISCOLOR` array are the same array.**

The radar backdrop and the spectrum analyzer aren't merely coordinated — they're driven by one palette definition. Bass energy and heavy precipitation are the same magenta. That's not a coincidence you engineered; it's what happens when you pick the ramp for a real reason.

### Safety constraint — treat this as non-negotiable

**Never let cached weather data look current.**

Someone glancing at this during a storm must not mistake a six-hour-old loop for a live one. Concretely:

- Age is always visible in the chrome, not buried in a tooltip: `RADAR · 14:32 EDT · 4h 12m old · OFFLINE`
- Past a staleness threshold, desaturate the backdrop and switch the age readout to `--ember`
- Active alerts always render with their issue time, and go struck-through when the data behind them is stale
- The theme never displays a "current conditions" claim of any kind

This is a music player, not a decision-support tool, and the interface should never let anyone confuse the two. A person deciding whether to move to an interior room should not be looking at your backdrop — and if they glance at it anyway, it must be immediately obvious how old it is.

### Implementation notes

- Fetch on a 10-minute timer, matching the source cadence. Never more often
- **Never block startup on the network.** Load cache, render, then fetch in the background. If the fetch fails, nothing changes visually except the age readout
- Frames cache to disk as PNG. Four hours at ten-minute intervals is 24 frames — trivial storage
- Radar site resolves from location once, then persists. Don't geolocate repeatedly
- **Prep mode should pre-cache the radar loop** alongside the bulk download. It's kilobytes next to gigabytes of video, and it's the difference between a full loop and a blank backdrop when the lights go out

### Why Eyewall stays the boot default

Ship Cone, but not as *the* default.

A default theme must work with zero external dependencies. First run, no network, fresh install — the app should look finished, not degraded. Eyewall has no dependencies at all.

Cone is the theme you switch to when a storm has a name.

---

## What design phase must produce

- All six palette tokens applied across every window, at 1x and 2x
- The 24-entry radar ramp, drawn on a real spectrum, not as swatches
- Bonded-edge seam: idle, hover, dragging, discharging
- Every window at rest and focused — remember focus is a *group* property under the bond model
- Windowshade / mini-bar at full fidelity. It's the state you'll actually live in
- ~~The native skin manifest schema~~ — **done, see `skin-manifest.md` (D36).** Both importers are mappings *into* it
- **Cone in all three data states:** live, stale, and no-data-ever. The stale state is the one that matters and the one that will get skipped

## What design phase should not produce

- A light variant of anything. Not even "just to see." That's how light mode gets in.
