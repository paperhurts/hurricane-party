//! Watching the library roots (#111, D137).
//!
//! A file that appears in a root while the app runs comes into the library
//! without anyone asking, and a file that leaves is counted and offered for
//! removal the way a click on the root counts it (D95, D83), never dropped.
//!
//! One thread holds every watch and does the looking. A platform watch
//! (`platform::watch_tree`) says only *where* something changed; this decides
//! what that means, once the root has gone quiet.
//!
//! - **Settle.** A copy of a hundred files is a burst of events, and a file
//!   half copied is not a track. A root's changes wait until it has been
//!   quiet for `SETTLE`, and a file another program still has open waits
//!   past that, looked at again each second until it is closed.
//! - **A download is the pipeline's.** yt-dlp writes its fragments, its
//!   source, its merge and the MP3's scratch file into the download folder,
//!   every one named with the video's `[id]`. A file carrying the id of a job
//!   that is not done is left alone: the pipeline records the finished file
//!   itself (`jobs::record_media`) before it marks the job done.
//! - **A row the library already has, at that size, is left as it is.** Only
//!   what is new or has changed size is read and written, through the same
//!   `Found` and `upsert` a scan uses. So a download keeps the title and
//!   artist the pipeline gave it, and a file retagged in place waits for a
//!   click on its root, as it did before.
//! - **Present roots only** (D28). A root that is not there is an unplugged
//!   drive: its watch ends, its rows stay, and it is watched again when it is
//!   back. When Windows asks for the drive, the watch lets go of it, and the
//!   root is not watched again until it has been seen gone, so the watch
//!   never takes hold of a drive halfway through its eject.
//! - **Not at launch.** A root is watched from when the app starts. What
//!   changed while the app was closed is a click on the root away, which is
//!   what D95 chose over rescanning every root at launch.

use crate::db::{Db, DbError};
use crate::localimport::{self, Found};
use crate::platform::{self, TreeEvent, TreeWatch};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// How long a root has to be quiet before its changes are looked at.
const SETTLE: Duration = Duration::from_secs(2);
/// How often the thread wakes with nothing to hear: a quiet root settles, and
/// a file that was still open is tried again.
const TICK: Duration = Duration::from_secs(1);
/// How often the roots are listed again, to watch one that was added or came
/// back and to let go of one that went away. The same ten seconds a download
/// waits for its folder (D136).
const RECONCILE: Duration = Duration::from_secs(10);
/// A root whose drive Windows asked for is watched again once it has gone
/// and come back, or after this long if it never went: something else
/// refused the eject.
const RELEASE_HOLD: Duration = Duration::from_secs(60);

enum Msg {
    Event {
        root: i64,
        generation: u64,
        event: TreeEvent,
    },
    Reconcile,
}

/// The way in, for whatever has just made a root.
pub struct Watcher(Sender<Msg>);

/// Look at the roots again now, rather than within ten seconds: a folder was
/// added, or a download landed in a folder that may be new.
pub fn nudge(app: &AppHandle) {
    if let Some(w) = app.try_state::<Watcher>() {
        let _ = w.0.send(Msg::Reconcile);
    }
}

/// Start watching. After the database is managed.
pub fn spawn(app: AppHandle) {
    let (tx, rx) = channel();
    app.manage(Watcher(tx.clone()));
    let spawned = std::thread::Builder::new()
        .name("hp-watch".into())
        .spawn(move || run(app, tx, rx));
    if let Err(e) = spawned {
        eprintln!("the library roots are not watched: {e}");
    }
}

/// The rows under a root whose files have gone, for the library window.
#[derive(Clone, Serialize)]
struct Gone {
    root_id: i64,
    root: String,
    missing: usize,
}

/// What a root has seen since it was last looked at.
struct Pending {
    /// Relative to the root. The empty path stands for the whole root.
    paths: BTreeSet<PathBuf>,
    /// The last change heard.
    since: Instant,
}

fn note(pending: &mut HashMap<i64, Pending>, root: i64, rel: PathBuf, now: Instant) {
    let p = pending.entry(root).or_insert_with(|| Pending {
        paths: BTreeSet::new(),
        since: now,
    });
    p.paths.insert(rel);
    p.since = now;
}

/// The roots that have been quiet long enough to look at.
fn due(pending: &HashMap<i64, Pending>, now: Instant) -> Vec<i64> {
    pending
        .iter()
        .filter(|(_, p)| now.saturating_duration_since(p.since) >= SETTLE)
        .map(|(id, _)| *id)
        .collect()
}

struct Active {
    generation: u64,
    path: PathBuf,
    label: String,
    _watch: TreeWatch,
}

/// Every watch, and what the thread remembers about the roots between looks.
#[derive(Default)]
struct Roots {
    active: HashMap<i64, Active>,
    /// Roots whose drive Windows asked for, and when.
    released: HashMap<i64, Instant>,
    /// Roots whose watch would not start, so the reason is printed once.
    refused: BTreeSet<i64>,
    /// Which watch an event came from, so one already let go of is ignored.
    generation: u64,
}

impl Roots {
    fn reconcile(&mut self, app: &AppHandle, tx: &Sender<Msg>) {
        let roots = {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            localimport::list_roots(&conn)
        };
        let Ok(roots) = roots else { return };
        let listed: BTreeSet<i64> = roots.iter().map(|r| r.id).collect();
        self.active.retain(|id, _| listed.contains(id));

        for r in roots {
            if !r.present {
                // Unplugged, or an eject that finished. The rows stay (D28).
                self.active.remove(&r.id);
                self.released.remove(&r.id);
                continue;
            }
            if self.active.contains_key(&r.id) {
                continue;
            }
            if self
                .released
                .get(&r.id)
                .is_some_and(|t| t.elapsed() < RELEASE_HOLD)
            {
                continue;
            }
            self.released.remove(&r.id);

            self.generation += 1;
            let (id, generation, tx) = (r.id, self.generation, tx.clone());
            let path = PathBuf::from(&r.path);
            let on_event = Box::new(move |event| {
                let _ = tx.send(Msg::Event {
                    root: id,
                    generation,
                    event,
                });
            });
            match platform::platform().watch_tree(&path, on_event) {
                Ok(watch) => {
                    self.refused.remove(&id);
                    self.active.insert(
                        id,
                        Active {
                            generation,
                            path,
                            label: r.label,
                            _watch: watch,
                        },
                    );
                }
                Err(e) => {
                    if self.refused.insert(id) {
                        eprintln!("not watching {}: {e}", r.path);
                    }
                }
            }
        }
    }
}

fn run(app: AppHandle, tx: Sender<Msg>, rx: Receiver<Msg>) {
    let mut roots = Roots::default();
    let mut pending: HashMap<i64, Pending> = HashMap::new();
    // The missing count last told for each root, so a notice comes when more
    // have gone, not every time the root is looked at.
    let mut told: HashMap<i64, usize> = HashMap::new();
    let mut reconciled: Option<Instant> = None;

    loop {
        match rx.recv_timeout(TICK) {
            Ok(Msg::Event {
                root,
                generation,
                event,
            }) => {
                let current = roots
                    .active
                    .get(&root)
                    .is_some_and(|a| a.generation == generation);
                if current {
                    match event {
                        TreeEvent::Changed(rel) => note(&mut pending, root, rel, Instant::now()),
                        TreeEvent::Overflow => {
                            note(&mut pending, root, PathBuf::new(), Instant::now())
                        }
                        TreeEvent::Released => {
                            roots.active.remove(&root);
                            roots.released.insert(root, Instant::now());
                        }
                        TreeEvent::Ended => {
                            roots.active.remove(&root);
                        }
                    }
                }
            }
            Ok(Msg::Reconcile) => reconciled = None,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        if reconciled.is_none_or(|t| t.elapsed() >= RECONCILE) {
            reconciled = Some(Instant::now());
            roots.reconcile(&app, &tx);
        }

        for id in due(&pending, Instant::now()) {
            let Some(p) = pending.remove(&id) else {
                continue;
            };
            // A root let go of since: what it saw goes with it, and a click
            // on the root is there when it is back.
            let Some(a) = roots.active.get(&id) else {
                continue;
            };
            let (path, label) = (a.path.clone(), a.label.clone());
            let seen = look(&app.state::<Db>(), id, &path, &p.paths, &|f| {
                platform::platform().in_use(f)
            });
            match seen {
                Ok(seen) => {
                    if !seen.held.is_empty() {
                        // Kept at their old quiet time, so they are tried
                        // again on the next tick rather than after another
                        // settle.
                        pending
                            .entry(id)
                            .or_insert(Pending {
                                paths: BTreeSet::new(),
                                since: p.since,
                            })
                            .paths
                            .extend(seen.held);
                    }
                    if seen.added + seen.updated > 0 {
                        let _ = app.emit("library-changed", ());
                    }
                    if let Some(missing) = seen.missing {
                        let before = told.insert(id, missing).unwrap_or(0);
                        if missing > before {
                            let _ = app.emit(
                                "library-watched",
                                Gone {
                                    root_id: id,
                                    root: label,
                                    missing,
                                },
                            );
                        }
                    }
                }
                Err(e) => eprintln!("couldn't look at {}: {e}", path.display()),
            }
        }
    }
}

/// What looking at a root's changes did.
#[derive(Debug, Default, PartialEq)]
struct Seen {
    added: usize,
    updated: usize,
    /// The rows under the root whose files are gone, counted when something
    /// may have left. `None` when every changed path is still there, so a
    /// long copy does not check every row in the library once a second.
    missing: Option<usize>,
    /// Files another program still has open, relative to the root.
    held: Vec<PathBuf>,
}

/// Look at what changed under a root, and bring the library up to date with
/// it.
fn look(
    db: &Db,
    root_id: i64,
    root: &Path,
    paths: &BTreeSet<PathBuf>,
    in_use: &dyn Fn(&Path) -> bool,
) -> Result<Seen, DbError> {
    let mut seen = Seen::default();
    let whole = paths.iter().any(|p| p.as_os_str().is_empty());
    // Too much changed to list, so anything may have left.
    let mut gone = whole;
    let starts: Vec<PathBuf> = if whole {
        vec![root.to_path_buf()]
    } else {
        paths.iter().map(|p| root.join(p)).collect()
    };
    let mut files = BTreeSet::new();
    for start in starts {
        if start.is_dir() {
            files.extend(localimport::media_files(&start).map(|e| e.into_path()));
        } else if localimport::claims(&start) {
            files.insert(start);
        } else if !start.exists() {
            gone = true;
        }
    }

    // What the library has already, and which downloads are under way, in
    // one hold of the lock: `record_media` runs before a job is marked done,
    // so a download's file is either still its job's or already a row.
    let (known, unfinished) = {
        let conn = db.0.lock().unwrap();
        let mut st =
            conn.prepare("SELECT filesize FROM media WHERE root_id = ?1 AND relpath = ?2")?;
        let mut known: HashMap<PathBuf, Option<i64>> = HashMap::new();
        for f in &files {
            let Ok(rel) = f.strip_prefix(root) else {
                continue;
            };
            let size = st
                .query_row(rusqlite::params![root_id, rel.to_string_lossy()], |r| {
                    r.get::<_, Option<i64>>(0)
                })
                .optional()?;
            if let Some(size) = size {
                known.insert(rel.to_path_buf(), size);
            }
        }
        (known, unfinished_ids(&conn)?)
    };

    // Tags are read with the lock let go: a folder of new files is a read of
    // every one, and playback must not wait on it.
    let mut read = Vec::new();
    for f in &files {
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        if carries_id(&name, &unfinished) {
            continue;
        }
        let Ok(rel) = f.strip_prefix(root) else {
            continue;
        };
        let size = std::fs::metadata(f).ok().map(|m| m.len() as i64);
        if size.is_some() && known.get(rel) == Some(&size) {
            continue;
        }
        if in_use(f) {
            seen.held.push(rel.to_path_buf());
            continue;
        }
        if let Some(found) = Found::read(root, f, size) {
            read.push(found);
        }
    }

    let mut conn = db.0.lock().unwrap();
    let tx = conn.transaction()?;
    for found in &read {
        if localimport::upsert(&tx, root_id, found)?.1 {
            seen.added += 1;
        } else {
            seen.updated += 1;
        }
    }
    if gone {
        seen.missing = Some(crate::library::missing(&tx, root_id)?.len());
    }
    tx.commit()?;
    Ok(seen)
}

/// The video ids of every download that is not done.
fn unfinished_ids(conn: &Connection) -> Result<Vec<String>, DbError> {
    let mut st =
        conn.prepare("SELECT video_id FROM jobs WHERE status != 'done' AND video_id IS NOT NULL")?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Whether a file name carries the `[id]` yt-dlp puts in everything it writes
/// for one of these downloads (the output template in `pipeline.rs`).
fn carries_id(name: &str, ids: &[String]) -> bool {
    ids.iter().any(|id| name.contains(&format!("[{id}]")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hp-watch-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A library with one root, at `dir`.
    fn library(dir: &Path) -> Db {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (1, 'Music', ?1)",
            [dir.to_string_lossy()],
        )
        .unwrap();
        Db(Mutex::new(conn))
    }

    fn paths(ps: &[&str]) -> BTreeSet<PathBuf> {
        ps.iter().map(PathBuf::from).collect()
    }

    fn titles(db: &Db) -> Vec<String> {
        let conn = db.0.lock().unwrap();
        let mut st = conn
            .prepare("SELECT title FROM media ORDER BY title")
            .unwrap();
        let rows = st.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    fn free(_: &Path) -> bool {
        false
    }

    #[test]
    fn a_root_is_looked_at_once_it_has_been_quiet() {
        let t0 = Instant::now();
        let mut pending = HashMap::new();
        note(&mut pending, 1, PathBuf::from("a.mp3"), t0);
        assert!(due(&pending, t0 + Duration::from_millis(1500)).is_empty());
        // Another file in the same copy puts it off again.
        note(
            &mut pending,
            1,
            PathBuf::from("b.mp3"),
            t0 + Duration::from_millis(1500),
        );
        assert!(due(&pending, t0 + Duration::from_millis(3000)).is_empty());
        assert_eq!(due(&pending, t0 + Duration::from_millis(3600)), [1]);
        assert_eq!(pending[&1].paths.len(), 2);
    }

    /// The Outcome of #111: drop files in, and they are in the library. A
    /// folder dropped in whole is walked.
    #[test]
    fn files_dropped_into_a_root_come_in() {
        let dir = scratch("drop");
        let db = library(&dir);
        std::fs::write(dir.join("Loose Track.mp3"), b"x").unwrap();
        std::fs::create_dir(dir.join("Album")).unwrap();
        std::fs::write(dir.join("Album").join("One.mp3"), b"x").unwrap();
        std::fs::write(dir.join("Album").join("cover.jpg"), b"x").unwrap();

        let seen = look(&db, 1, &dir, &paths(&["Loose Track.mp3", "Album"]), &free).unwrap();
        assert_eq!((seen.added, seen.updated), (2, 0));
        assert_eq!(seen.missing, None);
        assert_eq!(titles(&db), ["Loose Track", "One"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A row the library already has at that size is not read again: that is
    /// what keeps a download's title, and what keeps a long copy cheap.
    #[test]
    fn a_file_the_library_has_at_that_size_is_left_alone() {
        let dir = scratch("known");
        let db = library(&dir);
        std::fs::write(dir.join("song.mp3"), b"x").unwrap();
        {
            let conn = db.0.lock().unwrap();
            conn.execute(
                "INSERT INTO media (root_id, relpath, kind, title, filesize, added_at)
                 VALUES (1, 'song.mp3', 'audio', 'Named by the pipeline', 1, 0)",
                [],
            )
            .unwrap();
        }
        let seen = look(&db, 1, &dir, &paths(&["song.mp3"]), &free).unwrap();
        assert_eq!((seen.added, seen.updated), (0, 0));
        assert_eq!(titles(&db), ["Named by the pipeline"]);

        // Written again at another size, it is read again.
        std::fs::write(dir.join("song.mp3"), b"xx").unwrap();
        let seen = look(&db, 1, &dir, &paths(&["song.mp3"]), &free).unwrap();
        assert_eq!((seen.added, seen.updated), (0, 1));
        assert_eq!(titles(&db), ["song"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_still_being_copied_waits() {
        let dir = scratch("copying");
        let db = library(&dir);
        std::fs::write(dir.join("big.mp4"), b"x").unwrap();
        let seen = look(&db, 1, &dir, &paths(&["big.mp4"]), &|_| true).unwrap();
        assert_eq!(seen.added, 0);
        assert_eq!(seen.held, [PathBuf::from("big.mp4")]);
        assert!(titles(&db).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// yt-dlp's fragments and the MP3's scratch file carry the job's id and
    /// are left to the pipeline. The same id, done, is an ordinary file.
    #[test]
    fn a_download_under_way_is_left_to_the_pipeline() {
        let dir = scratch("download");
        let db = library(&dir);
        std::fs::create_dir(dir.join("youtube")).unwrap();
        for f in [
            "Song [abc123].f137.mp4",
            "Song [abc123].part.mp3",
            "Other [zzz].mp3",
        ] {
            std::fs::write(dir.join("youtube").join(f), b"x").unwrap();
        }
        {
            let conn = db.0.lock().unwrap();
            conn.execute(
                "INSERT INTO jobs (url, status, video_id, created_at, updated_at)
                 VALUES ('https://youtu.be/abc123', 'running', 'abc123', 0, 0)",
                [],
            )
            .unwrap();
        }
        let seen = look(&db, 1, &dir, &paths(&["youtube"]), &free).unwrap();
        assert_eq!(seen.added, 1);
        assert_eq!(titles(&db), ["Other [zzz]"]);

        assert!(carries_id("Song [abc123].webm", &["abc123".into()]));
        assert!(!carries_id("Song abc123.webm", &["abc123".into()]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #78, D83: a file that leaves is counted, never dropped.
    #[test]
    fn a_file_that_leaves_is_counted_and_its_row_stays() {
        let dir = scratch("leaves");
        let db = library(&dir);
        std::fs::write(dir.join("gone.mp3"), b"x").unwrap();
        look(&db, 1, &dir, &paths(&["gone.mp3"]), &free).unwrap();
        std::fs::remove_file(dir.join("gone.mp3")).unwrap();

        let seen = look(&db, 1, &dir, &paths(&["gone.mp3"]), &free).unwrap();
        assert_eq!(seen.missing, Some(1));
        assert_eq!(titles(&db), ["gone"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Too many changes to list: the whole root is looked at, and anything
    /// may have left.
    #[test]
    fn an_overflow_looks_at_the_whole_root() {
        let dir = scratch("overflow");
        let db = library(&dir);
        std::fs::create_dir_all(dir.join("a").join("b")).unwrap();
        std::fs::write(dir.join("a").join("b").join("deep.flac"), b"x").unwrap();
        let seen = look(&db, 1, &dir, &paths(&[""]), &free).unwrap();
        assert_eq!(seen.added, 1);
        assert_eq!(seen.missing, Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
