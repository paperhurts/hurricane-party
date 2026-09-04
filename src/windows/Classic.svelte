<script lang="ts">
  import type { Snippet } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { applyTheme } from "../lib/theme";

  // Shared shell for the three classic 275px windows. They differ only in what
  // is inside them — the drag, seam and focus wiring is identical for all three
  // and belongs in one file. A window with real contents passes them as
  // children; one that has none yet shows the placeholder label.
  let {
    label,
    title,
    body = "",
    children,
  }: { label: string; title: string; body?: string; children?: Snippet } = $props();

  // Each window is its own document, so each applies the theme itself. Cheap:
  // a handful of custom properties on :root, from design/tokens.json.
  applyTheme();

  type Side = "top" | "right" | "bottom" | "left";
  // null = no bond on that edge, true = live splitter, false = bonded but the
  // seam is a move handle (D35).
  type Edges = Record<Side, boolean | null>;

  const SIDES: Side[] = ["top", "right", "bottom", "left"];

  // Focus and seams are both group properties, so Rust owns both answers — only
  // Rust holds the bond graph. A window cannot work out on its own whether a
  // sibling being focused should light it up, or whether its bottom edge is
  // somebody else's top edge.
  type WmState = { edges: Edges; active: boolean; shaded: boolean };

  let active = $state(true);
  let shaded = $state(false);
  let edges = $state<Edges>({ top: null, right: null, bottom: null, left: null });

  function absorb(s: WmState) {
    edges = s.edges;
    active = s.active;
    shaded = s.shaded;
  }

  $effect(() => {
    // One event carrying the whole picture. Seams, focus and shade all derive
    // from the same locked state in Rust, so splitting them into separate
    // messages would only let this window hold two of them from different
    // moments.
    const subs = [listen<WmState>("wm:state", (e) => absorb(e.payload))];
    // Subscribe first, then ask. The push events fire when something changes,
    // and the first one is emitted during Rust setup -- long before this bundle
    // has loaded. Without the pull the seams simply never appear, and every
    // click on one falls through to the title bar underneath and moves the
    // group instead of resizing it.
    invoke<WmState | null>("wm_hello", { label }).then((s) => {
      if (s) absorb(s);
    });
    return () => {
      for (const s of subs) s.then((off) => off());
    };
  });

  // One frame in flight at a time. Rust reads the live cursor on every call, so
  // a skipped frame costs nothing — whereas letting the invokes queue would make
  // the windows trail further behind the pointer the longer the gesture ran.
  let pending = false;
  // Which gesture the seam actually gave us. A seam with no resizable neighbour
  // degrades to a group move rather than offering a splitter that does nothing.
  let gesture: "splitter" | "move" | "none" = "none";

  function frame(fn: () => Promise<unknown>) {
    if (pending) return;
    pending = true;
    fn().finally(() => {
      pending = false;
    });
  }

  // ---- title bar: always a group move ----

  // Capture on <html>, never on the element that was clicked, and never until
  // the pointer has actually moved.
  //
  // Two separate lessons, both learned the hard way.
  //
  // Capture is required at all because the cursor leaves the 275px window
  // almost immediately during a drag, and without it the move events stop
  // arriving the moment it does. But capture dies with the element holding it,
  // and these elements are Svelte-rendered — a `wm:state` push arriving
  // mid-gesture replaces the very seam strip the pointer is captured to, and
  // the drag goes silent with no error anywhere. <html> is outside Svelte's
  // control and cannot be re-rendered out from under a gesture.
  //
  // Taking capture on pointerdown then breaks double-click, because a captured
  // pointer retargets the derived click and dblclick events to the capture
  // element — so the title bar never sees the double-click that toggles shade.
  // Waiting for the first move fixes both: a click that never moves takes no
  // capture at all, and dblclick behaves normally.
  function arm(e: PointerEvent, begin: () => void) {
    if (e.button !== 0) return;
    const root = document.documentElement;
    let started = false;

    const onMove = () => {
      if (!started) {
        started = true;
        root.setPointerCapture(e.pointerId);
        begin();
      }
      if (gesture === "splitter") frame(() => invoke("wm_splitter_move"));
      else if (gesture === "move") frame(() => invoke("wm_drag_move"));
    };
    const onUp = () => {
      root.removeEventListener("pointermove", onMove);
      root.removeEventListener("pointerup", onUp);
      root.removeEventListener("pointercancel", onUp);
      if (!started) return; // a plain click: leave click/dblclick alone
      if (gesture === "splitter") invoke("wm_splitter_end");
      else if (gesture === "move") invoke("wm_drag_end");
      gesture = "none";
    };

    root.addEventListener("pointermove", onMove);
    root.addEventListener("pointerup", onUp);
    root.addEventListener("pointercancel", onUp);
  }

  // ---- title bar: always a group move ----

  // Double-click, detected from pointerdown timing rather than from the DOM's
  // dblclick event.
  //
  // dblclick is a *derived* event, and it stops being generated here: raising
  // the group on the first click re-applies window ownership and z-order
  // through Win32, and WebView2 does not produce a dblclick across that. The
  // click events themselves arrive fine — both reach Rust — so the timing is
  // all that is actually needed, and reading it directly removes the
  // dependency on a synthesised event surviving a native window operation.
  //
  // Keyed per target so a click on the title bar followed by one on a seam is
  // never mistaken for a double-click on either.
  let lastTapAt = 0;
  let lastTapKey = "";

  function doubleTap(key: string): boolean {
    const now = Date.now();
    const hit = key === lastTapKey && now - lastTapAt < 400;
    // Reset on a hit so a triple-click is not read as two overlapping doubles.
    lastTapAt = hit ? 0 : now;
    lastTapKey = hit ? "" : key;
    return hit;
  }

  function titleDown(e: PointerEvent) {
    if (e.button === 0 && doubleTap("title")) {
      toggleShade();
      return;
    }
    arm(e, () => {
      gesture = "move";
      invoke("wm_drag_start", { label });
    });
  }

  // ---- seam: splitter, or a move handle where nothing can resize ----

  function seamDown(e: PointerEvent, side: Side) {
    if (edges[side] === null) return;
    if (e.button === 0 && doubleTap(`seam:${side}`)) {
      demagnetize(side);
      return;
    }
    arm(e, () => {
      // Provisionally a move, so the frames arriving before Rust answers are
      // not dropped. Rust decides which it really is: a seam whose neighbours
      // cannot resize degrades to a group move (D35).
      gesture = "move";
      invoke<typeof gesture>("wm_seam_down", { label, edge: side }).then((g) => {
        if (gesture !== "none") gesture = g;
      });
    });
  }

  // D60. The seam already owns dblclick for demagnetize, and its 4px strip
  // stacks above the title bar, so the hit target decides which one fires.
  function toggleShade() {
    invoke("wm_toggle_shade", { label });
  }

  function demagnetize(side: Side) {
    if (edges[side] === null) return;
    invoke("wm_demagnetize", { label, edge: side });
  }

  // Belt and braces with the OS focus event: clicking a webview normally
  // activates its window, but the group has to raise even if it does not.
  function raise() {
    invoke("wm_focus", { label });
  }

  function cursorFor(side: Side): string {
    if (edges[side] === null) return "default";
    // D35: the cursor tells the truth. A splitter cursor appears only where a
    // neighbour can actually resize; everywhere else this is a move handle.
    if (!edges[side]) return "move";
    return side === "top" || side === "bottom" ? "ns-resize" : "ew-resize";
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="chrome" data-active={active} data-shaded={shaded} onpointerdown={raise}>
  <div class="titlebar" onpointerdown={titleDown}>
    {title}
  </div>
  <div class="body">
    {#if children}
      {@render children()}
    {:else}
      <span class="placeholder">{body}</span>
    {/if}
  </div>

  {#each SIDES as side (side)}
    {#if edges[side] !== null}
      <!-- The grab band sits *inside* the window. Two bonded windows are
           flush, so there is no gap between them to put a handle in — each
           side contributes half the band. -->
      <div
        class="seam {side}"
        style:cursor={cursorFor(side)}
        onpointerdown={(e) => seamDown(e, side)}
      ></div>
    {/if}
  {/each}
</div>
