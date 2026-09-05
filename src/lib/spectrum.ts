// The analyser's arithmetic, kept pure so it can be tested without a webview.
//
// The AnalyserNode hands back one byte per FFT bin, linear in frequency. Bars
// on a spectrum display are spaced by ear, which is logarithmic, so the first
// job is to decide which bins feed which bar. The second is the motion: bars
// that drop at a bounded rate and a peak marker that hangs, then falls under
// gravity. That is what makes it read as an instrument rather than a graph.
//
// Everything here is unitless (0..1) so it works at any canvas size and DPR.

/** Half-open bin ranges, one per bar: bar i reads bins edges[2i] .. edges[2i+1]. */
export type BandEdges = Uint16Array;

/**
 * Log-spaced bands over `fMin..fMax` Hz, mapped onto `binCount` FFT bins.
 *
 * Every band gets at least one bin and the bands are contiguous, so no bin is
 * read twice and none between fMin and fMax is skipped. At the low end several
 * log-spaced edges can land on the same bin; those bands are pushed up one bin
 * each rather than collapsed, which keeps the bar count honest even when the
 * FFT is too coarse to give each bar its own frequency.
 */
export function bandEdges(
  bars: number,
  binCount: number,
  sampleRate: number,
  fMin = 50,
  fMax = 16000,
): BandEdges {
  if (bars < 1) throw new RangeError("bars must be >= 1");
  if (binCount < bars + 1) throw new RangeError("not enough bins for that many bars");
  // Bin k is centred on k * sampleRate / (2 * binCount).
  const binHz = sampleRate / (2 * binCount);
  const top = Math.min(binCount, Math.floor(fMax / binHz));
  const edges = new Uint16Array(bars * 2);
  // Bin 0 is DC; skip it.
  let lo = Math.max(1, Math.floor(fMin / binHz));
  for (let i = 0; i < bars; i++) {
    const f = fMin * Math.pow(fMax / fMin, (i + 1) / bars);
    let hi = Math.floor(f / binHz);
    // At least one bin, and leave enough for the bars still to come.
    const remaining = bars - i - 1;
    hi = Math.max(hi, lo + 1);
    hi = Math.min(hi, top - remaining);
    edges[i * 2] = lo;
    edges[i * 2 + 1] = hi;
    lo = hi;
  }
  return edges;
}

/** Loudest bin in each band, scaled to 0..1. `out` is reused when given. */
export function reduceBands(
  data: Uint8Array,
  edges: BandEdges,
  out?: Float32Array,
): Float32Array {
  const n = edges.length / 2;
  const vals = out && out.length === n ? out : new Float32Array(n);
  for (let i = 0; i < n; i++) {
    let max = 0;
    for (let k = edges[i * 2]; k < edges[i * 2 + 1]; k++) {
      if (data[k] > max) max = data[k];
    }
    vals[i] = max / 255;
  }
  return vals;
}

export type LevelsOptions = {
  /** Most a bar may drop per frame, in full-scale units. */
  fall?: number;
  /** Frames a peak marker hangs before it starts to fall. */
  hold?: number;
  /** How much faster the peak falls each frame once released. */
  gravity?: number;
};

/**
 * Bar and peak motion. Rises are instant; falls are bounded. Feed it one frame
 * of band values (0..1) and read `bars` and `peaks` back, also 0..1.
 */
export class Levels {
  readonly bars: Float32Array;
  readonly peaks: Float32Array;
  private readonly holdLeft: Int16Array;
  private readonly peakVel: Float32Array;
  private readonly fall: number;
  private readonly hold: number;
  private readonly gravity: number;

  constructor(n: number, opts: LevelsOptions = {}) {
    this.bars = new Float32Array(n);
    this.peaks = new Float32Array(n);
    this.holdLeft = new Int16Array(n);
    this.peakVel = new Float32Array(n);
    this.fall = opts.fall ?? 0.05;
    this.hold = opts.hold ?? 18;
    this.gravity = opts.gravity ?? 0.0025;
  }

  step(values: ArrayLike<number>): void {
    const n = this.bars.length;
    for (let i = 0; i < n; i++) {
      const v = Math.min(1, Math.max(0, values[i] ?? 0));
      const bar = v >= this.bars[i] ? v : Math.max(v, this.bars[i] - this.fall);
      this.bars[i] = bar;

      if (bar >= this.peaks[i]) {
        this.peaks[i] = bar;
        this.holdLeft[i] = this.hold;
        this.peakVel[i] = 0;
      } else if (this.holdLeft[i] > 0) {
        this.holdLeft[i]--;
      } else {
        this.peakVel[i] += this.gravity;
        this.peaks[i] = Math.max(bar, this.peaks[i] - this.peakVel[i]);
      }
    }
  }

  /** True when every bar and peak has come to rest at zero. */
  settled(): boolean {
    for (let i = 0; i < this.bars.length; i++) {
      if (this.bars[i] > 0 || this.peaks[i] > 0) return false;
    }
    return true;
  }
}
