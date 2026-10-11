// A playlist row named by itself, not by where it was (#252, D197). The
// windows draw a list as it was when the library last said; an edit made
// before the next redraw names the row and the rows it was dropped between,
// and Rust finds them where they are now.

/**
 * Where a moved row is dropped: `before` the row it was dropped above (null at
 * the bottom), `after` the row above the line (null at the top).
 */
export type Place = { before: number | null; after: number | null };

/**
 * A row's key: its own id in a hand list, which may hold a track twice, and
 * the track's in the library and a smart list, which hold it once.
 */
export function rowKey(t: { id: number; item: number | null }): string {
  return t.item != null ? `i${t.item}` : `m${t.id}`;
}

/** The rows either side of a drop line at index `at` (0..n) among `ids`, null past either end. */
export function between(ids: readonly (number | null)[], at: number): { above: number | null; below: number | null } {
  return {
    above: at > 0 ? (ids[at - 1] ?? null) : null,
    below: at < ids.length ? (ids[at] ?? null) : null,
  };
}

/**
 * Where row `moved`, dropped between `above` and `below`, goes; null where it
 * would not move: dropped just above or just below itself, or alone.
 */
export function placeFor(moved: number, above: number | null, below: number | null): Place | null {
  if (above === moved || below === moved) return null;
  if (above == null && below == null) return null;
  return { before: below, after: above };
}
