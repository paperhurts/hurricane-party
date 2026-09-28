//! Companion packs a person brought in (#208, D162): where they live, how one
//! comes in, and what they are called. Cap'n Capy ships beside the player and
//! is not one of these; his id is `captain`.
//!
//! A pack is art and a manifest (`purricane.md`, "the line that keeps this
//! safe"), so bringing one in copies exactly three files and nothing else:
//! `companion.json`, the sheet it names, and that sheet's `@2x` twin when
//! there is one (D160). Whether the pack is good is not decided here: the
//! companion's own loader checks it (`hp-companion --check`), and a pack it
//! refuses is removed again.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The id of the companion that ships.
pub const CAPTAIN: &str = "captain";
pub const MANIFEST: &str = "companion.json";
/// No file in a pack is bigger than this.
const MAX_FILE: u64 = 32 * 1024 * 1024;
/// A zip with more entries than this is not a companion.
const MAX_ENTRIES: usize = 256;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
}

/// Every installed pack in `dir`, by name.
pub fn list(dir: &Path) -> Vec<PackInfo> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PackInfo> = entries
        .flatten()
        .filter(|e| e.path().join(MANIFEST).is_file())
        .map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let name = fs::read_to_string(e.path().join(MANIFEST))
                .ok()
                .and_then(|j| manifest_name(&j))
                .unwrap_or_else(|| id.clone());
            PackInfo { id, name }
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// A pack's folder, if `id` is an installed pack's plain folder name.
pub fn folder(dir: &Path, id: &str) -> Option<PathBuf> {
    let plain = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let p = dir.join(id);
    (plain && p.join(MANIFEST).is_file()).then_some(p)
}

fn manifest_name(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let n = v["name"].as_str()?.trim();
    (!n.is_empty()).then(|| n.to_string())
}

/// The files a pack consists of, by name: the manifest, its sheet, and the
/// sheet's `@2x` twin. The sheet's name is checked before it is used for
/// anything: a plain `.png` beside the manifest.
fn wanted(json: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("companion.json: {e}"))?;
    let sprite = v["sprite"]
        .as_str()
        .ok_or("companion.json names no sprite sheet")?;
    let plain = !sprite.is_empty()
        && !sprite.contains(['/', '\\', ':'])
        && sprite != ".."
        && sprite.to_ascii_lowercase().ends_with(".png");
    if !plain {
        return Err(format!(
            "sprite {sprite:?} must be a .png beside companion.json"
        ));
    }
    let stem = &sprite[..sprite.len() - 4];
    Ok(vec![
        MANIFEST.to_string(),
        sprite.to_string(),
        format!("{stem}@2x.png"),
    ])
}

/// Bring a finished pack in, from its `companion.json` (the folder beside
/// it) or from a zip holding one, into a new folder in `dir`. Returns the new
/// folder's id; the caller checks the pack and removes it if it is refused.
pub fn import(src: &Path, dir: &Path) -> Result<String, String> {
    let is = |ext: &str| {
        src.extension()
            .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))
    };
    let named_manifest = src
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(MANIFEST));
    let files = if named_manifest {
        from_folder(src.parent().ok_or("that file has no folder")?)?
    } else if is("zip") {
        from_zip(src)?
    } else {
        return Err("pick a companion's companion.json, or a zip of one".into());
    };
    let json = String::from_utf8(files[0].1.clone()).map_err(|_| "companion.json is not text")?;
    let name = manifest_name(&json).unwrap_or_else(|| "companion".into());
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let id = slug_for(dir, &name);
    let dest = dir.join(&id);
    fs::create_dir(&dest).map_err(|e| e.to_string())?;
    for (file, bytes) in files {
        if let Err(e) = fs::write(dest.join(&file), bytes) {
            let _ = fs::remove_dir_all(&dest);
            return Err(e.to_string());
        }
    }
    Ok(id)
}

/// Take an installed pack out again (a refused one, or one a person removes).
pub fn remove(dir: &Path, id: &str) -> Result<(), String> {
    let p = folder(dir, id).ok_or("no such companion")?;
    fs::remove_dir_all(p).map_err(|e| e.to_string())
}

/// The manifest first, then the sheet, then the twin when it is there.
fn from_folder(folder: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let read = |name: &str| -> Result<Option<Vec<u8>>, String> {
        let p = folder.join(name);
        if !p.is_file() {
            return Ok(None);
        }
        let size = fs::metadata(&p).map_err(|e| e.to_string())?.len();
        if size > MAX_FILE {
            return Err(format!("{name} is too big"));
        }
        fs::read(&p).map(Some).map_err(|e| e.to_string())
    };
    let json = read(MANIFEST)?.ok_or("no companion.json there")?;
    let names = wanted(&String::from_utf8_lossy(&json))?;
    let mut out = vec![(MANIFEST.to_string(), json)];
    for (i, name) in names.iter().enumerate().skip(1) {
        match read(name)? {
            Some(bytes) => out.push((name.clone(), bytes)),
            None if i == 1 => return Err(format!("the sheet {name} is not beside companion.json")),
            None => {}
        }
    }
    Ok(out)
}

/// The same three files from a zip, found beside the one `companion.json`
/// in it, wherever in the zip that is.
fn from_zip(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a zip: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("{} files in the zip", zip.len()));
    }
    let manifests: Vec<PathBuf> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().and_then(|e| e.enclosed_name()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(MANIFEST))
        })
        .collect();
    let [manifest] = manifests.as_slice() else {
        return Err(match manifests.len() {
            0 => "no companion.json in that zip".into(),
            _ => "that zip holds more than one companion; zip one at a time".into(),
        });
    };
    let base = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut read = |name: &str| -> Result<Option<Vec<u8>>, String> {
        // By parts, not as strings: a zip's names use `/`, and a path joined
        // here uses the platform's separator.
        let want = parts(&base.join(name));
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let hit = entry.enclosed_name().is_some_and(|p| parts(&p) == want);
            if !hit || entry.is_dir() {
                continue;
            }
            if entry.size() > MAX_FILE {
                return Err(format!("{name} is too big"));
            }
            let mut bytes = Vec::new();
            entry
                .by_ref()
                .take(MAX_FILE + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            return Ok(Some(bytes));
        }
        Ok(None)
    };
    let json = read(MANIFEST)?.ok_or("no companion.json in that zip")?;
    let names = wanted(&String::from_utf8_lossy(&json))?;
    let mut out = vec![(MANIFEST.to_string(), json)];
    for (i, name) in names.iter().enumerate().skip(1) {
        match read(name)? {
            Some(bytes) => out.push((name.clone(), bytes)),
            None if i == 1 => return Err(format!("the sheet {name} is not in the zip")),
            None => {}
        }
    }
    Ok(out)
}

/// A path as lower-case parts, whichever separator it was written with.
fn parts(p: &Path) -> Vec<String> {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// A folder name for a pack: its name, lower-case, dashes for the rest, and
/// a number when taken. `captain` is always taken.
fn slug_for(dir: &Path, name: &str) -> String {
    let mut base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    base = base.trim_matches('-').to_string();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    if base.is_empty() {
        base = "companion".into();
    }
    base.truncate(48);
    let taken = |c: &str| c == CAPTAIN || dir.join(c).exists();
    if !taken(&base) {
        return base;
    }
    (2..1000)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !taken(c))
        .unwrap_or_else(|| format!("{base}-x"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hp-packs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    const JSON: &str = r#"{"format":"hp-companion/1","name":"Sir Waddles","sprite":"sheet.png"}"#;

    fn pack_folder(root: &Path, with_twin: bool) -> PathBuf {
        let p = root.join("made");
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join(MANIFEST), JSON).unwrap();
        fs::write(p.join("sheet.png"), b"png").unwrap();
        if with_twin {
            fs::write(p.join("sheet@2x.png"), b"png2").unwrap();
        }
        fs::write(p.join("notes.txt"), b"not part of the pack").unwrap();
        p
    }

    #[test]
    fn a_folder_brings_its_three_files_and_nothing_else() {
        let root = tmp("folder");
        let src = pack_folder(&root, true);
        let dir = root.join("companions");
        let id = import(&src.join(MANIFEST), &dir).unwrap();
        assert_eq!(id, "sir-waddles");
        let mut got: Vec<String> = fs::read_dir(dir.join(&id))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        got.sort();
        assert_eq!(got, ["companion.json", "sheet.png", "sheet@2x.png"]);
        assert_eq!(
            list(&dir),
            vec![PackInfo {
                id: "sir-waddles".into(),
                name: "Sir Waddles".into()
            }]
        );
        assert!(folder(&dir, "sir-waddles").is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_twin_is_optional_and_the_sheet_is_not() {
        let root = tmp("sheet");
        let src = pack_folder(&root, false);
        let dir = root.join("companions");
        assert!(import(&src.join(MANIFEST), &dir).is_ok());
        fs::remove_file(src.join("sheet.png")).unwrap();
        let e = import(&src.join(MANIFEST), &dir).unwrap_err();
        assert!(e.contains("sheet.png"), "{e}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_zip_brings_the_same_files_from_wherever_the_manifest_sits() {
        let root = tmp("zip");
        let zpath = root.join("waddles.zip");
        {
            let mut z = zip::ZipWriter::new(fs::File::create(&zpath).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            for (n, b) in [
                ("inner/companion.json", JSON.as_bytes()),
                ("inner/sheet.png", b"png".as_slice()),
                ("inner/readme.txt", b"no".as_slice()),
            ] {
                z.start_file(n, o).unwrap();
                std::io::Write::write_all(&mut z, b).unwrap();
            }
            z.finish().unwrap();
        }
        let dir = root.join("companions");
        let id = import(&zpath, &dir).unwrap();
        assert!(dir.join(&id).join("sheet.png").is_file());
        assert!(!dir.join(&id).join("readme.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_sheet_named_outside_the_pack_is_refused_before_anything_is_read() {
        for bad in ["../x.png", "C:x.png", "sub/x.png", "x.gif"] {
            let json = format!(r#"{{"sprite":{bad:?}}}"#);
            assert!(wanted(&json).is_err(), "{bad}");
        }
    }

    #[test]
    fn names_become_plain_folder_names_and_captain_is_never_taken() {
        let root = tmp("slug");
        assert_eq!(slug_for(&root, "Sir  Waddles!!"), "sir-waddles");
        assert_eq!(slug_for(&root, "Captain"), "captain-2");
        assert_eq!(slug_for(&root, "???"), "companion");
        assert!(folder(&root, "../etc").is_none());
        let _ = fs::remove_dir_all(root);
    }
}
