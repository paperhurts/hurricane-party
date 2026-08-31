<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { applyTheme } from "./lib/theme";

  type Track = {
    id: string;
    title: string;
    uploader: string | null;
    duration_s: number | null;
    path: string;
    filesize: number;
  };

  type Progress = {
    url: string;
    stage: "probe" | "download" | "extract" | "done";
    bytes_done: number;
    bytes_total: number | null;
    speed_bps: number | null;
    eta_s: number | null;
    note: string | null;
  };

  let url = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let progress = $state<Progress | null>(null);
  let tracks = $state<Track[]>([]);
  let playing = $state<Track | null>(null);
  let libraryPath = $state("");
  let audio: HTMLAudioElement;

  // Percent is only meaningful once yt-dlp knows the total, which it often
  // doesn't until the transfer is under way.
  let pct = $derived(
    progress?.bytes_total
      ? Math.round((progress.bytes_done / progress.bytes_total) * 100)
      : null,
  );

  const mb = (n: number) => (n / 1_048_576).toFixed(1) + " MB";

  function duration(s: number | null) {
    if (s == null) return "";
    const m = Math.floor(s / 60);
    return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
  }

  onMount(() => {
    applyTheme("eyewall");
    refresh();
    invoke<string>("library_path").then((p) => (libraryPath = p));
    const un = listen<Progress>("job-progress", (e) => (progress = e.payload));
    return () => {
      un.then((f) => f());
    };
  });

  async function refresh() {
    try {
      tracks = await invoke<Track[]>("list_tracks");
    } catch (e) {
      error = String(e);
    }
  }

  async function add() {
    if (!url.trim() || busy) return;
    busy = true;
    error = null;
    progress = null;
    try {
      const t = await invoke<Track>("import_url", { url });
      // Replace rather than append: re-importing the same id overwrites on disk.
      tracks = [t, ...tracks.filter((x) => x.id !== t.id)];
      url = "";
    } catch (e) {
      // The Rust side sends a human-readable string, including the tail of
      // yt-dlp's stderr. Errors say what happened, not just that it failed.
      error = String(e);
    } finally {
      busy = false;
      progress = null;
    }
  }

  function play(t: Track) {
    playing = t;
    // Local file -> asset: URL. The CSP allows exactly this origin for media
    // and nothing remote (D29); the path must sit inside the assetProtocol
    // scope declared in tauri.conf.json.
    audio.src = convertFileSrc(t.path);
    audio.play();
  }
</script>

<main>
  <header>
    <h1>hurricane-party</h1>
    <span class="ver">v0.1 — paste a URL, get an MP3, hear it</span>
  </header>

  <form
    onsubmit={(e) => {
      e.preventDefault();
      add();
    }}
  >
    <input
      bind:value={url}
      placeholder="Paste a YouTube, Bandcamp, or SoundCloud URL"
      disabled={busy}
    />
    <button type="submit" disabled={busy || !url.trim()}>
      {busy ? "Working" : "Save"}
    </button>
  </form>

  {#if progress}
    <div class="progress">
      <div class="row">
        <span class="stage">{progress.stage}</span>
        {#if progress.note}<span class="note">{progress.note}</span>{/if}
        {#if progress.stage === "download"}
          <span class="bytes">
            {mb(progress.bytes_done)}{#if progress.bytes_total}
              / {mb(progress.bytes_total)}{/if}
            {#if progress.speed_bps}· {mb(progress.speed_bps)}/s{/if}
            {#if progress.eta_s != null}· {progress.eta_s}s left{/if}
          </span>
        {/if}
      </div>
      <div class="bar"><div class="fill" style:width={pct != null ? pct + "%" : "100%"} class:indeterminate={pct == null}></div></div>
    </div>
  {/if}

  {#if error}
    <p class="error">{error}</p>
  {/if}

  <ul class="tracks">
    {#each tracks as t (t.id)}
      <li class:current={playing?.id === t.id}>
        <button class="play" onclick={() => play(t)}>▶</button>
        <span class="title">{t.title}</span>
        <span class="meta">{duration(t.duration_s)} · {mb(t.filesize)}</span>
      </li>
    {:else}
      <li class="empty">Nothing saved yet. Paste a link above to keep it on disk.</li>
    {/each}
  </ul>

  <!-- svelte-ignore a11y_media_has_caption -->
  <audio bind:this={audio} controls></audio>

  <footer>
    <span>Library</span>
    <code>{libraryPath}</code>
  </footer>
</main>

<style>
  main {
    max-width: 780px;
    margin: 0 auto;
    padding: 24px 20px 40px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 400;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: var(--arc);
    text-shadow: 0 0 10px color-mix(in srgb, var(--arc) 45%, transparent);
  }
  .ver {
    font-size: 12px;
    color: color-mix(in srgb, var(--filament) 45%, transparent);
  }
  form {
    display: flex;
    gap: 8px;
  }
  form input {
    flex: 1 1 auto;
    min-width: 0;
  }

  .progress {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .progress .row {
    display: flex;
    gap: 10px;
    align-items: baseline;
    font-size: 12px;
    flex-wrap: wrap;
  }
  .stage {
    text-transform: uppercase;
    letter-spacing: 1.2px;
    color: var(--arc);
  }
  .note,
  .bytes {
    color: color-mix(in srgb, var(--filament) 55%, transparent);
  }
  .bar {
    height: 3px;
    background: var(--well);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--arc);
    box-shadow: 0 0 8px var(--arc);
    transition: width 120ms linear;
  }
  /* Before yt-dlp reports a total there is no honest percentage to draw, so
     the bar admits it rather than inventing one. */
  .fill.indeterminate {
    animation: pulse 1.1s ease-in-out infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 0.25; }
    50% { opacity: 0.9; }
  }
  @media (prefers-reduced-motion: reduce) {
    .fill.indeterminate { animation: none; opacity: 0.6; }
  }

  .error {
    margin: 0;
    padding: 10px 12px;
    font-size: 13px;
    color: var(--ember);
    border: 1px solid color-mix(in srgb, var(--ember) 45%, transparent);
    background: color-mix(in srgb, var(--ember) 8%, transparent);
    white-space: pre-wrap;
  }

  .tracks {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid color-mix(in srgb, var(--arc) 20%, transparent);
    background: var(--well);
  }
  .tracks li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--arc) 10%, transparent);
  }
  .tracks li:last-child { border-bottom: none; }
  .tracks li.current .title {
    color: var(--strike);
    text-shadow: 0 0 8px color-mix(in srgb, var(--strike) 45%, transparent);
  }
  .tracks li.empty {
    color: color-mix(in srgb, var(--filament) 45%, transparent);
    font-size: 13px;
  }
  .play {
    padding: 2px 8px;
    font-size: 11px;
  }
  .title {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    font-size: 12px;
    color: color-mix(in srgb, var(--filament) 45%, transparent);
    flex: 0 0 auto;
  }

  audio {
    width: 100%;
  }

  footer {
    display: flex;
    gap: 8px;
    align-items: baseline;
    font-size: 11px;
    color: color-mix(in srgb, var(--filament) 35%, transparent);
  }
  footer code {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
