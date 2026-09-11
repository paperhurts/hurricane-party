<script lang="ts">
  import type { Snippet } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { applyTheme } from "../lib/theme";
  import { elementsOf, type Element } from "../lib/skin";
  import { loadSkin, type LoadedSkin } from "../lib/skinsheet";
  import { EYEWALL, eyewallFile, windowNameOf } from "../lib/skins";
  import Sprite from "./Sprite.svelte";

  // Shared shell for the three classic 275px windows. They differ only in what
  // is inside them — the drag, seam and focus wiring is identical for all three
  // and belongs in one file. A window with real contents passes them as
  // children; one that has none yet shows the placeholder label.
  let {
    label,
    title,
    body = "",
    resizable = false,
    children,
    shade,
    binds = {},
    slots = {},
    onaction,
    onslide,
  }: {
    label: string;
    title: string;
    body?: string;
    /** Shows the corner grip. Only the playlist resizes (D30). */
    resizable?: boolean;
    children?: Snippet;
    /**
     * What the 275 x 14 strip shows while shaded (D60, D79). Rendered inside
     * the title bar, so the strip stays the one move handle and the
     * double-click that expands it; a window without one shows its name.
     */
    shade?: Snippet;
    /**
     * What this window is showing, by binding name (`skin.ts` BINDS): the
     * skin says where the clock goes and how it is drawn, the window says
     * what time it is.
     */
    binds?: Record<string, unknown>;
    /**
     * Content to place inside a named element's box. The skin positions it;
     * the window fills it. `vis` is how the analyser lands in whatever
     * rectangle the skin gave the visualizer.
     */
    slots?: Record<string, Snippet>;
    /** A button the shell does not own itself: transport, mostly. */
    onaction?: (action: string) => void;
    /** A slider moved, by binding name, 0..1 along its length. */
    onslide?: (bind: string, frac: number) => void;
  } = $props();

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
  type WmState = { edges: Edges; active: boolean; shaded: boolean; double: boolean };

  let active = $state(true);
  let shaded = $state(false);
  // 2x chrome (#47). Rust re-zooms the webview and re-lays the windows; this
  // only drives the toggle's label.
  let double = $state(false);
  let edges = $state<Edges>({ top: null, right: null, bottom: null, left: null });

  // The skin (#3, D73). Every window loads it itself, like the theme: three
  // documents, a few small sheets each. Until it arrives the window is its
  // `void` ground and nothing else; a skin that fails validation is refused
  // whole and logged, never half-drawn (skin-manifest.md).
  let skin = $state<LoadedSkin | null>(null);
  let win = $derived(windowNameOf(label));
  function reloadSkin() {
    loadSkin(EYEWALL, eyewallFile, window.devicePixelRatio).then(
      (s) => (skin = s),
      (e) => console.error(e),
    );
  }
  reloadSkin();
  // The sheet is chosen for the screen's pixel ratio, and that changes under
  // the window: the 2x toggle re-zooms the webview (D76) and a drag across a
  // DPI boundary re-scales it. A media query on the current ratio fires once
  // when it stops being true; re-arm it on the new one and load again.
  $effect(() => {
    let mq: MediaQueryList | null = null;
    const onChange = () => {
      reloadSkin();
      arm();
    };
    const arm = () => {
      mq?.removeEventListener("change", onChange);
      mq = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      mq.addEventListener("change", onChange);
    };
    arm();
    return () => mq?.removeEventListener("change", onChange);
  });

  // The window's logical size, for the playlist's right-anchored and
  // stretched elements (D30). CSS px are logical px whatever the zoom (D76).
  let current = $state<[number, number]>([window.innerWidth, window.innerHeight]);
  $effect(() => {
    const onResize = () => (current = [window.innerWidth, window.innerHeight]);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  });

  // The glow toggle (#108, D100): a person's say over the halo a `glow:
  // renderer` skin gets. Saved in settings; every classic window asks at
  // mount and hears the change, so all three turn over together.
  let glowOn = $state(true);
  $effect(() => {
    invoke<boolean>("get_glow").then(
      (on) => (glowOn = on),
      () => {},
    );
    const sub = listen<boolean>("chrome:glow", (e) => (glowOn = e.payload), {
      target: { kind: "WebviewWindow", label },
    });
    return () => {
      sub.then((off) => off());
    };
  });

  let set = $derived(skin ? elementsOf(skin.skin, win, shaded) : null);
  // The interior starts under the title bar, and the title bar's height is
  // the skin's to say.
  let bodyTop = $derived.by(() => {
    const bar = set?.elements.find((e) => e.name === "titlebar");
    return bar && bar.type === "image" ? bar.rect[1] + bar.rect[3] : 14;
  });

  // A button's `action` is a name from the app's list (skin.ts ACTIONS). The
  // shell owns the four that are about the window itself; everything else is
  // the window's, and reaches it through `onaction`.
  function act(action: string | null) {
    switch (action) {
      case null:
        return;
      case "minimize":
        // #86, D86: minimise is Main's gesture and takes the whole group; a
        // satellite has no taskbar button to come back from (D59).
        invoke("wm_minimize");
        return;
      case "shade":
        toggleShade();
        return;
      case "zoom":
        toggleDouble();
        return;
      case "close":
        // D63: closing Main saves the layout and exits, and it is the app's
        // only way out. Rust closes the window rather than exiting here, so
        // this takes the one exit path the title bar's × always took.
        invoke("wm_close", { label });
        return;
      default:
        onaction?.(action);
    }
  }

  // A toggle shows its `on` art for the state it names: the 2x button while
  // the chrome is doubled, the shade button while the window is the strip
  // (its arrow points the way the window will go).
  function toggleOn(el: Element): boolean {
    if (el.type !== "toggle") return false;
    if (el.action === "zoom") return double;
    if (el.action === "shade") return shaded;
    return false;
  }

  // The window's own bindings, plus the one the shell always knows. A window
  // never has to pass its own name in.
  let allBinds = $derived({ windowTitle: title, ...binds });

  // The discharge (#9, theme.md): a bond that just broke blooms for ~120 ms
  // and is gone. Detected here, from the edge going from bonded to null in a
  // state push, so both windows of the broken bond flash their own side.
  // The seam element itself is gone the moment the edge is null, so the
  // bloom is its own element, kept just long enough to finish.
  let bloom = $state<Record<Side, boolean>>({ top: false, right: false, bottom: false, left: false });
  const bloomTimers: Partial<Record<Side, number>> = {};

  function absorb(s: WmState) {
    for (const side of SIDES) {
      if (edges[side] !== null && s.edges[side] === null) {
        bloom[side] = true;
        clearTimeout(bloomTimers[side]);
        bloomTimers[side] = window.setTimeout(() => (bloom[side] = false), 240);
      }
    }
    edges = s.edges;
    active = s.active;
    shaded = s.shaded;
    double = s.double;
  }

  // Which seam is being dragged as a splitter, so it can render as such.
  let dragSide = $state<Side | null>(null);

  function toggleDouble() {
    invoke("wm_set_double", { on: !double });
  }

  $effect(() => {
    // One event carrying the whole picture. Seams, focus and shade all derive
    // from the same locked state in Rust, so splitting them into separate
    // messages would only let this window hold two of them from different
    // moments.
    // Targeted at THIS window. A listener with no target receives every
    // emit, including the pushes Rust aims at the other two windows, and the
    // last of the three to arrive won: Main would end up wearing the
    // playlist's edges, with a seam on top and none on the bottom, after any
    // click. A targeted listener still receives global emits.
    const subs = [
      listen<WmState>("wm:state", (e) => absorb(e.payload), {
        target: { kind: "WebviewWindow", label },
      }),
    ];
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
  let gesture: "splitter" | "move" | "resize" | "none" = "none";

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
      else if (gesture === "resize") frame(() => invoke("wm_resize_move"));
    };
    const onUp = () => {
      root.removeEventListener("pointermove", onMove);
      root.removeEventListener("pointerup", onUp);
      root.removeEventListener("pointercancel", onUp);
      if (!started) return; // a plain click: leave click/dblclick alone
      if (gesture === "splitter") invoke("wm_splitter_end");
      else if (gesture === "move") invoke("wm_drag_end");
      else if (gesture === "resize") invoke("wm_resize_end");
      gesture = "none";
      dragSide = null;
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
      // Spelled out: after the assignment above TypeScript narrows `gesture`
      // to "move", and `typeof gesture` would carry that narrowing here.
      invoke<"splitter" | "move" | "none">("wm_seam_down", { label, edge: side }).then((g) => {
        if (gesture !== "none") {
          gesture = g;
          if (g === "splitter") dragSide = side;
        }
      });
    });
  }

  // ---- corner grip: resize the free edges on the D30 grid ----

  // Which edges the grip can move. A bonded edge belongs to its seam.
  let gripAxes = $derived(
    !resizable || shaded
      ? "none"
      : edges.right === null && edges.bottom === null
        ? "both"
        : edges.right === null
          ? "w"
          : edges.bottom === null
            ? "h"
            : "none",
  );
  let gripCursor = $derived(
    gripAxes === "both" ? "nwse-resize" : gripAxes === "w" ? "ew-resize" : "ns-resize",
  );

  function gripDown(e: PointerEvent) {
    if (gripAxes === "none") return;
    // Stop it here: the chrome underneath would otherwise start a raise, and
    // the seam bands share this corner.
    e.stopPropagation();
    arm(e, () => {
      // Provisionally a resize; Rust says no if there is nothing to resize.
      gesture = "resize";
      invoke<boolean>("wm_resize_start", { label }).then((ok) => {
        if (!ok && gesture === "resize") gesture = "none";
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
<div
  class="chrome"
  data-active={active}
  data-shaded={shaded}
  data-glow={glowOn ? "on" : "off"}
  onpointerdown={raise}
>
  {#if skin && set}
    <!-- The chrome, element by element in the manifest's order, which is the
         z-order: the frame first. The title bar is the one move handle (D35)
         and keeps the double-tap that toggles shade (D60); a button on it
         stops the pointerdown, so a click there is neither. The windowshade
         snippet (D79) renders where the title would, so the strip stays the
         move handle around it. -->
    {#each set.elements as el (el.name)}
      {@const common = { el, skin, base: set.size, current, binds: allBinds, glowing: glowOn }}
      {#if el.type === "image" && el.role === "drag"}
        <Sprite {...common} onpointerdown={titleDown} />
      {:else if el.type === "button" || el.type === "toggle"}
        <Sprite {...common} on={toggleOn(el)} onclick={() => act(el.action)} />
      {:else if el.type === "text" && el.bind === "windowTitle" && shaded && shade}
        <Sprite {...common} slot={shade} />
      {:else if el.type === "slider"}
        <Sprite {...common} onslide={(f) => el.bind && onslide?.(el.bind, f)} />
      {:else}
        <Sprite {...common} slot={slots[el.name]} />
      {/if}
    {/each}
  {/if}
  {#if children || body}
    <div class="body" style:top="{bodyTop}px">
      {#if children}
        {@render children()}
      {:else}
        <span class="placeholder">{body}</span>
      {/if}
    </div>
  {/if}

  {#each SIDES as side (side)}
    {#if edges[side] !== null}
      <!-- The grab band sits *inside* the window. Two bonded windows are
           flush, so there is no gap between them to put a handle in — each
           side contributes half the band. -->
      <div
        class="seam {side}"
        class:live={edges[side] === true}
        class:dragging={dragSide === side}
        style:cursor={cursorFor(side)}
        onpointerdown={(e) => seamDown(e, side)}
      ></div>
    {/if}
    {#if bloom[side]}
      <div class="discharge {side}"></div>
    {/if}
  {/each}

  {#if gripAxes !== "none"}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="grip" style:cursor={gripCursor} onpointerdown={gripDown} title="Resize"></div>
  {/if}
</div>
