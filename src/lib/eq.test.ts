import { describe, expect, it } from "vitest";
import { shadeBarPx } from "./eq";
import {
  applyPreset,
  BANDS,
  clampDb,
  CUSTOM,
  dbToGain,
  defaultEq,
  loadEq,
  nextPreset,
  PRESETS,
  presetName,
  saveEq,
  STORAGE_KEY,
  trimDb,
} from "./eq";

function mem(): Storage & { data: Record<string, string> } {
  const data: Record<string, string> = {};
  return {
    data,
    getItem: (k) => (k in data ? data[k] : null),
    setItem: (k, v) => {
      data[k] = String(v);
    },
    removeItem: (k) => {
      delete data[k];
    },
    clear: () => {
      for (const k of Object.keys(data)) delete data[k];
    },
    key: () => null,
    length: 0,
  };
}

describe("model", () => {
  it("has the classic ten bands in order (D21)", () => {
    expect([...BANDS]).toEqual([60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000, 16000]);
  });

  it("clamps to ±12 dB and treats junk as flat", () => {
    expect(clampDb(40)).toBe(12);
    expect(clampDb(-40)).toBe(-12);
    expect(clampDb(3.5)).toBe(3.5);
    expect(clampDb(NaN)).toBe(0);
  });

  it("converts dB to an amplitude ratio", () => {
    expect(dbToGain(0)).toBe(1);
    expect(dbToGain(6)).toBeCloseTo(1.995, 2);
    expect(dbToGain(-12)).toBeCloseTo(0.251, 2);
  });

  it("every preset has a preamp and ten bands, all in range", () => {
    for (const vals of Object.values(PRESETS)) {
      expect(vals.length).toBe(1 + BANDS.length);
      for (const v of vals) expect(v).toBe(clampDb(v));
    }
  });
});

describe("trim", () => {
  it("is zero when flat or off", () => {
    expect(trimDb(defaultEq())).toBe(0);
    const loud = applyPreset(defaultEq(), "FULL LOUD");
    expect(trimDb({ ...loud, on: false })).toBe(0);
  });

  it("pulls down by the largest band boost, ignoring cuts and the preamp", () => {
    const s = defaultEq();
    s.bands[0] = 9;
    s.bands[5] = -12;
    expect(trimDb(s)).toBe(-9);
    s.preamp = 12;
    expect(trimDb(s)).toBe(-9);
    s.bands.fill(-3);
    expect(trimDb(s)).toBe(0);
  });
});

describe("presets", () => {
  it("names the preset a state matches, and CUSTOM otherwise", () => {
    expect(presetName(defaultEq())).toBe("FLAT");
    const s = applyPreset(defaultEq(), "STORM WATCH");
    expect(presetName(s)).toBe("STORM WATCH");
    s.bands[3] += 1;
    expect(presetName(s)).toBe(CUSTOM);
  });

  it("applying keeps the on/off switch and copies the values", () => {
    const off = { ...defaultEq(), on: false };
    const s = applyPreset(off, "VOICE / NOAA");
    expect(s.on).toBe(false);
    expect(s.preamp).toBe(-2);
    expect(s.bands).toEqual(PRESETS["VOICE / NOAA"].slice(1));
    expect(s.bands).not.toBe(PRESETS["VOICE / NOAA"]);
  });

  it("ignores an unknown preset name", () => {
    const s = defaultEq();
    expect(applyPreset(s, "NOPE")).toBe(s);
  });

  it("cycles and wraps", () => {
    const names = Object.keys(PRESETS);
    expect(nextPreset(names[0])).toBe(names[1]);
    expect(nextPreset(names[names.length - 1])).toBe(names[0]);
    expect(nextPreset(CUSTOM)).toBe(names[0]);
  });
});

describe("persistence", () => {
  it("round-trips", () => {
    const st = mem();
    const s = applyPreset(defaultEq(), "STORM WATCH");
    s.on = false;
    saveEq(st, s);
    expect(loadEq(st)).toEqual(s);
  });

  it("defaults when empty, corrupt, or the wrong shape", () => {
    expect(loadEq(mem())).toEqual(defaultEq());
    const bad = mem();
    bad.data[STORAGE_KEY] = "{not json";
    expect(loadEq(bad)).toEqual(defaultEq());
    const short = mem();
    short.data[STORAGE_KEY] = JSON.stringify({ on: true, preamp: 3, bands: [1, 2, 3] });
    expect(loadEq(short)).toEqual({ on: true, preamp: 3, bands: [...defaultEq().bands] });
  });

  it("clamps what it reads", () => {
    const st = mem();
    st.data[STORAGE_KEY] = JSON.stringify({ on: true, preamp: 99, bands: new Array(10).fill(-99) });
    const s = loadEq(st);
    expect(s.preamp).toBe(12);
    expect(s.bands.every((v) => v === -12)).toBe(true);
  });

  it("survives a storage that throws", () => {
    const boom = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
    };
    expect(loadEq(boom)).toEqual(defaultEq());
    expect(() => saveEq(boom, defaultEq())).not.toThrow();
  });
});

describe("shadeBarPx", () => {
  it("spans one pixel at the floor to the full height at the ceiling", () => {
    expect(shadeBarPx(-12)).toBe(1);
    expect(shadeBarPx(12)).toBe(9);
    expect(shadeBarPx(0)).toBe(5);
  });

  it("clamps out-of-range gains and never returns a gap", () => {
    expect(shadeBarPx(-40)).toBe(1);
    expect(shadeBarPx(40)).toBe(9);
    expect(shadeBarPx(Number.NaN)).toBe(5);
    expect(shadeBarPx(12, 4)).toBe(4);
  });
});
