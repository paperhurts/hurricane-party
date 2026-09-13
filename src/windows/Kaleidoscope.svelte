<script lang="ts">
  // Purricane's visualizer (#147, docs/purricane.md): radial mirrored segments
  // driven by the same spectrum the bars read. Bass pushes the pattern out
  // from the centre, the highs add detail near the rim, a beat blooms, and the
  // colour drifts through the theme's palette over a minute and a half. The
  // turn is a slow constant that audio never touches. The manifest and the
  // tokens name this component as `kaleidoscope`.
  //
  // Still — calm, or prefers-reduced-motion — is a mandala that does not turn,
  // does not bloom and keeps one colour, and answers the music in size only.
  // Those clamps are `lib/kaleidoscope.ts`'s, and no theme or skin reaches them.
  //
  // Physical pixels and no CSS filter on this element or any ancestor (D73),
  // for the same reasons as the bars.
  import { untrack } from "svelte";
  import { BloomGate, bloomAt, centres, hueAt, mix, rotationAt } from "../lib/kaleidoscope";
  import { bandEdges, Levels, reduceBands, type BandEdges } from "../lib/spectrum";
  import type { Token } from "../lib/skin";

  let {
    analyser,
    colours,
    active,
    calm = false,
    segments = 6,
    degPerSec = 4,
    maxBloomHz = 3,
  }: {
    analyser: AnalyserNode | null;
    /** The six the window is wearing. */
    colours: Record<Token, string>;
    active: boolean;
    calm?: boolean;
    segments?: 6 | 8;
    degPerSec?: number;
    maxBloomHz?: number;
  } = $props();

  const BANDS = 24;

  let canvas: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D | null = null;
  let W = 0;
  let H = 0;
  let edges: BandEdges | null = null;
  let data: Uint8Array<ArrayBuffer> | null = null;
  let vals: Float32Array | null = null;
  let levels = new Levels(BANDS, { fall: 0.03 });
  let gate = new BloomGate(untrack(() => maxBloomHz));
  let bloomed = -Infinity;
  let raf = 0;

  // The OS's say, live: a person can turn reduced motion on with the music
  // playing.
  let reduced = $state(false);
  $effect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    reduced = mq.matches;
    const on = () => (reduced = mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  });

  $effect(() => {
    gate = new BloomGate(maxBloomHz);
  });

  function prepare(a: AnalyserNode) {
    const bins = a.frequencyBinCount;
    edges = bandEdges(BANDS, bins, a.context.sampleRate);
    data = new Uint8Array(bins);
    vals = new Float32Array(BANDS);
    levels = new Levels(BANDS, { fall: 0.03 });
  }

  const avg = (a: Float32Array, from: number, to: number) => {
    let s = 0;
    for (let i = from; i < to; i++) s += a[i];
    return s / (to - from);
  };

  function draw() {
    raf = 0;
    if (!ctx) return;
    const now = performance.now();
    const t = now / 1000;
    const still = calm || reduced;

    if (analyser && data && edges && vals) {
      if (active) {
        analyser.getByteFrequencyData(data);
        reduceBands(data, edges, vals);
      } else {
        vals.fill(0);
      }
    }
    levels.step(vals ?? new Float32Array(BANDS));
    const L = levels.bars;
    const bass = avg(L, 0, 5);
    const highs = avg(L, 15, BANDS);
    const overall = avg(L, 0, BANDS);
    if (gate.feed(bass, now, still)) bloomed = now;
    const bloom = still ? 0 : bloomAt(now, bloomed);

    ctx.clearRect(0, 0, W, H);
    const turn = rotationAt(t, degPerSec, still);
    const seg = (2 * Math.PI) / segments;
    const stops = [colours.accent, colours.alert, colours.warn];
    const lead = still ? mix(colours.accent, colours.accent, 0) : hueAt(t, stops);
    const second = still ? mix(colours.alert, colours.alert, 0) : hueAt(t + 30, stops);

    centres(W, H).forEach((c, m) => {
      const ctxt = ctx!;
      ctxt.save();
      ctxt.translate(c.x, c.y);
      const grow = 1 + bloom * 0.18;
      ctxt.scale(grow, grow);
      // Neighbours turn opposite ways, so the band reads as gears, not a belt.
      const dir = m % 2 === 0 ? 1 : -1;
      for (let k = 0; k < segments; k++) {
        ctxt.save();
        ctxt.rotate(dir * turn + k * seg);
        if (k % 2 === 1) ctxt.scale(1, -1);
        wedge(ctxt, c.r, seg, L, bass, highs, overall, still, lead, second);
        ctxt.restore();
      }
      ctxt.restore();
    });

    const settling = !levels.settled();
    if (active || settling || bloom > 0) raf = requestAnimationFrame(draw);
  }

  /** One mirrored wedge, from the centre to the rim, between angle 0 and
   * half a segment; the loop above turns and flips it into the rest. */
  function wedge(
    g: CanvasRenderingContext2D,
    R: number,
    seg: number,
    L: Float32Array,
    bass: number,
    highs: number,
    overall: number,
    still: boolean,
    lead: string,
    second: string,
  ) {
    // Bass pushes the pattern out; still, only the overall level does.
    const reach = R * (still ? 0.45 + 0.55 * overall : 0.35 + 0.65 * bass);
    const half = seg / 2;

    // The spoke.
    g.globalAlpha = 0.3 + 0.5 * (still ? overall : bass);
    g.strokeStyle = lead;
    g.lineWidth = Math.max(1, R * 0.03);
    g.beginPath();
    g.moveTo(0, 0);
    g.lineTo(reach * Math.cos(half), reach * Math.sin(half));
    g.stroke();

    // A petal per band, from the lows at the centre to the highs at the rim.
    for (let i = 0; i < L.length; i++) {
      const f = (i + 1) / L.length;
      const lv = still ? overall : L[i];
      const r = reach * f;
      const a = half * (still ? 0.5 : 0.2 + 0.6 * lv);
      const size = Math.max(0.75, R * 0.022 * (1 + 2.5 * lv));
      g.globalAlpha = 0.35 + 0.65 * lv;
      g.fillStyle = i % 2 === 0 ? lead : second;
      g.beginPath();
      g.arc(r * Math.cos(a), r * Math.sin(a), size, 0, 2 * Math.PI);
      g.fill();
    }

    // Detail near the rim, from the highs. None when still: detail that comes
    // and goes is motion.
    const dots = still ? 0 : Math.round(highs * 8);
    for (let d = 0; d < dots; d++) {
      const r = reach * (0.78 + (0.22 * d) / 8);
      const a = (half * (d + 1)) / (dots + 1);
      g.globalAlpha = 0.6;
      g.fillStyle = second;
      g.beginPath();
      g.arc(r * Math.cos(a), r * Math.sin(a), Math.max(0.5, R * 0.012), 0, 2 * Math.PI);
      g.fill();
    }
    g.globalAlpha = 1;
  }

  function layout(w: number, h: number) {
    W = w;
    H = h;
    canvas.width = W;
    canvas.height = H;
    ctx = canvas.getContext("2d");
    if (!raf) draw();
  }

  $effect(() => {
    if (analyser) prepare(analyser);
  });

  $effect(() => {
    // Any of these changing wakes the loop for at least a frame.
    void active;
    void analyser;
    void calm;
    void reduced;
    void colours;
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
    /* The same well as the bars (D122). */
    background: color-mix(in srgb, var(--surface) calc(var(--vis-well, 1) * 100%), transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 14%, transparent);
  }
</style>
