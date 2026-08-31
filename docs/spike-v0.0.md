# v0.0 — Window Engine Spike

**Status:** brief. The spike itself lives in a separate throwaway repo (`hurricane-party-spike`), not here.

**Timebox: one weekend.** If it's still fighting you on day three, that *is* the finding. Write it up and stop.

---

## What this is for

The Excel-splitter bond model is the load-bearing assumption of the entire visual design. The skin renderer sits on top of it. If Tauri v2 on Windows can't support it, you need to know that now — not in month three with a half-built sprite renderer already committed to it.

**The output of this spike is a written finding, not code.** When it's done, write `spike-findings.md` — answering each question below with a measured number or a plain yes/no — and hand it back to the main `hurricane-party` repo, where the answers become entries in its decision log.

Assume the code is deleted afterward. **One exception:** the bond graph module and its unit tests (stage 3) are pure logic with no Tauri in them, and those get ported. Everything else is scaffolding for a measurement.

That framing matters, because the main way a spike like this overruns is that it starts becoming the app.

## Explicitly not in this spike

No skins. No sprites. No audio. No SQLite. No yt-dlp. No Svelte stores, no router, no component library. Three gray rectangles with a 14px darker strip across the top.

If you find yourself picking a color, you've drifted.

---

## Display configuration — do this before you write anything

**Set the monitor to 150% scaling and leave it there for the whole spike.**

The dev machine is a single 2560×1080 at 100%. At scale factor 1.0, physical pixels and logical pixels are *the same number*, so every coordinate-conversion bug is invisible and the spike would pass while telling you nothing about the most pervasive risk in the project.

The two failure modes are different and only one is testable here:

| | Catches | Testable on this machine |
|---|---|---|
| Single monitor at **150%** | Assuming 1 logical px = 1 physical px. A 10px snap threshold that's silently 15 physical px; wrong window sizes at 2x chrome | **Yes — free, do it** |
| Two monitors at **different** scales | Cross-monitor translation: "within 10px" meaning different things on either side of a boundary | **No.** Needs a second display |

Consistency failures and translation failures are not the same bug. Using logical coordinates *everywhere*, consistently, works fine on one monitor at any scale — consistently wrong is indistinguishable from right. 150% surfaces the *mixed* case, where one path reads `outer_position()` (physical) and another sets a threshold in CSS pixels (logical).

**Stage 6 is therefore blocked on hardware.** Run stages 0–5 at 150%, and borrow a monitor or HDMI TV for an afternoon to close stage 6. Do not silently skip it — an unrun stage is a finding ("not tested"), not a pass.

---

## Coordinate discipline — establish this before stage 1

**All snap and bond math is in physical pixels.** Convert at the boundaries only. (This is a standing convention of the parent project, not a spike choice — keep it even though the spike is otherwise allowed to be sloppy.)

In Tauri v2 that means `PhysicalPosition` / `PhysicalSize` throughout, `outer_position()` and `outer_size()` for reads, and never letting a logical coordinate into the bond graph. Set this up before writing any geometry, because retrofitting it means re-deriving every threshold.

Write down the scale factor of each monitor in the findings doc. You will want it when a number looks wrong.

---

## Stage order, and why

The stages are ordered by **whether a failure is survivable**, not by build convenience.

Stages 1 and 2 ask *"can the platform do this at all?"* — no amount of good code rescues a no. Stages 3–5 ask *"can I write this correctly?"*, which is just work. So the platform questions get answered against the cheapest possible prototype, before any effort is sunk into graph logic that a stage-1 failure would throw away.

**Each stage gates the next.** A stage that fails stops the spike; write up what you learned and bring it back.

---

## Stage 0 — Skeleton

Three borderless, undecorated 275×116 windows. One of them (stand-in for the playlist) resizable. A 14px strip at the top of each, visually distinct, as the title-bar hit region.

**Pass:** three windows appear, undecorated, correct physical size on the primary monitor at its actual scale factor.

This is setup, not a test. If Tauri v2 makes *this* hard, that's a finding worth reporting immediately.

### First, a gate that can silently void the entire spike

**Confirm the app is per-monitor DPI aware before trusting a single number.**

Windows lies to processes that don't declare DPI awareness. A DPI-unaware process is told the scale factor is 96 (100%) and gets virtualized coordinates, while Windows bitmap-scales its output. Everything *looks* fine and every measurement is worthless — you'd be measuring Windows' scaling, not your own math.

This is not hypothetical. On this machine at 150%, a DPI-unaware PowerShell probe reports raw DPI 96 while the real logical desktop is 1707×720 (= 2560/1.5 × 1080/1.5). The lie is convincing.

So, before stage 1:

- Log `window.scale_factor()` from Rust. **It must read 1.5, not 1.0.** If it says 1.0, the app is DPI-unaware and nothing downstream means anything
- Cross-check `outer_size()` against the known 2560×1080 physical desktop. A 275-logical-px window should be **412 physical px** wide at 150%
- Tauri v2 should set per-monitor-v2 awareness via its embedded manifest — but **verify it, don't assume it.** If it's wrong, fix it in the Windows application manifest before going further

Record the observed scale factor in the findings doc. It's the number every other measurement is relative to.

---

## Stage 1 — The drag loop ← **the make-or-break**

This is the single measurement the spike exists for. Do it first, before any bond logic.

Tauri's `start_dragging()` hands control to the OS and **stops giving you position events mid-drag**, so you can't snap. That forces manual dragging:

- `mousedown` in the top 14px → record the cursor-to-window offset, capture the pointer
- `mousemove` → compute the new physical position, call `set_position()`
- `mouseup` → release

**The question: is `set_position()` per mousemove smooth enough to ship?**

Not "does it work" — it will work. Whether it *feels* like a window or like a laggy remote desktop session is the thing, and it's unfixable by cleverness if the answer is no.

### Instrument it, don't eyeball it

"Feels laggy" is not a finding. Log to CSV on every move: timestamp, cursor physical position, window physical position, and the delta. Then you can state the result as a number.

Measure at three speeds — slow (~200 px/s), normal (~800 px/s), and a fast flick (~3000 px/s).

**Record:**

- Cursor-to-window lag in physical pixels at each speed
- Whether `set_position` calls coalesce or queue up behind you (does the window keep moving after you stop?)
- Frame pacing — are you actually landing one update per animation frame, or dropping?
- Behaviour at 2x scale specifically, since that's the default chrome scale (O3)

**Run all of this at 150% scaling** (see the display section). The scale factor is the point: at 100% a logical/physical mix-up is invisible, at 150% it's a 50% error and you see it on the first drag.

Concretely, the thing to watch for on day one: put the window at a known physical position, read it back, and confirm the number you set is the number you get. If `set_position(PhysicalPosition{x:1000,..})` reads back as 1500 or 667, you have found the bug class the whole coordinate convention exists to prevent — and you found it in an hour rather than in month three.

**Pass (D39 — revised after the first run):**

- **Excess over floor ≤ 1 px** at slow and normal speed, where `floor = velocity ÷ measured event rate`
- **Latency ≤ 1.1 frames**, i.e. `mean lag ÷ velocity` against the display's refresh interval
- No runaway after mouseup, no dropped frames, no stutter at 2x chrome
- Set/read round-trips exactly at 150%

> The original criterion here was *"≤ 2–3 px trail at normal speed."* **That was wrong** and the first run correctly rejected it: 2–3 px at 800 px/s implies under 4 ms end-to-end, a quarter of a frame, which measures the display rather than the code. Raw pixel trail is not a portable threshold — it scales with both velocity and refresh rate. Excess-over-floor is.

**If it fails**, the fallbacks in rough order of cost:

1. Throttle to `requestAnimationFrame` and accept a pixel or two — try this first, it's likely enough
2. Move the drag loop into Rust: handle it in the Win32 message loop with `SetWindowPos`, so the round trip never crosses the IPC boundary
3. Drop to `windows-rs` entirely and drive `WM_MOVING` / `WM_ENTERSIZEMOVE`, snapping by adjusting the proposed rect the OS hands you — this is how native apps do it, and it's the option most likely to actually be correct

Fallback 3 is a real answer, not a defeat. Note it before you need it so a stage-1 failure doesn't read as a dead end.

---

## Stage 2 — Grouped z-order

Clicking any window in a bond group raises all of them, without fighting the OS over focus.

Tauri's cross-platform API won't do this cleanly. The Windows primitive is **owned windows**: `SetWindowLongPtr(hwnd, GWLP_HWNDPARENT, main_hwnd)` via `raw-window-handle` + `windows-rs`. Owned windows always render above their owner and raise with it — you get grouped z-order from the OS instead of writing it.

**Pass:** click any of the three, all three raise together, focus lands sensibly, and none of them get stuck permanently on top of unrelated apps.

**Watch for:** ownership is a *tree*, not a set. Three peer windows can't all own each other. You'll likely nominate one as the group owner — which means **the owner has to change when bonds break** (stage 5). Note how re-parenting behaves at runtime; if `SetWindowLongPtr` on a live window flickers or drops z-position, that's a finding.

This is the second platform question, and like stage 1 it's unsalvageable by good code. Answer it before building the graph.

---

## Stage 3 — Bonding and group move

Now the correctness work starts.

- A window dragged within ~10px (physical) of another's edge **bonds** — snaps flush, records the bond
- Once bonded, **title-bar drag moves the whole connected component**, offsets preserved
- Windows also bond to screen edges (movement constraint only, no resize)

Model it as a real graph from the start. This is the parent project's specified model, not something to redesign:

```rust
struct Bond { a: WindowId, b: WindowId, edge: Edge, span: (i32, i32) }
struct WindowGraph { bonds: Vec<Bond> }  // connected components = groups
```

**Do not use a flat list.** The docs say this and they're right — stage 5 is where a flat list falls over, and by then you've built on it.

**Worth knowing:** the graph is pure logic with no windows in it. Connected components, bond formation, break behaviour — all of it unit-tests with fake geometry and no Tauri at all. Write those tests.

**This is the one module that gets ported back**, so it's the one place in this repo where clean code and real tests pay for themselves. Everything else here is disposable; this file is a draft of something real.

**Pass:** bond forms reliably at the threshold, group move preserves offsets exactly, no drift after twenty drags.

---

## Stage 4 — Splitter resize

Drag a shared edge between two bonded windows → both neighbors resize, like an Excel column divider.

**The resolved conflict (already decided upstream as D35 — do not relitigate it):** classic main and EQ windows are fixed at 275×116 and cannot resize. So the **cursor tells the truth** — the splitter cursor appears only on edges where at least one neighbor is resizable. Elsewhere it's the move cursor. Nothing is inert, because nothing is offered.

That's one capability check at hover time. Implement it here, because it's the interaction that proves the model, and a fixed/fixed pair is half the test cases.

**Pass:** dragging a live splitter resizes both neighbors with no gap and no overlap at any point during the drag; a fixed/fixed edge shows the move cursor and moves the group.

**Watch for:** the two `set_size` + `set_position` calls are not atomic. A one-frame gap or overlap flash between neighbors is the most likely visible defect, and it's exactly the seam the whole design is meant to celebrate. Note whether it happens.

---

## Stage 5 — Breaking bonds ← **where the bugs live**

Double-click a shared edge → demagnetize. Then re-run connected components.

**Test the three-in-a-row case explicitly.** Bond A–B–C. Break the **middle** bond. You must end up with two independent groups, not one confused one.

Then the cases the docs don't name, which is where this will actually break:

- A–B–C, break the middle, then drag A back onto C. Does it bond cleanly, or does stale state from the old group leak in?
- A–B–C in an **L shape** (A right of B, C below B). Break A–B. Does C stay with B?
- A–B–C, break the middle, **then check stage 2 still holds** — do the two new groups have correct, independent z-order? This is where the owner-reparenting question from stage 2 comes due
- Break a bond while a splitter drag is in flight on the *other* edge of the same window

**Pass:** connected components is correct in all of the above, and z-order follows the split.

---

## Stage 6 — Multi-monitor sweep

You already smoke-tested the DPI boundary in stage 1. Now do it properly, with a full group.

- Two monitors at **different** scale factors (e.g. 150% and 100%)
- Drag a bonded group of three across the boundary — do bonds survive, do offsets stay exact?
- Bond a window to a screen edge on the secondary monitor
- Form a bond with two windows **on different monitors at different scales** — does the 10px threshold mean the same thing to both?
- Disconnect a monitor while a group is on it. Does the group strand offscreen?

**Pass:** snap thresholds behave identically on both monitors, and nothing strands.

If stages 1–5 passed and this fails, it's a coordinate-conversion bug, not a model failure — survivable, but find it now.

---

## The findings doc

When the spike ends — passed, failed, or timed out — write `docs/spike-findings.md` in **this** repo answering:

1. Stage 1's measured numbers, at all three speeds and both scale factors. Which drag implementation won
2. Whether owned-HWND z-order works, and how runtime re-parenting behaves when a group splits
3. Any place the Tauri cross-platform API came up short and you dropped to `windows-rs` — this is the list that tells you how big the eventual platform trait is
4. Whether the splitter produces a visible gap or overlap flash mid-drag
5. Which of the stage 5 edge cases broke, and whether the graph model handled them or needed reshaping
6. **A go / no-go on the bond model, in one sentence**

Items 1, 2 and 6 become decisions in the main repo. Item 3 becomes the shape of the platform trait the parent project requires — it mandates that platform-specific calls sit behind a trait rather than scattered `#[cfg(windows)]`. **The spike deliberately does not build that trait** (see the ground rules appendix); its job is to produce the list of what would go in it.

---

## What a "no" looks like

Worth saying out loud so it's a real option rather than an admission of failure.

If stage 1 can't be made smooth and fallback 3 turns out to be a large project, the honest answer is that **true multi-window with manual drag isn't viable on this stack** — and the fork is between changing the window model and changing the stack. That's a decision for the main repo's log, made deliberately, with a measurement behind it.

A weekend spent finding that out is the spike working exactly as intended.

---

# Appendix — ground rules for the spike repo

Copy everything in the fenced block below into `hurricane-party-spike/CLAUDE.md` before starting. Then this brief plus that file is the whole handoff; nothing else needs to come across.

**Why this exists:** an agent (or a person) dropped into an empty repo with no instructions defaults to *good engineering* — abstract the platform layer, avoid magic numbers, handle the errors, structure it nicely. Every one of those instincts is wrong here, and acting on them is exactly how a weekend spike becomes a three-week project. The spike needs explicit permission to be sloppy, and a written list of the few things it may not be sloppy about.

```markdown
# hurricane-party-spike

A throwaway measurement rig. **This repo gets deleted when the spike ends.**

It exists to answer six questions about whether Tauri v2 on Windows can support
an Excel-splitter window bonding model. Read `spike-v0.0.md` for the questions,
the stage order, and the pass/fail criteria. Do not deviate from the stage order —
it is deliberately sorted by which failures are unsurvivable, not by build convenience.

## What you are optimizing for

Answering the six questions, fast, with numbers. Nothing else. Not correctness,
not maintainability, not elegance, not test coverage.

## You have explicit permission to

These are violations of the parent project's rules. They are correct here.

- **Hardcode everything.** Colors, sizes, offsets, paths. The windows are gray
  rectangles; `#808080` is the right answer and a design token is the wrong one
- **Scatter `#[cfg(windows)]` freely.** Do NOT build a platform abstraction trait.
  The parent project requires one; this spike deliberately does not build it,
  because *counting how many places need it* is one of the deliverables
- **`unwrap()` and `expect()` everywhere.** No error handling, no `Result` plumbing
- **Don't abstract until the third repetition.** Copy-paste twice before extracting
- **Keep it in one file** until that becomes genuinely painful
- **No framework anything.** No component library, no state management, no router,
  no CSS framework, no build tooling beyond what `create-tauri-app` gives you

## You may not skip

Short list, and it's short on purpose.

- **Physical pixels in all snap and bond math.** `PhysicalPosition`/`PhysicalSize`,
  `outer_position()`/`outer_size()`. Never let a logical coordinate into the graph.
  This is the one parent-project convention that survives into the spike, because
  the bug it prevents is one of the things being measured
- **Run at 150% display scaling.** At 100% the coordinate bugs are invisible
- **Instrument stage 1 with a CSV log.** "Feels laggy" is not a finding. Timestamp,
  cursor physical position, window physical position, delta. Three drag speeds
- **Write `spike-findings.md`.** It is the actual deliverable. The code is not
- **Respect the stage gates.** A failed stage stops the spike. Do not work around
  a stage-1 or stage-2 failure to get to the more interesting graph work

## The one exception to all of the above

**The bond graph deserves care.** Connected components, bond formation, and break
behaviour are pure logic with no windows in them — they unit-test with fake geometry
and no Tauri at all.

That module is the single artifact that gets ported back rather than deleted, so it
is the one place where clean code and real tests pay for themselves. Everything else
in this repo is disposable; that file is a draft of something real.

## Stop conditions

- **Timebox: one weekend.** Still fighting it on day three? That is the finding.
  Write it up and stop
- A stage fails → stop, write up what you learned, bring it back
- You catch yourself picking a color, naming a design token, or building an
  abstraction layer → you have drifted; reread this file

## Not in scope

No skins, no sprites, no audio, no SQLite, no yt-dlp, no playlist, no library,
no persistence, no settings. Three gray rectangles with a 14px strip on top.
```
