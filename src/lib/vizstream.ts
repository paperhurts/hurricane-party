// The viz channel's source side (control-api.md, D15). Rust owns the
// subscribers and their pipes; it tells this window when to capture and how
// fast, and this reads the graph's stream analyser on a timer and ships each
// frame over IPC as bytes: the raw FFT bins as the body, the scalars as
// headers. Rust maps bins to each subscriber's bands, so one loop here serves
// a 32-band LED wall and a 128-band desktop toy at once.
//
// Kept apart from Main.svelte so the loop can be tested without a webview:
// the analyser and the transport are both injected.

/** What Rust wants from the source. `viz_demand` answers it; `viz:capture` pushes changes. */
export type Demand = { active: boolean; rate_hz: number };

/** What the loop reads each tick. `sourceFromAnalyser` adapts an AnalyserNode. */
export interface VizSource {
  readonly frequencyBinCount: number;
  readonly fftSize: number;
  readonly sampleRate: number;
  /**
   * How long after the graph renders a block the speaker plays it, in ms
   * (`AudioContext.outputLatency` plus `baseLatency`). The analyser reads
   * ahead of the speaker by this much, which the latency doc accounts for.
   */
  outputLatencyMs(): number;
  getByteFrequencyData(out: Uint8Array<ArrayBuffer>): void;
  getFloatTimeDomainData(out: Float32Array<ArrayBuffer>): void;
}

export function sourceFromAnalyser(a: AnalyserNode): VizSource {
  const ctx = a.context as AudioContext;
  return {
    frequencyBinCount: a.frequencyBinCount,
    fftSize: a.fftSize,
    sampleRate: ctx.sampleRate,
    // Live values: outputLatency settles once the context is running.
    outputLatencyMs: () => ((ctx.outputLatency ?? 0) + (ctx.baseLatency ?? 0)) * 1000,
    getByteFrequencyData: (out) => a.getByteFrequencyData(out),
    getFloatTimeDomainData: (out) => a.getFloatTimeDomainData(out),
  };
}

/** Sends one frame. In the app this is `invoke("viz_frame", bins, { headers })`. */
export type Transport = (bins: Uint8Array<ArrayBuffer>, headers: Record<string, string>) => Promise<void>;

/** Peak and RMS of a block of float samples, as the frame's 0..255 bytes. */
export function levelBytes(samples: ArrayLike<number>): { peak: number; rms: number } {
  let peak = 0;
  let sum = 0;
  for (let i = 0; i < samples.length; i++) {
    const v = samples[i];
    const a = Math.abs(v);
    if (a > peak) peak = a;
    sum += v * v;
  }
  const rms = samples.length ? Math.sqrt(sum / samples.length) : 0;
  const byte = (x: number) => Math.min(255, Math.round(Math.min(1, x) * 255));
  return { peak: byte(peak), rms: byte(rms) };
}

/**
 * Microseconds since the epoch on this window's clock, as a decimal string
 * for a header. `timeOrigin + now()` is the wall clock at sub-millisecond
 * resolution; a client on the same machine subtracts it from its own clock
 * and has the end-to-end latency, which is what #7 asked for.
 */
export function wallMicros(now = performance.now(), origin = performance.timeOrigin): string {
  return String(Math.round((origin + now) * 1000));
}

/** Bins to assume before the graph exists, so a rig sees a live (silent) stream at launch. */
const DEFAULT_BINS = 1024;
const DEFAULT_RATE = 48000;

/**
 * The timer runs this often and the loop decides by the clock whether a frame
 * is due. A `setInterval` at the frame period itself came out quantised to the
 * 15.6 ms Windows tick (31 ms, 47 ms, 31 ms at 30 Hz): Chromium only raises
 * the timer resolution for delays under about 32 ms. A fine timer keeps it
 * raised and the accumulator lands each frame within a few ms of when it is due.
 */
const TIMER_MS = 4;

/** The capture loop. Follows Rust's demand; reads the source; ships frames. */
export class VizCapture {
  source: VizSource | null = null;
  /** Ticks skipped because the previous frame's IPC had not returned. */
  dropped = 0;
  /** Sends whose IPC call rejected, and what the last one said. */
  failed = 0;
  lastError = "";
  private seq = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private rateHz = 0;
  private periodMs = 0;
  private nextAt = 0;
  private inflight = false;
  private bins: Uint8Array<ArrayBuffer> | null = null;
  private samples: Float32Array<ArrayBuffer> | null = null;

  constructor(
    private readonly send: Transport,
    private readonly clock: () => string = () => wallMicros(),
    private readonly nowMs: () => number = () => performance.now(),
  ) {}

  get running(): boolean {
    return this.timer !== null;
  }

  get rate(): number {
    return this.rateHz;
  }

  /** Follow a demand. Idempotent: the same demand twice changes nothing. */
  apply(d: Demand): void {
    if (!d.active || d.rate_hz <= 0) {
      this.stop();
      return;
    }
    if (this.timer && this.rateHz === d.rate_hz) return;
    this.stop();
    this.rateHz = d.rate_hz;
    this.periodMs = 1000 / d.rate_hz;
    this.nextAt = this.nowMs() + this.periodMs;
    this.timer = setInterval(() => this.pump(), TIMER_MS);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.rateHz = 0;
  }

  /** How late the last frame's tick ran after it was due, in ms. Diagnostic. */
  lateMs = 0;

  /** The timer's callback: send a frame if one is due. */
  pump(): void {
    const now = this.nowMs();
    if (now < this.nextAt) return;
    this.lateMs = now - this.nextAt;
    this.nextAt += this.periodMs;
    if (now >= this.nextAt) {
      // Asleep for a while (a throttled webview): resume on the present
      // beat rather than bursting the missed ones, which would arrive late.
      this.nextAt = now + this.periodMs;
    }
    this.tick();
  }

  /**
   * One frame. Drops the tick rather than queueing if the last send is still
   * out: a frame that arrives late is worse than one that never leaves, the
   * same policy Rust applies on the pipe.
   */
  tick(): void {
    if (this.inflight) {
      this.dropped++;
      return;
    }
    const src = this.source;
    const n = src ? src.frequencyBinCount : DEFAULT_BINS;
    if (!this.bins || this.bins.length !== n) this.bins = new Uint8Array(n);
    let peak = 0;
    let rms = 0;
    let rate = DEFAULT_RATE;
    let outLat = 0;
    if (src) {
      src.getByteFrequencyData(this.bins);
      if (!this.samples || this.samples.length !== src.fftSize) {
        this.samples = new Float32Array(src.fftSize);
      }
      src.getFloatTimeDomainData(this.samples);
      ({ peak, rms } = levelBytes(this.samples));
      rate = src.sampleRate;
      outLat = src.outputLatencyMs();
    } else {
      this.bins.fill(0);
    }
    const headers = {
      "x-hp-ts": this.clock(),
      "x-hp-rate": String(rate),
      "x-hp-peak": String(peak),
      "x-hp-rms": String(rms),
      "x-hp-outlat": outLat.toFixed(1),
      // Diagnostics for HP_VIZ_TRACE: the source's own view of its cadence.
      "x-hp-seq": String(++this.seq),
      "x-hp-dropped": String(this.dropped),
      "x-hp-failed": String(this.failed),
      "x-hp-late": this.lateMs.toFixed(1),
      "x-hp-err": this.lastError,
    };
    this.inflight = true;
    this.send(this.bins, headers)
      .catch((e) => {
        this.failed++;
        this.lastError = String(e).replace(/[^\x20-\x7e]/g, " ").slice(0, 120);
      })
      .finally(() => {
        this.inflight = false;
      });
  }
}
