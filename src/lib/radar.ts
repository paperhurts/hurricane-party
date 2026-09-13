// Cone's radar (#85, D135), the half that needs no DOM: what the windows say
// about the loop's age, which frame is showing, and where each window sits
// in the one picture behind all three. Rust fetches and redraws the frames
// (`radar.rs`); the backdrop draws them (`RadarBackdrop.svelte`).
//
// The safety rule from theme.md is this module's whole reason to be pure and
// tested: cached weather must never look current. The age is always in the
// words, and past the staleness line, or when the last fetch failed, the
// words and the backdrop say so.

import tokens from "../../design/tokens.json";
import type { WindowName } from "./skin";

export type RadarSite = { id: string; name: string; state: string; lat: number; lon: number; region: string };
export type RadarFrame = { time_ms: number; path: string };
export type RadarAlert = {
  event: string;
  severity: string;
  headline: string | null;
  sent: string | null;
  expires: string | null;
};
export type RadarStatus = {
  site: RadarSite | null;
  frames: RadarFrame[];
  last_attempt_ms: number | null;
  last_ok_ms: number | null;
  last_error: string | null;
  alerts: RadarAlert[];
  alerts_ms: number | null;
  frame_size: [number, number];
  site_y: number;
};

/** How old the newest frame may be before it is stale: the tokens'. */
export const STALE_MS = tokens.themes.cone.radar.staleAfterSeconds * 1000;

/** `live` is fresh and refreshing; `stale` is past the line; `offline` is the
 * last fetch failing, whatever the age; `none` is nothing to show. */
export type RadarState = "live" | "stale" | "offline" | "none";

export type Readout = {
  state: RadarState;
  /** The line for a title bar: `RADAR · KJAX · 14:32 EDT · 4h 12m old · OFFLINE`. */
  text: string;
  /** For the windowshade strip: `4h 12m OLD`. */
  short: string;
  /** Whether it must not look current: drawn grey, said in the warning colour. */
  warn: boolean;
};

/** "2m old", "4h 12m old", "just now". */
export function age(ms: number): string {
  const m = Math.max(0, Math.floor(ms / 60000));
  if (m < 1) return "just now";
  if (m < 60) return `${m}m old`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m old`;
}

/** "14:32 EDT" in the machine's own zone. */
export function clock(ms: number, timeZone?: string): string {
  return new Date(ms).toLocaleTimeString("en-US", {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    timeZoneName: "short",
    timeZone,
  });
}

/** What a window says about the radar at `now`. */
export function readout(status: RadarStatus | null, now: number, timeZone?: string): Readout {
  if (!status?.site) {
    return { state: "none", text: "RADAR · PICK YOUR RADAR IN THE LIBRARY", short: "NO RADAR", warn: false };
  }
  const id = status.site.id;
  const failed = !!status.last_error && (status.last_attempt_ms ?? 0) >= (status.last_ok_ms ?? 0);
  const newest = status.frames.length ? status.frames[status.frames.length - 1].time_ms : null;
  if (newest === null) {
    const why = failed ? "OFFLINE" : status.last_attempt_ms ? "NO RAIN DATA YET" : "FETCHING";
    return { state: failed ? "offline" : "none", text: `RADAR · ${id} · NO DATA · ${why}`, short: "NO DATA", warn: failed };
  }
  const old = now - newest;
  const stale = old > STALE_MS;
  const state: RadarState = failed ? "offline" : stale ? "stale" : "live";
  const label = failed ? "OFFLINE" : stale ? "STALE" : "LIVE";
  const a = age(old);
  return {
    state,
    text: `RADAR · ${id} · ${clock(newest, timeZone)} · ${a} · ${label}`,
    short: a === "just now" ? "NOW" : a.replace(" old", "").replace(" ", "").toUpperCase() + " OLD",
    warn: state !== "live",
  };
}

/** Milliseconds each frame of the loop shows, and how many of those the
 * newest holds before the loop starts again. A slow loop, no flashing: at two
 * frames a second nothing here is near the 3 Hz line (purricane.md). */
export const FRAME_MS = 500;
export const HOLD_STEPS = 6;

/** Which frame the loop shows at `now`, the same in every window, since it
 * comes from the clock. Still (reduced motion, or one frame) is the newest. */
export function frameAt(now: number, count: number, still: boolean): number {
  if (count <= 0) return -1;
  if (still || count === 1) return count - 1;
  const i = Math.floor(now / FRAME_MS) % (count + HOLD_STEPS);
  return Math.min(i, count - 1);
}

/** Where a window sits in the picture, in logical pixels from its top: the
 * three stacked in their classic order, like a made skin's picture (D127). */
export function stackTop(window: WindowName): number {
  return window === "main" ? 0 : window === "equalizer" ? 116 : 232;
}

/** An alert's issue time, "issued 14:05 EDT", or "" when the service gave none. */
export function issued(alert: RadarAlert, timeZone?: string): string {
  const t = alert.sent ? Date.parse(alert.sent) : NaN;
  return Number.isFinite(t) ? `issued ${clock(t, timeZone)}` : "";
}

/** Whether the alerts are too old to be read as current. */
export function alertsStale(status: RadarStatus | null, now: number): boolean {
  return !status?.alerts_ms || now - status.alerts_ms > STALE_MS;
}
