/**
 * The storage budget (#162, D138): what is said about the library's size, the
 * download drive's room and a person's ceiling, and when it is said.
 *
 * The figures are Rust's (`src-tauri/src/storage.rs`): sums over the library,
 * the drive's room, the ceiling, and the bytes a second each kind takes in
 * this library. This is the wording, so the library window and prep mode
 * (#163) say the same thing from the same numbers.
 */

export type DiskSpace = { free: number; total: number };

export type StorageStatus = {
  library_bytes: number;
  /** The download folder's drive; null when the folder is not there. */
  drive: DiskSpace | null;
  ceiling: number | null;
  audio_bps: number | null;
  video_bps: number | null;
};

/** O10, and the owner's call on #162: warn once the download drive is this full. */
export const DRIVE_WARN = 0.85;

const UNITS = ["bytes", "KB", "MB", "GB", "TB"];

/** Bytes as Explorer shows them: 812 MB, 3.4 GB, 1.2 TB, in powers of 1024. */
export function size(bytes: number): string {
  let n = Math.max(0, bytes);
  let u = 0;
  while (n >= 1024 && u < UNITS.length - 1) {
    n /= 1024;
    u++;
  }
  if (u === 0) return `${Math.round(n)} bytes`;
  return `${n >= 100 ? Math.round(n) : n.toFixed(1)} ${UNITS[u]}`;
}

/** A fraction as a whole percentage, rounded down, so "85% full" is never shown below the line. */
const pct = (f: number) => `${Math.floor(f * 100)}%`;

/** How full the drive is, 0 to 1, after `adding` more bytes. */
export function used(d: DiskSpace, adding = 0): number {
  if (d.total <= 0) return 0;
  return Math.min(1, Math.max(0, 1 - (d.free - adding) / d.total));
}

export type Estimate = {
  bytes: number;
  /** Entries with no length, which the bytes leave out. */
  unknown: number;
};

/**
 * What downloads of these lengths will take, at this library's own rate for
 * the kind. Null when the library has no rate to go on yet.
 */
export function estimate(durations: (number | null)[], bps: number | null): Estimate | null {
  if (bps == null || bps <= 0) return null;
  let bytes = 0;
  let unknown = 0;
  for (const d of durations) {
    if (d == null || d <= 0) unknown++;
    else bytes += d * bps;
  }
  return { bytes: Math.round(bytes), unknown };
}

export type Pressure = {
  /** The drive would be at least DRIVE_WARN full, and what it would have left. */
  drive: { used: number; free: number } | null;
  /** More than the drive has left. */
  wontFit: boolean;
  /** The library would be past its ceiling. */
  ceiling: { after: number; over: number } | null;
};

/** What adding `adding` bytes does to the budget; null when nothing is worth saying. */
export function pressure(s: StorageStatus, adding = 0): Pressure | null {
  const drive =
    s.drive && used(s.drive, adding) >= DRIVE_WARN
      ? { used: used(s.drive, adding), free: Math.max(0, s.drive.free - adding) }
      : null;
  const wontFit = !!s.drive && adding > s.drive.free;
  const after = s.library_bytes + adding;
  const ceiling = s.ceiling != null && after > s.ceiling ? { after, over: after - s.ceiling } : null;
  return drive || wontFit || ceiling ? { drive, wontFit, ceiling } : null;
}

/** Whether this much more is what tips the budget over, rather than it being over already. */
export function tipsOver(s: StorageStatus, adding: number): boolean {
  return adding > 0 && pressure(s, 0) == null && pressure(s, adding) != null;
}

/** The words for a pressure. `adding` is what was just queued, 0 when its size is not known. */
export function sayPressure(p: Pressure, s: StorageStatus, adding: number): string {
  const would = adding > 0;
  const parts: string[] = [];
  if (p.wontFit && s.drive) {
    parts.push(`That is about ${size(adding)}, and the download drive has ${size(s.drive.free)} left: it will not all fit.`);
  } else if (p.drive) {
    parts.push(`The download drive ${would ? "would be" : "is"} ${pct(p.drive.used)} full, with ${size(p.drive.free)} left.`);
  }
  if (p.ceiling && s.ceiling != null) {
    parts.push(`The library ${would ? "would be" : "is"} ${size(p.ceiling.after)}, past its ${size(s.ceiling)} ceiling.`);
  }
  return parts.join(" ");
}

/** What audio takes next to video in this library, 0 to 1; null until both kinds have a rate. */
export function audioShare(s: StorageStatus): number | null {
  if (s.audio_bps == null || s.video_bps == null || s.video_bps <= 0) return null;
  return Math.min(1, s.audio_bps / s.video_bps);
}

/** The way out, in words, measured on this library rather than quoted. */
export function sayAudio(s: StorageStatus, videoBytes: number | null): string | null {
  const share = audioShare(s);
  if (share == null) return null;
  if (videoBytes != null && videoBytes > 0) return `As audio only it would be about ${size(videoBytes * share)}.`;
  return `Audio only takes about ${Math.max(1, Math.round(share * 100))}% of the space video does in this library.`;
}

/** The footer's meter: one line, in the warning colour when the budget is already under pressure. */
export function meter(s: StorageStatus): { text: string; warn: boolean; title: string } {
  const lib = s.ceiling != null ? `${size(s.library_bytes)} of a ${size(s.ceiling)} ceiling` : `${size(s.library_bytes)} in the library`;
  const drive = s.drive
    ? `${size(s.drive.free)} free of ${size(s.drive.total)}, ${pct(used(s.drive))} full`
    : "the download drive is not there";
  const p = pressure(s, 0);
  return {
    text: `${lib} · ${drive}`,
    warn: p != null,
    title: p
      ? sayPressure(p, s, 0)
      : `Everything in the library, on every folder. The drive is the one downloads go to; it warns at ${pct(DRIVE_WARN)} full.`,
  };
}
