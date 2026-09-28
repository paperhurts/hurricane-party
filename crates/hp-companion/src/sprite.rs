//! A cell made ready for the screen: scaled by a whole number, so pixel art
//! stays pixel art (the same rule as the player's chrome, D76: no fractional
//! scaling), mirrored when he faces left, and premultiplied BGRA, which is the
//! one layout a per-pixel-alpha layered window takes.

use crate::pack::Rgba;

/// Premultiplied BGRA, top-down.
#[derive(Debug, Clone, PartialEq)]
pub struct Bgra {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

/// Nearest-neighbour by `scale` (1 or more), mirrored if `flip`, premultiplied.
pub fn render(cell: &Rgba, scale: u32, flip: bool) -> Bgra {
    let s = scale.max(1);
    let (w, h) = (cell.w * s, cell.h * s);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let sx = if flip { cell.w - 1 - x / s } else { x / s };
            let i = (((y / s) * cell.w + sx) * 4) as usize;
            let [r, g, b, a] = [cell.px[i], cell.px[i + 1], cell.px[i + 2], cell.px[i + 3]];
            px.extend_from_slice(&[premul(b, a), premul(g, a), premul(r, a), a]);
        }
    }
    Bgra { w, h, px }
}

/// `c * a / 255`, rounded, so a fully transparent pixel is all zeros and an
/// opaque one is unchanged.
fn premul(c: u8, a: u8) -> u8 {
    ((c as u32 * a as u32 + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell() -> Rgba {
        // 2x1: an opaque red pixel, then a half-transparent white one.
        Rgba {
            w: 2,
            h: 1,
            px: vec![255, 0, 0, 255, 255, 255, 255, 128],
        }
    }

    #[test]
    fn channels_come_out_bgra_and_premultiplied() {
        let b = render(&cell(), 1, false);
        assert_eq!(&b.px[0..4], &[0, 0, 255, 255], "red, opaque, as BGRA");
        assert_eq!(
            &b.px[4..8],
            &[128, 128, 128, 128],
            "white at half alpha is half bright"
        );
    }

    #[test]
    fn a_transparent_pixel_is_all_zeros() {
        let c = Rgba {
            w: 1,
            h: 1,
            px: vec![200, 100, 50, 0],
        };
        assert_eq!(render(&c, 1, false).px, vec![0, 0, 0, 0]);
    }

    #[test]
    fn scale_repeats_each_pixel_whole() {
        let b = render(&cell(), 2, false);
        assert_eq!((b.w, b.h), (4, 2));
        let row = |y: usize| &b.px[y * 16..y * 16 + 16];
        assert_eq!(row(0), row(1), "both rows the same");
        assert_eq!(&row(0)[0..4], &row(0)[4..8], "the red pixel twice");
    }

    #[test]
    fn flip_mirrors_left_to_right() {
        let b = render(&cell(), 1, true);
        assert_eq!(&b.px[0..4], &[128, 128, 128, 128]);
        assert_eq!(&b.px[4..8], &[0, 0, 255, 255]);
    }
}
