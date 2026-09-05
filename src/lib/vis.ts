// Which visualizer Main shows. The skin manifest names a theme's default
// component (D20); the user's click on the display cycles through the ones
// the app has, and the choice outlives the session. Same storage rule as the
// EQ: localStorage is shared by every window of the app.

export const VIS_MODES = ["bars", "scope", "off"] as const;
export type VisMode = (typeof VIS_MODES)[number];

export const STORAGE_KEY = "hp.vis.v1";

type StorageLike = Pick<Storage, "getItem" | "setItem">;

export function isVisMode(v: unknown): v is VisMode {
  return typeof v === "string" && (VIS_MODES as readonly string[]).includes(v);
}

export function nextVisMode(m: VisMode): VisMode {
  return VIS_MODES[(VIS_MODES.indexOf(m) + 1) % VIS_MODES.length];
}

export function loadVisMode(storage: StorageLike, fallback: VisMode = "bars"): VisMode {
  try {
    const v = storage.getItem(STORAGE_KEY);
    return isVisMode(v) ? v : fallback;
  } catch {
    return fallback;
  }
}

export function saveVisMode(storage: StorageLike, m: VisMode): void {
  try {
    storage.setItem(STORAGE_KEY, m);
  } catch {
    // Not worth failing over.
  }
}

/**
 * Ramp index for a scope sample: 0 at the centre line, the last step at full
 * swing, so a loud waveform runs green through yellow and red to magenta the
 * way the bars do. `v` is a byte from getByteTimeDomainData, 128 is silence.
 */
export function scopeColorIndex(v: number, steps: number): number {
  const a = Math.abs(v - 128) / 128;
  return Math.min(steps - 1, Math.floor(a * steps));
}
