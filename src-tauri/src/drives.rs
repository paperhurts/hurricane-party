//! Roots on a drive that comes and goes (D28, D143).
//!
//! A flash drive is a root like any other while it is plugged in. While it is
//! not, its rows stay (D28), and the library window leaves them out of what
//! it lists and plays. What this module adds is the drive coming back under
//! another letter. Windows gives a stick the letter it had last time when
//! that letter is free and the next one along when it is not, so `E:\hp`
//! comes back as `F:\hp` because a phone was plugged in first. D28 said that
//! must not orphan the library, and until now it did.
//!
//! So a root that is there remembers the drive it is on, by the volume's
//! serial number, and where on the drive it is. A root that is not there is
//! looked for on every drive that is: one drive carrying that serial, with
//! the folder at the same place on it, is where the root has gone, and the
//! root's path is moved there. Its rows are `(root, relpath)` (D28), so they
//! follow with no more than that one write.
//!
//! The asking is split from the writing so the watcher can ask Windows with
//! the database lock let go: `known` reads, the platform answers, `mark` and
//! `follow` write.

use crate::db::{self, DbError};
use crate::platform::Volume;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A root as this module needs it.
#[derive(Debug, Clone)]
pub struct Known {
    pub id: i64,
    pub label: String,
    pub path: String,
    pub volume: Option<String>,
    pub volume_rel: Option<String>,
    pub present: bool,
}

/// Every root, whether it is there, and what it remembers of its drive.
pub fn known(conn: &Connection) -> Result<Vec<Known>, DbError> {
    let mut st =
        conn.prepare("SELECT id, label, path, volume, volume_rel FROM library_roots ORDER BY id")?;
    let rows = st.query_map([], |r| {
        let path: String = r.get(2)?;
        Ok(Known {
            id: r.get(0)?,
            label: r.get(1)?,
            present: Path::new(&path).is_dir(),
            path,
            volume: r.get(3)?,
            volume_rel: r.get(4)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Write down the drive a root that is there is on. Only a change is written,
/// so a root asked about every ten seconds costs a comparison.
///
/// A root whose path is not under the mount Windows gave is left unmarked
/// rather than marked wrongly: it cannot be found again, but it is no worse
/// off than before this existed.
pub fn mark(conn: &Connection, root: &Known, volume: &Volume) -> Result<bool, DbError> {
    let Ok(rel) = Path::new(&root.path).strip_prefix(&volume.mount) else {
        return Ok(false);
    };
    let rel = rel.to_string_lossy().to_string();
    if root.volume.as_deref() == Some(volume.id.as_str())
        && root.volume_rel.as_deref() == Some(rel.as_str())
    {
        return Ok(false);
    }
    conn.execute(
        "UPDATE library_roots SET volume = ?1, volume_rel = ?2 WHERE id = ?3",
        rusqlite::params![volume.id, rel, root.id],
    )?;
    Ok(true)
}

/// The name of the root a track is under, when that root is not there: a
/// track that will not open because its drive is out, which is not a track to
/// offer to remove. `None` for a track whose root is there, or no such track.
pub fn out_for(conn: &Connection, media_id: i64) -> Result<Option<String>, DbError> {
    let root: Option<(String, String)> = conn
        .query_row(
            "SELECT r.label, r.path FROM media m JOIN library_roots r ON r.id = m.root_id
             WHERE m.id = ?1",
            [media_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(root
        .filter(|(_, path)| !Path::new(path).is_dir())
        .map(|(label, _)| label))
}

/// A root that moved to where its drive is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Moved {
    pub id: i64,
    pub label: String,
    pub from: String,
    pub to: String,
}

/// Where a root that is not there has gone, given the mounts its drive is at
/// now (`platform::mounts_of`). `None` when it is nowhere, or when two drives
/// carry its serial with the folder on both, which is a cloned stick and not
/// a guess worth making.
pub fn whereabouts(root: &Known, mounts: &[PathBuf]) -> Option<String> {
    if root.present {
        return None;
    }
    let rel = root.volume_rel.as_deref()?;
    let mut found = mounts
        .iter()
        .map(|m| m.join(rel))
        .filter(|p| p.is_dir())
        .map(|p| db::plain_path(&p.to_string_lossy()));
    let first = found.next()?;
    (found.next().is_none() && first != root.path).then_some(first)
}

/// Move a root to `to`. A root already at `to` is the same folder, added again
/// by hand while this one was lost, so it is folded into this one: the older
/// root keeps its rows, which carry what downloads and checks wrote on them,
/// and gains whatever the newer one found that it did not have.
pub fn follow(conn: &mut Connection, root: &Known, to: &str) -> Result<Moved, DbError> {
    let tx = conn.transaction()?;
    let twin: Option<i64> = tx
        .query_row(
            "SELECT id FROM library_roots WHERE path = ?1 AND id != ?2",
            rusqlite::params![to, root.id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(twin) = twin {
        db::fold_root(&tx, twin, root.id)?;
    }
    tx.execute(
        "UPDATE library_roots SET path = ?1, last_seen_at = ?2 WHERE id = ?3",
        rusqlite::params![to, db::now(), root.id],
    )?;
    tx.commit()?;
    eprintln!("library: {} moved from {} to {to}", root.label, root.path);
    Ok(Moved {
        id: root.id,
        label: root.label.clone(),
        from: root.path.clone(),
        to: to.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(db::schema_for_tests()).unwrap();
        conn
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hp-drives-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn root(conn: &Connection, id: i64, path: &str) {
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (?1, 'hp', ?2)",
            rusqlite::params![id, path],
        )
        .unwrap();
    }

    fn track(conn: &Connection, root_id: i64, rel: &str) -> i64 {
        conn.execute(
            "INSERT INTO media (root_id, relpath, kind, title, added_at)
             VALUES (?1, ?2, 'audio', ?2, 0)",
            rusqlite::params![root_id, rel],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn one(conn: &Connection, id: i64) -> Known {
        known(conn)
            .unwrap()
            .into_iter()
            .find(|k| k.id == id)
            .unwrap()
    }

    /// The whole road: the stick is marked while it is `E:`, unplugged, and
    /// back as `F:`, and its rows and its playlist come with it.
    #[test]
    fn a_drive_back_under_another_letter_takes_its_root_with_it() {
        let base = scratch("letters");
        let (e, f) = (base.join("E"), base.join("F"));
        std::fs::create_dir_all(e.join("hp")).unwrap();
        let mut conn = library();
        let was = e.join("hp").to_string_lossy().to_string();
        root(&conn, 1, &was);
        let song = track(&conn, 1, "At Port.mp3");
        let pid = crate::playlist::create(&conn, "Road Tripping").unwrap();
        crate::playlist::add(&conn, pid, song).unwrap();

        let stick = Volume {
            id: "1A2B3C4D".into(),
            mount: e.clone(),
        };
        assert!(mark(&conn, &one(&conn, 1), &stick).unwrap());
        // Asked again with nothing changed, nothing is written.
        assert!(!mark(&conn, &one(&conn, 1), &stick).unwrap());
        assert_eq!(one(&conn, 1).volume_rel.as_deref(), Some("hp"));

        // Unplugged: E is gone, and the stick is nowhere.
        std::fs::remove_dir_all(&e).unwrap();
        let lost = one(&conn, 1);
        assert!(!lost.present);
        assert_eq!(whereabouts(&lost, &[]), None);

        // Back as F.
        std::fs::create_dir_all(f.join("hp")).unwrap();
        let to = whereabouts(&lost, std::slice::from_ref(&f)).unwrap();
        let moved = follow(&mut conn, &lost, &to).unwrap();
        assert_eq!(moved.from, was);
        assert!(moved.to.ends_with("hp"), "{}", moved.to);

        let now = one(&conn, 1);
        assert!(now.present);
        let rows = crate::playlist::items(&conn, pid).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            PathBuf::from(&rows[0].path),
            f.join("hp").join("At Port.mp3")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_root_that_is_there_is_not_moved_and_a_drive_without_the_folder_is_not_it() {
        let base = scratch("stays");
        std::fs::create_dir_all(base.join("E").join("hp")).unwrap();
        std::fs::create_dir_all(base.join("F")).unwrap();
        let conn = library();
        root(&conn, 1, &base.join("E").join("hp").to_string_lossy());
        conn.execute(
            "UPDATE library_roots SET volume = 'X', volume_rel = 'hp'",
            [],
        )
        .unwrap();
        let here = one(&conn, 1);
        assert_eq!(whereabouts(&here, &[base.join("E")]), None);

        // Lost, and the drive with its serial has no `hp` on it.
        conn.execute(
            "UPDATE library_roots SET path = ?1",
            [base.join("G").join("hp").to_string_lossy()],
        )
        .unwrap();
        assert_eq!(whereabouts(&one(&conn, 1), &[base.join("F")]), None);
        // A root never marked is never looked for.
        conn.execute("UPDATE library_roots SET volume_rel = NULL", [])
            .unwrap();
        assert_eq!(whereabouts(&one(&conn, 1), &[base.join("E")]), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Two drives with one serial are a cloned stick. Picking one would be a
    /// coin toss about which copy the library plays from.
    #[test]
    fn two_drives_that_could_be_it_are_neither() {
        let base = scratch("clones");
        std::fs::create_dir_all(base.join("F").join("hp")).unwrap();
        std::fs::create_dir_all(base.join("G").join("hp")).unwrap();
        let conn = library();
        root(&conn, 1, &base.join("E").join("hp").to_string_lossy());
        conn.execute(
            "UPDATE library_roots SET volume = 'X', volume_rel = 'hp'",
            [],
        )
        .unwrap();
        assert_eq!(
            whereabouts(&one(&conn, 1), &[base.join("F"), base.join("G")]),
            None
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The person plugged the stick in as F and added `F:\hp` by hand before
    /// the watcher looked. The first root keeps its rows and its playlists,
    /// and takes the one track only the second scan found.
    #[test]
    fn a_folder_added_again_by_hand_folds_into_the_root_that_was_lost() {
        let base = scratch("again");
        std::fs::create_dir_all(base.join("F").join("hp")).unwrap();
        let mut conn = library();
        root(&conn, 1, &base.join("E").join("hp").to_string_lossy());
        conn.execute(
            "UPDATE library_roots SET volume = 'X', volume_rel = 'hp'",
            [],
        )
        .unwrap();
        let first = track(&conn, 1, "Beach.mp3");
        let pid = crate::playlist::create(&conn, "p").unwrap();
        crate::playlist::add(&conn, pid, first).unwrap();

        let again = base.join("F").join("hp").to_string_lossy().to_string();
        root(&conn, 2, &again);
        let dup = track(&conn, 2, "Beach.mp3");
        let only_new = track(&conn, 2, "Swamp.mp3");
        crate::playlist::add(&conn, pid, dup).unwrap();

        let lost = one(&conn, 1);
        let to = whereabouts(&lost, &[base.join("F")]).unwrap();
        follow(&mut conn, &lost, &to).unwrap();

        let roots = known(&conn).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, 1);
        assert_eq!(roots[0].path, again);
        let ids: Vec<i64> = crate::playlist::list_media(&conn)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&first) && ids.contains(&only_new));
        // The list's second entry named the duplicate; it names the survivor.
        let members: Vec<i64> = crate::playlist::items(&conn, pid)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(members, [first, first]);
        let _ = std::fs::remove_dir_all(&base);
    }
}
