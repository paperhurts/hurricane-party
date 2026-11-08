import { afterEach, describe, expect, it, vi } from "vitest";
import { AudioGraph, clampVolume } from "./audio";

// Just enough of Web Audio to see how the graph is wired: every node records
// what it was connected to, and a param records its value and its ramps.
class FakeParam {
  value = 1;
  ramps: number[] = [];
  setTargetAtTime(v: number) {
    this.ramps.push(v);
    this.value = v;
  }
}
class FakeNode {
  out: FakeNode[] = [];
  gain = new FakeParam();
  frequency = new FakeParam();
  Q = new FakeParam();
  type = "";
  fftSize = 0;
  smoothingTimeConstant = 0;
  minDecibels = 0;
  maxDecibels = 0;
  constructor(readonly kind: string) {}
  connect(n: FakeNode) {
    this.out.push(n);
  }
}
class FakeContext {
  currentTime = 0;
  state = "running";
  destination = new FakeNode("destination");
  createMediaElementSource() {
    return new FakeNode("source");
  }
  createGain() {
    return new FakeNode("gain");
  }
  createBiquadFilter() {
    return new FakeNode("biquad");
  }
  createAnalyser() {
    return new FakeNode("analyser");
  }
}

function build(volume?: number) {
  vi.stubGlobal("AudioContext", FakeContext);
  const el = { volume: 0.3 } as HTMLMediaElement;
  const graph = new AudioGraph(el, volume);
  const source = (graph as unknown as { source: FakeNode }).source;
  return { el, graph, first: source.out[0] };
}

afterEach(() => vi.unstubAllGlobals());

describe("the volume (#267)", () => {
  it("is a gain straight after the source, where WebView2 applied the element's", () => {
    const { first } = build(0.4);
    expect(first.kind).toBe("gain");
    expect(first.gain.value).toBe(0.4);
    // And the EQ's preamp after it, so the EQ and the analysers are fed as before.
    expect(first.out[0].kind).toBe("gain");
  });

  it("leaves the element at full volume, which WebKitGTK would ignore anyway", () => {
    expect(build(0.4).el.volume).toBe(1);
  });

  it("follows the slider, clamped, without touching the element", () => {
    const { el, graph, first } = build();
    expect(first.gain.value).toBe(1);
    graph.setVolume(0.25);
    graph.setVolume(7);
    graph.setVolume(Number.NaN);
    expect(first.gain.ramps).toEqual([0.25, 1, 0]);
    expect(el.volume).toBe(1);
  });
});

describe("clampVolume", () => {
  it("keeps 0..1 and makes anything else silence or full", () => {
    expect(clampVolume(0.5)).toBe(0.5);
    expect(clampVolume(-1)).toBe(0);
    expect(clampVolume(2)).toBe(1);
    expect(clampVolume(Number.NaN)).toBe(0);
  });
});
