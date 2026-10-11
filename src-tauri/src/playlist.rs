//! Playlists and library queries.

use crate::db::{self, DbError};
use crate::smart;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub count: i64,
    /// Of `count`, the tracks on a drive that is not plugged in (D143), which
    /// the library leaves out of the list unless asked to show them.
    pub offline: i64,
    pub created_at: i64,
    /// A smart playlist fills itself from its rule (#165, D144): its rows
    /// are what the rule matches now, and nobody adds, removes or orders
    /// them by hand.
    pub smart: bool,
    /// The rule, when it is smart and the rule reads.
    pub rule: Option<smart::Rule>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaRow {
    pub id: i64,
    pub title: String,
    pub uploader: Option<String>,
    pub duration_s: Option<f64>,
    pub filesize: Option<i64>,
    /// "audio" | "video" — the frontend routes video to its own window (D13).
    pub kind: String,
    /// Which root it is under, so the library can tell a track on an
    /// unplugged drive from one whose file has gone (D28, D143).
    pub root_id: i64,
    /// Absolute path, rebuilt from (root_id, relpath) at read time. The DB never
    /// stores it (D28) — this is derived for the player, not persisted.
    pub path: String,
    /// Present only when the row came from a playlist query.
    pub position: Option<i64>,
    /// The row's own id, its `playlist_items` rowid, in the hand list it was
    /// read from (#252, D197): what an edit names, since a list may hold a
    /// track twice. None in the library and in a smart list.
    pub item: Option<i64>,
    /// What the last integrity check made of it (#164, D142): `changed`,
    /// `unreadable`, or nothing at all.
    pub integrity: Option<String>,
}

/// Where a moved row is dropped, by the rows on either side of the line
/// (#252, D197): `before` the row it was dropped above (None at the bottom),
/// `after` the row above the line (None at the top). Rows, not indexes, so a
/// drop drawn before the last edit came back still lands where it was made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct Place {
    pub before: Option<i64>,
    pub after: Option<i64>,
}

const MEDIA_SELECT: &str = "
    SELECT m.id, m.title, m.uploader, m.duration_s, m.filesize, m.kind,
           m.root_id, r.path AS root_path, m.relpath, m.integrity";

fn row_to_media(r: &rusqlite::Row, position: Option<i64>) -> rusqlite::Result<MediaRow> {
    let root: String = r.get("root_path")?;
    let rel: String = r.get("relpath")?;
    Ok(MediaRow {
        id: r.get("id")?,
        title: r.get("title")?,
        uploader: r.get("uploader")?,
        duration_s: r.get("duration_s")?,
        filesize: r.get("filesize")?,
        kind: r.get("kind")?,
        root_id: r.get("root_id")?,
        path: std::path::Path::new(&root)
            .join(&rel)
            .to_string_lossy()
            .to_string(),
        position,
        item: None,
        integrity: r.get("integrity")?,
    })
}

/// The library. Flat and sortable (O6) — not a tree.
pub fn list_media(conn: &Connection) -> Result<Vec<MediaRow>, DbError> {
    let sql = format!(
        "{MEDIA_SELECT} FROM media m
         JOIN library_roots r ON r.id = m.root_id
         ORDER BY m.added_at DESC, m.id DESC"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([], |r| row_to_media(r, None))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// The library as a smart list reads it: in the library's own order, the
/// ties broken the same way, with when each row came in (#165).
fn candidates(conn: &Connection) -> Result<Vec<smart::Candidate>, DbError> {
    let sql = format!(
        "{MEDIA_SELECT}, m.added_at FROM media m
         JOIN library_roots r ON r.id = m.root_id
         ORDER BY m.added_at DESC, m.id DESC"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([], |r| {
        Ok(smart::Candidate {
            row: row_to_media(r, None)?,
            added_at: r.get("added_at")?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// The roots whose drive is out (D143). A question for the disk, asked once
/// per root rather than once per track.
fn absent_roots(conn: &Connection) -> Result<std::collections::HashSet<i64>, DbError> {
    Ok(crate::localimport::list_roots(conn)?
        .into_iter()
        .filter(|r| !r.present)
        .map(|r| r.id)
        .collect())
}

/// The pipe's `search` (#182): tracks whose title or artist hold every word,
/// matched as the library's search box matches (`smart::words_match`), newest
/// first, leaving out any on a drive that is out (D143). How many matched,
/// and at most `limit` of them.
pub fn search(conn: &Connection, q: &str, limit: usize) -> Result<(usize, Vec<MediaRow>), DbError> {
    let absent = absent_roots(conn)?;
    let terms = smart::terms(q);
    let hits: Vec<MediaRow> = list_media(conn)?
        .into_iter()
        .filter(|t| !absent.contains(&t.root_id))
        .filter(|t| smart::words_match(&terms, &t.title, t.uploader.as_deref()))
        .collect();
    let total = hits.len();
    Ok((total, hits.into_iter().take(limit).collect()))
}

/// What `queue_playlist` would start (#182): the list's name and how many of
/// its tracks can play now. `None` when there is no such list.
pub fn playable(conn: &Connection, id: i64) -> Result<Option<(String, usize)>, DbError> {
    let name: Option<String> = conn
        .query_row("SELECT name FROM playlists WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(name) = name else {
        return Ok(None);
    };
    let absent = absent_roots(conn)?;
    let n = items(conn, id)?
        .iter()
        .filter(|t| !absent.contains(&t.root_id))
        .count();
    Ok(Some((name, n)))
}

/// A smart list's rule, `None` for a list made by hand. A rule that does not
/// read is an error that says why, not an empty list pretending all is well.
fn rule_of(conn: &Connection, id: i64) -> Result<Option<smart::Rule>, DbError> {
    let row: Option<(i64, Option<String>)> = conn
        .query_row(
            "SELECT COALESCE(is_smart, 0), rule_json FROM playlists WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((1, json)) => smart::Rule::parse(json.as_deref().unwrap_or(""))
            .map(Some)
            .map_err(DbError::Io),
        _ => Ok(None),
    }
}

/// Refuse a hand edit on a smart list (#165): the rule is what decides its
/// rows and their order. In Rust, so no window can get round it.
fn by_hand(conn: &Connection, id: i64) -> Result<(), DbError> {
    let smart: Option<(i64, String)> = conn
        .query_row(
            "SELECT COALESCE(is_smart, 0), name FROM playlists WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match smart {
        Some((1, name)) => Err(DbError::Io(format!(
            "“{name}” is a smart playlist: it fills itself from its rule, so nothing is added, removed or moved in it by hand"
        ))),
        _ => Ok(()),
    }
}

/// Make a smart playlist, at the bottom of the order like any new list.
pub fn create_smart(conn: &Connection, name: &str, rule: &smart::Rule) -> Result<i64, DbError> {
    rule.check().map_err(DbError::Io)?;
    let id = create(conn, name)?;
    conn.execute(
        "UPDATE playlists SET is_smart = 1, rule_json = ?1 WHERE id = ?2",
        params![rule.to_json(), id],
    )?;
    Ok(id)
}

/// Change a smart playlist's rule. A list made by hand has no rule to change.
pub fn set_rule(conn: &Connection, id: i64, rule: &smart::Rule) -> Result<(), DbError> {
    rule.check().map_err(DbError::Io)?;
    let n = conn.execute(
        "UPDATE playlists SET rule_json = ?1 WHERE id = ?2 AND is_smart = 1",
        params![rule.to_json(), id],
    )?;
    if n == 0 {
        return Err(DbError::Io(format!("no smart playlist {id}")));
    }
    Ok(())
}

pub fn list(conn: &Connection) -> Result<Vec<Playlist>, DbError> {
    let absent = absent_roots(conn)?;
    let mut st = conn.prepare(
        "SELECT i.playlist_id, m.root_id, COUNT(*) FROM playlist_items i
         JOIN media m ON m.id = i.media_id GROUP BY i.playlist_id, m.root_id",
    )?;
    let mut offline: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for row in st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })? {
        let (pid, root, n) = row?;
        if absent.contains(&root) {
            *offline.entry(pid).or_default() += n;
        }
    }
    let mut st = conn.prepare(
        "SELECT p.id, p.name, p.created_at, COALESCE(p.is_smart, 0) AS smart, p.rule_json,
                (SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id = p.id) AS count
         FROM playlists p ORDER BY COALESCE(p.position, 1e18), p.created_at, p.id",
    )?;
    let rows = st.query_map([], |r| {
        let id: i64 = r.get("id")?;
        let smart: i64 = r.get("smart")?;
        let json: Option<String> = r.get("rule_json")?;
        Ok(Playlist {
            id,
            name: r.get("name")?,
            count: r.get("count")?,
            offline: offline.get(&id).copied().unwrap_or(0),
            created_at: r.get("created_at")?,
            smart: smart == 1,
            rule: (smart == 1)
                .then(|| smart::Rule::parse(json.as_deref().unwrap_or("")).ok())
                .flatten(),
        })
    })?;
    let mut lists: Vec<Playlist> = rows.filter_map(|r| r.ok()).collect();

    // A smart list counts what its rule matches now, read once for them all.
    if lists.iter().any(|p| p.smart) {
        let library = candidates(conn)?;
        let now = db::now();
        for p in lists.iter_mut().filter(|p| p.smart) {
            let rows = p
                .rule
                .as_ref()
                .map(|rule| smart::select(rule, &library, now))
                .unwrap_or_default();
            p.count = rows.len() as i64;
            p.offline = rows.iter().filter(|t| absent.contains(&t.root_id)).count() as i64;
        }
    }
    Ok(lists)
}

pub fn create(conn: &Connection, name: &str) -> Result<i64, DbError> {
    let name = if name.is_empty() { "Untitled" } else { name };
    // A new list goes to the bottom of the order a person arranged (D116).
    conn.execute(
        "INSERT INTO playlists (name, profile_id, created_at, position)
         VALUES (?1, 1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM playlists))",
        params![name, db::now()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Give a playlist a new name (D116). A name is trimmed, and an empty one is
/// refused rather than quietly turned into "Untitled": renaming to nothing is
/// far more likely a slip than a wish.
pub fn rename(conn: &Connection, id: i64, name: &str) -> Result<(), DbError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DbError::Io("a playlist needs a name".into()));
    }
    let n = conn.execute(
        "UPDATE playlists SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    if n == 0 {
        return Err(DbError::Io(format!("no playlist {id}")));
    }
    Ok(())
}

/// Delete a playlist, never its tracks (D116). The schema cascades the
/// memberships, sets `jobs.playlist_id` to NULL for anything still queued into
/// it — those downloads still land, just in the library rather than the list —
/// and the order closes over the gap.
pub fn delete(conn: &mut Connection, id: i64) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    let n = tx.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(DbError::Io(format!("no playlist {id}")));
    }
    let order = playlist_order(&tx)?;
    rewrite_order(&tx, &order)?;
    tx.commit()?;
    Ok(())
}

/// Move a playlist to index `to` in the list (D116). An index past the end
/// is the end; a playlist that is not there is nothing to do.
pub fn move_to(conn: &mut Connection, id: i64, to: i64) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    let mut order = playlist_order(&tx)?;
    let Some(idx) = order.iter().position(|p| *p == id) else {
        return Ok(());
    };
    let moved = order.remove(idx);
    let dest = (to.max(0) as usize).min(order.len());
    order.insert(dest, moved);
    rewrite_order(&tx, &order)?;
    tx.commit()?;
    Ok(())
}

/// Every playlist id, in the order the list shows them.
fn playlist_order(conn: &Connection) -> Result<Vec<i64>, DbError> {
    let mut st =
        conn.prepare("SELECT id FROM playlists ORDER BY COALESCE(position, 1e18), created_at, id")?;
    let rows = st.query_map([], |r| r.get::<_, i64>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Positions 0..n in the given order. No UNIQUE constraint on the column, so
/// unlike `rewrite` for a playlist's items this needs no two-phase dance.
fn rewrite_order(conn: &Connection, order: &[i64]) -> Result<(), DbError> {
    for (i, id) in order.iter().enumerate() {
        conn.execute(
            "UPDATE playlists SET position = ?1 WHERE id = ?2",
            params![i as i64, id],
        )?;
    }
    Ok(())
}

pub fn items(conn: &Connection, playlist_id: i64) -> Result<Vec<MediaRow>, DbError> {
    // A smart list's rows are what its rule matches now, positioned in its
    // order, so every reader sees it as it sees any other list (#165).
    if let Some(rule) = rule_of(conn, playlist_id)? {
        return Ok(smart::select(&rule, &candidates(conn)?, db::now()));
    }
    let sql = format!(
        "{MEDIA_SELECT}, i.position, i.rowid AS item FROM playlist_items i
         JOIN media m ON m.id = i.media_id
         JOIN library_roots r ON r.id = m.root_id
         WHERE i.playlist_id = ?1 ORDER BY i.position"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([playlist_id], |r| {
        let pos: i64 = r.get("position")?;
        let mut m = row_to_media(r, Some(pos))?;
        m.item = Some(r.get("item")?);
        Ok(m)
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn add(conn: &Connection, playlist_id: i64, media_id: i64) -> Result<(), DbError> {
    by_hand(conn, playlist_id)?;
    let next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM playlist_items WHERE playlist_id = ?1",
        [playlist_id],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO playlist_items (playlist_id, media_id, position) VALUES (?1, ?2, ?3)",
        params![playlist_id, media_id, next],
    )?;
    Ok(())
}

/// Append a checked selection to a playlist, in the order given, all or
/// nothing (#114): the add counterpart of `library::remove_many` (D84). A
/// track already in the list goes in again, as one **+** puts it in again.
/// Returns how many went in.
pub fn add_many(
    conn: &mut Connection,
    playlist_id: i64,
    media_ids: &[i64],
) -> Result<usize, DbError> {
    let tx = conn.transaction()?;
    let exists: bool = tx
        .query_row(
            "SELECT 1 FROM playlists WHERE id = ?1",
            [playlist_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Err(DbError::Io(format!("no playlist {playlist_id}")));
    }
    for id in media_ids {
        // Checked here, not left to the foreign key: the refusal should say
        // which track, and nothing should go in when one is not there.
        let known: bool = tx
            .query_row("SELECT 1 FROM media WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some();
        if !known {
            return Err(DbError::Io(format!("track {id} is not in the library")));
        }
        add(&tx, playlist_id, *id)?;
    }
    tx.commit()?;
    Ok(media_ids.len())
}

/// Rewrite the whole playlist's positions from an explicit old-order list.
///
/// **This is the two-phase form the schema comment warns about.** The
/// `PRIMARY KEY (playlist_id, position)` invariant is worth keeping — positions
/// genuinely should be unique and dense — but SQLite has no deferred UNIQUE, so
/// the obvious sequence of `UPDATE`s collides mid-transaction against rows it
/// hasn't moved yet.
///
/// Phase 1 parks every affected row at `-1 - position`, which is guaranteed
/// free because real positions are non-negative. Phase 2 writes finals into the
/// now-vacant range. Rows are identified by their *parked position*, not by
/// `media_id`, because the same track may legitimately appear twice.
fn rewrite(tx: &rusqlite::Transaction, playlist_id: i64, order: &[i64]) -> Result<(), DbError> {
    // Phase 1 — vacate.
    tx.execute(
        "UPDATE playlist_items SET position = -1 - position WHERE playlist_id = ?1",
        [playlist_id],
    )?;
    // Phase 2 — refill, densely, from 0.
    for (new_pos, old_pos) in order.iter().enumerate() {
        tx.execute(
            "UPDATE playlist_items SET position = ?3
             WHERE playlist_id = ?1 AND position = ?2",
            params![playlist_id, -1 - old_pos, new_pos as i64],
        )?;
    }
    Ok(())
}

/// Close the gaps left by a route other than `remove_item`: a track taken out of
/// the library takes its memberships with it (the schema cascades), and the
/// positions it held stay empty until this runs (#78).
pub(crate) fn compact(tx: &rusqlite::Transaction, playlist_id: i64) -> Result<(), DbError> {
    let order = positions(tx, playlist_id)?;
    rewrite(tx, playlist_id, &order)
}

fn positions(conn: &Connection, playlist_id: i64) -> Result<Vec<i64>, DbError> {
    let mut st = conn
        .prepare("SELECT position FROM playlist_items WHERE playlist_id = ?1 ORDER BY position")?;
    let rows = st.query_map([playlist_id], |r| r.get::<_, i64>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Take one row out of a hand list (#252, D197): the row `item` of list
/// `playlist_id`, holding track `media_id`, wherever it is now. All three
/// must match, since SQLite may hand a deleted row's id out again: a row
/// already gone is nothing to do, never the row that slid into its place.
/// Returns whether a row went.
pub fn remove_item(
    conn: &mut Connection,
    playlist_id: i64,
    item: i64,
    media_id: i64,
) -> Result<bool, DbError> {
    let tx = conn.transaction()?;
    by_hand(&tx, playlist_id)?;
    let gone = tx.execute(
        "DELETE FROM playlist_items WHERE rowid = ?1 AND playlist_id = ?2 AND media_id = ?3",
        params![item, playlist_id, media_id],
    )?;
    if gone == 0 {
        return Ok(false);
    }
    // Close the gap so positions stay dense.
    compact(&tx, playlist_id)?;
    tx.commit()?;
    Ok(true)
}

/// Move one row of a hand list to `place` (#252, D197), found where it and
/// its neighbours are now, in the transaction that moves it: just before the
/// row it was dropped above, or, where that row has gone or the drop was at
/// the bottom, just after the row above the line. Rows hidden on a drive that
/// is out keep their places (D143). Returns whether anything moved.
pub fn move_item(
    conn: &mut Connection,
    playlist_id: i64,
    item: i64,
    media_id: i64,
    place: Place,
) -> Result<bool, DbError> {
    let tx = conn.transaction()?;
    by_hand(&tx, playlist_id)?;
    // (row, track, position), in order.
    let mut order: Vec<(i64, i64, i64)> = {
        let mut st = tx.prepare(
            "SELECT rowid, media_id, position FROM playlist_items
             WHERE playlist_id = ?1 ORDER BY position",
        )?;
        let rows = st.query_map([playlist_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let Some(from) = order.iter().position(|r| r.0 == item && r.1 == media_id) else {
        return Ok(false);
    };
    let moved = order.remove(from);
    let find = |row: Option<i64>| row.and_then(|row| order.iter().position(|r| r.0 == row));
    let to = match (find(place.before), find(place.after)) {
        (Some(b), _) => b,
        (None, Some(a)) => a + 1,
        (None, None) => return Ok(false),
    };
    if to == from {
        return Ok(false);
    }
    order.insert(to, moved);
    let old: Vec<i64> = order.iter().map(|r| r.2).collect();
    rewrite(&tx, playlist_id, &old)?;
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(conn: &Connection) -> Vec<String> {
        list(conn).unwrap().into_iter().map(|p| p.name).collect()
    }

    #[test]
    fn a_new_playlist_goes_to_the_bottom_and_can_be_moved() {
        let (mut conn, _) = fixture(0);
        let b = create(&conn, "b").unwrap();
        let c = create(&conn, "c").unwrap();
        assert_eq!(names(&conn), ["test", "b", "c"]);
        move_to(&mut conn, c, 0).unwrap();
        assert_eq!(names(&conn), ["c", "test", "b"]);
        // Past the end is the end; a missing id is nothing to do.
        move_to(&mut conn, c, 99).unwrap();
        assert_eq!(names(&conn), ["test", "b", "c"]);
        move_to(&mut conn, 12345, 0).unwrap();
        assert_eq!(names(&conn), ["test", "b", "c"]);
        let _ = b;
    }

    #[test]
    fn a_rename_trims_and_refuses_nothing() {
        let (conn, pid) = fixture(0);
        rename(&conn, pid, "  Storm Prep  ").unwrap();
        assert_eq!(names(&conn), ["Storm Prep"]);
        assert!(
            rename(&conn, pid, "   ").is_err(),
            "an empty name is a slip, not a wish"
        );
        assert_eq!(names(&conn), ["Storm Prep"]);
        assert!(rename(&conn, 9999, "x").is_err());
    }

    /// #165: a smart list is what its rule matches now, counted as such,
    /// current as the library changes, and closed to hand edits in Rust.
    #[test]
    fn a_smart_list_fills_itself_and_refuses_a_hand() {
        let (mut conn, hand) = fixture(0);
        let insert = |conn: &Connection, title: &str, kind: &str| {
            conn.execute(
                "INSERT INTO media (root_id, relpath, kind, title, uploader, added_at)
                 VALUES (1, ?1, ?2, ?1, 'Ian Stocker', ?3)",
                params![title, kind, db::now()],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let beach = insert(&conn, "Beach", "audio");
        insert(&conn, "Storm", "video");
        let rule = smart::Rule::parse(r#"{"v":1,"words":"stocker","kind":"audio","sort":"title"}"#)
            .unwrap();
        let id = create_smart(&conn, "Stocker", &rule).unwrap();

        let titles = |conn: &Connection| -> Vec<String> {
            items(conn, id)
                .unwrap()
                .into_iter()
                .map(|t| t.title)
                .collect()
        };
        assert_eq!(titles(&conn), ["Beach"]);
        let port = insert(&conn, "At Port", "audio");
        assert_eq!(
            titles(&conn),
            ["At Port", "Beach"],
            "a new download appears"
        );
        let lists = list(&conn).unwrap();
        let me = lists.iter().find(|p| p.id == id).unwrap();
        assert!(me.smart);
        assert_eq!(me.rule.as_ref(), Some(&rule));
        assert_eq!(me.count, 2);
        let theirs = lists.iter().find(|p| p.id == hand).unwrap();
        assert!(!theirs.smart && theirs.rule.is_none());

        // Removing a track from the library takes it out, with nothing to cascade.
        crate::library::remove(&mut conn, port).unwrap();
        assert_eq!(titles(&conn), ["Beach"]);

        // No hand edits, and the list is as it was.
        let why = add(&conn, id, beach).unwrap_err().to_string();
        assert!(why.contains("smart playlist"), "{why}");
        assert!(remove_item(&mut conn, id, 1, beach).is_err());
        let place = Place {
            before: None,
            after: Some(2),
        };
        assert!(move_item(&mut conn, id, 1, beach, place).is_err());
        assert_eq!(titles(&conn), ["Beach"]);

        // The rule changes; a hand list has none to change; a bad rule is refused.
        let videos = smart::Rule::parse(r#"{"v":1,"kind":"video"}"#).unwrap();
        set_rule(&conn, id, &videos).unwrap();
        assert_eq!(titles(&conn), ["Storm"]);
        assert!(set_rule(&conn, hand, &videos).is_err());
        let bad = smart::Rule {
            limit: Some(0),
            ..videos
        };
        assert!(set_rule(&conn, id, &bad).is_err());
        // Deleting it deletes no track (D116).
        delete(&mut conn, id).unwrap();
        assert_eq!(list_media(&conn).unwrap().len(), 2);
    }

    /// A list with tracks on a stick that is out says how many of them are
    /// out (D143), and still counts every one: the stick is coming back.
    #[test]
    fn a_list_counts_its_tracks_on_a_drive_that_is_not_plugged_in() {
        let (conn, pid) = fixture(0);
        let here = std::env::temp_dir().to_string_lossy().to_string();
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (2, 'here', ?1),
                    (3, 'stick', 'Q:\\definitely\\not\\plugged\\in')",
            [here],
        )
        .unwrap();
        for (root, rel) in [(2, "a.mp3"), (3, "b.mp3"), (3, "c.mp3")] {
            conn.execute(
                "INSERT INTO media (root_id, relpath, kind, title, added_at)
                 VALUES (?1, ?2, 'audio', ?2, 0)",
                params![root, rel],
            )
            .unwrap();
            add(&conn, pid, conn.last_insert_rowid()).unwrap();
        }
        let other = create(&conn, "nothing out").unwrap();
        let lists = list(&conn).unwrap();
        let find = |id| lists.iter().find(|p| p.id == id).unwrap();
        assert_eq!((find(pid).count, find(pid).offline), (3, 2));
        assert_eq!((find(other).count, find(other).offline), (0, 0));
        let roots: Vec<i64> = items(&conn, pid)
            .unwrap()
            .iter()
            .map(|m| m.root_id)
            .collect();
        assert_eq!(roots, [2, 3, 3]);
    }

    #[test]
    fn deleting_a_playlist_keeps_its_tracks_and_closes_the_order() {
        let (mut conn, pid) = fixture(3);
        let b = create(&conn, "b").unwrap();
        let c = create(&conn, "c").unwrap();
        delete(&mut conn, pid).unwrap();
        assert_eq!(names(&conn), ["b", "c"]);
        // The three tracks it held are still in the library.
        assert_eq!(list_media(&conn).unwrap().len(), 3);
        let pos: Vec<i64> = conn
            .prepare("SELECT position FROM playlists ORDER BY position")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(pos, [0, 1], "dense again after the gap");
        assert!(delete(&mut conn, pid).is_err(), "it is gone");
        let _ = (b, c);
    }

    #[test]
    fn a_queued_download_outlives_the_playlist_it_was_for() {
        let (mut conn, pid) = fixture(0);
        conn.execute(
            "INSERT INTO jobs (url, status, stage, playlist_id, created_at, updated_at)
             VALUES ('https://x', 'queued', 'probe', ?1, 0, 0)",
            [pid],
        )
        .unwrap();
        delete(&mut conn, pid).unwrap();
        let (n, playlist): (i64, Option<i64>) = conn
            .query_row("SELECT COUNT(*), MAX(playlist_id) FROM jobs", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(n, 1, "the download still happens");
        assert_eq!(playlist, None, "it just lands in the library instead");
    }

    /// Build an in-memory library with `n` tracks in one playlist.
    fn fixture(n: i64) -> (Connection, i64) {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        // A root that is nowhere on any machine. `/tmp` was, on Linux, where
        // it is also `temp_dir()`, which a test adds as a second root.
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (1, 'test', '/hp-test-root')",
            [],
        )
        .unwrap();
        let pid = create(&conn, "test").unwrap();
        for i in 0..n {
            conn.execute(
                "INSERT INTO media (root_id, relpath, kind, title, added_at)
                 VALUES (1, ?1, 'audio', ?2, 0)",
                params![format!("{i}.mp3"), format!("track {i}")],
            )
            .unwrap();
            let mid = conn.last_insert_rowid();
            add(&conn, pid, mid).unwrap();
        }
        let _ = &mut conn;
        (conn, pid)
    }

    fn titles(conn: &Connection, pid: i64) -> Vec<String> {
        items(conn, pid)
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect()
    }

    #[test]
    fn add_appends_densely() {
        let (conn, pid) = fixture(3);
        assert_eq!(titles(&conn, pid), ["track 0", "track 1", "track 2"]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2]);
    }

    /// #182: the pipe's search matches as the search box does, leaves out a
    /// track on a drive that is out, caps what it returns and says how many
    /// matched; and a playlist says how much of it could play now.
    #[test]
    fn the_pipe_searches_like_the_search_box_and_skips_a_drive_that_is_out() {
        let (conn, _) = fixture(0);
        let here = std::env::temp_dir().to_string_lossy().to_string();
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (2, 'here', ?1),
                    (3, 'stick', 'Q:\\definitely\\not\\plugged\\in')",
            [here],
        )
        .unwrap();
        let add_track = |root: i64, title: &str, by: &str, at: i64| {
            conn.execute(
                "INSERT INTO media (root_id, relpath, kind, title, uploader, added_at)
                 VALUES (?1, ?2, 'audio', ?2, ?3, ?4)",
                params![root, title, by, at],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let beach = add_track(2, "Beach", "Ian Stocker", 1);
        let port = add_track(2, "At Port", "Ian Stocker", 2);
        add_track(3, "Swamp", "Ian Stocker", 3);
        add_track(2, "Déjà Vu", "Beyoncé", 4);

        let (total, hits) = search(&conn, "STOCKER", 50).unwrap();
        assert_eq!(total, 2, "Swamp is on the stick, which is out");
        let ids: Vec<i64> = hits.iter().map(|t| t.id).collect();
        assert_eq!(ids, [port, beach], "newest first");
        let (total, hits) = search(&conn, "beyonce deja", 50).unwrap();
        assert_eq!((total, hits.len()), (1, 1));
        let (total, hits) = search(&conn, "stocker", 1).unwrap();
        assert_eq!((total, hits.len()), (2, 1), "capped, with the total");

        let list = create(&conn, "Road Tripping").unwrap();
        add(&conn, list, beach).unwrap();
        let swamp: i64 = conn
            .query_row("SELECT id FROM media WHERE title = 'Swamp'", [], |r| {
                r.get(0)
            })
            .unwrap();
        add(&conn, list, swamp).unwrap();
        assert_eq!(
            playable(&conn, list).unwrap(),
            Some(("Road Tripping".to_string(), 1))
        );
        assert_eq!(playable(&conn, 999).unwrap(), None);
    }

    /// #114: a checked selection goes in at the end, in the order given, a
    /// track already there goes in again as one + would put it, and a
    /// selection with a stranger in it puts nothing in at all.
    #[test]
    fn a_selection_is_added_in_order_and_all_or_nothing() {
        let (mut conn, pid) = fixture(3);
        let ids: Vec<i64> = list_media(&conn).unwrap().iter().map(|m| m.id).collect();
        let other = create(&conn, "other").unwrap();
        add(&conn, other, ids[1]).unwrap();

        // Newest first is the library's order; the caller decides the order.
        let (t0, t2) = (
            ids.iter().copied().min().unwrap(),
            ids.iter().copied().max().unwrap(),
        );
        assert_eq!(add_many(&mut conn, other, &[t2, t0]).unwrap(), 2);
        assert_eq!(titles(&conn, other), ["track 1", "track 2", "track 0"]);
        assert_eq!(positions(&conn, other).unwrap(), [0, 1, 2]);

        // Again, with a duplicate: it goes in again, as a single add does.
        add_many(&mut conn, other, &[t0]).unwrap();
        assert_eq!(titles(&conn, other).len(), 4);

        assert!(add_many(&mut conn, other, &[t0, 999]).is_err());
        assert_eq!(
            titles(&conn, other).len(),
            4,
            "nothing went in with the stranger"
        );
        assert!(add_many(&mut conn, 999, &[t0]).is_err());

        // A smart list takes no hand, however many tracks come at once (D144).
        let rule = smart::Rule::parse(r#"{"v":1}"#).unwrap();
        let s = create_smart(&conn, "all", &rule).unwrap();
        assert!(add_many(&mut conn, s, &[t0]).is_err());
        assert_eq!(titles(&conn, pid).len(), 3);
    }

    /// Each row of a hand list as a window holds it: (row, track), in order.
    fn rows(conn: &Connection, pid: i64) -> Vec<(i64, i64)> {
        items(conn, pid)
            .unwrap()
            .into_iter()
            .map(|m| (m.item.unwrap(), m.id))
            .collect()
    }

    fn at(before: Option<(i64, i64)>, after: Option<(i64, i64)>) -> Place {
        Place {
            before: before.map(|r| r.0),
            after: after.map(|r| r.0),
        }
    }

    /// Move row `r` of list `pid` to `place`.
    fn mv(conn: &mut Connection, pid: i64, r: (i64, i64), place: Place) -> bool {
        move_item(conn, pid, r.0, r.1, place).unwrap()
    }

    /// Take row `r` out of list `pid`.
    fn rm(conn: &mut Connection, pid: i64, r: (i64, i64)) -> bool {
        remove_item(conn, pid, r.0, r.1).unwrap()
    }

    fn tracks(conn: &Connection, pid: i64) -> Vec<String> {
        titles(conn, pid)
            .into_iter()
            .map(|t| t.replace("track ", ""))
            .collect()
    }

    /// #252: drag A below C, then Delete before the redraw. The window still
    /// holds A's row, and that is what goes, not B, which now has A's place.
    #[test]
    fn a_delete_straight_after_a_move_takes_the_row_that_moved() {
        let (mut conn, pid) = fixture(5);
        let r = rows(&conn, pid);
        assert!(mv(&mut conn, pid, r[0], at(Some(r[3]), Some(r[2]))));
        assert_eq!(tracks(&conn, pid), ["1", "2", "0", "3", "4"]);
        assert!(rm(&mut conn, pid, r[0]));
        assert_eq!(tracks(&conn, pid), ["1", "2", "3", "4"]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2, 3]);
    }

    /// #252: Delete twice on C. The second names C again, which has gone.
    #[test]
    fn a_second_delete_of_the_same_row_is_nothing() {
        let (mut conn, pid) = fixture(5);
        let r = rows(&conn, pid);
        assert!(rm(&mut conn, pid, r[2]));
        assert!(!rm(&mut conn, pid, r[2]), "not D, which took C's place");
        assert_eq!(tracks(&conn, pid), ["0", "1", "3", "4"]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2, 3]);
    }

    /// #252: remove B, then drag D to the top before the redraw. D moves,
    /// not E, which now has D's old place.
    #[test]
    fn a_move_straight_after_a_removal_moves_the_row_that_was_dragged() {
        let (mut conn, pid) = fixture(5);
        let r = rows(&conn, pid);
        assert!(rm(&mut conn, pid, r[1]));
        assert!(mv(&mut conn, pid, r[3], at(Some(r[0]), None)));
        assert_eq!(tracks(&conn, pid), ["3", "0", "2", "4"]);
    }

    /// A drop made just above a row removed a moment ago lands after the row
    /// above the line; with neither row left, nothing moves.
    #[test]
    fn a_drop_beside_a_row_that_has_gone_lands_beside_the_one_still_there() {
        let (mut conn, pid) = fixture(5);
        let r = rows(&conn, pid);
        assert!(rm(&mut conn, pid, r[1]));
        assert!(mv(&mut conn, pid, r[4], at(Some(r[1]), Some(r[0]))));
        assert_eq!(tracks(&conn, pid), ["0", "4", "2", "3"]);
        assert!(!mv(&mut conn, pid, r[2], at(Some(r[1]), Some((999, 0)))));
        assert_eq!(tracks(&conn, pid), ["0", "4", "2", "3"]);
    }

    /// The same track twice is legal (#114), which is why `rewrite` identifies
    /// rows by parked position and an edit names the row, never the track.
    #[test]
    fn handles_the_same_track_twice() {
        let (mut conn, pid) = fixture(2);
        let first = rows(&conn, pid)[0].1;
        add(&conn, pid, first).unwrap();
        assert_eq!(tracks(&conn, pid), ["0", "1", "0"]);
        let r = rows(&conn, pid);
        assert_ne!(r[0].0, r[2].0, "each copy is its own row");
        // The lower copy to the top: the upper one stays where it was.
        assert!(mv(&mut conn, pid, r[2], at(Some(r[0]), None)));
        assert_eq!(rows(&conn, pid), [r[2], r[0], r[1]]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2]);
        // And that copy goes, not the first of the track's rows.
        assert!(rm(&mut conn, pid, r[2]));
        assert_eq!(rows(&conn, pid), [r[0], r[1]]);
    }

    /// An edit lands only on its own list's row of its own track: a row id
    /// SQLite hands out again, or one from another list, is nothing.
    #[test]
    fn an_edit_touches_only_its_own_lists_row_of_its_own_track() {
        let (mut conn, pid) = fixture(3);
        let other = create(&conn, "other").unwrap();
        let r = rows(&conn, pid);
        add(&conn, other, r[0].1).unwrap();
        add(&conn, other, r[1].1).unwrap();
        let o = rows(&conn, other);
        let (mine, theirs) = (tracks(&conn, pid), tracks(&conn, other));
        assert!(!rm(&mut conn, other, r[0]), "another list's row");
        assert!(!rm(&mut conn, pid, (r[0].0, r[1].1)), "the wrong track");
        assert!(!mv(&mut conn, pid, (r[0].0, r[1].1), at(None, Some(r[2]))));
        assert!(!mv(&mut conn, pid, r[0], at(Some(o[1]), None)));
        assert!(!mv(&mut conn, pid, r[0], at(Some(r[0]), None)), "itself");
        assert_eq!((tracks(&conn, pid), tracks(&conn, other)), (mine, theirs));
    }

    /// D143: rows hidden on a drive that is out are not in the window, so a
    /// drop names the showing rows either side of it, and the hidden ones
    /// keep their places. A B C D with B hidden: the window shows A C D.
    #[test]
    fn a_move_lands_beside_its_neighbours_and_a_hidden_row_keeps_its_place() {
        let cases = [
            (0, Some(3), Some(2), ["1", "2", "0", "3"]),
            (3, Some(0), None, ["3", "0", "1", "2"]),
            (0, None, Some(3), ["1", "2", "3", "0"]),
        ];
        for (moved, before, after, want) in cases {
            let (mut conn, pid) = fixture(4);
            let r = rows(&conn, pid);
            assert!(mv(
                &mut conn,
                pid,
                r[moved],
                at(before.map(|i| r[i]), after.map(|i| r[i]))
            ));
            assert_eq!(tracks(&conn, pid), want);
        }
        // D hidden: A dropped at the bottom of A B C lands after C, D last.
        let (mut conn, pid) = fixture(4);
        let r = rows(&conn, pid);
        assert!(mv(&mut conn, pid, r[0], at(None, Some(r[2]))));
        assert_eq!(tracks(&conn, pid), ["1", "2", "0", "3"]);
    }

    /// The collision case: moving a row makes every row between want a
    /// position another row still occupies. A naive UPDATE sequence hits
    /// the unique constraint here.
    #[test]
    fn a_move_either_way_does_not_collide() {
        let (mut conn, pid) = fixture(5);
        let r = rows(&conn, pid);
        assert!(mv(&mut conn, pid, r[0], at(Some(r[4]), Some(r[3]))));
        assert_eq!(tracks(&conn, pid), ["1", "2", "3", "0", "4"]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2, 3, 4]);
        let (mut conn, pid) = fixture(5);
        let r = rows(&conn, pid);
        assert!(mv(&mut conn, pid, r[4], at(Some(r[1]), Some(r[0]))));
        assert_eq!(tracks(&conn, pid), ["0", "4", "1", "2", "3"]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_removal_closes_the_gap() {
        let (mut conn, pid) = fixture(4);
        let r = rows(&conn, pid);
        assert!(rm(&mut conn, pid, r[1]));
        assert_eq!(tracks(&conn, pid), ["0", "2", "3"]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2]);
    }

    /// A row or a place that is not there is nothing to do: a stale edit
    /// never lands on a neighbour.
    #[test]
    fn a_row_or_place_that_is_gone_is_nothing() {
        let (mut conn, pid) = fixture(3);
        let r = rows(&conn, pid);
        assert!(!mv(&mut conn, pid, (999, r[0].1), at(None, Some(r[2]))));
        assert!(!mv(
            &mut conn,
            pid,
            r[0],
            at(Some((999, 0)), Some((998, 0)))
        ));
        assert!(
            !mv(&mut conn, pid, r[0], at(Some(r[1]), None)),
            "just below itself"
        );
        assert!(!rm(&mut conn, pid, (999, r[0].1)));
        assert_eq!(tracks(&conn, pid), ["0", "1", "2"]);
    }

    /// A row keeps its id through a move and through the compaction a track
    /// leaving the library makes; the library and a smart list have none.
    #[test]
    fn a_row_keeps_its_id_through_moves_and_compaction() {
        let (mut conn, pid) = fixture(4);
        let r = rows(&conn, pid);
        let ids: std::collections::BTreeSet<i64> = r.iter().map(|x| x.0).collect();
        assert_eq!(ids.len(), 4);
        assert!(mv(&mut conn, pid, r[0], at(None, Some(r[3]))));
        crate::library::remove(&mut conn, r[1].1).unwrap();
        assert_eq!(rows(&conn, pid), [r[2], r[3], r[0]]);
        assert!(list_media(&conn).unwrap().iter().all(|m| m.item.is_none()));
        let rule = smart::Rule::parse(r#"{"v":1}"#).unwrap();
        let s = create_smart(&conn, "all", &rule).unwrap();
        let smart_rows = items(&conn, s).unwrap();
        assert!(!smart_rows.is_empty() && smart_rows.iter().all(|m| m.item.is_none()));
    }

    /// A drop at the top sends no `after`, and one at the bottom no `before`.
    #[test]
    fn a_place_reads_as_the_window_sends_it() {
        let p: Place = serde_json::from_str(r#"{"before":7,"after":null}"#).unwrap();
        assert_eq!(
            p,
            Place {
                before: Some(7),
                after: None
            }
        );
        let p: Place = serde_json::from_str(r#"{"after":3}"#).unwrap();
        assert_eq!(
            p,
            Place {
                before: None,
                after: Some(3)
            }
        );
    }
}
