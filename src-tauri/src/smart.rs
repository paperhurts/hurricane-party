//! Smart playlists (#165, D144): a list that fills itself from a rule.
//!
//! A rule is data from a fixed vocabulary, the things the library already
//! knows about a track: words in its title and artist, its kind, when it was
//! added, how long it is, which root it is under, and an order with an
//! optional limit. Every condition must hold (the owner's call, the way the
//! search box treats words). It is kept in `playlists.rule_json` with a
//! version, read into a typed [`Rule`], and refused by the field's name when
//! a field is not one of these. It never becomes SQL: the rows are read and
//! the rule is applied to them here, so the queue (D120), SEL (D124), the
//! playlist window and anything on the control pipe see a smart list the way
//! they see any other, through `playlist::items`.
//!
//! The words match exactly as the library's search does (D121), so a search
//! saved as a list and the same search typed again never disagree: each word
//! must appear in "title artist", with both folded blind to case and accents
//! by the same steps `fold` takes in `src/App.svelte`.

use crate::playlist::MediaRow;
use icu_normalizer::DecomposingNormalizerBorrowed;
use icu_properties::props::{GeneralCategory, GeneralCategoryGroup};
use icu_properties::CodePointMapData;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// The version a rule is written in. A rule of another is refused rather
/// than half read.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Audio,
    Video,
}

/// D121's four orders.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    /// Newest first, the library's own order.
    #[default]
    Added,
    Title,
    Artist,
    Longest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub v: u32,
    /// Every word must appear in the title or artist.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub words: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    /// Added within the last this many days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_within_days: Option<u32>,
    /// Longer than this many seconds. A track of no known length is neither
    /// longer nor shorter than anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longer_than_s: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shorter_than_s: Option<u32>,
    /// Under this library root (D28).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<i64>,
    #[serde(default)]
    pub sort: Sort,
    /// Only the first this many, after the order: "the newest 50".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl Rule {
    /// A rule as stored, refused with the reason when it is not one this app
    /// reads: an unknown field by its name, another version by its number.
    pub fn parse(json: &str) -> Result<Rule, String> {
        let rule: Rule =
            serde_json::from_str(json).map_err(|e| format!("this rule cannot be read: {e}"))?;
        rule.check()?;
        Ok(rule)
    }

    /// What a rule has to be, whichever way it arrived.
    pub fn check(&self) -> Result<(), String> {
        if self.v != VERSION {
            return Err(format!(
                "this rule is version {}, and this app reads version {VERSION}",
                self.v
            ));
        }
        if self.limit == Some(0) {
            return Err("a limit of none would make an empty list".into());
        }
        if let (Some(long), Some(short)) = (self.longer_than_s, self.shorter_than_s) {
            if long >= short {
                return Err("nothing is longer than that and shorter than this".into());
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Blind to case and accents, as the library's search box is: decompose,
/// drop every mark, lowercase. "Beyoncé" is "beyonce".
pub fn fold(s: &str) -> String {
    let marks = CodePointMapData::<GeneralCategory>::new();
    DecomposingNormalizerBorrowed::new_nfd()
        .normalize(s)
        .chars()
        .filter(|c| !GeneralCategoryGroup::Mark.contains(marks.get(*c)))
        .collect::<String>()
        .to_lowercase()
}

/// A row, with the one thing the list query adds that the player does not
/// need: when it came into the library.
#[derive(Clone)]
pub struct Candidate {
    pub row: MediaRow,
    pub added_at: i64,
}

/// What a rule makes of the library, in the rule's order. `rows` come newest
/// first, as the library lists them; `now` is seconds since the epoch.
pub fn select(rule: &Rule, rows: &[Candidate], now: i64) -> Vec<MediaRow> {
    let terms: Vec<String> = fold(&rule.words)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let since = rule.added_within_days.map(|d| now - i64::from(d) * 86_400);
    let mut kept: Vec<MediaRow> = rows
        .iter()
        .filter(|c| since.is_none_or(|s| c.added_at >= s))
        .map(|c| c.row.clone())
        .filter(|t| {
            rule.kind.is_none_or(|k| {
                t.kind
                    == match k {
                        Kind::Audio => "audio",
                        Kind::Video => "video",
                    }
            })
        })
        .filter(|t| rule.root.is_none_or(|r| t.root_id == r))
        .filter(|t| {
            rule.longer_than_s
                .is_none_or(|s| t.duration_s.is_some_and(|d| d > f64::from(s)))
        })
        .filter(|t| {
            rule.shorter_than_s
                .is_none_or(|s| t.duration_s.is_some_and(|d| d < f64::from(s)))
        })
        .filter(|t| {
            if terms.is_empty() {
                return true;
            }
            let hay = fold(&format!(
                "{} {}",
                t.title,
                t.uploader.as_deref().unwrap_or("")
            ));
            terms.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect();

    // Stable, as the library's own sort is, so ties keep the newest first.
    match rule.sort {
        Sort::Added => {}
        Sort::Title => kept.sort_by(|a, b| natural(&a.title, &b.title)),
        Sort::Artist => kept.sort_by(|a, b| {
            // No artist sorts after every artist, as it does in the library.
            match (&a.uploader, &b.uploader) {
                (Some(x), Some(y)) => natural(x, y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
            .then_with(|| natural(&a.title, &b.title))
        }),
        Sort::Longest => kept.sort_by(|a, b| {
            let (x, y) = (a.duration_s.unwrap_or(-1.0), b.duration_s.unwrap_or(-1.0));
            y.partial_cmp(&x).unwrap_or(Ordering::Equal)
        }),
    }
    if let Some(n) = rule.limit {
        kept.truncate(n as usize);
    }
    for (i, t) in kept.iter_mut().enumerate() {
        t.position = Some(i as i64);
    }
    kept
}

/// Text in the order the library sorts it: blind to case and accents, and a
/// run of digits read as a number, so "Track 2" comes before "Track 10".
fn natural(a: &str, b: &str) -> Ordering {
    let (a, b) = (fold(a), fold(b));
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        n.push(c);
                        it.next();
                    }
                    n
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let by = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if by != Ordering::Equal {
                    return by;
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x.next();
                y.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_789_600_000;

    fn row(id: i64, title: &str, uploader: Option<&str>, kind: &str, dur: Option<f64>) -> MediaRow {
        MediaRow {
            id,
            title: title.into(),
            uploader: uploader.map(Into::into),
            duration_s: dur,
            filesize: None,
            kind: kind.into(),
            root_id: 1,
            path: String::new(),
            position: None,
            integrity: None,
        }
    }

    /// Newest first, as the library query hands them over.
    fn library() -> Vec<Candidate> {
        vec![
            Candidate {
                row: row(6, "Track 10", None, "audio", Some(60.0)),
                added_at: NOW - DAY / 2,
            },
            Candidate {
                row: row(5, "Track 2", Some("Ian Stocker"), "audio", Some(90.0)),
                added_at: NOW - 2 * DAY,
            },
            Candidate {
                row: row(4, "Déjà Vu", Some("Beyoncé"), "audio", Some(240.0)),
                added_at: NOW - 3 * DAY,
            },
            Candidate {
                row: row(3, "Storm documentary", Some("NWS"), "video", Some(1800.0)),
                added_at: NOW - 10 * DAY,
            },
            Candidate {
                row: row(2, "At Port", Some("Ian Stocker"), "audio", None),
                added_at: NOW - 20 * DAY,
            },
            Candidate {
                row: MediaRow {
                    root_id: 2,
                    ..row(1, "Beach", Some("Ian Stocker"), "audio", Some(224.0))
                },
                added_at: NOW - 30 * DAY,
            },
        ]
    }

    fn rule() -> Rule {
        Rule::parse(r#"{"v":1}"#).unwrap()
    }

    fn ids(rule: &Rule) -> Vec<i64> {
        select(rule, &library(), NOW).iter().map(|t| t.id).collect()
    }

    /// The cases the webview's `fold` is tested on too (`src/lib/smart.test.ts`),
    /// so a saved search and a typed one cannot drift apart: accents, a
    /// letter that is not a letter plus a mark, Greek's final sigma.
    #[test]
    fn folding_is_blind_to_case_and_accents_as_the_search_box_is() {
        let cases: Vec<(String, String)> =
            serde_json::from_str(include_str!("../../src/lib/fold.cases.json")).unwrap();
        assert!(cases.len() > 10);
        for (input, folded) in cases {
            assert_eq!(fold(&input), folded, "{input}");
        }
    }

    #[test]
    fn an_empty_rule_is_the_library_newest_first() {
        assert_eq!(ids(&rule()), [6, 5, 4, 3, 2, 1]);
        let rows = select(&rule(), &library(), NOW);
        let positions: Vec<i64> = rows.iter().filter_map(|t| t.position).collect();
        assert_eq!(positions, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn every_word_must_be_in_the_title_or_the_artist() {
        let r = Rule {
            words: "ian port".into(),
            ..rule()
        };
        assert_eq!(ids(&r), [2]);
        let r = Rule {
            words: "  stocker  ".into(),
            ..rule()
        };
        assert_eq!(ids(&r), [5, 2, 1]);
        let r = Rule {
            words: "beyonce deja".into(),
            ..rule()
        };
        assert_eq!(ids(&r), [4]);
    }

    #[test]
    fn kind_root_age_and_length_each_narrow() {
        assert_eq!(
            ids(&Rule {
                kind: Some(Kind::Video),
                ..rule()
            }),
            [3]
        );
        assert_eq!(
            ids(&Rule {
                root: Some(2),
                ..rule()
            }),
            [1]
        );
        assert_eq!(
            ids(&Rule {
                added_within_days: Some(7),
                ..rule()
            }),
            [6, 5, 4]
        );
        // No known length is neither longer nor shorter.
        assert_eq!(
            ids(&Rule {
                longer_than_s: Some(200),
                ..rule()
            }),
            [4, 3, 1]
        );
        assert_eq!(
            ids(&Rule {
                shorter_than_s: Some(100),
                ..rule()
            }),
            [6, 5]
        );
        // And all of them at once.
        let all = Rule {
            words: "stocker".into(),
            kind: Some(Kind::Audio),
            added_within_days: Some(25),
            ..rule()
        };
        assert_eq!(ids(&all), [5, 2]);
    }

    #[test]
    fn the_four_orders_and_a_limit() {
        assert_eq!(
            ids(&Rule {
                sort: Sort::Title,
                ..rule()
            }),
            [2, 1, 4, 3, 5, 6]
        );
        // No artist last; ties by title.
        assert_eq!(
            ids(&Rule {
                sort: Sort::Artist,
                ..rule()
            }),
            [4, 2, 1, 5, 3, 6]
        );
        // No known length last.
        assert_eq!(
            ids(&Rule {
                sort: Sort::Longest,
                ..rule()
            }),
            [3, 4, 1, 5, 6, 2]
        );
        assert_eq!(
            ids(&Rule {
                limit: Some(2),
                ..rule()
            }),
            [6, 5]
        );
        assert_eq!(
            ids(&Rule {
                sort: Sort::Longest,
                limit: Some(1),
                ..rule()
            }),
            [3]
        );
    }

    #[test]
    fn a_number_in_a_title_sorts_as_a_number() {
        assert_eq!(natural("Track 2", "Track 10"), Ordering::Less);
        assert_eq!(natural("track 02", "Track 2"), Ordering::Equal);
        assert_eq!(natural("Beach", "beach"), Ordering::Equal);
    }

    #[test]
    fn a_rule_is_refused_by_what_is_wrong_with_it() {
        let unknown = Rule::parse(r#"{"v":1,"played_most":true}"#).unwrap_err();
        assert!(unknown.contains("played_most"), "{unknown}");
        let later = Rule::parse(r#"{"v":2}"#).unwrap_err();
        assert!(later.contains("version 2"), "{later}");
        assert!(Rule::parse(r#"{"v":1,"kind":"podcast"}"#).is_err());
        assert!(Rule::parse(r#"{"v":1,"limit":0}"#).is_err());
        assert!(Rule::parse(r#"{"v":1,"longer_than_s":600,"shorter_than_s":60}"#).is_err());
    }

    #[test]
    fn a_rule_is_stored_with_only_what_it_says() {
        let r = Rule {
            words: "ian".into(),
            kind: Some(Kind::Audio),
            sort: Sort::Longest,
            ..rule()
        };
        let json = r.to_json();
        assert_eq!(
            json,
            r#"{"v":1,"words":"ian","kind":"audio","sort":"longest"}"#
        );
        assert_eq!(Rule::parse(&json).unwrap(), r);
    }
}
