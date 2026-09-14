import { describe, expect, it } from "vitest";
import {
  audioShare,
  estimate,
  meter,
  pressure,
  sayAudio,
  sayPressure,
  size,
  tipsOver,
  used,
  type StorageStatus,
} from "./storage";

const GB = 1024 ** 3;
const status = (over: Partial<StorageStatus> = {}): StorageStatus => ({
  library_bytes: 200 * GB,
  drive: { free: 500 * GB, total: 1000 * GB },
  ceiling: null,
  audio_bps: 24_000,
  video_bps: 240_000,
  ...over,
});

describe("sizes read the way Explorer shows them (#162)", () => {
  it("picks the unit and keeps one decimal below a hundred", () => {
    expect(size(512)).toBe("512 bytes");
    expect(size(812 * 1024 ** 2)).toBe("812 MB");
    expect(size(3.44 * GB)).toBe("3.4 GB");
    expect(size(1.2 * 1024 * GB)).toBe("1.2 TB");
    expect(size(-5)).toBe("0 bytes");
  });
});

describe("an estimate is this library's rate times the lengths it knows (#162)", () => {
  it("counts what has no length instead of guessing it", () => {
    expect(estimate([60, null, 120, 0], 24_000)).toEqual({ bytes: 4_320_000, unknown: 2 });
  });
  it("has nothing to say without a rate", () => {
    expect(estimate([60], null)).toBeNull();
  });
});

describe("the budget warns at 85% of the drive and past the ceiling, never before (#162, the owner's calls)", () => {
  it("is quiet with room to spare", () => {
    expect(pressure(status())).toBeNull();
    expect(pressure(status(), 300 * GB)).toBeNull();
  });

  it("warns when the drive is, or would be, 85% full", () => {
    expect(pressure(status(), 350 * GB)?.drive).toEqual({ used: 0.85, free: 150 * GB });
    const full = status({ drive: { free: 100 * GB, total: 1000 * GB } });
    expect(pressure(full)?.drive?.used).toBeCloseTo(0.9);
    expect(used({ free: 151 * GB, total: 1000 * GB })).toBeLessThan(0.85);
    expect(pressure(status({ drive: { free: 151 * GB, total: 1000 * GB } }))).toBeNull();
  });

  it("says when it will not fit at all", () => {
    const p = pressure(status({ drive: { free: 2 * GB, total: 1000 * GB } }), 3 * GB);
    expect(p?.wontFit).toBe(true);
  });

  it("warns past the ceiling, which counts the whole library", () => {
    const s = status({ ceiling: 250 * GB });
    expect(pressure(s, 40 * GB)).toBeNull();
    expect(pressure(s, 60 * GB)?.ceiling).toEqual({ after: 260 * GB, over: 10 * GB });
  });

  it("an unplugged download drive has no drive warning to give", () => {
    expect(pressure(status({ drive: null }), 900 * GB)).toBeNull();
  });

  it("tells a download that tips it over from one that lands on a drive already full", () => {
    expect(tipsOver(status(), 360 * GB)).toBe(true);
    expect(tipsOver(status({ drive: { free: 100 * GB, total: 1000 * GB } }), 1 * GB)).toBe(false);
    expect(tipsOver(status(), 0)).toBe(false);
  });
});

describe("the words (#162)", () => {
  it("says what a queued list would do, and what it is now when the size is unknown", () => {
    const s = status({ ceiling: 250 * GB });
    const p = pressure(s, 360 * GB)!;
    expect(sayPressure(p, s, 360 * GB)).toBe(
      "The download drive would be 86% full, with 140 GB left. The library would be 560 GB, past its 250 GB ceiling.",
    );
    const full = status({ drive: { free: 100 * GB, total: 1000 * GB } });
    expect(sayPressure(pressure(full)!, full, 0)).toBe("The download drive is 90% full, with 100 GB left.");
  });

  it("a list that will not fit says so rather than giving a percentage", () => {
    const s = status({ drive: { free: 2 * GB, total: 1000 * GB } });
    expect(sayPressure(pressure(s, 3 * GB)!, s, 3 * GB)).toBe(
      "That is about 3.0 GB, and the download drive has 2.0 GB left: it will not all fit.",
    );
  });

  it("offers audio with a saving measured on this library", () => {
    const s = status();
    expect(audioShare(s)).toBeCloseTo(0.1);
    expect(sayAudio(s, 3 * GB)).toBe("As audio only it would be about 307 MB.");
    expect(sayAudio(s, null)).toBe("Audio only takes about 10% of the space video does in this library.");
    expect(sayAudio(status({ video_bps: null }), 3 * GB)).toBeNull();
  });

  it("the meter is one line, in the warning colour only when the budget is already pressed", () => {
    expect(meter(status())).toMatchObject({
      text: "200 GB in the library · 500 GB free of 1000 GB, 50% full",
      warn: false,
    });
    const m = meter(status({ ceiling: 250 * GB, drive: { free: 100 * GB, total: 1000 * GB } }));
    expect(m.text).toBe("200 GB of a 250 GB ceiling · 100 GB free of 1000 GB, 90% full");
    expect(m.warn).toBe(true);
    expect(meter(status({ drive: null })).text).toBe("200 GB in the library · the download drive is not there");
  });
});
