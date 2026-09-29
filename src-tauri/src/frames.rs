//! A companion from frames (#208, D165): a folder of `<state>-<n>.png` at any
//! size, or an Aseprite sprite-sheet export with one tag per state, packed in
//! the app the way `tools/sheet.ps1 -Centre mass` packs (D164):
//!
//! - every pose is cut to its opaque box, and ONE factor sizes them all, so a
//!   crouch stays smaller than a stand;
//! - that factor fits the widest and the tallest pose to the 64 px cell, but
//!   art smaller than the cell is never stretched to fill it: it takes the
//!   largest whole factor that fits, so pixel art stays pixel art;
//! - each pose's weight sits in the middle of its cell, nudged only as far as
//!   it must be to stay inside, and its feet on the cell's bottom edge;
//! - colour is averaged over the area each cell pixel covers, weighted by
//!   alpha, which is exact at whole factors and even across frames otherwise.
//!
//! Both sheets are packed from the frames themselves, the `@2x` twin at twice
//! the factor (D160), and the manifest carries the format's timings.

use crate::painted::{self, Image, CELL, COLUMNS, STATES};
use std::fs;
use std::path::Path;

/// No frame file, sheet or data file is bigger than this.
const MAX_FILE: u64 = 32 * 1024 * 1024;

/// A state's frames, labelled for the errors.
type Poses = Vec<(String, Image)>;

/// Whether a file is named like one frame of a folder of frames.
pub fn is_frame_name(name: &str) -> bool {
    frame_of(name).is_some()
}

/// `walk-2.png` as (the walk row, 2).
fn frame_of(name: &str) -> Option<(usize, u32)> {
    let stem = name
        .len()
        .checked_sub(4)
        .filter(|&n| name[n..].eq_ignore_ascii_case(".png"))
        .map(|n| &name[..n])?;
    let (state, n) = stem.rsplit_once('-')?;
    let row = STATES.iter().position(|s| s.eq_ignore_ascii_case(state))?;
    Some((row, n.parse().ok()?))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let len = fs::metadata(path)
        .map_err(|e| format!("{name}: {e}"))?
        .len();
    if len > MAX_FILE {
        return Err(format!("{name} is too big"));
    }
    fs::read(path).map_err(|e| format!("{name}: {e}"))
}

/// The frames in a folder of `<state>-<n>.png`, each state's in the order of
/// its numbers. Any other file in the folder is left alone.
pub fn from_folder(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut found: Vec<(usize, u32, std::path::PathBuf)> = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| {
            let (row, n) = frame_of(&e.file_name().to_string_lossy())?;
            Some((row, n, e.path()))
        })
        .collect();
    found.sort_by_key(|(row, n, _)| (*row, *n));
    let mut states: Vec<Poses> = STATES.iter().map(|_| Vec::new()).collect();
    for (row, _, path) in found {
        let label = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let img = painted::decode_named(&read(&path)?, &label)?;
        states[row].push((label, img));
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "companion".into());
    pack(&name, states)
}

// ---- an Aseprite export ----

#[derive(serde::Deserialize)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(serde::Deserialize)]
struct AseFrame {
    frame: Rect,
    #[serde(default)]
    rotated: bool,
}

/// Aseprite writes its frames as an array, or as an object keyed by name.
/// Both are read in the order they are written, which is the frame order the
/// tags count in; a map type would sort the keys, and "wee 10" before "wee 2".
struct Frames(Vec<AseFrame>);

impl<'de> serde::Deserialize<'de> for Frames {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Frames;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("Aseprite's frames, as an array or an object")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut s: A) -> Result<Frames, A::Error> {
                let mut v = Vec::new();
                while let Some(f) = s.next_element()? {
                    v.push(f);
                }
                Ok(Frames(v))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut m: A) -> Result<Frames, A::Error> {
                let mut v = Vec::new();
                while let Some((_, f)) = m.next_entry::<serde::de::IgnoredAny, AseFrame>()? {
                    v.push(f);
                }
                Ok(Frames(v))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(serde::Deserialize)]
struct Tag {
    name: String,
    from: usize,
    to: usize,
    #[serde(default)]
    direction: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    image: String,
    #[serde(default)]
    frame_tags: Vec<Tag>,
}

#[derive(serde::Deserialize)]
struct AseData {
    frames: Frames,
    meta: Meta,
}

/// Whether a JSON file is Aseprite's sprite-sheet data.
pub fn is_aseprite(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .map(|v| {
            v["meta"]["app"]
                .as_str()
                .is_some_and(|a| a.contains("aseprite"))
                || v["meta"]["frameTags"].is_array()
        })
        .unwrap_or(false)
}

/// An Aseprite sprite-sheet export: the data file, and the sheet it names
/// beside it. Each tag named for a state gives that state its frames, in the
/// tag's order (reversed for a reverse tag); a tag with any other name is left
/// alone, as the format leaves an unknown state alone.
pub fn from_aseprite(data: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let text = String::from_utf8(read(data)?).map_err(|_| "the Aseprite data is not text")?;
    let ase: AseData = serde_json::from_str(&text)
        .map_err(|e| format!("that is not Aseprite's sprite-sheet data: {e}"))?;
    // The sheet beside the data file, by its own name only: nothing is read
    // from anywhere else.
    let image = Path::new(&ase.meta.image)
        .file_name()
        .ok_or("the Aseprite data names no sheet")?;
    let dir = data.parent().ok_or("that file has no folder")?;
    let sheet = painted::decode_named(&read(&dir.join(image))?, "the Aseprite sheet")?;
    let mut states: Vec<Poses> = STATES.iter().map(|_| Vec::new()).collect();
    let mut seen = [false; 7];
    for tag in &ase.meta.frame_tags {
        let Some(row) = STATES
            .iter()
            .position(|s| s.eq_ignore_ascii_case(tag.name.trim()))
        else {
            continue;
        };
        if seen[row] {
            return Err(format!("two tags are named {}; one per state", STATES[row]));
        }
        seen[row] = true;
        if tag.from > tag.to || tag.to >= ase.frames.0.len() {
            return Err(format!("the {} tag runs past the last frame", STATES[row]));
        }
        let mut order: Vec<usize> = (tag.from..=tag.to).collect();
        if tag.direction == "reverse" {
            order.reverse();
        }
        for (k, &i) in order.iter().enumerate() {
            let f = &ase.frames.0[i];
            if f.rotated {
                return Err("the sheet has rotated frames; export it without rotation".into());
            }
            // A frame with nothing drawn in it is one not drawn yet, and is
            // left out: a tag not drawn yet falls back to idle, as an empty
            // row of the painted template does (D163). Trimmed, such a frame
            // can come out with no size at all.
            if f.frame.w == 0 || f.frame.h == 0 {
                continue;
            }
            let img = crop_rect(&sheet, &f.frame)?;
            if img.px.chunks_exact(4).any(|p| p[3] > 8) {
                let label = format!("the {} tag's frame {}", STATES[row], k + 1);
                states[row].push((label, img));
            }
        }
    }
    if !seen[0] {
        return Err(
            "no tag is named idle: tag the frames idle, walk, dance, sleep, startle, pet and carry"
                .into(),
        );
    }
    if states[0].is_empty() {
        return Err(
            "nothing is drawn in the idle tag: its first frame is the one a companion cannot go without"
                .into(),
        );
    }
    let name = data
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "companion".into());
    pack(&name, states)
}

/// One frame of a sheet, by the rectangle the data gives it.
fn crop_rect(sheet: &Image, r: &Rect) -> Result<Image, String> {
    if r.w == 0 || r.h == 0 || r.x + r.w > sheet.w || r.y + r.h > sheet.h {
        return Err("a frame in the Aseprite data lies outside its sheet".into());
    }
    let mut px = Vec::with_capacity((r.w * r.h * 4) as usize);
    for y in r.y..r.y + r.h {
        let s = ((y * sheet.w + r.x) * 4) as usize;
        px.extend_from_slice(&sheet.px[s..s + (r.w * 4) as usize]);
    }
    Ok(Image { w: r.w, h: r.h, px })
}

// ---- packing ----

/// A pose cut to its opaque box, and where its weight sits across it.
struct Pose {
    img: Image,
    mass_x: f64,
}

fn cut(label: &str, img: &Image) -> Result<Pose, String> {
    let (mut x0, mut y0, mut x1, mut y1) = (img.w, img.h, 0, 0);
    for y in 0..img.h {
        for x in 0..img.w {
            if img.px[((y * img.w + x) * 4 + 3) as usize] > 8 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    if x1 == 0 {
        return Err(format!("{label} is empty"));
    }
    let img = crop_rect(
        img,
        &Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        },
    )?;
    let (mut weight, mut moment) = (0.0, 0.0);
    for y in 0..img.h {
        for x in 0..img.w {
            let a = img.px[((y * img.w + x) * 4 + 3) as usize];
            if a > 8 {
                weight += a as f64;
                moment += a as f64 * (x as f64 + 0.5);
            }
        }
    }
    Ok(Pose {
        img,
        mass_x: moment / weight,
    })
}

/// The frames, one list per state in the format's order, packed into a
/// finished pack: its manifest, the 1x sheet and the twin.
fn pack(name: &str, states: Vec<Poses>) -> Result<Vec<(String, Vec<u8>)>, String> {
    if states[0].is_empty() {
        return Err("there is no idle frame (idle-0.png): idle is the one state a companion cannot go without".into());
    }
    let mut poses: Vec<Vec<Pose>> = Vec::new();
    for (state, frames) in STATES.iter().zip(&states) {
        if frames.len() > COLUMNS as usize {
            return Err(format!(
                "{state} has {} frames; a state holds {COLUMNS}",
                frames.len()
            ));
        }
        poses.push(
            frames
                .iter()
                .map(|(l, i)| cut(l, i))
                .collect::<Result<_, _>>()?,
        );
    }
    let all = poses.iter().flatten();
    let widest = all.clone().map(|p| p.img.w).max().unwrap_or(1);
    let tallest = all.map(|p| p.img.h).max().unwrap_or(1);
    let mut factor = (CELL as f64 / widest as f64).min(CELL as f64 / tallest as f64);
    if factor >= 1.0 {
        factor = factor.floor();
    }
    let counts: Vec<u32> = poses.iter().map(|p| p.len() as u32).collect();
    Ok(vec![
        (
            "companion.json".into(),
            painted::manifest(name, &counts).into_bytes(),
        ),
        (
            "sheet.png".into(),
            painted::encode(&sheet(&poses, factor, 1))?,
        ),
        (
            "sheet@2x.png".into(),
            painted::encode(&sheet(&poses, factor, 2))?,
        ),
    ])
}

fn sheet(poses: &[Vec<Pose>], factor: f64, scale: u32) -> Image {
    let cell = CELL * scale;
    let (w, h) = (COLUMNS * cell, STATES.len() as u32 * cell);
    let mut out = Image {
        w,
        h,
        px: vec![0; (w * h * 4) as usize],
    };
    let f = factor * scale as f64;
    for (row, frames) in poses.iter().enumerate() {
        for (col, p) in frames.iter().enumerate() {
            let pw = ((p.img.w as f64 * f).round() as u32).clamp(1, cell);
            let ph = ((p.img.h as f64 * f).round() as u32).clamp(1, cell);
            let small = resize(&p.img, pw, ph);
            let left = ((cell as f64 / 2.0 - p.mass_x * f).round() as i64)
                .clamp(0, (cell - pw) as i64) as u32;
            let (x0, y0) = (col as u32 * cell + left, row as u32 * cell + cell - ph);
            for y in 0..ph {
                let s = (y * pw * 4) as usize;
                let d = (((y0 + y) * w + x0) * 4) as usize;
                out.px[d..d + (pw * 4) as usize]
                    .copy_from_slice(&small.px[s..s + (pw * 4) as usize]);
            }
        }
    }
    out
}

/// Which source pixels each destination pixel covers along one axis, and by
/// how much, summing to one.
fn spans(src: u32, dst: u32) -> Vec<Vec<(usize, f32)>> {
    let step = src as f64 / dst as f64;
    (0..dst)
        .map(|d| {
            let (lo, hi) = (d as f64 * step, (d + 1) as f64 * step);
            (lo.floor() as u32..(hi.ceil() as u32).min(src))
                .filter_map(|s| {
                    let cover = hi.min(s as f64 + 1.0) - lo.max(s as f64);
                    (cover > 1e-9).then(|| (s as usize, (cover / step) as f32))
                })
                .collect()
        })
        .collect()
}

/// Resampled by area, with colour weighted by alpha so the transparent
/// pixels around a pose never darken or lighten its edge. At a whole factor
/// up, every source pixel becomes a clean block.
fn resize(src: &Image, w: u32, h: u32) -> Image {
    let (sw, sh) = (src.w as usize, src.h as usize);
    let mut pm = vec![0f32; sw * sh * 4];
    for i in 0..sw * sh {
        let a = src.px[i * 4 + 3] as f32 / 255.0;
        for c in 0..3 {
            pm[i * 4 + c] = src.px[i * 4 + c] as f32 * a;
        }
        pm[i * 4 + 3] = a;
    }
    let (across, down) = (spans(src.w, w), spans(src.h, h));
    let w = w as usize;
    let mut mid = vec![0f32; w * sh * 4];
    for y in 0..sh {
        for (x, span) in across.iter().enumerate() {
            for &(s, k) in span {
                for c in 0..4 {
                    mid[(y * w + x) * 4 + c] += pm[(y * sw + s) * 4 + c] * k;
                }
            }
        }
    }
    let mut px = vec![0u8; w * h as usize * 4];
    for (y, span) in down.iter().enumerate() {
        for x in 0..w {
            let mut v = [0f32; 4];
            for &(s, k) in span {
                for (c, e) in v.iter_mut().enumerate() {
                    *e += mid[(s * w + x) * 4 + c] * k;
                }
            }
            let d = (y * w + x) * 4;
            if v[3] > 0.0 {
                for c in 0..3 {
                    px[d + c] = (v[c] / v[3]).round().clamp(0.0, 255.0) as u8;
                }
            }
            px[d + 3] = (v[3] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Image { w: w as u32, h, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("hp-frames-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A w x h canvas with an opaque block drawn in it.
    fn block(w: u32, h: u32, bx: u32, by: u32, bw: u32, bh: u32) -> Image {
        let mut img = Image {
            w,
            h,
            px: vec![0; (w * h * 4) as usize],
        };
        for y in by..by + bh {
            for x in bx..bx + bw {
                let i = ((y * w + x) * 4) as usize;
                img.px[i..i + 4].copy_from_slice(&[200, 100, 50, 255]);
            }
        }
        img
    }

    fn put(dir: &Path, name: &str, img: &Image) {
        fs::write(dir.join(name), painted::encode(img).unwrap()).unwrap();
    }

    fn sheets(files: &[(String, Vec<u8>)]) -> (serde_json::Value, Image, Image) {
        (
            serde_json::from_slice(&files[0].1).unwrap(),
            painted::decode_named(&files[1].1, "1x").unwrap(),
            painted::decode_named(&files[2].1, "2x").unwrap(),
        )
    }

    /// The opaque box of one cell of a sheet, relative to the cell.
    fn opaque_in(sheet: &Image, cell: u32, col: u32, row: u32) -> Option<(u32, u32, u32, u32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (cell, cell, 0, 0);
        for y in 0..cell {
            for x in 0..cell {
                let i = (((row * cell + y) * sheet.w + col * cell + x) * 4 + 3) as usize;
                if sheet.px[i] > 8 {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                }
            }
        }
        (x1 > 0).then_some((x0, y0, x1, y1))
    }

    #[test]
    fn frames_are_named_for_a_state_and_a_number() {
        assert!(is_frame_name("idle-0.png"));
        assert!(is_frame_name("Walk-12.PNG"));
        assert!(!is_frame_name("idle.png"));
        assert!(!is_frame_name("wave-0.png"));
        assert!(!is_frame_name("idle-x.png"));
        assert!(!is_frame_name("companion.json"));
    }

    #[test]
    fn big_frames_share_one_factor_so_the_tallest_fills_the_cell_and_a_crouch_stays_small() {
        let dir = tmp("big");
        // A stand 400 px tall and a crouch 200 px tall, on canvases of their own.
        put(&dir, "idle-0.png", &block(500, 500, 200, 90, 100, 400));
        put(&dir, "startle-0.png", &block(300, 300, 50, 50, 100, 200));
        put(&dir, "notes.txt.png", &block(4, 4, 0, 0, 1, 1)); // not a frame: left alone
        let (m, one, two) = sheets(&from_folder(&dir).unwrap());
        assert_eq!(
            m["name"],
            dir.file_name().unwrap().to_string_lossy().as_ref()
        );
        assert_eq!((one.w, one.h, two.w, two.h), (512, 448, 1024, 896));
        let stand = opaque_in(&one, 64, 0, 0).unwrap();
        let crouch = opaque_in(&one, 64, 0, 4).unwrap();
        assert_eq!(
            (stand.1, stand.3),
            (0, 64),
            "the tallest pose fills the cell, feet on the bottom"
        );
        assert_eq!(
            (crouch.1, crouch.3),
            (32, 64),
            "the crouch keeps half the height"
        );
        assert_eq!(
            opaque_in(&two, 128, 0, 0).unwrap().1,
            0,
            "the twin is packed at twice the factor"
        );
        assert_eq!(m["states"]["startle"]["frames"], serde_json::json!([32]));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pixel_art_smaller_than_the_cell_is_scaled_by_a_whole_number_and_stays_crisp() {
        let dir = tmp("pixel");
        // 24 px tall: 64/24 is 2.67, so 2 at 1x and 4 at 2x, never stretched to fill.
        let mut art = block(20, 24, 0, 0, 20, 24);
        art.px[0..4].copy_from_slice(&[10, 20, 30, 255]); // one odd pixel, top left
        put(&dir, "idle-0.png", &art);
        let (_, one, two) = sheets(&from_folder(&dir).unwrap());
        let b = opaque_in(&one, 64, 0, 0).unwrap();
        assert_eq!((b.2 - b.0, b.3 - b.1), (40, 48));
        let t = opaque_in(&two, 128, 0, 0).unwrap();
        assert_eq!((t.2 - t.0, t.3 - t.1), (80, 96));
        // The odd pixel is a clean 2x2 block at 1x, and nothing is blended.
        let at = |img: &Image, x: u32, y: u32| {
            let i = ((y * img.w + x) * 4) as usize;
            [img.px[i], img.px[i + 1], img.px[i + 2], img.px[i + 3]]
        };
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            assert_eq!(at(&one, b.0 + dx, b.1 + dy), [10, 20, 30, 255]);
        }
        assert_eq!(at(&one, b.0 + 2, b.1), [200, 100, 50, 255]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pose_is_centred_on_its_weight_and_nudged_to_stay_in_the_cell() {
        let dir = tmp("mass");
        // A body with a thin tail out to the left: centred on the body, not the box.
        let mut tailed = block(128, 64, 64, 0, 64, 64);
        for x in 0..64 {
            let i = ((62 * 128 + x) * 4) as usize;
            tailed.px[i..i + 4].copy_from_slice(&[200, 100, 50, 255]);
        }
        put(&dir, "idle-0.png", &tailed);
        put(&dir, "idle-1.png", &block(64, 64, 0, 0, 64, 64));
        let (_, one, _) = sheets(&from_folder(&dir).unwrap());
        let tail = opaque_in(&one, 64, 0, 0).unwrap();
        let body = opaque_in(&one, 64, 1, 0).unwrap();
        assert_eq!(
            (tail.0, tail.2),
            (0, 64),
            "the widest pose fills the cell, nudged inside"
        );
        assert!(
            body.0 > 10 && body.2 < 54,
            "a narrower pose sits on its weight: {body:?}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_long_strip_is_taken_since_the_limit_is_on_pixels_not_sides() {
        // Aseprite's rows export of nineteen 256 px frames is 4864 x 256.
        let strip = painted::encode(&block(4864, 4, 0, 0, 1, 1)).unwrap();
        assert!(painted::decode_named(&strip, "strip").is_ok());
        let wide = painted::encode(&block(16385, 1, 0, 0, 1, 1)).unwrap();
        let refused = painted::decode_named(&wide, "wide")
            .err()
            .unwrap_or_default();
        assert!(refused.contains("too big"), "{refused}");
    }

    #[test]
    fn a_folder_without_idle_or_with_nine_walks_is_refused() {
        let dir = tmp("refuse");
        put(&dir, "walk-0.png", &block(8, 8, 0, 0, 8, 8));
        assert!(from_folder(&dir).unwrap_err().contains("idle"));
        put(&dir, "idle-0.png", &block(8, 8, 0, 0, 8, 8));
        for n in 1..9 {
            put(&dir, &format!("walk-{n}.png"), &block(8, 8, 0, 0, 8, 8));
        }
        assert!(from_folder(&dir).unwrap_err().contains("walk has 9 frames"));
        let _ = fs::remove_dir_all(dir);
    }

    fn aseprite_sheet(dir: &Path) {
        // Four 16 px frames in a row, frame k drawn 4(k + 1) px tall, to tell them apart.
        let mut sheet = Image {
            w: 64,
            h: 16,
            px: vec![0; 64 * 16 * 4],
        };
        for k in 0..4u32 {
            for y in 16 - 4 * (k + 1)..16 {
                for x in k * 16 + 4..k * 16 + 12 {
                    let i = ((y * 64 + x) * 4) as usize;
                    sheet.px[i..i + 4].copy_from_slice(&[9, 9, 9, 255]);
                }
            }
        }
        put(dir, "sheet.png", &sheet);
    }

    fn frame(k: u32) -> serde_json::Value {
        serde_json::json!({ "frame": { "x": k * 16, "y": 0, "w": 16, "h": 16 }, "rotated": false })
    }

    #[test]
    fn an_aseprite_export_takes_each_state_from_its_tag_as_an_array_or_in_written_order() {
        let dir = tmp("ase");
        aseprite_sheet(&dir);
        let tags = serde_json::json!([
            { "name": "idle", "from": 0, "to": 0, "direction": "forward" },
            { "name": "wave-hello", "from": 0, "to": 3, "direction": "forward" },
            { "name": "Walk", "from": 1, "to": 3, "direction": "reverse" },
        ]);
        let array = serde_json::json!({
            "frames": [frame(0), frame(1), frame(2), frame(3)],
            "meta": { "app": "https://www.aseprite.org/", "image": "sheet.png", "frameTags": tags },
        });
        // Keys that sort differently from the order they are written in.
        let hash = format!(
            r#"{{"frames":{{"b 0":{},"a 1":{},"d 2":{},"c 3":{}}},"meta":{}}}"#,
            frame(0),
            frame(1),
            frame(2),
            frame(3),
            array["meta"]
        );
        fs::write(dir.join("array.json"), array.to_string()).unwrap();
        fs::write(dir.join("hash.json"), hash).unwrap();
        for file in ["array.json", "hash.json"] {
            let (m, one, _) = sheets(&from_aseprite(&dir.join(file)).unwrap());
            assert_eq!(m["name"], file.trim_end_matches(".json"));
            assert_eq!(
                m["states"]["walk"]["frames"],
                serde_json::json!([24, 25, 26])
            );
            assert!(m["states"].get("sleep").is_none());
            // Reversed: the tallest frame (the sheet's last) walks first.
            let heights: Vec<u32> = (0..3)
                .map(|c| opaque_in(&one, 64, c, 3).map(|b| b.3 - b.1).unwrap())
                .collect();
            assert!(
                heights[0] > heights[1] && heights[1] > heights[2],
                "{file}: {heights:?}"
            );
        }
        assert!(is_aseprite(
            &fs::read_to_string(dir.join("array.json")).unwrap()
        ));
        assert!(!is_aseprite(
            r#"{"format":"hp-companion/1","sprite":"sheet.png"}"#
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn frames_not_drawn_yet_are_left_out_and_an_undrawn_tag_falls_back_to_idle() {
        let dir = tmp("ase-empty");
        // Frame 0 drawn, frame 1 empty; frame 2 as a trimmed export gives an
        // empty one, with no size.
        put(&dir, "sheet.png", &block(32, 16, 4, 4, 8, 12));
        let empty = serde_json::json!({ "frame": { "x": 0, "y": 0, "w": 0, "h": 0 } });
        let data = |tags: serde_json::Value| {
            serde_json::json!({
                "frames": [frame(0), frame(1), empty],
                "meta": { "image": "sheet.png", "frameTags": tags },
            })
            .to_string()
        };
        let path = dir.join("starter.json");
        fs::write(
            &path,
            data(serde_json::json!([
                { "name": "idle", "from": 0, "to": 1 },
                { "name": "walk", "from": 1, "to": 2 },
            ])),
        )
        .unwrap();
        let (m, _, _) = sheets(&from_aseprite(&path).unwrap());
        assert_eq!(m["states"]["idle"]["frames"], serde_json::json!([0]));
        assert!(
            m["states"].get("walk").is_none(),
            "nothing drawn: idle instead"
        );
        fs::write(
            &path,
            data(serde_json::json!([{ "name": "idle", "from": 1, "to": 2 }])),
        )
        .unwrap();
        let refused = from_aseprite(&path).err().unwrap_or_default();
        assert!(
            refused.contains("nothing is drawn in the idle tag"),
            "{refused}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_aseprite_export_without_idle_or_with_rotated_frames_is_refused() {
        let dir = tmp("ase-refuse");
        aseprite_sheet(&dir);
        let data = |tags: serde_json::Value, rotated: bool| {
            let mut f = frame(0);
            f["rotated"] = serde_json::json!(rotated);
            serde_json::json!({ "frames": [f, frame(1)], "meta": { "image": "sheet.png", "frameTags": tags } })
                .to_string()
        };
        let path = dir.join("x.json");
        fs::write(
            &path,
            data(
                serde_json::json!([{ "name": "walk", "from": 0, "to": 1 }]),
                false,
            ),
        )
        .unwrap();
        assert!(from_aseprite(&path)
            .unwrap_err()
            .contains("no tag is named idle"));
        fs::write(
            &path,
            data(
                serde_json::json!([{ "name": "idle", "from": 0, "to": 1 }]),
                true,
            ),
        )
        .unwrap();
        assert!(from_aseprite(&path).unwrap_err().contains("rotated"));
        fs::write(
            &path,
            data(
                serde_json::json!([{ "name": "idle", "from": 0, "to": 5 }]),
                false,
            ),
        )
        .unwrap();
        assert!(from_aseprite(&path)
            .unwrap_err()
            .contains("past the last frame"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn wee_mans_own_2x_cells_as_a_folder_of_frames_pack_back_into_wee_man() {
        // His shipped twin, cut into its cells and brought in as frames: the
        // packer must lay them out and time them as his pack does (D164).
        let dir_in = Path::new(env!("CARGO_MANIFEST_DIR")).join("../skins/companions/wee-man");
        let his: serde_json::Value =
            serde_json::from_slice(&fs::read(dir_in.join("companion.json")).unwrap()).unwrap();
        let twin =
            painted::decode_named(&fs::read(dir_in.join("sheet@2x.png")).unwrap(), "twin").unwrap();
        let dir = tmp("wee");
        for (row, state) in STATES.iter().enumerate() {
            for col in 0..COLUMNS {
                let cell = crop_rect(
                    &twin,
                    &Rect {
                        x: col * 128,
                        y: row as u32 * 128,
                        w: 128,
                        h: 128,
                    },
                )
                .unwrap();
                if cell.px.chunks_exact(4).any(|p| p[3] > 8) {
                    put(&dir, &format!("{state}-{col}.png"), &cell);
                }
            }
        }
        let (m, _, _) = sheets(&from_folder(&dir).unwrap());
        assert_eq!(m["states"], his["states"]);
        let _ = fs::remove_dir_all(dir);
    }
}
