// The visuals window's frames (#167, D169), as `viz.rs` sends them over the
// channel: a flags byte (bit 0, a beat), the peak and RMS levels, a spare
// byte, the bands, then the waveform, 128 at rest. Pure, so it is tested
// without a webview.

export const VISUALS_BANDS = 64;
export const VISUALS_WAVE = 256;

export type VisualsFrame = {
  beat: boolean;
  /** 0..1 */
  peak: number;
  /** 0..1 */
  rms: number;
  /** 0..255 per band, low to high. */
  bands: Uint8Array;
  /** 0..255 around 128. */
  wave: Uint8Array;
};

/** A frame of silence: what is drawn before the first one arrives. */
export function silentFrame(): VisualsFrame {
  return {
    beat: false,
    peak: 0,
    rms: 0,
    bands: new Uint8Array(VISUALS_BANDS),
    wave: new Uint8Array(VISUALS_WAVE).fill(128),
  };
}

/** One frame from the channel's bytes, or null when they are not one. */
export function parseVisualsFrame(buf: ArrayBuffer | Uint8Array): VisualsFrame | null {
  const b = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  const bands = b.length - 4 - VISUALS_WAVE;
  if (bands < 1) return null;
  return {
    beat: (b[0] & 1) === 1,
    peak: b[1] / 255,
    rms: b[2] / 255,
    bands: b.slice(4, 4 + bands),
    wave: b.slice(4 + bands),
  };
}

/** The mean of the lowest `n` bands, 0..1: the bass the flow breathes with. */
export function bassOf(f: VisualsFrame, n = 6): number {
  const k = Math.min(n, f.bands.length);
  if (!k) return 0;
  let sum = 0;
  for (let i = 0; i < k; i++) sum += f.bands[i];
  return sum / k / 255;
}

/**
 * Beats into pulses, never more than three a second: the same ceiling on
 * flashing Purricane's blooms keep (`purricane.md`, accessibility), so a
 * visual driven by a fast track cannot strobe. A pulse starts at 1 and
 * fades; `value` is where it is now.
 */
export class Pulse {
  static readonly MIN_GAP_S = 1 / 3;
  static readonly FADE_S = 0.35;
  private lastAt = -Infinity;

  /** A beat at `t` seconds. True when it starts a pulse. */
  beat(t: number): boolean {
    if (t - this.lastAt < Pulse.MIN_GAP_S) return false;
    this.lastAt = t;
    return true;
  }

  /** 1 at a pulse's start, down to 0 over `FADE_S`. */
  value(t: number): number {
    const age = t - this.lastAt;
    return age >= Pulse.FADE_S || age < 0 ? 0 : 1 - age / Pulse.FADE_S;
  }
}

/** `#rrggbb` as 0..1 floats. Anything else is black. */
export function rgbOf(hex: string): [number, number, number] {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(hex.trim());
  if (!m) return [0, 0, 0];
  return [parseInt(m[1], 16) / 255, parseInt(m[2], 16) / 255, parseInt(m[3], 16) / 255];
}
