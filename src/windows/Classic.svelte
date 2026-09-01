<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { applyTheme } from "../lib/theme";

  // Shared shell for the three classic 275px windows. They differ only in what
  // is inside them, and in v0.4b that becomes the sprite renderer — the drag,
  // seam and focus wiring is identical for all three and belongs in one file.
  let { label, title, body }: { label: string; title: string; body: string } =
    $props();

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

  function titleDown(e: PointerEvent) {
    if (e.button !== 0) return;
    // Pointer capture, not a window-level listener: the cursor leaves the 275px
    // window almost immediately during a drag, and without capture the move
    // events stop arriving the moment it does.
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    gesture = "move";
    invoke("wm_drag_start", { label });
  }

  // ---- seam: splitter, or a move handle where nothing can resize ----

  async function seamDown(e: PointerEvent, side: Side) {
    if (e.button !== 0 || edges[side] === null) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    gesture = (await invoke("wm_seam_down", { label, edge: side })) as typeof gesture;
  }

  function move(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    if (!el.hasPointerCapture(e.pointerId)) return;
    if (gesture === "splitter") frame(() => invoke("wm_splitter_move"));
    else if (gesture === "move") frame(() => invoke("wm_drag_move"));
  }

  function up(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    if (!el.hasPointerCapture(e.pointerId)) return;
    el.releasePointerCapture(e.pointerId);
    if (gesture === "splitter") invoke("wm_splitter_end");
    else if (gesture === "move") invoke("wm_drag_end");
    gesture = "none";
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
  <div
    class="titlebar"
    onpointerdown={titleDown}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
    ondblclick={toggleShade}
  >
    {title}
  </div>
  <div class="body">{body}</div>

  {#each SIDES as side (side)}
    {#if edges[side] !== null}
      <!-- The grab band sits *inside* the window. Two bonded windows are
           flush, so there is no gap between them to put a handle in — each
           side contributes half the band. -->
      <div
        class="seam {side}"
        style:cursor={cursorFor(side)}
        onpointerdown={(e) => seamDown(e, side)}
        onpointermove={move}
        onpointerup={up}
        onpointercancel={up}
        ondblclick={() => demagnetize(side)}
      ></div>
    {/if}
  {/each}
</div>
