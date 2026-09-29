//! Where the windows are, on the control pipe (#181, D148).
//!
//! A program outside the player (Cap'n Capy first, D147; an overlay; an LED
//! rig that wants to know where the screen's windows sit) asks `layout` once
//! and then hears `layout_changed`. Both carry the same thing: every window a
//! companion could stand on, in physical pixels, and the bond graph between
//! the classic three.
//!
//! **Visibility comes from the OS, not from this app's bookkeeping** (D58):
//! what a window reports is what is on screen. That means window getters,
//! which on Windows ask the UI thread and wait for it, so nothing here runs on
//! that thread or under the window manager's lock (D54, D55). The UI thread
//! only *pings*: a send on a channel that never blocks. A thread of this
//! module's own does the looking, at most every 50 ms, and always once more
//! after the last ping, so a drag streams at about 20 events a second and
//! still ends on the true final layout.

use crate::bond::{Layout, WindowGraph, WindowId};
use crate::control::Broadcaster;
use crate::wm::{self, Wm};
use hp_control::{BondRect, Event, LayoutInfo, WindowRect};
use std::collections::BTreeSet;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// The fastest a client hears a drag.
const EVERY: Duration = Duration::from_millis(50);

/// The decorated windows a companion may also stand on (#181, the owner's
/// call). Listed while they exist; never bonded.
const DECORATED: [&str; 5] = ["library", "video", "prep", "visuals", "settings"];

/// A decorated window as the OS reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Decorated {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
}

/// Put a layout into the pipe's words. Pure: the window manager's rectangles
/// and bonds, which classic windows are shaded, what the OS says is visible,
/// and the decorated windows the OS reported.
pub fn describe(
    layout: &Layout,
    graph: &WindowGraph,
    shaded: &BTreeSet<WindowId>,
    visible: &dyn Fn(WindowId) -> bool,
    decorated: &[Decorated],
) -> LayoutInfo {
    let mut windows: Vec<WindowRect> = wm::CLASSIC
        .iter()
        .filter_map(|id| {
            let r = layout.get(id)?;
            Some(WindowRect {
                id: wm::label_of(*id).into(),
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
                group: true,
                shaded: shaded.contains(id),
                visible: visible(*id),
            })
        })
        .collect();
    windows.extend(decorated.iter().map(|d| WindowRect {
        id: d.id.clone(),
        x: d.x,
        y: d.y,
        w: d.w,
        h: d.h,
        group: false,
        shaded: false,
        visible: d.visible,
    }));
    let bonds = graph
        .bonds
        .iter()
        .map(|b| BondRect {
            a: wm::label_of(b.a).into(),
            b: wm::label_of(b.b).into(),
            edge: wm::edge_name(b.edge).into(),
            span: [b.span.0, b.span.1],
        })
        .collect();
    LayoutInfo { windows, bonds }
}

/// Shown and not minimised, by the OS's account. A window that cannot be
/// asked is not on screen.
fn on_screen(app: &AppHandle, label: &str) -> bool {
    app.get_webview_window(label)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
}

/// The layout now. **Never call this on the UI thread**: the getters wait for
/// it (see the module doc).
pub fn current(app: &AppHandle) -> LayoutInfo {
    let (layout, graph, shaded) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        (s.layout.clone(), s.graph.clone(), s.shaded.clone())
    }; // The lock is let go before any window is asked anything (D54).
    let decorated: Vec<Decorated> = DECORATED
        .iter()
        .filter_map(|label| {
            let w = app.get_webview_window(label)?;
            let pos = w.outer_position().ok()?;
            let size = w.outer_size().ok()?;
            Some(Decorated {
                id: (*label).into(),
                x: pos.x,
                y: pos.y,
                w: size.width as i32,
                h: size.height as i32,
                visible: on_screen(app, label),
            })
        })
        .collect();
    describe(
        &layout,
        &graph,
        &shaded,
        &|id| on_screen(app, wm::label_of(id)),
        &decorated,
    )
}

/// The way in: something may have moved, shown, hidden, shaded or bonded.
pub struct Pinger(Sender<()>);

/// Say the layout may have changed. Cheap and never blocks, so it is safe
/// from the UI thread and from inside a window event handler.
pub fn ping(app: &AppHandle) {
    if let Some(p) = app.try_state::<Pinger>() {
        let _ = p.0.send(());
    }
}

/// Start the thread that turns pings into `layout_changed`. After the
/// broadcaster and the window manager are managed.
pub fn spawn(app: &AppHandle) {
    let (tx, rx) = channel();
    app.manage(Pinger(tx));
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("hp-layout".into())
        .spawn(move || run(app, rx));
    if let Err(e) = spawned {
        eprintln!("layout_changed will not be sent: {e}");
    }
}

fn run(app: AppHandle, rx: Receiver<()>) {
    let mut told: Option<LayoutInfo> = None;
    let mut sent_at: Option<Instant> = None;
    // Wait for a ping; then hold until 50 ms have passed since the last send,
    // taking every ping that arrives meanwhile; then look once and tell.
    while rx.recv().is_ok() {
        if let Some(due) = sent_at.map(|t| t + EVERY) {
            loop {
                let left = due.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                match rx.recv_timeout(left) {
                    Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        }
        while rx.try_recv().is_ok() {}
        sent_at = Some(Instant::now());
        if !app.state::<Broadcaster>().has_clients() {
            // Nobody is listening, so nothing is asked of the windows; the
            // next client asks `layout` for where things stand.
            told = None;
            continue;
        }
        let now = current(&app);
        if told.as_ref() != Some(&now) {
            app.state::<Broadcaster>()
                .send(&Event::LayoutChanged(now.clone()));
            told = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bond::{Bond, Edge, Rect};
    use crate::wm::{EQ, MAIN, PLAYLIST};

    #[test]
    fn the_classic_three_bond_and_a_decorated_window_stands_apart() {
        let mut layout = Layout::new();
        layout.insert(MAIN, Rect::new(420, 300, 550, 232));
        layout.insert(EQ, Rect::new(420, 532, 550, 232));
        layout.insert(PLAYLIST, Rect::new(420, 764, 550, 28));
        let mut graph = WindowGraph::default();
        graph
            .bonds
            .push(Bond::new(MAIN, EQ, Edge::Bottom, (420, 970)));
        // Stored canonically: eq's top is main's bottom, whichever way round
        // it was made.
        graph
            .bonds
            .push(Bond::new(PLAYLIST, EQ, Edge::Top, (420, 970)));
        let shaded: BTreeSet<WindowId> = [PLAYLIST].into_iter().collect();
        let library = Decorated {
            id: "library".into(),
            x: -1200,
            y: 80,
            w: 916,
            h: 659,
            visible: false,
        };

        let info = describe(&layout, &graph, &shaded, &|id| id != EQ, &[library]);
        let ids: Vec<(&str, bool, bool, bool)> = info
            .windows
            .iter()
            .map(|w| (w.id.as_str(), w.group, w.shaded, w.visible))
            .collect();
        assert_eq!(
            ids,
            [
                ("main", true, false, true),
                ("eq", true, false, false),
                ("playlist", true, true, true),
                ("library", false, false, false),
            ]
        );
        assert_eq!(info.windows[3].x, -1200, "physical, even off to the left");
        let bonds: Vec<(&str, &str, &str, [i32; 2])> = info
            .bonds
            .iter()
            .map(|b| (b.a.as_str(), b.b.as_str(), b.edge.as_str(), b.span))
            .collect();
        assert_eq!(
            bonds,
            [
                ("main", "eq", "bottom", [420, 970]),
                ("eq", "playlist", "bottom", [420, 970]),
            ]
        );
    }

    #[test]
    fn a_window_the_manager_has_not_placed_is_left_out() {
        let mut layout = Layout::new();
        layout.insert(MAIN, Rect::new(0, 0, 275, 116));
        let info = describe(
            &layout,
            &WindowGraph::default(),
            &BTreeSet::new(),
            &|_| true,
            &[],
        );
        assert_eq!(info.windows.len(), 1);
        assert!(info.bonds.is_empty());
    }
}
