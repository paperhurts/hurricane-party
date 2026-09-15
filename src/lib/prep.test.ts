import { describe, expect, it } from "vitest";
import { headline, plan, remaining, type Line, type ListItem, type ReadState } from "./prep";
import type { StorageStatus } from "./storage";

const none = { audio: false, video: false };
const video = (n: number, id: string | null, held = none): Line => ({
  n,
  text: `https://youtu.be/${id ?? "x"}`,
  kind: "video",
  url: `https://youtu.be/${id ?? "x"}`,
  video_id: id,
  held,
});
const list = (n: number, id: string, videoId: string | null = null): Line => ({
  n,
  text: `https://www.youtube.com/playlist?list=${id}`,
  kind: "list",
  url: `https://www.youtube.com/playlist?list=${id}`,
  video_id: videoId,
  held: none,
});
const item = (id: string, over: Partial<ListItem> = {}): ListItem => ({
  id,
  title: `Song ${id}`,
  url: `https://www.youtube.com/watch?v=${id}`,
  duration_s: 200,
  have_audio: false,
  have_video: false,
  missing: null,
  ...over,
});
const storage: StorageStatus = {
  library_bytes: 0,
  drive: { free: 1e12, total: 2e12 },
  ceiling: null,
  audio_bps: 24_000,
  video_bps: 240_000,
};
const ok = (title: string, items: ListItem[]): ReadState => ({
  status: "ok",
  list: { id: "L", title, uploader: null, items },
  message: null,
});

describe("a prep run adds up the way the library would queue it (#163)", () => {
  it("is empty with nothing pasted", () => {
    const p = plan([], new Map(), {}, false, storage);
    expect(p.state).toBe("empty");
    expect(headline(p)).toBe("Paste every link you want to keep");
  });

  it("queues single links and each list's new entries, with an estimate for what has a length", () => {
    const lines = [video(1, "aaaaaaaaaaa"), list(2, "PL1"), { n: 3, text: "groceries", kind: "not_link", url: null, video_id: null, held: none } as Line];
    const reads = new Map([[lines[1].url!, ok("Storm songs", [item("b1"), item("b2", { have_audio: true }), item("b3", { missing: "private" })])]]);
    const p = plan(lines, reads, {}, false, storage);
    expect(p.count).toBe(2);
    expect(p.go).toEqual({
      want_video: false,
      singles: [{ url: "https://youtu.be/aaaaaaaaaaa", estimate_bytes: null }],
      lists: [{ name: "Storm songs", entries: [{ url: "https://www.youtube.com/watch?v=b1", estimate_bytes: 4_800_000 }] }],
    });
    // The single link has no length until its download starts.
    expect(p.audio).toEqual({ bytes: 4_800_000, unknown: 1 });
    expect(p.video).toEqual({ bytes: 48_000_000, unknown: 1 });
    expect(p.state).toBe("some failed");
    expect(p.leftOver).toEqual([3]);
    expect(headline(p)).toBe("2 ready · 1 not saved");
  });

  it("counts a video once, on the first line that has it", () => {
    const lines = [video(1, "same12345ab"), list(2, "PL1"), video(3, "same12345ab")];
    const reads = new Map([[lines[1].url!, ok("L", [item("same12345ab"), item("other")])]]);
    const p = plan(lines, reads, {}, false, storage);
    expect(p.count).toBe(2);
    const entries = p.rows[1].kind === "list" ? p.rows[1].entries : [];
    expect(entries.map((e) => [e.item.id, e.picked, e.dupOf])).toEqual([
      ["same12345ab", false, 1],
      ["other", true, null],
    ]);
    expect(p.rows[2]).toMatchObject({ kind: "video", picked: false, dupOf: 1 });
  });

  it("what the library holds is per kind, and follows the choice (D139)", () => {
    const lines = [video(1, "held1234567", { audio: true, video: false })];
    expect(plan(lines, new Map(), {}, false, storage).rows[0]).toMatchObject({ picked: false, held: true });
    expect(plan(lines, new Map(), {}, true, storage).rows[0]).toMatchObject({ picked: true, held: false, otherHeld: true });
  });

  it("a person's own picks win over every default", () => {
    const lines = [list(1, "PL1")];
    const url = lines[0].url!;
    const reads = new Map([[url, ok("L", [item("one"), item("gone", { missing: "no details" })])]]);
    const p = plan(lines, reads, { [`${url}#one`]: false, [`${url}#gone`]: true }, false, storage);
    expect(p.go.lists[0].entries.map((e) => e.url)).toEqual(["https://www.youtube.com/watch?v=gone"]);
  });

  it("a list still reading, or waiting for a connection, stays behind for a later press", () => {
    const lines = [list(1, "PL1"), list(2, "PL2")];
    const reads = new Map<string, ReadState>([
      [lines[1].url!, { status: "offline", list: null, message: "No connection." }],
    ]);
    const p = plan(lines, reads, {}, false, storage);
    expect(p.state).toBe("reading");
    expect(p.leftOver).toEqual([1, 2]);
    expect(headline(p)).toBe("0 ready · 1 list reading · 1 waiting for a connection");
    const after = plan(lines, new Map<string, ReadState>([[lines[0].url!, ok("A", [item("a")])], ...reads]), {}, false, storage);
    expect(after.state).toBe("offline");
  });

  it("a list that will not read still brings the video its link names", () => {
    const lines = [list(1, "RDmix", "vvvvvvvvvvv")];
    const reads = new Map<string, ReadState>([[lines[0].url!, { status: "failed", list: null, message: "a mix" }]]);
    const p = plan(lines, reads, {}, false, storage);
    expect(p.rows[0]).toMatchObject({ kind: "video", picked: true, fromList: "a mix" });
    expect(p.count).toBe(1);
  });

  it("with no rate to go on, everything picked is not known yet", () => {
    const lines = [list(1, "PL1")];
    const reads = new Map([[lines[0].url!, ok("L", [item("a"), item("b")])]]);
    const p = plan(lines, reads, {}, false, { ...storage, audio_bps: null, video_bps: null });
    expect(p.audio).toEqual({ bytes: 0, unknown: 2 });
    expect(p.go.lists[0].entries[0].estimate_bytes).toBeNull();
  });
});

describe("the paste box after a press keeps what was not queued (#163)", () => {
  it("keeps those lines, by number, and drops the rest", () => {
    expect(remaining("a\r\n\r\nb\nc", [3, 4])).toBe("b\nc");
    expect(remaining("a\nb", [])).toBe("");
  });
});
