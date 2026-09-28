//! Where he can stand. Pure: the windows from the pipe's layout (#181, D148),
//! the monitors' work areas and his drawn size go in; ledges come out.
//! Physical pixels throughout, as the pipe reports them.
//!
//! A ledge is a stretch of window top edge where his feet can be: his whole
//! sprite on a monitor's work area and clear of the player's other windows, so
//! a top with a window bonded onto it is not one (D154). Touching stretches at
//! the same height join, which is how he walks across the seam between two
//! windows bonded side by side. Each work area's bottom, above the taskbar, is
//! a floor: a ledge that is always there.

use hp_control::{LayoutInfo, WindowRect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// His drawn size, and the contact point measured from the sprite's top-left:
/// `ax` across to his feet, `ay` down to the row just below them. Both already
/// scaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Body {
    pub w: i32,
    pub h: i32,
    pub ax: i32,
    pub ay: i32,
}

/// Where his feet can be: `y` is the edge, `x0..x1` (half-open) the feet's x.
#[derive(Debug, Clone, PartialEq)]
pub struct Ledge {
    pub y: i32,
    pub x0: i32,
    pub x1: i32,
    /// The windows whose top edges make it; empty for a floor.
    pub on: Vec<String>,
}

impl Ledge {
    pub fn holds(&self, x: i32) -> bool {
        x >= self.x0 && x < self.x1
    }

    pub fn is_floor(&self) -> bool {
        self.on.is_empty()
    }
}

/// The classic windows' width at 1x. Main is twice that with 2x chrome (D76),
/// and he doubles with it, so he is always drawn at the player's own zoom.
pub const MAIN_W_1X: i32 = 275;

pub fn zoom(layout: &LayoutInfo) -> u32 {
    layout
        .windows
        .iter()
        .find(|w| w.id == "main")
        .map(|m| ((m.w + MAIN_W_1X / 2) / MAIN_W_1X).max(1) as u32)
        .unwrap_or(1)
}

/// The windows he would rather start on, in order.
pub const PREFERENCE: [&str; 6] = ["main", "eq", "playlist", "library", "video", "prep"];

pub fn shown(layout: &LayoutInfo) -> impl Iterator<Item = &WindowRect> {
    layout
        .windows
        .iter()
        .filter(|w| w.visible && w.w > 0 && w.h > 0)
}

/// Every ledge, window tops first by height, then the floors.
pub fn ledges(layout: &LayoutInfo, work: &[Rect], b: Body) -> Vec<Ledge> {
    let windows: Vec<&WindowRect> = shown(layout).collect();
    let mut pieces: Vec<Ledge> = Vec::new();
    for w in &windows {
        let y = w.y;
        // Feet x where the sprite sits on some work area, at this height.
        let mut ok: Vec<(i32, i32)> = work
            .iter()
            .filter(|a| y - b.ay >= a.y && y - b.ay + b.h <= a.y + a.h)
            .map(|a| (a.x + b.ax, a.x + a.w - b.w + b.ax + 1))
            .map(|(x0, x1)| (x0.max(w.x), x1.min(w.x + w.w)))
            .filter(|(x0, x1)| x0 < x1)
            .collect();
        // Less wherever the sprite would overlap another of the player's windows.
        for o in windows.iter().filter(|o| o.id != w.id) {
            let (top, bottom) = (y - b.ay, y - b.ay + b.h);
            if o.y < bottom && top < o.y + o.h {
                ok = cut(ok, o.x - b.w + b.ax + 1, o.x + o.w + b.ax);
            }
        }
        pieces.extend(ok.into_iter().map(|(x0, x1)| Ledge {
            y,
            x0,
            x1,
            on: vec![w.id.clone()],
        }));
    }
    pieces.sort_by_key(|l| (l.y, l.x0));
    let mut out: Vec<Ledge> = Vec::new();
    for p in pieces {
        match out.last_mut() {
            Some(l) if l.y == p.y && p.x0 <= l.x1 => {
                l.x1 = l.x1.max(p.x1);
                for id in p.on {
                    if !l.on.contains(&id) {
                        l.on.push(id);
                    }
                }
            }
            _ => out.push(p),
        }
    }
    for a in work {
        let (x0, x1) = (a.x + b.ax, (a.x + a.w - b.w + b.ax + 1).max(a.x + b.ax + 1));
        out.push(Ledge {
            y: a.y + a.h,
            x0,
            x1,
            on: vec![],
        });
    }
    out
}

/// Remove the half-open `x0..x1` from each interval.
fn cut(ivs: Vec<(i32, i32)>, x0: i32, x1: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for (a, b) in ivs {
        if x1 <= a || b <= x0 {
            out.push((a, b));
            continue;
        }
        if a < x0 {
            out.push((a, x0));
        }
        if x1 < b {
            out.push((x1, b));
        }
    }
    out
}

/// Where he appears: on the first window in `PREFERENCE` that has a ledge, 30%
/// along it (not a hood ornament), else on the floor under Main. `None` when
/// none of the player's windows is showing: he goes with the player.
pub fn spawn(layout: &LayoutInfo, ledges: &[Ledge]) -> Option<(i32, i32)> {
    let windows: Vec<&WindowRect> = shown(layout).collect();
    if windows.is_empty() {
        return None;
    }
    let by_pref = PREFERENCE
        .iter()
        .filter_map(|id| windows.iter().find(|w| w.id == *id))
        .chain(windows.iter());
    for w in by_pref {
        if let Some(l) = ledges.iter().find(|l| l.on.contains(&w.id)) {
            let want = w.x + (w.w as f32 * 0.3).round() as i32;
            return Some((want.clamp(l.x0, l.x1 - 1), l.y));
        }
    }
    let main = windows
        .iter()
        .find(|w| w.id == "main")
        .unwrap_or(&windows[0]);
    let cx = main.x + main.w / 2;
    let floor = floor_under(ledges, cx)?;
    Some((cx.clamp(floor.x0, floor.x1 - 1), floor.y))
}

/// The floor whose monitor is under `x`, or the nearest one.
pub fn floor_under(ledges: &[Ledge], x: i32) -> Option<&Ledge> {
    let floors = ledges.iter().filter(|l| l.is_floor());
    floors
        .clone()
        .find(|l| l.holds(x))
        .or_else(|| floors.min_by_key(|l| (l.x0 - x).abs().min((l.x1 - 1 - x).abs())))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn win(id: &str, x: i32, y: i32, w: i32, h: i32) -> WindowRect {
        WindowRect {
            id: id.into(),
            x,
            y,
            w,
            h,
            group: false,
            shaded: false,
            visible: true,
        }
    }

    pub(crate) fn layout(ws: Vec<WindowRect>) -> LayoutInfo {
        LayoutInfo {
            windows: ws,
            bonds: vec![],
        }
    }

    /// The captain at 1x: a 64 px cell, feet at (32, 63).
    pub(crate) const B: Body = Body {
        w: 64,
        h: 64,
        ax: 32,
        ay: 64,
    };
    pub(crate) const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1032,
    };

    fn tops(ls: &[Ledge]) -> Vec<&Ledge> {
        ls.iter().filter(|l| !l.is_floor()).collect()
    }

    #[test]
    fn a_window_with_room_above_is_a_ledge_its_whole_width() {
        let ls = ledges(&layout(vec![win("main", 500, 400, 275, 116)]), &[SCREEN], B);
        assert_eq!(
            tops(&ls),
            vec![&Ledge {
                y: 400,
                x0: 500,
                x1: 775,
                on: vec!["main".into()]
            }]
        );
        assert_eq!(
            ls.iter().filter(|l| l.is_floor()).count(),
            1,
            "and the floor"
        );
    }

    #[test]
    fn he_appears_on_main_30_percent_along() {
        let l = layout(vec![win("main", 500, 400, 275, 116)]);
        let ls = ledges(&l, &[SCREEN], B);
        assert_eq!(spawn(&l, &ls), Some((583, 400)));
    }

    #[test]
    fn with_main_at_the_top_of_the_screen_he_appears_on_the_library() {
        // The owner's layout on 2026-09-27: the stack at the top of the
        // screen, each window's top under the one above, the library free.
        let l = layout(vec![
            win("main", 1970, 0, 275, 116),
            win("eq", 1970, 116, 275, 14),
            win("playlist", 1970, 130, 275, 14),
            win("library", 208, 208, 916, 659),
        ]);
        let work = [
            Rect {
                x: 0,
                y: 0,
                w: 2560,
                h: 1032,
            },
            Rect {
                x: 2560,
                y: 0,
                w: 2560,
                h: 1392,
            },
        ];
        let ls = ledges(&l, &work, B);
        let t = tops(&ls);
        assert_eq!(t.len(), 1, "only the library's top: {t:?}");
        assert_eq!(t[0].on, vec!["library".to_string()]);
        assert_eq!(spawn(&l, &ls), Some((483, 208)));
    }

    #[test]
    fn a_top_under_another_window_is_not_a_ledge() {
        let l = layout(vec![
            win("main", 600, 300, 275, 116),
            win("eq", 600, 416, 275, 116),
        ]);
        let ls = ledges(&l, &[SCREEN], B);
        assert!(tops(&ls).iter().all(|l| !l.on.contains(&"eq".to_string())));
    }

    #[test]
    fn a_window_beside_his_head_shortens_the_ledge() {
        // The library's top at y 400; Main stands taller just to its right, so
        // he cannot stand where his sprite would go into Main.
        let l = layout(vec![
            win("library", 100, 400, 600, 300),
            win("main", 700, 300, 275, 116),
        ]);
        let ls = ledges(&l, &[SCREEN], B);
        let lib = tops(&ls)
            .into_iter()
            .find(|l| l.on == vec!["library".to_string()])
            .unwrap();
        assert_eq!((lib.x0, lib.x1), (100, 700 - 64 + 32 + 1));
    }

    #[test]
    fn windows_side_by_side_at_one_height_make_one_ledge_across_the_seam() {
        let l = layout(vec![
            win("main", 500, 400, 275, 116),
            win("eq", 775, 400, 275, 116),
        ]);
        let ls = ledges(&l, &[SCREEN], B);
        let t = tops(&ls);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].x0, t[0].x1), (500, 1050));
        assert_eq!(t[0].on, vec!["main".to_string(), "eq".to_string()]);
    }

    #[test]
    fn with_no_room_on_any_window_he_appears_on_the_floor_under_main() {
        let l = layout(vec![win("main", 0, 0, 275, 116)]);
        let ls = ledges(&l, &[SCREEN], B);
        assert!(tops(&ls).is_empty());
        assert_eq!(spawn(&l, &ls), Some((137, 1032)));
    }

    #[test]
    fn nothing_showing_means_he_is_not_there() {
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        let l = layout(vec![main]);
        assert_eq!(spawn(&l, &ledges(&l, &[SCREEN], B)), None);
    }

    #[test]
    fn zoom_follows_mains_width() {
        assert_eq!(zoom(&layout(vec![win("main", 0, 0, 275, 116)])), 1);
        assert_eq!(zoom(&layout(vec![win("main", 0, 0, 550, 232)])), 2);
        assert_eq!(zoom(&layout(vec![])), 1);
    }
}
