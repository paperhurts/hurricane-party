<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { emitTo, listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { applyTheme } from "./lib/theme";
  // The video window's "Loading…" moment (#62): waiting with the boombox,
  // a seagull on his head. Brief, but a home is a home.
  import waiting from "./assets/capybara-seagull-boombox.png";

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
  let video = $state<HTMLVideoElement | undefined>();

  /**
   * Mirror into Rust as the video transport (D70), the same call Main makes
   * for audio. Rust answers `status` from whichever kind last said it was
   * playing, so a video on screen is what the channel describes.
   */
  function report() {
    const v = video;
    invoke("report_state", {
      state: {
        state: !track || !v ? "stopped" : v.paused ? "paused" : "playing",
        kind: "video",
        media_id: track?.id ?? null,
        title: track?.title ?? null,
        uploader: track?.uploader ?? null,
        duration_s: v && Number.isFinite(v.duration) ? v.duration : (track?.duration_s ?? null),
        pos_s: v?.currentTime ?? 0,
        volume: v?.volume ?? 1,
      },
    }).catch(() => {});
  }

  /** A transport command Rust routed here because this window is the transport. */
  function onCommand(cmd: string, arg: unknown) {
    const v = video;
    if (!v) return;
    if (cmd === "play") v.play().catch(() => {});
    else if (cmd === "pause") v.pause();
    else if (cmd === "toggle") v.paused ? v.play().catch(() => {}) : v.pause();
    else if (cmd === "stop") {
      // Stop is pause and rewind; the window stays open on its first frame
      // (D69: nothing closes a window the user opened).
      v.pause();
      v.currentTime = 0;
      report();
    } else if (cmd === "seek") {
      v.currentTime = Number(arg) || 0;
      report();
    } else if (cmd === "volume") {
      v.volume = Math.min(1, Math.max(0, Number(arg)));
      report();
    }
  }

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

    // One transport (D69): the Main window starting a track pauses this one.
    // The window stays open on its frame; nothing resumes it.
    const pauseSub = listen("hp://pause", () => video?.pause());

    // Transport over the control pipe (D70). Targeted at this window: Rust
    // sends a command to whichever window is the transport, and a listener
    // with no target would also hear the ones meant for Main.
    const cmdSub = listen<{ cmd: string; arg: unknown }>(
      "control-command",
      (e) => onCommand(e.payload.cmd, e.payload.arg),
      { target: { kind: "WebviewWindow", label: "video" } },
    );

    return () => {
      sub.then((unlisten) => unlisten());
      pauseSub.then((unlisten) => unlisten()).catch(() => {});
      cmdSub.then((unlisten) => unlisten()).catch(() => {});
    };
  });
</script>

<main>
  {#if error}
    <p class="error">{error}</p>
  {:else if track}
    <!-- svelte-ignore a11y_media_has_caption -->
    <!-- A file that has moved used to leave this blank and silent (#43). The
         element reports it; it just has to be listened to. -->
    <video
      bind:this={video}
      src={convertFileSrc(track.path)}
      controls
      autoplay
      onplay={() => {
        // One transport (D69), the other way round: this window's own
        // controls resuming the video pauses the track in Main, not only the
        // library's click that opened it.
        emitTo("main", "player:pause").catch(() => {});
        report();
      }}
      onpause={report}
      ontimeupdate={report}
      onvolumechange={report}
      onerror={() => {
        error = "Can't open this file. Moved or deleted?";
        report();
      }}
      onended={() => {
        report();
        emitTo("library", "player:step", 1).catch(() => {});
      }}
    ></video>
    <footer>
      <span class="title">{track.title}</span>
      {#if track.uploader}<span class="by">{track.uploader}</span>{/if}
    </footer>
  {:else}
    <div class="wait">
      <img src={waiting} alt="" draggable="false" width="160" height="160">
      <p class="loading">Loading…</p>
    </div>
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
  /* Waiting: the capybara above the word, centred, briefly. */
  .wait { margin: auto; display: flex; flex-direction: column; align-items: center; gap: 4px; }
  .wait img { width: 160px; height: 160px; }
  .wait .loading { margin: 0; }
  .error { color: var(--ember); }
</style>
