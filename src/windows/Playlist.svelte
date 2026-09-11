<script lang="ts">
  // The classic playlist window: the play queue, as rows. The queue itself is
  // owned by the library window, which knows which list is showing and
  // broadcasts it as `queue:set`; this window mirrors it, and every action
  // here goes back to the library so the audio/video branch stays in one
  // place. Main says what is playing over `player:now`.
  import { invoke } from "@tauri-apps/api/core";
  import { emit, emitTo, listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
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

  // The play order's switches as the library holds them (#115). The library
  // owns the order (D74); these buttons only ask it to change, and show what
  // it says back.
  type Repeat = "off" | "one" | "all";
  let shuffle = $state(false);
  let repeat = $state<Repeat>("off");

  // A standing Play starts on the row picked here (#116, D97), so the library
  // hears of every pick: every press and every arrow key, not only a change
  // of selection. A click on the row already selected, after Stop, is still
  // the person pointing at it, and a change-only report never said so.
  function pick(id: number | null) {
    selected = id;
    emitTo("library", "queue:select", id).catch(() => {});
  }
  let rowsEl: HTMLDivElement;

  // The bottom bar doubles as a one-line URL field, and as a one-line notice
  // for a few seconds after something happened.
  let urlMode = $state(false);
  let url = $state("");
  let urlEl = $state<HTMLInputElement | undefined>();
  let flash = $state<string | null>(null);
  let flashTimer = 0;

  function say(msg: string) {
    flash = msg;
    clearTimeout(flashTimer);
    flashTimer = window.setTimeout(() => (flash = null), 4000);
  }

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
  // The shade's one line: the playing row, or the list's name when nothing is.
  let now = $derived(queue.items.find((t) => t.id === nowId) ?? null);
  let nowLine = $derived(
    now ? (now.uploader ? `${now.uploader} — ${now.title}` : now.title) : queue.name || "Library",
  );
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
      listen<{ shuffle: boolean; repeat: Repeat }>("play:mode", (e) => {
        shuffle = e.payload.shuffle;
        repeat = e.payload.repeat;
      }),
    ];
    // Pull once: the library's first broadcast may have gone out before this
    // window had a listener (D67), and a push it missed is a push it never
    // gets. Both the list and the switches.
    emit("queue:hello").catch(() => {});
    emit("play:hello").catch(() => {});
    // A reloaded window has nothing selected; a pick the library still held
    // from before would start a row nobody can see is chosen.
    emitTo("library", "queue:select", null).catch(() => {});
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

  // ADD: a folder, scanned into the library; and when a real playlist is
  // showing, the folder's tracks are appended to it, which is what ADD means
  // on a playlist window. Every track the scan touched, not only the new
  // ones: a folder the library already knows is still a folder the user
  // wants in this list. Tracks already in the list are left alone. The
  // library hears library-changed and re-broadcasts the queue.
  async function addFolder() {
    const picked = await openDialog({ directory: true, multiple: false, title: "Add a music folder" });
    if (typeof picked !== "string") return;
    try {
      const r = await invoke<{ found: number; added: number; updated: number; ids: number[] }>(
        "add_local_folder",
        { path: picked },
      );
      let put = 0;
      if (queue.listId != null) {
        const have = new Set(queue.items.map((t) => t.id));
        for (const mediaId of r.ids) {
          if (have.has(mediaId)) continue;
          await invoke("add_to_playlist", { playlistId: queue.listId, mediaId });
          put++;
        }
        if (put) await emit("library-changed");
      }
      say(
        r.found === 0
          ? "No audio or video files in that folder."
          : queue.listId != null
            ? `${put} put in ${queue.name}` +
              (r.ids.length - put > 0 ? `, ${r.ids.length - put} already there` : "") +
              (r.added ? `, ${r.added} new to the library` : "")
            : `${r.added} added to the library, ${r.updated} already known`,
      );
    } catch (e) {
      say(String(e));
    }
  }

  // URL: queued for download into the library. Where it lands in a playlist
  // is decided when it finishes, and that is not wired yet.
  function openUrl() {
    urlMode = true;
    url = "";
    setTimeout(() => urlEl?.focus(), 0);
  }

  async function queueUrl() {
    const u = url.trim();
    urlMode = false;
    if (!u) return;
    try {
      await invoke("enqueue_url", { url: u, wantVideo: false });
      say("Queued. It shows up in the library when the download finishes.");
    } catch (e) {
      say(String(e));
    }
  }

  function urlKey(e: KeyboardEvent) {
    if (e.key === "Enter") queueUrl();
    else if (e.key === "Escape") urlMode = false;
  }

  // Double-click by pointerdown timing, not the DOM's dblclick: raising the
  // group on the first click reorders windows through Win32, and WebView2
  // does not produce a dblclick across that (see Classic.svelte).
  let lastTapAt = 0;
  let lastTapId = -1;

  // Drag-to-reorder, on a real playlist: the lifted row and the insertion
  // index (0..n) it would land at. A press that moves a few pixels becomes
  // a drag; one that does not stays a click, so double-click still works.
  let dragId = $state<number | null>(null);
  let dropAt = $state<number | null>(null);

  function rowDown(e: PointerEvent, t: Item) {
    if (e.button !== 0) return;
    const now = Date.now();
    const dbl = t.id === lastTapId && now - lastTapAt < 400;
    lastTapAt = dbl ? 0 : now;
    lastTapId = dbl ? -1 : t.id;
    pick(t.id);
    rowsEl?.focus();
    if (dbl) {
      play(t);
      return;
    }
    if (queue.listId == null || t.position == null) return;

    const row = e.currentTarget as HTMLElement;
    const startY = e.clientY;
    const i = queue.items.indexOf(t);
    let dragging = false;
    // Capture at the press, not at the threshold: the rows are ten pixels
    // tall, so the pointer has usually left the pressed row before it has
    // moved the four pixels that make this a drag, and an uncaptured row
    // never sees the move that would have started it. Capturing here does
    // not break double-click, which is read from press timing above.
    row.setPointerCapture(e.pointerId);

    const onMove = (ev: PointerEvent) => {
      if (!dragging) {
        if (Math.abs(ev.clientY - startY) < 4) return;
        dragging = true;
        dragId = t.id;
      }
      const rows = Array.from(rowsEl.querySelectorAll<HTMLElement>(".row[data-idx]"));
      let at = rows.length;
      for (const r of rows) {
        const b = r.getBoundingClientRect();
        if (ev.clientY < b.top + b.height / 2) {
          at = Number(r.dataset.idx);
          break;
        }
      }
      dropAt = at;
    };
    const onUp = () => {
      row.removeEventListener("pointermove", onMove);
      row.removeEventListener("pointerup", onUp);
      row.removeEventListener("pointercancel", onUp);
      if (!dragging) return;
      const at = dropAt ?? i;
      dragId = null;
      dropAt = null;
      // The row leaves first, so a target below it shifts up by one.
      const dest = at > i ? at - 1 : at;
      if (dest !== i) emitTo("library", "queue:move", { from: t.position, to: dest }).catch(() => {});
    };
    row.addEventListener("pointermove", onMove);
    row.addEventListener("pointerup", onUp);
    row.addEventListener("pointercancel", onUp);
  }

  // ---- what the skin draws (#3, D99) ----
  //
  // The manifest says where the rows, the six buttons, the count and the
  // link field go, and how the buttons look; this window says what they
  // read, and draws the rows and the count itself.
  let binds = $derived({
    shuffle: shuffle ? "on" : "off",
    repeatOn: repeat === "off" ? "off" : "on",
    // The same three words the bar always said, in its three-letter voice.
    repeatLabel: repeat === "one" ? "1x" : repeat === "all" ? "ALL" : "REP",
    plCanRemove: canRemove ? "yes" : "no",
  });

  function action(name: string) {
    if (name === "add") addFolder();
    else if (name === "addUrl") openUrl();
    else if (name === "remove") remove();
    else if (name === "library") invoke("show_library");
    // The library owns the order (D74); these only ask it to change, and the
    // buttons light from what it says back over `play:mode`.
    else if (name === "shuffle") emitTo("library", "play:shuffle").catch(() => {});
    else if (name === "repeat") emitTo("library", "play:repeat").catch(() => {});
  }

  function onKey(e: KeyboardEvent) {
    const items = queue.items;
    if (items.length === 0) return;
    const i = items.findIndex((t) => t.id === selected);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const j = i < 0 ? 0 : Math.min(items.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)));
      pick(items[j].id);
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

<!-- The playlist's shade: what is playing, and how much list there is (D79). -->
{#snippet shade()}
  <div class="shade">
    <span class="stag">PL</span>
    <span class="stext" class:now={!!now}>{nowLine}</span>
    <span class="stag">{queue.items.length} · {clock(total)}</span>
  </div>
{/snippet}

<!-- The rows, in the box the skin gave the list, in its face and colours
     (the --list-* properties). The drag, the keys and the double-click are
     this window's, as they always were. -->
{#snippet rows()}
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
        class:lifted={dragId === t.id}
        class:drop-before={dragId != null && dropAt === i}
        class:drop-after={dragId != null && dropAt === queue.items.length && i === queue.items.length - 1}
        data-idx={i}
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
      <div class="empty">Nothing here yet. ADD a folder, paste a URL, or pick tracks in the library.</div>
    {/each}
  </div>
{/snippet}

<!-- The count and the running time, or a notice for a few seconds after
     something happened. Right-aligned in the skin's box. -->
{#snippet status()}
  {#if flash}
    <div class="flash" title={flash}>{flash}</div>
  {:else}
    <div class="stat">
      <span>{queue.items.length} {queue.items.length === 1 ? "ITEM" : "ITEMS"}</span>
      <span class="tot">{clock(total)}</span>
    </div>
  {/if}
{/snippet}

<!-- The link field, over the whole bar while it is open. -->
{#snippet urlField()}
  {#if urlMode}
    <input
      class="url"
      bind:this={urlEl}
      bind:value={url}
      placeholder="Paste a link, Enter to queue, Esc to cancel"
      onkeydown={urlKey}
      onblur={() => (urlMode = false)}
    />
  {/if}
{/snippet}

<Classic
  label="playlist"
  {title}
  resizable
  {shade}
  {binds}
  slots={{ list: rows, listStatus: status, urlField }}
  onaction={action}
/>

<style>
  /* ---- rows ----
   *
   * The well and its edge are the skin's (`listWell`, `listFrame`); so are
   * the row height, the face and the three colours, which arrive as
   * --list-fg (a row), --list-hi (the same token at full strength), --list-sel
   * and --list-now. Everything below is how a row is laid out, not how it
   * looks. */
  .rows {
    position: absolute;
    inset: 0;
    overflow-y: auto;
    overflow-x: hidden;
    outline: none;
  }
  .rows::-webkit-scrollbar {
    width: 5px;
  }
  .rows::-webkit-scrollbar-track {
    background: transparent;
  }
  .rows::-webkit-scrollbar-thumb {
    background: color-mix(in srgb, var(--list-sel) 35%, transparent);
  }
  .rows::-webkit-scrollbar-thumb:hover {
    background: var(--list-sel);
  }
  .row {
    height: var(--list-row);
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 3px;
    white-space: nowrap;
    touch-action: none;
  }
  .row.sel {
    background: color-mix(in srgb, var(--list-sel) 13%, transparent);
    color: var(--list-hi);
  }
  .row.lifted {
    opacity: 0.4;
  }
  .row.drop-before {
    box-shadow: inset 0 1px 0 var(--list-sel);
  }
  .row.drop-after {
    box-shadow: inset 0 -1px 0 var(--list-sel);
  }
  /* The playing row, with a static halo: not the viz (theme.md). */
  .row.now {
    color: var(--list-now);
    text-shadow: 0 0 6px color-mix(in srgb, var(--list-now) 55%, transparent);
  }
  .n {
    flex: 0 0 auto;
    color: color-mix(in srgb, var(--list-hi) 35%, transparent);
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
    color: color-mix(in srgb, var(--list-hi) 45%, transparent);
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
    white-space: normal;
    color: color-mix(in srgb, var(--list-hi) 45%, transparent);
  }

  /* ---- the count, the notice, the link field ---- */
  .stat {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    white-space: nowrap;
    font-size: 6px;
    letter-spacing: 0.1em;
    color: color-mix(in srgb, var(--filament) 40%, transparent);
  }
  .tot {
    color: var(--arc);
  }
  .flash {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    overflow: hidden;
    white-space: nowrap;
    font-size: 7px;
    letter-spacing: 0.02em;
    color: var(--arc);
    /* The whole notice is in its tooltip; the slot lets the pointer through
       and this takes it back. */
    pointer-events: auto;
  }
  .url {
    /* Block, or it sits on a text baseline a pixel and a half low. */
    display: block;
    box-sizing: border-box;
    width: 100%;
    height: 100%;
    padding: 0 4px;
    font: inherit;
    font-size: 8px;
    letter-spacing: 0;
    color: var(--filament);
    background: var(--well);
    border: 0;
    box-shadow: inset 0 0 0 1px var(--arc);
    outline: none;
    pointer-events: auto;
  }
  .url::placeholder {
    color: color-mix(in srgb, var(--filament) 40%, transparent);
  }
</style>
