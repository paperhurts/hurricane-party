// The audio graph (D5): one <audio> element feeding a Web Audio chain, so the
// analyser and the EQ come from AnalyserNode and BiquadFilterNode instead of
// hand-rolled DSP. It lives in the classic Main window, which owns playback;
// every other window is a remote.
//
//   <audio> -> source -> preamp -> lowshelf -> peaking x8 -> highshelf
//           -> trim -> analyser -> destination
//
// The order is the architecture doc's "Equalizer spec" (D21). The trim sits
// after the filters so it can pay for their boost; the analyser sits after
// the trim so the spectrum shows what the speakers get and the clip check
// sees the real ceiling.
//
// Once createMediaElementSource has been called the element's sound reaches the
// speakers only through this graph, so it must be built before the first play.
import { BANDS, dbToGain, trimDb, type EqState } from "./eq";

/** Q for the eight peaking bands. Shelves ignore it. */
const PEAK_Q = 1.2;
/** Time constant for parameter ramps: no zipper noise under a dragged slider. */
const RAMP_S = 0.02;

export class AudioGraph {
  readonly ctx: AudioContext;
  readonly analyser: AnalyserNode;
  private readonly source: MediaElementAudioSourceNode;
  private readonly preamp: GainNode;
  private readonly filters: BiquadFilterNode[];
  private readonly trim: GainNode;

  constructor(el: HTMLMediaElement) {
    this.ctx = new AudioContext();
    this.source = this.ctx.createMediaElementSource(el);

    this.preamp = this.ctx.createGain();
    this.filters = BANDS.map((hz, i) => {
      const f = this.ctx.createBiquadFilter();
      f.type = i === 0 ? "lowshelf" : i === BANDS.length - 1 ? "highshelf" : "peaking";
      f.frequency.value = hz;
      f.Q.value = PEAK_Q;
      f.gain.value = 0;
      return f;
    });
    this.trim = this.ctx.createGain();

    this.analyser = this.ctx.createAnalyser();
    // 2048 gives 1024 bins, 21.5 Hz each at 44.1 kHz: enough that the lowest
    // bar (50-70 Hz) owns at least one bin of its own. Smoothing is the
    // analyser's own decay; the bar physics in spectrum.ts sit on top of it.
    this.analyser.fftSize = 2048;
    this.analyser.smoothingTimeConstant = 0.72;
    this.analyser.minDecibels = -85;
    this.analyser.maxDecibels = -15;

    let node: AudioNode = this.source;
    for (const next of [this.preamp, ...this.filters, this.trim, this.analyser]) {
      node.connect(next);
      node = next;
    }
    node.connect(this.ctx.destination);
  }

  /** A context created without a user gesture starts suspended. */
  resume(): Promise<void> {
    return this.ctx.state === "running" ? Promise.resolve() : this.ctx.resume();
  }

  /** Push an EQ state into the chain. Off means every stage at unity. */
  applyEq(s: EqState): void {
    const t = this.ctx.currentTime;
    const on = s.on;
    this.preamp.gain.setTargetAtTime(dbToGain(on ? s.preamp : 0), t, RAMP_S);
    this.filters.forEach((f, i) => f.gain.setTargetAtTime(on ? (s.bands[i] ?? 0) : 0, t, RAMP_S));
    this.trim.gain.setTargetAtTime(dbToGain(trimDb(s)), t, RAMP_S);
  }

  /**
   * Did the most recent block that reached the analyser go past the ceiling?
   * Web Audio carries floats, so a sample beyond ±1.0 is visible here and
   * will be clamped at the output: that is a clip, exactly.
   */
  clipping(buf: Float32Array<ArrayBuffer>): boolean {
    this.analyser.getFloatTimeDomainData(buf);
    for (let i = 0; i < buf.length; i++) {
      const v = buf[i];
      if (v > 1 || v < -1) return true;
    }
    return false;
  }
}
