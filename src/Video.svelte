<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { applyTheme } from "./lib/theme";

  type MediaRow = {
    id: number;
    title: string;
    uploader: string | null;
    duration_s: number | null;
    path: string;
    kind: string;
  };

  let track = $state<MediaRow | null>(null);
  let error = $state<string | null>(null);
  // eslint-disable-next-line no-unused-vars -- bound for future transport control
  let video = $state<HTMLVideoElement | undefined>();

  onMount(async () => {
    applyTheme("eyewall");
    // The window is opened with ?id=N rather than being told over IPC, so a
    // reload reopens the same track instead of a blank window.
    const id = Number(new URLSearchParams(location.search).get("id"));
    if (!Number.isFinite(id) || id <= 0) {
      error = "No track specified.";
      return;
    }
    try {
      const rows = await invoke<MediaRow[]>("list_tracks");
      const found = rows.find((r) => r.id === id);
      if (!found) {
        error = "That track is no longer in the library.";
        return;
      }
      track = found;
    } catch (e) {
      error = String(e);
      return;
    }

    // Naming the window is cosmetic, so it gets its own catch. It used to sit
    // inside the try above, which meant a missing `core:window:allow-set-title`
    // permission set `error` — and the template renders the error INSTEAD of
    // the video. A decorative failure took the whole feature down.
    try {
      await getCurrentWindow().setTitle(track.title);
    } catch (e) {
      console.warn("couldn't set the window title:", e);
    }
  });
</script>

<main>
  {#if error}
    <p class="error">{error}</p>
  {:else if track}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} src={convertFileSrc(track.path)} controls autoplay></video>
    <footer>
      <span class="title">{track.title}</span>
      {#if track.uploader}<span class="by">{track.uploader}</span>{/if}
    </footer>
  {:else}
    <p class="loading">Loading…</p>
  {/if}
</main>

<style>
  /* The video window is decorated and resizable (D13) and is NOT part of the
     bond group, so it fills whatever size the OS gives it. */
  :global(body) { overflow: hidden; }
  main {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--well);
  }
  video {
    flex: 1 1 auto;
    min-height: 0;
    width: 100%;
    /* The letterbox bars around a video that doesn't fill the window follow the
       theme rather than being a literal black (D65). `well` is the deepest
       colour in the palette, so this still reads as black in practice. */
    background: var(--well);
  }
  footer {
    flex: 0 0 auto;
    display: flex;
    gap: 10px;
    align-items: baseline;
    padding: 6px 10px;
    background: var(--void);
    border-top: 1px solid color-mix(in srgb, var(--arc) 20%, transparent);
    font-size: 12px;
  }
  .title {
    color: var(--filament);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .by { color: color-mix(in srgb, var(--filament) 45%, transparent); }
  .error, .loading {
    margin: auto;
    font-size: 13px;
    color: color-mix(in srgb, var(--filament) 55%, transparent);
  }
  .error { color: var(--ember); }
</style>
