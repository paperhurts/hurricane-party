<script lang="ts">
  // Eyewall's visualizer: spectrum bars on the 24-step radar reflectivity ramp
  // (D20, theme.md). The manifest names this component as `spectrum-bars`.
  //
  // Drawn on a canvas in PHYSICAL pixels. The window is 275 logical px and lives
  // on monitors at 100% and 150%, and a bar that is 4.5 device pixels wide is a
  // smear. So the canvas backing store is sized from the element's device-pixel
  // box and every rectangle is an integer in that space.
  //
  // No CSS filter on this element or any ancestor of it, ever (D73). Whatever
  // glow the analyser gets is painted into the ramp column below, once, and
  // blitted; nothing on the 60 Hz path is post-processed.
  import { untrack } from "svelte";
  import { bandEdges, Levels, reduceBands, type BandEdges } from "../lib/spectrum";

  let {
    analyser,
    palette,
    active,
    bars = 19,
  }: {
    analyser: AnalyserNode | null;
    /** Exactly 24 entries, low energy first. */
    palette: string[];
    /** Playing right now. The loop keeps running after this drops until the bars settle. */
    active: boolean;
    bars?: number;
  } = $props();

  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D | null = null;

  // Geometry, all physical px.
  let W = 0;
  let H = 0;
  let pitch = 0;
  let barW = 0;
  let x0 = 0;

  // One full-height column of the ramp, pre-rendered. Every bar is a clipped
  // blit of this, so a tall bar shows green through yellow and red to magenta
  // exactly like a radar return, and a short one stays green.
  let ramp: HTMLCanvasElement | null = null;

  let edges: BandEdges | null = null;
  let data: Uint8Array<ArrayBuffer> | null = null;
  let vals: Float32Array | null = null;
  let levels = new Levels(untrack(() => bars));
  let raf = 0;

  function layout(w: number, h: number) {
    W = w;
    H = h;
    canvas.width = W;
    canvas.height = H;
    pitch = Math.max(2, Math.floor(W / bars));
    // Roughly a quarter of the pitch is gap: 3+1 at 100%, 4+2 at 150%.
    const gap = Math.max(1, Math.round(pitch / 4));
    barW = pitch - gap;
    x0 = Math.floor((W - pitch * bars + gap) / 2);
    ramp = renderRamp();
    ctx = canvas.getContext("2d");
    if (ctx) ctx.imageSmoothingEnabled = false;
    if (!raf) draw();
  }

  function renderRamp(): HTMLCanvasElement | null {
    if (barW < 1 || H < 1 || palette.length === 0) return null;
    const c = document.createElement("canvas");
    c.width = barW;
    c.height = H;
    const g = c.getContext("2d");
    if (!g) return null;
    const steps = palette.length;
    for (let s = 0; s < steps; s++) {
      // Row band s counts from the bottom: index 0 is the floor of the display.
      const yTop = H - Math.round(((s + 1) * H) / steps);
      const yBot = H - Math.round((s * H) / steps);
      g.fillStyle = palette[s];
      g.fillRect(0, yTop, barW, yBot - yTop);
    }
    return c;
  }

  function prepare(a: AnalyserNode) {
    const bins = a.frequencyBinCount;
    edges = bandEdges(bars, bins, a.context.sampleRate);
    data = new Uint8Array(bins);
    vals = new Float32Array(bars);
    levels = new Levels(bars);
  }

  function draw() {
    raf = 0;
    if (!ctx) return;
    ctx.clearRect(0, 0, W, H);

    if (analyser && data && edges && vals) {
      if (active) {
        analyser.getByteFrequencyData(data);
        reduceBands(data, edges, vals);
      } else {
        vals.fill(0);
      }
      levels.step(vals);
    }

    if (ramp) {
      const peakColor = palette[palette.length - 1];
      for (let i = 0; i < bars; i++) {
        const x = x0 + i * pitch;
        const h = Math.round(levels.bars[i] * H);
        if (h > 0) {
          ctx.drawImage(ramp, 0, H - h, barW, h, x, H - h, barW, h);
        }
        const p = Math.round(levels.peaks[i] * H);
        if (p > 0) {
          ctx.fillStyle = peakColor;
          ctx.fillRect(x, Math.max(0, H - p - 1), barW, 1);
        }
      }
    }

    // Keep drawing while there is sound, and after it stops until the last bar
    // has fallen. Then go idle: a still display costs nothing.
    if (active || !levels.settled()) raf = requestAnimationFrame(draw);
  }

  $effect(() => {
    if (analyser) prepare(analyser);
  });

  $effect(() => {
    // Re-read so the effect tracks them; a change to either wakes the loop.
    void active;
    void analyser;
    if (!raf && ctx) raf = requestAnimationFrame(draw);
  });

  $effect(() => {
    // device-pixel-content-box is the whole reason for the ResizeObserver: it
    // reports the backing size in device pixels after the compositor has
    // snapped it, which is the number nothing else exposes. Fires again when
    // the window crosses to a monitor with a different scale factor.
    const ro = new ResizeObserver((entries) => {
      const box = entries[0]?.devicePixelContentBoxSize?.[0];
      if (box) {
        layout(box.inlineSize, box.blockSize);
      } else {
        const r = canvas.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        layout(Math.round(r.width * dpr), Math.round(r.height * dpr));
      }
    });
    try {
      ro.observe(canvas, { box: "device-pixel-content-box" });
    } catch {
      ro.observe(canvas);
    }
    return () => {
      ro.disconnect();
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
    };
  });
</script>

<canvas bind:this={canvas} class="vis"></canvas>

<style>
  .vis {
    display: block;
    width: 100%;
    height: 100%;
    background: var(--well);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--arc) 14%, transparent);
  }
</style>
