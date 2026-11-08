/**
 * Tracks on a drive that is not plugged in (D143).
 *
 * A flash drive's tracks stay in the library while it is out (D28): their
 * rows, their places in playlists, their order. What changes is what the
 * library window lists and what plays. They are left out of both, one line
 * says so, and a button lists them greyed. They never play while the drive is
 * out, shown or not: a row that cannot open is not one the transport should
 * land on.
 *
 * Rust says which roots are there (`list_roots`, watched every ten seconds,
 * sooner when a drive is ejected) and which root each track is under. This is
 * the rule and the words.
 */

export type RootPresence = { id: number; label: string; present: boolean };
export type OnRoot = { root_id: number };

/** The ids of the roots that are not there. */
export function outRoots(roots: readonly RootPresence[]): Set<number> {
  return new Set(roots.filter((r) => !r.present).map((r) => r.id));
}

/**
 * Whether a track's drive is plugged in. A root nobody has listed counts as
 * there, so a download into a folder that has just become a root is not
 * hidden for the moment before the roots are listed again.
 */
export function onDrive(t: OnRoot, out: ReadonlySet<number>): boolean {
  return !out.has(t.root_id);
}

/** "hp", "hp and stick", "hp, stick and backup". */
function names(labels: readonly string[]): string {
  return labels.length < 2 ? (labels[0] ?? "") : `${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1]}`;
}

/**
 * The line above a list with tracks on a drive that is out, or null when it
 * has none. `showing` is whether they are listed, greyed.
 */
export function sayOut(rows: readonly OnRoot[], roots: readonly RootPresence[], showing: boolean): string | null {
  const out = outRoots(roots);
  const away = rows.filter((t) => !onDrive(t, out));
  if (!away.length) return null;
  const held = new Set(away.map((t) => t.root_id));
  const labels = roots.filter((r) => held.has(r.id)).map((r) => r.label);
  const one = away.length === 1;
  const tracks = one ? "1 track" : `${away.length} tracks`;
  const drives = labels.length === 1;
  return showing
    ? `${tracks} on ${names(labels)} won't play until ${drives ? "it's" : "they're"} plugged in.`
    : `${tracks} ${one ? "is" : "are"} hidden: ${one ? "it's" : "they're"} on ${names(labels)}, which ${drives ? "isn't" : "aren't"} plugged in.`;
}
