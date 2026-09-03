<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
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
    kind: string;
    path: string;
    position: number | null;
  };

  type Root = { id: number; label: string; path: string; count: number; present: boolean };

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
  let wantVideo = $state(false);
  let roots = $state<Root[]>([]);
  let scanning = $state(false);
  let notice = $state<string | null>(null);
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
      // Transport arriving from the control pipe. Rust relays rather than
      // executes, because the audio is here.
      listen<{ cmd: string; arg: unknown }>("control-command", (e) => {
        const { cmd, arg } = e.payload;
        if (!audio) return;
        if (cmd === "play") audio.play();
        else if (cmd === "pause") audio.pause();
        else if (cmd === "toggle") audio.paused ? audio.play() : audio.pause();
        else if (cmd === "stop") { audio.pause(); audio.currentTime = 0; playing = null; }
        else if (cmd === "next") step(1);
        else if (cmd === "prev") step(-1);
        else if (cmd === "seek") audio.currentTime = Number(arg) || 0;
        else if (cmd === "volume") audio.volume = Math.min(1, Math.max(0, Number(arg)));
        pushState();
      }),
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
    // A new attempt clears the last verdict, whichever kind it was. Clearing
    // only inside the video branch left a video failure sitting over a later
    // audio play that worked.
    error = null;
    // Video gets its own decorated OS window (D13) — it is deliberately not
    // part of the bond group, and the audio element here can't show it.
    if (t.kind === "video") {
      // Resolves when the window confirms the switch, rejects when it does
      // not (D68). Before this the call could not fail (D67), so a dead
      // window read as success.
      invoke("open_video", { id: t.id }).catch((e) => (error = String(e)));
      return;
    }
    playing = t;
    audio.src = convertFileSrc(t.path);
    audio.play();
    pushState();
  }

  /// Mirror playback state into Rust so the control channel can answer
  /// `status` truthfully — the audio graph lives here, not there (D5).
  function pushState() {
    invoke("report_state", {
      state: {
        state: !playing ? "stopped" : audio?.paused ? "paused" : "playing",
        media_id: playing?.id ?? null,
        title: playing?.title ?? null,
        uploader: playing?.uploader ?? null,
        duration_s: playing?.duration_s ?? null,
        pos_s: audio?.currentTime ?? null,
        volume: audio?.volume ?? 1,
      },
    }).catch(() => {});
  }

  function step(delta: number) {
    if (!playing) return;
    const i = shown.findIndex((t) => t.id === playing!.id);
    const next = shown[i + delta];
    if (next) play(next);
  }

  async function addFolder() {
    // Clear first. Leaving the previous run's notice on screen while a new one
    // is in flight is how a no-op reads as a success.
    notice = null;
    error = null;

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
      const r = await invoke<{ found: number; added: number; updated: number }>(
        "add_local_folder", { path: picked }
      );
      notice =
        r.found === 0
          ? `${picked} — no audio files found in that folder or below it.`
          : `Scanned ${r.found} file${r.found === 1 ? "" : "s"} — ${r.added} added, ${r.updated} updated.`;
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

<main>
  <header>
    <h1>hurricane-party</h1>
    <span class="ver">v0.3 — video window, local folders, control API</span>
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

  {#if notice}<p class="notice">{notice}</p>{/if}

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

    <ul class="tracks">
      {#each shown as t, i (t.id + ":" + (t.position ?? "l"))}
        <li class:current={playing?.id === t.id}>
          <button class="play" onclick={() => play(t)}>{t.kind === "video" ? "▣" : "▶"}</button>
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
  <audio bind:this={audio} controls
         onplay={pushState} onpause={pushState} onended={() => step(1)}
         onvolumechange={pushState}></audio>

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
  .vid { font-size: 11px; display: flex; align-items: center; gap: 4px;
         color: color-mix(in srgb, var(--filament) 55%, transparent); white-space: nowrap; }
  .notice { margin: 0; font-size: 12px; color: var(--arc); }
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
