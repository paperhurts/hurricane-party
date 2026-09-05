import { describe, expect, it } from "vitest";
import { isVisMode, loadVisMode, nextVisMode, saveVisMode, scopeColorIndex, STORAGE_KEY } from "./vis";

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
    clear: () => {},
    key: () => null,
    length: 0,
  };
}

describe("vis mode", () => {
  it("cycles bars, scope, off, bars", () => {
    expect(nextVisMode("bars")).toBe("scope");
    expect(nextVisMode("scope")).toBe("off");
    expect(nextVisMode("off")).toBe("bars");
  });

  it("round-trips and falls back on junk", () => {
    const st = mem();
    expect(loadVisMode(st)).toBe("bars");
    saveVisMode(st, "scope");
    expect(loadVisMode(st)).toBe("scope");
    st.data[STORAGE_KEY] = "kaleidoscope";
    expect(loadVisMode(st)).toBe("bars");
    expect(isVisMode("off")).toBe(true);
    expect(isVisMode(3)).toBe(false);
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
    expect(loadVisMode(boom)).toBe("bars");
    expect(() => saveVisMode(boom, "off")).not.toThrow();
  });
});

describe("scope colour", () => {
  it("is the floor of the ramp at silence and the top at full swing", () => {
    expect(scopeColorIndex(128, 24)).toBe(0);
    expect(scopeColorIndex(0, 24)).toBe(23);
    expect(scopeColorIndex(255, 24)).toBe(23);
    expect(scopeColorIndex(192, 24)).toBe(12);
    expect(scopeColorIndex(64, 24)).toBe(12);
  });
});
