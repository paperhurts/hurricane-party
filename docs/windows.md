# hurricane-party — Window System Spec

You picked true multi-window. That's now the single highest-risk component in the project, so it gets its own document.

I recommended against it and you overruled me. Fair — the hybrid would have looked like Winamp without *being* Winamp, and the difference is the whole point. But I want to be straight about what it costs and, more usefully, how to find out early whether it's going to work.

---

## Do this first: the gray rectangle spike

**Before the design phase finishes, build a throwaway Tauri app with three undecorated gray rectangles.** No skins, no audio, no library. Just:

- Three borderless 275×116 windows, at least one of them resizable
- Drag any one **by its top 14 pixels** → moves that window
- Bond to another window's edge when within ~10px
- Once bonded, **title-bar drag moves the whole group**
- **Drag a shared edge → resizes both neighbors** (the Excel splitter)
- **Double-click a shared edge → demagnetizes**, and the group correctly splits into two
- Clicking any one raises all of them together
- Works across two monitors with different DPI

The splitter and the double-click-to-break are the parts most likely to surprise you. Test the three-in-a-row case specifically: bond A–B–C, break the middle bond, confirm you end up with two independent groups and not one confused one.

If that works with gray boxes in a weekend, everything after it is decoration. If it doesn't, you've spent a weekend instead of finding out in month three with a half-built skin renderer on top of it.

**This is not optional.** It's the load-bearing assumption of the entire visual design.

---

## What actually makes this hard

Ordered by how likely each one is to eat a week.

### 1. The bond model (hardest) — revised per your Excel-splitter design

You changed this and I think it's better than classic Winamp. Stating it precisely because the whole engine follows from it:

| Gesture | Result |
|---|---|
| Drag a **title bar** | Moves the whole bonded group, offsets preserved |
| Drag a **shared edge between two bonded windows** | **Resize.** Both neighbors adjust, like an Excel column divider |
| **Double-click a shared edge** | Demagnetize. The bond breaks; windows are independent again |
| Drag a window near another | Bond forms within ~10px |

This is a tiling-window-manager model wearing Winamp's clothes, and it's genuinely nicer than positional adhesion. It also means the graph edges carry more than an offset:

```rust
struct Bond {
    a: WindowId,
    b: WindowId,
    edge: Edge,        // A's Right ↔ B's Left, etc.
    span: (i32, i32),  // the overlapping extent of the shared boundary
}

struct WindowGraph {
    bonds: Vec<Bond>,  // connected components = groups
}
```

Rules:
- Title-bar drag moves every member of the connected component
- Breaking a bond runs **connected-components again** — A–B–C, break the A–B bond, and you get two groups. This is where the bugs live; model it as a real graph, not a flat list
- Windows also bond to screen edges (movement constraint only, no resize)

#### The conflict you need to resolve before design

**Classic `.wsz` main and equalizer windows are fixed at 275×116. They cannot resize.** Only the playlist can. So what does dragging the shared edge between Main and EQ do?

Three options:

1. **Splitter is inert on fixed-size pairs.** Edge drag does nothing; only title-bar drag moves them. Honest but feels broken — the cursor changes and then nothing happens.
2. **Falls back to group-move.** Dragging the edge between two fixed windows moves the group. Consistent-feeling, slightly magic.
3. **Cursor tells the truth.** Splitter cursor only appears on edges where at least one neighbor is resizable; elsewhere it's the move cursor. Nothing is inert because nothing is offered.

**I'd take 3.** It's the only one where the interface never lies about what's available, and it costs one capability check at hover time.

This also connects directly to your `.wal` note below — modern skins *are* resizable, so on a `.wal` skin most edges become live splitters and the interaction model comes fully alive. On a classic `.wsz` it degrades gracefully to mostly-move. Same engine, different skin capabilities. That's a clean story.

**Consequence for the theme contract:** every window needs a per-skin `resizable` capability flag, and if resizable, a size step (sprite tiling) or a 9-slice definition. Design phase must lock this.

### 2. Z-order grouping

Clicking the playlist should raise the main window and EQ too, without stealing focus in a way that fights the OS.

Tauri's cross-platform API won't give you this cleanly. On Windows the right primitive is **owned windows** — owned windows always render above their owner and raise with it, so you get grouped z-order from the OS instead of writing it.

> **Corrected by the v0.0 spike.** This section originally said to make the main window the owner of the EQ and playlist HWNDs. **Don't** — that's the "star-main" topology, and it permanently pins the owner to the *back* of its own group: an owned window is always above its owner, and clicking the owner takes focus without fixing the order. Use a **hidden root** instead (D41): a never-shown fourth window owns all three, so no real member is special and any of them can come to the top. Measured across six topologies; hidden-root was the only one where every member could reach the top of its own group.

Reach for `raw-window-handle` + `windows-rs` and call `SetWindowLongPtr(hwnd, GWLP_HWNDPARENT, root_hwnd)`. Platform-specific, but you're Windows-first (O7) so pay it once and put the other platforms behind a trait.

**The exact gap, per the spike:** Tauri exposes `owner()` / `owner_raw()` on `WebviewWindowBuilder` and `WindowBuilder` only. There is no `set_owner` on a live window, and tao's Windows extension trait has none either. Ownership is settable at construction and not at runtime — that asymmetry, not the initial setup, is the entire reason `windows-rs` is required here. Note also that Windows applies the ownership invariant lazily, on next activation (D42).

### 3. Drag implementation

Tauri's `start_dragging()` hands control to the OS, which means **you don't get position events mid-drag** — so you can't snap. You'll need manual dragging:

- `mousedown` on the title bar region → record offset, capture pointer
- `mousemove` → compute new position, run snap check, `set_position()`
- `mouseup` → release, commit group membership

Expect to fight jitter. `set_position` on every mousemove can lag behind the cursor. Throttle to animation frames and accept a pixel or two of lag rather than chasing perfection.

### 4. DPI and multi-monitor

Every window can be on a different monitor at a different scale factor. Snap math must happen in **physical pixels**, consistently. Mixing logical and physical coordinates here produces snapping that works on one monitor and is off by 40% on the other. Pick physical, convert at the boundaries, write it down.

### 5. Non-rectangular windows (`REGION.TXT`)

Some skins define non-rectangular shapes. Defer this entirely to v0.5+ and treat it as best-effort. Options are transparent windows with per-pixel hit-testing, or `SetWindowRgn` on Windows. Most skins' main windows are rectangular; ship without it.

---

## Skin formats — `.wsz` and `.wal`

You want both. Here's the honest asymmetry, because these are not two similar problems:

| | `.wsz` (classic) | `.wal` (modern) |
|---|---|---|
| Contents | ZIP of bitmaps + a few text files | ZIP of XML layout descriptions, PNGs, and compiled scripts |
| Layout | **Fixed, conventional** sprite coordinates | **Declared** in a UI markup language |
| Resizable | No (except playlist) | Yes — that's the point of it |
| Scripting | None | Yes, a compiled bytecode |
| Scope | **Bounded.** Finite sprite sheets, known offsets | **Open-ended.** You are implementing a UI runtime |

`.wsz` is a weekend-to-a-fortnight problem: parse the ZIP, slice known rectangles, done. Webamp proves it's tractable.

`.wal` is not the same size of problem. Full fidelity means implementing a UI description language *and* a script interpreter. That's larger than hurricane-party itself, and it's the kind of thing that quietly becomes the project.

### What MAKI actually is

**MAKI — "Make A Killer Interface."** Modern skins are built on Winamp's Wasabi framework. A `.wal` is a zip containing XML layout definitions, PNG art, and `.maki` files: source written in a C-like language, compiled to cross-platform bytecode, executed by a VM inside the player. Scripts attach handlers to any event on any object in the UI — button presses, visibility changes, playback events, timers — and manipulate the interface imperatively.

It isn't a data format with a few expressions in it. It's a general-purpose programming language with a large host API, and a skin is free to call any of it.

### Four reasons we don't want it

**1. It isn't a parsing problem, it's a reimplementation problem.** `.wsz` is finite — known sprite rectangles at known offsets, and you're done. Supporting MAKI means writing a bytecode VM *and* the entire Wasabi object model the scripts call into. You wouldn't be reading a file, you'd be reimplementing Winamp's internals.

**2. Someone better-positioned already bounced off it.** Jordan Eldredge, who solved classic skins in the browser with Webamp, reverse-engineered MAKI bytecode and got a JavaScript interpreter working as a proof of concept. He shelved it — the API surface was too large and he never found a satisfactory way to connect the nested object model to the DOM. That's the person who nailed the tractable version of this problem.

**3. It's a plugin loader wearing a costume.** This is the decisive one. D8 says no in-process extension points, no arbitrary code inside hurricane-party. A MAKI interpreter is exactly that, reintroduced through the skins folder, executing bytecode from files people download off skin sites. Winamp shipped this in 2002 when nobody thought hard about untrusted local code. Properly sandboxing a bytecode VM is a serious project on its own, and it buys animated transitions.

**4. The scripts target objects we don't have.** MAKI assumes Winamp's window classes, component hierarchy, and config system. Our window model is different — Excel-splitter bonding rather than positional adhesion — and our visualizer is a swappable component. Even a flawless VM would run scripts manipulating objects that don't exist here.

### The good failure mode

Most MAKI in the wild is decorative: animations, transitions, custom config dialogs. Ignoring it yields **static but correct** — the skin loads, the art is right, the layout is right, the sparkles don't sparkle. That's a fine outcome, and it's honest as long as it's documented.

**The rule, stated once: art and layout come in, code does not.** That line shows up again in the companion format for the kittens, and for the same reason.

### Recommendation: three tiers

1. **Native format is primary.** JSON manifest + 9-slice PNGs + a `resizable` capability per window. Resizable by design, which is what the splitter model wants. This is what the default skin ships in and what new skins should target.
2. **`.wsz` importer — full support.** Bounded, well-understood, and it's where the enormous existing skin library lives.
3. **`.wal` importer — partial, explicitly.** Parse the XML layout, map what corresponds to native concepts, render the PNGs, **ignore the bytecode.** Document it as "many modern skins load; heavily scripted ones will look right but sit still."

"It's a feature, not the point" is exactly right — and tier 3 is how you keep it that way. Promising `.wal` fidelity is how it stops being a feature and becomes the point.

Design phase needs the native manifest schema locked, since both importers are mappings *into* it.

---

## Window inventory

| Window | Size | Decorated | Snap group | Shade mode |
|---|---|---|---|---|
| Main | 275 × 116 | no | ✅ | ✅ → 275 × 14 |
| Equalizer | 275 × 116 | no | ✅ | ✅ → 275 × 14 |
| Playlist | 275 × 116 default, resizable | no | ✅ | ✅ → 275 × 14 |
| Library | arbitrary | **yes** | ❌ | ❌ |
| Video | arbitrary | **yes** | ❌ | ❌ |
| Downloads | arbitrary | **yes** | ❌ | ❌ |
| Prep mode | arbitrary | **yes** | ❌ | ❌ |
| Settings | arbitrary | **yes** | ❌ | ❌ |

**The rule:** the three classic 275px windows are skinned, undecorated, and snap. Everything else is a normal OS window with modern chrome. Don't try to make the library window sprite-skinned — you'd be inventing sprite layouts Winamp never had, and no `.wsz` file contains art for it.

All dimensions above are at 1x. Double them for 2x mode.

**Playlist resize increments: 25 px horizontal, 29 px vertical.** Verified against Webamp's source (`WINDOW_RESIZE_SEGMENT_WIDTH = 25`, `WINDOW_RESIZE_SEGMENT_HEIGHT = 29`, on `WINDOW_WIDTH = 275` / `WINDOW_HEIGHT = 116`). Locked as **D30**.

So every valid playlist size is `275 + 25n` wide by `116 + 29m` tall, for non-negative integers *n* and *m*. The splitter must quantize to these steps, not to pixels — which matters for the bond model, because a resize that snaps in 29px jumps has to keep the bonded neighbor flush at every intermediate step, not just at the end of the drag.

> ⚠️ The design prototype (`design/screens/PlaylistWindow.dc.html`) declares its height prop as `step:10, min:58`. That's off-spec. Per `CLAUDE.md`, the prototype is not the spec and the spec wins — build to 29.

---

## The mini-bar is already built into this

Your floating mini-player **is Winamp's windowshade mode.** Main window collapses to a 275×14 bar with transport buttons, a small time readout, and a scrolling title. Add always-on-top and you've got exactly what you wanted when you're deep in doc-md — and it's authentic rather than bolted on.

Which means **doc-md transport got deferred** (D14). You wanted doc-md to control playback because you didn't want to leave doc-md. An always-on-top 275×14 bar solves that with zero cross-app protocol, zero shared crate, zero version drift.

That's a real simplification, and I don't think you made it on purpose — it fell out of preferring the authentic window model. Worth noticing.

> **Correction.** An earlier draft of this document concluded the control API was deferred "possibly permanently." That was wrong, and **D15 supersedes it.** The mini-bar replaces the *transport* justification, but it does nothing for the **viz data stream** — and nothing else in the app provides that. `hp-control` is in scope, public, and versioned, landing at v0.3. See `control-api.md`.

---

## Design phase implications

Things the prototype now has to show that it wouldn't have otherwise:

- **Each window independently**, at 1x and 2x
- **Each window's shade state** — three more layouts, one of which is your mini-player
- **Snapped configurations** — at minimum the classic stack (main on top, EQ, playlist below) and a couple of alternates, so the sprite edges are checked for seams
- **Title bar active vs inactive** — Winamp skins ship both, and the snap group means "active" is a *group* property, not a per-window one. Getting this wrong looks broken immediately.
- **The playlist at several valid sizes**, since it tiles

Prototyping snapping *behavior* in a design tool is mostly wasted effort. Show the resulting states as static configurations and let the spike prove the behavior.

---

## Revised milestone impact

Multi-window pushes work earlier, because the skin renderer sits on top of it and you don't want to build the renderer twice.

**The milestone table that used to live here has been deleted.** It had drifted — it still said "Control API drops off," which D15 reversed. `decisions.md` carries the canonical table (D27).

The two points specific to this document that survive:

- **v0.4 is the big one**, and is worth splitting into v0.4a (window system) and v0.4b (skin renderer). They are separable and the second is much easier once the first is proven
- **`REGION.TXT` is explicitly best-effort at v0.5.** Non-rectangular windows are not a v1.0 commitment

---

## Honest risk note — answered by the spike

The prediction was right and the magnitude was pessimistic.

> The Tauri multi-window path here is less traveled than the single-window one. You will hit at least one thing where the cross-platform API doesn't expose what you need and you drop to `windows-rs`.

**Measured: four calls** (D44). Ownership get/set via `GWLP_HWNDPARENT` — the real gap, because Tauri exposes `owner()` on *builders* only and has no `set_owner` on a live window — plus `SetWindowPos` for D42's lazy application, plus the D37 DPI assertion. Everything else stages 0–5 needed was covered cross-platform and was already physical-first. **The trait is a file, not an archaeology project.**

The spike returned **go** on the bond model (D45): drag costs one display frame with 0.2–0.8 px over the theoretical floor, owned HWNDs group z-order and re-parent in ~40 µs with no visual disturbance, and bonds form and break correctly with zero drift over twenty group drags and zero seam error across 61 splitter steps in both the model and the OS. The one open question is cross-scale behaviour (stage 6, O14), still blocked on hardware.

### The trap that nearly ate the signature interaction

**Build every window `resizable(false)`, including the playlist** (D43).

For an undecorated *resizable* window, `tauri-runtime-wry` spawns an invisible `TAURI_DRAG_RESIZE_WINDOW` overlay that hit-tests **above** the webview in a band roughly 8 physical px wide at 150%. That band sits exactly on a bond seam — so it covers precisely the edges D35 makes interactive, and it silently kills seam double-clicks and splitter drags alike. `set_size()` still works with resizing off, and the app does its own resizing through the splitter, so the native affordance was never wanted.

Worth noting *how* this was found: the scripted sweeps passed at 0 px error while the real interaction was completely dead, because they bypassed the mouse. It only surfaced when stage 5 required a real double-click. **Test the interaction the way a user performs it, at least once per stage.**
