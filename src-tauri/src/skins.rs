//! Importing a skin (#107, D6, D17): the disk half. A `.wsz` is a zip of BMPs
//! and two text files, and this unpacks one into the app's own skins folder
//! so the renderer can load it like any other.
//!
//! The mapping — which sprite is where, what the two text files mean — is the
//! frontend's (`src/lib/wsz.ts`), because that is where the manifest is
//! validated and where a refusal has to reach the person who chose the file.
//! This module knows only about bytes: unpack, cap, hand back the names.
//!
//! **A pack is untrusted input from the internet** (skin-manifest.md), so:
//! only the basename of each entry is ever used, which is what closes zip
//! slip; only the extensions a skin is made of are written; and the entry
//! count, each file and the total are capped, because a 16k x 16k PNG is a
//! denial of service dressed as a skin.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// At most this many files out of one zip.
const MAX_ENTRIES: usize = 200;
/// At most this much from any one of them, decompressed.
const MAX_FILE: u64 = 8 * 1024 * 1024;
/// And at most this much in total.
const MAX_TOTAL: u64 = 32 * 1024 * 1024;

/// What a skin is made of. Anything else in the zip (cursors, a readme, the
/// author's own screenshots) is left where it was.
const KEEP: [&str; 2] = ["bmp", "txt"];

#[derive(Debug, thiserror::Error)]
pub enum SkinError {
    #[error("{0}")]
    Io(String),
    #[error("that file is not a zip: {0}")]
    NotAZip(String),
    #[error("this skin is too big: {0}")]
    TooBig(String),
    #[error("there is nothing a skin is made of in that zip")]
    Empty,
}

impl serde::Serialize for SkinError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl From<std::io::Error> for SkinError {
    fn from(e: std::io::Error) -> Self {
        SkinError::Io(e.to_string())
    }
}

/// What the frontend needs to build a manifest: where the files went, what
/// they are called, and the two text files' contents.
#[derive(serde::Serialize)]
pub struct Unpacked {
    /// The folder's name inside the skins directory, which is also the id.
    pub id: String,
    /// That folder, in full, so the webview can ask for its sheets over the
    /// asset protocol and measure them before mapping (D106).
    pub dir: String,
    /// The name to show, from the zip's own file name.
    pub name: String,
    /// Every file written, lower case, without a path.
    pub files: Vec<String>,
    pub pledit: Option<String>,
    pub viscolor: Option<String>,
}

/// A folder name from a skin's file name: lower case, and nothing that could
/// mean something to a path. A collision takes a number, so importing two
/// skins called `base` keeps both.
fn slug_for(root: &Path, stem: &str) -> String {
    let mut base: String = stem
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
    if base.is_empty() {
        base = "skin".into();
    }
    base.truncate(48);
    if !root.join(&base).exists() {
        return base;
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !root.join(&candidate).exists() {
            return candidate;
        }
    }
    base
}

/// Classic skins were written on Windows in the nineties, so their text files
/// are whatever the author's code page was. UTF-8 when it parses, otherwise
/// each byte as its own character, which is right for the ASCII the colour
/// lines are made of and harmless for the comments around them.
fn text_of(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    }
}

/// Unpack one `.wsz` into `skins_dir`, and say what came out.
pub fn unpack(zip_path: &Path, skins_dir: &Path) -> Result<Unpacked, SkinError> {
    let file = fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| SkinError::NotAZip(e.to_string()))?;
    if zip.len() > MAX_ENTRIES {
        return Err(SkinError::TooBig(format!("{} files in the zip", zip.len())));
    }

    let stem = zip_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    fs::create_dir_all(skins_dir)?;
    let id = slug_for(skins_dir, &stem);
    let dir = skins_dir.join(&id);
    fs::create_dir_all(&dir)?;

    let mut out = Unpacked {
        id: id.clone(),
        dir: dir.to_string_lossy().to_string(),
        name: if stem.is_empty() { id } else { stem },
        files: Vec::new(),
        pledit: None,
        viscolor: None,
    };
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| SkinError::NotAZip(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        // Two belts. `enclosed_name` refuses any entry that climbs out of the
        // archive's own root, and then only the basename of what is left is
        // used, so everything a skin brings lands in this one folder.
        let name = match entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        {
            Some(n) => n.to_lowercase(),
            None => continue,
        };
        let ext = Path::new(&name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !KEEP.contains(&ext.as_str()) {
            continue;
        }
        if entry.size() > MAX_FILE {
            return Err(SkinError::TooBig(format!(
                "{name} is {} bytes",
                entry.size()
            )));
        }
        total += entry.size();
        if total > MAX_TOTAL {
            return Err(SkinError::TooBig(format!(
                "more than {MAX_TOTAL} bytes unpacked"
            )));
        }
        // Read through a limit as well as trusting the header: the size in a
        // zip's directory is the zip's word, not a fact.
        let mut bytes = Vec::new();
        entry.by_ref().take(MAX_FILE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(SkinError::TooBig(format!(
                "{name} unpacks to more than {MAX_FILE} bytes"
            )));
        }
        match name.as_str() {
            "pledit.txt" => out.pledit = Some(text_of(bytes.clone())),
            "viscolor.txt" => out.viscolor = Some(text_of(bytes.clone())),
            _ => {}
        }
        fs::write(dir.join(&name), &bytes)?;
        out.files.push(name);
    }

    if !out.files.iter().any(|f| f.ends_with(".bmp")) {
        fs::remove_dir_all(&dir).ok();
        return Err(SkinError::Empty);
    }
    Ok(out)
}

/// What a skin folder holds, for rebuilding its manifest (D107).
pub struct Contents {
    pub files: Vec<String>,
    pub pledit: Option<String>,
    pub viscolor: Option<String>,
}

/// Read back what was unpacked: the art's file names and the two text files.
pub fn contents(dir: &Path) -> Contents {
    let mut out = Contents {
        files: Vec::new(),
        pledit: None,
        viscolor: None,
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let Some(name) = e.file_name().to_str().map(|n| n.to_lowercase()) else {
            continue;
        };
        if name == "manifest.json" {
            continue;
        }
        if name == "pledit.txt" {
            out.pledit = fs::read(e.path()).ok().map(text_of);
        }
        if name == "viscolor.txt" {
            out.viscolor = fs::read(e.path()).ok().map(text_of);
        }
        out.files.push(name);
    }
    out.files.sort();
    out
}

/// Write the manifest the frontend built, beside the art it describes.
pub fn write_manifest(skins_dir: &Path, id: &str, json: &str) -> Result<(), SkinError> {
    let dir = child_of(skins_dir, id)?;
    fs::write(dir.join("manifest.json"), json)?;
    Ok(())
}

/// Throw a skin away: the manifest it produced was refused, and a folder of
/// art nothing can load is rubbish, not a skin (skin-manifest.md).
pub fn discard(skins_dir: &Path, id: &str) -> Result<(), SkinError> {
    let dir = child_of(skins_dir, id)?;
    fs::remove_dir_all(dir)?;
    Ok(())
}

/// The skins that are installed: a folder with a manifest in it is a skin.
pub fn installed(skins_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(skins_dir) else {
        return out;
    };
    for e in entries.flatten() {
        if e.path().join("manifest.json").is_file() {
            if let Some(name) = e.file_name().to_str() {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}

/// An id is one folder name under the skins directory, and never a path.
fn child_of(skins_dir: &Path, id: &str) -> Result<PathBuf, SkinError> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(SkinError::Io(format!("{id:?} is not a skin id")));
    }
    Ok(skins_dir.join(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, bytes) in files {
                w.start_file(*name, opts).unwrap();
                w.write_all(bytes).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    fn write_zip(dir: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, zip_with(files)).unwrap();
        path
    }

    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("hp-skins-{}", std::process::id()));
        let p = p.join(
            format!("{:?}", std::time::SystemTime::now())
                .replace(|c: char| !c.is_ascii_alphanumeric(), ""),
        );
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn unpacks_the_files_a_skin_is_made_of_and_leaves_the_rest() {
        let t = temp();
        let zip = write_zip(
            &t,
            "My Cool Skin.wsz",
            &[
                ("MAIN.BMP", b"BM main"),
                ("nested/CBUTTONS.BMP", b"BM buttons"),
                ("PLEDIT.TXT", b"[Text]\r\nNormal=#00FF00\r\n"),
                ("VISCOLOR.TXT", b"0,0,0\r\n"),
                ("readme.nfo", b"hello"),
                ("cursor.cur", b"\0"),
            ],
        );
        let skins = t.join("skins");
        let out = unpack(&zip, &skins).unwrap();
        assert_eq!(out.id, "my-cool-skin");
        assert_eq!(out.name, "My Cool Skin");
        // Lower case, flattened, and only what a skin is made of.
        assert_eq!(
            out.files,
            vec!["main.bmp", "cbuttons.bmp", "pledit.txt", "viscolor.txt"]
        );
        assert!(skins.join("my-cool-skin/main.bmp").is_file());
        assert!(!skins.join("my-cool-skin/readme.nfo").exists());
        assert!(out.pledit.unwrap().contains("Normal"));
        assert_eq!(out.viscolor.unwrap().trim(), "0,0,0");
    }

    #[test]
    fn an_entry_that_climbs_out_of_the_folder_never_lands_anywhere() {
        let t = temp();
        let zip = write_zip(
            &t,
            "evil.wsz",
            &[
                ("../../../MAIN.BMP", b"BM"),
                ("skin/CBUTTONS.BMP", b"BM"),
                ("main.bmp", b"BM"),
            ],
        );
        let skins = t.join("skins");
        let out = unpack(&zip, &skins).unwrap();
        // The climbing entry is refused outright, the nested one is kept by
        // its basename, and nothing is written outside the skin's folder.
        assert_eq!(out.files, vec!["cbuttons.bmp", "main.bmp"]);
        assert!(skins.join("evil/main.bmp").is_file());
        assert!(skins.join("evil/cbuttons.bmp").is_file());
        assert!(!t.join("MAIN.BMP").exists());
    }

    #[test]
    fn two_skins_with_one_name_both_keep_their_art() {
        let t = temp();
        let skins = t.join("skins");
        let first = write_zip(&t, "base.wsz", &[("main.bmp", b"BM one")]);
        let a = unpack(&first, &skins).unwrap();
        let b = unpack(&first, &skins).unwrap();
        assert_eq!(a.id, "base");
        assert_eq!(b.id, "base-2");
        assert_eq!(fs::read(skins.join("base/main.bmp")).unwrap(), b"BM one");
    }

    #[test]
    fn a_zip_with_no_art_is_not_a_skin_and_leaves_nothing_behind() {
        let t = temp();
        let skins = t.join("skins");
        let zip = write_zip(&t, "notaskin.wsz", &[("readme.txt", b"hello")]);
        assert!(matches!(unpack(&zip, &skins), Err(SkinError::Empty)));
        assert!(!skins.join("notaskin").exists());
    }

    #[test]
    fn a_file_bigger_than_the_cap_is_refused() {
        let t = temp();
        let skins = t.join("skins");
        let big = vec![0u8; (MAX_FILE + 1) as usize];
        let zip = write_zip(&t, "big.wsz", &[("main.bmp", &big)]);
        assert!(matches!(unpack(&zip, &skins), Err(SkinError::TooBig(_))));
    }

    #[test]
    fn something_that_is_not_a_zip_says_so() {
        let t = temp();
        let path = t.join("main.bmp");
        fs::write(&path, b"BM not a zip at all").unwrap();
        assert!(matches!(
            unpack(&path, &t.join("skins")),
            Err(SkinError::NotAZip(_))
        ));
    }

    #[test]
    fn an_id_is_a_folder_name_and_never_a_path() {
        let t = temp();
        assert!(child_of(&t, "good-2").is_ok());
        for bad in ["../escape", "a/b", "", "C:\\windows"] {
            assert!(child_of(&t, bad).is_err(), "{bad:?} should not be an id");
        }
    }

    #[test]
    fn contents_reads_back_the_art_and_the_two_text_files() {
        let t = temp();
        let skins = t.join("skins");
        let zip = write_zip(
            &t,
            "again.wsz",
            &[
                ("MAIN.BMP", b"BM"),
                ("PLEDIT.TXT", b"[Text]\r\nNormal=#00FF00\r\n"),
                ("VISCOLOR.TXT", b"1,2,3\r\n"),
            ],
        );
        unpack(&zip, &skins).unwrap();
        fs::write(skins.join("again/manifest.json"), "{}").unwrap();
        let c = contents(&skins.join("again"));
        // The manifest itself is not art, and everything else comes back.
        assert_eq!(c.files, vec!["main.bmp", "pledit.txt", "viscolor.txt"]);
        assert!(c.pledit.unwrap().contains("Normal"));
        assert_eq!(c.viscolor.unwrap().trim(), "1,2,3");
    }

    #[test]
    fn installed_lists_the_folders_that_have_a_manifest() {
        let t = temp();
        let skins = t.join("skins");
        fs::create_dir_all(skins.join("one")).unwrap();
        fs::create_dir_all(skins.join("two")).unwrap();
        fs::write(skins.join("one/manifest.json"), "{}").unwrap();
        assert_eq!(installed(&skins), vec!["one"]);
    }
}
