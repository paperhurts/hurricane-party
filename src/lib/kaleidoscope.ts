// Purricane's analyser (#147, `docs/purricane.md`): the parts of the
// kaleidoscope that decide what may move and how fast, kept pure so they are
// tested rather than trusted. The drawing is `Kaleidoscope.svelte`'s.
//
// The accessibility clamps live here and nowhere a theme or a skin can reach:
// rotating high-contrast radial patterns are a known photosensitivity and
// migraine trigger, and a visualizer that hurts someone is a bug.

/** The fastest the pattern turns, whatever a theme asks. A slow constant. */
export const MAX_DEG_PER_SEC = 10;
/** Blooms stay below this rate whatever the tempo (`docs/purricane.md`). */
export const MAX_BLOOM_HZ = 3;
/** A bloom: a fast scale-and-fade, never a colour change. */
export const BLOOM_MS = 150;
/** One trip through the palette's hues. */
export const HUE_PERIOD_S = 90;

/** The pattern's turn at time `t` (seconds), in radians. Zero when still:
 * calm, or `prefers-reduced-motion`. Audio never reaches this. */
export function rotationAt(t: number, degPerSec: number, still: boolean): number {
  if (still) return 0;
  const d = Math.min(MAX_DEG_PER_SEC, Math.max(0, Number.isFinite(degPerSec) ? degPerSec : 0));
  return ((t * d) % 360) * (Math.PI / 180);
}

/**
 * When a beat may bloom. An onset is the bass rising well above its recent
 * average; one that comes sooner than the rate allows is skipped, not queued,
 * so a fast track blooms less rather than flickering. Still never blooms.
 */
export class BloomGate {
  private avg = 0;
  private last = -Infinity;
  private readonly gapMs: number;

  constructor(maxHz = MAX_BLOOM_HZ) {
    const hz = Math.min(MAX_BLOOM_HZ, Math.max(0.1, Number.isFinite(maxHz) ? maxHz : MAX_BLOOM_HZ));
    // A hair over the period, so the rate is strictly below the cap.
    this.gapMs = 1000 / hz + 1;
  }

  /** Feed one frame's bass level, 0..1, at `nowMs`. True when it blooms. */
  feed(bass: number, nowMs: number, still: boolean): boolean {
    const onset = bass > 0.2 && bass > this.avg * 1.4;
    this.avg = this.avg * 0.94 + bass * 0.06;
    if (still || !onset || nowMs - this.last < this.gapMs) return false;
    this.last = nowMs;
    return true;
  }
}

/** How far into a bloom `nowMs` is: 1 at its start, falling to 0. */
export function bloomAt(nowMs: number, bloomedAtMs: number): number {
  const age = nowMs - bloomedAtMs;
  if (!(age >= 0) || age >= BLOOM_MS) return 0;
  return 1 - age / BLOOM_MS;
}

function rgb(h: string): [number, number, number] {
  const n = parseInt(h.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** A colour between two, `t` of the way from `a` to `b`, as `rgb()`. */
export function mix(a: string, b: string, t: number): string {
  const [x, y] = [rgb(a), rgb(b)];
  const k = Math.min(1, Math.max(0, t));
  const c = x.map((v, i) => Math.round(v + (y[i] - v) * k));
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
}

/** How much of each colour's turn is spent on the way to the next. The rest
 * holds: a blend between two colours from opposite sides of the wheel passes
 * through grey, and the first version sat in that grey half the time. */
export const HUE_BLEND = 0.3;

/** The colour the pattern is at time `t`: drifting through `stops` once
 * every `HUE_PERIOD_S`, and back to the first. Each colour holds, then
 * blends into the next over the last `HUE_BLEND` of its turn. */
export function hueAt(t: number, stops: string[], period = HUE_PERIOD_S): string {
  if (stops.length === 0) return "transparent";
  if (stops.length === 1) return mix(stops[0], stops[0], 0);
  const phase = (((t / period) % 1) + 1) % 1;
  const at = phase * stops.length;
  const i = Math.floor(at);
  const into = at - i;
  const k = into <= 1 - HUE_BLEND ? 0 : (into - (1 - HUE_BLEND)) / HUE_BLEND;
  return mix(stops[i % stops.length], stops[(i + 1) % stops.length], k);
}

/** Where the mandalas sit in a box `w` x `h`: as many as fit side by side at
 * a radius that fills the height, so a wide, short display (Main's is about
 * five times as wide as it is tall) is a band of them rather than one small
 * one in the middle or a large one cut to a sliver. */
export function centres(w: number, h: number): { x: number; y: number; r: number }[] {
  if (!(w > 0 && h > 0)) return [];
  const r = h * 0.62;
  const n = Math.max(1, Math.round(w / (h * 1.6)));
  return Array.from({ length: n }, (_, i) => ({ x: ((i + 0.5) * w) / n, y: h / 2, r }));
}
