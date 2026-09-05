<script lang="ts">
  // The Main window owns playback. The <audio> element and the Web Audio graph
  // (D5) live here, so the analyser reads the sound that is actually coming
  // out; the library window is a remote that says "play this" and mirrors what
  // is playing. Transport arriving from the control pipe lands here too.
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { emit, emitTo, listen } from "@tauri-apps/api/event";
  import Classic from "./Classic.svelte";
  import SpectrumBars from "./SpectrumBars.svelte";
  import Oscilloscope from "./Oscilloscope.svelte";
  import { untrack } from "svelte";
  import { AudioGraph } from "../lib/audio";
  import { sourceFromAnalyser, VizCapture, type Demand } from "../lib/vizstream";
  import { loadEq, type EqState } from "../lib/eq";
  import { viscolor } from "../lib/theme";
  import { loadVisMode, nextVisMode, saveVisMode, type VisMode } from "../lib/vis";
  // The cooler capybara's home (#62): the display, while a file cannot be
  // opened. The analyser has nothing to draw then, and he has a drink.
  import cooler from "../assets/capybara-cooler.png";

  type Track = {
    id: number;
    title: string;
    uploader: string | null;
    duration_s: number | null;
    path: string;
    kind: string;
  };

  let audio: HTMLAudioElement;
  let graph: AudioGraph | null = null;
  let analyser = $state<AnalyserNode | null>(null);
  // The viz channel's source (control-api.md). Rust says when and how fast;
  // this reads the graph's unsmoothed tap and ships bytes, not JSON.
  const viz = new VizCapture((bins, headers) => invoke("viz_frame", bins, { headers }));
  const palette = viscolor("eyewall");
  // The EQ window owns the sliders and the saved copy; this is the applied
  // copy. Same saved state at mount, then live updates over eq:set.
  let eq: EqState = loadEq(localStorage);

  // Click the display to cycle bars, scope, off, as the classic did (D20:
  // the visualizer is a swappable component; the theme names the default).
  let visMode = $state<VisMode>(loadVisMode(localStorage));
  function cycleVis() {
    visMode = nextVisMode(visMode);
    saveVisMode(localStorage, visMode);
  }

  let track = $state<Track | null>(null);
  // `playing` follows the element; `stopped` is the transport's own state,
  // because a paused track and a stopped one both have the element paused.
  let playing = $state(false);
  let stopped = $state(true);
  let pos = $state(0);
  let dur = $state(0);
  let vol = $state(1);
  let error = $state<string | null>(null);
  // The error is the file not opening (#43), as opposed to the engine or the
  // element refusing. Only then is "remove it" the right offer (#78).
  let missing = $state(false);

  // Main is the one transport (D81). When the video window is the thing
  // playing, Rust mirrors its state here as `player:current` (the same state
  // the pipe's `status` reports, D70), and this window's clock, title, tags,
  // seek bar, volume and buttons follow it. `remote` is that mirror while the
  // video is the transport, and null while the track here is.
  type Remote = {
    state: string;
    kind: string;
    title?: string | null;
    uploader?: string | null;
    duration_s?: number | null;
    pos_s?: number | null;
    volume: number;
  };
  let remote = $state<Remote | null>(null);
  function absorbCurrent(s: Remote) {
    remote = s.kind === "video" ? s : null;
  }

  const mmss = (s: number) =>
    `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
  const nameOf = (t: { title?: string | null; uploader?: string | null } | null) =>
    t?.title ? (t.uploader ? `${t.uploader} — ${t.title}` : t.title) : "hurricane-party";

  // What the window shows: the video's state while it is the transport, this
  // element's otherwise. Everything below the fold reads these, never the raw
  // audio state directly.
  let uiPlaying = $derived(remote ? remote.state === "playing" : playing);
  let uiPaused = $derived(remote ? remote.state === "paused" : !!track && !stopped && !playing);
  let uiStopped = $derived(remote ? remote.state === "stopped" : !!track && stopped);
  let uiPos = $derived(remote ? (remote.pos_s ?? 0) : pos);
  let uiDur = $derived(remote ? (remote.duration_s ?? 0) : dur);
  let uiVol = $derived(remote ? remote.volume : vol);
  let elapsed = $derived(mmss(uiPos));
  let posPct = $derived(uiDur > 0 ? Math.min(100, (uiPos / uiDur) * 100) : 0);
  let title = $derived(remote ? nameOf(remote) : nameOf(track));
  // Scroll only when there is something to scroll and something happening.
  let marquee = $derived(uiPlaying && title.length > 34);
  // The shade's line is narrower: three buttons and the clock share it.
  let shadeMarquee = $derived(uiPlaying && title.length > 28);
  // A control in the title strip: neither a drag nor a double-tap (Classic).
  const eat = (e: Event) => e.stopPropagation();

  // Built on first play rather than at mount: an AudioContext made before any
  // gesture starts suspended, and resume() is the same call either way.
  function ensureGraph() {
    if (!graph) {
      graph = new AudioGraph(audio);
      graph.applyEq(eq);
      analyser = graph.analyser;
      viz.source = sourceFromAnalyser(graph.stream);
    }
    graph.resume().catch((e) => (error = `Audio engine: ${String(e)}`));
    // A resume the browser refuses (no gesture in this document yet) leaves
    // the promise pending rather than rejecting it, so the context has to be
    // asked directly. Say so instead of playing silence.
    setTimeout(() => {
      if (graph && graph.ctx.state !== "running" && !audio.paused) {
        error = "Audio engine is suspended. Click Play in this window once.";
      }
    }, 400);
  }

  async function load(t: Track) {
    error = null;
    missing = false;
    // Re-clicking the track already loaded is "play", not "start over" (the
    // same rule the video window keeps, D68).
    if (track?.id === t.id && !stopped) {
      await play();
      return;
    }
    track = t;
    stopped = false;
    pos = 0;
    dur = t.duration_s ?? 0;
    // Come forward so the user sees it start, without taking their focus.
    invoke("wm_raise", { label: "main" }).catch(() => {});
    ensureGraph();
    audio.src = convertFileSrc(t.path);
    // One transport (D69): starting a track pauses a video that is playing.
    // Nothing resumes it; the video window stays on its paused frame.
    emitTo("video", "hp://pause").catch(() => {});
    await play();
    tell();
  }

  async function play() {
    if (!track) return;
    stopped = false;
    ensureGraph();
    try {
      await audio.play();
    } catch (e) {
      // The element's own verdict (onError: "moved or deleted?") is the
      // specific one, and it is racing this rejection for the same strip. When
      // the element holds an error, it has said or is about to say what went
      // wrong; this generic line is for the refusals that never reach it.
      if (!audio.error) error = `Couldn't play: ${(e as Error).message}`;
    }
    push();
  }

  function pause() {
    audio.pause();
    push();
  }

  function stop() {
    audio.pause();
    audio.currentTime = 0;
    pos = 0;
    stopped = true;
    push();
  }

  // The play order lives with the library, which knows which list is showing.
  function step(delta: number) {
    emitTo("library", "player:step", delta).catch(() => {});
  }

  function seekTo(frac: number) {
    if (dur <= 0) return;
    audio.currentTime = frac * dur;
    pos = audio.currentTime;
    push();
  }

  function setVol(frac: number) {
    vol = Math.min(1, Math.max(0, frac));
    audio.volume = vol;
    push();
  }

  // The buttons, seek bar and volume act on whatever is playing (D81). With
  // the video as the transport they go through Rust's router, the same one
  // the pipe uses, so a press here and a `pause` over the pipe are the same
  // thing; otherwise they act on the element here directly.
  function remoteCmd(cmd: string, arg?: number) {
    invoke("transport", { cmd, arg: arg ?? null }).catch(() => {});
  }
  const uiPlay = () => (remote ? remoteCmd("play") : play());
  const uiPause = () => (remote ? remoteCmd("pause") : pause());
  const uiToggle = () => (remote ? remoteCmd("toggle") : audio.paused ? play() : pause());
  const uiStop = () => (remote ? remoteCmd("stop") : stop());
  const uiSeek = (frac: number) =>
    remote ? remoteCmd("seek", frac * (remote.duration_s ?? 0)) : seekTo(frac);
  const uiVolume = (frac: number) => (remote ? remoteCmd("volume", frac) : setVol(frac));

  /** Tell every window what is playing, so the library's and the playlist's rows can light up. */
  function tell() {
    emit("player:now", { id: track?.id ?? null, playing }).catch(() => {});
  }

  /** Mirror into Rust so the control channel answers `status` truthfully. */
  function push() {
    invoke("report_state", {
      state: {
        state: !track || stopped ? "stopped" : audio.paused ? "paused" : "playing",
        kind: "audio",
        media_id: track?.id ?? null,
        title: track?.title ?? null,
        uploader: track?.uploader ?? null,
        duration_s: dur > 0 ? dur : null,
        pos_s: pos,
        volume: vol,
      },
    }).catch(() => {});
  }

  // A file that has gone missing used to play as silence with no message (#43).
  // The element does say so, in the error event; it just has to be listened to.
  function onError() {
    // Letting go of a track (unload) empties the element, and an empty
    // element has nothing to fail on.
    if (!track) return;
    const code = audio.error?.code;
    missing = code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED;
    error = missing
      ? "Can't open this file. Moved or deleted?"
      : `Playback failed (${audio.error?.message || code || "unknown"})`;
    playing = false;
    push();
    tell();
    // The library is where the click came from and where the row is; it
    // shows the same words with the way out beside them (#78).
    if (missing) {
      emitTo("library", "player:missing", { id: track.id, title: track.title }).catch(() => {});
    }
  }

  /** Let go of the track: the library removed it (#78), here or from its own window. */
  function unload() {
    audio.pause();
    audio.removeAttribute("src");
    audio.load();
    track = null;
    error = null;
    missing = false;
    stopped = true;
    playing = false;
    pos = 0;
    dur = 0;
    push();
    tell();
  }

  function removeMissing() {
    if (!track) return;
    invoke("remove_from_library", { id: track.id })
      .then(unload)
      .catch((e) => (error = String(e)));
  }

  $effect(() => {
    // Say "stopped, volume 1" up front: until the first play nobody has
    // reported anything, and `status` over the pipe answered with blanks.
    // Untracked: push() reads pos, vol and the rest, and without this the
    // effect re-ran on every timeupdate, tearing down and re-subscribing
    // every listener below four times a second. timeupdate pushes itself.
    untrack(push);
    // What the channel says is playing (D81): pulled once here, pushed on
    // every change after. A reload of this window while a video plays must
    // come back showing the video, not a blank clock.
    invoke<Remote>("transport_state").then(absorbCurrent).catch(() => {});
    const subs = [
      listen<Remote>("player:current", (e) => absorbCurrent(e.payload), {
        target: { kind: "WebviewWindow", label: "main" },
      }),
      listen<Track>("player:load", (e) => load(e.payload)),
      // The library removed a row (#78); if it is the one loaded here, let go.
      listen<number>("player:removed", (e) => {
        if (track?.id === e.payload) unload();
      }, { target: { kind: "WebviewWindow", label: "main" } }),
      // The library opened a video (D69), or clicked the row that is playing.
      listen("player:pause", () => pause()),
      listen("player:toggle", () => (audio.paused ? play() : pause())),
      listen<EqState>("eq:set", (e) => {
        eq = e.payload;
        graph?.applyEq(eq);
      }),
      // Targeted at this window: Rust routes a transport command to whichever
      // window is the transport (D70), and a listener with no target would
      // also receive the ones aimed at the video window.
      listen<{ cmd: string; arg: unknown }>(
        "control-command",
        (e) => {
          const { cmd, arg } = e.payload;
          if (cmd === "play") play();
          else if (cmd === "pause") pause();
          else if (cmd === "toggle") audio.paused ? play() : pause();
          else if (cmd === "stop") stop();
          else if (cmd === "next") step(1);
          else if (cmd === "prev") step(-1);
          else if (cmd === "seek") {
            audio.currentTime = Number(arg) || 0;
            pos = audio.currentTime;
            push();
          } else if (cmd === "volume") setVol(Number(arg));
        },
        { target: { kind: "WebviewWindow", label: "main" } },
      ),
    ];
    return () => {
      for (const s of subs) s.then((off) => off());
    };
  });

  // The viz channel's source loop follows Rust's demand. Its own effect,
  // reading no state, so nothing in this window can restart the timer: a
  // restart defers the next frame by a whole period, and that showed up as a
  // 67 ms gap four times a second when it lived in the effect above.
  $effect(() => {
    // Ask whether a rig is already listening: a reload of this window must
    // not silently end a stream Rust still has subscribers for.
    invoke<Demand>("viz_demand")
      .then((d) => viz.apply(d))
      .catch(() => {});
    const off = listen<Demand>("viz:capture", (e) => viz.apply(e.payload));
    return () => {
      viz.stop();
      off.then((f) => f());
    };
  });

  // The clip check (D21): while sound plays, look at each block that reached
  // the analyser and tell the EQ window when one went past the ceiling.
  // Rate-limited so a sustained clip is a few events a second, not sixty.
  let clipBuf: Float32Array<ArrayBuffer> | null = null;
  let clipRaf = 0;
  let lastClipAt = 0;

  function clipLoop() {
    clipRaf = 0;
    if (!graph || !playing) return;
    clipBuf ??= new Float32Array(graph.analyser.fftSize);
    if (graph.clipping(clipBuf)) {
      const now = performance.now();
      if (now - lastClipAt > 120) {
        lastClipAt = now;
        emitTo("eq", "eq:clip", true).catch(() => {});
      }
    }
    clipRaf = requestAnimationFrame(clipLoop);
  }

  $effect(() => {
    if (playing && !clipRaf) clipRaf = requestAnimationFrame(clipLoop);
    return () => {
      if (clipRaf) cancelAnimationFrame(clipRaf);
      clipRaf = 0;
    };
  });

  // A horizontal slider: press or drag anywhere on the node to set 0..1.
  function slider(node: HTMLElement, set: (frac: number) => void) {
    const at = (e: PointerEvent) => {
      const r = node.getBoundingClientRect();
      set(Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)));
    };
    const move = (e: PointerEvent) => at(e);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    const down = (e: PointerEvent) => {
      if (e.button !== 0) return;
      at(e);
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    };
    node.addEventListener("pointerdown", down);
    return {
      destroy() {
        node.removeEventListener("pointerdown", down);
        up();
      },
    };
  }
</script>

<!-- The windowshade: the always-on-top mini-player, "the state you'll live
     in" (theme.md, D79). Transport, the title, the clock, in fourteen pixels. -->
{#snippet shade()}
  <div class="shade">
    <button class="sb" onpointerdown={eat} onclick={() => step(-1)} title="Previous">◀◀</button>
    <button
      class="sb"
      class:lit={uiPlaying}
      onpointerdown={eat}
      onclick={uiToggle}
      title={uiPlaying ? "Pause" : "Play"}>{uiPlaying ? "‖" : "▶"}</button
    >
    <button class="sb" onpointerdown={eat} onclick={() => step(1)} title="Next">▶▶</button>
    <span class="sep"></span>
    <span class="stext" class:err={!!error}>
      {#if error}
        {error}
      {:else if shadeMarquee}
        <span class="scroll" style:animation-duration="{title.length * 0.35}s">
          {title}&nbsp;&nbsp;&nbsp;///&nbsp;&nbsp;&nbsp;{title}&nbsp;&nbsp;&nbsp;///&nbsp;&nbsp;&nbsp;
        </span>
      {:else}
        {title}
      {/if}
    </span>
    <span class="stime">{elapsed}</span>
  </div>
{/snippet}

<Classic label="main" title="MAIN" {shade}>
  <div class="player">
    <div class="top">
      <div class="clock">
        <div class="time">{elapsed}</div>
        <div class="tags">
          <span class:lit={uiPlaying}>PLAY</span>
          <span class:lit={uiPaused}>PAUSE</span>
          <span class:lit={uiStopped} class="strike">STOP</span>
        </div>
      </div>
      <div
        class="visbox"
        class:off={visMode === "off"}
        role="button"
        tabindex="0"
        onclick={cycleVis}
        onkeydown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            cycleVis();
          }
        }}
        title={visMode === "bars" ? "Spectrum. Click for scope" : visMode === "scope" ? "Scope. Click for off" : "Off. Click for spectrum"}
      >
        {#if error}
          <img class="oops" src={cooler} alt="" draggable="false">
        {:else if visMode === "bars"}
          <SpectrumBars {analyser} {palette} active={playing} />
        {:else if visMode === "scope"}
          <Oscilloscope {analyser} {palette} active={playing} />
        {/if}
      </div>
    </div>

    <div class="strip" class:err={!!error} title={error ?? undefined}>
      {#if error}
        <span class="static">{error}</span>
        {#if missing && track}
          <!-- The way out, on the message itself (#78). The file is gone;
               there is nothing to offer to delete. -->
          <button class="act" onpointerdown={eat} onclick={removeMissing} title="Remove from the library">remove</button>
        {/if}
      {:else if marquee}
        <span class="scroll" style:animation-duration="{title.length * 0.35}s">
          {title}&nbsp;&nbsp;&nbsp;///&nbsp;&nbsp;&nbsp;{title}&nbsp;&nbsp;&nbsp;///&nbsp;&nbsp;&nbsp;
        </span>
      {:else}
        <span class="static">{title}</span>
      {/if}
    </div>

    <div class="seek" use:slider={uiSeek}>
      <div class="fill" style:width="{posPct}%"></div>
      <div class="thumb" style:left="{posPct}%"></div>
    </div>

    <div class="bottom">
      <div class="transport">
        <button class="tb" onclick={() => step(-1)} title="Previous">◀◀</button>
        <button class="tb" class:lit={uiPlaying} onclick={uiPlay} title="Play">▶</button>
        <button class="tb" class:lit={uiPaused} onclick={uiPause} title="Pause">‖</button>
        <button class="tb stop" class:lit={uiStopped} onclick={uiStop} title="Stop">■</button>
        <button class="tb" onclick={() => step(1)} title="Next">▶▶</button>
      </div>
      <div class="volume">
        <div class="vbar" use:slider={uiVolume}>
          <div class="fill" style:width="{uiVol * 100}%"></div>
        </div>
        <div class="vlabel">VOL {Math.round(uiVol * 100)}</div>
      </div>
    </div>

    <!-- crossorigin is load-bearing. The file comes from the asset protocol,
         which is another origin, and a media element inside a Web Audio graph
         outputs SILENCE for a cross-origin resource fetched without CORS: the
         element plays, the clock runs, and nothing reaches the speakers or
         the analyser. Tauri's asset handler answers with Access-Control-Allow-
         Origin for the app's origin, so anonymous mode is enough. -->
    <!-- svelte-ignore a11y_media_has_caption -->
    <audio
      bind:this={audio}
      crossorigin="anonymous"
      hidden
      onplay={() => {
        playing = true;
        // One transport (D69), on the element's own event so it holds for
        // every way of starting: this window's button, the pipe, a key. The
        // library's row click already did this; this window's play did not,
        // and a video kept running under a resumed track.
        emitTo("video", "hp://pause").catch(() => {});
        push();
        tell();
      }}
      onpause={() => {
        playing = false;
        push();
        tell();
      }}
      onended={() => step(1)}
      ontimeupdate={() => {
        pos = audio.currentTime;
        push();
      }}
      ondurationchange={() => {
        if (Number.isFinite(audio.duration)) dur = audio.duration;
      }}
      onerror={onError}
    ></audio>
  </div>
</Classic>

<style>
  .player {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px 4px 3px;
    font-size: 8px;
    letter-spacing: 0.06em;
  }

  /* ---- top row: clock and the analyser ---- */
  .top {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    gap: 5px;
    align-items: stretch;
  }
  .clock {
    flex: 0 0 56px;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: 3px;
  }
  .time {
    font-size: 17px;
    line-height: 17px;
    letter-spacing: -0.03em;
    color: var(--arc);
    /* Static glow on a static element: this is not on the 60 Hz path and
     * never animates (theme.md). */
    text-shadow:
      0 0 7px color-mix(in srgb, var(--arc) 65%, transparent),
      0 0 18px color-mix(in srgb, var(--arc) 25%, transparent);
  }
  .tags {
    display: flex;
    gap: 4px;
    font-size: 6px;
    letter-spacing: 0.1em;
    color: color-mix(in srgb, var(--filament) 35%, transparent);
  }
  .tags .lit {
    color: var(--arc);
  }
  .tags .strike.lit {
    color: var(--strike);
  }
  .visbox {
    flex: 1 1 auto;
    min-width: 0;
    cursor: pointer;
    /* The visualizer and everything above it: no filter, ever (D73). */
  }
  /* A file that cannot be opened: the cooler capybara where the bars were,
   * the well behind him, the message in the strip below. */
  .visbox .oops {
    display: block;
    height: 100%;
    width: 100%;
    object-fit: contain;
    object-position: center bottom;
    padding: 2px 0 0;
    box-sizing: border-box;
    background: var(--well);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ember) 30%, transparent);
  }
  /* Off: the well, and nothing drawing into it. */
  .visbox.off {
    background: var(--well);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 14%, transparent);
  }

  /* ---- title strip ---- */
  .strip {
    flex: 0 0 17px;
    display: flex;
    align-items: center;
    padding: 0 4px;
    overflow: hidden;
    white-space: nowrap;
    font-size: 11px;
    letter-spacing: 0;
    color: var(--filament);
    background: var(--well);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 14%, transparent);
  }
  .strip.err {
    color: var(--ember);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ember) 45%, transparent);
  }
  .static {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* The action on the message: a word in the strip's own type, boxed so it
     reads as a control, ember like the state it belongs to. The message
     yields to it rather than the other way round. */
  .act {
    flex: 0 0 auto;
    margin-left: auto;
    padding: 0 4px;
    font: inherit;
    font-size: 9px;
    letter-spacing: 1px;
    text-transform: uppercase;
    line-height: 13px;
    color: var(--ember);
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--ember) 55%, transparent);
    cursor: pointer;
  }
  .act:hover {
    background: color-mix(in srgb, var(--ember) 18%, transparent);
    border-color: var(--ember);
  }
  .scroll {
    display: inline-block;
    animation: marquee linear infinite;
  }
  @keyframes marquee {
    from {
      transform: translateX(0);
    }
    to {
      transform: translateX(-50%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .scroll {
      animation: none;
    }
  }

  /* ---- seek ---- */
  .seek {
    flex: 0 0 9px;
    position: relative;
    background: var(--well);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 14%, transparent);
    cursor: pointer;
  }
  .seek .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--arc) 25%, transparent),
      color-mix(in srgb, var(--arc) 75%, transparent)
    );
  }
  .seek .thumb {
    position: absolute;
    top: -1px;
    bottom: -1px;
    width: 3px;
    margin-left: -1px;
    background: var(--arc);
    box-shadow: 0 0 6px color-mix(in srgb, var(--arc) 95%, transparent);
  }

  /* ---- transport and volume ---- */
  .bottom {
    flex: 0 0 16px;
    display: flex;
    gap: 5px;
    align-items: stretch;
  }
  .transport {
    display: flex;
    gap: 1px;
  }
  .tb {
    width: 17px;
    height: 14px;
    padding: 0;
    border: 0;
    display: grid;
    place-items: center;
    font-size: 6px;
    line-height: 1;
    color: color-mix(in srgb, var(--filament) 80%, transparent);
    background: color-mix(in srgb, var(--void) 70%, var(--well));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 22%, transparent);
    cursor: pointer;
  }
  .tb:hover:not(:disabled) {
    color: var(--arc);
    background: color-mix(in srgb, var(--void) 70%, var(--well));
    box-shadow: inset 0 0 0 1px var(--arc);
  }
  .tb.lit {
    color: var(--arc);
    background: color-mix(in srgb, var(--arc) 14%, var(--void));
    box-shadow:
      inset 0 0 0 1px var(--arc),
      0 0 8px color-mix(in srgb, var(--arc) 45%, transparent);
  }
  .tb.stop.lit {
    color: var(--strike);
    background: color-mix(in srgb, var(--strike) 14%, var(--void));
    box-shadow:
      inset 0 0 0 1px var(--strike),
      0 0 8px color-mix(in srgb, var(--strike) 45%, transparent);
  }
  .volume {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 3px;
  }
  .vbar {
    height: 5px;
    position: relative;
    background: var(--well);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 14%, transparent);
    cursor: pointer;
  }
  .vbar .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--arc);
    box-shadow: 0 0 5px color-mix(in srgb, var(--arc) 70%, transparent);
  }
  .vlabel {
    font-size: 6px;
    letter-spacing: 0.08em;
    color: color-mix(in srgb, var(--filament) 40%, transparent);
  }
</style>
