/**
 * Smart playlists (#165, D144): the rule as the webview holds it, and the
 * words for it.
 *
 * Rust owns what a rule means (`src-tauri/src/smart.rs`): it refuses a field
 * it does not know, and it is where the rows are matched, so the queue and
 * the playlist window see a smart list like any other. This is the find bar
 * turned into a rule, the rule editor's fields, and the one line that says
 * what a list fills itself with.
 */

export type Kind = "audio" | "video";
export type Sort = "added" | "title" | "artist" | "longest";

/** A rule, as `smart::Rule` reads it. Absent is "any". */
export type Rule = {
  v: 1;
  words?: string;
  kind?: Kind;
  added_within_days?: number;
  longer_than_s?: number;
  shorter_than_s?: number;
  root?: number;
  sort?: Sort;
  limit?: number;
};

/**
 * Case- and accent-blind, so "beyonce" finds "Beyoncé" (D121). The same steps
 * as `smart::fold`, and the two are tested on the same cases
 * (`fold.cases.json`), so a saved search matches what the typed one did.
 */
export const fold = (s: string) => s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

/** What the find bar is showing, as a rule: its words, its type, its order. */
export function ruleFromFind(search: string, kind: "all" | Kind, sort: Sort): Rule {
  const rule: Rule = { v: 1 };
  const words = search.trim().replace(/\s+/g, " ");
  if (words) rule.words = words;
  if (kind !== "all") rule.kind = kind;
  if (sort !== "added") rule.sort = sort;
  return rule;
}

const SORT_WORDS: Record<Sort, string> = {
  added: "newest first",
  title: "by title",
  artist: "by artist",
  longest: "longest first",
};

/** A name to offer when a search is saved: what was searched for. */
export function nameFor(rule: Rule): string {
  const kind = rule.kind === "video" ? "videos" : rule.kind === "audio" ? "audio" : "";
  const said = [rule.words, kind].filter(Boolean).join(" · ") || SORT_WORDS[rule.sort ?? "added"];
  return said.charAt(0).toUpperCase() + said.slice(1);
}

const minutes = (s: number) => (s % 60 === 0 ? `${s / 60} min` : `${Math.round((s / 60) * 10) / 10} min`);

/** The one line above a smart list: what it fills itself with. */
export function sayRule(rule: Rule, roots: readonly { id: number; label: string }[]): string {
  const parts: string[] = [];
  if (rule.words) parts.push(`“${rule.words}”`);
  if (rule.kind) parts.push(rule.kind === "video" ? "videos" : "audio");
  if (rule.added_within_days != null)
    parts.push(rule.added_within_days === 1 ? "added today" : `added in the last ${rule.added_within_days} days`);
  if (rule.longer_than_s != null) parts.push(`longer than ${minutes(rule.longer_than_s)}`);
  if (rule.shorter_than_s != null) parts.push(`shorter than ${minutes(rule.shorter_than_s)}`);
  if (rule.root != null) parts.push(`on ${roots.find((r) => r.id === rule.root)?.label ?? "a root that is gone"}`);
  const order = SORT_WORDS[rule.sort ?? "added"];
  const limit = rule.limit != null ? `, the first ${rule.limit}` : "";
  return parts.length
    ? `Fills itself with ${parts.join(" · ")}, ${order}${limit}.`
    : `Fills itself with the whole library, ${order}${limit}.`;
}

/** The rule editor's fields, as typed. Durations are in minutes, which is how a person thinks of them. */
export type Draft = {
  words: string;
  kind: "all" | Kind;
  days: string;
  longerMin: string;
  shorterMin: string;
  root: string;
  sort: Sort;
  limit: string;
};

export function draftOf(rule: Rule): Draft {
  const n = (v: number | undefined, scale = 1) => (v == null ? "" : String(Math.round((v / scale) * 10) / 10));
  return {
    words: rule.words ?? "",
    kind: rule.kind ?? "all",
    days: n(rule.added_within_days),
    longerMin: n(rule.longer_than_s, 60),
    shorterMin: n(rule.shorter_than_s, 60),
    root: rule.root == null ? "" : String(rule.root),
    sort: rule.sort ?? "added",
    limit: n(rule.limit),
  };
}

/** The fields as a rule, or what is wrong with them in the person's words. */
export function ruleOf(d: Draft): Rule | string {
  const rule = ruleFromFind(d.words, d.kind, d.sort);
  const whole = (s: string, what: string): number | string | undefined => {
    if (!s.trim()) return undefined;
    const v = Number(s);
    return Number.isInteger(v) && v > 0 ? v : `${what} has to be a whole number above nothing.`;
  };
  const mins = (s: string, what: string): number | string | undefined => {
    if (!s.trim()) return undefined;
    const v = Number(s);
    return Number.isFinite(v) && v > 0 ? Math.round(v * 60) : `${what} has to be a number of minutes above nothing.`;
  };
  const days = whole(d.days, "Days");
  const limit = whole(d.limit, "The limit");
  const longer = mins(d.longerMin, "Longer than");
  const shorter = mins(d.shorterMin, "Shorter than");
  for (const v of [days, limit, longer, shorter]) if (typeof v === "string") return v;
  if (typeof days === "number") rule.added_within_days = days;
  if (typeof limit === "number") rule.limit = limit;
  if (typeof longer === "number") rule.longer_than_s = longer;
  if (typeof shorter === "number") rule.shorter_than_s = shorter;
  if (typeof longer === "number" && typeof shorter === "number" && longer >= shorter)
    return "Nothing is longer than that and shorter than this.";
  if (d.root) rule.root = Number(d.root);
  return rule;
}
