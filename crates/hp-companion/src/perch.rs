//! Where he stands. Pure: the windows from the pipe's layout (#181, D148), the
//! monitors' work areas, and his drawn size go in; a spot comes out. Physical
//! pixels throughout, as the pipe reports them.
//!
//! He stands on the top edge of a visible window, never in the air: his whole
//! sprite has to be on a monitor's work area and clear of the player's other
//! windows (so the top of a window with another bonded on top of it is not a
//! perch). He keeps the window he is on while it still works, and his place
//! along it, so he rides a window that moves instead of hopping to another.
//! When no window has room, as when Main sits at the top of the screen with
//! everything else under it, he stands on the floor: the bottom of the work
//! area, above the taskbar.

use hp_control::{LayoutInfo, WindowRect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn inside(&self, o: &Rect) -> bool {
        self.x >= o.x
            && self.y >= o.y
            && self.x + self.w <= o.x + o.w
            && self.y + self.h <= o.y + o.h
    }

    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

fn rect(w: &WindowRect) -> Rect {
    Rect {
        x: w.x,
        y: w.y,
        w: w.w,
        h: w.h,
    }
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

#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    /// The window he stands on; `None` is the floor.
    pub on: Option<String>,
    /// Where the sprite's top-left goes.
    pub x: i32,
    pub y: i32,
    /// How far along the window's top edge his feet are, 0 to 1.
    pub along: f32,
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

/// The windows he tries, in order, after the one he is already on.
const PREFERENCE: [&str; 6] = ["main", "eq", "playlist", "library", "video", "prep"];
/// The places along a top edge he tries, in order: off-centre first, so he is
/// not a hood ornament.
const ALONG: [f32; 5] = [0.3, 0.5, 0.7, 0.15, 0.85];

/// Where he goes now, or `None` when none of the player's windows is showing:
/// he goes with the player.
pub fn choose(layout: &LayoutInfo, work: &[Rect], body: Body, was: Option<&Spot>) -> Option<Spot> {
    let shown: Vec<&WindowRect> = layout
        .windows
        .iter()
        .filter(|w| w.visible && w.w > 0 && w.h > 0)
        .collect();
    if shown.is_empty() {
        return None;
    }
    // The window he is on, then the preference, then anything else showing.
    let wanted = was
        .and_then(|s| s.on.as_deref())
        .into_iter()
        .chain(PREFERENCE)
        .chain(shown.iter().map(|w| w.id.as_str()));
    let mut order: Vec<&WindowRect> = Vec::new();
    for id in wanted {
        if order.iter().any(|o| o.id == id) {
            continue;
        }
        if let Some(w) = shown.iter().find(|w| w.id == id) {
            order.push(w);
        }
    }

    for w in order {
        let kept = was
            .filter(|s| s.on.as_deref() == Some(w.id.as_str()))
            .map(|s| s.along);
        for along in kept.into_iter().chain(ALONG) {
            if let Some(spot) = stand_on(w, along, &shown, work, body) {
                return Some(spot);
            }
        }
    }
    Some(floor(&shown, work, body))
}

fn stand_on(
    w: &WindowRect,
    along: f32,
    shown: &[&WindowRect],
    work: &[Rect],
    b: Body,
) -> Option<Spot> {
    let feet = w.x + (w.w as f32 * along).round() as i32;
    let sprite = Rect {
        x: feet - b.ax,
        y: w.y - b.ay,
        w: b.w,
        h: b.h,
    };
    let on_screen = work.iter().any(|m| sprite.inside(m));
    let clear = shown
        .iter()
        .all(|o| o.id == w.id || !sprite.overlaps(&rect(o)));
    (on_screen && clear).then(|| Spot {
        on: Some(w.id.clone()),
        x: sprite.x,
        y: sprite.y,
        along,
    })
}

/// The bottom of the work area under Main (or the first window showing),
/// centred on it, above the taskbar.
fn floor(shown: &[&WindowRect], work: &[Rect], b: Body) -> Spot {
    let anchor = shown.iter().find(|w| w.id == "main").unwrap_or(&shown[0]);
    let (cx, cy) = (anchor.x + anchor.w / 2, anchor.y + anchor.h / 2);
    let area = work
        .iter()
        .find(|m| cx >= m.x && cx < m.x + m.w && cy >= m.y && cy < m.y + m.h)
        .or(work.first())
        .copied()
        .unwrap_or(Rect {
            x: 0,
            y: 0,
            w: b.w,
            h: b.h,
        });
    let x = (cx - b.ax).clamp(area.x, (area.x + area.w - b.w).max(area.x));
    Spot {
        on: None,
        x,
        y: area.y + area.h - b.ay,
        along: 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: &str, x: i32, y: i32, w: i32, h: i32) -> WindowRect {
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

    fn layout(ws: Vec<WindowRect>) -> LayoutInfo {
        LayoutInfo {
            windows: ws,
            bonds: vec![],
        }
    }

    /// The captain at 1x: a 64 px cell, feet at (32, 63).
    const B: Body = Body {
        w: 64,
        h: 64,
        ax: 32,
        ay: 64,
    };
    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1032,
    };

    #[test]
    fn he_stands_on_mains_top_edge_when_there_is_room() {
        let l = layout(vec![win("main", 500, 400, 275, 116)]);
        let s = choose(&l, &[SCREEN], B, None).unwrap();
        assert_eq!(s.on.as_deref(), Some("main"));
        assert_eq!(s.y + B.ay, 400, "the row under his feet is Main's top edge");
        assert_eq!(s.x + B.ax, 500 + 83, "30% along");
    }

    #[test]
    fn with_main_at_the_top_of_the_screen_he_finds_the_library() {
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
        let s = choose(&l, &work, B, None).unwrap();
        assert_eq!(s.on.as_deref(), Some("library"));
        assert_eq!(s.y + B.ay, 208);
    }

    #[test]
    fn a_top_edge_with_a_window_on_it_is_not_a_perch() {
        // Main at the top of the screen, the EQ bonded under it: the EQ's top
        // edge is Main's bottom, and standing there he would be inside Main.
        let l = layout(vec![
            win("main", 600, 0, 275, 116),
            win("eq", 600, 116, 275, 116),
        ]);
        let s = choose(&l, &[SCREEN], B, None).unwrap();
        assert_eq!(s.on, None, "not on the EQ: the floor");
    }

    #[test]
    fn a_hidden_window_is_never_a_perch_and_nothing_shown_means_gone() {
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        assert!(choose(&layout(vec![main.clone()]), &[SCREEN], B, None).is_none());
        let s = choose(
            &layout(vec![main, win("library", 100, 500, 600, 300)]),
            &[SCREEN],
            B,
            None,
        );
        assert_eq!(s.unwrap().on.as_deref(), Some("library"));
    }

    #[test]
    fn with_no_room_anywhere_he_stands_on_the_floor() {
        let l = layout(vec![win("main", 0, 0, 275, 116)]);
        let s = choose(&l, &[SCREEN], B, None).unwrap();
        assert_eq!(s.on, None);
        assert_eq!(
            s.y + B.ay,
            1032,
            "feet on the work area's bottom, above the taskbar"
        );
        assert_eq!(s.x + B.ax, 137, "under Main's centre");
    }

    #[test]
    fn he_keeps_his_window_and_his_place_along_it() {
        let l = layout(vec![
            win("main", 500, 400, 275, 116),
            win("library", 100, 700, 600, 300),
        ]);
        let was = Spot {
            on: Some("library".into()),
            x: 0,
            y: 0,
            along: 0.62,
        };
        let s = choose(&l, &[SCREEN], B, Some(&was)).unwrap();
        assert_eq!(
            s.on.as_deref(),
            Some("library"),
            "Main has room, but he stays put"
        );
        assert_eq!(s.x + B.ax, 100 + 372, "at the same 62%");
        // The library moves: he rides along.
        let moved = layout(vec![
            win("main", 500, 400, 275, 116),
            win("library", 300, 650, 600, 300),
        ]);
        let s2 = choose(&moved, &[SCREEN], B, Some(&s)).unwrap();
        assert_eq!((s2.x - s.x, s2.y - s.y), (200, -50));
    }

    #[test]
    fn zoom_follows_mains_width() {
        assert_eq!(zoom(&layout(vec![win("main", 0, 0, 275, 116)])), 1);
        assert_eq!(zoom(&layout(vec![win("main", 0, 0, 550, 232)])), 2);
        assert_eq!(zoom(&layout(vec![])), 1);
    }
}
