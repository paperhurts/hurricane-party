<script lang="ts">
  // Purricane's visualizer (#147, docs/purricane.md), drawn as the designer
  // drew it (design/screens/Kaleidoscope, D132): mirrored petals round a
  // bright centre, each segment a step further round the wheel, a glow on
  // every petal and dots at the rim. Bass lengthens the petals and swells the
  // centre, the mids widen them, the highs add dots, a beat blooms, and the
  // hue drifts once round the palette's ramp in a minute and a half. The turn
  // is a slow constant that audio never touches. The manifest and the tokens
  // name this component as `kaleidoscope`.
  //
  // What the designer's was not is slow: it turned at twenty degrees a second
  // and bloomed on any bass over a fixed line. The turn and the bloom are
  // `lib/kaleidoscope.ts`'s clamps, which no theme or skin reaches. Still —
  // calm, or prefers-reduced-motion — is a mandala that does not turn, does
  // not bloom and keeps one hue, and answers the music in size only.
  //
  // Physical pixels and no CSS filter on this element or any ancestor (D73),
  // for the same reasons as the bars. The petals' glow is the canvas's own.
  import { untrack } from "svelte";
  import { BloomGate, bloomAt, centres, driftHue, HUE_PERIOD_S, hueOf, rotationAt } from "../lib/kaleidoscope";
  import { bandEdges, Levels, reduceBands, type BandEdges } from "../lib/spectrum";

  let {
    analyser,
    ramp,
    active,
    calm = false,
    segments = 6,
    degPerSec = 4,
    maxBloomHz = 3,
  }: {
    analyser: AnalyserNode | null;
    /** The ramp the window wears, whose hues the pattern drifts through. */
    ramp: string[];
    active: boolean;
    calm?: boolean;
    segments?: 6 | 8;
    degPerSec?: number;
    maxBloomHz?: number;
  } = $props();

  const BANDS = 24;
  /** Where still holds the drift: a quarter of the way round, where the
   * designer's calm held it. */
  const HOLD = 0.25;

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
  let hues = $derived(ramp.map(hueOf));

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

  /** A glow at `hue`, at the designer's saturation and lightness. The hue is
   * the palette's; how bright a petal is, is this component's. */
  const hsla = (hue: number, s: number, l: number, a: number) =>
    `hsla(${((hue % 360) + 360) % 360}, ${s}%, ${l}%, ${a})`;

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
    const overall = avg(L, 0, BANDS);
    // Still, every part follows the one overall level, so the size answers
    // the music and nothing else does: no dots coming and going.
    const bass = still ? overall : avg(L, 0, 4);
    const mids = still ? overall : L[6];
    const inner = still ? overall : L[10];
    const treble = still ? 0 : (L[17] + L[19] + L[21] + L[23]) / 4;
    if (gate.feed(avg(L, 0, 4), now, still)) bloomed = now;
    const bloom = still ? 0 : bloomAt(now, bloomed);

    ctx.clearRect(0, 0, W, H);
    const turn = rotationAt(t, degPerSec, still);
    const hue = driftHue(still ? HOLD * HUE_PERIOD_S : t, hues);
    const seg = (2 * Math.PI) / segments;

    centres(W, H).forEach((c, m) => {
      const g = ctx!;
      const R = c.r;
      g.save();
      g.translate(c.x, c.y);
      g.scale(1 + bloom * 0.11, 1 + bloom * 0.11);
      // Neighbours in a band turn opposite ways, so it reads as gears.
      g.rotate((m % 2 === 0 ? 1 : -1) * turn);
      for (let k = 0; k < segments; k++) {
        g.save();
        g.rotate(k * seg);
        // Every other petal mirrored: the kaleidoscope's mirror.
        if (k % 2) g.scale(1, -1);
        petal(g, R, hue + k * (180 / segments), bass, mids, inner, treble, bloom);
        g.restore();
      }
      // The bright centre, swelling with the bass.
      const cr = R * (0.22 + bass * 0.12);
      const glow = g.createRadialGradient(0, 0, 0, 0, 0, cr);
      glow.addColorStop(0, hsla(hue, 100, 98, 0.95));
      glow.addColorStop(0.55, hsla(hue, 95, 82, 0.75));
      glow.addColorStop(1, hsla(hue, 95, 75, 0));
      g.beginPath();
      g.arc(0, 0, cr, 0, 2 * Math.PI);
      g.fillStyle = glow;
      g.fill();
      g.restore();
    });

    const settling = !levels.settled();
    if (active || settling || bloom > 0) raf = requestAnimationFrame(draw);
  }

  /** One segment's petal, its inner petal and its rim dots, pointing down the
   * y axis from the centre; the loop above turns and mirrors it into the rest. */
  function petal(
    g: CanvasRenderingContext2D,
    R: number,
    hs: number,
    bass: number,
    mids: number,
    inner: number,
    treble: number,
    bloom: number,
  ) {
    const reach = R * (0.34 + bass * 0.58);
    const wide = R * (0.09 + mids * 0.16);
    g.shadowBlur = R * 0.22;
    g.shadowColor = hsla(hs, 95, 72, 0.5 + bloom * 0.4);
    g.beginPath();
    g.moveTo(0, 0);
    g.quadraticCurveTo(wide, reach * 0.45, 0, reach);
    g.quadraticCurveTo(-wide * 0.6, reach * 0.4, 0, 0);
    g.fillStyle = hsla(hs, 92, 72 - bass * 8, 0.62);
    g.fill();
    g.shadowBlur = 0;
    g.lineWidth = Math.max(1, R / 17);
    g.strokeStyle = hsla(hs, 95, 52, 0.85);
    g.stroke();

    const short = R * (0.16 + inner * 0.2);
    g.beginPath();
    g.moveTo(0, 0);
    g.quadraticCurveTo(wide * 1.5, short * 0.5, 0, short);
    g.quadraticCurveTo(-wide * 1.1, short * 0.45, 0, 0);
    g.fillStyle = hsla(hs + 40, 95, 80, 0.7);
    g.fill();

    // The rim: four dots, and more with the highs.
    const dots = Math.round(4 + treble * 14);
    const across = (2 * Math.PI) / segments;
    for (let d = 0; d < dots; d++) {
      const a = (-0.3 + 0.6 * (d / Math.max(1, dots - 1))) * across;
      const rr = R * (0.8 + 0.16 * ((d % 3) / 2));
      g.beginPath();
      g.arc(Math.sin(a) * rr, Math.cos(a) * rr, Math.max(0.7, R * (0.012 + treble * 0.016)), 0, 2 * Math.PI);
      g.fillStyle = d % 2 ? hsla(hs + 80, 95, 70, 0.9) : hsla(hs + 200, 90, 72, 0.9);
      g.fill();
    }
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
    void hues;
    void segments;
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
    /* A round badge is a circle (D132), and so is its well. */
    border-radius: var(--vis-radius, 0);
  }
</style>
