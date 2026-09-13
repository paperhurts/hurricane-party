//! EQ presets a person keeps (#145): saved from the EQ window, or read out of
//! Winamp `.eqf` files. The four that ship are the frontend's constant
//! (`src/lib/eq.ts`); this table holds only the person's own, so a preset
//! that ships can never be removed or overwritten from here.

use crate::db::{self, DbError};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

/// The first bytes of every `.eqf` (D31).
const SIGNATURE: &[u8; 31] = b"Winamp EQ library file v1.1\x1A!--";
/// A preset's name buffer, NUL-padded.
const NAME_LEN: usize = 257;
/// Ten band bytes, then the preamp byte (D31: the preamp is last).
const VALUES: usize = 11;
const RECORD: usize = NAME_LEN + VALUES;
/// A library file of every preset Winamp ever shipped is a few KB. Anything
/// near this is not an EQ file, and is not read into memory to find out.
const MAX_FILE: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preset {
    pub id: i64,
    pub name: String,
    pub preamp: f64,
    pub bands: Vec<f64>,
}

/// One preset as a file holds it, before it has an id.
#[derive(Debug, Clone, PartialEq)]
pub struct FilePreset {
    pub name: String,
    pub preamp: f64,
    pub bands: Vec<f64>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EqfError {
    #[error("not a Winamp EQ file: it does not start with the EQ library signature")]
    NotEqf,
    #[error("the EQ file has no presets in it")]
    Empty,
    #[error("that file is {0} KB; an EQ file is a few")]
    TooBig(u64),
    #[error("{0}")]
    Io(String),
}

/// A file byte, `0..=63`, to dB (D31). Inverted: `0x00` is +12 dB, `0x1F`
/// is 0 dB and `0x3F` is −12 dB. Two slopes, not one, because 31 steps lie
/// above 0 dB and 32 below: a single line through both ends puts `0x1F` at
/// +0.19 dB, and a file's flat preset would then not read as flat.
pub fn byte_to_db(b: u8) -> f64 {
    let v = f64::from(b.min(63));
    if v <= 31.0 {
        12.0 * (31.0 - v) / 31.0
    } else {
        -12.0 * (v - 31.0) / 32.0
    }
}

/// dB back to the file's byte, the inverse of `byte_to_db` at the file's
/// resolution. Clamped to the EQ's range first. Only the round-trip test uses
/// it until something writes `.eqf` files; it is the proof the mapping above
/// loses nothing a file can say.
#[cfg(test)]
pub fn db_to_byte(db: f64) -> u8 {
    let d = if db.is_finite() {
        db.clamp(-12.0, 12.0)
    } else {
        0.0
    };
    let v = if d >= 0.0 {
        31.0 - d * 31.0 / 12.0
    } else {
        31.0 - d * 32.0 / 12.0
    };
    v.round().clamp(0.0, 63.0) as u8
}

/// Every preset in an `.eqf`. A library file repeats the name-and-values
/// record after the signature, so a file with one preset and a file with
/// twenty are the same shape; a trailing partial record is left out and
/// counted in the second value.
pub fn parse(bytes: &[u8]) -> Result<(Vec<FilePreset>, usize), EqfError> {
    if bytes.len() < SIGNATURE.len() || &bytes[..SIGNATURE.len()] != SIGNATURE {
        return Err(EqfError::NotEqf);
    }
    let body = &bytes[SIGNATURE.len()..];
    let mut out = Vec::new();
    for rec in body.chunks_exact(RECORD) {
        let raw = &rec[..NAME_LEN];
        let end = raw.iter().position(|&c| c == 0).unwrap_or(NAME_LEN);
        // Winamp wrote the name in the machine's ANSI code page; Latin-1 is
        // the part of that every code page agrees on, and never fails.
        let name: String = raw[..end].iter().map(|&c| char::from(c)).collect();
        let values = &rec[NAME_LEN..];
        out.push(FilePreset {
            name: name.trim().to_string(),
            bands: values[..10].iter().map(|&b| byte_to_db(b)).collect(),
            preamp: byte_to_db(values[10]),
        });
    }
    let partial = body.len() % RECORD;
    if out.is_empty() {
        return Err(EqfError::Empty);
    }
    Ok((out, partial))
}

/// Read and parse one file, capped. The second value is the bytes of a cut
/// short record at the end, 0 for a whole file.
pub fn read(path: &Path) -> Result<(Vec<FilePreset>, usize), EqfError> {
    let size = std::fs::metadata(path)
        .map_err(|e| EqfError::Io(e.to_string()))?
        .len();
    if size > MAX_FILE {
        return Err(EqfError::TooBig(size / 1024));
    }
    let bytes = std::fs::read(path).map_err(|e| EqfError::Io(e.to_string()))?;
    parse(&bytes)
}

/// The person's presets, oldest first, which is the order they were added.
pub fn list(conn: &Connection) -> Result<Vec<Preset>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, preamp_db, bands_db FROM eq_presets
         WHERE COALESCE(is_builtin, 0) = 0 ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([], |r| {
        let bands: String = r.get(3)?;
        Ok(Preset {
            id: r.get(0)?,
            name: r.get(1)?,
            preamp: r.get(2)?,
            bands: serde_json::from_str::<Vec<f64>>(&bands).unwrap_or_default(),
        })
    })?;
    // A row whose bands are not ten numbers is a row nothing can apply.
    Ok(rows
        .filter_map(Result::ok)
        .filter(|p| p.bands.len() == 10)
        .collect())
}

/// Save a preset under a name. A preset of the person's with that name
/// already is replaced in place, keeping its place in the list: saving
/// again, or importing the same file twice, updates rather than piles up.
pub fn save(conn: &Connection, name: &str, preamp: f64, bands: &[f64]) -> Result<Preset, DbError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DbError::Io("a preset needs a name".into()));
    }
    if bands.len() != 10 {
        return Err(DbError::Io(format!(
            "a preset has ten bands, not {}",
            bands.len()
        )));
    }
    let clamp = |v: f64| {
        if v.is_finite() {
            v.clamp(-12.0, 12.0)
        } else {
            0.0
        }
    };
    let bands: Vec<f64> = bands.iter().map(|&v| clamp(v)).collect();
    let preamp = clamp(preamp);
    let json = serde_json::to_string(&bands).map_err(|e| DbError::Io(e.to_string()))?;
    let existing: Option<i64> = conn
        .query_row(
            // Without case: the EQ window draws every name in capitals, so
            // "Mine" and "MINE" are one preset to anyone looking at it.
            "SELECT id FROM eq_presets WHERE COALESCE(is_builtin, 0) = 0 AND name = ?1 COLLATE NOCASE",
            [name],
            |r| r.get(0),
        )
        .ok();
    let id = match existing {
        Some(id) => {
            conn.execute(
                "UPDATE eq_presets SET name = ?2, preamp_db = ?3, bands_db = ?4 WHERE id = ?1",
                params![id, name, preamp, json],
            )?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO eq_presets (name, preamp_db, bands_db, is_builtin, created_at)
                 VALUES (?1, ?2, ?3, 0, ?4)",
                params![name, preamp, json, db::now()],
            )?;
            conn.last_insert_rowid()
        }
    };
    Ok(Preset {
        id,
        name: name.to_string(),
        preamp,
        bands,
    })
}

/// Remove one of the person's presets. A preset that ships is not in this
/// table, and a builtin row, if one ever is, is not removed from here.
pub fn delete(conn: &Connection, id: i64) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM eq_presets WHERE id = ?1 AND COALESCE(is_builtin, 0) = 0",
        [id],
    )?;
    Ok(())
}

/// What an import did, for the EQ window to say.
#[derive(Debug, Default, Serialize)]
pub struct Imported {
    /// Presets read and saved, across every file.
    pub saved: usize,
    /// Files that could not be read, and why, by file name.
    pub refused: Vec<(String, String)>,
    /// Files that ended part-way through a preset: what came before was read.
    pub cut_short: Vec<String>,
}

/// Import every preset in every file. A file that fails is reported and the
/// rest go on; a preset with no name takes the file's.
pub fn import(conn: &Connection, paths: &[String]) -> Imported {
    let mut out = Imported::default();
    for p in paths {
        let path = Path::new(p);
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.clone());
        match read(path) {
            Ok((presets, partial)) => {
                if partial > 0 {
                    out.cut_short.push(file.clone());
                }
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Preset".into());
                for (i, fp) in presets.iter().enumerate() {
                    let name = if !fp.name.is_empty() {
                        fp.name.clone()
                    } else if presets.len() == 1 {
                        stem.clone()
                    } else {
                        format!("{stem} {}", i + 1)
                    };
                    match save(conn, &name, fp.preamp, &fp.bands) {
                        Ok(_) => out.saved += 1,
                        Err(e) => out.refused.push((file.clone(), e.to_string())),
                    }
                }
            }
            Err(e) => out.refused.push((file, e.to_string())),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, values: [u8; 11]) -> Vec<u8> {
        let mut r = vec![0u8; NAME_LEN];
        r[..name.len()].copy_from_slice(name.as_bytes());
        r.extend_from_slice(&values);
        r
    }

    fn file(records: &[Vec<u8>]) -> Vec<u8> {
        let mut f = SIGNATURE.to_vec();
        for r in records {
            f.extend_from_slice(r);
        }
        f
    }

    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        conn
    }

    #[test]
    fn the_bytes_are_inverted_and_zero_db_is_0x1f() {
        assert_eq!(byte_to_db(0x00), 12.0);
        assert_eq!(byte_to_db(0x1F), 0.0);
        assert_eq!(byte_to_db(0x3F), -12.0);
        // Out of range in a damaged file is the floor, not a panic.
        assert_eq!(byte_to_db(200), -12.0);
    }

    #[test]
    fn every_byte_survives_the_trip_to_db_and_back() {
        for b in 0..=63u8 {
            assert_eq!(db_to_byte(byte_to_db(b)), b, "byte {b}");
        }
        assert_eq!(db_to_byte(40.0), 0);
        assert_eq!(db_to_byte(f64::NAN), 31);
    }

    #[test]
    fn the_preamp_is_the_last_byte_not_the_first() {
        // 60 Hz fully up, the preamp fully down.
        let mut v = [0x1F; 11];
        v[0] = 0x00;
        v[10] = 0x3F;
        let (presets, partial) = parse(&file(&[record("Test", v)])).unwrap();
        assert_eq!(partial, 0);
        assert_eq!(presets[0].name, "Test");
        assert_eq!(presets[0].bands[0], 12.0);
        assert_eq!(presets[0].bands[1..], [0.0; 9]);
        assert_eq!(presets[0].preamp, -12.0);
    }

    #[test]
    fn a_library_file_holds_every_preset_in_order() {
        let f = file(&[
            record("Classical", [0x1F; 11]),
            record("Rock", [0x10; 11]),
            record("Techno", [0x28; 11]),
        ]);
        let (presets, _) = parse(&f).unwrap();
        let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Classical", "Rock", "Techno"]);
        // A cut short at the end keeps the whole records and says so.
        let (cut, partial) = parse(&f[..f.len() - 5]).unwrap();
        assert_eq!(cut.len(), 2);
        assert_eq!(partial, RECORD - 5);
    }

    #[test]
    fn a_file_that_is_not_an_eq_library_is_refused() {
        assert_eq!(parse(b"PK\x03\x04 a zip"), Err(EqfError::NotEqf));
        assert_eq!(parse(SIGNATURE), Err(EqfError::Empty));
    }

    #[test]
    fn saving_a_name_again_replaces_it_and_keeps_its_place() {
        let conn = fixture();
        let a = save(&conn, "Mine", 0.0, &[1.0; 10]).unwrap();
        save(&conn, "Other", 0.0, &[0.0; 10]).unwrap();
        let again = save(&conn, " MINE ", 3.0, &[2.0; 10]).unwrap();
        assert_eq!(again.id, a.id);
        let all = list(&conn).unwrap();
        assert_eq!(all.len(), 2);
        // The newest spelling is the one kept.
        assert_eq!(all[0].name, "MINE");
        assert_eq!(all[0].preamp, 3.0);
        assert_eq!(all[0].bands, vec![2.0; 10]);
    }

    #[test]
    fn a_preset_is_clamped_named_and_ten_bands() {
        let conn = fixture();
        let p = save(&conn, "Loud", 40.0, &[-99.0; 10]).unwrap();
        assert_eq!(p.preamp, 12.0);
        assert_eq!(p.bands, vec![-12.0; 10]);
        assert!(save(&conn, "   ", 0.0, &[0.0; 10]).is_err());
        assert!(save(&conn, "Short", 0.0, &[0.0; 9]).is_err());
    }

    #[test]
    fn delete_removes_only_the_persons_own() {
        let conn = fixture();
        let p = save(&conn, "Gone", 0.0, &[0.0; 10]).unwrap();
        conn.execute(
            "INSERT INTO eq_presets (name, preamp_db, bands_db, is_builtin, created_at)
             VALUES ('Shipped', 0, '[0,0,0,0,0,0,0,0,0,0]', 1, 0)",
            [],
        )
        .unwrap();
        delete(&conn, p.id).unwrap();
        let builtin: i64 = conn
            .query_row(
                "SELECT id FROM eq_presets WHERE name = 'Shipped'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        delete(&conn, builtin).unwrap();
        assert!(list(&conn).unwrap().is_empty());
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM eq_presets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 1);
    }

    #[test]
    fn import_reads_every_file_and_reports_the_ones_it_could_not() {
        let conn = fixture();
        let dir = std::env::temp_dir().join(format!("hp-eqf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("pack.eqf");
        std::fs::write(
            &good,
            file(&[record("Club", [0x10; 11]), record("", [0x1F; 11])]),
        )
        .unwrap();
        let bad = dir.join("notes.eqf");
        std::fs::write(&bad, b"just some text").unwrap();
        let paths = [
            good.to_string_lossy().into_owned(),
            bad.to_string_lossy().into_owned(),
        ];
        let got = import(&conn, &paths);
        assert_eq!(got.saved, 2);
        assert_eq!(got.refused.len(), 1);
        assert_eq!(got.refused[0].0, "notes.eqf");
        assert!(got.cut_short.is_empty());
        let names: Vec<String> = list(&conn).unwrap().into_iter().map(|p| p.name).collect();
        // A nameless preset in a file of two takes the file's name and its place.
        assert_eq!(names, ["Club", "pack 2"]);
        // The same file again updates rather than doubles.
        assert_eq!(import(&conn, &paths[..1]).saved, 2);
        assert_eq!(list(&conn).unwrap().len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }
}
