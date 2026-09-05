import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { levelBytes, VizCapture, wallMicros, type VizSource } from "./vizstream";

function fakeSource(bins = 16, fftSize = 32, sampleRate = 48000, fill = 100): VizSource {
  return {
    frequencyBinCount: bins,
    fftSize,
    sampleRate,
    outputLatencyMs: () => 20,
    getByteFrequencyData: (out) => out.fill(fill),
    getFloatTimeDomainData: (out) => out.fill(0.5),
  };
}

describe("levelBytes", () => {
  it("is peak and rms of the block, scaled to bytes", () => {
    const { peak, rms } = levelBytes([0.5, -0.5, 0.5, -0.5]);
    expect(peak).toBe(128);
    expect(rms).toBe(128);
  });

  it("clamps past full scale rather than wrapping", () => {
    expect(levelBytes([3, -3]).peak).toBe(255);
  });

  it("is zero for an empty block", () => {
    expect(levelBytes([])).toEqual({ peak: 0, rms: 0 });
  });
});

describe("wallMicros", () => {
  it("is timeOrigin plus now, in whole microseconds", () => {
    expect(wallMicros(1.5, 1_700_000_000_000)).toBe("1700000000001500");
  });
});

describe("VizCapture", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  /** A 4 ms timer with the clock deciding: one frame per period, whatever the timer's phase. */
  it("ticks at the demanded rate and stops when demand ends", async () => {
    let t = 0;
    const send = vi.fn(() => Promise.resolve());
    const cap = new VizCapture(send, () => "1", () => t);
    cap.apply({ active: true, rate_hz: 60 });
    expect(cap.running).toBe(true);
    for (let i = 0; i < 250; i++) {
      t += 4;
      await vi.advanceTimersByTimeAsync(4);
    }
    // 1000 ms at 16.67 ms per frame, sampled every 4 ms: 59 or 60.
    expect(send.mock.calls.length).toBeGreaterThanOrEqual(59);
    expect(send.mock.calls.length).toBeLessThanOrEqual(60);
    cap.apply({ active: false, rate_hz: 0 });
    expect(cap.running).toBe(false);
    const n = send.mock.calls.length;
    t += 500;
    await vi.advanceTimersByTimeAsync(500);
    expect(send.mock.calls.length).toBe(n);
  });

  /** A send that resolved still settles on a microtask; let it, as the real loop does between timer ticks. */
  async function settle() {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  }

  it("lands frames on the beat, not on the timer's phase", async () => {
    let t = 0;
    const send = vi.fn(() => Promise.resolve());
    const cap = new VizCapture(send, () => "1", () => t);
    cap.apply({ active: true, rate_hz: 30 }); // 33.3 ms
    const sentAt: number[] = [];
    for (t = 0; t <= 204; t += 4) {
      const before = send.mock.calls.length;
      cap.pump();
      if (send.mock.calls.length > before) sentAt.push(t);
      await settle();
    }
    // Due at 33.3, 66.7, 100, 133.3, 166.7, 200.0(…03): each lands within one 4 ms step.
    expect(sentAt).toEqual([36, 68, 100, 136, 168, 204]);
    cap.stop();
  });

  it("resumes on the present beat after a long sleep instead of bursting", async () => {
    let t = 0;
    const send = vi.fn(() => Promise.resolve());
    const cap = new VizCapture(send, () => "1", () => t);
    cap.apply({ active: true, rate_hz: 60 });
    t = 500; // a throttled webview woke up late
    cap.pump();
    await settle();
    expect(send).toHaveBeenCalledTimes(1);
    t = 504;
    cap.pump();
    await settle();
    expect(send).toHaveBeenCalledTimes(1);
    t = 517;
    cap.pump();
    await settle();
    expect(send).toHaveBeenCalledTimes(2);
    cap.stop();
  });

  it("re-times when the rate changes and ignores a repeat", () => {
    const cap = new VizCapture(() => Promise.resolve(), () => "1");
    cap.apply({ active: true, rate_hz: 30 });
    expect(cap.rate).toBe(30);
    cap.apply({ active: true, rate_hz: 30 });
    expect(cap.rate).toBe(30);
    cap.apply({ active: true, rate_hz: 60 });
    expect(cap.rate).toBe(60);
    cap.stop();
  });

  it("ships the source's bins with the scalars as headers", () => {
    const send = vi.fn(() => Promise.resolve());
    const cap = new VizCapture(send, () => "1234");
    cap.source = fakeSource(8, 16, 44100, 200);
    cap.tick();
    expect(send).toHaveBeenCalledTimes(1);
    const [bins, headers] = send.mock.calls[0] as unknown as [Uint8Array, Record<string, string>];
    expect(bins.length).toBe(8);
    expect(bins[0]).toBe(200);
    expect(headers["x-hp-ts"]).toBe("1234");
    expect(headers["x-hp-rate"]).toBe("44100");
    // 0.5 everywhere: peak and rms both 128.
    expect(headers["x-hp-peak"]).toBe("128");
    expect(headers["x-hp-rms"]).toBe("128");
  });

  it("sends silence, not nothing, before the graph exists", () => {
    const send = vi.fn(() => Promise.resolve());
    const cap = new VizCapture(send, () => "1");
    cap.tick();
    const [bins, headers] = send.mock.calls[0] as unknown as [Uint8Array, Record<string, string>];
    expect(bins.length).toBe(1024);
    expect(bins.every((b) => b === 0)).toBe(true);
    expect(headers["x-hp-peak"]).toBe("0");
  });

  /// A tick while the last send is still out is dropped, not queued.
  it("drops a tick while a send is in flight", async () => {
    let release: () => void = () => {};
    const send = vi.fn(() => new Promise<void>((r) => (release = r)));
    const cap = new VizCapture(send, () => "1");
    cap.tick();
    cap.tick();
    cap.tick();
    expect(send).toHaveBeenCalledTimes(1);
    expect(cap.dropped).toBe(2);
    release();
    await Promise.resolve();
    await Promise.resolve();
    cap.tick();
    expect(send).toHaveBeenCalledTimes(2);
  });

  it("keeps going after a send that rejects", async () => {
    const send = vi.fn(() => Promise.reject(new Error("no")));
    const cap = new VizCapture(send, () => "1");
    cap.tick();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    cap.tick();
    expect(send).toHaveBeenCalledTimes(2);
  });
});
