//! Taking things out of the library (#78).
//!
//! Three verbs, kept apart on purpose:
//!
//!   - **`remove`**: the row goes, the file stays. Its playlist memberships go
//!     with it (the schema cascades) and each playlist closes the gap, so
//!     positions stay dense the way `playlist::remove` keeps them. Adding the
//!     folder or the URL again brings the row back, which is why there is no
//!     undo.
//!   - **`delete_file`**: the file goes. A separate call, made after the row is
//!     gone, from the one place that has already said the path out loud. It is
//!     the one destructive action in the app, so it refuses anything outside a
//!     library root rather than trusting its caller.
//!   - **`prune`**: every row under one root whose file is no longer there. A
//!     rescan counts them; this is the "yes, drop them" that follows.

use crate::db::DbError;
use crate::playlist;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Removed {
    pub id: i64,
    pub title: String,
    /// Where the file still is. The row no longer exists to say so, and the
    /// caller's next question is whether to delete it.
    pub path: String,
}

/// Title and absolute path of one row, rebuilt from `(root, relpath)` the way
/// `playlist::MediaRow` does (D28: the absolute path is never stored).
fn lookup(conn: &Connection, id: i64) -> Result<Option<(String, String)>, DbError> {
    Ok(conn
        .query_row(
            "SELECT m.title, r.path, m.relpath FROM media m
             JOIN library_roots r ON r.id = m.root_id WHERE m.id = ?1",
            [id],
            |r| {
                let title: String = r.get(0)?;
                let root: String = r.get(1)?;
                let rel: String = r.get(2)?;
                Ok((
                    title,
                    Path::new(&root).join(rel).to_string_lossy().to_string(),
                ))
            },
        )
        .optional()?)
}

fn remove_in(tx: &Transaction, id: i64) -> Result<Removed, DbError> {
    let Some((title, path)) = lookup(tx, id)? else {
        return Err(DbError::Io(format!("track {id} is not in the library")));
    };
    // Which lists lose a member, asked before the cascade takes the evidence.
    let lists: Vec<i64> = {
        let mut st =
            tx.prepare("SELECT DISTINCT playlist_id FROM playlist_items WHERE media_id = ?1")?;
        let rows = st.query_map([id], |r| r.get::<_, i64>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    tx.execute("DELETE FROM media WHERE id = ?1", [id])?;
    for pid in lists {
        playlist::compact(tx, pid)?;
    }
    Ok(Removed { id, title, path })
}

/// Remove one row. The file stays.
pub fn remove(conn: &mut Connection, id: i64) -> Result<Removed, DbError> {
    let tx = conn.transaction()?;
    let removed = remove_in(&tx, id)?;
    tx.commit()?;
    Ok(removed)
}

/// Ids of the rows under `root_id` whose files are not on disk.
///
/// A missing *root* is not a missing file (D28: the drive is unplugged, the
/// library is fine), so a root that is not a directory reports nothing rather
/// than everything.
pub fn missing(conn: &Connection, root_id: i64) -> Result<Vec<i64>, DbError> {
    let root: Option<String> = conn
        .query_row(
            "SELECT path FROM library_roots WHERE id = ?1",
            [root_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(root) = root.filter(|p| Path::new(p).is_dir()) else {
        return Ok(Vec::new());
    };
    let mut st = conn.prepare("SELECT id, relpath FROM media WHERE root_id = ?1 ORDER BY id")?;
    let rows = st.query_map([root_id], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(rows
        .filter_map(|r| r.ok())
        .filter(|(_, rel)| !Path::new(&root).join(rel).is_file())
        .map(|(id, _)| id)
        .collect())
}

/// Drop every row under `root_id` whose file is gone. Returns how many.
///
/// Re-checks the disk here rather than trusting a count from an earlier scan:
/// the user may have plugged the drive back in between the offer and the
/// click, and the answer has to be about now.
pub fn prune(conn: &mut Connection, root_id: i64) -> Result<usize, DbError> {
    let ids = missing(conn, root_id)?;
    let tx = conn.transaction()?;
    for id in &ids {
        remove_in(&tx, *id)?;
    }
    tx.commit()?;
    Ok(ids.len())
}

/// Delete a file from disk. Only inside a library root, only a file.
///
/// The path arrives from the webview, so the check is not a formality: a
/// track title is untrusted text, and the webview already gets no shell.
/// Both sides are canonicalized first, because `Path::starts_with` compares
/// components as written and `root\..\elsewhere\x` begins with `root`;
/// canonicalizing resolves the `..` and any junction on the way, so what is
/// compared is where the file really is. A root that is not mounted cannot
/// be canonicalized and simply does not vouch for anything.
pub fn delete_file(conn: &Connection, path: &str) -> Result<(), DbError> {
    let roots: Vec<String> = {
        let mut st = conn.prepare("SELECT path FROM library_roots")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let real = std::fs::canonicalize(path)
        .map_err(|e| DbError::Io(format!("{path} is not there: {e}")))?;
    let inside = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .any(|r| real.starts_with(&r));
    if !inside {
        return Err(DbError::Io(format!(
            "{path} is not inside a library folder, so this app will not delete it"
        )));
    }
    if !real.is_file() {
        return Err(DbError::Io(format!("{path} is not a file")));
    }
    std::fs::remove_file(&real).map_err(|e| DbError::Io(format!("couldn't delete {path}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// A library with one root at `root` and `n` tracks, all in one playlist.
    fn fixture(root: &str, n: i64) -> (Connection, i64) {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (1, 'test', ?1)",
            [root],
        )
        .unwrap();
        let pid = playlist::create(&conn, "test").unwrap();
        for i in 0..n {
            conn.execute(
                "INSERT INTO media (root_id, relpath, kind, title, added_at)
                 VALUES (1, ?1, 'audio', ?2, 0)",
                params![format!("{i}.mp3"), format!("track {i}")],
            )
            .unwrap();
            playlist::add(&conn, pid, conn.last_insert_rowid()).unwrap();
        }
        (conn, pid)
    }

    /// What the library holds, sorted: the browser orders by added_at, and
    /// every fixture row shares one.
    fn titles(conn: &Connection) -> Vec<String> {
        let mut v: Vec<String> = playlist::list_media(conn)
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect();
        v.sort();
        v
    }

    fn list_titles(conn: &Connection, pid: i64) -> Vec<String> {
        playlist::items(conn, pid)
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect()
    }

    fn positions(conn: &Connection, pid: i64) -> Vec<i64> {
        let mut st = conn
            .prepare("SELECT position FROM playlist_items WHERE playlist_id = ?1 ORDER BY position")
            .unwrap();
        st.query_map([pid], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hp-library-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn remove_takes_the_row_and_its_memberships_and_closes_the_gap() {
        let (mut conn, pid) = fixture("/tmp", 3);
        let id: i64 = conn
            .query_row("SELECT id FROM media WHERE title = 'track 1'", [], |r| {
                r.get(0)
            })
            .unwrap();

        let gone = remove(&mut conn, id).unwrap();
        assert_eq!(gone.title, "track 1");
        assert!(gone.path.ends_with("1.mp3"), "{}", gone.path);

        // The other rows are untouched (D80: one row, one identity).
        assert_eq!(titles(&conn), ["track 0", "track 2"]);
        // The playlist lost the member and stayed dense, as playlist::remove
        // would have left it.
        assert_eq!(list_titles(&conn, pid), ["track 0", "track 2"]);
        assert_eq!(positions(&conn, pid), [0, 1]);
    }

    #[test]
    fn remove_of_an_unknown_id_is_an_error_not_a_silent_success() {
        let (mut conn, _) = fixture("/tmp", 1);
        assert!(remove(&mut conn, 99).is_err());
        assert_eq!(titles(&conn).len(), 1);
    }

    #[test]
    fn prune_drops_only_the_rows_whose_files_are_gone() {
        let dir = scratch("prune");
        std::fs::write(dir.join("0.mp3"), b"x").unwrap();
        std::fs::write(dir.join("2.mp3"), b"x").unwrap();
        let (mut conn, pid) = fixture(&dir.to_string_lossy(), 3);

        let ids = missing(&conn, 1).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(prune(&mut conn, 1).unwrap(), 1);
        assert_eq!(titles(&conn), ["track 0", "track 2"]);
        assert_eq!(positions(&conn, pid), [0, 1]);
        // Nothing left to prune; the answer is about now, not the last scan.
        assert_eq!(prune(&mut conn, 1).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unplugged_root_is_not_a_library_full_of_missing_files() {
        // D28: the storm drive comes and goes. Offering to drop every row the
        // moment it is unplugged would be the opposite of what multi-root is
        // for.
        let (conn, _) = fixture("/definitely/not/mounted", 3);
        assert!(missing(&conn, 1).unwrap().is_empty());
    }

    #[test]
    fn delete_file_stays_inside_the_library_roots() {
        let inside = scratch("delete-inside");
        let outside = scratch("delete-outside");
        std::fs::write(inside.join("keep.mp3"), b"x").unwrap();
        std::fs::write(outside.join("mine.mp3"), b"x").unwrap();
        let (conn, _) = fixture(&inside.to_string_lossy(), 0);

        let elsewhere = outside.join("mine.mp3");
        assert!(delete_file(&conn, &elsewhere.to_string_lossy()).is_err());
        assert!(
            elsewhere.is_file(),
            "a path outside every root must survive"
        );

        // Spelled as if inside the root: `<root>\..\<outside>\mine.mp3` begins
        // with the root component-wise, and a check on the string as written
        // would let it through. Where the file really is decides.
        let dressed = inside
            .join("..")
            .join(outside.file_name().unwrap())
            .join("mine.mp3");
        assert!(delete_file(&conn, &dressed.to_string_lossy()).is_err());
        assert!(
            elsewhere.is_file(),
            "a traversal out of the root must survive"
        );

        // A folder is not a file, even inside the root.
        assert!(delete_file(&conn, &inside.to_string_lossy()).is_err());
        assert!(inside.is_dir());

        let ours = inside.join("keep.mp3");
        delete_file(&conn, &ours.to_string_lossy()).unwrap();
        assert!(!ours.exists());
        // Gone is gone: a second delete says so instead of pretending.
        assert!(delete_file(&conn, &ours.to_string_lossy()).is_err());

        let _ = std::fs::remove_dir_all(&inside);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
