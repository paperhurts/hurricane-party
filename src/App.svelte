<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { emit, emitTo, listen } from "@tauri-apps/api/event";
  import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
  import { applyTheme } from "./lib/theme";
  // The library's empty state (#62): the surfer, boombox on his shoulder,
  // riding the warning flag. The art is the one place a literal colour is
  // allowed; everything around him is tokens.
  import surfer from "./assets/capybara-surfing.png";

  type Job = {
    id: number;
    url: string;
    status: "queued" | "running" | "done" | "failed" | "paused";
    stage: "probe" | "download" | "extract" | "verify";
    title: string | null;
    progress: number;
    bytes_done: number;
    bytes_total: number | null;
    error: string | null;
    attempts: number;
  };

  type MediaRow = {
    id: number;
    title: string;
    uploader: string | null;
    duration_s: number | null;
    filesize: number | null;
    kind: string;
    path: string;
    position: number | null;
  };

  type Root = { id: number; label: string; path: string; count: number; present: boolean };

  type Playlist = { id: number; name: string; count: number };

  // What Rust says after a row is removed: the file is still at `path`, and
  // the row no longer exists to say so (#78).
  type Removed = { id: number; title: string; path: string };

  let url = $state("");
  let error = $state<string | null>(null);
  let jobs = $state<Job[]>([]);
  let tracks = $state<MediaRow[]>([]);
  let playlists = $state<Playlist[]>([]);
  let selectedList = $state<number | null>(null);
  let listItems = $state<MediaRow[]>([]);
  // Two different things, kept apart on purpose. `current` is this window's
  // cursor: the row the user last played, audio or video, and the one next
  // and prev step from. `nowId` is which audio track Main holds and whether
  // it is sounding, for the row's ‖ glyph. Merging them is how skipping onto
  // a video got stuck: Main paused, reported the MP3, and the cursor snapped
  // back to it.
  let current = $state<MediaRow | null>(null);
  let nowId = $state<number | null>(null);
  let isPlaying = $state(false);
  let libraryPath = $state("");
  let concurrency = $state(2);
  let wantVideo = $state(false);
  let roots = $state<Root[]>([]);
  let scanning = $state(false);
  let notice = $state<string | null>(null);
  // Drag-to-reorder within a playlist: the lifted row, and the insertion
  // index in `shown` (0..n) it would land at.
  let dragId = $state<number | null>(null);
  let dropAt = $state<number | null>(null);
  let rowsEl: HTMLUListElement;
  // Which row's "+" menu is open, by track id. One at a time.
  let addMenuFor = $state<number | null>(null);
  // The offers that ride on a notice (#78). Deleting the file is a second,
  // separate step after a removal, never part of it; pruning follows a rescan
  // that found rows whose files are gone. Each is cleared with the notice.
  let pendingDelete = $state<Removed[]>([]);
  let pendingPrune = $state<{ rootId: number; count: number } | null>(null);
  // The checked rows (D84). Removal from the library is a selection and a
  // button, never a one-click glyph: a × beside every row was one slip away
  // from a row vanishing, and the playlist's × sets the expectation that a ×
  // is small.
  let selected = $state<number[]>([]);
  // A track Main could not open. It says so in its own strip; this is the
  // same message where the row is, with the way out beside it.
  let missing = $state<{ id: number; title: string } | null>(null);

  let active = $derived(jobs.filter((j) => j.status === "running" || j.status === "queued"));
  let shown = $derived(selectedList == null ? tracks : listItems);

  const mb = (n: number | null) => (n == null ? "—" : (n / 1_048_576).toFixed(1) + " MB");

  function duration(s: number | null) {
    if (s == null) return "";
    return `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
  }

  async function refreshJobs() {
    try {
      jobs = await invoke<Job[]>("list_jobs");
    } catch (e) {
      error = String(e);
    }
  }
  async function refreshLibrary() {
    tracks = await invoke<MediaRow[]>("list_tracks");
    // The row the message was about may have been removed from another
    // window (Main's strip, the video's); a message about a row that is not
    // there any more is noise.
    if (missing && !tracks.some((t) => t.id === missing!.id)) missing = null;
    // A check on a row that has gone (removed elsewhere, pruned) is dropped;
    // the others keep their check across the refresh.
    if (selected.length) selected = selected.filter((id) => tracks.some((t) => t.id === id));
    playlists = await invoke<Playlist[]>("list_playlists");
    roots = await invoke<Root[]>("list_roots");
    if (selectedList != null) await openList(selectedList);
  }

  onMount(() => {
    applyTheme("eyewall");
    refreshJobs();
    refreshLibrary();
    invoke<string>("library_path").then((p) => (libraryPath = p));
    invoke<number>("get_concurrency").then((n) => (concurrency = n));

    const subs = [
      listen("jobs-changed", refreshJobs),
      listen("library-changed", refreshLibrary),
      // Playback lives in the Main window (D5); this window is the remote.
      // Main asks for the next or previous track because the play order —
      // which list is showing — is known only here.
      listen<number>("player:step", (e) => step(e.payload)),
      // ...and says what it is playing, so the row can light up and its
      // button can show pause.
      listen<{ id: number | null; playing: boolean }>("player:now", (e) => {
        nowId = e.payload.id;
        isPlaying = e.payload.playing;
      }),
      // ...and when it could not open the file (#43), so the row's window can
      // offer to remove the row (#78).
      listen<{ id: number; title: string }>("player:missing", (e) => {
        missing = e.payload;
      }),
      // The classic playlist window mirrors the list showing here. It asks
      // once on mount, in case the first broadcast went out before it had a
      // listener, and sends its clicks back here so the audio/video branch
      // stays in one place.
      listen("queue:hello", announceQueue),
      listen<number>("queue:play", (e) => {
        const t = shown.find((x) => x.id === e.payload);
        if (t) play(t);
      }),
      listen<number>("queue:remove", (e) => removeAt(e.payload)),
      listen<{ from: number; to: number }>("queue:move", (e) => move(e.payload.from, e.payload.to)),
    ];
    // The DB is the source of truth for progress, and it's written throttled
    // to ~4Hz. Polling it while work is in flight beats trying to reconcile a
    // firehose of events against rows that may have been resumed from a
    // previous run.
    const tick = setInterval(() => {
      if (active.length) refreshJobs();
    }, 400);

    return () => {
      clearInterval(tick);
      subs.forEach((s) => s.then((f) => f()));
    };
  });

  async function add() {
    const u = url.trim();
    if (!u) return;
    error = null;
    try {
      await invoke<number>("enqueue_url", { url: u, wantVideo });
      url = "";
      refreshJobs();
    } catch (e) {
      error = String(e);
    }
  }

  /** Broadcast the play queue: whatever list is showing, as the playlist window sees it. */
  function announceQueue() {
    const name =
      selectedList == null ? "Library" : (playlists.find((p) => p.id === selectedList)?.name ?? "Playlist");
    const items = shown.map((t) => ({
      id: t.id,
      title: t.title,
      uploader: t.uploader,
      duration_s: t.duration_s,
      kind: t.kind,
      position: t.position,
    }));
    emit("queue:set", { name, listId: selectedList, items }).catch(() => {});
  }

  // Re-broadcast whenever the queue changes: a list switch, a scan, a
  // reorder, a removal.
  $effect(() => {
    announceQueue();
  });

  async function openList(id: number | null) {
    selectedList = id;
    listItems = id == null ? [] : await invoke<MediaRow[]>("playlist_items", { id });
  }

  async function newList() {
    const name = prompt("Playlist name")?.trim();
    if (!name) return;
    await invoke("create_playlist", { name });
    refreshLibrary();
  }

  /** From a row's "+" menu: make the list and put this track in it, one step. */
  async function newListWith(mediaId: number) {
    const name = prompt("Playlist name")?.trim();
    if (!name) return;
    const id = await invoke<number>("create_playlist", { name });
    await invoke("add_to_playlist", { playlistId: id, mediaId });
    refreshLibrary();
  }

  // Reorder by dragging the grip. Pointer events rather than HTML5 drag and
  // drop: on Windows the webview's own drop handler eats HTML5 drags unless
  // it is switched off for the window, and this needs no such switch. The
  // grip captures the pointer, every move re-reads the rows' midpoints to
  // find the insertion index, and release asks Rust to move the row.
  function gripDown(e: PointerEvent, i: number) {
    if (e.button !== 0 || selectedList == null) return;
    e.preventDefault();
    const t = shown[i];
    const grip = e.currentTarget as HTMLElement;
    grip.setPointerCapture(e.pointerId);
    dragId = t.id;
    dropAt = i;

    const onMove = (ev: PointerEvent) => {
      const rows = Array.from(rowsEl.querySelectorAll<HTMLElement>("li[data-idx]"));
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
      grip.removeEventListener("pointermove", onMove);
      grip.removeEventListener("pointerup", onUp);
      grip.removeEventListener("pointercancel", onUp);
      const at = dropAt ?? i;
      dragId = null;
      dropAt = null;
      // `at` is an index among the rows as they are; the row itself leaves
      // first, so a target below it shifts up by one.
      const dest = at > i ? at - 1 : at;
      if (dest !== i) move(t.position!, dest);
    };
    grip.addEventListener("pointermove", onMove);
    grip.addEventListener("pointerup", onUp);
    grip.addEventListener("pointercancel", onUp);
  }

  async function addTo(playlistId: number, mediaId: number) {
    await invoke("add_to_playlist", { playlistId, mediaId });
    refreshLibrary();
  }

  async function removeAt(position: number) {
    if (selectedList == null) return;
    await invoke("remove_from_playlist", { playlistId: selectedList, position });
    refreshLibrary();
  }

  async function move(from: number, to: number) {
    if (selectedList == null) return;
    await invoke("reorder_playlist", { playlistId: selectedList, from, to });
    refreshLibrary();
  }

  function play(t: MediaRow) {
    // A new attempt clears the last verdict, whichever kind it was. Clearing
    // only inside the video branch left a video failure sitting over a later
    // audio play that worked.
    error = null;
    missing = null;
    // The cursor moves for either kind, so next and prev walk on from a video
    // as well as from a track.
    current = t;
    // Video gets its own decorated OS window (D13) — it is deliberately not
    // part of the bond group, and the audio element here can't show it.
    if (t.kind === "video") {
      // Resolves when the window confirms the switch, rejects when it does
      // not (D68). Before this the call could not fail (D67), so a dead
      // window read as success.
      invoke("open_video", { id: t.id }).catch((e) => (error = String(e)));
      // One transport (D69): a video starting pauses the audio. It does not
      // resume when the video ends or closes; the user restarts it.
      emitTo("main", "player:pause").catch(() => {});
      return;
    }
    emitTo("main", "player:load", t).catch((e) => (error = String(e)));
  }

  // The row that is playing toggles instead of restarting.
  function toggle() {
    emitTo("main", "player:toggle").catch(() => {});
  }

  function step(delta: number) {
    if (!current) return;
    const i = shown.findIndex((t) => t.id === current!.id);
    const next = shown[i + delta];
    if (next) play(next);
  }

  /** A notice and the offers riding on it go together. */
  function clearNotice() {
    notice = null;
    error = null;
    pendingDelete = [];
    pendingPrune = null;
  }

  /**
   * Take the row out of the library (#78). The file stays; the notice says
   * where, and offers the separate step. Playlist memberships go with the
   * row in Rust. Every window holding the track lets go of it.
   */
  async function removeTracks(ids: number[]) {
    if (!ids.length) return;
    clearNotice();
    try {
      const gone = await invoke<Removed[]>("remove_tracks", { ids });
      for (const id of ids) {
        emitTo("main", "player:removed", id).catch(() => {});
        emitTo("video", "hp://removed", id).catch(() => {});
        if (current?.id === id) current = null;
        if (missing?.id === id) missing = null;
      }
      selected = selected.filter((id) => !ids.includes(id));
      pendingDelete = gone;
      notice =
        gone.length === 1
          ? `Removed “${gone[0].title}” from the library. Its file is still at ${gone[0].path}`
          : `Removed ${gone.length} tracks from the library. Their files are still on disk.`;
      await refreshLibrary();
    } catch (e) {
      error = String(e);
    }
  }

  /** The single-row form, for the "moved or deleted" message's button. */
  const removeTrack = (id: number) => removeTracks([id]);

  function toggleSelected(id: number) {
    selected = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id];
  }

  /**
   * The one destructive action in the app, and it reads like one: a warning
   * dialog that prints the path, with Keep as the safe answer. Rust refuses
   * anything outside a library root regardless.
   */
  async function deleteFiles() {
    const rs = pendingDelete;
    if (!rs.length) return;
    // Every path, in the dialog, up to a screenful; past that the count
    // carries it, and the notice above still lists what was removed.
    const shown = rs.slice(0, 12).map((r) => r.path);
    const more = rs.length - shown.length;
    const list = shown.join("\n") + (more > 0 ? `\n…and ${more} more` : "");
    const head = rs.length === 1 ? "Delete this file from disk?" : `Delete these ${rs.length} files from disk?`;
    const yes = await ask(`${head}\n\n${list}\n\nThere is no undo.`, {
      title: rs.length === 1 ? "Delete the file" : `Delete ${rs.length} files`,
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Keep",
    });
    if (!yes) return;
    const failed: string[] = [];
    for (const r of rs) {
      try {
        await invoke("delete_media_file", { path: r.path });
      } catch (e) {
        failed.push(String(e));
      }
    }
    pendingDelete = [];
    const done = rs.length - failed.length;
    notice = rs.length === 1 && !failed.length ? `Deleted ${rs[0].path}` : `Deleted ${done} of ${rs.length} files.`;
    if (failed.length) error = failed.join("\n");
  }

  /** After a rescan found rows whose files are gone: drop them. */
  async function pruneMissing() {
    const p = pendingPrune;
    if (!p) return;
    try {
      const n = await invoke<number>("prune_root", { rootId: p.rootId });
      pendingPrune = null;
      notice = `Removed ${n} row${n === 1 ? "" : "s"} whose file${n === 1 ? " is" : "s are"} gone. The playlists closed up around ${n === 1 ? "it" : "them"}.`;
      await refreshLibrary();
    } catch (e) {
      error = String(e);
    }
  }

  async function addFolder() {
    // Clear first. Leaving the previous run's notice on screen while a new one
    // is in flight is how a no-op reads as a success.
    clearNotice();

    const picked = await openDialog({ directory: true, multiple: false, title: "Add a music folder" });

    // A cancel and a dialog that failed to return a path both arrive here, and
    // returning silently made them indistinguishable — from each other and from
    // a scan that ran and found nothing. That is exactly how a folder that
    // imports perfectly well when handed straight to the scanner can look like
    // it "silently does nothing" in the UI.
    if (picked === null || picked === undefined) {
      notice = "No folder chosen.";
      return;
    }
    if (typeof picked !== "string") {
      error = `The folder picker returned something unexpected: ${JSON.stringify(picked)}`;
      return;
    }

    scanning = true;
    try {
      const r = await invoke<{ root_id: number; found: number; added: number; updated: number; missing: number }>(
        "add_local_folder", { path: picked }
      );
      notice =
        r.found === 0
          ? `${picked} — no audio or video files found in that folder or below it.`
          : `Scanned ${r.found} file${r.found === 1 ? "" : "s"} — ${r.added} added, ${r.updated} updated.`;
      // A known root, rescanned: rows whose files have left are counted, not
      // dropped. Dropping them is the offer beside the notice (#78).
      if (r.missing > 0) {
        notice += ` ${r.missing} row${r.missing === 1 ? "" : "s"} in the library point${r.missing === 1 ? "s" : ""} at a file that is gone.`;
        pendingPrune = { rootId: r.root_id, count: r.missing };
      }
      await refreshLibrary();
    } catch (e) {
      error = String(e);
    } finally {
      scanning = false;
    }
  }

  async function setConc(n: number) {
    concurrency = n;
    await invoke("set_concurrency", { n });
  }
</script>

<svelte:window
  onpointerdown={() => (addMenuFor = null)}
  onkeydown={(e) => {
    if (e.key === "Escape") {
      addMenuFor = null;
      selected = [];
    }
  }}
/>

<main>
  <header>
    <h1>hurricane-party</h1>
    <span class="ver">v0.4 — the classic windows play: analyser, EQ, playlist, bonds that glow</span>
    <label class="conc">
      concurrent
      <select value={concurrency} onchange={(e) => setConc(+e.currentTarget.value)}>
        {#each [1, 2, 3, 4] as n}<option value={n}>{n}</option>{/each}
      </select>
    </label>
  </header>

  <form onsubmit={(e) => { e.preventDefault(); add(); }}>
    <input bind:value={url} placeholder="Paste a URL — it queues, and survives a restart" />
    <label class="vid"><input type="checkbox" bind:checked={wantVideo} /> video</label>
    <button type="submit" disabled={!url.trim()}>Queue</button>
    <button type="button" onclick={addFolder} disabled={scanning}>
      {scanning ? "Scanning…" : "Add folder"}
    </button>
  </form>

  {#if notice}
    <p class="notice">
      <span>{notice}</span>
      {#if pendingDelete.length}
        <button class="mini danger" onclick={deleteFiles} title="Asks first, and shows every path">
          {pendingDelete.length === 1 ? "Delete the file…" : `Delete the ${pendingDelete.length} files…`}
        </button>
      {/if}
      {#if pendingPrune}
        <button class="mini" onclick={pruneMissing}>Remove {pendingPrune.count === 1 ? "it" : `those ${pendingPrune.count}`} from the library</button>
      {/if}
    </p>
  {/if}

  {#if error}<p class="error">{error}</p>{/if}

  <!-- Main could not open the file (#43). The same words it shows, here where
       the row is, with the way out (#78). The file is already gone, so there
       is nothing to offer to delete. -->
  {#if missing}
    <p class="error">
      <span>Can't open “{missing.title}”. Moved or deleted?</span>
      <button class="mini" onclick={() => removeTrack(missing!.id)}>Remove from library</button>
    </p>
  {/if}

  {#if jobs.length}
    <section class="queue">
      <h2>Downloads</h2>
      {#each jobs as j (j.id)}
        <div class="job" class:failed={j.status === "failed"}>
          <div class="line">
            <span class="status {j.status}">{j.status}</span>
            <span class="stage">{j.stage}</span>
            <span class="what">{j.title ?? j.url}</span>
            {#if j.status === "running" && j.bytes_total}
              <span class="bytes">{mb(j.bytes_done)} / {mb(j.bytes_total)}</span>
            {/if}
            {#if j.status === "failed"}
              <button class="mini" onclick={() => invoke("retry_job", { id: j.id }).then(refreshJobs)}>Retry</button>
            {/if}
            {#if j.status === "queued" || j.status === "running"}
              <button class="mini" onclick={() => invoke("cancel_job", { id: j.id }).then(refreshJobs)}>Pause</button>
            {/if}
          </div>
          {#if j.status === "running"}
            <div class="bar">
              <div class="fill" class:indeterminate={!j.bytes_total}
                   style:width={j.bytes_total ? j.progress * 100 + "%" : "100%"}></div>
            </div>
          {/if}
          {#if j.error}<p class="joberr">{j.error}</p>{/if}
        </div>
      {/each}
    </section>
  {/if}

  <section class="body">
    <nav>
      <button class="lib" class:sel={selectedList == null} onclick={() => openList(null)}>
        Library <span class="n">{tracks.length}</span>
      </button>
      {#each playlists as p (p.id)}
        <button class:sel={selectedList === p.id} onclick={() => openList(p.id)}>
          {p.name} <span class="n">{p.count}</span>
        </button>
      {/each}
      <button class="new" onclick={newList}>+ New playlist</button>
      {#if roots.length > 1}
        <div class="roots">
          <span class="rootlabel">Roots</span>
          {#each roots as r (r.id)}
            <!-- A missing root is an unplugged drive, not a broken library (D28) -->
            <span class="root" class:gone={!r.present} title={r.path}>
              {r.label} <span class="n">{r.count}</span>
            </span>
          {/each}
        </div>
      {/if}
    </nav>

    <div class="listcol">
      <!-- The selection bar (D84): only while something is checked, so the
           list is quiet until it is asked for something. Removal here keeps
           the files; the delete offer follows on the notice, as before. -->
      {#if selectedList == null && selected.length}
        <div class="selbar">
          <span class="count">{selected.length} selected</span>
          <button class="mini" onclick={() => removeTracks(selected)} title="The files stay on disk">Remove from library</button>
          <button class="mini ghost" onclick={() => (selected = [])} title="Esc">Clear</button>
        </div>
      {/if}
    <ul class="tracks" bind:this={rowsEl}>
      {#each shown as t, i (t.id + ":" + (t.position ?? "l"))}
        <li
          class:current={current?.id === t.id}
          class:lifted={dragId === t.id}
          class:drop-before={dragId != null && dropAt === i}
          class:drop-after={dragId != null && dropAt === shown.length && i === shown.length - 1}
          data-idx={i}
        >
          {#if selectedList != null}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <span class="grip" onpointerdown={(e) => gripDown(e, i)} title="Drag to reorder">⋮⋮</span>
          {:else}
            <!-- Check to select; the bar above does the removing (D84). -->
            <input
              class="tick"
              type="checkbox"
              checked={selected.includes(t.id)}
              onchange={() => toggleSelected(t.id)}
              title="Select"
            />
          {/if}
          {#if t.kind === "video"}
            <button class="play" onclick={() => play(t)}>▣</button>
          {:else if nowId === t.id}
            <button class="play" onclick={toggle}>{isPlaying ? "‖" : "▶"}</button>
          {:else}
            <button class="play" onclick={() => play(t)}>▶</button>
          {/if}
          <span class="title">{t.title}</span>
          <span class="meta">{duration(t.duration_s)} · {mb(t.filesize)}</span>
          {#if selectedList == null}
            <!-- Pointerdowns inside stay inside, so the window-level
                 "click anywhere else closes the menu" does not close it
                 under a click on one of its own items. -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <span class="addwrap" onpointerdown={(e) => e.stopPropagation()}>
              <button
                class="add"
                class:open={addMenuFor === t.id}
                onclick={() => (addMenuFor = addMenuFor === t.id ? null : t.id)}
                title="Add to a playlist">+</button
              >
              {#if addMenuFor === t.id}
                <div class="menu" role="menu">
                  {#each playlists as p (p.id)}
                    <button role="menuitem" onclick={() => { addTo(p.id, t.id); addMenuFor = null; }}>{p.name}</button>
                  {:else}
                    <div class="none">No playlists yet</div>
                  {/each}
                  <button role="menuitem" class="new" onclick={() => { addMenuFor = null; newListWith(t.id); }}>+ New playlist…</button>
                </div>
              {/if}
            </span>
          {:else}
            <button class="mini" onclick={() => removeAt(t.position!)} title="Remove from this playlist">×</button>
          {/if}
        </li>
      {:else}
        {#if selectedList == null}
          <!-- First run. An invitation, not an apology (design brief), and
               the surfer's home (#62). -->
          <li class="empty hangten">
            <img src={surfer} alt="A capybara surfing a red board with the hurricane warning's black square, boombox on his shoulder" width="220" height="220">
            <div class="say">
              <div class="big">Hang ten. Nothing but surf ahead.</div>
              <div>Paste a link above to keep it on disk, or add a folder of MP3s you already own.</div>
            </div>
          </li>
        {:else}
          <li class="empty">Empty playlist. Add tracks from the library.</li>
        {/if}
      {/each}
    </ul>
    </div>
  </section>

  <footer><span>Library</span><code>{libraryPath}</code></footer>
</main>

<style>
  /* No max-width and a small gutter: the list is the point of this window, and
     a centred 900px box just put a margin on both sides of it (#48). */
  main { margin: 0; padding: 12px 14px; display: flex; flex-direction: column; gap: 14px; }
  header { display: flex; align-items: baseline; gap: 12px; flex-wrap: wrap; }
  h1 { margin: 0; font-size: 19px; font-weight: 400; letter-spacing: 2px; text-transform: uppercase;
       color: var(--arc); text-shadow: 0 0 10px color-mix(in srgb, var(--arc) 45%, transparent); }
  h2 { margin: 0 0 6px; font-size: 10px; letter-spacing: 1.5px; text-transform: uppercase;
       color: color-mix(in srgb, var(--filament) 45%, transparent); font-weight: 400; }
  .ver { font-size: 12px; color: color-mix(in srgb, var(--filament) 45%, transparent); }
  .vid { font-size: 11px; display: flex; align-items: center; gap: 4px;
         color: color-mix(in srgb, var(--filament) 55%, transparent); white-space: nowrap; }
  .notice { margin: 0; font-size: 12px; color: var(--arc); display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .notice span, .error span { overflow-wrap: anywhere; }
  .error { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  /* The one destructive control reads as one: ember, not arc. */
  .mini.danger { color: var(--ember); border-color: color-mix(in srgb, var(--ember) 50%, transparent); }
  .mini.danger:hover { background: color-mix(in srgb, var(--ember) 14%, transparent); border-color: var(--ember); }
  .roots { display: flex; flex-direction: column; gap: 2px; margin-top: 10px;
           padding-top: 8px; border-top: 1px solid color-mix(in srgb, var(--arc) 12%, transparent); }
  .rootlabel { font-size: 9px; letter-spacing: 1.2px; text-transform: uppercase;
               color: color-mix(in srgb, var(--filament) 30%, transparent); }
  .root { font-size: 11px; padding: 2px 8px; display: flex; justify-content: space-between;
          color: color-mix(in srgb, var(--filament) 70%, transparent); }
  .root.gone { color: var(--ember); text-decoration: line-through; }
  .conc { margin-left: auto; font-size: 11px; color: color-mix(in srgb, var(--filament) 45%, transparent); }
  select { font: inherit; font-size: 11px; background: var(--well); color: var(--filament);
           border: 1px solid color-mix(in srgb, var(--arc) 30%, transparent); padding: 2px 4px; }

  form { display: flex; gap: 8px; }
  form input { flex: 1 1 auto; min-width: 0; }

  .queue { display: flex; flex-direction: column; gap: 8px; }
  .job { display: flex; flex-direction: column; gap: 4px; padding: 7px 9px; background: var(--well);
         border: 1px solid color-mix(in srgb, var(--arc) 16%, transparent); }
  .job.failed { border-color: color-mix(in srgb, var(--ember) 45%, transparent); }
  .line { display: flex; align-items: baseline; gap: 8px; font-size: 12px; }
  .status { font-size: 9px; letter-spacing: 1px; text-transform: uppercase; }
  .status.running { color: var(--arc); }
  .status.queued  { color: color-mix(in srgb, var(--filament) 45%, transparent); }
  .status.failed  { color: var(--ember); }
  .status.done    { color: var(--strike); }
  .status.paused  { color: color-mix(in srgb, var(--filament) 35%, transparent); }
  .stage { font-size: 9px; letter-spacing: 1px; text-transform: uppercase;
           color: color-mix(in srgb, var(--filament) 35%, transparent); }
  .what { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bytes { font-size: 11px; color: color-mix(in srgb, var(--filament) 50%, transparent); }
  .joberr { margin: 0; font-size: 11px; color: var(--ember); white-space: pre-wrap; }

  .bar { height: 2px; background: color-mix(in srgb, var(--void) 80%, black); overflow: hidden; }
  .fill { height: 100%; background: var(--arc); box-shadow: 0 0 6px var(--arc); transition: width 200ms linear; }
  .fill.indeterminate { animation: pulse 1.1s ease-in-out infinite; }
  @keyframes pulse { 0%,100% { opacity: .25 } 50% { opacity: .9 } }
  @media (prefers-reduced-motion: reduce) { .fill.indeterminate { animation: none; opacity: .6 } }

  .error { margin: 0; padding: 9px 11px; font-size: 13px; color: var(--ember);
           border: 1px solid color-mix(in srgb, var(--ember) 45%, transparent);
           background: color-mix(in srgb, var(--ember) 8%, transparent); white-space: pre-wrap; }

  /* minmax(0, 1fr), not 1fr: a bare 1fr is minmax(auto, 1fr), and the track
     rows' nowrap titles make the list's minimum width the longest title, so the
     column grew past the window and the page scrolled sideways (#48). */
  .body { display: grid; grid-template-columns: 170px minmax(0, 1fr); gap: 12px; align-items: start; }
  nav { display: flex; flex-direction: column; gap: 3px; }
  nav button { text-align: left; border-color: transparent; color: var(--filament);
               padding: 5px 8px; font-size: 12px; display: flex; justify-content: space-between; gap: 6px; }
  nav button.sel { border-color: var(--arc); color: var(--arc); }
  nav button.new { color: color-mix(in srgb, var(--filament) 45%, transparent); font-size: 11px; margin-top: 4px; }
  .n { font-size: 10px; color: color-mix(in srgb, var(--filament) 35%, transparent); }

  .tracks { list-style: none; margin: 0; padding: 0; background: var(--well);
            border: 1px solid color-mix(in srgb, var(--arc) 20%, transparent); }
  .tracks li { display: flex; align-items: center; gap: 8px; padding: 6px 9px;
               border-bottom: 1px solid color-mix(in srgb, var(--arc) 9%, transparent); }
  .tracks li:last-child { border-bottom: none; }
  .tracks li.current .title { color: var(--strike); text-shadow: 0 0 8px color-mix(in srgb, var(--strike) 45%, transparent); }
  .tracks li.empty { color: color-mix(in srgb, var(--filament) 45%, transparent); font-size: 13px; }
  /* The surfer's home. The image sits on the well; the words beside it. */
  .tracks li.hangten { gap: 22px; padding: 28px 24px; align-items: center; }
  .tracks li.hangten img { flex: 0 0 auto; width: 220px; height: 220px;
                           filter: drop-shadow(0 0 18px color-mix(in srgb, var(--arc) 22%, transparent)); }
  .tracks li.hangten .say { display: flex; flex-direction: column; gap: 8px; max-width: 420px; line-height: 1.5; }
  .tracks li.hangten .big { font-size: 17px; color: var(--filament); }
  .play { padding: 1px 7px; font-size: 10px; }
  .mini { padding: 1px 6px; font-size: 10px; border-color: color-mix(in srgb, var(--arc) 30%, transparent); }
  .mini.ghost { border-color: transparent; color: color-mix(in srgb, var(--filament) 55%, transparent); }
  /* The right column: the selection bar, when there is one, sits on the list. */
  .listcol { display: flex; flex-direction: column; min-width: 0; }
  .selbar { display: flex; align-items: center; gap: 10px; padding: 5px 9px; font-size: 12px;
            color: var(--arc); background: color-mix(in srgb, var(--arc) 10%, var(--well));
            border: 1px solid color-mix(in srgb, var(--arc) 45%, transparent); border-bottom: none; }
  .selbar .count { flex: 1 1 auto; }
  .tick { flex: 0 0 auto; width: 13px; height: 13px; margin: 0; accent-color: var(--arc); cursor: pointer; }
  /* Drag-to-reorder: the grip, the lifted row, and the insertion line. */
  .grip { flex: 0 0 auto; padding: 0 2px; font-size: 12px; letter-spacing: -3px; line-height: 1;
          color: color-mix(in srgb, var(--filament) 30%, transparent); cursor: grab; user-select: none; touch-action: none; }
  .grip:hover { color: var(--arc); }
  .tracks li.lifted { opacity: 0.4; }
  .tracks li.lifted .grip { cursor: grabbing; }
  .tracks li.drop-before { box-shadow: inset 0 2px 0 var(--arc); }
  .tracks li.drop-after { box-shadow: inset 0 -2px 0 var(--arc); }

  /* "+" opens a menu of playlists, anchored to the row. */
  .addwrap { position: relative; flex: 0 0 auto; }
  .add { width: 22px; height: 22px; padding: 0; display: grid; place-items: center;
         font-size: 16px; line-height: 1; color: var(--arc);
         border: 1px solid color-mix(in srgb, var(--arc) 35%, transparent); background: transparent; }
  .add:hover, .add.open { background: color-mix(in srgb, var(--arc) 14%, transparent); border-color: var(--arc);
                          box-shadow: 0 0 8px color-mix(in srgb, var(--arc) 35%, transparent); }
  .menu { position: absolute; right: 0; top: 26px; z-index: 5; min-width: 160px; padding: 4px 0;
          display: flex; flex-direction: column; background: var(--void); border: 1px solid var(--arc);
          box-shadow: 0 0 0 1px color-mix(in srgb, var(--arc) 40%, transparent), 0 0 12px color-mix(in srgb, var(--arc) 25%, transparent); }
  .menu button { text-align: left; border: 0; color: var(--filament); padding: 6px 10px; font-size: 12px;
                 white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .menu button:hover { background: color-mix(in srgb, var(--arc) 14%, transparent); color: var(--arc); }
  .menu .new { color: color-mix(in srgb, var(--filament) 55%, transparent); margin-top: 2px;
               border-top: 1px solid color-mix(in srgb, var(--arc) 15%, transparent); }
  .menu .none { padding: 6px 10px; font-size: 11px; color: color-mix(in srgb, var(--filament) 40%, transparent); }
  .title { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: 11px; color: color-mix(in srgb, var(--filament) 45%, transparent); flex: 0 0 auto; }

  footer { display: flex; gap: 8px; align-items: baseline; font-size: 11px;
           color: color-mix(in srgb, var(--filament) 35%, transparent); }
  footer code { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
