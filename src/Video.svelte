<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
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

  // Show track `id`. Runs on mount for the ?id= the window was opened with,
  // and again for every switch after that (D68). `src` is reactive on `track`,
  // so assigning it swaps the source in place; nothing reloads.
  async function load(id: number) {
    if (!Number.isFinite(id) || id <= 0) {
      error = "No track specified.";
      return;
    }
    // Re-clicking the track already showing is not a switch. Returning here is
    // what keeps it playing from where it was instead of restarting (D68). A
    // failed switch since then may have left its message over the video, and
    // this click is the user asking for the track back.
    if (track?.id === id) {
      error = null;
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
      error = null;
    } catch (e) {
      error = String(e);
      return;
    }

    // The window is opened with ?id=N rather than being told over IPC, so a
    // reload reopens the same track instead of a blank window. A switch has to
    // keep that true, or F5 rewinds to whatever the window was opened with
    // (D68). Only after a successful load: a failed one leaves the URL on the
    // last track that worked.
    history.replaceState(null, "", `?id=${id}`);

    // Naming the window is cosmetic, so it gets its own catch. It used to sit
    // inside the try above, which meant a missing `core:window:allow-set-title`
    // permission set `error` — and the template renders the error INSTEAD of
    // the video. A decorative failure took the whole feature down.
    try {
      await getCurrentWindow().setTitle(track.title);
    } catch (e) {
      console.warn("couldn't set the window title:", e);
    }
  }

  onMount(() => {
    applyTheme("eyewall");
    const id = Number(new URLSearchParams(location.search).get("id"));

    // A switch arrives as an event and is acked with a command, because the
    // emit that carries it cannot tell Rust whether this window was alive to
    // receive it (D67). Ack after load whatever load decided: the ack means
    // "handled", and what the window shows is the window's business.
    const sub = listen<number>("hp://switch-track", async (e) => {
      await load(e.payload);
      await invoke("video_ready", { id: e.payload });
    });

    // The first load runs after the listener is registered, and acks the same
    // way. Rust holds every later click until this ack arrives, so this order
    // is what makes "up" mean "and listening" (D68). A listen that fails is
    // logged and the load still runs: the window can show its track even if
    // it can never be switched.
    sub
      .catch((e) => console.warn("couldn't listen for switches:", e))
      .then(async () => {
        await load(id);
        if (Number.isFinite(id)) await invoke("video_ready", { id });
      });

    return () => {
      sub.then((unlisten) => unlisten());
    };
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
