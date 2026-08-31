# hurricane-party — Design Brief

Self-contained. Claude Design won't have the architecture conversation, so everything it needs is here.

---

## First: don't paste all of it at once

Twelve windows × five states in one session produces a blend of two design languages, and the blend is wrong for both. The classic chrome is fixed-pixel sprite work at 275px. The modern windows are normal desktop app UI. Those are different jobs.

**Three passes:**

| Pass | Scope | Why separate |
|---|---|---|
| **1** | Main, EQ, Playlist + shade states, Eyewall theme | This is the identity work. Highest value, hardest constraints. Do it alone |
| **2** | Library, Import, Downloads, Prep mode, Settings | Modern desktop UI in the same palette. Different mode of working |
| **3** | Cone radar states | Variation on a locked foundation |
| **4** | Purricane — kaleidoscope visualizer + kittens | Its own design problem, not a palette swap |

Pass 1 is the one that matters. If it comes out right the rest follows.

---

# PASS 1 BRIEF — paste from here

## What this is

A desktop media player for saving YouTube playlists, videos, and MP3s to disk so you can watch and listen when the internet is down during a hurricane. Windows-first, built with Tauri and Svelte.

It's styled after classic Winamp: small fixed-size skinnable windows that magnetize to each other. This is a personal tool for one engineer in Gainesville, Florida, not a product for a market.

## The thesis, which should drive every decision

**During a hurricane, this screen is the only light source in the room.**

Everything glows because everything is emitting light. Nothing is lit from outside. That's why the app is dark-only and why it has no light mode — a light theme would be lying about the conditions it was built for.

Design for a dark room, at 2am, with the power out.

## Hard constraints

- **Dark only.** Do not produce a light variant of anything, not even to compare
- Main, Equalizer, and Playlist windows are **exactly 275 × 116 logical pixels**. This is not adjustable
- Windowshade (collapsed) state is **275 × 14**
- Rendered at integer scale only: 1x or 2x. Default 2x. Never fractional
- Undecorated — no OS title bars. The windows draw their own chrome
- Playlist resizes in discrete increments because its frame is built from tiled sprites — **25 px horizontally, 29 px vertically** (D30, verified). Valid sizes are `275 + 25n` × `116 + 29m`. Main and EQ do not resize at all
- **Reference Winamp's layout conventions; do not reproduce Winamp's artwork.** Those skins are other people's copyrighted work. This is original art in that tradition

## Palette — use these exact values

| Token | Hex | Role |
|---|---|---|
| `--void` | `#0C0A14` | Window fill. Near-black with a violet cast |
| `--well` | `#05040A` | Insets — playlist background, seek trough, EQ bed |
| `--filament` | `#E8F4FF` | Bright text. Cold near-white, reads as emission not paint |
| `--arc` | `#6FE3FF` | Primary glow. Outlines, borders, active edges |
| `--strike` | `#FF4FD8` | Now-playing, current row, peak indicator |
| `--ember` | `#FFB347` | Warnings, storage pressure, failed state |

Cyan and magenta together are the CRT phosphor pair — the historical source of "glowing outline on black."

### Visualizer palette — deliberate and important

The spectrum analyzer bars use a **weather radar reflectivity ramp**: low energy green → yellow → red → peaking magenta-white. 24 steps.

The analyser should look like a radar return. Given the app's name and purpose, this is the one place the design should be literal. Draw it on a realistic spectrum, not as a swatch strip.

## Typography

- **Iosevka** for all classic chrome — track title, time display, playlist rows, EQ labels. Chosen partly because it's narrow: at 275px, character count is a hard constraint and Iosevka buys ~20% more track title before it scrolls
- No serifs anywhere. At 11px on dark with a glow halo they turn to mush

## How glow should look

A **crisp 1px stroke at full brightness with a soft halo underneath.** Glow reads as glow because there's a sharp core inside the diffusion. A single soft blurred border with no hard edge just looks out of focus.

## Signature element — the bond seam

These windows magnetize to each other. Unlike Winamp, a shared edge between two bonded windows behaves like an Excel column divider: **drag it to resize both neighbors, double-click it to break the bond.**

Make that visible. **When two windows are bonded, the shared edge lights up** — a hairline `--arc` seam brighter than any other edge in the interface. You can see at a glance which windows are joined.

Four states to draw:
1. **Idle** — bonded, hairline seam
2. **Hover** — seam brightens and thickens by a pixel
3. **Dragging** — seam tracks the cursor, both neighbors resizing
4. **Discharging** — the double-click break. Fast bloom, then out, ~120ms

This is the one animated moment in the app. Everything else stays still. Spend the boldness here and nowhere else. Honor `prefers-reduced-motion` by making the discharge an instant state change.

## Windows to design

### Main (275 × 116)
Track title with scroll-on-overflow, elapsed time, seek bar, transport buttons (prev / play / pause / stop / next), volume, balance, the spectrum analyser, and toggle buttons for the EQ and Playlist windows.

### Equalizer (275 × 116)
Preamp plus 10 bands, preset dropdown, on/off and auto toggles.

Band labels are the classic ten in Hz: 60, 170, 310, 600, 1k, 3k, 6k, 12k, 14k, 16k. Range is ±12 dB per band. Draw at least three states: flat, a preset applied with visible curve, and the clip indicator lit. The `auto` toggle means "load this track's saved preset on play" — make it read as a per-track binding, not a global on/off.

### Playlist (275 × 116 default, resizable)
Track rows with index, title, duration. Current track in `--strike`. Selection state. Scrollbar. Bottom strip with total duration and add/remove controls.

### Windowshade states (275 × 14 each)
All three collapsed. **The Main shade is the most important single screen in this brief** — it doubles as an always-on-top floating mini-player, and it's the state the owner will actually live in while working in another app. It needs the track title, transport, and a time readout in fourteen pixels of height.

### Required combinations
- The classic stack: Main above EQ above Playlist, all bonded, with the seams visible
- One alternate arrangement, to check sprite edges for seams
- Playlist at two different sizes
- **Focused vs unfocused** — note that focus is a *group* property. When any bonded window has focus, all of them read as active. Getting this wrong looks broken immediately

## Copy voice

Short, active, no apology. `Save` not `Submit`. If a button says `Download`, the toast says `Downloaded`. Errors state what happened and what to do:

> Couldn't reach YouTube. Check your connection, or try again later.

Empty states are invitations, not apologies.

## Explicitly do not prototype

- **Snapping and drag behavior.** That's being validated separately in a code spike. Show bonded configurations as static states
- Any light variant
- The library, import, downloads, settings, or prep windows — those are pass 2
- Winamp's actual skin art

## Deliverable

Clickable prototype. Transport buttons respond, shade toggle collapses and expands, EQ and Playlist toggles show and hide, playlist rows select, the bond seam shows all four states. The visualiser can be a static or looping representation — it doesn't need real audio.

# END PASS 1 BRIEF

---

## Pass 2 — the modern windows

Run after pass 1 locks. Add to the top:

> The classic 275px chrome is already designed and locked — see attached. These windows are different: normal desktop windows with OS decoration, arbitrary size, modern layout. Same palette, same glow language, but they are not sprite-based and should not pretend to be. Winamp never had these windows and no skin file contains art for them.
>
> Body typeface here is **Iosevka Aile**, the quasi-proportional companion to Iosevka. One family, two registers.

Windows:

- **Library** — flat sortable table with a filter box. Columns: title, uploader, duration, kind, size, source. Not a tree
- **Import (paste)** — URL field, a Probe action, recent imports
- **Import (probe results)** — checklist of found items with per-item size estimates, running total, select all/none, audio-only toggle per item
- **Downloads** — active, queued, and failed jobs. Per-item progress, pause / retry / cancel
- **Prep mode** — bulk paste area, total GB against free disk space, one large go button. Used under time pressure with a storm coming, so it must be fast, not a wizard
- **Settings** — library roots, storage budget, download concurrency, chrome scale, skin picker, and a diagnostics panel showing helper-tool health

Every one needs five states: **empty, loading, populated, error, offline-degraded.**

The last one is the whole point of the app and the one that gets skipped. What does the Import window look like with no network? Disabled with a clear explanation — never a spinner that never resolves.

## Pass 3 — Cone

**Cone** puts a live weather radar loop behind the chrome, cached to disk so it keeps looping when the network dies. Three states:

1. **Live** — recent frames, full saturation
2. **Stale** — desaturated backdrop, age readout in `--ember`. `RADAR · 14:32 EDT · 4h 12m old · OFFLINE`
3. **No data** — never fetched. Falls back cleanly to plain Eyewall

The stale state is the one that matters and the one that will get skipped. **Cached weather data must never look current** — someone glancing at this during a storm has to see instantly how old it is.

## Pass 4 — Purricane

A full brief lives in `purricane.md`. Hand Design that document, plus the Pass 1 output for geometry. The short version of why it's its own pass:

**The kaleidoscope replaces the spectrum analyser.** This isn't a recolor of Eyewall — the visualizer itself is a different component. Radial mirrored segments driven by the same FFT data, rotating slowly, blooming on beat. That means the native skin manifest has to treat the visualizer as a **swappable component**, which is a real architectural requirement this theme surfaced.

Same 275px geometry, entirely different personality. Comic Sans, set larger than Iosevka would sit — small x-height relative to cap height, it looks cramped at 11px.

It is *not* light mode. It's still emissive, just high-key — a lit aquarium, not a white webpage. Text is dark on light here; everything else still glows.

**Draw the accessibility states, not just the pretty one.** Rotating high-contrast radial patterns are a genuine photosensitivity trigger in a way spectrum bars are not. The reduced-motion variant — a static mandala responding in amplitude only — is a required deliverable, not a nice-to-have.

Kittens are a separate external application, designed after the theme lands.

---
# output for hurricane-party from claude design

Use the claude_design MCP (https://api.anthropic.com/v1/design/mcp, auth via /design-login) to import this project:
https://claude.ai/design/p/12fea1d6-8208-450d-bbc4-2fa73f6ab89c

Focus on these files (the whole project is readable):
- `ConeBackdrop.dc.html`
- `ConeMain.dc.html`
- `ConeReadout.dc.html`
- `DownloadsWindow.dc.html`
- `EqWindow.dc.html`
- `ImportPasteWindow.dc.html`
- `ImportProbeWindow.dc.html`
- `Kaleidoscope.dc.html`
- `Kitten.dc.html`
- `LibraryWindow.dc.html`
- `MainWindow.dc.html`
- `PlaylistWindow.dc.html`
- `PrepWindow.dc.html`
- `PurricaneMain.dc.html`
- `PurricanePlaylist.dc.html`
- `Radar Pass 2.dc.html`
- `Radar Pass 3 - Cone.dc.html`
- `Radar Pass 4 - Purricane.dc.html`
- `Radar Player.dc.html`
- `SettingsWindow.dc.html`
- `support.js`

Implement: the selected files