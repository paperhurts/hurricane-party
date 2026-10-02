//! A companion painted on the template (#208, D163): the sheet a person
//! painted, turned into a pack. The template's `companion.json` says
//! `"painted": true`; the rest is read off the sheet itself:
//!
//! - one row per state, in the format's order, up to 32 cells a row (the
//!   template is 32 across, and one written before D183 is 8);
//! - a cell counts once anything is painted in it, and a row's frames are its
//!   painted cells from the left, so a gap is a mistake worth saying;
//! - idle's first cell is the one cell a companion cannot go without.
//!
//! Painted at 64 px a cell (true pixel art, as Aseprite draws it), the sheet
//! is the 1x and is doubled pixel for pixel for the `@2x` twin (D160), which
//! stays crisp. Painted at 128 px, it is the twin, and the 1x is its 2x2
//! average. The timings are the format's, the ones `tools/sheet.ps1` writes,
//! and so is the layout: as wide as the longest row and never narrower than
//! eight, whatever width it was painted on.

/// The format's vocabulary, in the sheet's row order (`purricane.md`).
pub const STATES: [&str; 7] = ["idle", "sleep", "dance", "walk", "startle", "pet", "carry"];
/// A finished sheet is never narrower than this, the layout every pack had
/// before D183, so a pack of eight or fewer poses a state comes out as it did.
pub const MIN_COLUMNS: u32 = 8;
/// The most poses a state holds: 32 cells of 128 px, the twin, is 4096 px.
pub const MAX_COLUMNS: u32 = 32;
pub const CELL: u32 = 64;

/// How wide a finished sheet is, in cells: the longest state, and never
/// narrower than `MIN_COLUMNS`.
pub(crate) fn columns_for(counts: &[u32]) -> u32 {
    counts.iter().copied().fold(MIN_COLUMNS, u32::max)
}

/// Whether a manifest is a painted template waiting to be read.
pub fn is_painted(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .map(|v| v["painted"] == true)
        .unwrap_or(false)
}

/// Straight RGBA8, top-down.
pub(crate) struct Image {
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) px: Vec<u8>,
}

/// The painted pack's three files: its manifest, the 1x sheet and the twin.
pub fn finish(json: &str, sheet_png: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let name = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v["name"].as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "My companion".into());
    let sheet = decode(sheet_png)?;
    let rows = STATES.len() as u32;
    // The height says the size it was painted at, and the width how many
    // cells a row has: 32 on the template, 8 on one written before D183.
    let scale = [1, 2].into_iter().find(|s| sheet.h == s * rows * CELL);
    let across = scale.map(|s| (sheet.w / (s * CELL), sheet.w % (s * CELL)));
    if !matches!(across, Some((1..=MAX_COLUMNS, 0))) {
        return Err(format!(
            "the painted sheet is {}x{}; the template's is {}x{}, {MAX_COLUMNS} cells of {CELL} px across (any number up to {MAX_COLUMNS} will do), or {}x{} painted at twice the size",
            sheet.w,
            sheet.h,
            MAX_COLUMNS * CELL,
            rows * CELL,
            2 * MAX_COLUMNS * CELL,
            2 * rows * CELL
        ));
    }
    let (one, painted_twin) = if scale == Some(1) {
        (sheet, None)
    } else {
        (halve(&sheet), Some(sheet))
    };
    let counts = count_cells(&one)?;
    // Laid out as wide as the longest row, so the same painting comes out the
    // same pack on a template of any width, and an 8-wide one as it always did.
    let columns = columns_for(&counts);
    let one = fit(one, columns * CELL);
    let two = match painted_twin {
        Some(twin) => fit(twin, 2 * columns * CELL),
        None => double(&one),
    };
    let manifest = manifest(&name, &counts, columns);
    Ok(vec![
        ("companion.json".into(), manifest.into_bytes()),
        ("sheet.png".into(), encode(&one)?),
        ("sheet@2x.png".into(), encode(&two)?),
    ])
}

/// How many cells each row has painted, left to right. A painted cell after
/// an empty one in the same row is refused with where it is, since the
/// frames would otherwise skip it silently.
fn count_cells(sheet: &Image) -> Result<Vec<u32>, String> {
    let mut counts = Vec::new();
    for (r, state) in STATES.iter().enumerate() {
        let painted: Vec<bool> = (0..sheet.w / CELL)
            .map(|c| cell_painted(sheet, c, r as u32))
            .collect();
        let n = painted.iter().take_while(|p| **p).count() as u32;
        if let Some(later) = painted.iter().skip(n as usize).position(|p| *p) {
            return Err(format!(
                "the {state} row has cell {} painted after an empty cell {}; paint cells left to right",
                n as usize + later + 1,
                n + 1
            ));
        }
        counts.push(n);
    }
    if counts[0] == 0 {
        return Err("nothing is painted in the idle row: its first cell is the one a companion cannot go without".into());
    }
    Ok(counts)
}

fn cell_painted(sheet: &Image, c: u32, r: u32) -> bool {
    let (x0, y0) = (c * CELL, r * CELL);
    (y0..y0 + CELL)
        .any(|y| (x0..x0 + CELL).any(|x| sheet.px[((y * sheet.w + x) * 4 + 3) as usize] > 8))
}

/// The manifest, with the format's timings, for a sheet `columns` cells wide.
/// Idle's first frame is the pose and each later one a moment in it, as
/// `tools/sheet.ps1` writes it: the pose held eleven frames, the moment once
/// (a blink every 3 s at 4 fps).
pub(crate) fn manifest(name: &str, counts: &[u32], columns: u32) -> String {
    let mut states = serde_json::Map::new();
    for (r, (state, &n)) in STATES.iter().zip(counts).enumerate() {
        if n == 0 {
            continue;
        }
        let first = r as u32 * columns;
        let mut frames: Vec<u32> = (first..first + n).collect();
        if *state == "idle" && n > 1 {
            frames = frames[1..]
                .iter()
                .flat_map(|&m| std::iter::repeat_n(first, 11).chain(std::iter::once(m)))
                .collect();
        }
        let timing = match *state {
            "idle" => serde_json::json!({ "fps": 4, "loop": true }),
            "sleep" => serde_json::json!({ "fps": 1, "loop": true }),
            "dance" => serde_json::json!({ "syncTo": "beat" }),
            "walk" => serde_json::json!({ "fps": 8, "loop": true }),
            "startle" => serde_json::json!({ "fps": 12, "loop": false, "then": "idle" }),
            "pet" => serde_json::json!({ "fps": 6, "loop": false, "then": "idle" }),
            _ => serde_json::json!({ "fps": 3, "loop": true }),
        };
        let mut entry = timing.as_object().cloned().unwrap_or_default();
        entry.insert("frames".into(), serde_json::json!(frames));
        states.insert((*state).into(), serde_json::Value::Object(entry));
    }
    let m = serde_json::json!({
        "format": "hp-companion/1",
        "name": name,
        "sprite": "sheet.png",
        "frameSize": [CELL, CELL],
        "anchor": [CELL / 2, CELL - 1],
        "palette": "fixed",
        "states": states,
        "walkPxPerSec": 24,
        "defaultCount": 1,
    });
    serde_json::to_string_pretty(&m).unwrap_or_default()
}

/// The same rows cut or widened to `w`: cells past the last painted one are
/// empty, and a narrower template's missing cells are transparent.
fn fit(src: Image, w: u32) -> Image {
    if src.w == w {
        return src;
    }
    let keep = (src.w.min(w) * 4) as usize;
    let mut px = vec![0u8; (w * src.h * 4) as usize];
    for (y, row) in px.chunks_exact_mut((w * 4) as usize).enumerate() {
        let s = y * (src.w * 4) as usize;
        row[..keep].copy_from_slice(&src.px[s..s + keep]);
    }
    Image { w, h: src.h, px }
}

/// Every pixel twice across and twice down: pixel art stays pixel art.
fn double(src: &Image) -> Image {
    let (w, h) = (src.w * 2, src.h * 2);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let s = (((y / 2) * src.w + x / 2) * 4) as usize;
            let d = ((y * w + x) * 4) as usize;
            px[d..d + 4].copy_from_slice(&src.px[s..s + 4]);
        }
    }
    Image { w, h, px }
}

/// Each 2x2 block averaged, colour weighted by alpha so a transparent pixel
/// does not darken its neighbours' edges.
fn halve(src: &Image) -> Image {
    let (w, h) = (src.w / 2, src.h / 2);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let s = (((2 * y + dy) * src.w + 2 * x + dx) * 4) as usize;
                let pa = src.px[s + 3] as u32;
                r += src.px[s] as u32 * pa;
                g += src.px[s + 1] as u32 * pa;
                b += src.px[s + 2] as u32 * pa;
                a += pa;
            }
            let d = ((y * w + x) * 4) as usize;
            if a > 0 {
                px[d] = (r / a) as u8;
                px[d + 1] = (g / a) as u8;
                px[d + 2] = (b / a) as u8;
            }
            px[d + 3] = (a / 4) as u8;
        }
    }
    Image { w, h, px }
}

fn decode(bytes: &[u8]) -> Result<Image, String> {
    decode_named(bytes, "the sheet")
}

/// A PNG as straight RGBA8, refused before anything is allocated when it is
/// bigger than a companion could want (`purricane.md`: a 16k x 16k PNG is a
/// denial of service dressed as a unicorn). `what` names it in the errors.
pub(crate) fn decode_named(bytes: &[u8], what: &str) -> Result<Image, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec
        .read_info()
        .map_err(|e| format!("{what} is not a PNG: {e}"))?;
    // What costs memory is the pixels, not the length of a side: an Aseprite
    // strip of nineteen 256 px frames is 4864 px long and harmless (D165).
    let (w, h) = (reader.info().width, reader.info().height);
    if w > 16384 || h > 16384 || w as u64 * h as u64 > 4096 * 4096 {
        return Err(format!("{what} is {w}x{h}; too big to be a companion"));
    }
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| format!("{what} is too large"))?
    ];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("{what} would not decode: {e}"))?;
    let n = (w * h) as usize;
    let px = match info.color_type {
        png::ColorType::Rgba => buf[..n * 4].to_vec(),
        png::ColorType::Rgb => buf[..n * 3]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..n * 2]
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf[..n].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(format!("{what}'s palette did not expand")),
    };
    Ok(Image { w, h, px })
}

pub(crate) fn encode(img: &Image) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.w, img.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&img.px).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A template from before D183, 8 cells wide, at `scale` (1 or 2).
    fn painted(scale: u32, counts: &[u32; 7]) -> Vec<u8> {
        painted_on(scale, 8, counts)
    }

    /// A template `columns` cells wide at `scale` (1 or 2), with the given
    /// number of painted cells per row: each painted cell has one opaque
    /// pixel, coloured by where it is.
    fn painted_on(scale: u32, columns: u32, counts: &[u32; 7]) -> Vec<u8> {
        let cell = CELL * scale;
        let mut img = Image {
            w: columns * cell,
            h: 7 * cell,
            px: vec![0; (columns * cell * 7 * cell * 4) as usize],
        };
        for (r, &n) in counts.iter().enumerate() {
            for c in 0..n {
                let (x, y) = (c * cell + cell / 2, r as u32 * cell + cell - 1);
                let i = ((y * img.w + x) * 4) as usize;
                img.px[i..i + 4].copy_from_slice(&[200, 100 + c as u8, 50 + r as u8, 255]);
            }
        }
        encode(&img).unwrap()
    }

    const JSON: &str =
        r#"{"format":"hp-companion/1","name":"Sir Waddles","sprite":"sheet.png","painted":true}"#;

    fn manifest_of(files: &[(String, Vec<u8>)]) -> serde_json::Value {
        serde_json::from_slice(&files[0].1).unwrap()
    }

    #[test]
    fn the_captains_own_sheet_painted_in_the_template_is_the_captain() {
        // His sheet is laid out the way the template is, so read as a painted
        // one it must come back as him: the same frames, the same timings.
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../skins/companions/captain");
        let sheet = std::fs::read(dir.join("sheet.png")).unwrap();
        let his: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("companion.json")).unwrap()).unwrap();
        let files = finish(JSON, &sheet).unwrap();
        assert_eq!(manifest_of(&files)["states"], his["states"]);
    }

    #[test]
    fn a_template_says_it_is_painted_and_a_finished_pack_does_not() {
        assert!(is_painted(JSON));
        assert!(!is_painted(
            r#"{"format":"hp-companion/1","sprite":"sheet.png"}"#
        ));
        assert!(!is_painted("not json"));
    }

    #[test]
    fn the_painted_cells_become_each_states_frames() {
        let files = finish(JSON, &painted(1, &[2, 2, 4, 4, 0, 1, 2])).unwrap();
        let m = manifest_of(&files);
        assert_eq!(m["name"], "Sir Waddles");
        assert_eq!(
            m["states"]["walk"]["frames"],
            serde_json::json!([24, 25, 26, 27])
        );
        assert_eq!(m["states"]["dance"]["syncTo"], "beat");
        assert_eq!(m["states"]["pet"]["frames"], serde_json::json!([40]));
        assert!(
            m["states"].get("startle").is_none(),
            "an empty row falls back to idle"
        );
        let idle: Vec<u64> = m["states"]["idle"]["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect();
        assert_eq!(
            idle.len(),
            12,
            "the pose held eleven frames, the blink once"
        );
        assert_eq!((idle[0], idle[11]), (0, 1));
        assert!(
            m.get("painted").is_none(),
            "the finished manifest is not a template"
        );
    }

    #[test]
    fn pixel_art_at_64_doubles_for_the_twin_and_a_128_painting_halves_for_the_1x() {
        for scale in [1, 2] {
            let files = finish(JSON, &painted(scale, &[1, 0, 0, 0, 0, 0, 0])).unwrap();
            let one = decode(&files[1].1).unwrap();
            let two = decode(&files[2].1).unwrap();
            assert_eq!((one.w, one.h), (512, 448), "scale {scale}");
            assert_eq!((two.w, two.h), (1024, 896), "scale {scale}");
        }
        let files = finish(JSON, &painted(1, &[1, 0, 0, 0, 0, 0, 0])).unwrap();
        let two = decode(&files[2].1).unwrap();
        let at = |x: u32, y: u32| two.px[((y * two.w + x) * 4 + 3) as usize];
        assert_eq!(
            (at(64, 126), at(65, 127)),
            (255, 255),
            "one pixel became a 2x2 block"
        );
    }

    #[test]
    fn a_gap_in_a_row_is_refused_with_where_it_is() {
        let mut counts_sheet = painted(1, &[1, 0, 0, 0, 0, 0, 0]);
        // Paint walk's third cell with the first two empty.
        let mut img = decode(&counts_sheet).unwrap();
        let (x, y) = (2 * CELL + 10, 3 * CELL + 10);
        let i = ((y * img.w + x) * 4) as usize;
        img.px[i + 3] = 255;
        counts_sheet = encode(&img).unwrap();
        let e = finish(JSON, &counts_sheet).unwrap_err();
        assert!(
            e.contains("walk") && e.contains("cell 3") && e.contains("left to right"),
            "{e}"
        );
    }

    #[test]
    fn no_idle_is_refused_and_a_wrong_size_says_the_right_one() {
        let e = finish(JSON, &painted(1, &[0, 1, 0, 0, 0, 0, 0])).unwrap_err();
        assert!(e.contains("idle"), "{e}");
        let blank = |w: u32, h: u32| {
            encode(&Image {
                w,
                h,
                px: vec![0; (w * h * 4) as usize],
            })
            .unwrap()
        };
        // Not a template's height; a width that is not whole cells; 33 cells.
        for (w, h) in [(100, 100), (2048, 450), (2000, 448), (33 * 64, 448)] {
            let e = finish(JSON, &blank(w, h)).unwrap_err();
            assert!(
                e.contains(&format!("{w}x{h}")) && e.contains("2048x448"),
                "{e}"
            );
        }
    }

    #[test]
    fn a_32_wide_template_holds_a_20_pose_dance_and_the_pack_is_as_wide_as_it() {
        for scale in [1, 2] {
            let files = finish(JSON, &painted_on(scale, 32, &[8, 2, 20, 4, 0, 1, 2])).unwrap();
            let (one, two) = (decode(&files[1].1).unwrap(), decode(&files[2].1).unwrap());
            assert_eq!((one.w, one.h), (20 * 64, 448), "scale {scale}");
            assert_eq!((two.w, two.h), (20 * 128, 896), "scale {scale}");
            let m = manifest_of(&files);
            assert_eq!(
                m["states"]["dance"]["frames"],
                serde_json::json!((40..60).collect::<Vec<u32>>())
            );
            assert_eq!(
                m["states"]["walk"]["frames"],
                serde_json::json!([60, 61, 62, 63])
            );
            let idle = m["states"]["idle"]["frames"].as_array().unwrap();
            assert_eq!(
                idle.len(),
                7 * 12,
                "seven moments, each after the pose held"
            );
            assert_eq!((idle[0].as_u64(), idle[83].as_u64()), (Some(0), Some(7)));
            // The 20th dance pose's pixel, at the bottom of row 2, column 19.
            let at = |x: u32, y: u32| one.px[((y * one.w + x) * 4 + 1) as usize];
            assert_eq!(at(19 * 64 + 32, 3 * 64 - 1), 100 + 19);
        }
    }

    #[test]
    fn the_same_painting_on_any_width_of_template_is_the_same_pack() {
        // Eight or fewer a row: the 32-wide template, the 8-wide one from
        // before D183 and a 5-wide cut-down all come out as the 8-wide always
        // did, byte for byte.
        let counts = [3, 2, 8, 4, 2, 1, 2];
        for scale in [1, 2] {
            let old = finish(JSON, &painted_on(scale, 8, &counts)).unwrap();
            let wide = finish(JSON, &painted_on(scale, 32, &counts)).unwrap();
            assert!(wide == old, "scale {scale}: 32 wide is not the 8-wide pack");
        }
        let narrow = finish(JSON, &painted_on(1, 5, &[3, 2, 5, 4, 2, 1, 2])).unwrap();
        let one = decode(&narrow[1].1).unwrap();
        assert_eq!((one.w, one.h), (512, 448), "widened to eight cells");
    }
}
