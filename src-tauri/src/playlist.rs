//! Playlists and library queries.

use crate::db::{self, DbError};
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub created_at: i64,
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
    /// Absolute path, rebuilt from (root_id, relpath) at read time. The DB never
    /// stores it (D28) — this is derived for the player, not persisted.
    pub path: String,
    /// Present only when the row came from a playlist query.
    pub position: Option<i64>,
}

const MEDIA_SELECT: &str = "
    SELECT m.id, m.title, m.uploader, m.duration_s, m.filesize, m.kind,
           r.path AS root_path, m.relpath";

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
        path: std::path::Path::new(&root)
            .join(&rel)
            .to_string_lossy()
            .to_string(),
        position,
    })
}

/// The library. Flat and sortable (O6) — not a tree.
pub fn list_media(conn: &Connection) -> Result<Vec<MediaRow>, DbError> {
    let sql = format!(
        "{MEDIA_SELECT} FROM media m
         JOIN library_roots r ON r.id = m.root_id
         ORDER BY m.added_at DESC"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([], |r| row_to_media(r, None))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn list(conn: &Connection) -> Result<Vec<Playlist>, DbError> {
    let mut st = conn.prepare(
        "SELECT p.id, p.name, p.created_at,
                (SELECT COUNT(*) FROM playlist_items i WHERE i.playlist_id = p.id) AS count
         FROM playlists p ORDER BY COALESCE(p.position, 1e18), p.created_at, p.id",
    )?;
    let rows = st.query_map([], |r| {
        Ok(Playlist {
            id: r.get("id")?,
            name: r.get("name")?,
            count: r.get("count")?,
            created_at: r.get("created_at")?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
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
    let sql = format!(
        "{MEDIA_SELECT}, i.position FROM playlist_items i
         JOIN media m ON m.id = i.media_id
         JOIN library_roots r ON r.id = m.root_id
         WHERE i.playlist_id = ?1 ORDER BY i.position"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map([playlist_id], |r| {
        let pos: i64 = r.get("position")?;
        row_to_media(r, Some(pos))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn add(conn: &Connection, playlist_id: i64, media_id: i64) -> Result<(), DbError> {
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

/// Close the gaps left by a route other than `remove`: a track taken out of
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

pub fn remove(conn: &mut Connection, playlist_id: i64, position: i64) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM playlist_items WHERE playlist_id = ?1 AND position = ?2",
        params![playlist_id, position],
    )?;
    // Close the gap so positions stay dense.
    let remaining: Vec<i64> = {
        let mut st = tx.prepare(
            "SELECT position FROM playlist_items WHERE playlist_id = ?1 ORDER BY position",
        )?;
        let rows = st.query_map([playlist_id], |r| r.get::<_, i64>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    rewrite(&tx, playlist_id, &remaining)?;
    tx.commit()?;
    Ok(())
}

pub fn reorder(conn: &mut Connection, playlist_id: i64, from: i64, to: i64) -> Result<(), DbError> {
    let mut order = positions(conn, playlist_id)?;
    let Some(idx) = order.iter().position(|p| *p == from) else {
        return Ok(()); // nothing at that position; nothing to do
    };
    let moved = order.remove(idx);
    let dest = (to.max(0) as usize).min(order.len());
    order.insert(dest, moved);

    let tx = conn.transaction()?;
    rewrite(&tx, playlist_id, &order)?;
    tx.commit()?;
    Ok(())
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
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (1, 'test', '/tmp')",
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

    /// The collision case: moving an item downward makes every intervening row
    /// want a position another row still occupies. A naive UPDATE sequence hits
    /// the unique constraint here.
    #[test]
    fn reorder_downward_does_not_collide() {
        let (mut conn, pid) = fixture(5);
        reorder(&mut conn, pid, 0, 3).unwrap();
        assert_eq!(
            titles(&conn, pid),
            ["track 1", "track 2", "track 3", "track 0", "track 4"]
        );
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn reorder_upward_does_not_collide() {
        let (mut conn, pid) = fixture(5);
        reorder(&mut conn, pid, 4, 1).unwrap();
        assert_eq!(
            titles(&conn, pid),
            ["track 0", "track 4", "track 1", "track 2", "track 3"]
        );
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn remove_closes_the_gap() {
        let (mut conn, pid) = fixture(4);
        remove(&mut conn, pid, 1).unwrap();
        assert_eq!(titles(&conn, pid), ["track 0", "track 2", "track 3"]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2]);
    }

    /// The same track twice is legal, which is why `rewrite` identifies rows by
    /// parked position rather than by media_id.
    #[test]
    fn handles_the_same_track_twice() {
        let (mut conn, pid) = fixture(2);
        let first: i64 = conn
            .query_row(
                "SELECT media_id FROM playlist_items WHERE position = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        add(&conn, pid, first).unwrap();
        assert_eq!(titles(&conn, pid), ["track 0", "track 1", "track 0"]);
        reorder(&mut conn, pid, 2, 0).unwrap();
        assert_eq!(titles(&conn, pid), ["track 0", "track 0", "track 1"]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2]);
    }

    #[test]
    fn reorder_of_missing_position_is_a_noop() {
        let (mut conn, pid) = fixture(3);
        reorder(&mut conn, pid, 99, 0).unwrap();
        assert_eq!(titles(&conn, pid), ["track 0", "track 1", "track 2"]);
    }

    #[test]
    fn reorder_clamps_out_of_range_destination() {
        let (mut conn, pid) = fixture(3);
        reorder(&mut conn, pid, 0, 99).unwrap();
        assert_eq!(titles(&conn, pid), ["track 1", "track 2", "track 0"]);
        assert_eq!(positions(&conn, pid).unwrap(), [0, 1, 2]);
    }
}
