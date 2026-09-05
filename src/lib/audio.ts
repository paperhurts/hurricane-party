// The audio graph (D5): one <audio> element feeding a Web Audio chain, so the
// analyser and the EQ read from AnalyserNode and BiquadFilterNode instead of
// hand-rolled DSP. It lives in the classic Main window, which owns playback;
// every other window is a remote.
//
//   <audio> -> source -> [EQ chain, D21, not yet] -> analyser -> destination
//
// Once createMediaElementSource has been called the element's sound reaches the
// speakers only through this graph, so it must be built before the first play.

export class AudioGraph {
  readonly ctx: AudioContext;
  readonly analyser: AnalyserNode;
  private readonly source: MediaElementAudioSourceNode;

  constructor(el: HTMLMediaElement) {
    this.ctx = new AudioContext();
    this.source = this.ctx.createMediaElementSource(el);
    this.analyser = this.ctx.createAnalyser();
    // 2048 gives 1024 bins, 21.5 Hz each at 44.1 kHz: enough that the lowest
    // bar (50-70 Hz) owns at least one bin of its own. Smoothing is the
    // analyser's own decay; the bar physics in spectrum.ts sit on top of it.
    this.analyser.fftSize = 2048;
    this.analyser.smoothingTimeConstant = 0.72;
    this.analyser.minDecibels = -85;
    this.analyser.maxDecibels = -15;
    this.source.connect(this.analyser);
    this.analyser.connect(this.ctx.destination);
  }

  /** A context created without a user gesture starts suspended. */
  resume(): Promise<void> {
    return this.ctx.state === "running" ? Promise.resolve() : this.ctx.resume();
  }
}
