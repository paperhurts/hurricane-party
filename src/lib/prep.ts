/**
 * Hurricane Party Planning (#163, D140): how pasted lines, the lists they
 * read into, a person's picks and the audio-or-video choice add up to what
 * the window shows and what one press queues.
 *
 * Rust sorts the lines (`prep::lines`), reads the lists (`prep::read`) and
 * queues the run (`prep::go`). Everything between is here, so it can be
 * tested without a window: which entries are picked, which lines repeat
 * another, what the run comes to in audio and in video, and what state the
 * window is in.
 */

import { estimate, type Estimate, type StorageStatus } from "./storage";

export type Held = { audio: boolean; video: boolean };

export type Line = {
  n: number;
  text: string;
  kind: "video" | "list" | "not_link";
  url: string | null;
  video_id: string | null;
  held: Held;
};

export type ListItem = {
  id: string;
  title: string;
  url: string;
  duration_s: number | null;
  have_audio: boolean;
  have_video: boolean;
  missing: string | null;
};

export type ListProbe = { id: string; title: string; uploader: string | null; items: ListItem[] };

export type Read = { status: "ok" | "offline" | "failed"; list: ListProbe | null; message: string | null };

/** A list's read: not asked yet or under way, or what it came to. */
export type ReadState = "reading" | Read;

export type Entry = {
  item: ListItem;
  key: string;
  picked: boolean;
  /** Held in the library for the kind being queued (D139). */
  held: boolean;
  /** The other kind is held: why a familiar entry is still picked. */
  otherHeld: boolean;
  /** The line this entry already appears on, when an earlier one has it. */
  dupOf: number | null;
};

export type Row =
  | { n: number; kind: "not_link"; text: string }
  | {
      n: number;
      kind: "video";
      text: string;
      url: string;
      key: string;
      picked: boolean;
      held: boolean;
      otherHeld: boolean;
      dupOf: number | null;
      /** A list that could not be read, queued as the one video its link names. */
      fromList: string | null;
    }
  | {
      n: number;
      kind: "list";
      text: string;
      url: string;
      state: "reading" | "ok" | "offline" | "failed";
      message: string | null;
      name: string;
      entries: Entry[];
    };

export type Picks = Record<string, boolean>;

export type GoOne = { url: string; estimate_bytes: number | null };
export type Go = { want_video: boolean; singles: GoOne[]; lists: { name: string; entries: GoOne[] }[]; remaining?: string };

export type WindowState = "empty" | "reading" | "offline" | "some failed" | "ready";

export type Plan = {
  rows: Row[];
  /** What one press queues. */
  count: number;
  go: Go;
  /** What the picks come to as audio, and as video. */
  audio: Estimate;
  video: Estimate;
  reading: number;
  offline: number;
  failed: number;
  state: WindowState;
  /** Line numbers still to be queued by a later press: lists not yet read, and lines that are not links. */
  leftOver: number[];
};

const heldFor = (h: Held, wantVideo: boolean) => (wantVideo ? h.video : h.audio);

/**
 * Add it all up. `picks` holds only what a person changed by hand, keyed by a
 * single link's URL or `listUrl#entryId`; everything else takes its default:
 * picked unless the library holds it for this kind, YouTube gave no details
 * for it (D119), or an earlier line already has it.
 */
export function plan(
  lines: Line[],
  reads: Map<string, ReadState>,
  picks: Picks,
  wantVideo: boolean,
  storage: StorageStatus | null,
): Plan {
  const seen = new Map<string, number>();
  const rows: Row[] = [];
  const go: Go = { want_video: wantVideo, singles: [], lists: [] };
  const durations: (number | null)[] = [];
  let reading = 0;
  let offline = 0;
  let failed = 0;
  const leftOver: number[] = [];
  const bps = storage ? (wantVideo ? storage.video_bps : storage.audio_bps) : null;
  const each = (duration: number | null) => (bps != null && duration != null && duration > 0 ? Math.round(duration * bps) : null);

  const single = (line: Line, url: string, fromList: string | null) => {
    const key = url;
    const id = line.video_id;
    const dupOf = id != null && seen.has(id) ? seen.get(id)! : null;
    const held = heldFor(line.held, wantVideo);
    const picked = picks[key] ?? (dupOf == null && !held);
    if (picked && id != null && dupOf == null) seen.set(id, line.n);
    rows.push({
      n: line.n,
      kind: "video",
      text: line.text,
      url,
      key,
      picked,
      held,
      otherHeld: !held && heldFor(line.held, !wantVideo),
      dupOf,
      fromList,
    });
    if (picked) {
      go.singles.push({ url, estimate_bytes: null });
      durations.push(null);
    }
  };

  for (const line of lines) {
    if (line.kind === "not_link" || line.url == null) {
      failed++;
      leftOver.push(line.n);
      rows.push({ n: line.n, kind: "not_link", text: line.text });
      continue;
    }
    if (line.kind === "video") {
      single(line, line.url, null);
      continue;
    }
    const read = reads.get(line.url) ?? "reading";
    if (read !== "reading" && read.status === "failed" && line.video_id != null) {
      // A mix, or a list that would not read: the video its link names still
      // comes, as the library does it (#137).
      single(line, line.url, read.message);
      continue;
    }
    const state = read === "reading" ? "reading" : read.status;
    const entries: Entry[] = [];
    if (read !== "reading" && read.list) {
      const picked: GoOne[] = [];
      for (const item of read.list.items) {
        const key = `${line.url}#${item.id}`;
        const dupOf = seen.has(item.id) ? seen.get(item.id)! : null;
        const held = wantVideo ? item.have_video : item.have_audio;
        const on = picks[key] ?? (dupOf == null && !held && !item.missing);
        if (on && dupOf == null) seen.set(item.id, line.n);
        entries.push({ item, key, picked: on, held, otherHeld: !held && (wantVideo ? item.have_audio : item.have_video), dupOf });
        if (on) {
          picked.push({ url: item.url, estimate_bytes: each(item.duration_s) });
          durations.push(item.duration_s);
        }
      }
      if (picked.length) go.lists.push({ name: read.list.title, entries: picked });
    }
    if (state === "reading") reading++;
    if (state === "offline") offline++;
    if (state === "failed") failed++;
    if (state !== "ok") leftOver.push(line.n);
    rows.push({
      n: line.n,
      kind: "list",
      text: line.text,
      url: line.url,
      state,
      message: read === "reading" ? null : read.message,
      name: read !== "reading" && read.list ? read.list.title : line.text,
      entries,
    });
  }

  const count = go.singles.length + go.lists.reduce((n, l) => n + l.entries.length, 0);
  const none = { bytes: 0, unknown: durations.length };
  const audio = estimate(durations, storage?.audio_bps ?? null) ?? none;
  const video = estimate(durations, storage?.video_bps ?? null) ?? none;
  const state: WindowState =
    rows.length === 0 ? "empty" : reading ? "reading" : offline ? "offline" : failed ? "some failed" : "ready";
  return { rows, count, go, audio, video, reading, offline, failed, state, leftOver };
}

/** The window's one-line headline, in the order a person under time pressure needs it. */
export function headline(p: Plan): string {
  if (p.state === "empty") return "Paste every link you want to keep";
  const parts = [`${p.count} ready`];
  if (p.reading) parts.push(`${p.reading} ${p.reading === 1 ? "list" : "lists"} reading`);
  if (p.offline) parts.push(`${p.offline} waiting for a connection`);
  if (p.failed) parts.push(`${p.failed} not saved`);
  return parts.join(" · ");
}

/** What is left in the paste box after a press: the lines that were not queued. */
export function remaining(text: string, leftOver: number[]): string {
  const keep = new Set(leftOver);
  return text
    .split(/\r?\n/)
    .filter((_, i) => keep.has(i + 1))
    .join("\n");
}
