<script lang="ts">
  // The Main window owns playback. The <audio> element and the Web Audio graph
  // (D5) live here, so the analyser reads the sound that is actually coming
  // out; the library window is a remote that says "play this" and mirrors what
  // is playing. Transport arriving from the control pipe lands here too.
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { emit, emitTo, listen } from "@tauri-apps/api/event";
  import Classic from "./Classic.svelte";
  import SpectrumBars from "./SpectrumBars.svelte";
  import { AudioGraph } from "../lib/audio";
  import { loadEq, type EqState } from "../lib/eq";
  import { viscolor } from "../lib/theme";

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
  const palette = viscolor("eyewall");
  // The EQ window owns the sliders and the saved copy; this is the applied
  // copy. Same saved state at mount, then live updates over eq:set.
  let eq: EqState = loadEq(localStorage);

  let track = $state<Track | null>(null);
  // `playing` follows the element; `stopped` is the transport's own state,
  // because a paused track and a stopped one both have the element paused.
  let playing = $state(false);
  let stopped = $state(true);
  let pos = $state(0);
  let dur = $state(0);
  let vol = $state(1);
  let error = $state<string | null>(null);

  const mmss = (s: number) =>
    `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
  let elapsed = $derived(mmss(pos));
  let posPct = $derived(dur > 0 ? Math.min(100, (pos / dur) * 100) : 0);
  let title = $derived(
    track ? (track.uploader ? `${track.uploader} — ${track.title}` : track.title) : "hurricane-party",
  );
  // Scroll only when there is something to scroll and something happening.
  let marquee = $derived(playing && title.length > 34);

  // Built on first play rather than at mount: an AudioContext made before any
  // gesture starts suspended, and resume() is the same call either way.
  function ensureGraph() {
    if (!graph) {
      graph = new AudioGraph(audio);
      graph.applyEq(eq);
      analyser = graph.analyser;
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
      error = `Couldn't play: ${(e as Error).message}`;
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

  /** Tell every window what is playing, so the library's and the playlist's rows can light up. */
  function tell() {
    emit("player:now", { id: track?.id ?? null, playing }).catch(() => {});
  }

  /** Mirror into Rust so the control channel answers `status` truthfully. */
  function push() {
    invoke("report_state", {
      state: {
        state: !track || stopped ? "stopped" : audio.paused ? "paused" : "playing",
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
    const code = audio.error?.code;
    error =
      code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED
        ? "Can't open this file. Moved or deleted?"
        : `Playback failed (${audio.error?.message || code || "unknown"})`;
    playing = false;
    push();
    tell();
  }

  $effect(() => {
    // Say "stopped, volume 1" up front: until the first play nobody has
    // reported anything, and `status` over the pipe answered with blanks.
    push();
    const subs = [
      listen<Track>("player:load", (e) => load(e.payload)),
      // The library opened a video (D69), or clicked the row that is playing.
      listen("player:pause", () => pause()),
      listen("player:toggle", () => (audio.paused ? play() : pause())),
      listen<EqState>("eq:set", (e) => {
        eq = e.payload;
        graph?.applyEq(eq);
      }),
      listen<{ cmd: string; arg: unknown }>("control-command", (e) => {
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
      }),
    ];
    return () => {
      for (const s of subs) s.then((off) => off());
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

<Classic label="main" title="MAIN">
  <div class="player">
    <div class="top">
      <div class="clock">
        <div class="time">{elapsed}</div>
        <div class="tags">
          <span class:lit={playing}>PLAY</span>
          <span class:lit={!!track && !stopped && !playing}>PAUSE</span>
          <span class:lit={!!track && stopped} class="strike">STOP</span>
        </div>
      </div>
      <div class="visbox">
        <SpectrumBars {analyser} {palette} active={playing} />
      </div>
    </div>

    <div class="strip" class:err={!!error}>
      {#if error}
        <span class="static">{error}</span>
      {:else if marquee}
        <span class="scroll" style:animation-duration="{title.length * 0.35}s">
          {title}&nbsp;&nbsp;&nbsp;///&nbsp;&nbsp;&nbsp;{title}&nbsp;&nbsp;&nbsp;///&nbsp;&nbsp;&nbsp;
        </span>
      {:else}
        <span class="static">{title}</span>
      {/if}
    </div>

    <div class="seek" use:slider={seekTo}>
      <div class="fill" style:width="{posPct}%"></div>
      <div class="thumb" style:left="{posPct}%"></div>
    </div>

    <div class="bottom">
      <div class="transport">
        <button class="tb" onclick={() => step(-1)} title="Previous">◀◀</button>
        <button class="tb" class:lit={playing} onclick={play} title="Play">▶</button>
        <button
          class="tb"
          class:lit={!!track && !stopped && !playing}
          onclick={pause}
          title="Pause">‖</button
        >
        <button class="tb stop" class:lit={!!track && stopped} onclick={stop} title="Stop">■</button>
        <button class="tb" onclick={() => step(1)} title="Next">▶▶</button>
      </div>
      <div class="volume">
        <div class="vbar" use:slider={setVol}>
          <div class="fill" style:width="{vol * 100}%"></div>
        </div>
        <div class="vlabel">VOL {Math.round(vol * 100)}</div>
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
    /* The visualizer and everything above it: no filter, ever (D73). */
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
