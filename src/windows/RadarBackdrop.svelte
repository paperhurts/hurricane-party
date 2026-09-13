<script lang="ts">
  // Cone's backdrop (#85, D135): the radar loop behind a classic window's
  // chrome. The three windows share one picture, each showing its own band of
  // it in their classic order, like a made skin's picture (D127), so stacked
  // they read as one map with the radar in the middle.
  //
  // Drawn on a canvas under every sprite. The frames are Rust's, already in
  // the analyser's ramp; this draws them softened, rings the radar's reach,
  // lays the window's ground over them so the chrome stays readable, and, when
  // the loop is stale or the fetch failed, draws them grey and dim. That is a
  // canvas filter, inside the canvas: no CSS filter touches anything here, and
  // the analyser is not under this element (D73).
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { frameAt, stackTop, type RadarStatus } from "../lib/radar";
  import type { WindowName } from "../lib/skin";

  let {
    status,
    window: win,
    warn,
  }: {
    status: RadarStatus | null;
    window: WindowName;
    /** Stale or offline: drawn so it cannot be mistaken for now. */
    warn: boolean;
  } = $props();

  let canvas: HTMLCanvasElement;
  let W = 0;
  let H = 0;
  let dpr = 1;
  let raf = 0;
  let shown = -2;
  let images: HTMLImageElement[] = [];

  let reduced = $state(false);
  $effect(() => {
    const mq = globalThis.matchMedia("(prefers-reduced-motion: reduce)");
    reduced = mq.matches;
    const on = () => (reduced = mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  });

  // The frames as images, loaded once each. A frame that has not loaded yet
  // is skipped rather than drawn blank.
  $effect(() => {
    const paths = status?.frames.map((f) => f.path) ?? [];
    images = paths.map((p) => {
      const img = new Image();
      img.decoding = "async";
      img.onload = () => {
        shown = -2;
        wake();
      };
      img.src = convertFileSrc(p);
      return img;
    });
    shown = -2;
    wake();
  });

  $effect(() => {
    void warn;
    void reduced;
    shown = -2;
    wake();
  });

  function token(name: string): string {
    return getComputedStyle(document.documentElement).getPropertyValue(`--${name}`).trim();
  }

  function draw() {
    raf = 0;
    const ctx = canvas?.getContext("2d");
    if (!ctx || W === 0) return;
    const count = images.length;
    // Stale or offline still loops, grey: the last hours the radar saw,
    // looping, is the point of Cone (theme.md). Only reduced motion holds it.
    const i = frameAt(Date.now(), count, reduced);
    if (i !== shown) {
      shown = i;
      paint(ctx, i);
    }
    // Only a moving loop needs another frame, and only while it is seen.
    if (count > 1 && !reduced && !document.hidden) raf = requestAnimationFrame(draw);
  }

  function paint(ctx: CanvasRenderingContext2D, i: number) {
    const ground = token("ground");
    ctx.clearRect(0, 0, W, H);
    ctx.fillStyle = ground;
    ctx.fillRect(0, 0, W, H);
    const top = stackTop(win);
    const scale = W / 275; // device px per logical px
    const img = i >= 0 ? images[i] : null;
    const [fw] = status?.frame_size ?? [550, 1044];
    const perLogical = fw / 275; // frame px per logical px
    if (img && img.complete && img.naturalWidth > 0) {
      ctx.save();
      ctx.filter = `blur(${1.2 * dpr}px)${warn ? " grayscale(0.85) brightness(0.55)" : ""}`;
      ctx.drawImage(img, 0, top * perLogical, fw, (H / scale) * perLogical, 0, 0, W, H);
      ctx.restore();
    }
    // The radar's reach, 75, 150 and 230 km, and where it stands.
    const siteY = (status?.site_y ?? 188) - top;
    const kmPerLogical = 0.84 * perLogical;
    const accent = token("accent");
    ctx.save();
    ctx.strokeStyle = accent;
    ctx.lineWidth = Math.max(1, dpr);
    for (const [km, alpha] of [
      [75, 0.12],
      [150, 0.09],
      [230, 0.07],
    ] as const) {
      ctx.globalAlpha = alpha;
      ctx.beginPath();
      ctx.arc(137.5 * scale, siteY * scale, (km / kmPerLogical) * scale, 0, 2 * Math.PI);
      ctx.stroke();
    }
    if (status?.site) {
      ctx.globalAlpha = 0.9;
      ctx.fillStyle = token("alert");
      ctx.shadowColor = token("alert");
      ctx.shadowBlur = 6 * dpr;
      const s = 3 * scale;
      ctx.fillRect(137.5 * scale - s / 2, siteY * scale - s / 2, s, s);
    }
    ctx.restore();
    // The ground over the radar, so the words on the chrome stay words.
    ctx.globalAlpha = 0.35;
    ctx.fillStyle = ground;
    ctx.fillRect(0, 0, W, H);
    ctx.globalAlpha = 1;
  }

  function wake() {
    if (!raf && canvas) raf = requestAnimationFrame(draw);
  }

  $effect(() => {
    const onVisible = () => {
      shown = -2;
      wake();
    };
    document.addEventListener("visibilitychange", onVisible);
    const ro = new ResizeObserver(() => {
      dpr = globalThis.devicePixelRatio || 1;
      const r = canvas.getBoundingClientRect();
      W = Math.round(r.width * dpr);
      H = Math.round(r.height * dpr);
      canvas.width = W;
      canvas.height = H;
      shown = -2;
      wake();
    });
    ro.observe(canvas);
    return () => {
      ro.disconnect();
      document.removeEventListener("visibilitychange", onVisible);
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
    };
  });
</script>

<canvas bind:this={canvas} class="radar" aria-hidden="true"></canvas>

<style>
  /* Under every sprite, the whole window. */
  .radar {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
    pointer-events: none;
  }
</style>
