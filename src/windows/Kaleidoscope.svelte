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
      // Each wedge is drawn symmetric about its own middle, so turning it into
      // place is the mirror. Flipping every other one as well folded each
      // petal onto its neighbour's and six arms read as three.
      for (let k = 0; k < segments; k++) {
        ctxt.save();
        ctxt.rotate(dir * turn + k * seg);
        wedge(ctxt, c.r, seg, L, bass, highs, overall, still, lead, second);
        ctxt.restore();
      }
      ctxt.restore();
    });

    const settling = !levels.settled();
    if (active || settling || bloom > 0) raf = requestAnimationFrame(draw);
  }

  /** One wedge, from the centre to the rim, between angle 0 and a segment,
   * symmetric about its middle; the loop above turns it into the rest. */
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
    const reach = R * (still ? 0.5 + 0.5 * overall : 0.4 + 0.6 * bass);
    const half = seg / 2;
    const at = (r: number, a: number): [number, number] => [r * Math.cos(a), r * Math.sin(a)];
    const RINGS = 4;
    const per = Math.floor(L.length / RINGS);

    for (let j = 0; j < RINGS; j++) {
      // A ring of the pattern per quarter of the spectrum, lows innermost.
      let lv = 0;
      for (let i = j * per; i < (j + 1) * per; i++) lv += L[i];
      lv = still ? overall : lv / per;
      const r0 = (reach * j) / RINGS;
      const r1 = (reach * (j + 1)) / RINGS;
      const rm = (r0 + r1) / 2;

      // The ring itself, as the arc of this wedge: six of them close a circle.
      g.globalAlpha = 0.25 + 0.45 * lv;
      g.strokeStyle = j % 2 === 0 ? second : lead;
      g.lineWidth = Math.max(1, R * 0.02 * (0.5 + 1.5 * lv));
      g.beginPath();
      g.arc(0, 0, r1, 0, seg);
      g.stroke();

      // A petal down the middle of the wedge, wider the louder its ring.
      const w = half * (still ? 0.55 : 0.25 + 0.7 * lv);
      g.globalAlpha = 0.45 + 0.5 * lv;
      g.fillStyle = j % 2 === 0 ? lead : second;
      g.beginPath();
      g.moveTo(...at(r0, half));
      g.quadraticCurveTo(...at(rm * 1.08, half - w), ...at(r1, half));
      g.quadraticCurveTo(...at(rm * 1.08, half + w), ...at(r0, half));
      g.fill();

      // A leaf on each edge of the wedge, which meets its neighbour's there:
      // the pair is the mirror across the seam.
      const e = half * (still ? 0.3 : 0.15 + 0.45 * lv);
      g.globalAlpha = 0.3 + 0.4 * lv;
      g.fillStyle = j % 2 === 0 ? second : lead;
      for (const [edge, s] of [
        [0, 1],
        [seg, -1],
      ] as const) {
        g.beginPath();
        g.moveTo(...at(rm, edge));
        g.quadraticCurveTo(...at(r1, edge + s * e * 0.5), ...at(r1 * 0.98, edge + s * e));
        g.quadraticCurveTo(...at(rm, edge + s * e * 0.6), ...at(rm, edge));
        g.fill();
      }

      // A jewel at the tip.
      g.globalAlpha = 0.6 + 0.4 * lv;
      g.fillStyle = lead;
      g.beginPath();
      g.arc(...at(r1, half), Math.max(0.75, R * 0.035 * (0.4 + lv)), 0, 2 * Math.PI);
      g.fill();
    }

    // The rim's detail, from the highs. None when still: detail that comes
    // and goes is motion.
    const dots = still ? 0 : Math.round(highs * 6);
    for (let d = 0; d < dots; d++) {
      g.globalAlpha = 0.7;
      g.fillStyle = second;
      g.beginPath();
      g.arc(...at(reach * 1.06, (half * 2 * (d + 1)) / (dots + 1)), Math.max(0.5, R * 0.018), 0, 2 * Math.PI);
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
