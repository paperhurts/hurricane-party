import { describe, expect, it } from "vitest";
import cases from "./fold.cases.json";
import { draftOf, fold, nameFor, ruleFromFind, ruleOf, sayRule, type Rule } from "./smart";

const roots = [
  { id: 1, label: "mp3" },
  { id: 4, label: "hp" },
];

describe("a search saved as a smart playlist (#165, D144)", () => {
  it("folds on the cases smart::fold is tested on", () => {
    for (const [input, folded] of cases as [string, string][]) expect(fold(input), input).toBe(folded);
  });

  it("takes the find bar's words, type and order, and nothing it did not say", () => {
    expect(ruleFromFind("", "all", "added")).toEqual({ v: 1 });
    expect(ruleFromFind("  ian   stocker ", "audio", "title")).toEqual({
      v: 1,
      words: "ian stocker",
      kind: "audio",
      sort: "title",
    });
  });

  it("offers a name from what was searched", () => {
    expect(nameFor({ v: 1, words: "ian stocker" })).toBe("Ian stocker");
    expect(nameFor({ v: 1, words: "storm", kind: "video" })).toBe("Storm · videos");
    expect(nameFor({ v: 1, kind: "video" })).toBe("Videos");
    expect(nameFor({ v: 1, sort: "longest" })).toBe("Longest first");
  });
});

describe("what a smart list says it fills itself with", () => {
  it("says every condition, the order and the limit", () => {
    expect(sayRule({ v: 1 }, roots)).toBe("Fills itself with the whole library, newest first.");
    const all: Rule = {
      v: 1,
      words: "stocker",
      kind: "audio",
      added_within_days: 7,
      longer_than_s: 90,
      shorter_than_s: 1200,
      root: 4,
      sort: "longest",
      limit: 50,
    };
    expect(sayRule(all, roots)).toBe(
      "Fills itself with “stocker” · audio · added in the last 7 days · longer than 1.5 min · shorter than 20 min · on hp, longest first, the first 50.",
    );
    expect(sayRule({ v: 1, added_within_days: 1, root: 9 }, roots)).toBe(
      "Fills itself with added today · on a root that is gone, newest first.",
    );
  });
});

describe("the rule editor", () => {
  const blank = draftOf({ v: 1 });

  it("round-trips a rule through its fields", () => {
    const rule: Rule = { v: 1, words: "ian", kind: "video", added_within_days: 7, longer_than_s: 1200, root: 4, sort: "artist", limit: 10 };
    expect(ruleOf(draftOf(rule))).toEqual(rule);
    expect(ruleOf(blank)).toEqual({ v: 1 });
  });

  it("reads minutes as minutes", () => {
    expect(ruleOf({ ...blank, longerMin: "1.5", shorterMin: "20" })).toEqual({ v: 1, longer_than_s: 90, shorter_than_s: 1200 });
  });

  it("says what is wrong rather than saving it", () => {
    expect(ruleOf({ ...blank, days: "a week" })).toBe("Days has to be a whole number above nothing.");
    expect(ruleOf({ ...blank, limit: "0" })).toBe("The limit has to be a whole number above nothing.");
    expect(ruleOf({ ...blank, longerMin: "-3" })).toBe("Longer than has to be a number of minutes above nothing.");
    expect(ruleOf({ ...blank, longerMin: "20", shorterMin: "5" })).toBe("Nothing is longer than that and shorter than this.");
  });
});
