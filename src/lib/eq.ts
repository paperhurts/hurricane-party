// The equalizer's model (D21), kept pure and shared by both windows that care:
// the EQ window that shows the sliders and the Main window that owns the
// audio graph and applies them. Values are decibels throughout; the graph
// converts at the edge.
//
// Shape matters for later: `.eqf` presets (D31) are ten band bytes then the
// preamp byte, inverted (0x00 = +12 dB). Keeping bands as a ten-element array
// in D21 order with the preamp separate means that import is a byte-mapping,
// not a refactor.

/** The classic ten, in Hz, in order (D21, verified in D31). */
export const BANDS = [60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000, 16000] as const;
export const LABELS = ["60", "170", "310", "600", "1k", "3k", "6k", "12k", "14k", "16k"] as const;

export const DB_MIN = -12;
export const DB_MAX = 12;

export type EqState = {
  on: boolean;
  /** dB, -12..12 */
  preamp: number;
  /** dB, -12..12, ten entries in BANDS order */
  bands: number[];
};

export const FLAT: readonly number[] = Object.freeze(new Array(BANDS.length).fill(0));

/** Preamp first, then the ten bands. Names as they appear in the window. */
export const PRESETS: Record<string, readonly number[]> = {
  FLAT: [0, ...FLAT],
  "STORM WATCH": [4, 9, 7, 3, -1, -3, -2, 2, 5, 7, 8],
  "VOICE / NOAA": [-2, -6, -4, 2, 7, 8, 6, 1, -3, -5, -6],
  "FULL LOUD": [11, 12, 10, 7, 4, 3, 5, 8, 11, 12, 12],
};

export const CUSTOM = "CUSTOM";

export function defaultEq(): EqState {
  return { on: true, preamp: 0, bands: [...FLAT] };
}

export function clampDb(v: number): number {
  if (!Number.isFinite(v)) return 0;
  return Math.min(DB_MAX, Math.max(DB_MIN, v));
}

/** Amplitude ratio for a dB figure: 0 dB is unity, +6 is about double. */
export function dbToGain(db: number): number {
  return Math.pow(10, db / 20);
}

/**
 * The automatic trim after the chain: pulled down by the largest band boost
 * (architecture.md, "Equalizer spec"), so boosting one band reads as cutting
 * the others rather than pushing the whole mix past unity. The preamp is left
 * out on purpose: it is the user's explicit gain, and compensating it would
 * make it do nothing. The clip lamp covers what this does not.
 */
export function trimDb(s: EqState): number {
  if (!s.on) return 0;
  const boost = Math.max(0, ...s.bands);
  return boost > 0 ? -boost : 0;
}

/** Which preset the state matches exactly, or CUSTOM. */
export function presetName(s: EqState): string {
  for (const [name, vals] of Object.entries(PRESETS)) {
    if (vals[0] === s.preamp && vals.slice(1).every((v, i) => v === s.bands[i])) return name;
  }
  return CUSTOM;
}

export function applyPreset(s: EqState, name: string): EqState {
  const vals = PRESETS[name];
  if (!vals) return s;
  return { on: s.on, preamp: vals[0], bands: vals.slice(1) };
}

/** Cycle to the next preset name after the current one, wrapping. */
export function nextPreset(current: string): string {
  const names = Object.keys(PRESETS);
  const i = names.indexOf(current);
  return names[(i + 1) % names.length];
}

// ---- persistence ------------------------------------------------------------
//
// localStorage is shared by every window of the app (same origin), so both
// windows read the same saved state at mount and only the EQ window writes.

export const STORAGE_KEY = "hp.eq.v1";

type StorageLike = Pick<Storage, "getItem" | "setItem">;

export function loadEq(storage: StorageLike): EqState {
  const d = defaultEq();
  let raw: string | null = null;
  try {
    raw = storage.getItem(STORAGE_KEY);
  } catch {
    return d;
  }
  if (!raw) return d;
  try {
    const p = JSON.parse(raw) as Partial<EqState>;
    const bands = Array.isArray(p.bands) && p.bands.length === BANDS.length ? p.bands.map(clampDb) : d.bands;
    return {
      on: typeof p.on === "boolean" ? p.on : d.on,
      preamp: clampDb(Number(p.preamp)),
      bands,
    };
  } catch {
    return d;
  }
}

export function saveEq(storage: StorageLike, s: EqState): void {
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify(s));
  } catch {
    // A full or blocked store is not worth failing playback over.
  }
}
