# Decision Log

Authoritative. When anything contradicts this file, the other thing is wrong or stale.

**Append, don't rewrite.** Superseded decisions get struck through with a pointer to what replaced them, so the reasoning survives.

| # | Decision | Resolution |
|---|---|---|
| D1 | Shell | Tauri v2 + Svelte 5 + Vite |
| D2 | Fetch backend | yt-dlp sidecar. No custom extractors, ever |
| D3 | MP3 derivation | ffmpeg-extract locally from the downloaded video. Never download twice |
| D4 | Progress parsing | `--progress-template`. Never screen-scrape the progress bar |
| D5 | Audio path | HTML5 `<audio>` → Web Audio graph (EQ + analyser). Chosen so the analyser and EQ come from `AnalyserNode` and `BiquadFilterNode` rather than hand-rolled DSP |
| D6 | Skin strategy | Sprite-ready architecture from day one. `.wsz` loader at v0.5. "Most skins work" is acceptable |
| D7 | Default skin art | Ship our own. Never bundle third-party `.wsz` — that's other people's copyrighted art |
| D8 | Extensibility | **Control API only, and it's public.** No plugin loader, no add-ons folder, no in-process extension points. External programs consume a documented protocol; they never run inside hurricane-party |
| D9 | Control transport | Named pipe / local socket, NDJSON. Not localhost HTTP — a pipe makes the zero-network property true by construction |
| D10 | Job durability | SQLite WAL. On launch, `running` → `queued`. **Refined by D26** — recovery resumes bytes, it does not restart the item |
| D11 | Network posture | Zero outbound at playback time. See D19 for the single exception, D29 for how it's enforced |
| D12 | Window model | True multi-window with bonding. Not a single-window imitation |
| D13 | Video window | Separate, decorated, resizable. **Not** part of the bond group |
| D14 | ~~doc-md integration~~ | ~~**Deferred.** The windowshade mini-bar (always-on-top, 275×14) solves the same problem with no cross-app protocol~~ — **superseded by D15.** The mini-bar does replace doc-md *transport*, but the viz stream justifies the API on its own. The control API is in scope. Anything still saying "control API drops off" is stale |
| D15 | Control API scope | **In scope, public, versioned.** Justified by the viz data stream for third-party hardware, not by doc-md transport |
| D16 | Window bonding | **Excel model.** Title-bar drag moves the group. Shared-edge drag resizes both neighbors. Double-click the edge demagnetizes |
| D17 | Skin formats | Native format primary (9-slice, resizable) · `.wsz` full support · `.wal` **partial, no script execution** |
| D18 | Themes shipped | **Eyewall** (boot default, zero dependencies) · **Cone** (cached NEXRAD backdrop) · **Purricane** (kaleidoscope kawaii) |
| D19 | Network exception | Cone may fetch radar and alerts **on a timer, never at playback time, never blocking startup.** D11 holds otherwise. Cached data always displays its age |
| D20 | Visualizer | A **swappable themed component**, not a fixed widget. Eyewall draws spectrum bars on a radar reflectivity ramp; Purricane draws a kaleidoscope. The skin manifest must express this |
| D21 | EQ | 10 bands (60/170/310/600/1k/3k/6k/12k/14k/16k Hz), ±12 dB per band and preamp — matching the classic range so `.eqf` presets map 1:1. Trim gain after the chain to prevent clipping, plus a clip indicator. **Frequencies verified — see D31** |
| D22 | Kittens | External process, consumes the public viz API. Not a feature inside the player. Dogfoods the protocol, contains the animation subsystem, and a kitten crash doesn't stop the music |
| D23 | Companion packs | Sprite sheet + declarative `companion.json`. **The behavior vocabulary is fixed by the app; packs supply art only.** Same rule as `.wal`: art in, code out. Lets anyone ship unicorns or space marines without a plugin loader |
| D24 | Extensibility, stated whole | Two tiers, neither executing inside the player: **data packs** (skins, companions) for the easy 90%, and **the public protocol** for everything else, running in the author's own process |
| D25 | PoToken | **Bundle `bgutil-ytdlp-pot-provider` as a third sidecar, script mode** — invoked per request, no long-running listener. A localhost HTTP server would sit badly against D11's by-construction framing. The provider is swappable, its health shows in the diagnostics panel, and it follows O11: pinned, never silently auto-updated |
| D26 | Job resume semantics | **True byte-level resume, not restart.** yt-dlp `.part` files are preserved across a crash and re-invoked with `--continue`. `jobs` records the resolved output path and which `stage` it died in, because resuming a download and resuming an ffmpeg extract are different recoveries. D10's `running` → `queued` still applies; it means "re-enter the runner," not "start over" |
| D27 | Milestone authority | **The milestone table below is canonical.** `CLAUDE.md` and every other doc point at it. Duplicated milestone tables elsewhere have been deleted rather than maintained in parallel |
| D28 | Library roots and paths | Many roots from day one (was O8). A `library_roots` table; `media` stores **`(root_id, relpath)`, never an absolute path.** An external drive that comes back as a different letter must not orphan the library — which is the exact case multi-root exists to serve |
| D29 | Zero-network enforcement | A **tested property, not an aspiration.** The webview CSP forbids remote origins outright — no `connect-src`, `img-src`, or `font-src` beyond `self` and `asset:`. All egress lives in Rust behind a single allowlisted radar-fetch command (D19). A test runs the app with the interface down and asserts no connection is attempted |
| D30 | Playlist resize increments | **25 px horizontal, 29 px vertical**, on the 275×116 base. Valid playlist sizes are `275 + 25n` × `116 + 29m`. **Verified** against Webamp (`WINDOW_RESIZE_SEGMENT_WIDTH` / `_HEIGHT`). Closes the open item; the design prototype's `step:10` is off-spec and the spec wins |
| D31 | `.eqf` byte mapping | **Verified.** 31-byte signature (`Winamp EQ library file v1.1\x1A!--`), then a 257-byte name buffer, then **10 band bytes followed by the preamp byte** — preamp is last, not first. Values are `0..63` and **inverted**: `0x00` = **+12 dB**, `0x1F` = 0 dB, `0x3F` = **−12 dB**. Band order matches D21. A naive reader flips every preset upside down and lands the preamp on 60 Hz |
| D32 | Settings storage | A `settings` key/value table in the same SQLite file, not a side config file. Puts settings in the same WAL transaction domain as the library, so a hard power loss can't desync them |
| D33 | Layout persistence | Window geometry **and the bond graph** persist across restart, in physical pixels with the monitor identity recorded. Restoring a group onto a monitor that no longer exists must not strand it offscreen |
| D34 | `media` carries its own title | `media` denormalizes `title`, `uploader`, and `duration_s`. Local folder imports have `source_id NULL` (they have no source), so without this the library browser cannot render a row for a file the user already owned |
| D35 | Splitter on fixed-size pairs | **The cursor tells the truth** (`windows.md` option 3). The splitter cursor appears only on edges where at least one neighbor is resizable; elsewhere it's the move cursor and the drag moves the group. Nothing is inert, because nothing is offered. Costs one capability check at hover time |
| D36 | Native skin manifest | **`hp-skin/1`**, specified in `skin-manifest.md`. It is the format the default skin ships in and the target both the `.wsz` and `.wal` importers map *into* — so it is the load-bearing schema for v0.4 and v0.5 both. Per-window `resizable` capability, 9-slice regions, and a swappable `visualizer` component (D20) |
| D37 | DPI awareness is asserted, not assumed | Tauri does **not** declare DPI awareness in its embedded manifest — `tauri-build`'s `windows-app-manifest.xml` has no `<dpiAware>` element at all. Awareness comes from tao's `become_dpi_aware()`, a four-rung fallback ladder where every rung below the top is a silent `let _ =`. A v1 or system-aware fallback still reads `scale_factor() == 1.5` on a single monitor while breaking cross-monitor behaviour. **Ship an explicit application manifest, and keep a startup assertion** using `AreDpiAwarenessContextsEqual` — the `DPI_AWARENESS` enum cannot distinguish v2 from v1. Verified in the v0.0 spike |
| D38 | Window sizes are declared in physical pixels | `275 × 1.5 = 412.5`. A logically-sized window inherits a half-pixel that the toolkit resolves by its own rounding rule (tao rounds up, to 413). For a bond model whose premise is two windows sitting *flush* with a hairline seam, that means "flush" is defined by tao's rounding mode and a 1 px gap or overlap can appear in the most visually load-bearing place in the app. Use `PhysicalSize`, never the logical `width`/`height` config keys. Verified in the v0.0 spike |
| D39 | Drag smoothness criterion | The original "≤ 2–3 px trail at normal speed" was **wrong** — at 800 px/s it implies under 4 ms end-to-end, a quarter of a frame. Replaced by: **excess over floor** (mean lag − velocity ÷ event rate) **≤ 1 px at slow and normal speed**, cross-checked by **latency ≤ 1.1 frames**. Measured on the spike: 0.18–0.81 px excess, 17.2–17.9 ms (1.03–1.07 frames at 59 Hz). Trail in raw pixels is not a portable criterion — it scales with both velocity and refresh rate |
| D40 | Round once, from logical | **Extends D38.** Physical dimensions are derived from the logical source in a single step and rounded exactly once. Never scale, accumulate, or double an already-rounded physical value. Verified in the spike: `275 × 1.5` rounds to 413, but 2x chrome is `550 × 1.5` = 825, and `413 × 2` = **826** — rounding does not commute with doubling. Worse, it compounds: stepping the playlist by a rounded physical increment (D30's 25 logical px → 38 physical at 1.5) drifts **4 px from true after ten steps**, which walks a bonded neighbour progressively out of flush. The logical size is the source of truth; physical is recomputed fresh, never carried forward |
| D41 | Bond group z-order topology | **Hidden-root ownership.** Every member of a bond group is owned by a never-shown window; no real member is the owner. Measured across six topologies in the spike: this is the only one where *every* member can reach the top of its own group, because an owned window is always above its owner and clicking the owner cannot fix that — a star or chain topology permanently pins one window to the back of its own group. It also shrinks the D16 bond-break problem: splitting a group means creating a second hidden root and re-pointing children, not promoting a member and re-parenting everything around it |
| D42 | Ownership is applied lazily | Windows enforces the owned-above-owner invariant **on next activation, not when `SetWindowLongPtrW` returns.** Re-parenting a live window costs ~28–47 µs with no flicker, no z-position loss, and no activation change — but it also does nothing visible, leaving the z-order stale-but-valid-looking until the next click. A bond break that must look correct immediately has to force it with `SetWindowPos(SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)` after the re-parent |

---

## Milestones

**Canonical per D27.** Work one at a time. Don't build ahead.

| | Done means |
|---|---|
| v0.0 | *Separate throwaway repo.* Window engine spike — three gray rectangles, bonding, splitter resize, double-click break, grouped z-order, two monitors at different DPI. Brief: `spike-v0.0.md` |
| v0.1 | Paste a URL, get an MP3, see it in a list, click it, hear it. Ugly is fine. Proves the sidecar plumbing, including the PoToken provider (D25) |
| v0.2 | Kill the app mid-download, relaunch, it **resumes the bytes** (D26). Playlists persist. Library schema and multi-root paths land here (D28) |
| v0.3 | Video in its own window. Local folder import. Control API: handshake + transport, undocumented and unstable |
| v0.4 | Multi-window bonding, shade modes, Eyewall skin from sprites, EQ audibly works, analyser reacts. Viz stream lands |
| v0.5 | `.wsz` loader. Kaleidoscope visualizer + Purricane palette. `.eqf` import (D31) |
| v0.6 | Prep mode, storage budget, integrity checking, smart playlists, Cone theme |
| v1.0 | Freeze control protocol v1. Publish docs + example client |
| v0.7 | Desktop kittens, external process, viz-API client. After the freeze, not before |

---

## Resolved — promoted from "still open"

These carried recommendations; they are now decisions. Listed separately from the D-table only because they were resolved as a batch.

| # | Question | Resolution |
|---|---|---|
| O3 | Chrome scale steps | Integer only: 1x / 2x, default 2x. Never fractional — it makes sprite bitmaps mushy |
| O5 | Prep mode: own window? | Yes. Opened on demand, decorated, normal size. Not a wizard — it gets used under time pressure |
| O6 | Library browser shape | Flat sortable table with a filter box. Not a tree |
| O7 | Platform targets | Windows first. Keep code portable, don't test elsewhere until needed |
| O8 | Library roots: one or many? | Many, from day one. **See D28** for the path model this forces |
| O9 | Profiles | Skip the feature, but carry `profile_id` on `playlists`, `play_history`, and `jobs`, defaulting to 1. Free now, a migration later |
| O10 | Default storage cap | None. Show the meter, warn at 85% of free disk |
| O11 | yt-dlp updates | Pinned bundle plus an in-app "check for update." **Never silent auto-update** — a surprise yt-dlp bump the day before a storm is the wrong failure. Applies to the PoToken provider too (D25) |
| O12 | Download concurrency | Default 2, adjustable 1–4. Higher gets throttled, not faster |
| O13 | Does the Library window bond? | No. The bond group is Main + EQ + Playlist only — the three classic 275px windows |

## Verified

Both items from the old "to verify before locking" list are closed.

- **Playlist resize increments** → D30. 25 × 29, confirmed against Webamp's source
- **EQ frequency set** → D21, confirmed. The mapping that actually carries the "1:1" claim is the `.eqf` encoding → D31

## Still open

| # | Question | Status |
|---|---|---|
| O16 | WebView2 blank-page flake at startup | **Observed once, unreproduced.** All three webviews came up on WebView2's error page with correct HWNDs but no JS running. Hypothesis: the DPI gate slept ~120 ms on the main thread, which is also WebView2's message pump, aborting navigation. Gate moved to a spawned thread, but **the fix is unproven** — 11 consecutive launches came up clean, including 5/5 on the *old* binary. Recorded as a flake with an unverified mitigation. First place to look if it recurs |
| O15 | Is the webview's input coalescing perceptible? | **Open, low priority.** The spike measured 14.1 px rect-level trail at 800 px/s, of which 13.3 px is the webview coalescing 125 Hz of mouse input down to one `pointermove` per frame. A native Win32 drag loop processing every message would trail ~4 px at rect level. Whether that ~10 px gap is *visible* is unknown — both present at 59 Hz, and photon-level latency was not measured (needs a camera). **Do not chase this now.** Revisit only if drag ever feels loose in real use, and answer it by building fallback 2 as a comparison rather than as a rescue |
| O14 | Second display for DPI validation | **Hardware, not a decision.** The dev machine is a single 2560×1080 at 100%, so cross-monitor coordinate translation cannot be tested. Spike stages 0–5 run at 150% scaling (which catches logical/physical *mixing*); stage 6 needs a borrowed monitor or HDMI TV for one afternoon. Track it as an unrun stage, not a passed one |

New questions get appended here and promoted to a D-number when resolved.
