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
/** One trip through the palette's hues: the designer's minute and a half and
 * five seconds (design/screens/Kaleidoscope, D132). */
export const HUE_PERIOD_S = 95;

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

/** A colour's hue in degrees, 0..360, from `#RRGGBB`. A grey has none and
 * answers 0. */
export function hueOf(hex: string): number {
  const n = parseInt(hex.slice(1, 7), 16);
  if (!/^#[0-9a-fA-F]{6}/.test(hex) || !Number.isFinite(n)) return 0;
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => v / 255);
  const max = Math.max(r, g, b);
  const d = max - Math.min(r, g, b);
  if (d === 0) return 0;
  const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return (h * 60 + 360) % 360;
}

/**
 * The hue the pattern is at time `t` (seconds), in degrees: once round the
 * ramp's hues every `period`, gliding from each to the next the short way
 * round the wheel, and back to the first. The kaleidoscope reads a ramp's
 * hues and draws them at its own lightness, which is what keeps a pastel
 * ramp and a dark one both a glow: the designer's hues were numbers, and the
 * palette's colours are where this app keeps them (D132).
 */
export function driftHue(t: number, hues: number[], period = HUE_PERIOD_S): number {
  if (hues.length === 0) return 0;
  if (hues.length === 1) return hues[0];
  const phase = (((t / period) % 1) + 1) % 1;
  const at = phase * hues.length;
  const i = Math.floor(at) % hues.length;
  const a = hues[i];
  const b = hues[(i + 1) % hues.length];
  const step = ((((b - a) % 360) + 540) % 360) - 180;
  return (((a + step * (at - Math.floor(at))) % 360) + 360) % 360;
}

/** Where the mandalas sit in a box `w` x `h`. A box about as wide as it is
 * tall holds one, filling it: Purricane's round badge, which is the
 * designer's. A wide, short one (Eyewall's display is about five times as
 * wide as it is tall) is a band of them side by side, a little larger than
 * the height, rather than one small one in the middle. */
export function centres(w: number, h: number): { x: number; y: number; r: number }[] {
  if (!(w > 0 && h > 0)) return [];
  const n = Math.max(1, Math.round(w / (h * 1.6)));
  if (n === 1) return [{ x: w / 2, y: h / 2, r: Math.min(w, h) / 2 }];
  const r = h * 0.55;
  return Array.from({ length: n }, (_, i) => ({ x: ((i + 0.5) * w) / n, y: h / 2, r }));
}
