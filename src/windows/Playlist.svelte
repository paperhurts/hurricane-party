<script lang="ts">
  // The classic playlist window: the play queue, as rows. The queue itself is
  // owned by the library window, which knows which list is showing and
  // broadcasts it as `queue:set`; this window mirrors it, and every action
  // here goes back to the library so the audio/video branch stays in one
  // place. Main says what is playing over `player:now`.
  import { invoke } from "@tauri-apps/api/core";
  import { emit, emitTo, listen } from "@tauri-apps/api/event";
  import Classic from "./Classic.svelte";

  type Item = {
    id: number;
    title: string;
    uploader: string | null;
    duration_s: number | null;
    kind: string;
    position: number | null;
  };
  type Queue = { name: string; listId: number | null; items: Item[] };

  let queue = $state<Queue>({ name: "", listId: null, items: [] });
  let nowId = $state<number | null>(null);
  let selected = $state<number | null>(null);
  let rowsEl: HTMLDivElement;

  function clock(s: number | null): string {
    if (s == null) return "";
    const t = Math.floor(s);
    const h = Math.floor(t / 3600);
    const m = Math.floor((t % 3600) / 60);
    const sec = String(t % 60).padStart(2, "0");
    return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
  }

  let total = $derived(queue.items.reduce((a, t) => a + (t.duration_s ?? 0), 0));
  let title = $derived(queue.name ? `PLAYLIST — ${queue.name}` : "PLAYLIST");
  let selectedItem = $derived(queue.items.find((t) => t.id === selected) ?? null);
  // Only a real playlist has rows to remove; the library is not a list.
  let canRemove = $derived(queue.listId != null && selectedItem?.position != null);

  $effect(() => {
    const subs = [
      listen<Queue>("queue:set", (e) => {
        queue = e.payload;
      }),
      listen<{ id: number | null; playing: boolean }>("player:now", (e) => {
        nowId = e.payload.id;
      }),
    ];
    // Pull once: the library's first broadcast may have gone out before this
    // window had a listener (D67), and a push it missed is a push it never
    // gets.
    emit("queue:hello").catch(() => {});
    return () => {
      for (const s of subs) s.then((off) => off());
    };
  });

  // Keep the playing row in view as the queue advances.
  $effect(() => {
    void nowId;
    void queue;
    rowsEl?.querySelector<HTMLElement>(".row.now")?.scrollIntoView({ block: "nearest" });
  });

  function play(t: Item) {
    emitTo("library", "queue:play", t.id).catch(() => {});
  }

  function remove() {
    if (!canRemove || !selectedItem) return;
    emitTo("library", "queue:remove", selectedItem.position).catch(() => {});
  }

  // Double-click by pointerdown timing, not the DOM's dblclick: raising the
  // group on the first click reorders windows through Win32, and WebView2
  // does not produce a dblclick across that (see Classic.svelte).
  let lastTapAt = 0;
  let lastTapId = -1;

  function rowDown(e: PointerEvent, t: Item) {
    if (e.button !== 0) return;
    const now = Date.now();
    const dbl = t.id === lastTapId && now - lastTapAt < 400;
    lastTapAt = dbl ? 0 : now;
    lastTapId = dbl ? -1 : t.id;
    selected = t.id;
    rowsEl?.focus();
    if (dbl) play(t);
  }

  function onKey(e: KeyboardEvent) {
    const items = queue.items;
    if (items.length === 0) return;
    const i = items.findIndex((t) => t.id === selected);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const j = i < 0 ? 0 : Math.min(items.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)));
      selected = items[j].id;
      rowsEl?.querySelector<HTMLElement>(".row.sel")?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter" && selectedItem) {
      e.preventDefault();
      play(selectedItem);
    } else if (e.key === "Delete") {
      e.preventDefault();
      remove();
    }
  }
</script>

<Classic label="playlist" {title}>
  <div class="pl">
    <div
      class="rows"
      bind:this={rowsEl}
      role="listbox"
      aria-label="Play queue"
      tabindex="0"
      onkeydown={onKey}
    >
      {#each queue.items as t, i (t.id + ":" + (t.position ?? "l"))}
        <div
          class="row"
          class:now={t.id === nowId}
          class:sel={t.id === selected}
          role="option"
          aria-selected={t.id === selected}
          tabindex="-1"
          onpointerdown={(e) => rowDown(e, t)}
          title={t.title}
        >
          <span class="n">{String(i + 1).padStart(2, "0")}</span>
          <span class="t">{t.kind === "video" ? "▣ " : ""}{t.uploader ? `${t.uploader} — ` : ""}{t.title}</span>
          <span class="d">{clock(t.duration_s)}</span>
        </div>
      {:else}
        <div class="empty">Nothing here yet. Add tracks in the library.</div>
      {/each}
    </div>
    <div class="bar">
      <div class="btns">
        <button class="pb" onclick={() => invoke("show_library")} title="Open the library">ADD</button>
        <button class="pb rem" onclick={remove} disabled={!canRemove} title="Remove from this playlist">REM</button>
      </div>
      <div class="stat">
        <span>{queue.items.length} {queue.items.length === 1 ? "ITEM" : "ITEMS"}</span>
        <span class="tot">{clock(total)}</span>
      </div>
    </div>
  </div>
</Classic>

<style>
  .pl {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 3px 4px;
  }

  /* ---- rows ---- */
  .rows {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    background: var(--well);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 14%, transparent);
    outline: none;
  }
  .rows::-webkit-scrollbar {
    width: 5px;
  }
  .rows::-webkit-scrollbar-track {
    background: transparent;
  }
  .rows::-webkit-scrollbar-thumb {
    background: color-mix(in srgb, var(--arc) 35%, transparent);
  }
  .rows::-webkit-scrollbar-thumb:hover {
    background: var(--arc);
  }
  .row {
    height: 10px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 3px;
    font-size: 9px;
    line-height: 1;
    white-space: nowrap;
    color: color-mix(in srgb, var(--filament) 72%, transparent);
  }
  .row.sel {
    background: color-mix(in srgb, var(--arc) 13%, transparent);
    color: var(--filament);
  }
  /* Now-playing is `strike` (theme.md), with a static halo: not the viz. */
  .row.now {
    color: var(--strike);
    text-shadow: 0 0 6px color-mix(in srgb, var(--strike) 55%, transparent);
  }
  .n {
    flex: 0 0 auto;
    color: color-mix(in srgb, var(--filament) 35%, transparent);
  }
  .row.now .n {
    color: inherit;
  }
  .t {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .d {
    flex: 0 0 auto;
    color: color-mix(in srgb, var(--filament) 45%, transparent);
  }
  .row.now .d {
    color: inherit;
  }
  .empty {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 8px;
    text-align: center;
    font-size: 9px;
    color: color-mix(in srgb, var(--filament) 45%, transparent);
  }

  /* ---- bottom bar ---- */
  .bar {
    flex: 0 0 13px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 6px;
    letter-spacing: 0.1em;
  }
  .btns {
    display: flex;
    gap: 2px;
  }
  .pb {
    height: 13px;
    padding: 0 5px;
    border: 0;
    display: grid;
    place-items: center;
    font: inherit;
    font-size: 6px;
    letter-spacing: 0.1em;
    line-height: 1;
    color: color-mix(in srgb, var(--filament) 70%, transparent);
    background: color-mix(in srgb, var(--void) 70%, var(--well));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 22%, transparent);
    cursor: pointer;
  }
  .pb:hover:not(:disabled) {
    color: var(--arc);
    background: color-mix(in srgb, var(--void) 70%, var(--well));
    box-shadow: inset 0 0 0 1px var(--arc);
  }
  .pb.rem:hover:not(:disabled) {
    color: var(--strike);
    box-shadow: inset 0 0 0 1px var(--strike);
  }
  .pb:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .stat {
    display: flex;
    gap: 6px;
    color: color-mix(in srgb, var(--filament) 40%, transparent);
  }
  .tot {
    color: var(--arc);
  }
</style>
