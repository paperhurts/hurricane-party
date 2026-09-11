// The play order (#115, #116, D97): what plays next, and what plays first.
//
// The library owns it (D74): Main asks it to step, the playlist window mirrors
// it. This module is the pure half, ids in and an id out, so the rules below
// are under test and the library only has to hold the state. An "order" is the
// sequence the transport walks: the showing list as it is, or a shuffled
// permutation of it.

export const REPEATS = ["off", "one", "all"] as const;
export type Repeat = (typeof REPEATS)[number];

export function isRepeat(v: unknown): v is Repeat {
  return typeof v === "string" && (REPEATS as readonly string[]).includes(v);
}

/** One press of the repeat button: off, then one, then all, then off. */
export function nextRepeat(r: Repeat): Repeat {
  return REPEATS[(REPEATS.indexOf(r) + 1) % REPEATS.length];
}

/**
 * A shuffled order of `ids`. `first`, when it is one of them, leads: turning
 * shuffle on mid-song keeps that song playing and shuffles what follows, and
 * starting from a chosen row starts there. Fisher-Yates over the rest, with
 * the random source injectable so a test can pin it.
 */
export function shuffled(ids: readonly number[], first: number | null, rng: () => number = Math.random): number[] {
  const rest = ids.filter((id) => id !== first);
  for (let i = rest.length - 1; i > 0; i--) {
    const j = Math.floor(rng() * (i + 1));
    [rest[i], rest[j]] = [rest[j], rest[i]];
  }
  return first !== null && ids.includes(first) ? [first, ...rest] : rest;
}

/**
 * What the transport starts on when nothing is loaded: the row the playlist
 * window has selected, if it is in the order, otherwise the order's first.
 */
export function startId(order: readonly number[], selected: number | null): number | null {
  if (selected !== null && order.includes(selected)) return selected;
  return order[0] ?? null;
}

/**
 * The id `delta` steps from `current` along `order`. With nothing current,
 * forward starts at the first and back at the last, so Next and Previous do
 * something from a standing start rather than nothing. At either end the
 * order wraps when `wrap` (repeat all) and stops otherwise. A `current` that
 * has left the order (removed, or a different list now showing) is treated
 * as nothing current.
 */
export function stepId(order: readonly number[], current: number | null, delta: number, wrap: boolean): number | null {
  if (order.length === 0) return null;
  const i = current === null ? -1 : order.indexOf(current);
  if (i < 0) return delta < 0 ? order[order.length - 1] : order[0];
  const j = i + delta;
  if (j >= 0 && j < order.length) return order[j];
  if (!wrap) return null;
  return order[((j % order.length) + order.length) % order.length];
}

/**
 * What plays when a track ends on its own, as opposed to Next being pressed:
 * repeat one plays it again, repeat all walks on and wraps, off walks on and
 * stops at the end. Only an ending honours repeat one; a press of Next moves
 * on even then, because the person asked to.
 */
export function endedId(order: readonly number[], current: number | null, repeat: Repeat): number | null {
  if (repeat === "one" && current !== null) return current;
  return stepId(order, current, 1, repeat === "all");
}
