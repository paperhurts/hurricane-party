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

/// What an `hp-skin/1` pack keeps besides its manifest (#146): the PNGs a
/// painted or made skin draws from, and the BMPs and text files a zipped
/// import folder carries, which its manifest still names.
const KEEP_NATIVE: [&str; 3] = ["png", "bmp", "txt"];

/// A native skin is a folder with this in it (`installed`), and so is a pack.
const MANIFEST: &str = "manifest.json";

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
    /// An `hp-skin/1` pack's own manifest, as written (#146). None for a
    /// `.wsz`, whose manifest the webview builds from the art.
    pub manifest: Option<String>,
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

/// A pack's manifest names the skin; a manifest that does not parse, or
/// has no name, leaves the file or folder name to do it.
fn name_in(manifest: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(manifest)
        .ok()?
        .get("name")?
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Unpack one skin zip into `skins_dir`, and say what came out: a `.wsz`, or
/// a zip of an `hp-skin/1` folder, which has a `manifest.json` in it (#146).
pub fn unpack(zip_path: &Path, skins_dir: &Path) -> Result<Unpacked, SkinError> {
    let file = fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| SkinError::NotAZip(e.to_string()))?;
    if zip.len() > MAX_ENTRIES {
        return Err(SkinError::TooBig(format!("{} files in the zip", zip.len())));
    }
    // Only basenames are ever used, so two manifests would be two skins
    // poured into one folder.
    let manifests = zip
        .file_names()
        .filter(|n| {
            Path::new(n)
                .file_name()
                .is_some_and(|b| b.to_string_lossy().eq_ignore_ascii_case(MANIFEST))
        })
        .count();
    if manifests > 1 {
        return Err(SkinError::Io(
            "that zip holds more than one skin; zip one skin's folder at a time".into(),
        ));
    }
    let native = manifests == 1;

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
        manifest: None,
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
        let keep = if native {
            name == MANIFEST || KEEP_NATIVE.contains(&ext.as_str())
        } else {
            KEEP.contains(&ext.as_str())
        };
        if !keep {
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
            MANIFEST => out.manifest = Some(text_of(bytes.clone())),
            _ => {}
        }
        fs::write(dir.join(&name), &bytes)?;
        if name != MANIFEST {
            out.files.push(name);
        }
    }

    let art = |f: &String| f.ends_with(".bmp") || (native && f.ends_with(".png"));
    if !out.files.iter().any(art) || (native && out.manifest.is_none()) {
        fs::remove_dir_all(&dir).ok();
        return Err(SkinError::Empty);
    }
    if let Some(name) = out.manifest.as_deref().and_then(name_in) {
        out.name = name;
    }
    Ok(out)
}

/// Copy an `hp-skin/1` folder into `skins_dir` (#146), given its manifest:
/// the file a person picks, since one dialog then serves a `.wsz`, a zip and
/// a painted folder. The same caps and the same basename-only rule as a zip,
/// and the folder's own subfolders are not followed.
pub fn copy_folder(manifest_path: &Path, skins_dir: &Path) -> Result<Unpacked, SkinError> {
    let src = manifest_path
        .parent()
        .ok_or_else(|| SkinError::Io("that manifest is not in a folder".into()))?;
    let manifest_size = fs::metadata(manifest_path)?.len();
    if manifest_size > MAX_FILE {
        return Err(SkinError::TooBig(format!(
            "{MANIFEST} is {manifest_size} bytes"
        )));
    }
    let manifest = text_of(fs::read(manifest_path)?);

    let mut art: Vec<(String, PathBuf)> = Vec::new();
    let mut total = manifest_size;
    for e in fs::read_dir(src)?.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_lowercase();
        let ext = Path::new(&name)
            .extension()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name == MANIFEST || !KEEP_NATIVE.contains(&ext.as_str()) {
            continue;
        }
        let size = e.metadata()?.len();
        if size > MAX_FILE {
            return Err(SkinError::TooBig(format!("{name} is {size} bytes")));
        }
        total += size;
        if total > MAX_TOTAL {
            return Err(SkinError::TooBig(format!(
                "more than {MAX_TOTAL} bytes of art"
            )));
        }
        art.push((name, path));
        if art.len() > MAX_ENTRIES {
            return Err(SkinError::TooBig(format!("more than {MAX_ENTRIES} files")));
        }
    }
    if !art
        .iter()
        .any(|(n, _)| n.ends_with(".png") || n.ends_with(".bmp"))
    {
        return Err(SkinError::Empty);
    }

    let stem = src
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    fs::create_dir_all(skins_dir)?;
    let id = slug_for(skins_dir, &stem);
    let dir = skins_dir.join(&id);
    fs::create_dir_all(&dir)?;
    let mut out = Unpacked {
        id: id.clone(),
        dir: dir.to_string_lossy().to_string(),
        name: name_in(&manifest).unwrap_or(if stem.is_empty() { id } else { stem }),
        files: Vec::new(),
        pledit: None,
        viscolor: None,
        manifest: Some(manifest.clone()),
    };
    fs::write(dir.join(MANIFEST), manifest.as_bytes())?;
    art.sort();
    for (name, path) in art {
        let bytes = fs::read(&path)?;
        match name.as_str() {
            "pledit.txt" => out.pledit = Some(text_of(bytes.clone())),
            "viscolor.txt" => out.viscolor = Some(text_of(bytes.clone())),
            _ => {}
        }
        fs::write(dir.join(&name), &bytes)?;
        out.files.push(name);
    }
    Ok(out)
}

// ---- a template to paint (#146) ----

/// The folder `start_template` made this session, and the only place
/// `write_template_file` writes. The webview names the parent through the
/// OS dialog; it never names a path to write to.
#[derive(Default)]
pub struct TemplateDir(pub std::sync::Mutex<Option<PathBuf>>);

/// What a template is: the manifest, the sheet to paint, the guide that
/// names every part of it, and a note on how.
const TEMPLATE_FILES: [&str; 4] = [MANIFEST, "chrome.png", "guide.png", "README.txt"];

/// Make a new, empty folder for a template inside the one a person chose.
/// A name already there takes a number, so nothing is ever painted over.
pub fn start_template(parent: &Path, name: &str) -> Result<PathBuf, SkinError> {
    if !parent.is_dir() {
        return Err(SkinError::Io("that is not a folder".into()));
    }
    let dir = parent.join(slug_for(parent, name));
    fs::create_dir(&dir)?;
    Ok(dir)
}

/// Write one of a template's four files into its folder.
pub fn write_template_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), SkinError> {
    if !TEMPLATE_FILES.contains(&name) {
        return Err(SkinError::Io(format!("{name:?} is not part of a template")));
    }
    if bytes.len() as u64 > MAX_FILE {
        return Err(SkinError::TooBig(format!("{name} is too big")));
    }
    fs::write(dir.join(name), bytes)?;
    Ok(())
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

// ---- making a skin from a picture (#131) ------------------------------------

/// The sheets a made skin wears: Eyewall's own. They are mask art the renderer
/// tints (D73), so a made skin needs no drawing at all — its palette is what
/// makes it look like its picture, and the picture itself is the backdrop.
const EYEWALL_CHROME: &[u8] = include_bytes!("../../skins/eyewall/chrome.png");
const EYEWALL_CHROME_2X: &[u8] = include_bytes!("../../skins/eyewall/chrome@2x.png");

/// The largest picture the maker reads. A phone photo is a few megabytes; this
/// is room for a large one without handing the webview something that will
/// stall it decoding.
const MAX_PICTURE: u64 = 25 * 1024 * 1024;

/// A made skin's folder, before its manifest exists.
#[derive(serde::Serialize)]
pub struct Made {
    pub id: String,
    pub dir: String,
}

/// Start a made skin: a folder named after it, with Eyewall's sheets in it.
/// The picture and the manifest follow; a skin that never gets its manifest is
/// thrown away by the caller, the same as a refused import.
pub fn make(skins_dir: &Path, name: &str) -> Result<Made, SkinError> {
    fs::create_dir_all(skins_dir)?;
    let id = slug_for(skins_dir, name);
    let dir = skins_dir.join(&id);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("chrome.png"), EYEWALL_CHROME)?;
    fs::write(dir.join("chrome@2x.png"), EYEWALL_CHROME_2X)?;
    Ok(Made {
        id,
        dir: dir.to_string_lossy().into_owned(),
    })
}

/// A picture's first bytes, for the four formats a webview decodes. The
/// extension is not trusted: a file picked in a dialog can be called anything.
fn is_picture(b: &[u8]) -> bool {
    b.starts_with(&[0x89, b'P', b'N', b'G'])
        || b.starts_with(&[0xFF, 0xD8, 0xFF])
        || b.starts_with(b"GIF8")
        || (b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP")
}

/// Read a picture a person chose in a dialog, for the webview to decode.
///
/// Through Rust rather than the asset protocol because the asset scope is the
/// app's own folders, and a picture lives wherever the person keeps it. It is
/// read once, here, and never stored: what the skin keeps is the crop the
/// webview makes, written back with `write_picture`.
pub fn read_picture(path: &Path) -> Result<Vec<u8>, SkinError> {
    let size = fs::metadata(path)?.len();
    if size > MAX_PICTURE {
        return Err(SkinError::TooBig(format!(
            "that picture is {} MB; the maker reads up to {} MB",
            size / (1024 * 1024),
            MAX_PICTURE / (1024 * 1024)
        )));
    }
    let bytes = fs::read(path)?;
    if !is_picture(&bytes) {
        return Err(SkinError::Io(
            "that file is not a PNG, JPEG, GIF or WebP picture".into(),
        ));
    }
    Ok(bytes)
}

/// Write the backdrop cropped from that picture into a made skin's folder.
/// Two names only, PNG only, and no bigger than any sheet an import may write:
/// the webview made these bytes, but the command is callable, so it is checked
/// like any other input.
pub fn write_picture(skins_dir: &Path, id: &str, scale: u8, bytes: &[u8]) -> Result<(), SkinError> {
    let name = match scale {
        1 => "picture.png",
        2 => "picture@2x.png",
        _ => return Err(SkinError::Io(format!("no picture at scale {scale}"))),
    };
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Err(SkinError::Io("a skin's picture is written as PNG".into()));
    }
    if bytes.len() as u64 > MAX_FILE {
        return Err(SkinError::TooBig(format!("{name} is too big")));
    }
    let dir = child_of(skins_dir, id)?;
    fs::write(dir.join(name), bytes)?;
    Ok(())
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

    #[test]
    fn a_made_skin_starts_as_a_folder_of_eyewall_sheets() {
        let root = temp();
        let made = make(&root, "Storm Kitty").unwrap();
        assert_eq!(made.id, "storm-kitty");
        let dir = root.join(&made.id);
        assert_eq!(fs::read(dir.join("chrome.png")).unwrap(), EYEWALL_CHROME);
        assert_eq!(
            fs::read(dir.join("chrome@2x.png")).unwrap(),
            EYEWALL_CHROME_2X
        );
        // A second skin of the same name keeps both, as a second import does.
        assert_eq!(make(&root, "Storm Kitty").unwrap().id, "storm-kitty-2");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn only_a_picture_is_read_and_only_a_png_is_written() {
        let root = temp();
        let png = [0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0];
        let not = root.join("notes.txt");
        fs::write(&not, b"just some words").unwrap();
        assert!(read_picture(&not).is_err(), "an extension is not trusted");
        let pic = root.join("cat.dat");
        fs::write(&pic, png).unwrap();
        assert!(read_picture(&pic).is_ok(), "a PNG called anything is a PNG");

        let made = make(&root, "cat").unwrap();
        assert!(write_picture(&root, &made.id, 1, &png).is_ok());
        assert!(root.join(&made.id).join("picture.png").is_file());
        assert!(
            write_picture(&root, &made.id, 3, &png).is_err(),
            "no scale 3"
        );
        assert!(
            write_picture(&root, &made.id, 1, b"GIF89a").is_err(),
            "PNG only"
        );
        // The id is a path segment inside the skins folder, never a way out.
        assert!(write_picture(&root, "../escape", 1, &png).is_err());
        fs::remove_dir_all(&root).ok();
    }
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

    #[test]
    fn a_zipped_hp_skin_folder_keeps_its_manifest_and_its_pngs() {
        let t = temp();
        let skins = t.join("skins");
        let manifest = br#"{ "format": "hp-skin/1", "name": "Painted Storm" }"#;
        let zip = write_zip(
            &t,
            "my-skin.zip",
            &[
                ("my-skin/manifest.json", manifest),
                ("my-skin/chrome.png", b"png bytes"),
                ("my-skin/guide.png", b"png bytes"),
                ("my-skin/README.txt", b"paint it"),
                ("my-skin/tool.exe", b"MZ"),
            ],
        );
        let out = unpack(&zip, &skins).unwrap();
        assert_eq!(out.name, "Painted Storm");
        assert_eq!(
            out.manifest.as_deref(),
            Some(std::str::from_utf8(manifest).unwrap())
        );
        assert_eq!(out.files, ["chrome.png", "guide.png", "readme.txt"]);
        let dir = skins.join(&out.id);
        assert!(dir.join("manifest.json").is_file());
        assert!(!dir.join("tool.exe").exists());
        fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_zip_with_two_skins_in_it_is_refused() {
        let t = temp();
        let zip = write_zip(
            &t,
            "two.zip",
            &[
                ("a/manifest.json", b"{}"),
                ("a/chrome.png", b"x"),
                ("b/manifest.json", b"{}"),
            ],
        );
        assert!(unpack(&zip, &t.join("skins")).is_err());
        fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_painted_folder_is_copied_by_its_manifest_and_nothing_else_comes_along() {
        let t = temp();
        let src = t.join("Painted");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("manifest.json"), br#"{ "name": "Mine" }"#).unwrap();
        fs::write(src.join("chrome.png"), b"png").unwrap();
        fs::write(src.join("guide.png"), b"png").unwrap();
        fs::write(src.join("notes.docx"), b"no").unwrap();
        fs::write(src.join("sub").join("deep.png"), b"no").unwrap();
        let skins = t.join("skins");
        let out = copy_folder(&src.join("manifest.json"), &skins).unwrap();
        assert_eq!((out.id.as_str(), out.name.as_str()), ("painted", "Mine"));
        assert_eq!(out.files, ["chrome.png", "guide.png"]);
        assert!(skins.join("painted").join("manifest.json").is_file());
        assert!(!skins.join("painted").join("deep.png").exists());
        // A folder with a manifest and no art is not a skin.
        let bare = t.join("bare");
        fs::create_dir_all(&bare).unwrap();
        fs::write(bare.join("manifest.json"), b"{}").unwrap();
        assert!(matches!(
            copy_folder(&bare.join("manifest.json"), &skins),
            Err(SkinError::Empty)
        ));
        fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_template_goes_in_a_new_folder_and_writes_only_its_own_files() {
        let t = temp();
        let a = start_template(&t, "my-skin").unwrap();
        let b = start_template(&t, "my-skin").unwrap();
        assert_ne!(a, b, "a second template never paints over the first");
        assert!(write_template_file(&a, "chrome.png", b"png").is_ok());
        assert!(write_template_file(&a, "README.txt", b"hello").is_ok());
        assert!(write_template_file(&a, "../escape.png", b"x").is_err());
        assert!(write_template_file(&a, "run.bat", b"x").is_err());
        assert!(start_template(&t.join("nope"), "my-skin").is_err());
        fs::remove_dir_all(&t).ok();
    }

    fn temp() -> PathBuf {
        // A counter as well as the clock: tests run in parallel, two can read
        // the same instant, and a test that cleans up after itself would
        // then delete the other's folder out from under it.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("hp-skins-{}", std::process::id()));
        let p = p.join(format!(
            "{}-{n}",
            format!("{:?}", std::time::SystemTime::now())
                .replace(|c: char| !c.is_ascii_alphanumeric(), "")
        ));
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
