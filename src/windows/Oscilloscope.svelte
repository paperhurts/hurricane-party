<script lang="ts">
  // The oscilloscope: the waveform, one column per physical pixel, drawn as
  // the classic did it, with a vertical stroke from the last sample's height
  // to this one's so a fast swing reads as a line and not a scatter of dots.
  // Coloured on the radar ramp by amplitude, like the bars: quiet is green,
  // a full swing is magenta.
  //
  // Same rules as SpectrumBars: physical pixels from the device-pixel box,
  // integer coordinates, no CSS filter on this or any ancestor (D73), and the
  // loop idles when nothing is playing.
  import { scopeColorIndex } from "../lib/vis";

  let {
    analyser,
    palette,
    active,
  }: {
    analyser: AnalyserNode | null;
    palette: string[];
    active: boolean;
  } = $props();

  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D | null = null;
  let W = 0;
  let H = 0;
  let data: Uint8Array<ArrayBuffer> | null = null;
  let raf = 0;
  // One frame of silence after `active` drops, so the trace clears.
  let settle = false;

  function layout(w: number, h: number) {
    W = w;
    H = h;
    canvas.width = W;
    canvas.height = H;
    ctx = canvas.getContext("2d");
    if (ctx) ctx.imageSmoothingEnabled = false;
    if (!raf) draw();
  }

  function draw() {
    raf = 0;
    if (!ctx || W === 0 || H === 0) return;
    ctx.clearRect(0, 0, W, H);

    const mid = Math.floor(H / 2);
    // The centre line is there even in silence: it says "scope", not "off".
    ctx.fillStyle = palette[0];
    ctx.fillRect(0, mid, W, 1);

    if (analyser && active) {
      data ??= new Uint8Array(analyser.fftSize);
      analyser.getByteTimeDomainData(data);
      const n = data.length;
      const half = mid - 1;
      let prevY = mid;
      for (let x = 0; x < W; x++) {
        // Nearest sample for this column; the buffer is wider than the display.
        const v = data[Math.floor((x * n) / W)];
        const y = mid - Math.round(((v - 128) / 128) * half);
        ctx.fillStyle = palette[scopeColorIndex(v, palette.length)];
        const top = Math.min(prevY, y);
        const len = Math.abs(y - prevY) + 1;
        ctx.fillRect(x, top, 1, len);
        prevY = y;
      }
      settle = true;
      raf = requestAnimationFrame(draw);
    } else if (settle) {
      // Drawn once more with no signal: the flat line. Then idle.
      settle = false;
    }
  }

  $effect(() => {
    void active;
    void analyser;
    if (!raf && ctx) raf = requestAnimationFrame(draw);
  });

  $effect(() => {
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
