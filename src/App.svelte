<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { applyTheme } from "./lib/theme";

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
    path: string;
    position: number | null;
  };

  type Playlist = { id: number; name: string; count: number };

  let url = $state("");
  let error = $state<string | null>(null);
  let jobs = $state<Job[]>([]);
  let tracks = $state<MediaRow[]>([]);
  let playlists = $state<Playlist[]>([]);
  let selectedList = $state<number | null>(null);
  let listItems = $state<MediaRow[]>([]);
  let playing = $state<MediaRow | null>(null);
  let libraryPath = $state("");
  let concurrency = $state(2);
  let audio: HTMLAudioElement;

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
    playlists = await invoke<Playlist[]>("list_playlists");
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
      await invoke<number>("enqueue_url", { url: u });
      url = "";
      refreshJobs();
    } catch (e) {
      error = String(e);
    }
  }

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
    playing = t;
    audio.src = convertFileSrc(t.path);
    audio.play();
  }

  async function setConc(n: number) {
    concurrency = n;
    await invoke("set_concurrency", { n });
  }
</script>

<main>
  <header>
    <h1>hurricane-party</h1>
    <span class="ver">v0.2 — persistent queue, library, playlists</span>
    <label class="conc">
      concurrent
      <select value={concurrency} onchange={(e) => setConc(+e.currentTarget.value)}>
        {#each [1, 2, 3, 4] as n}<option value={n}>{n}</option>{/each}
      </select>
    </label>
  </header>

  <form onsubmit={(e) => { e.preventDefault(); add(); }}>
    <input bind:value={url} placeholder="Paste a URL — it queues, and survives a restart" />
    <button type="submit" disabled={!url.trim()}>Queue</button>
  </form>

  {#if error}<p class="error">{error}</p>{/if}

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
    </nav>

    <ul class="tracks">
      {#each shown as t, i (t.id + ":" + (t.position ?? "l"))}
        <li class:current={playing?.id === t.id}>
          <button class="play" onclick={() => play(t)}>▶</button>
          <span class="title">{t.title}</span>
          <span class="meta">{duration(t.duration_s)} · {mb(t.filesize)}</span>
          {#if selectedList == null}
            {#if playlists.length}
              <select class="addto" onchange={(e) => { addTo(+e.currentTarget.value, t.id); e.currentTarget.selectedIndex = 0; }}>
                <option>add to…</option>
                {#each playlists as p}<option value={p.id}>{p.name}</option>{/each}
              </select>
            {/if}
          {:else}
            <button class="mini" disabled={i === 0} onclick={() => move(t.position!, i - 1)}>↑</button>
            <button class="mini" disabled={i === shown.length - 1} onclick={() => move(t.position!, i + 1)}>↓</button>
            <button class="mini" onclick={() => removeAt(t.position!)}>×</button>
          {/if}
        </li>
      {:else}
        <li class="empty">
          {selectedList == null
            ? "Nothing saved yet. Paste a link above to keep it on disk."
            : "Empty playlist. Add tracks from the library."}
        </li>
      {/each}
    </ul>
  </section>

  <!-- svelte-ignore a11y_media_has_caption -->
  <audio bind:this={audio} controls></audio>

  <footer><span>Library</span><code>{libraryPath}</code></footer>
</main>

<style>
  main { max-width: 900px; margin: 0 auto; padding: 20px; display: flex; flex-direction: column; gap: 14px; }
  header { display: flex; align-items: baseline; gap: 12px; flex-wrap: wrap; }
  h1 { margin: 0; font-size: 19px; font-weight: 400; letter-spacing: 2px; text-transform: uppercase;
       color: var(--arc); text-shadow: 0 0 10px color-mix(in srgb, var(--arc) 45%, transparent); }
  h2 { margin: 0 0 6px; font-size: 10px; letter-spacing: 1.5px; text-transform: uppercase;
       color: color-mix(in srgb, var(--filament) 45%, transparent); font-weight: 400; }
  .ver { font-size: 12px; color: color-mix(in srgb, var(--filament) 45%, transparent); }
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

  .body { display: grid; grid-template-columns: 170px 1fr; gap: 12px; align-items: start; }
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
  .play { padding: 1px 7px; font-size: 10px; }
  .mini { padding: 1px 6px; font-size: 10px; border-color: color-mix(in srgb, var(--arc) 30%, transparent); }
  .addto { font-size: 10px; }
  .title { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: 11px; color: color-mix(in srgb, var(--filament) 45%, transparent); flex: 0 0 auto; }

  audio { width: 100%; }
  footer { display: flex; gap: 8px; align-items: baseline; font-size: 11px;
           color: color-mix(in srgb, var(--filament) 35%, transparent); }
  footer code { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
