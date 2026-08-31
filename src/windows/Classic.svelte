<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { applyTheme } from "../lib/theme";

  // Shared shell for the three classic 275px windows. They differ only in what
  // is inside them, and in v0.4b that becomes the sprite renderer — the drag,
  // focus and bonding wiring is identical for all three and belongs in one file.
  let { label, title, body }: { label: string; title: string; body: string } =
    $props();

  // Each window is its own document, so each applies the theme itself. Cheap:
  // a handful of custom properties on :root, from design/tokens.json.
  applyTheme();

  // Focus is a group property. Rust owns the answer, because only Rust knows
  // the bond graph — the window cannot work out on its own whether a sibling
  // being focused should light it up.
  let active = $state(true);

  $effect(() => {
    const stop = listen<boolean>("wm:active", (e) => {
      active = e.payload;
    });
    return () => {
      stop.then((off) => off());
    };
  });

  // One drag frame in flight at a time. Rust reads the live cursor position on
  // every call, so a skipped frame costs nothing — whereas letting the invokes
  // queue up would make the windows trail further and further behind the
  // pointer the longer the drag went on.
  let pending = false;

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    const el = e.currentTarget as HTMLElement;
    // Pointer capture, not a window-level listener: the cursor leaves the
    // 275px window almost immediately during a drag, and without capture the
    // move events stop arriving the moment it does.
    el.setPointerCapture(e.pointerId);
    invoke("wm_drag_start", { label });
  }

  function move(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    if (!el.hasPointerCapture(e.pointerId) || pending) return;
    pending = true;
    invoke("wm_drag_move").finally(() => {
      pending = false;
    });
  }

  function up(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    if (!el.hasPointerCapture(e.pointerId)) return;
    el.releasePointerCapture(e.pointerId);
    invoke("wm_drag_end");
  }

  // Belt and braces with the OS focus event: clicking a webview normally
  // activates its window, but the group has to raise even if it does not.
  function raise() {
    invoke("wm_focus", { label });
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="chrome" data-active={active} onpointerdown={raise}>
  <div
    class="titlebar"
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
  >
    {title}
  </div>
  <div class="body">{body}</div>
</div>
