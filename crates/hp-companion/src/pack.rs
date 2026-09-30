//! An `hp-companion/1` pack: a sprite sheet plus the manifest that says which
//! cells belong to which state (`docs/purricane.md`, "Companions are
//! skinnable"). Art comes in, code does not: the seven states are fixed here,
//! and a pack only supplies pictures and a few numbers for them.
//!
//! Packs are untrusted input, so one loads whole or not at all. The sheet's
//! size is checked before a pixel is decoded, a missing `idle` is a hard
//! failure, a missing optional state falls back to `idle`, and unknown keys
//! are ignored so an `hp-companion/2` pack degrades instead of failing.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const FORMAT: &str = "hp-companion/1";

/// The fixed vocabulary, in the sheet's row order. A pack cannot add to it.
pub const STATES: [&str; 7] = ["idle", "sleep", "dance", "walk", "startle", "pet", "carry"];

/// A sheet bigger than this is refused before it is decoded: a 16k x 16k PNG
/// is a denial of service dressed as a unicorn (`purricane.md`).
pub const MAX_SHEET_SIDE: u32 = 4096;
pub const MAX_FRAME_SIDE: u32 = 256;
pub const MAX_FRAMES_PER_STATE: usize = 64;

/// One cell, straight (not premultiplied) RGBA, top-down.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub frames: Vec<u32>,
    /// Frames per second; a `syncTo` state advances on its cue instead.
    #[serde(default)]
    pub fps: Option<f32>,
    #[serde(default = "yes", rename = "loop")]
    pub looping: bool,
    /// The state a non-looping one hands over to when it ends.
    #[serde(default)]
    pub then: Option<String>,
    /// `"beat"`: advance one frame per beat flag on the viz stream.
    #[serde(default)]
    pub sync_to: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    name: String,
    sprite: String,
    frame_size: [u32; 2],
    anchor: [u32; 2],
    states: BTreeMap<String, State>,
    #[serde(default = "default_walk")]
    walk_px_per_sec: f32,
    #[serde(default = "default_count")]
    default_count: u32,
}

fn default_walk() -> f32 {
    24.0
}

fn default_count() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    pub name: String,
    /// The cell, in sheet pixels.
    pub frame: (u32, u32),
    /// The contact point at the feet, in cell pixels: what stands on a window's edge.
    pub anchor: (u32, u32),
    pub walk_px_per_sec: f32,
    pub default_count: u32,
    states: BTreeMap<String, State>,
    /// Every cell some state uses, by its index on the sheet.
    cells: BTreeMap<u32, Rgba>,
    /// The same cells at twice the size, from `<sheet>@2x.png` when the pack
    /// has one (D160), for drawing beside 2x chrome from real detail.
    cells2x: Option<BTreeMap<u32, Rgba>>,
}

impl Pack {
    /// Load a pack folder: `companion.json`, the sheet it names beside it,
    /// and that sheet's `@2x` twin if there is one (`sheet.png` and
    /// `sheet@2x.png`, the convention the skins' chrome already uses, D76).
    pub fn load(dir: &Path) -> Result<Pack, String> {
        let json = std::fs::read_to_string(dir.join("companion.json"))
            .map_err(|e| format!("{}: no companion.json ({e})", dir.display()))?;
        let sprite = sprite_name(&json)?;
        let sheet = std::fs::read(dir.join(&sprite))
            .map_err(|e| format!("{}: the sheet {sprite} would not open ({e})", dir.display()))?;
        let stem = &sprite[..sprite.len() - ".png".len()];
        let twin = dir.join(format!("{stem}@2x.png"));
        let sheet2x = if twin.is_file() {
            Some(std::fs::read(&twin).map_err(|e| format!("{}: {e}", twin.display()))?)
        } else {
            None
        };
        Pack::parse_with_2x(&json, &sheet, sheet2x.as_deref())
    }

    /// Check a manifest against its sheet and cut the cells out. Pure, for tests.
    #[cfg(test)]
    pub fn parse(json: &str, sheet_png: &[u8]) -> Result<Pack, String> {
        Pack::parse_with_2x(json, sheet_png, None)
    }

    /// As `parse`, with the sheet's `@2x` twin when there is one. The twin must
    /// be exactly twice the sheet, or the pack is refused, not half-loaded.
    pub fn parse_with_2x(
        json: &str,
        sheet_png: &[u8],
        sheet2x_png: Option<&[u8]>,
    ) -> Result<Pack, String> {
        let m: Manifest = serde_json::from_str(json).map_err(|e| format!("companion.json: {e}"))?;
        if m.format != FORMAT {
            return Err(format!("not an {FORMAT} pack (format is {:?})", m.format));
        }
        plain_png(&m.sprite)?;
        let [fw, fh] = m.frame_size;
        if fw == 0 || fh == 0 || fw > MAX_FRAME_SIDE || fh > MAX_FRAME_SIDE {
            return Err(format!(
                "frameSize {fw}x{fh} is outside 1..={MAX_FRAME_SIDE}"
            ));
        }
        let [ax, ay] = m.anchor;
        if ax >= fw || ay >= fh {
            return Err(format!("anchor {ax},{ay} is outside the {fw}x{fh} cell"));
        }
        if !(m.walk_px_per_sec.is_finite() && m.walk_px_per_sec >= 0.0) {
            return Err("walkPxPerSec must be a number, zero or more".into());
        }

        // Unknown states are ignored, like unknown keys: a pack cannot add a
        // behaviour, and a newer format's extra state is not an error here.
        let mut states = BTreeMap::new();
        for (name, s) in m.states {
            if !STATES.contains(&name.as_str()) || s.frames.is_empty() {
                continue;
            }
            check_state(&name, &s)?;
            states.insert(name, s);
        }
        if !states.contains_key("idle") {
            return Err("no idle frames: idle is the one state a pack cannot go without".into());
        }

        let sheet = decode(sheet_png, MAX_SHEET_SIDE)?;
        let cols = sheet.w / fw;
        let rows = sheet.h / fh;
        let mut cells = BTreeMap::new();
        for s in states.values() {
            for &i in &s.frames {
                if i >= cols * rows {
                    return Err(format!(
                        "frame {i} is off the sheet ({cols} x {rows} cells of {fw}x{fh})"
                    ));
                }
                cells
                    .entry(i)
                    .or_insert_with(|| cut(&sheet, (i % cols) * fw, (i / cols) * fh, fw, fh));
            }
        }
        let cells2x = match sheet2x_png {
            None => None,
            Some(bytes) => {
                let big = decode(bytes, 2 * MAX_SHEET_SIDE)?;
                if (big.w, big.h) != (2 * sheet.w, 2 * sheet.h) {
                    return Err(format!(
                        "the @2x sheet is {}x{}; it must be twice the sheet, {}x{}",
                        big.w,
                        big.h,
                        2 * sheet.w,
                        2 * sheet.h
                    ));
                }
                let (w2, h2) = (2 * fw, 2 * fh);
                let cut2 = |i: u32| cut(&big, (i % cols) * w2, (i / cols) * h2, w2, h2);
                Some(cells.keys().map(|&i| (i, cut2(i))).collect())
            }
        };
        Ok(Pack {
            name: m.name,
            frame: (fw, fh),
            anchor: (ax, ay),
            walk_px_per_sec: m.walk_px_per_sec,
            default_count: m.default_count.max(1),
            states,
            cells,
            cells2x,
        })
    }

    /// A state by name, or `idle` when the pack has no frames for it.
    pub fn state(&self, name: &str) -> &State {
        self.states.get(name).unwrap_or(&self.states["idle"])
    }

    /// A cell by its sheet index. Every index a state names was cut at load.
    #[cfg(test)]
    pub fn cell(&self, index: u32) -> &Rgba {
        &self.cells[&index]
    }

    /// The cell to draw at a whole-number `scale`, and the whole number to
    /// scale it by: at an even scale, the `@2x` cell at half (drawn from real
    /// detail), and otherwise the 1x cell as it is scaled (D160).
    pub fn cell_for(&self, index: u32, scale: u32) -> (&Rgba, u32) {
        let scale = scale.max(1);
        match &self.cells2x {
            Some(big) if scale.is_multiple_of(2) => (&big[&index], scale / 2),
            _ => (&self.cells[&index], scale),
        }
    }
}

/// The sheet's file name, read before anything else so a manifest cannot
/// point outside its own folder.
fn sprite_name(json: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Just {
        sprite: String,
    }
    let s = serde_json::from_str::<Just>(json)
        .map_err(|e| format!("companion.json: {e}"))?
        .sprite;
    plain_png(&s)?;
    Ok(s)
}

/// A file name beside `companion.json`: no folder, no drive, no `..`.
fn plain_png(s: &str) -> Result<(), String> {
    let plain = !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains(['/', '\\', ':'])
        && s.to_ascii_lowercase().ends_with(".png");
    if plain {
        Ok(())
    } else {
        Err(format!("sprite {s:?} must be a .png beside companion.json"))
    }
}

fn check_state(name: &str, s: &State) -> Result<(), String> {
    if s.frames.len() > MAX_FRAMES_PER_STATE {
        return Err(format!("{name}: more than {MAX_FRAMES_PER_STATE} frames"));
    }
    if let Some(fps) = s.fps {
        if !(fps > 0.0 && fps <= 60.0) {
            return Err(format!("{name}: fps {fps} is outside (0, 60]"));
        }
    }
    match s.sync_to.as_deref() {
        None | Some("beat") => {}
        Some(other) => return Err(format!("{name}: syncTo {other:?} is not \"beat\"")),
    }
    if let Some(then) = &s.then {
        if !STATES.contains(&then.as_str()) {
            return Err(format!("{name}: then {then:?} is not a state"));
        }
    }
    if s.fps.is_none() && s.sync_to.is_none() {
        return Err(format!("{name}: needs fps or syncTo"));
    }
    Ok(())
}

/// Decode a PNG to straight RGBA8, refusing one over `max` a side before it is read.
fn decode(bytes: &[u8], max: u32) -> Result<Rgba, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec
        .read_info()
        .map_err(|e| format!("the sheet is not a PNG: {e}"))?;
    let (w, h) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if w > max || h > max {
        return Err(format!("the sheet is {w}x{h}; the limit is {max} a side"));
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
    Ok(Rgba { w, h, px })
}

fn cut(sheet: &Rgba, x: u32, y: u32, w: u32, h: u32) -> Rgba {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * sheet.w + x) * 4) as usize;
        px.extend_from_slice(&sheet.px[start..start + (w * 4) as usize]);
    }
    Rgba { w, h, px }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A sheet of `cols` x `rows` cells of `side` px, each cell filled with its
    /// own index in the red channel, fully opaque.
    pub(crate) fn sheet(cols: u32, rows: u32, side: u32) -> Vec<u8> {
        let (w, h) = (cols * side, rows * side);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let i = (y / side) * cols + x / side;
                px.extend_from_slice(&[i as u8, 0, 0, 255]);
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&px).unwrap();
        }
        out
    }

    fn manifest(states: &str) -> String {
        format!(
            r#"{{"format":"hp-companion/1","name":"T","sprite":"sheet.png",
                "frameSize":[4,4],"anchor":[2,3],"states":{{{states}}}}}"#
        )
    }

    const IDLE: &str = r#""idle":{"frames":[0,1],"fps":4,"loop":true}"#;

    #[test]
    fn the_captains_own_pack_loads() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skins/companions/captain");
        let p = Pack::load(&dir).expect("the shipped pack loads");
        assert_eq!(p.name, "Cap'n Capy");
        assert_eq!(p.frame, (64, 64));
        assert_eq!(p.anchor, (32, 63), "feet on the cell's bottom edge");
        assert_eq!(p.default_count, 1, "one captain, not a flock (#192)");
        assert_eq!(p.state("dance").sync_to.as_deref(), Some("beat"));
        let idle = p.cell(p.state("idle").frames[0]);
        assert_eq!((idle.w, idle.h), (64, 64));
        assert_eq!(
            idle.px[3], 0,
            "the corner is transparent: the frames were keyed"
        );
        let (big, by) = p.cell_for(p.state("idle").frames[0], 2);
        assert_eq!(
            (big.w, by),
            (128, 1),
            "beside 2x chrome, drawn from sheet@2x.png"
        );
    }

    #[test]
    fn wee_mans_own_pack_loads() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skins/companions/wee-man");
        let p = Pack::load(&dir).expect("the shipped pack loads");
        assert_eq!(p.name, "Wee Man");
        assert_eq!((p.frame, p.anchor), ((64, 64), (32, 63)));
        assert_eq!(p.default_count, 1, "one kitten with a name");
        // His grooming row became idle's moments: the pose, then a blink, a
        // lick and a look up, each after the pose held eleven frames (D164).
        let idle = &p.state("idle").frames;
        assert_eq!(idle.len(), 36);
        assert_eq!((idle[11], idle[23], idle[35]), (1, 2, 3));
        assert_eq!(
            p.state("startle").frames,
            [32, 33],
            "the leap, then landing puffed"
        );
        assert_eq!(p.cell_for(p.state("walk").frames[0], 2).0.w, 128);
    }

    #[test]
    fn senor_bones_own_pack_loads() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skins/companions/senor-bones");
        let p = Pack::load(&dir).expect("the shipped pack loads");
        assert_eq!(p.name, "Señor Bones");
        assert_eq!((p.frame, p.anchor), ((64, 64), (32, 63)));
        assert_eq!(p.default_count, 1);
        // His first row became idle's moments: the pose, then a blink, a
        // strum and two verses sung, each after the pose held (D176).
        let idle = &p.state("idle").frames;
        assert_eq!(
            (idle[0], idle[11], idle[23], idle[35], idle[47]),
            (0, 1, 2, 3, 4)
        );
        assert_eq!(p.state("walk").frames.len(), 7);
        assert_eq!(p.state("dance").frames.len(), 6);
        assert_eq!(
            p.state("startle").frames,
            [32, 33],
            "the jump, then landing dizzy"
        );
        // The sheet has no scruff pose: held up, he sits with his legs out.
        assert_eq!(p.state("carry").frames, [48]);
        assert_eq!(p.cell_for(p.state("walk").frames[0], 2).0.w, 128);
    }

    #[test]
    fn an_even_scale_draws_the_2x_cell_and_an_odd_one_the_1x() {
        let p =
            Pack::parse_with_2x(&manifest(IDLE), &sheet(8, 1, 4), Some(&sheet(8, 1, 8))).unwrap();
        let w = |scale| {
            let (c, by) = p.cell_for(0, scale);
            (c.w, by)
        };
        assert_eq!(w(1), (4, 1));
        assert_eq!(w(2), (8, 1));
        assert_eq!(w(3), (4, 3), "an odd scale has no 2x cell to halve");
        assert_eq!(w(4), (8, 2));
        assert_eq!(p.cell_for(1, 2).0.px[0], 1, "the 2x twin of the same cell");
    }

    #[test]
    fn without_a_2x_sheet_the_1x_cell_is_doubled() {
        let p = Pack::parse(&manifest(IDLE), &sheet(8, 1, 4)).unwrap();
        let (c, by) = p.cell_for(0, 2);
        assert_eq!((c.w, by), (4, 2));
    }

    #[test]
    fn a_2x_sheet_that_is_not_twice_the_sheet_refuses_the_pack() {
        let e = Pack::parse_with_2x(&manifest(IDLE), &sheet(8, 1, 4), Some(&sheet(8, 1, 6)));
        assert!(e.unwrap_err().contains("twice the sheet"));
    }

    #[test]
    fn cells_are_cut_by_index_row_by_row() {
        let p = Pack::parse(&manifest(IDLE), &sheet(8, 2, 4)).unwrap();
        assert_eq!(p.cell(0).px[0], 0);
        assert_eq!(p.cell(1).px[0], 1);
        let p = Pack::parse(
            &manifest(&format!(r#"{IDLE},"sleep":{{"frames":[8,9],"fps":1}}"#)),
            &sheet(8, 2, 4),
        )
        .unwrap();
        assert_eq!(p.cell(9).px[0], 9, "index 9 is row 1, column 1");
    }

    #[test]
    fn a_missing_state_falls_back_to_idle() {
        let p = Pack::parse(&manifest(IDLE), &sheet(8, 1, 4)).unwrap();
        assert_eq!(p.state("walk"), p.state("idle"));
    }

    #[test]
    fn no_idle_is_a_hard_failure() {
        let e = Pack::parse(
            &manifest(r#""walk":{"frames":[0],"fps":8}"#),
            &sheet(8, 1, 4),
        );
        assert!(e.unwrap_err().contains("idle"));
    }

    #[test]
    fn unknown_keys_and_states_are_ignored() {
        let json = manifest(&format!(r#"{IDLE},"fly":{{"frames":[0],"fps":2}}"#))
            .replace(r#""name":"T""#, r#""name":"T","hat":"tall""#);
        let p = Pack::parse(&json, &sheet(8, 1, 4)).unwrap();
        assert_eq!(
            p.state("fly"),
            p.state("idle"),
            "a pack cannot add a behaviour"
        );
    }

    #[test]
    fn a_frame_off_the_sheet_refuses_the_whole_pack() {
        let e = Pack::parse(
            &manifest(r#""idle":{"frames":[0,99],"fps":4}"#),
            &sheet(8, 1, 4),
        );
        assert!(e.unwrap_err().contains("off the sheet"));
    }

    #[test]
    fn bad_values_are_refused() {
        for (states, why) in [
            (r#""idle":{"frames":[0],"fps":0}"#, "fps"),
            (r#""idle":{"frames":[0],"fps":4,"then":"explode"}"#, "then"),
            (r#""idle":{"frames":[0],"syncTo":"bar"}"#, "syncTo"),
            (r#""idle":{"frames":[0]}"#, "fps or syncTo"),
        ] {
            let e = Pack::parse(&manifest(states), &sheet(8, 1, 4)).unwrap_err();
            assert!(e.contains(why), "{states} -> {e}");
        }
        let wrong = manifest(IDLE).replace("hp-companion/1", "hp-companion/9");
        assert!(Pack::parse(&wrong, &sheet(8, 1, 4)).is_err());
        let outside = manifest(IDLE).replace("[2,3]", "[2,4]");
        assert!(Pack::parse(&outside, &sheet(8, 1, 4))
            .unwrap_err()
            .contains("anchor"));
    }

    #[test]
    fn a_sprite_path_cannot_leave_the_pack() {
        for bad in ["../x.png", "..\\x.png", "C:x.png", "sub/x.png", "x.gif", ""] {
            let json = format!(r#"{{"sprite":{bad:?}}}"#);
            assert!(sprite_name(&json).is_err(), "{bad}");
        }
        assert_eq!(
            sprite_name(r#"{"sprite":"sheet.png"}"#).unwrap(),
            "sheet.png"
        );
    }

    #[test]
    fn an_oversized_sheet_is_refused_before_decoding() {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, MAX_SHEET_SIDE + 1, 1);
            enc.set_color(png::ColorType::Grayscale);
            enc.write_header()
                .unwrap()
                .write_image_data(&vec![0; (MAX_SHEET_SIDE + 1) as usize])
                .unwrap();
        }
        assert!(Pack::parse(&manifest(IDLE), &out)
            .unwrap_err()
            .contains("limit"));
    }
}
