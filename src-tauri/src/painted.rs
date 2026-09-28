//! A companion painted on the template (#208, D163): the sheet a person
//! painted, turned into a pack. The template's `companion.json` says
//! `"painted": true`; the rest is read off the sheet itself:
//!
//! - one row per state, in the format's order, eight cells a row;
//! - a cell counts once anything is painted in it, and a row's frames are its
//!   painted cells from the left, so a gap is a mistake worth saying;
//! - idle's first cell is the one cell a companion cannot go without.
//!
//! Painted at 64 px a cell (true pixel art, as Aseprite draws it), the sheet
//! is the 1x and is doubled pixel for pixel for the `@2x` twin (D160), which
//! stays crisp. Painted at 128 px, it is the twin, and the 1x is its 2x2
//! average. The timings are the format's, the ones `tools/sheet.ps1` writes.

/// The format's vocabulary, in the sheet's row order (`purricane.md`).
pub const STATES: [&str; 7] = ["idle", "sleep", "dance", "walk", "startle", "pet", "carry"];
pub const COLUMNS: u32 = 8;
pub const CELL: u32 = 64;

/// Whether a manifest is a painted template waiting to be read.
pub fn is_painted(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .map(|v| v["painted"] == true)
        .unwrap_or(false)
}

/// Straight RGBA8, top-down.
struct Image {
    w: u32,
    h: u32,
    px: Vec<u8>,
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
    let (one, two) = if (sheet.w, sheet.h) == (COLUMNS * CELL, rows * CELL) {
        let two = double(&sheet);
        (sheet, two)
    } else if (sheet.w, sheet.h) == (2 * COLUMNS * CELL, 2 * rows * CELL) {
        let one = halve(&sheet);
        (one, sheet)
    } else {
        return Err(format!(
            "the painted sheet is {}x{}; the template's is {}x{} (64 px cells), or {}x{} painted at twice the size",
            sheet.w,
            sheet.h,
            COLUMNS * CELL,
            rows * CELL,
            2 * COLUMNS * CELL,
            2 * rows * CELL
        ));
    };
    let counts = count_cells(&one)?;
    let manifest = manifest(&name, &counts);
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
        let painted: Vec<bool> = (0..COLUMNS)
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

/// The manifest, with the format's timings. Idle's first frame is the pose
/// and each later one a moment in it, as `tools/sheet.ps1` writes it: the
/// pose held eleven frames, the moment once (a blink every 3 s at 4 fps).
fn manifest(name: &str, counts: &[u32]) -> String {
    let mut states = serde_json::Map::new();
    for (r, (state, &n)) in STATES.iter().zip(counts).enumerate() {
        if n == 0 {
            continue;
        }
        let first = r as u32 * COLUMNS;
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
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec
        .read_info()
        .map_err(|e| format!("the sheet is not a PNG: {e}"))?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w > 4096 || h > 4096 {
        return Err(format!("the sheet is {w}x{h}; too big to be a companion"));
    }
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("the sheet is too large")?
    ];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("the sheet would not decode: {e}"))?;
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
        png::ColorType::Indexed => return Err("the sheet's palette did not expand".into()),
    };
    Ok(Image { w, h, px })
}

fn encode(img: &Image) -> Result<Vec<u8>, String> {
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

    /// A template-sized sheet at `scale` (1 or 2), with the given number of
    /// painted cells per row: each painted cell has one opaque pixel.
    fn painted(scale: u32, counts: &[u32; 7]) -> Vec<u8> {
        let cell = CELL * scale;
        let mut img = Image {
            w: COLUMNS * cell,
            h: 7 * cell,
            px: vec![0; (COLUMNS * cell * 7 * cell * 4) as usize],
        };
        for (r, &n) in counts.iter().enumerate() {
            for c in 0..n {
                let (x, y) = (c * cell + cell / 2, r as u32 * cell + cell - 1);
                let i = ((y * img.w + x) * 4) as usize;
                img.px[i..i + 4].copy_from_slice(&[200, 100, 50, 255]);
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
        let odd = encode(&Image {
            w: 100,
            h: 100,
            px: vec![0; 40000],
        })
        .unwrap();
        let e = finish(JSON, &odd).unwrap_err();
        assert!(e.contains("512x448"), "{e}");
    }
}
