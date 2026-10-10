//! The window manager for the three classic 275 px windows.
//!
//! `bond.rs` is the geometry and knows nothing about windows; this module is
//! what connects it to real HWNDs, real monitors, and the OS z-order. The split
//! is deliberate — everything here that can be pure is pure and tested, and the
//! only things that are not are the OS calls themselves.
//!
//! **Physical pixels throughout** (project convention), converted at the
//! boundaries via `bond::d40`. The logical constants below are the source of
//! truth and every physical number is recomputed from them, never carried
//! forward (D40).

use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

use crate::bond::{self, Bond, Edge, Layout, Px, Rect, WindowGraph, WindowId};
use crate::platform::{self, NativeWindow};

// ---- geometry ---------------------------------------------------------------

/// Logical chrome geometry, 1x. `windows.md`'s inventory. 2x mode (#47)
/// multiplies these before the single rounding to physical (D40); it never
/// scales a physical number. See `WmState::zoom` and `rezoom_layout`.
pub const CHROME_W: f64 = 275.0;
pub const CHROME_H: f64 = 116.0;
/// Windowshade collapses a window to a 275 x 14 bar.
pub const SHADE_H: f64 = 14.0;
/// #101, D88: how much of a title bar's width must stay inside a display's
/// work area for the window to count as reachable. A hand needs a grab.
pub const REACH_W: f64 = 40.0;

/// D30: every valid playlist size is `275 + 25n` by `116 + 29m`. Verified
/// against Webamp's source. The design prototype says `step:10, min:58`, which
/// is off-spec — the prototype is not the spec (CLAUDE.md).
pub const PLAYLIST_STEP_W: f64 = 25.0;
pub const PLAYLIST_STEP_H: f64 = 29.0;

/// Logical snap distance. Recomputed to physical per interaction, from the
/// monitor under the *cursor* (D51).
pub const SNAP_THRESHOLD: f64 = 10.0;

// ---- identity ---------------------------------------------------------------

pub const MAIN: WindowId = WindowId(0);
pub const EQ: WindowId = WindowId(1);
pub const PLAYLIST: WindowId = WindowId(2);

/// The bondable windows, in stacking order. The library, video, downloads,
/// prep and settings windows are ordinary decorated OS windows and are
/// deliberately not here (`windows.md` inventory, D13).
pub const CLASSIC: [WindowId; 3] = [MAIN, EQ, PLAYLIST];

/// D41: one never-shown owner per possible connected component. Three windows
/// can split into at most three groups, so three roots. A split is "point them
/// at a different root", never "promote a member".
pub const ROOT_LABELS: [&str; 3] = ["_root0", "_root1", "_root2"];

pub fn label_of(id: WindowId) -> &'static str {
    match id {
        MAIN => "main",
        EQ => "eq",
        PLAYLIST => "playlist",
        _ => unreachable!("not a classic window: {id:?}"),
    }
}

/// Label back to id. Returns `None` for the library, video and root windows —
/// the frontend sends its own label, and only the classic three are bondable.
pub fn id_of(label: &str) -> Option<WindowId> {
    CLASSIC.iter().copied().find(|id| label_of(*id) == label)
}

fn entry_of(id: WindowId) -> &'static str {
    match id {
        MAIN => "main.html",
        EQ => "eq.html",
        PLAYLIST => "playlist.html",
        _ => unreachable!("not a classic window: {id:?}"),
    }
}

/// Edge names as they cross IPC. Kept as strings rather than a serialised enum
/// so the frontend can name an edge without importing a Rust type.
pub fn edge_from_str(name: &str) -> Option<Edge> {
    match name {
        "top" => Some(Edge::Top),
        "right" => Some(Edge::Right),
        "bottom" => Some(Edge::Bottom),
        "left" => Some(Edge::Left),
        _ => None,
    }
}

/// D35 / D30: only the playlist can actually change size, so only seams that
/// touch it are live splitters. Everywhere else the seam is a move handle.
pub fn is_resizable(id: WindowId) -> bool {
    id == PLAYLIST
}

// ---- monitors ---------------------------------------------------------------

/// One display, in physical virtual-desktop coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorInfo {
    pub rect: Rect,
    pub scale: f64,
    /// The part of the display the taskbar leaves free. A title bar outside
    /// it cannot be grabbed (#101, D88).
    pub work: Rect,
}

/// D55: the topology is cached, never queried live.
///
/// `monitor_from_point()` from a synchronous command deadlocks, and the
/// topology changes about once a day — so it is read at startup and on
/// `WM_DISPLAYCHANGE`, and every interaction after that is a pure lookup.
pub fn monitor_at(monitors: &[MonitorInfo], x: Px, y: Px) -> Option<MonitorInfo> {
    monitors
        .iter()
        .find(|m| x >= m.rect.x && x < m.rect.right() && y >= m.rect.y && y < m.rect.bottom())
        .copied()
}

/// The scale factor to use for an interaction happening at `(x, y)`.
///
/// D51: *interaction* thresholds come from the monitor under the **cursor**,
/// not from the window being dragged. A 10 px magnet has to feel like 10 px
/// under the hand, whichever display the window itself is on.
pub fn scale_at(monitors: &[MonitorInfo], x: Px, y: Px, fallback: f64) -> f64 {
    monitor_at(monitors, x, y)
        .map(|m| m.scale)
        .unwrap_or(fallback)
}

/// D53: a shared monitor edge is **not** a screen edge.
///
/// Snapping to the inner boundary between two adjacent displays would drop an
/// invisible wall down the middle of a continuous desktop. So each side of the
/// starting monitor is pushed out to the far side of any neighbour that shares
/// it, repeatedly, until only the desktop's genuine outer edges remain.
///
/// A neighbour only counts if it **fully covers the group's extent** along the
/// seam. That qualifier is the whole difficulty. Taking the union of every
/// monitor instead claims screen that does not exist: on an L-shaped desktop
/// with a display below the left-hand one, a bounding box hands back the empty
/// quadrant under the right-hand display as somewhere a window may be snapped
/// to. And the coverage test has to be against the **group**, not against the
/// monitor, because the question being asked is whether this group can actually
/// slide across the seam — a neighbour too short to hold it does not hide the
/// edge, it just moves where the window falls off.
pub fn screen_rect_for(monitors: &[MonitorInfo], start: Rect, group: Rect) -> Rect {
    let rects: Vec<Rect> = monitors.iter().map(|m| m.rect).collect();
    merged_rect_for(&rects, start, group)
}

/// `screen_rect_for` over any one rect per display: the displays themselves,
/// or their work areas (D182).
fn merged_rect_for(rects: &[Rect], start: Rect, group: Rect) -> Rect {
    let mut r = start;
    // Each side walks independently. The guard bounds a loop that already
    // cannot cycle, since every step strictly grows `r` in one direction.
    let limit = rects.len() + 1;

    for _ in 0..limit {
        let Some(n) = rects
            .iter()
            .copied()
            .find(|n| n.x == r.right() && n.y <= group.y && n.bottom() >= group.bottom())
        else {
            break;
        };
        r.w = n.right() - r.x;
    }
    for _ in 0..limit {
        let Some(n) = rects
            .iter()
            .copied()
            .find(|n| n.right() == r.x && n.y <= group.y && n.bottom() >= group.bottom())
        else {
            break;
        };
        r.w = r.right() - n.x;
        r.x = n.x;
    }
    for _ in 0..limit {
        let Some(n) = rects
            .iter()
            .copied()
            .find(|n| n.y == r.bottom() && n.x <= group.x && n.right() >= group.right())
        else {
            break;
        };
        r.h = n.bottom() - r.y;
    }
    for _ in 0..limit {
        let Some(n) = rects
            .iter()
            .copied()
            .find(|n| n.bottom() == r.y && n.x <= group.x && n.right() >= group.right())
        else {
            break;
        };
        r.h = r.bottom() - n.y;
        r.y = n.y;
    }
    r
}

// ---- state ------------------------------------------------------------------

/// Everything the window manager knows, behind one lock.
///
/// **D54: never call into `platform` while holding this.** A cross-thread Win32
/// call on a window owned by the main thread sends a message and waits for that
/// thread's pump; if the pump is waiting on this lock, the process deadlocks
/// hard. Every mutating path here computes a plan under the lock, drops it, and
/// only then touches the OS.
#[derive(Default)]
pub struct WmState {
    pub graph: WindowGraph,
    pub layout: Layout,
    /// Native handle per classic window, indexed by `WindowId.0`.
    pub handles: Vec<NativeWindow>,
    /// The hidden group owners (D41).
    pub roots: Vec<NativeWindow>,
    /// Scale factor the current layout was derived at. Goes stale on
    /// `WM_DISPLAYCHANGE` (D57), so it is re-read rather than trusted.
    pub scale: f64,
    /// Cached display topology (D55).
    pub monitors: Vec<MonitorInfo>,
    /// Set for the duration of a title-bar drag.
    pub drag: Option<DragState>,
    /// Where the cursor was when a title bar or seam was pressed, for the
    /// drag that may follow to measure from (D182).
    pub pressed_at: Option<(Px, Px)>,
    /// `register` found no geometry to check D58 against (X11 before the
    /// show, D182), so the check runs once the windows are shown.
    pub confirm_pending: bool,
    /// Set for the duration of a splitter drag on a seam.
    pub splitter: Option<SplitterState>,
    /// Which window last took focus, so a window that mounts late can be told
    /// whether its group is active.
    pub focused: Option<WindowId>,
    /// Windows currently collapsed to the 275 x 14 strip (D60).
    pub shaded: BTreeSet<WindowId>,
    /// Height to restore on expand, per window. The playlist can be at any
    /// legal D30 size, so the base height is not the right answer for it.
    pub unshaded_h: BTreeMap<WindowId, Px>,
    /// 2x chrome (#47). Integer only: fractional chrome scaling is anti-scope.
    /// A bool rather than a factor so `Default` is 1x without a custom impl.
    pub double: bool,
    /// D190: the scale each window's size in `layout` was derived at, where
    /// each display has its own. A re-derive starts from it, never from the
    /// scale the window system reports now: the two differ for as long as a
    /// change of scale is on its way, and a size read at the wrong one is a
    /// window 1.5 times too big or too small, for good.
    pub drawn_at: BTreeMap<WindowId, f64>,
    /// D191: the presses whose gesture ends were heard lately, per window, so
    /// a start that arrives after its own end (each invoke is its own request,
    /// and they are not ordered) starts nothing.
    pub ended: BTreeMap<WindowId, Vec<u64>>,
    /// D191: the press whose gesture is live, so an end for another press,
    /// arriving late, does not end it.
    pub live: Option<(WindowId, u64)>,
    /// D190: when each window was last re-derived by a reconcile, so a window
    /// Windows flips back and forth stops being chased; cleared when a
    /// gesture begins or the chrome is re-zoomed (D191).
    pub reconciles: BTreeMap<WindowId, Vec<std::time::Instant>>,
    /// Set for the duration of a corner-grip resize of the playlist.
    pub resize: Option<ResizeState>,
    /// #86: the group is in the taskbar because Main's minimise put it there.
    /// The display watchdog (D57) reads "minimised" as "display lost" and
    /// would restore it two seconds later; this is how it tells the two apart.
    pub minimized: bool,
}

/// A title-bar drag in flight.
///
/// D40 lives here: the origin layout and origin cursor are captured once, and
/// every frame recomputes `origin + total_delta`. Nothing accumulates, so a
/// long drag cannot walk a bonded neighbour out of flush one rounding error at
/// a time — which is the same error shape that drifts 20 px over forty resize
/// steps.
#[derive(Clone, Debug)]
pub struct DragState {
    /// The connected component being moved. A title-bar drag moves the whole
    /// group with offsets preserved (`windows.md` gesture table).
    pub moving: Vec<WindowId>,
    pub origin_layout: Layout,
    pub origin_cursor: (Px, Px),
    /// The unshaded heights when the drag began, in the scale each window was
    /// on then, so a shaded window that crosses to another scale and back
    /// comes home with its own height (D187).
    pub origin_unshaded: BTreeMap<WindowId, Px>,
    /// The scale each moving window was drawn at when the drag began, as the
    /// window system had it (D188): the scale its origin size is in.
    pub origin_scales: BTreeMap<WindowId, f64>,
    /// The window whose title bar or seam was grabbed, raised last within
    /// its group on the release (D193).
    pub grabbed: WindowId,
}

impl WmState {
    fn handle(&self, id: WindowId) -> NativeWindow {
        self.handles
            .get(id.0 as usize)
            .copied()
            .unwrap_or(NativeWindow::NONE)
    }

    /// The chrome zoom the logical constants are multiplied by: 1.0 or 2.0.
    pub fn zoom(&self) -> f64 {
        if self.double {
            2.0
        } else {
            1.0
        }
    }
}

/// Tauri-managed wrapper. Separate type so `WmState` itself stays plain data
/// that the tests can build without a running app.
#[derive(Default)]
pub struct Wm(pub Mutex<WmState>);

// ---- window creation --------------------------------------------------------

/// The stack the app opens with: main on top, eq under it, playlist under that,
/// all three flush and bonded.
///
/// Pure, and computed **before** any window exists. That ordering is the whole
/// point — see `seed_state`.
pub fn initial_layout(scale: f64, zoom: f64) -> (Layout, WindowGraph) {
    let w = bond::d40::physical(CHROME_W * zoom, scale);
    let h = bond::d40::physical(CHROME_H * zoom, scale);
    let x0 = bond::d40::physical(120.0, scale);
    let y0 = bond::d40::physical(120.0, scale);

    let mut layout = Layout::new();
    for (i, id) in CLASSIC.iter().enumerate() {
        layout.insert(*id, Rect::new(x0, y0 + h * i as Px, w, h));
    }

    let mut graph = WindowGraph::new();
    for (a, b) in [(MAIN, EQ), (EQ, PLAYLIST)] {
        graph.insert(Bond::new(a, b, Edge::Bottom, (x0, x0 + w)));
    }
    (layout, graph)
}

/// Put the intended layout and bond graph into state **before** the windows
/// exist.
///
/// Not premature: it is the fix for a real race. A webview begins loading the
/// moment its window is constructed, and it calls `wm_hello` as soon as it
/// mounts — which lands before `register()` has run, and before the `listen`
/// subscriptions it would have raced are even live. Windows then come up
/// believing they have no bonds, so no seam is drawn, and every click on a seam
/// falls through to the title bar underneath and moves the group instead of
/// resizing it. Seeding first removes the race rather than narrowing it.
///
/// D58 still holds: this is *intent*, and `register` reconciles it against what
/// the OS actually did.
pub fn seed_state(app: &AppHandle) -> tauri::Result<()> {
    let monitors = read_monitors(app);
    let scale = monitors.first().map(|m| m.scale).unwrap_or(1.0);

    // D33: last session's geometry, bonds and shade state, if there are any,
    // and the chrome zoom they were saved at (#47). The two are stored
    // together for a reason: a 2x layout read back as 1x would be a stack of
    // windows twice the size the model thinks they are.
    let (restored, double) = match app.try_state::<crate::db::Db>() {
        Some(db) => {
            let conn = db.0.lock().unwrap();
            (
                load(&conn),
                crate::db::get_setting(&conn, DOUBLE_SETTING).as_deref() == Some("1"),
            )
        }
        None => (None, false),
    };
    let zoom = if double { 2.0 } else { 1.0 };

    let fresh = restored.is_none();
    let per_display = !platform::platform().one_scale();
    // D191: a fresh stack is sized for the display it lands on, which need
    // not be the first one listed.
    let fresh_scale = {
        let (probe, _) = initial_layout(scale, zoom);
        match (per_display, probe.get(&MAIN)) {
            (true, Some(r)) => dpi_monitor(&monitors, *r).map_or(scale, |m| m.scale),
            _ => scale,
        }
    };
    let (mut layout, mut graph, shaded, mut unshaded_h, stored) = match restored {
        Some(r) => (r.layout, r.graph, r.shaded, r.unshaded_h, r.drawn_at),
        None => {
            let (l, g) = initial_layout(fresh_scale, zoom);
            (l, g, BTreeSet::new(), BTreeMap::new(), BTreeMap::new())
        }
    };
    // D191: the scale each window's size is in once healed: the scale the
    // heal below lays it out for. A window whose saved size's scale cannot be
    // told has none, and is written so, to be told again next time.
    let drawn_at: BTreeMap<WindowId, f64> = if fresh {
        layout.keys().map(|id| (*id, fresh_scale)).collect()
    } else {
        launch_scales(&layout, &graph, &monitors, scale, zoom, &stored, &shaded)
            .into_iter()
            .filter_map(|(id, (saved, target))| saved.map(|_| (id, target)))
            .collect()
    };

    // D182: a layout saved at another scale comes back at the size it was
    // saved, where one scale covers the desktop and the toolkit draws at the
    // new one: a quarter of the chrome showing, or a quarter of the window
    // used. Main is a fixed size, so its width says the scale it was saved
    // at. The stored heights are the unshaded ones, as a re-derive wants.
    // Where each display has its own scale, a window saved at the wrong size
    // for its display (every build before D187 left one so on a second
    // display) is re-derived in place instead.
    if per_display && !fresh {
        if let Some((l, g)) =
            heal_saved_sizes(&layout, &graph, &monitors, scale, zoom, &stored, &shaded)
        {
            eprintln!("wm: the layout was saved at sizes its displays do not draw; re-derived");
            (layout, graph) = (l, g);
            for (id, h) in unshaded_h.iter_mut() {
                if let Some(r) = layout.get(id) {
                    *h = r.h;
                }
            }
        }
    }
    if platform::platform().one_scale() {
        if let Some(saved) = saved_scale(&layout, zoom) {
            if (saved - scale).abs() > 0.01 {
                eprintln!("wm: the layout was saved at scale {saved}, the desktop is at {scale}");
                // Positions stay: what the displays were then is not known,
                // and the rescue and the clamp below cover a layout that no
                // longer lands.
                (layout, graph) = rescale_layout(&layout, &graph, &[], &[], (saved, scale), zoom);
                for (id, h) in unshaded_h.iter_mut() {
                    if let Some(r) = layout.get(id) {
                        *h = r.h;
                    }
                }
            }
        }
    }

    // D33 again, and the reason the column records a monitor at all: a layout
    // saved on a display that is no longer attached must not restore into empty
    // space. Same rigid-translation rescue the display watchdog uses, so a
    // group that comes back does so with its bonds intact.
    layout = rescue_layout(&layout, &graph, &monitors);
    // #101, D88: and a group whose title bars sit off the usable screen, or
    // under the taskbar, is pulled to where a hand can reach it.
    layout = keep_layout_in_reach(
        &layout,
        &graph,
        &monitors,
        zoom,
        platform::platform().confines_to_work_area(),
    );

    // Re-collapse whatever was left shaded. The stored height is the *unshaded*
    // one, so this is a fresh collapse from a known-good size rather than a
    // 14 px rect remembered from last time — which is what keeps a resized
    // playlist from being lost across a restart.
    for id in CLASSIC {
        if !shaded.contains(&id) {
            continue;
        }
        let corner = layout
            .get(&id)
            .and_then(|r| monitor_at(&monitors, r.x, r.y))
            .map(|m| m.scale)
            .unwrap_or(scale);
        // D191: the strip at the scale the window's size is in.
        let at = derived_scale_in(&drawn_at, id, corner, per_display);
        apply_shade(
            &mut layout,
            &graph,
            id,
            bond::d40::physical(SHADE_H * zoom, at),
        );
    }

    let state = app.state::<Wm>();
    let mut s = state.0.lock().unwrap();
    s.double = double;
    s.scale = scale;
    // D190: what the heal above sized each window for. Where the rescue moved
    // a group, or Windows puts a window at another scale once it is shown,
    // the reconcile after the show follows it (D191).
    s.drawn_at = drawn_at;
    s.monitors = monitors;
    s.layout = layout;
    s.graph = graph;
    s.shaded = shaded;
    s.unshaded_h = unshaded_h;
    Ok(())
}

/// D192: the classic windows just built, laid out again for the scale Windows
/// gave each where that is not the one it was laid out for, and every window
/// whose rect is not the layout's put there, until a read finds nothing to
/// do; before `register` reads them back. `register` takes the window
/// system's rects as the truth and drops any bond they do not keep (D58), so
/// a window left at another scale's size here is a group come apart at
/// launch. The same following `reconcile` does (`follow_scales`), and the
/// same placing; four rounds at most, ending on a read.
fn settle_built(app: &AppHandle) {
    let p = platform::platform();
    if p.one_scale() {
        return;
    }
    let windows: Vec<(WindowId, NativeWindow, tauri::WebviewWindow)> = CLASSIC
        .iter()
        .filter_map(|id| {
            let win = app.get_webview_window(label_of(*id))?;
            Some((*id, platform::handle_of(&win), win))
        })
        .collect();
    for _ in 0..4 {
        for (_, h, _) in &windows {
            p.pump(*h);
        }
        let now: BTreeMap<WindowId, f64> = windows
            .iter()
            .filter_map(|(id, h, _)| Some((*id, p.window_scale(*h)?)))
            .collect();
        if now.is_empty() {
            return;
        }
        let (layout, followed) = {
            let state = app.state::<Wm>();
            let mut s = state.0.lock().unwrap();
            let followed = follow_scales(
                &mut s,
                &now,
                &BTreeSet::new(),
                std::time::Instant::now(),
                true,
                p.confines_to_work_area(),
            );
            (s.layout.clone(), followed)
        }; // D54: the lock is gone before any window call.
        if followed.rescaled {
            eprintln!("wm: Windows drew a window at another scale as it was built; followed");
        }
        let mut placed = false;
        for (id, h, win) in &windows {
            let (Some(r), Some((pos, size))) = (layout.get(id), platform::rect_of(win)) else {
                continue;
            };
            if (pos.x, pos.y, size.width as Px, size.height as Px) != (r.x, r.y, r.w, r.h) {
                if !p.place(*h, r.x, r.y, r.w, r.h) {
                    push_to_os(app, &layout, &[*id]);
                }
                placed = true;
            }
        }
        if !followed.rescaled && !placed {
            return;
        }
    }
    eprintln!("wm: the classic windows had not settled after four rounds at launch");
}

/// Build the three classic windows plus their hidden roots.
///
/// Sizes are declared in `PhysicalSize`, never the logical config keys (D38):
/// `275 x 1.5 = 412.5`, and a logically-sized window inherits a half pixel that
/// the toolkit resolves by its own rounding rule. For a bond model whose whole
/// premise is two windows sitting flush with a hairline seam, that would make
/// "flush" a property of tao's rounding mode.
pub fn build_classic_windows(app: &AppHandle) -> tauri::Result<()> {
    // Placed from the layout seeded before any window existed, so the model and
    // the screen start out saying the same thing.
    let (seeded, zoom, drawn_at, monitors) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        (
            s.layout.clone(),
            s.zoom(),
            s.drawn_at.clone(),
            s.monitors.clone(),
        )
    };
    let p = platform::platform();

    for id in CLASSIC.iter() {
        let win =
            WebviewWindowBuilder::new(app, label_of(*id), WebviewUrl::App(entry_of(*id).into()))
                .title(format!("hurricane-party — {}", label_of(*id)))
                .decorations(false)
                .shadow(false)
                // D43: EVERY classic window is resizable(false), the playlist
                // included, even though the app does resize it. An undecorated
                // resizable window gets an invisible TAURI_DRAG_RESIZE_WINDOW
                // helper ~8 physical px wide that hit-tests ABOVE the webview —
                // sitting exactly on top of the bond seam, which is precisely
                // where D35 puts the splitter. It ate every click in the spike
                // and nearly cost us the signature interaction. set_size() still
                // works, which is how the playlist resizes at all.
                .resizable(false)
                // D59: skipTaskbar + undecorated + minimized is unrecoverable,
                // and these windows are undecorated permanently. Main keeps its
                // taskbar button as the OS-level escape hatch; the other two are
                // satellites and follow it. Deliberate deviation from the spike,
                // which skipped the taskbar for all three and had no way back.
                .skip_taskbar(*id != MAIN)
                .disable_drag_drop_handler()
                .visible(false)
                .build()?;

        let r = seeded[id];
        // D192: where each display has its own scale, Windows is to judge the
        // saved rect from the scale its size is in, as it did while the app
        // ran (`land`). Moved first and sized after (D52), the window crossed
        // the seam at tao's default 800 x 600 and was judged on that: the
        // stack the owner left at 100 % on the seam came back 367 x 155 with
        // every bond dropped.
        let h = platform::handle_of(&win);
        let placed = match p.window_scale(h) {
            Some(now) => {
                // A window whose size's scale could not be told is laid out
                // for the display holding most of it.
                let to = drawn_at
                    .get(id)
                    .copied()
                    .unwrap_or_else(|| dpi_monitor(&monitors, r).map_or(now, |m| m.scale));
                land(p, h, r, now, to, &monitors);
                true
            }
            None => false,
        };
        if !placed {
            // D52: position, THEN size. Crossing a DPI boundary is not a pure
            // move — Windows sends WM_DPICHANGED and tao rescales to preserve
            // logical size, so size-then-position leaves the window 1.5x too
            // big.
            win.set_position(PhysicalPosition::new(r.x, r.y))?;
            platform::hold_size(&win, r.w as u32, r.h as u32);
            win.set_size(PhysicalSize::new(r.w as u32, r.h as u32))?;
        }
        // #47: the webview's own zoom factor, not a CSS zoom. The page lays
        // out at 275 x 116 as always and the browser renders it doubled, so
        // pointer maths, rects and the canvas backing store all agree.
        if zoom != 1.0 {
            win.set_zoom(zoom)?;
        }
        // Deliberately NOT shown here. See show_classic_windows: the webviews
        // start loading the moment a window exists, and they reach wm_hello
        // before setup has finished registering the graph.
    }

    // D41: the hidden roots. Never shown, never in the taskbar, never focusable
    // by the user — they exist only to be owners. An owned window is always
    // above its owner, so if a real member owned the group it would be pinned to
    // the back of its own group forever.
    for label in ROOT_LABELS {
        // An empty page, not main.html. A root is never shown, so rendering a
        // whole classic window in it costs a webview for nothing -- and every
        // one of them would also try to listen for events it has no capability
        // to receive.
        WebviewWindowBuilder::new(app, label, WebviewUrl::App("root.html".into()))
            .title(label)
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .skip_taskbar(true)
            .visible(false)
            .build()?;
    }

    // D193: each window's owner set now, before the settle, so a change of
    // scale it causes is seen there; `register`'s own setting then finds the
    // owners already set and leaves them.
    own_built(app);
    // D192: last, after every webview is built (building one pumps messages,
    // which can deliver a change of scale), and directly before `register`.
    settle_built(app);
    Ok(())
}

/// D193: the owners `register` will give the windows just built, given them
/// now, from the seeded graph and the roots just built, the same way
/// `plan_ownership` assigns them.
fn own_built(app: &AppHandle) {
    let roots: Vec<NativeWindow> = ROOT_LABELS
        .iter()
        .filter_map(|l| app.get_webview_window(l))
        .map(|w| platform::handle_of(&w))
        .collect();
    if roots.is_empty() {
        return;
    }
    let handle = |id: WindowId| {
        app.get_webview_window(label_of(id))
            .map(|w| platform::handle_of(&w))
            .unwrap_or(NativeWindow::NONE)
    };
    let plan = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        let mut owners = Vec::new();
        for (i, comp) in s.graph.components(&CLASSIC).iter().enumerate() {
            let root = roots[i.min(roots.len() - 1)];
            owners.extend(comp.iter().map(|id| (handle(*id), root)));
        }
        OwnPlan {
            owners,
            raise: Vec::new(),
            monitors: s.monitors.clone(),
            rects: CLASSIC
                .iter()
                .filter_map(|id| Some((handle(*id), *s.layout.get(id)?)))
                .collect(),
            scales: CLASSIC
                .iter()
                .filter_map(|id| Some((handle(*id), *s.drawn_at.get(id)?)))
                .collect(),
        }
    }; // D54: the lock is gone before any window call.
    apply_ownership(&plan);
}

/// D192: put a just-built window at `r`, laid out for scale `to`, so that
/// Windows judges `r` from `to`, as it would have while the app ran. The
/// window is made at the scale `now` of wherever the window system put it.
/// Where that is not `to`, it first goes wholly onto the display at `to`
/// nearest `r`, at its size at `now`, so Windows gives it `to` and tao's
/// resize for that makes it about `r`'s size; judged from anything else, a
/// stack left at 150 % half over the 100 % display came back at 100 %.
/// Then `r` exactly, judged from `to`, as while the app ran. A change of
/// scale is delivered as each placement is made (`pump`); what Windows
/// decides otherwise, `settle_built` follows.
fn land<P: OwnerOps + ?Sized>(
    p: &P,
    h: NativeWindow,
    r: Rect,
    now: f64,
    to: f64,
    monitors: &[MonitorInfo],
) {
    if !same_scale(now, to) {
        let centre = |q: Rect| (q.x as i64 + q.w as i64 / 2, q.y as i64 + q.h as i64 / 2);
        let (cx, cy) = centre(r);
        let near = monitors
            .iter()
            .filter(|m| same_scale(m.scale, to))
            .min_by_key(|m| {
                let (mx, my) = centre(m.work);
                (mx - cx).pow(2) + (my - cy).pow(2)
            });
        if let Some(m) = near {
            let e = sized_for(r, to, now);
            p.put(h, m.work.x, m.work.y, e.w.min(m.work.w), e.h.min(m.work.h));
            p.deliver(h);
        }
    }
    p.put(h, r.x, r.y, r.w, r.h);
    p.deliver(h);
}

/// Reveal the windows, once the graph behind them is real.
///
/// Splitting this out is not tidiness. The webviews begin loading as soon as
/// their window exists, and they call `wm_hello` as soon as they mount — which
/// lands **before** `register()` finishes. The windows then come up believing
/// they have no bonds, so no seam is drawn, and every click on a seam falls
/// through to the title bar underneath and moves the group instead of resizing
/// it. Building hidden and showing afterwards removes the race rather than
/// narrowing it, and it also avoids a frame of unbonded windows on screen.
pub fn show_classic_windows(app: &AppHandle) -> tauri::Result<()> {
    for id in CLASSIC {
        if let Some(win) = app.get_webview_window(label_of(id)) {
            win.show()?;
        }
    }
    // D58 needs the OS's answer, and on X11 there is none until the windows
    // are mapped, which is now, and asynchronously (D182). `register` left
    // the check for this moment; it runs off the main thread, which has to
    // keep turning for the answer to arrive. Windows answered in `register`.
    let pending = std::mem::take(&mut app.state::<Wm>().0.lock().unwrap().confirm_pending);
    if pending {
        let app = app.clone();
        std::thread::spawn(move || confirm_layout(&app));
    }
    // D191: a window Windows shows at another scale than the layout was
    // healed for, or that tao resized for its display as it was placed, is
    // followed once the show has settled.
    reconcile_later(app);
    Ok(())
}

/// D58, once the OS can answer: read the windows back, take the OS's word for
/// the layout, and drop any bond it does not bear out. Only where `register`
/// had no geometry to read (D182).
fn confirm_layout(app: &AppHandle) {
    // Settled is three reads in a row that agree: X moves a window after it
    // maps, so the first answer can be the compositor's placement, not ours.
    // About 0.2 to 0.7 s after the show, measured; two seconds is the cap.
    let started = std::time::Instant::now();
    let (mut last, mut agreeing) = (Layout::new(), 0);
    let read = loop {
        let mut read = Layout::new();
        for id in CLASSIC {
            if let Some(win) = app.get_webview_window(label_of(id)) {
                if let Some((p, s)) = platform::rect_of(&win) {
                    read.insert(id, Rect::new(p.x, p.y, s.width as Px, s.height as Px));
                }
            }
        }
        agreeing = if read.len() == CLASSIC.len() && read == last {
            agreeing + 1
        } else {
            0
        };
        if agreeing >= 2 || started.elapsed().as_millis() > 2000 {
            break read;
        }
        last = read;
        std::thread::sleep(std::time::Duration::from_millis(16));
    };
    // A window has no X id until GTK realizes it, which for the classic three
    // is the show, so the handles `register` read are read again.
    let handles: Vec<NativeWindow> = CLASSIC
        .iter()
        .map(|id| {
            app.get_webview_window(label_of(*id))
                .map(|w| platform::handle_of(&w))
                .unwrap_or(NativeWindow::NONE)
        })
        .collect();
    let plan = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        s.handles = handles;
        for (id, got) in read.iter() {
            if s.layout.get(id).is_some_and(|want| want != got) {
                eprintln!(
                    "wm: {id:?} was put at {:?}, the OS has it at {got:?}",
                    s.layout[id]
                );
            }
            s.layout.insert(*id, *got);
        }
        let stale: Vec<(WindowId, WindowId)> = bond::violations(&s.graph, &s.layout)
            .into_iter()
            .map(|(b, why)| {
                eprintln!(
                    "wm: dropping bond {:?}-{:?}, the OS disagrees: {why}",
                    b.a, b.b
                );
                b.pair()
            })
            .collect();
        for (a, b) in stale {
            s.graph.break_bond(a, b);
        }
        plan_ownership(&s, Some(MAIN))
    };
    apply_ownership(&plan);
    emit_state(app);
}

/// Read the windows back out of the OS and seed the state from what is actually
/// on screen, rather than from what we asked for.
///
/// D58 is the reason this reads instead of assuming: the bond graph stays
/// perfectly self-consistent while describing a layout that exists nowhere, so
/// the OS is the authority and the model is the thing that gets corrected.
pub fn register(app: &AppHandle) -> tauri::Result<()> {
    let scale = app
        .primary_monitor()?
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);

    // D55: read the topology once, here. Every interaction afterwards is a
    // lookup against this cache, because monitor_from_point() from a sync
    // command deadlocks and displays move about once a day.
    let monitors: Vec<MonitorInfo> = app
        .available_monitors()?
        .iter()
        .map(|m| MonitorInfo {
            rect: Rect::new(
                m.position().x,
                m.position().y,
                m.size().width as Px,
                m.size().height as Px,
            ),
            scale: m.scale_factor(),
            work: {
                let wa = m.work_area();
                Rect::new(
                    wa.position.x,
                    wa.position.y,
                    wa.size.width as Px,
                    wa.size.height as Px,
                )
            },
        })
        .collect();

    let mut handles = Vec::with_capacity(CLASSIC.len());
    let mut layout = Layout::new();
    // On X11 a window built hidden has no geometry until it is mapped, and X
    // answers a move later, not now (D182). A window with no answer yet keeps
    // its seeded rect, and D58's check waits for `confirm_layout` after the
    // show rather than judging the bonds by nothing. Windows always answers.
    let mut unanswered = false;
    for id in CLASSIC {
        let Some(win) = app.get_webview_window(label_of(id)) else {
            continue;
        };
        handles.push(platform::handle_of(&win));
        let Some((p, s)) = platform::rect_of(&win) else {
            unanswered = true;
            continue;
        };
        layout.insert(id, Rect::new(p.x, p.y, s.width as Px, s.height as Px));
    }

    let roots = ROOT_LABELS
        .iter()
        .filter_map(|l| app.get_webview_window(l))
        .map(|w| platform::handle_of(&w))
        .collect();

    // D58: the seeded graph is intent, and intent is not evidence. Drop any
    // seeded bond the OS does not actually agree with — a graph that stays
    // perfectly self-consistent while describing a layout existing nowhere is
    // the exact failure that decision is about, and internal agreement would
    // never catch it.
    let plan = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        s.scale = scale;
        s.monitors = monitors;
        s.handles = handles;
        s.roots = roots;
        s.confirm_pending = unanswered;
        if unanswered {
            for id in CLASSIC {
                if let (None, Some(r)) = (layout.get(&id), s.layout.get(&id)) {
                    layout.insert(id, *r);
                }
            }
        }
        s.layout = layout;
        let violations = if unanswered {
            Vec::new()
        } else {
            bond::violations(&s.graph, &s.layout)
        };
        let stale: Vec<(WindowId, WindowId)> = violations
            .into_iter()
            .map(|(b, why)| {
                eprintln!(
                    "wm: dropping bond {:?}-{:?}, the OS disagrees: {why}",
                    b.a, b.b
                );
                b.pair()
            })
            .collect();
        for (a, b) in stale {
            s.graph.break_bond(a, b);
        }
        plan_ownership(&s, Some(MAIN))
    }; // D54: lock dropped here, before a single OS call.

    apply_ownership(&plan);
    emit_state(app);
    Ok(())
}

// ---- z-order ----------------------------------------------------------------

/// What to do about ownership, computed under the lock. Performing it is a
/// separate step that must run with the lock released (D54).
#[derive(Debug, Default, PartialEq)]
pub struct OwnPlan {
    /// `(window, owner)` pairs.
    pub owners: Vec<(NativeWindow, NativeWindow)>,
    /// Windows to force to the top, in order, bottom group first.
    pub raise: Vec<NativeWindow>,
    /// The displays, for putting a hidden root where its window is (D193).
    pub monitors: Vec<MonitorInfo>,
    /// Where each window is laid out, to put one back that setting its
    /// owner moved to another scale all the same (D193).
    pub rects: Vec<(NativeWindow, Rect)>,
    /// The scale each window is laid out for (`drawn_at`): what the
    /// read-back judges against and puts a window back to (D193). The root
    /// is put at the scale the window is at, not this.
    pub scales: Vec<(NativeWindow, f64)>,
}

/// D41 + D42. Give every connected component its own hidden root, then force the
/// z-order — ownership alone applies lazily, on next activation, so after a bond
/// break the order is stale-but-plausible until the user clicks something.
///
/// `active` is the component the user just touched; it ends up on top.
///
/// D56 qualifies all of this: within our own process the ordering is real, but
/// none of it can lift a window above another application's foreground window
/// without stealing focus, which we will not do.
pub fn plan_ownership(state: &WmState, active: Option<WindowId>) -> OwnPlan {
    if state.roots.is_empty() {
        return OwnPlan::default();
    }
    let comps = state.graph.components(&CLASSIC);

    let mut owners = Vec::new();
    for (i, comp) in comps.iter().enumerate() {
        // More components than roots cannot happen with three windows, but
        // clamping is cheaper than a panic if a fourth ever appears.
        let root = state.roots[i.min(state.roots.len() - 1)];
        for id in comp {
            owners.push((state.handle(*id), root));
        }
    }

    let mut order: Vec<&Vec<WindowId>> = comps.iter().collect();
    if let Some(a) = active {
        // Stable sort, false before true: the active component ends up last, so
        // it is raised last and therefore sits on top.
        order.sort_by_key(|c| c.contains(&a));
    }
    // And within it the window touched last of all, so it is the one on top
    // where its group's windows overlap (D193: the owner's hand test found
    // the clicked Main behind the EQ).
    let raise = order
        .iter()
        .flat_map(|c| {
            let mut ids: Vec<WindowId> = c.to_vec();
            if let Some(a) = active {
                ids.sort_by_key(|id| *id == a);
            }
            ids.into_iter().map(|id| state.handle(id))
        })
        .collect();

    OwnPlan {
        owners,
        raise,
        monitors: state.monitors.clone(),
        rects: CLASSIC
            .iter()
            .filter_map(|id| Some((state.handle(*id), *state.layout.get(id)?)))
            .collect(),
        scales: CLASSIC
            .iter()
            .filter_map(|id| Some((state.handle(*id), *state.drawn_at.get(id)?)))
            .collect(),
    }
}

/// D193: whether setting a window's owner moved it off the scale it is laid
/// out for, and so is to be put back: the scale it is at now and the one to
/// put it back to. Only a window that was at that scale before the change
/// (`before`, read once the drop's own change was delivered): one not there
/// yet is still crossing, and `reconcile` follows where it lands; judging it
/// from the scale it had been at "put back" a correct crossing. `laid_out`
/// is `drawn_at`, or where there is none the scale it was at.
fn put_back(before: Option<f64>, laid_out: Option<f64>, after: Option<f64>) -> Option<(f64, f64)> {
    let (b, a) = (before?, after?);
    let t = laid_out.unwrap_or(b);
    (same_scale(b, t) && !same_scale(a, t)).then_some((a, t))
}

/// D193: the `(window, owner)` pairs of a plan whose owner is not already the
/// one planned: the only ones to set, since setting an owner gives the window
/// the owner's scale for a moment.
fn owner_changes(
    plan: &OwnPlan,
    owner_of: impl Fn(NativeWindow) -> NativeWindow,
) -> Vec<(NativeWindow, NativeWindow)> {
    plan.owners
        .iter()
        .copied()
        .filter(|(w, owner)| owner_of(*w) != *owner)
        .collect()
}

/// Perform an [`OwnPlan`]. **Must be called with no lock held** (D54).
pub fn apply_ownership(plan: &OwnPlan) {
    apply_ownership_on(platform::platform(), plan);
}

/// The window calls setting ownership and putting a window back make, so
/// the order they are made in can be tested against a window system that
/// holds a change of scale back until it is pumped (D193). Named apart from
/// [`platform::WindowPlatform`]'s, which every window system has through
/// the blanket impl below.
pub trait OwnerOps {
    fn owner(&self, w: NativeWindow) -> NativeWindow;
    fn own(&self, w: NativeWindow, owner: NativeWindow);
    fn lift(&self, w: NativeWindow);
    fn scale(&self, w: NativeWindow) -> Option<f64>;
    fn put(&self, w: NativeWindow, x: i32, y: i32, cx: i32, cy: i32) -> bool;
    fn deliver(&self, w: NativeWindow);
}

impl<T: platform::WindowPlatform + ?Sized> OwnerOps for T {
    fn owner(&self, w: NativeWindow) -> NativeWindow {
        self.owner_of(w)
    }
    fn own(&self, w: NativeWindow, owner: NativeWindow) {
        self.set_owner(w, owner);
    }
    fn lift(&self, w: NativeWindow) {
        self.raise_no_activate(w);
    }
    fn scale(&self, w: NativeWindow) -> Option<f64> {
        self.window_scale(w)
    }
    fn put(&self, w: NativeWindow, x: i32, y: i32, cx: i32, cy: i32) -> bool {
        self.place(w, x, y, cx, cy)
    }
    fn deliver(&self, w: NativeWindow) {
        self.pump(w);
    }
}

fn apply_ownership_on<P: OwnerOps + ?Sized>(p: &P, plan: &OwnPlan) {
    // D193: a window's owner is set only when it changes. Setting it gives
    // the window the owner's scale for a moment, and the hidden roots sit
    // wherever tao made them, on the primary display at 100 %: an 825 stack
    // dropped at Main x = 2285 went to 96 DPI as its ownership was re-applied
    // on the release, tao shrank it to 550, and Windows judged the 550 to be
    // mostly on the 100 % display, where it stayed.
    for (w, owner) in owner_changes(plan, |w| p.owner(w)) {
        // The scale the window is laid out for, with any change of scale the
        // drop that brought it here caused delivered first: a re-dock that
        // also crossed the seam on its last frame is not yet at its new
        // scale, and judged from the old one it was "put back" across.
        p.deliver(w);
        let before = p.scale(w);
        let laid_out = plan.scales.iter().find(|(h, _)| *h == w).map(|(_, s)| *s);
        // Where it changes, the root goes first to a display at the scale the
        // window is at, so the change gives it no other scale. Not the one it
        // is laid out for, where the two differ: that would push a window
        // Windows has put elsewhere across, and `reconcile` follows Windows.
        if let Some(scale) = before {
            if let Some(m) = plan.monitors.iter().find(|m| same_scale(m.scale, scale)) {
                p.put(owner, m.work.x, m.work.y, 1, 1);
                p.deliver(owner);
            }
        }
        p.own(w, owner);
        p.deliver(w);
        // Read back: a window that was at the scale it is laid out for and
        // the change moved off it all the same (the root's own change not
        // delivered yet, or no display at its scale cached) is put back,
        // judged from that scale (`land`). One not there yet is `reconcile`'s.
        if let Some((now, to)) = put_back(before, laid_out, p.scale(w)) {
            eprintln!("wm: setting a window's owner moved it to another scale; put back");
            if let Some((_, r)) = plan.rects.iter().find(|(h, _)| *h == w) {
                land(p, w, *r, now, to, &plan.monitors);
            }
        }
    }
    for w in &plan.raise {
        p.lift(*w);
    }
}

// ---- pushing geometry to the OS ---------------------------------------------

/// Write a layout back to the OS.
///
/// D52: position first, then size, always. A `set_position` that crosses a DPI
/// boundary makes Windows send `WM_DPICHANGED`, and tao responds by rescaling
/// the window to preserve its *logical* size — so doing it the other way round
/// leaves the window 1.5x too big on the far side of the seam.
pub fn push_to_os(app: &AppHandle, layout: &Layout, ids: &[WindowId]) {
    for id in ids {
        let (Some(r), Some(win)) = (layout.get(id), app.get_webview_window(label_of(*id))) else {
            continue;
        };
        let _ = win.set_position(PhysicalPosition::new(r.x, r.y));
        platform::hold_size(&win, r.w as u32, r.h as u32);
        let _ = win.set_size(PhysicalSize::new(r.w as u32, r.h as u32));
    }
}

// ---- drag -------------------------------------------------------------------

/// One frame of a title-bar drag, as pure geometry.
///
/// Everything the drag does that could be wrong is in here, and none of it
/// touches the OS: recompute from the origin, translate the group rigidly, then
/// look for a magnet.
///
/// The order matters. The group is translated **first** and probed **after**,
/// so the snap is measured against where the windows now are rather than where
/// they were — probing first would make the magnet fire a frame late and read
/// as lag rather than as attraction.
pub fn drag_frame(
    origin_layout: &Layout,
    moving: &[WindowId],
    total: (Px, Px),
    others: &[WindowId],
    threshold: Px,
    screen: Option<Rect>,
) -> Layout {
    // D40: origin + total delta, every frame, from scratch. Never
    // current + frame delta — that is the accumulating form, and it walks a
    // bonded neighbour out of flush one rounding error at a time.
    let mut layout = origin_layout.clone();
    bond::translate_group(&mut layout, moving, total.0, total.1);

    // Best magnet across every moving/stationary pair. Cheapest wins, so a
    // window between two candidates goes to the nearer one rather than to
    // whichever happened to be checked first.
    let mut best: Option<(Px, Px, Px)> = None;
    for m in moving {
        let Some(mr) = layout.get(m).copied() else {
            continue;
        };
        for f in others {
            let Some(fr) = layout.get(f).copied() else {
                continue;
            };
            if let Some(snap) = bond::probe(mr, fr, threshold) {
                // #100, D89: a magnet never pulls a window onto another's body.
                // A window snapped flush to one edge can still be lying on a
                // third window that shares it; that candidate is not a bond.
                let mut trial = layout.clone();
                bond::translate_group(&mut trial, moving, snap.dx, snap.dy);
                if bond::any_overlap(&trial, moving, others) {
                    continue;
                }
                let cost = snap.dx.abs() + snap.dy.abs();
                if best.is_none_or(|(c, _, _)| cost < c) {
                    best = Some((cost, snap.dx, snap.dy));
                }
            }
        }
    }
    if let Some((_, dx, dy)) = best {
        bond::translate_group(&mut layout, moving, dx, dy);
        return layout;
    }

    // Nothing to bond to, so try the desktop edge instead. A movement
    // constraint only — no resize and no graph node (windows.md).
    if let (Some(screen), Some(bounds)) = (screen, bond::bounds(&layout, moving)) {
        let (dx, dy) = bond::screen_edge_snap(bounds, screen, threshold);
        if dx != 0 || dy != 0 {
            bond::translate_group(&mut layout, moving, dx, dy);
        }
    }
    layout
}

/// The bonds a completed drag has earned.
///
/// Only moving-to-stationary pairs are considered. Two windows that both sat
/// still and merely happen to be flush are left alone — otherwise a bond the
/// user had just demagnetized would silently re-form on the next unrelated
/// drag, and the break would look like it had never worked.
pub fn bonds_after_drag(layout: &Layout, moving: &[WindowId], others: &[WindowId]) -> Vec<Bond> {
    // #100, D89: a window whose body lies over another never bonds. It stays
    // loose, above whatever it landed on, and the next drag of the group
    // leaves it behind. The edge test alone cannot tell a docked window from
    // one dropped on a neighbour's body when a third window shares the edge.
    if bond::any_overlap(layout, moving, others) {
        return vec![];
    }
    let mut out = vec![];
    for m in moving {
        let Some(mr) = layout.get(m).copied() else {
            continue;
        };
        for f in others {
            let Some(fr) = layout.get(f).copied() else {
                continue;
            };
            // Exactly flush, not nearly. The drag has already snapped, so any
            // gap left at this point is a real gap, not a rounding artefact.
            let vspan = (mr.y.max(fr.y), mr.bottom().min(fr.bottom()));
            let hspan = (mr.x.max(fr.x), mr.right().min(fr.right()));
            if vspan.1 > vspan.0 {
                if mr.right() == fr.x {
                    out.push(Bond::new(*m, *f, Edge::Right, vspan));
                } else if fr.right() == mr.x {
                    out.push(Bond::new(*m, *f, Edge::Left, vspan));
                }
            }
            if hspan.1 > hspan.0 {
                if mr.bottom() == fr.y {
                    out.push(Bond::new(*m, *f, Edge::Bottom, hspan));
                } else if fr.bottom() == mr.y {
                    out.push(Bond::new(*m, *f, Edge::Top, hspan));
                }
            }
        }
    }
    out
}

/// Begin a title-bar drag. Moves the whole connected component with offsets
/// preserved (windows.md gesture table).
pub fn drag_start(app: &AppHandle, id: WindowId) {
    // Read the cursor before taking the lock. It takes no window handle so it
    // could not deadlock either way, but keeping every OS call outside the
    // critical section is the habit D54 is asking for.
    let cursor = platform::platform().cursor_pos();
    let moving = app.state::<Wm>().0.lock().unwrap().graph.component(id);
    let now = os_scales(app, &moving);
    let state = app.state::<Wm>();
    let mut s = state.0.lock().unwrap();
    // D190: the origin's sizes are in the scale they were derived at.
    let origin_scales = laid_out_at(&s, &now);
    begin_gesture(&mut s);
    let origin_layout = s.layout.clone();
    // The drag starts on the first pointermove, so the cursor has already
    // left the press by the time this runs; measuring from here would leave
    // the group that far behind the pointer for the whole drag (up to 24 px on
    // a flick, measured on Linux, and less but the same on Windows, D182).
    // The press is the true origin.
    let origin_cursor = s.pressed_at.take().unwrap_or(cursor);
    let origin_unshaded = s.unshaded_h.clone();
    s.drag = Some(DragState {
        moving,
        origin_layout,
        origin_cursor,
        origin_unshaded,
        origin_scales,
        grabbed: id,
    });
}

/// A title bar or seam was pressed: note where the cursor is, for the drag
/// that may follow. No capture and no state beyond this, so a click stays a
/// click; the next press overwrites it.
pub fn press(app: &AppHandle) {
    let cursor = platform::platform().cursor_pos();
    app.state::<Wm>().0.lock().unwrap().pressed_at = Some(cursor);
}

/// One drag frame, driven by the webview pointermove that the compositor has
/// already coalesced to one per display frame (O15).
///
/// D191: one push per frame. Windows changes a window's scale after the
/// placement that caused it, so a read-back in the frame cannot be trusted
/// to have seen it; the next frame starts from the scale each window is drawn
/// at by then, and once the drag ends `reconcile` follows what is left.
pub fn drag_move(app: &AppHandle) {
    let cursor = platform::platform().cursor_pos();
    let Some(moving) = app
        .state::<Wm>()
        .0
        .lock()
        .unwrap()
        .drag
        .as_ref()
        .map(|d| d.moving.clone())
    else {
        return;
    };
    let now = os_scales(app, &moving);
    let Some((layout, targets)) = drag_layout(app, cursor, &now) else {
        return;
    };
    // D54: drag_layout has released the lock before the OS is touched.
    push_settled(app, &layout, &moving, &targets);
}

/// One drag frame's layout, as `drag_move` pushes it, with the scale it
/// lays each moving window out at. `now` is the scale each is drawn at.
fn drag_layout(
    app: &AppHandle,
    cursor: (Px, Px),
    now: &BTreeMap<WindowId, f64>,
) -> Option<(Layout, BTreeMap<WindowId, f64>)> {
    let state = app.state::<Wm>();
    let mut s = state.0.lock().unwrap();
    let drag = s.drag.clone()?;

    let total = (
        cursor.0 - drag.origin_cursor.0,
        cursor.1 - drag.origin_cursor.1,
    );
    let others: Vec<WindowId> = CLASSIC
        .iter()
        .copied()
        .filter(|c| !drag.moving.contains(c))
        .collect();

    // D51: the threshold comes from the monitor under the *cursor*, and it
    // is recomputed from its logical definition every frame rather than
    // scaled up from a previous physical value (D40).
    let scale = scale_at(&s.monitors, cursor.0, cursor.1, s.scale);
    let threshold = bond::d40::threshold(SNAP_THRESHOLD, scale);

    // D53: the edge to snap against is the desktop's outer edge, never the
    // boundary between two adjacent displays. Which edges count depends on
    // the group's own extent, so it is measured from where the group is
    // right now rather than from where the drag started.
    let dragged_now = {
        let mut l = drag.origin_layout.clone();
        bond::translate_group(&mut l, &drag.moving, total.0, total.1);
        bond::bounds(&l, &drag.moving)
    };
    let screen = match (monitor_at(&s.monitors, cursor.0, cursor.1), dragged_now) {
        (Some(m), Some(g)) => Some(screen_rect_for(&s.monitors, m.rect, g)),
        _ => None,
    };

    let frame = drag_frame(
        &drag.origin_layout,
        &drag.moving,
        total,
        &others,
        threshold,
        screen,
    );
    // D187: a window that has crossed onto a display at another scale is
    // re-derived for it, rather than pushed back to the size it had where
    // the drag began, which undid Windows' own resize every frame. D188:
    // which scale is settled against where the re-pack puts each window,
    // not where it was before, and the way Windows picks it.
    let zoom = s.zoom();
    let confines = platform::platform().confines_to_work_area();
    let (targets, layout, unshaded) = settle_scales(&drag.moving, now, &s.monitors, |t| {
        let scales: BTreeMap<WindowId, (f64, f64)> = drag
            .moving
            .iter()
            .filter_map(|id| Some((*id, (*drag.origin_scales.get(id)?, *t.get(id)?))))
            .collect();
        let (mut layout, unshaded) = fit_to_displays(
            &frame,
            &drag.moving,
            &s.graph,
            &s.shaded,
            &drag.origin_unshaded,
            &scales,
            zoom,
        )
        .unwrap_or_else(|| (frame.clone(), BTreeMap::new()));
        // #101, D88: a title bar never leaves a display's work area; where
        // the window manager confines windows, no part of one does (D182).
        // After the magnet, so a snap cannot put one out of reach either.
        let (cx, cy) = keep_in_reach(&layout, &drag.moving, &s.monitors, zoom, confines);
        if (cx, cy) != (0, 0) {
            bond::translate_group(&mut layout, &drag.moving, cx, cy);
        }
        (layout, unshaded)
    });
    for id in &drag.moving {
        if let Some(h) = drag.origin_unshaded.get(id) {
            s.unshaded_h.insert(*id, *h);
        }
    }
    s.unshaded_h.extend(unshaded);
    s.layout = layout.clone();
    s.drawn_at.extend(&targets);
    Some((layout, targets))
}

/// D191: the scale a window's size in the layout is in, for anything that
/// sizes it in place: a splitter, the grip, a shade. Where each display has
/// its own scale, the one it was derived at (D190); `legacy` where one scale
/// covers the desktop, or for a window not derived yet. Taking the scale
/// under the cursor or the top-left corner instead sized a window on the
/// seam at a scale it was not drawn at.
fn derived_scale(s: &WmState, id: WindowId, legacy: f64) -> f64 {
    derived_scale_in(&s.drawn_at, id, legacy, !platform::platform().one_scale())
}

/// `derived_scale`, pure: `per_display` is whether each display has its own
/// scale.
fn derived_scale_in(
    drawn_at: &BTreeMap<WindowId, f64>,
    id: WindowId,
    legacy: f64,
    per_display: bool,
) -> f64 {
    if !per_display {
        return legacy;
    }
    drawn_at.get(&id).copied().unwrap_or(legacy)
}

/// D190: the scale each window's size in the layout is in: the one it was
/// derived at where each display has its own scale, and `now` where one
/// scale covers the desktop and the engine's answer is the only one.
fn laid_out_at(s: &WmState, now: &BTreeMap<WindowId, f64>) -> BTreeMap<WindowId, f64> {
    laid_out_at_in(&s.drawn_at, now, !platform::platform().one_scale())
}

/// `laid_out_at`, pure.
fn laid_out_at_in(
    drawn_at: &BTreeMap<WindowId, f64>,
    now: &BTreeMap<WindowId, f64>,
    per_display: bool,
) -> BTreeMap<WindowId, f64> {
    now.iter()
        .map(|(id, n)| (*id, derived_scale_in(drawn_at, *id, *n, per_display)))
        .collect()
}

/// D188: the scale each window is drawn at now. The window system's answer
/// where each display has its own; where one covers the desktop, the
/// display holding most of the window, which is that one scale.
fn os_scales(app: &AppHandle, ids: &[WindowId]) -> BTreeMap<WindowId, f64> {
    let (handles, layout, monitors, fallback) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        let handles: Vec<(WindowId, NativeWindow)> =
            ids.iter().map(|id| (*id, s.handle(*id))).collect();
        (handles, s.layout.clone(), s.monitors.clone(), s.scale)
    }; // D54: no OS call under the lock.
    let p = platform::platform();
    handles
        .into_iter()
        .filter_map(|(id, h)| {
            let scale = p.window_scale(h).or_else(|| {
                let r = layout.get(&id)?;
                Some(dpi_monitor(&monitors, *r).map_or(fallback, |m| m.scale))
            })?;
            Some((id, scale))
        })
        .collect()
}

/// D188: push a layout laid out for `targets`, the scale each window will
/// be drawn at.
///
/// A window going to another scale is first put where it goes at the size it
/// has at the scale it is drawn at now. Whether that moves it to the other
/// display's scale is the window system's call, and the resize for the new
/// scale is its own. Setting the new scale's size first is what flashed
/// 825 x 348 up to 1238 x 522 on a crossing: Windows changed the window's
/// scale on the resize, and grew the already grown window by the ratio again.
/// If the scale is already the new one when the placement returns, the window
/// gets its exact size there and then; if not, it keeps the old scale's size,
/// never drawn wrong, and is left for the change to land: the next drag frame,
/// or `reconcile` (D191), which follows the change or, if Windows kept the
/// window where it was, the refusal. Where there is no answer to ask for (one
/// scale for the desktop), the toolkit's setters as before.
fn push_settled(
    app: &AppHandle,
    layout: &Layout,
    ids: &[WindowId],
    targets: &BTreeMap<WindowId, f64>,
) {
    let handles: Vec<(WindowId, NativeWindow)> = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        ids.iter().map(|id| (*id, s.handle(*id))).collect()
    }; // D54: no OS call under the lock.
    let p = platform::platform();
    for (id, h) in handles {
        let Some(r) = layout.get(&id).copied() else {
            continue;
        };
        let (Some(now), Some(&to)) = (p.window_scale(h), targets.get(&id)) else {
            push_to_os(app, layout, &[id]);
            continue;
        };
        if !same_scale(now, to) {
            let e = sized_for(r, to, now);
            p.place(h, e.x, e.y, e.w, e.h);
            if !p.window_scale(h).is_some_and(|got| same_scale(got, to)) {
                continue;
            }
        }
        if !p.place(h, r.x, r.y, r.w, r.h) {
            push_to_os(app, layout, &[id]);
        }
    }
}

/// Recompute every bond's span from where the windows actually are.
///
/// The span is the overlapping extent of a shared boundary, so it moves when
/// the windows move. Nothing reads it yet — `violations` checks flushness and
/// overlap, not the recorded span — which is exactly why it needs doing now:
/// it is persisted (D33), and a stale span would be silently written to disk
/// and read back as though it meant something.
pub fn resync_spans(graph: &mut WindowGraph, layout: &Layout) {
    for b in &mut graph.bonds {
        let (Some(ra), Some(rb)) = (layout.get(&b.a).copied(), layout.get(&b.b).copied()) else {
            continue;
        };
        b.span = if b.edge.is_vertical_seam() {
            (ra.y.max(rb.y), ra.bottom().min(rb.bottom()))
        } else {
            (ra.x.max(rb.x), ra.right().min(rb.right()))
        };
    }
}

/// End a drag: form whatever bonds the final position earned, then re-apply the
/// ownership topology so the new group shape is real in the z-order too.
pub fn drag_end(app: &AppHandle, release: bool) {
    // The webview drops a pointermove while the last one is still in flight
    // (Classic.svelte's `frame`), so the final position can arrive only as the
    // release. One last move from the real cursor puts the group where the
    // pointer let go; a move is computed from the drag's origin, so a repeat
    // costs nothing (up to 24 px short without it, D182). Only on a release:
    // a drag whose release was lost is ended when the pointer comes back,
    // wherever that is, and is left where its last frame put it (D192).
    if release {
        drag_move(app);
    }
    let plan = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        let Some(drag) = s.drag.take() else {
            // D191: a seam whose `wm_seam_down` answered after the release
            // was ended as a move; Rust had made it a splitter. One mouse,
            // one gesture: any end ends it, or `reconcile` waits forever.
            let ended = end_gestures(&mut s);
            drop(s);
            if ended {
                save_now(app);
            }
            reconcile_later(app);
            return;
        };

        finish_drag(&mut s, &drag)
    };
    apply_ownership(&plan);
    emit_state(app);
    save_now(app);
    reconcile_later(app);
}

/// D191: may the gesture for press `seq` in window `id` start? Not once its
/// own end has been heard: a start overtaken by its end would leave a drag,
/// splitter or grip recorded that nothing ends, and `reconcile` waiting.
pub fn gesture_begins(app: &AppHandle, id: WindowId, seq: u64) -> bool {
    begins(&app.state::<Wm>().0.lock().unwrap(), id, seq)
}

/// D192: the gesture for press `seq` in window `id` has started, so it is
/// the live one. Only once it has: a grip Rust refuses is no gesture, and
/// taking the live slot for it left an older press's late end ignored.
pub fn gesture_started(app: &AppHandle, id: WindowId, seq: u64) {
    started(&mut app.state::<Wm>().0.lock().unwrap(), id, seq);
}

/// D191: the gesture for press `seq` in window `id` has ended. False when
/// another press's gesture is live, which this end must leave alone: it is
/// a late end, and its own gesture was ended when the next one began.
pub fn gesture_ends(app: &AppHandle, id: WindowId, seq: u64) -> bool {
    ends(&mut app.state::<Wm>().0.lock().unwrap(), id, seq)
}

fn begins(s: &WmState, id: WindowId, seq: u64) -> bool {
    !s.ended.get(&id).is_some_and(|e| e.contains(&seq))
}

fn started(s: &mut WmState, id: WindowId, seq: u64) {
    s.live = Some((id, seq));
}

/// D192: the gesture for press `seq` has started if Rust accepted it, and
/// only then is it the live one.
pub fn gesture_started_if(app: &AppHandle, id: WindowId, seq: u64, accepted: bool) {
    start_if(&mut app.state::<Wm>().0.lock().unwrap(), id, seq, accepted);
}

fn start_if(s: &mut WmState, id: WindowId, seq: u64, accepted: bool) {
    if accepted {
        started(s, id, seq);
    }
}

fn ends(s: &mut WmState, id: WindowId, seq: u64) -> bool {
    let e = s.ended.entry(id).or_default();
    e.push(seq);
    if e.len() > 8 {
        e.remove(0);
    }
    let current = s.live.is_none_or(|l| l == (id, seq));
    if current {
        s.live = None;
    }
    current
}

/// A drag released: the bonds it earned, the spans resynced, and the
/// ownership and z-order to apply, the grabbed window raised last within
/// its group (D193).
fn finish_drag(s: &mut WmState, drag: &DragState) -> OwnPlan {
    let others: Vec<WindowId> = CLASSIC
        .iter()
        .copied()
        .filter(|c| !drag.moving.contains(c))
        .collect();
    for b in bonds_on_release(&drag.origin_layout, &s.layout, &drag.moving, &others) {
        s.graph.insert(b);
    }
    let layout = s.layout.clone();
    resync_spans(&mut s.graph, &layout);
    plan_ownership(s, Some(drag.grabbed))
}

/// D191: a gesture is starting, so any other one still recorded is stale:
/// one mouse, one gesture. Without this, a splitter whose end never came
/// (its start answered after the release) kept `reconcile` waiting for good.
fn begin_gesture(s: &mut WmState) {
    end_gestures(s);
    // A layout the person is making: a window chased to the limit before it
    // is followed again (D190's "until the next drag", for every gesture).
    s.reconciles.clear();
}

/// D191: end every gesture recorded; whether one was. Any end ends them
/// all, so an end sent for the wrong kind (a seam answered as a splitter
/// after it was ended as a move) still ends what Rust started.
fn end_gestures(s: &mut WmState) -> bool {
    s.drag.take().is_some() | s.splitter.take().is_some() | s.resize.take().is_some()
}

/// D191: whether a `reconcile` is queued on the UI thread and not yet run.
static RECONCILE_QUEUED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// D191: run `reconcile` on the UI thread, once, after whatever the UI
/// thread is doing now. Every placement is made there (the drag, splitter
/// and grip commands are synchronous, so they run there too), so a reconcile
/// queued here cannot interleave with a drag frame or a re-zoom; D190 ran it
/// on a thread of its own per event, and two of them, pushing out of order,
/// left the playlist where an older layout put it. Posted from a thread of
/// its own because tauri runs a closure inline when it is handed one on the
/// UI thread, and `ScaleFactorChanged` is heard there before tao's own
/// resize for the new scale, which the reconcile's placement must follow.
/// Any number of requests before it runs are one reconcile.
fn queue_reconcile(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    if RECONCILE_QUEUED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let ui = app.clone();
        let queued = app.run_on_main_thread(move || {
            RECONCILE_QUEUED.store(false, Ordering::SeqCst);
            reconcile(&ui);
        });
        if queued.is_err() {
            RECONCILE_QUEUED.store(false, Ordering::SeqCst);
        }
    });
}

/// D191: a reconcile a moment from now, once a change of scale a placement
/// caused has had time to land, or not: Windows keeping a window where it
/// was sends nothing to hear. After every gesture and re-zoom.
fn reconcile_later(app: &AppHandle) {
    if platform::platform().one_scale() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        queue_reconcile(&app);
    });
}

/// D190: a window changed scale. Windows makes the change after the call
/// that caused it has returned, and a drop is the last push of a drag: the
/// owner's playlist went to 100 % on the drop and the engine kept it at
/// 150 %, and a stack dragged 48 px up on the seam went to 100 % entire,
/// with gaps of 58 px between. So the change is heard as it lands and the
/// layout follows it (D191: queued on the UI thread). Nothing here but the
/// queueing: this can be heard in the middle of a placement.
pub fn scale_changed(app: &AppHandle) {
    if !platform::platform().one_scale() {
        queue_reconcile(app);
    }
}

/// D190: lay the windows out again for the scales the window system draws
/// them at, where those are not the ones their sizes were derived at, and
/// (D191) put every window whose rect is not the layout's where the layout
/// has it, whatever moved it: tao's resize for a new scale, or a placement
/// made before the scale settled. During a gesture it does nothing; each
/// ends with one. A minimised window is left until it is restored, which
/// brings another. Runs on the UI thread only (`queue_reconcile`).
fn reconcile(app: &AppHandle) {
    if platform::platform().one_scale() {
        return;
    }
    let now = os_scales(app, &CLASSIC);
    let handles: Vec<(WindowId, NativeWindow)> = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        CLASSIC.iter().map(|id| (*id, s.handle(*id))).collect()
    }; // D54: no OS call under the lock.
    let p = platform::platform();
    let iconic: BTreeSet<WindowId> = handles
        .iter()
        .filter(|(_, h)| !h.is_none() && p.is_minimized(*h))
        .map(|(id, _)| *id)
        .collect();
    let (layout, followed) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        if s.drag.is_some() || s.splitter.is_some() || s.resize.is_some() {
            return;
        }
        let followed = follow_scales(
            &mut s,
            &now,
            &iconic,
            std::time::Instant::now(),
            true,
            p.confines_to_work_area(),
        );
        (s.layout.clone(), followed)
    }; // D54: the lock is gone before any window call.
    let mut placed = false;
    for (id, h) in handles {
        if followed.left.contains(&id) || h.is_none() {
            continue;
        }
        let (Some(r), Some(win)) = (layout.get(&id), app.get_webview_window(label_of(id))) else {
            continue;
        };
        let Some((pos, size)) = platform::rect_of(&win) else {
            continue;
        };
        if (pos.x, pos.y, size.width as Px, size.height as Px) == (r.x, r.y, r.w, r.h) {
            continue;
        }
        placed = true;
        if !p.place(h, r.x, r.y, r.w, r.h) {
            push_to_os(app, &layout, &[id]);
        }
    }
    if followed.rescaled || placed {
        emit_state(app);
    }
    if followed.rescaled {
        save_now(app);
    }
}

/// What `follow_scales` did: whether it re-derived, and the windows the
/// placement after it must leave where they are.
#[derive(Debug, Default, PartialEq)]
struct Followed {
    rescaled: bool,
    left: BTreeSet<WindowId>,
}

/// D190, D191: the layout re-derived for the scales the window system draws
/// the windows at (`now`), where those are not the ones their sizes are in:
/// each such window from the one to the other, the playlist keeping its
/// steps, and each group re-packed from its top-left window so seams close.
/// No guessing: the window system has already said. A group the re-derive
/// grew off the displays is lifted back on, as a re-zoom's is (D189); one it
/// did not grow, or did not touch, stays where the person put it.
///
/// A window minimised (`iconic`) is left alone. A window Windows keeps
/// flipping, which a size near the seam can make it do, is followed six
/// times in two seconds and then left as it is until the next gesture or
/// re-zoom, while the others are still followed and placed. `per_display`
/// is whether each display has its own scale, and `confine` whether the
/// window manager keeps windows wholly in the work area.
fn follow_scales(
    s: &mut WmState,
    now: &BTreeMap<WindowId, f64>,
    iconic: &BTreeSet<WindowId>,
    t: std::time::Instant,
    per_display: bool,
    confine: bool,
) -> Followed {
    let from = laid_out_at_in(&s.drawn_at, now, per_display);
    let off: BTreeSet<WindowId> = from
        .iter()
        .filter(|(id, f)| !iconic.contains(id) && now.get(id).is_some_and(|n| !same_scale(**f, *n)))
        .map(|(id, _)| *id)
        .collect();
    let mut flipping = BTreeSet::new();
    for id in &off {
        let times = s.reconciles.entry(*id).or_default();
        times.retain(|at| t.duration_since(*at) < std::time::Duration::from_secs(2));
        if times.len() >= 6 {
            flipping.insert(*id);
        }
    }
    if !flipping.is_empty() {
        eprintln!("wm: a window keeps changing scale; the layout follows it no further");
    }
    let follow: BTreeSet<WindowId> = off.difference(&flipping).copied().collect();
    let left: BTreeSet<WindowId> = iconic.union(&flipping).copied().collect();
    if follow.is_empty() {
        return Followed {
            rescaled: false,
            left,
        };
    }
    for id in &follow {
        s.reconciles.entry(*id).or_default().push(t);
    }
    let zoom = s.zoom();
    let pair = |id: WindowId| {
        let f = from.get(&id).copied().unwrap_or(s.scale);
        if follow.contains(&id) {
            (f, now.get(&id).copied().unwrap_or(f))
        } else {
            (f, f)
        }
    };
    let (layout, graph, unshaded) = rederive_whole(
        s,
        zoom,
        |l, g| {
            rederive_layout(l, g, &|id, _| pair(id), (zoom, zoom), &|r: &Rect| {
                (r.x, r.y)
            })
        },
        &|id, _| pair(id).1,
        confine,
    );
    let before = s.layout.clone();
    let grew = |comp: &[WindowId]| {
        comp.iter().any(|id| follow.contains(id))
            && match (bond::bounds(&before, comp), bond::bounds(&layout, comp)) {
                (Some(a), Some(b)) => b.w > a.w || b.h > a.h,
                _ => false,
            }
    };
    let layout = lift_onto_displays(&before, &layout, &graph, &s.monitors, &grew);
    s.unshaded_h.extend(unshaded);
    s.graph = graph;
    s.layout = layout;
    for id in &follow {
        if let Some(n) = now.get(id) {
            s.drawn_at.insert(*id, *n);
        }
    }
    Followed {
        rescaled: true,
        left,
    }
}

/// The bonds a finished drag has earned. None for a drag that never moved: a
/// click on a title bar is a zero-length drag, and a window demagnetised a
/// moment ago is still flush with its old neighbour, so bonding on the click
/// would undo the double-click before it (#10, D85). The gesture table says
/// "drag a window near another"; a click is not a drag.
pub fn bonds_on_release(
    origin: &Layout,
    layout: &Layout,
    moving: &[WindowId],
    others: &[WindowId],
) -> Vec<Bond> {
    if layout == origin {
        return Vec::new();
    }
    bonds_after_drag(layout, moving, others)
}

// ---- seams ------------------------------------------------------------------

/// Which of a window's four edges carry a bond, and whether each one is a live
/// splitter.
///
/// `None` means no bond on that edge. `Some(false)` means bonded but inert as a
/// splitter, which per D35 makes it a move handle rather than something that
/// offers a resize and then refuses to perform one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Edges {
    pub top: Option<bool>,
    pub right: Option<bool>,
    pub bottom: Option<bool>,
    pub left: Option<bool>,
}

/// D35: the cursor tells the truth.
///
/// Bonds are stored canonically — `a` is always the left or top window — so a
/// bond's edge is read from whichever end of it this window sits on.
pub fn edges_for(state: &WmState, id: WindowId) -> Edges {
    let mut e = Edges::default();
    for b in &state.graph.bonds {
        if !b.touches(id) {
            continue;
        }
        let live = Some(bond::splitter_is_live(is_resizable(b.a), is_resizable(b.b)));
        match (b.edge, b.a == id) {
            (Edge::Right, true) => e.right = live,
            (Edge::Right, false) => e.left = live,
            (Edge::Bottom, true) => e.bottom = live,
            (Edge::Bottom, false) => e.top = live,
            // A non-canonical bond cannot reach here: Bond::new normalises
            // every Left/Top into its mirror image on construction.
            _ => {}
        }
    }
    e
}

/// Push every classic window its whole view of the world: seams, focus, shade.
///
/// One event rather than three. They all derive from the same locked state, so
/// splitting them into separate messages only creates opportunities for a
/// window to hold two of them from different moments.
pub fn emit_state(app: &AppHandle) {
    let all: Vec<(WindowId, Hello)> = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        let focused = s.focused;
        let flags = focus_plan(&s, focused);
        CLASSIC
            .iter()
            .map(|id| {
                (
                    *id,
                    Hello {
                        edges: edges_for(&s, *id),
                        active: flags.iter().any(|(w, a)| w == id && *a),
                        shaded: s.shaded.contains(id),
                        double: s.double,
                    },
                )
            })
            .collect()
    };
    for (id, h) in all {
        let _ = app.emit_to(label_of(id), "wm:state", h);
    }
    // Every gesture that ends comes through here: a bond made or broken, a
    // shade, the zoom. The pipe hears it too (#181); a move or resize alone is
    // heard from the OS's own window events.
    crate::layout::ping(app);
}

/// D30: quantise a seam position so the resizable neighbour lands on a legal
/// size.
///
/// Every valid playlist size is `275 + 25n` wide by `116 + 29m` tall, so the
/// seam cannot stop wherever the cursor happens to be — it has to jump. The
/// step count is derived from the raw position and the size is then recomputed
/// **from the logical base** (D40), never by adding a rounded physical step to
/// a previous physical value: stepping by a rounded increment drifts 5 px after
/// ten steps and 20 px after forty, which walks the bonded neighbour out of
/// flush a little more every time.
pub fn quantize_seam(layout: &Layout, b: &Bond, raw: Px, scale: f64, zoom: f64) -> Px {
    let vertical = b.edge.is_vertical_seam();
    let (base, step) = if vertical {
        (CHROME_W * zoom, PLAYLIST_STEP_W * zoom)
    } else {
        (CHROME_H * zoom, PLAYLIST_STEP_H * zoom)
    };
    let (Some(ra), Some(rb)) = (layout.get(&b.a).copied(), layout.get(&b.b).copied()) else {
        return raw;
    };

    // Quantise against whichever side actually resizes. The fixed side keeps
    // its size and slides, so it constrains nothing.
    // The resizable side's size is measured from its own far edge, which does
    // not move. `sign` carries which direction that measurement runs in, so the
    // two cases share one expression instead of duplicating the rounding.
    let (fixed_edge, sign) = if is_resizable(b.b) {
        // b grows leftward/upward from its far edge: size = fixed_edge - pos.
        (if vertical { rb.right() } else { rb.bottom() }, 1)
    } else if is_resizable(b.a) {
        // a grows rightward/downward from its near edge: size = pos - fixed_edge.
        (if vertical { ra.x } else { ra.y }, -1)
    } else {
        return raw;
    };

    // Steps of the resizable side implied by where the cursor is, rounded to
    // the nearest legal size and never below the base.
    let span = ((fixed_edge - raw) * sign) as f64;
    let n = (((span / scale) - base) / step).round().max(0.0) as i32;
    fixed_edge - sign * bond::d40::stepped(base, step, n, scale)
}

/// A splitter drag in flight.
#[derive(Clone, Debug)]
pub struct SplitterState {
    pub bond: Bond,
    pub origin_layout: Layout,
}

/// Begin a splitter drag on one of `id`'s edges.
///
/// Returns false when that edge is not a live splitter, which is the caller's
/// signal to treat the gesture as a group move instead (D35).
pub fn splitter_start(app: &AppHandle, id: WindowId, edge: Edge) -> bool {
    let state = app.state::<Wm>();
    let mut s = state.0.lock().unwrap();
    let Some(b) = seam_on(&s, id, edge) else {
        return false;
    };
    if !bond::splitter_is_live(is_resizable(b.a), is_resizable(b.b)) {
        return false;
    }
    begin_gesture(&mut s);
    s.splitter = Some(SplitterState {
        bond: b,
        origin_layout: s.layout.clone(),
    });
    true
}

/// The bond sitting on a given edge of a window, in canonical form.
fn seam_on(state: &WmState, id: WindowId, edge: Edge) -> Option<Bond> {
    state
        .graph
        .bonds
        .iter()
        .find(|b| match (b.edge, b.a == id, edge) {
            (Edge::Right, true, Edge::Right) => true,
            (Edge::Right, false, Edge::Left) => b.b == id,
            (Edge::Bottom, true, Edge::Bottom) => true,
            (Edge::Bottom, false, Edge::Top) => b.b == id,
            _ => false,
        })
        .copied()
}

/// One splitter frame.
pub fn splitter_move(app: &AppHandle) {
    let cursor = platform::platform().cursor_pos();

    let (layout, touched) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        let Some(sp) = s.splitter.clone() else {
            return;
        };

        // D191: the seam moves in the steps of the window it resizes, at the
        // scale that window's size is in.
        let sized = if is_resizable(sp.bond.b) {
            sp.bond.b
        } else {
            sp.bond.a
        };
        let scale = derived_scale(
            &s,
            sized,
            scale_at(&s.monitors, cursor.0, cursor.1, s.scale),
        );
        let vertical = sp.bond.edge.is_vertical_seam();
        let raw = if vertical { cursor.0 } else { cursor.1 };

        // D40 again: recompute from the origin layout every frame. Applying the
        // splitter to the *current* layout would compound its own rounding.
        let mut layout = sp.origin_layout.clone();
        let zoom = s.zoom();
        let pos = quantize_seam(&layout, &sp.bond, raw, scale, zoom);
        let min = if vertical {
            bond::d40::physical(CHROME_W * zoom, scale)
        } else {
            bond::d40::physical(CHROME_H * zoom, scale)
        };
        bond::apply_splitter_in_graph(&mut layout, &s.graph, &sp.bond, pos, &is_resizable, min);
        let touched = s.graph.component(sp.bond.a);
        if grows_out(&s.layout, &layout, &touched, &s.monitors) {
            return;
        }
        s.layout = layout.clone();
        (layout, touched)
    };

    push_to_os(app, &layout, &touched);
}

pub fn splitter_end(app: &AppHandle) {
    {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        // D191: any end ends every gesture.
        end_gestures(&mut s);
    }
    save_now(app);
    reconcile_later(app);
}

// ---- corner grip --------------------------------------------------------------
//
// The classic playlist had a grip in its bottom-right corner, and without one a
// playlist that is not bonded to anything cannot be resized at all: the only
// other way to change its size is a seam it shares with a neighbour. The grip
// resizes the free edges. An edge that carries a bond belongs to that seam and
// keeps its place, so the grip never opens a gap in a group.

/// A corner-grip resize in flight.
#[derive(Clone, Debug)]
pub struct ResizeState {
    pub id: WindowId,
    pub origin: Rect,
    pub origin_cursor: (Px, Px),
    /// The right edge is free (no bond), so the width may change.
    pub w_free: bool,
    /// The bottom edge is free, so the height may change.
    pub h_free: bool,
}

/// One frame of a grip resize, as pure geometry.
///
/// D40 twice over: origin plus the total delta, never the current size plus a
/// frame's worth; and the size comes fresh from the logical base and step on
/// the D30 grid, rounded once to physical. The top-left corner stays put.
pub fn resize_frame(
    origin: Rect,
    delta: (Px, Px),
    w_free: bool,
    h_free: bool,
    scale: f64,
    zoom: f64,
) -> Rect {
    let grid = |px: Px, base: f64, step: f64| {
        let n = (((px as f64 / scale) - base * zoom) / (step * zoom))
            .round()
            .max(0.0) as i32;
        bond::d40::stepped(base * zoom, step * zoom, n, scale)
    };
    let w = if w_free {
        grid(origin.w + delta.0, CHROME_W, PLAYLIST_STEP_W)
    } else {
        origin.w
    };
    let h = if h_free {
        grid(origin.h + delta.1, CHROME_H, PLAYLIST_STEP_H)
    } else {
        origin.h
    };
    Rect::new(origin.x, origin.y, w, h)
}

/// Begin a grip resize. False when the window cannot resize, is shaded, or has
/// both edges bonded, which is the caller's signal to do nothing.
pub fn resize_start(app: &AppHandle, id: WindowId) -> bool {
    let state = app.state::<Wm>();
    let mut s = state.0.lock().unwrap();
    if !is_resizable(id) || s.shaded.contains(&id) {
        return false;
    }
    let Some(origin) = s.layout.get(&id).copied() else {
        return false;
    };
    let w_free = seam_on(&s, id, Edge::Right).is_none();
    let h_free = seam_on(&s, id, Edge::Bottom).is_none();
    if !w_free && !h_free {
        return false;
    }
    // D54: the cursor read is a Win32 call, but not one aimed at another
    // thread's window, so it is safe under the lock.
    let origin_cursor = platform::platform().cursor_pos();
    begin_gesture(&mut s);
    s.resize = Some(ResizeState {
        id,
        origin,
        origin_cursor,
        w_free,
        h_free,
    });
    true
}

/// One grip frame.
pub fn resize_move(app: &AppHandle) {
    let cursor = platform::platform().cursor_pos();
    let (layout, id) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        let Some(r) = s.resize.clone() else {
            return;
        };
        // D51: rendered geometry resolves from the window's own monitor;
        // D191: the scale its size is in, where each display has its own.
        let scale = derived_scale(
            &s,
            r.id,
            scale_at(&s.monitors, r.origin.x, r.origin.y, s.scale),
        );
        let delta = (cursor.0 - r.origin_cursor.0, cursor.1 - r.origin_cursor.1);
        let rect = resize_frame(r.origin, delta, r.w_free, r.h_free, scale, s.zoom());
        let mut layout = s.layout.clone();
        layout.insert(r.id, rect);
        if grows_out(&s.layout, &layout, &s.graph.component(r.id), &s.monitors) {
            return;
        }
        s.layout = layout;
        (s.layout.clone(), r.id)
    };
    push_to_os(app, &layout, &[id]);
}

/// Release the grip: the bonds on the edges that stayed put may now span a
/// different length of seam, and the size is worth keeping.
pub fn resize_end(app: &AppHandle) {
    {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        // D191: any end ends every gesture.
        let Some(_) = s.resize.take() else {
            let ended = end_gestures(&mut s);
            drop(s);
            if ended {
                save_now(app);
            }
            reconcile_later(app);
            return;
        };
        end_gestures(&mut s);
        let layout = s.layout.clone();
        resync_spans(&mut s.graph, &layout);
    }
    emit_state(app);
    save_now(app);
    reconcile_later(app);
}

/// Double-click on a seam: demagnetize.
///
/// Breaking one bond in the middle of a chain has to split one group into two,
/// which is why the model is a real graph and not a flat list of groups — the
/// components are recomputed, and each side gets its own hidden root (D41).
/// The forcing raise afterwards is D42: ownership is applied lazily, so without
/// it the z-order stays stale-but-plausible until the next click.
pub fn demagnetize(app: &AppHandle, id: WindowId, edge: Edge) -> bool {
    let (broke, plan) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        let Some(b) = seam_on(&s, id, edge) else {
            return false;
        };
        let broke = s.graph.break_bond(b.a, b.b);
        let plan = plan_ownership(&s, Some(id));
        (broke, plan)
    };
    if broke {
        apply_ownership(&plan);
        emit_state(app);
        save_now(app);
    }
    broke
}

// ---- windowshade -------------------------------------------------------------

/// Collapse or expand a window in place, keeping the group flush.
///
/// The window's **top-left stays put** and only its height changes; everything
/// bonded below it slides by the difference. That is what makes a shade feel
/// like a collapse rather than a re-layout — the thing you clicked does not
/// move, and neither does anything above it.
///
/// `side_of` is what makes this safe in a group of any shape: it walks the
/// graph from the far end of the bottom seam *without crossing that seam*, so
/// exactly the rigid body below moves, however many windows are in it and
/// whatever else they are bonded to.
pub fn apply_shade(layout: &mut Layout, graph: &WindowGraph, id: WindowId, new_h: Px) {
    let Some(r) = layout.get(&id).copied() else {
        return;
    };
    let delta = new_h - r.h;
    if delta == 0 {
        return;
    }

    // Take the bottom seam before resizing, while the geometry still agrees
    // with the graph.
    let below = graph
        .bonds
        .iter()
        .find(|b| b.edge == Edge::Bottom && b.a == id)
        .map(|b| {
            let mut side = bond::side_of(graph, b, b.b);
            side.retain(|s| *s != id);
            side
        })
        .unwrap_or_default();

    layout.insert(id, Rect { h: new_h, ..r });
    bond::translate_group(layout, &below, 0, delta);
}

/// The height a window should have right now, in physical px.
///
/// D51: this is *rendered geometry*, so it resolves from the window's own
/// monitor — the 14 px strip really is physically taller on a 150% display.
/// D40: recomputed from the logical constant, never scaled from the height the
/// window happens to have.
pub fn height_for(state: &WmState, id: WindowId, shaded: bool) -> Px {
    let corner = state
        .layout
        .get(&id)
        .and_then(|r| monitor_at(&state.monitors, r.x, r.y))
        .map(|m| m.scale)
        .unwrap_or(state.scale);
    // D191: the scale the window's size is in, where each display has its
    // own; the strip under a top-left corner on the other display was not.
    let scale = derived_scale(state, id, corner);
    if shaded {
        bond::d40::physical(SHADE_H * state.zoom(), scale)
    } else {
        // The playlist can be any legal D30 size, so expanding restores the
        // height it had before it was collapsed rather than the base height.
        // Losing a resized playlist to a double-click would be a data loss the
        // user did not ask for.
        state
            .unshaded_h
            .get(&id)
            .copied()
            .unwrap_or_else(|| bond::d40::physical(CHROME_H * state.zoom(), scale))
    }
}

/// D61: which windows should be topmost.
///
/// Shading Main makes Main's whole connected component float. Pure so the rule
/// is testable without a window: the interesting part is that it follows the
/// *group*, not the window.
pub fn topmost_set(state: &WmState) -> Vec<WindowId> {
    if state.shaded.contains(&MAIN) {
        state.graph.component(MAIN)
    } else {
        vec![]
    }
}

/// Toggle windowshade on one window (D60).
pub fn toggle_shade(app: &AppHandle, id: WindowId) {
    let (layout, moved, topmost) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();

        let shaded = !s.shaded.contains(&id);
        if shaded {
            // Remember the height to come back to before it is overwritten.
            if let Some(h) = s.layout.get(&id).map(|r| r.h) {
                s.unshaded_h.insert(id, h);
            }
            s.shaded.insert(id);
        } else {
            s.shaded.remove(&id);
        }

        let h = height_for(&s, id, shaded);
        let graph = s.graph.clone();
        let mut layout = s.layout.clone();
        apply_shade(&mut layout, &graph, id, h);

        // Everything in the component may have moved, plus the window itself.
        let moved = s.graph.component(id);
        // D182: a group that unshades past the work area's edge comes back
        // inside it whole, rather than the window manager pushing one window.
        if platform::platform().confines_to_work_area() {
            let (dx, dy) = confine_clamp(&layout, &moved, &s.monitors);
            bond::translate_group(&mut layout, &moved, dx, dy);
        }
        s.layout = layout.clone();
        (layout, moved, topmost_set(&s))
    }; // D54: lock dropped before any OS call.

    push_to_os(app, &layout, &moved);

    let p = platform::platform();
    let handles: Vec<(NativeWindow, bool)> = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        CLASSIC
            .iter()
            .map(|id| (s.handle(*id), topmost.contains(id)))
            .collect()
    };
    for (w, on) in handles {
        p.set_topmost(w, on);
    }

    emit_state(app);
    save_now(app);
}

// ---- focus ------------------------------------------------------------------

/// Focus is a group property: when any bonded window has focus, all of them
/// render active. Getting this wrong looks broken immediately.
///
/// Returns the per-window flags so the caller can emit them once the lock is
/// gone.
pub fn focus_plan(state: &WmState, focused: Option<WindowId>) -> Vec<(WindowId, bool)> {
    let group = focused
        .map(|f| state.graph.component(f))
        .unwrap_or_default();
    CLASSIC.iter().map(|id| (*id, group.contains(id))).collect()
}

/// What a window needs to know the moment it mounts.
///
/// Both of these are pushed as events when they change, but a window that is
/// still loading cannot receive a push — and the first `emit_edges` happens
/// during setup, before any webview has subscribed. That is not a race to be
/// tightened; it is a missing pull. So every window asks once on mount and
/// listens for changes after that.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Hello {
    pub edges: Edges,
    pub active: bool,
    pub shaded: bool,
    /// 2x chrome (#47), so the title bar's toggle shows the right label.
    pub double: bool,
}

pub fn hello(app: &AppHandle, id: WindowId) -> Hello {
    let state = app.state::<Wm>();
    let s = state.0.lock().unwrap();
    let focused = s.focused;
    Hello {
        edges: edges_for(&s, id),
        active: focus_plan(&s, focused)
            .into_iter()
            .find(|(w, _)| *w == id)
            .map(|(_, a)| a)
            .unwrap_or(false),
        shaded: s.shaded.contains(&id),
        double: s.double,
    }
}

/// A window was clicked or focused: raise its whole group (D42) and tell every
/// classic window whether it should render active.
pub fn focus_group(app: &AppHandle, focused: Option<WindowId>) {
    let (plan, flags) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        s.focused = focused;
        (plan_ownership(&s, focused), focus_plan(&s, focused))
    };
    // Only a genuine focus gain reorders anything. On focus *loss* the app is
    // no longer foreground, so raising would be shoving our windows up through
    // somebody else's stack for no reason — and D56 says it would not reach the
    // top anyway.
    if focused.is_some() {
        apply_ownership(&plan);
    }
    let _ = flags;
    emit_state(app);
}

/// Bring a window's whole group to the front of our own stack without taking
/// focus. The library asked Main to play something and the user wants to see
/// it happen, but they are still working in the library, so activating Main
/// would be rude. Leaves `focused` alone: the chrome should render active
/// only where the OS focus actually is.
///
/// **Never out of the taskbar** (#191, D152). It used to restore a minimised
/// member first, and Main called it on every track it loaded, so a group the
/// person had minimised came back up with every new song. Now a group that is
/// down, by Main's minimise (D86) or any other way, stays down: nothing is
/// raised, and the music plays on.
pub fn raise_group(app: &AppHandle, id: WindowId) {
    let (plan, handles, down) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        let comp = s.graph.component(id);
        (
            plan_ownership(&s, Some(id)),
            comp.iter().map(|w| s.handle(*w)).collect::<Vec<_>>(),
            s.minimized,
        )
    }; // D54: the lock is gone before any Win32 call.
    let p = platform::platform();
    if down || handles.iter().any(|w| !w.is_none() && p.is_minimized(*w)) {
        return;
    }
    apply_ownership(&plan);
}

/// Whether the person has the player down in the taskbar (#191): Main's
/// minimise (D86), or Main minimised any other way (its own taskbar button,
/// Win+D). A video that comes up in the queue then goes to the taskbar too.
pub fn player_down(app: &AppHandle) -> bool {
    let (flag, main) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        (s.minimized, s.handle(MAIN))
    }; // D54: the lock is gone before the OS is asked.
    flag || (!main.is_none() && platform::platform().is_minimized(main))
}

// ---- minimise (#86) ---------------------------------------------------------

/// Main's minimise button (D86). Main speaks for the group, so the whole
/// connected component goes to the taskbar behind Main's one button; the
/// satellites have none (D59), which is why only Main offers the gesture.
/// Satellites first, Main last, so the taskbar animation is Main's. The way
/// back is `watch_restore`: when the taskbar button (or Alt+Tab) brings Main
/// up, the satellites come with it.
pub fn minimize_group(app: &AppHandle) {
    let members = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        s.minimized = true;
        minimize_plan(&s)
    }; // D54: the lock is gone before any window is touched.
       // D182: where a window that skips the taskbar cannot be minimised
       // (Mutter), the satellites are hidden instead, and come back with Main.
    let hide = !platform::platform().minimises_taskbarless();
    for id in members {
        if let Some(w) = app.get_webview_window(label_of(id)) {
            let _ = if hide && id != MAIN {
                w.hide()
            } else {
                w.minimize()
            };
        }
    }
    watch_restore(app);
}

/// While the group is down, watch for Main coming back and bring the
/// satellites with it. Polled, like the display watchdog (D62), and not
/// driven by focus: minimising a satellite makes Windows activate whatever is
/// next, and that arrives as a focus gain for Main before Main has even gone,
/// which restored the whole group in the same event-loop turn that minimised
/// it. 100 ms is well under the taskbar's own restore animation.
fn watch_restore(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let down = app.state::<Wm>().0.lock().unwrap().minimized;
            if !down {
                break;
            }
            let handle = app.clone();
            if app
                .run_on_main_thread(move || restore_if_back(&handle))
                .is_err()
            {
                break; // app is shutting down
            }
        }
    });
}

/// One poll of the watch: once Main is up again, restore the rest of its group
/// without activating anything (Main already has the focus the OS gave it),
/// and clear the flag so the watch and the display watchdog both stand down.
fn restore_if_back(app: &AppHandle) {
    let (main, members) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        if !s.minimized {
            return;
        }
        let members = s
            .graph
            .component(MAIN)
            .iter()
            .map(|w| s.handle(*w))
            .collect::<Vec<_>>();
        (s.handle(MAIN), members)
    }; // D54: the lock is gone before is_minimized touches a window.
    let p = platform::platform();
    if main.is_none() || p.is_minimized(main) {
        return;
    }
    for w in members
        .iter()
        .filter(|w| !w.is_none() && p.is_minimized(**w))
    {
        p.restore_no_activate(*w);
    }
    // D182: and the satellites `minimize_group` hid, where it could not
    // minimise them. Main is already up, so this shows them behind it.
    if !p.minimises_taskbarless() {
        show_hidden(app);
    }
    app.state::<Wm>().0.lock().unwrap().minimized = false;
    // D191: a reconcile the minimise turned away is owed now.
    reconcile_later(app);
}

/// Show the classic windows `minimize_group` hid (D182), where the model has
/// them. A window manager places a window that comes back from hidden as if
/// it were new (Mutter put the EQ and playlist wherever it liked, found by
/// hand), so the model's rects are pushed again until the OS reads them back,
/// as `confirm_layout` waits at startup. Off the main thread, which has to
/// keep turning for the windows to map.
fn show_hidden(app: &AppHandle) {
    let shown: Vec<WindowId> = CLASSIC
        .iter()
        .copied()
        .filter(|id| {
            app.get_webview_window(label_of(*id))
                .is_some_and(|w| !w.is_visible().unwrap_or(true) && w.show().is_ok())
        })
        .collect();
    settle(app, shown);
}

/// D182: hide the windows and show them again, then settle them where the
/// model has them. After X's one scale changes, Mutter refuses every resize of
/// a window mapped before the change, its own size hints and a direct
/// `XResizeWindow` alike, until the window is mapped afresh (found by hand,
/// GNOME 50). A flicker on a rare event: a display coming or going, or the
/// scale chosen in the settings. Called on the main thread; the show waits a
/// beat so GTK does not fold the hide and the show into nothing.
fn remap(app: &AppHandle, ids: Vec<WindowId>) {
    let visible: Vec<WindowId> = ids
        .into_iter()
        .filter(|id| {
            app.get_webview_window(label_of(*id))
                .is_some_and(|w| w.is_visible().unwrap_or(false) && w.hide().is_ok())
        })
        .collect();
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let shown = app.clone();
        let ids = visible.clone();
        let _ = app.run_on_main_thread(move || {
            for id in &ids {
                if let Some(w) = shown.get_webview_window(label_of(*id)) {
                    let _ = w.show();
                }
            }
        });
        settle_now(&app, &visible);
        // And a resize the webview sees: after the remap it is still laid out
        // for the old scale (the playlist drew its contents at twice its
        // size, found by hand) until its window's size changes. Two pixels
        // wider and back.
        let layout = app.state::<Wm>().0.lock().unwrap().layout.clone();
        let mut nudged = layout.clone();
        for id in &visible {
            if let Some(r) = nudged.get_mut(id) {
                r.w += 2;
            }
        }
        push_to_os(&app, &nudged, &visible);
        std::thread::sleep(std::time::Duration::from_millis(150));
        push_to_os(&app, &layout, &visible);
    });
}

/// Push the model's rects for `ids` until the OS reads them back (D182), off
/// the main thread, which has to keep turning for the answers to arrive. For
/// the moments a window manager is still placing windows of its own accord: a
/// window shown again after being hidden, which Mutter places as if it were
/// new. Gives up after two seconds, saying so.
fn settle(app: &AppHandle, ids: Vec<WindowId>) {
    if ids.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || settle_now(&app, &ids));
}

/// `settle`, on the calling thread, which must not be the main thread.
fn settle_now(app: &AppHandle, ids: &[WindowId]) {
    for _ in 0..40 {
        let (layout, handles) = {
            let state = app.state::<Wm>();
            let s = state.0.lock().unwrap();
            let handles: Vec<NativeWindow> = ids.iter().map(|id| s.handle(*id)).collect();
            (s.layout.clone(), handles)
        }; // D54: the lock is gone before the OS is asked.
        let off: Vec<WindowId> = ids
            .iter()
            .zip(handles)
            .filter(|(id, h)| {
                // X's answer where there is one: tao's reads back what it
                // asked for before the window manager has placed anything.
                let read = match platform::platform().placed(*h) {
                    Some((x, y, w, h)) => Some(Rect::new(x, y, w as Px, h as Px)),
                    None if h.is_none() => app
                        .get_webview_window(label_of(**id))
                        .and_then(|w| platform::rect_of(&w))
                        .map(|(p, s)| Rect::new(p.x, p.y, s.width as Px, s.height as Px)),
                    None => None,
                };
                read != layout.get(id).copied()
            })
            .map(|(id, _)| *id)
            .collect();
        if off.is_empty() {
            return;
        }
        push_to_os(app, &layout, &off);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    eprintln!("wm: the windows did not settle where the model has them");
}

/// The windows Main's minimise takes with it: its component, Main last.
pub fn minimize_plan(state: &WmState) -> Vec<WindowId> {
    let mut members: Vec<WindowId> = state
        .graph
        .component(MAIN)
        .into_iter()
        .filter(|id| *id != MAIN)
        .collect();
    members.sort();
    members.push(MAIN);
    members
}

// ---- chrome zoom (#47) ------------------------------------------------------

/// Settings key for the chrome zoom. "1" is 2x; anything else is 1x.
pub const DOUBLE_SETTING: &str = "chrome_double";

/// Re-derive a layout for a new chrome zoom.
///
/// Every size comes fresh from the logical base times the new zoom, rounded
/// once (D40); the playlist keeps its step count (D30). Each bonded group is
/// then re-packed by walking its bonds out from an anchor window whose
/// top-left corner stays put, so positions are physical arithmetic on the
/// rounded sizes and never a scaled copy of the old positions. The difference
/// is real: at 150% two 275-wide windows side by side are 413 + 413, and
/// scaling the second one's offset would put a third at round(550 x 1.5) =
/// 825, one pixel out of flush. Walking the bonds says 826. Spans are
/// recomputed from the result.
///
/// `layout` must be the expanded one, with no shaded heights in it: a shaded
/// strip is a state to reapply, not a size to scale. `set_double` expands
/// first and collapses after, the same way `save` does.
pub fn rezoom_layout(
    layout: &Layout,
    graph: &WindowGraph,
    monitors: &[MonitorInfo],
    fallback_scale: f64,
    old_zoom: f64,
    new_zoom: f64,
) -> (Layout, WindowGraph) {
    let scale_of = |_: WindowId, r: &Rect| {
        let s = monitor_at(monitors, r.x, r.y)
            .map(|m| m.scale)
            .unwrap_or(fallback_scale);
        (s, s)
    };
    rederive_layout(
        layout,
        graph,
        &scale_of,
        (old_zoom, new_zoom),
        &|r: &Rect| (r.x, r.y),
    )
}

/// The display a window takes its scale from: the one holding most of its
/// area, which is how Windows picks the DPI a window gets (D187), and the
/// nearest when it is on none.
pub fn dpi_monitor(monitors: &[MonitorInfo], r: Rect) -> Option<MonitorInfo> {
    let overlap = |m: &MonitorInfo| {
        let n = m.rect;
        let w = (r.right().min(n.right()) - r.x.max(n.x)).max(0) as i64;
        let h = (r.bottom().min(n.bottom()) - r.y.max(n.y)).max(0) as i64;
        w * h
    };
    // The first of the largest, as Windows picks on a tie (D193).
    let best = monitors.iter().map(overlap).max().filter(|a| *a > 0);
    best.and_then(|b| monitors.iter().find(|m| overlap(m) == b))
        .copied()
        .or_else(|| nearest_monitor(monitors, r))
}

/// D188: the scale a window is drawn at once put at `r`: the scale of the
/// display holding most of it, and on a tie the first display listed. D188
/// kept a window at the scale it was at on a tie, from one measurement
/// taken from 100 %; D193's probes found Windows giving the first display
/// on a tie from either scale. `current` answers only where there are no
/// displays to judge by.
pub fn judged_scale(monitors: &[MonitorInfo], r: Rect, current: f64) -> f64 {
    let overlap = |m: &MonitorInfo| {
        let n = m.rect;
        let w = (r.right().min(n.right()) - r.x.max(n.x)).max(0) as i64;
        let h = (r.bottom().min(n.bottom()) - r.y.max(n.y)).max(0) as i64;
        w * h
    };
    let Some(best) = monitors.iter().map(overlap).max().filter(|a| *a > 0) else {
        return nearest_monitor(monitors, r).map_or(current, |m| m.scale);
    };
    monitors
        .iter()
        .find(|m| overlap(m) == best)
        .map_or(current, |m| m.scale)
}

/// Two scales the same, as far as a window's size can tell.
pub fn same_scale(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// `r`, laid out at scale `to`, at the size it has at scale `from`: where
/// a window going to another scale is first put (D188).
pub fn sized_for(r: Rect, to: f64, from: f64) -> Rect {
    if same_scale(to, from) {
        return r;
    }
    let k = from / to;
    let at = |px: Px| (px as f64 * k).round() as Px;
    Rect::new(r.x, r.y, at(r.w), at(r.h))
}

/// D188: lay out `ids` at the scales they will be drawn at, settled against
/// where the layout puts them.
///
/// `derive` lays the windows out for a scale each; which scale each will be
/// drawn at depends on where that puts it, and a re-pack moves windows, so
/// the two are iterated until they agree. The owner's sweep left, a stack
/// 825 wide coming back onto 100 %: Main and the EQ went to 550 and the
/// re-pack lifted the playlist up beside them, mostly onto 100 %, while its
/// scale had been read from where it hung before, below the 100 % display's
/// bottom edge, so it stayed 825 for good. Each window is judged at its
/// current scale's size, as `push_settled` first puts it. `now` is the scale
/// each is drawn at. Returns the scales, and what `derive` made of them.
pub fn settle_scales<T>(
    ids: &[WindowId],
    now: &BTreeMap<WindowId, f64>,
    monitors: &[MonitorInfo],
    derive: impl Fn(&BTreeMap<WindowId, f64>) -> (Layout, T),
) -> (BTreeMap<WindowId, f64>, Layout, T) {
    let mut targets = now.clone();
    let (mut layout, mut extra) = derive(&targets);
    // Two windows can only take each other back and forth so many times; a
    // pass that has not settled by then is pushed as it is, and `reconcile`
    // has the last word.
    for _ in 0..4 {
        let mut next = targets.clone();
        for id in ids {
            let (Some(r), Some(cur)) = (layout.get(id), now.get(id)) else {
                continue;
            };
            next.insert(
                *id,
                judged_scale(monitors, sized_for(*r, targets[id], *cur), *cur),
            );
        }
        if next == targets {
            break;
        }
        targets = next;
        (layout, extra) = derive(&targets);
    }
    (targets, layout, extra)
}

/// D187: a saved layout whose windows are not the size their displays draw,
/// re-derived in place; None when every window already is. Where each display
/// has its own scale, a drag onto a second display saved the first display's
/// sizes there until D187, and a restart brought them back. Positions stay:
/// they are where the person put them. `layout` must be the expanded one, as
/// stored. Each window from the scale its saved size is in to the one it will
/// be drawn at, as `launch_scales` tells them; a window whose size's scale
/// cannot be told is left as it is.
pub fn heal_saved_sizes(
    layout: &Layout,
    graph: &WindowGraph,
    monitors: &[MonitorInfo],
    fallback: f64,
    zoom: f64,
    stored: &BTreeMap<WindowId, f64>,
    shaded: &BTreeSet<WindowId>,
) -> Option<(Layout, WindowGraph)> {
    let scales = launch_scales(layout, graph, monitors, fallback, zoom, stored, shaded);
    if scales
        .values()
        .all(|(saved, target)| saved.is_none_or(|s| same_scale(s, *target)))
    {
        return None;
    }
    Some(rederive_layout(
        layout,
        graph,
        &|id, r| match scales.get(&id) {
            Some((Some(saved), target)) => (*saved, *target),
            Some((None, target)) => (*target, *target),
            None => {
                let s = dpi_monitor(monitors, *r).map_or(fallback, |m| m.scale);
                (s, s)
            }
        },
        (zoom, zoom),
        &|r: &Rect| (r.x, r.y),
    ))
}

/// D191: for each restored window, the scale its saved size is in (None
/// where that cannot be told) and the scale it will be drawn at.
///
/// The scale a size is in: Main's and the EQ's widths say it exactly, as
/// they are never resized. The playlist's is the one saved beside it where
/// it was kept; in an older row, its own size where that is a size the
/// playlist can have at its display's scale and the rest of its group is
/// already the right size where it is (a group straddling the seam, Main at
/// 100 % and the playlist docked beside it at 150 %, healed the playlist
/// from Main's width to steps too big at every launch), its group's where
/// the group was saved wrong as a whole, and for a playlist alone the one
/// display scale its size fits. The scale it will be drawn at is
/// `launch_scale`'s from that, so a window saved on a tie stays as Windows
/// kept it.
pub fn launch_scales(
    layout: &Layout,
    graph: &WindowGraph,
    monitors: &[MonitorInfo],
    fallback: f64,
    zoom: f64,
    stored: &BTreeMap<WindowId, f64>,
    shaded: &BTreeSet<WindowId>,
) -> BTreeMap<WindowId, (Option<f64>, f64)> {
    // A size rounded once (D40) reads back a hair off its scale.
    let snap = |s: f64| {
        monitors
            .iter()
            .map(|m| m.scale)
            .find(|m| (m - s).abs() < 0.01)
            .unwrap_or(s)
    };
    let area = |r: &Rect| dpi_monitor(monitors, *r).map_or(fallback, |m| m.scale);
    let read = |id: WindowId| -> Option<f64> {
        let r = layout.get(&id)?;
        (!is_resizable(id) && r.w > 0).then(|| snap(r.w as f64 / (CHROME_W * zoom)))
    };
    // Sizes are read from the rows, which are stored expanded; the scale a
    // window will be drawn at is judged where it will be put, which for a
    // shaded window is its strip (D192): a shaded Main at 100 % whose
    // expanded rect would hang below DISPLAY1 onto DISPLAY2 was judged 150 %.
    let mut placed = layout.clone();
    for id in CLASSIC {
        if !shaded.contains(&id) {
            continue;
        }
        let Some(r) = layout.get(&id) else { continue };
        let at = read(id)
            .or_else(|| stored.get(&id).copied().map(snap))
            .unwrap_or_else(|| area(r));
        apply_shade(
            &mut placed,
            graph,
            id,
            bond::d40::physical(SHADE_H * zoom, at),
        );
    }
    let mut out = BTreeMap::new();
    for comp in graph.components(&CLASSIC) {
        let group = comp.iter().find_map(|id| read(*id));
        let group_right = comp.iter().all(|id| match (read(*id), placed.get(id)) {
            (Some(s), Some(r)) => same_scale(s, launch_scale(monitors, *r, Some(s), fallback)),
            _ => true,
        });
        for id in &comp {
            let (Some(r), Some(at)) = (layout.get(id), placed.get(id)) else {
                continue;
            };
            let saved = read(*id)
                .or_else(|| stored.get(id).copied().map(snap))
                .or_else(|| {
                    let fits = |scale: f64| playlist_fits(*r, scale, zoom);
                    let here = area(at);
                    match group {
                        Some(_) if group_right && fits(here) => Some(here),
                        Some(g) => Some(g),
                        None if fits(here) => Some(here),
                        None => {
                            let mut at: Vec<f64> = monitors
                                .iter()
                                .map(|m| m.scale)
                                .filter(|s| fits(*s))
                                .collect();
                            at.dedup_by(|a, b| same_scale(*a, *b));
                            (at.len() == 1).then(|| at[0])
                        }
                    }
                });
            let target = launch_scale(monitors, *at, saved, fallback);
            out.insert(*id, (saved, target));
        }
    }
    out
}

/// D191: the scale a restored window will be drawn at, as the launch judges
/// it: as Windows judges it (`judged_scale`), the first display of the
/// largest on a tie (D193). A stack left at 100 % split evenly on the seam
/// came back 150 % and grew, judged by the last display of the largest.
/// `saved`, the scale its size is in, answers only where there are no
/// displays to judge by; `fallback` where that cannot be told either.
pub fn launch_scale(monitors: &[MonitorInfo], r: Rect, saved: Option<f64>, fallback: f64) -> f64 {
    judged_scale(monitors, r, saved.unwrap_or(fallback))
}

/// Whether `r` is a size the playlist can have at `scale` and `zoom`: its
/// base and a whole number of steps each way, rounded once (D30, D40).
fn playlist_fits(r: Rect, scale: f64, zoom: f64) -> bool {
    let on = |px: Px, base: f64, step: f64| {
        let n = (((px as f64 / scale) - base * zoom) / (step * zoom))
            .round()
            .max(0.0) as i32;
        bond::d40::stepped(base * zoom, step * zoom, n, scale) == px
    };
    on(r.w, CHROME_W, PLAYLIST_STEP_W) && on(r.h, CHROME_H, PLAYLIST_STEP_H)
}

/// D187: re-derive a dragged group for the displays its windows are on now.
///
/// A drag recomputes every frame from where it began (D40), sizes included,
/// so a window that crossed onto a display at another scale was put back each
/// frame to the size it had on the first one, undoing the resize Windows
/// makes for the new DPI: a 275 x 116 window on a 150 % display, its chrome
/// drawn at 150 % and clipped. D52 described the re-derive and it was never
/// built; v0.0's stage 6 kept the windows at the primary's size, so nothing
/// caught it. Each moving window whose display's scale differs from the one
/// it started on is sized from the logical base for the new scale, the
/// playlist keeping its step count, and the group is re-packed by the walk
/// a re-zoom uses, its top-left window staying where the pointer put it, so
/// seams stay flush. Shaded windows are expanded first, from the heights the
/// drag began with, and collapsed again to the new scale's strip; their
/// unshaded heights in the new scale come back beside the layout. `scales`
/// is each moving window's (where it began, where it is now) scale, which
/// `settle_scales` works out (D188); every moving window has one. None when
/// no moving window has changed scale, which includes coming back.
#[allow(clippy::too_many_arguments)]
pub fn fit_to_displays(
    now: &Layout,
    moving: &[WindowId],
    graph: &WindowGraph,
    shaded: &BTreeSet<WindowId>,
    origin_unshaded: &BTreeMap<WindowId, Px>,
    scales: &BTreeMap<WindowId, (f64, f64)>,
    zoom: f64,
) -> Option<(Layout, BTreeMap<WindowId, Px>)> {
    let pairs: BTreeMap<WindowId, (f64, f64)> = moving
        .iter()
        .filter_map(|id| Some((*id, *scales.get(id)?)))
        .collect();
    if pairs.values().all(|(a, b)| same_scale(*a, *b)) {
        return None;
    }
    let mut sub: Layout = moving
        .iter()
        .filter_map(|id| now.get(id).map(|r| (*id, *r)))
        .collect();
    let mut subgraph = WindowGraph::new();
    for b in &graph.bonds {
        if sub.contains_key(&b.a) && sub.contains_key(&b.b) {
            subgraph.insert(*b);
        }
    }
    let folded: Vec<WindowId> = moving
        .iter()
        .copied()
        .filter(|id| shaded.contains(id) && sub.contains_key(id))
        .collect();
    for id in &folded {
        if let Some(h) = origin_unshaded.get(id) {
            apply_shade(&mut sub, &subgraph, *id, *h);
        }
    }
    let (mut out, _) = rederive_layout(
        &sub,
        &subgraph,
        &|id, _| pairs.get(&id).copied().unwrap_or((1.0, 1.0)),
        (zoom, zoom),
        &|r: &Rect| (r.x, r.y),
    );
    let mut unshaded = BTreeMap::new();
    for id in &folded {
        let Some(r) = out.get(id).copied() else {
            continue;
        };
        unshaded.insert(*id, r.h);
        let scale = pairs.get(id).map_or(1.0, |p| p.1);
        apply_shade(
            &mut out,
            &subgraph,
            *id,
            bond::d40::physical(SHADE_H * zoom, scale),
        );
    }
    let mut fitted = now.clone();
    fitted.extend(out);
    Some((fitted, unshaded))
}

/// D182: re-derive a layout for a new scale, where one scale covers the whole
/// desktop (X11) and a display coming or going, or a scale chosen in the
/// settings, has just changed it. Each window keeps its place on its own
/// display: its offset from the display's corner grows or shrinks as the
/// display's X rect did, which is nothing for one display taken from 100 % to
/// 200 % (X's screen stays the panel's pixels) and double when a 150 %
/// display joins a 100 % one (X's screen doubles). Displays are matched by
/// their order, which follows the connectors; a window whose display is gone
/// keeps its position for the rescue to bring back. Every size comes fresh
/// from the logical base at its display's new scale, the playlist keeping its
/// step count, and bonded groups are re-packed by the walk a re-zoom uses, so
/// seams stay flush however the rounding falls. `fallback` is the (old, new)
/// scale for a window on no display, and a restored layout passes no
/// displays at all. `layout` must be the expanded one, as for a re-zoom.
pub fn rescale_layout(
    layout: &Layout,
    graph: &WindowGraph,
    old: &[MonitorInfo],
    new: &[MonitorInfo],
    fallback: (f64, f64),
    zoom: f64,
) -> (Layout, WindowGraph) {
    let pair = |r: &Rect| {
        let n = |m: &MonitorInfo| {
            r.x >= m.rect.x && r.x < m.rect.right() && r.y >= m.rect.y && r.y < m.rect.bottom()
        };
        let i = old.iter().position(n)?;
        Some((old[i], *new.get(i)?))
    };
    rederive_layout(
        layout,
        graph,
        &|_, r: &Rect| pair(r).map_or(fallback, |(o, n)| (o.scale, n.scale)),
        (zoom, zoom),
        &|r: &Rect| {
            let Some((o, n)) = pair(r) else {
                return (r.x, r.y);
            };
            let along = |p: Px, o0: Px, ow: Px, n0: Px, nw: Px| {
                n0 + ((p - o0) as f64 * nw as f64 / ow as f64).round() as Px
            };
            (
                along(r.x, o.rect.x, o.rect.w, n.rect.x, n.rect.w),
                along(r.y, o.rect.y, o.rect.h, n.rect.y, n.rect.h),
            )
        },
    )
}

/// The walk `rezoom_layout` and `rescale_layout` share. `scales` gives a
/// window's (old, new) scale from its old rect, `zooms` the (old, new) chrome
/// zoom, and `anchor` where each group's anchor window's top-left lands.
fn rederive_layout(
    layout: &Layout,
    graph: &WindowGraph,
    scales: &dyn Fn(WindowId, &Rect) -> (f64, f64),
    zooms: (f64, f64),
    anchor_at: &dyn Fn(&Rect) -> (Px, Px),
) -> (Layout, WindowGraph) {
    let (old_zoom, new_zoom) = zooms;
    let mut sizes: BTreeMap<WindowId, (Px, Px)> = BTreeMap::new();
    for (id, r) in layout {
        let (old_scale, scale) = scales(*id, r);
        let size = if is_resizable(*id) {
            let steps = |px: Px, base: f64, step: f64| {
                (((px as f64 / old_scale) - base * old_zoom) / (step * old_zoom))
                    .round()
                    .max(0.0) as i32
            };
            let n = steps(r.w, CHROME_W, PLAYLIST_STEP_W);
            let m = steps(r.h, CHROME_H, PLAYLIST_STEP_H);
            (
                bond::d40::stepped(CHROME_W * new_zoom, PLAYLIST_STEP_W * new_zoom, n, scale),
                bond::d40::stepped(CHROME_H * new_zoom, PLAYLIST_STEP_H * new_zoom, m, scale),
            )
        } else {
            (
                bond::d40::physical(CHROME_W * new_zoom, scale),
                bond::d40::physical(CHROME_H * new_zoom, scale),
            )
        };
        sizes.insert(*id, size);
    }

    let ids: Vec<WindowId> = layout.keys().copied().collect();
    let mut out = Layout::new();
    for comp in graph.components(&ids) {
        let Some(anchor) = comp
            .iter()
            .copied()
            .filter(|id| layout.contains_key(id))
            .min_by_key(|id| (layout[id].y, layout[id].x))
        else {
            continue;
        };
        let (ax, ay) = anchor_at(&layout[&anchor]);
        let (aw, ah) = sizes[&anchor];
        out.insert(anchor, Rect::new(ax, ay, aw, ah));

        let mut queue = VecDeque::from([anchor]);
        while let Some(p) = queue.pop_front() {
            let p_old = layout[&p];
            let p_new = out[&p];
            let (old_scale, scale) = scales(p, &p_old);
            // An offset along the seam, re-derived from the logical base.
            let along = |old: Px| {
                bond::d40::physical((old as f64 / old_scale / old_zoom) * new_zoom, scale)
            };
            for q in graph.neighbours(p) {
                if out.contains_key(&q) {
                    continue;
                }
                let (Some(b), Some(q_old), Some(&(qw, qh))) =
                    (graph.bond_between(p, q), layout.get(&q), sizes.get(&q))
                else {
                    continue;
                };
                // Bonds are canonical: `edge` is the side of `a` that `b`
                // sits against, and only Right and Bottom occur.
                let r = match (b.edge, b.a == p) {
                    (Edge::Right, true) => {
                        Rect::new(p_new.right(), p_new.y + along(q_old.y - p_old.y), qw, qh)
                    }
                    (Edge::Bottom, true) => {
                        Rect::new(p_new.x + along(q_old.x - p_old.x), p_new.bottom(), qw, qh)
                    }
                    (Edge::Right, false) => {
                        Rect::new(p_new.x - qw, p_new.y + along(q_old.y - p_old.y), qw, qh)
                    }
                    (Edge::Bottom, false) => {
                        Rect::new(p_new.x + along(q_old.x - p_old.x), p_new.y - qh, qw, qh)
                    }
                    _ => Rect::new(
                        p_new.x + along(q_old.x - p_old.x),
                        p_new.y + along(q_old.y - p_old.y),
                        qw,
                        qh,
                    ),
                };
                out.insert(q, r);
                queue.push_back(q);
            }
        }
    }

    let mut g = WindowGraph::new();
    for b in &graph.bonds {
        let (Some(ra), Some(rb)) = (out.get(&b.a), out.get(&b.b)) else {
            continue;
        };
        let span = if b.edge.is_vertical_seam() {
            (ra.y.max(rb.y), ra.bottom().min(rb.bottom()))
        } else {
            (ra.x.max(rb.x), ra.right().min(rb.right()))
        };
        g.insert(Bond::new(b.a, b.b, b.edge, span));
    }
    (out, g)
}

/// Switch the chrome between 1x and 2x. The whole layout is re-derived, the
/// webviews are re-zoomed, the windows are moved and sized (D52: position,
/// then size), and both the layout and the flag are saved together.
pub fn set_double(app: &AppHandle, on: bool) {
    if app.state::<Wm>().0.lock().unwrap().double == on {
        return;
    }
    let (old_zoom, new_zoom) = if on { (1.0, 2.0) } else { (2.0, 1.0) };
    // D188: each window re-derived from the scale its size is in (D190), and
    // to the one it will be drawn at once grown or shrunk and kept in reach:
    // a stack doubled on the seam at the foot of a display grows down, is
    // lifted back up, and may land mostly on the other display. The scale
    // under each window's top-left corner, as before, was neither. One push
    // (D191): what Windows makes of it, `reconcile` follows.
    let now = os_scales(app, &CLASSIC);
    let (layout, targets) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        let from = laid_out_at(&s, &now);
        let (targets, layout, (graph, unshaded)) =
            rezoom_settled(&s, &from, &now, old_zoom, new_zoom);
        s.unshaded_h.extend(unshaded);
        s.graph = graph;
        s.layout = layout.clone();
        s.drawn_at.extend(&targets);
        // D191: a re-zoom is a layout the person asked for, as a drag is.
        s.reconciles.clear();
        s.double = on;
        (layout, targets)
    }; // D54: the lock is gone before any window call.

    for id in CLASSIC {
        if let Some(win) = app.get_webview_window(label_of(id)) {
            let _ = win.set_zoom(new_zoom);
        }
    }
    push_settled(app, &layout, &CLASSIC, &targets);
    emit_state(app);
    save_now(app);
    reconcile_later(app);
}

/// A re-zoom of the whole state, settled (D188): each window from the scale
/// its size is in, `from` (D190), to the one it will be drawn at once
/// re-derived, rescued and kept in reach, judged from `now`, the scale it
/// is drawn at. The state is not changed; `set_double` commits it.
#[allow(clippy::type_complexity)]
fn rezoom_settled(
    s: &WmState,
    from: &BTreeMap<WindowId, f64>,
    now: &BTreeMap<WindowId, f64>,
    old_zoom: f64,
    new_zoom: f64,
) -> (
    BTreeMap<WindowId, f64>,
    Layout,
    (WindowGraph, BTreeMap<WindowId, Px>),
) {
    settle_scales(&CLASSIC, now, &s.monitors, |t| {
        let (l, g, u) = rederive_whole(
            s,
            new_zoom,
            |l, g| {
                rederive_layout(
                    l,
                    g,
                    &|id, _| {
                        let from = from.get(&id).copied().unwrap_or(s.scale);
                        (from, t.get(&id).copied().unwrap_or(from))
                    },
                    (old_zoom, new_zoom),
                    &|r: &Rect| (r.x, r.y),
                )
            },
            &|id, _| t.get(&id).copied().unwrap_or(s.scale),
            platform::platform().confines_to_work_area(),
        );
        let l = lift_onto_displays(&s.layout, &l, &g, &s.monitors, &|_| true);
        (l, (g, u))
    })
}

/// D189: a group that a re-zoom grew off the displays is lifted back on.
///
/// The owner's 1x to 2x at the foot of DISPLAY1 on the seam: the stack grew
/// down from Main's corner, and the EQ and playlist hung below DISPLAY1's
/// bottom edge, where no display is. Their title bars were in reach, which
/// is all D88 asks, but the person had not put them there. So a group every
/// window of which was wholly on the displays before the re-zoom, over the
/// taskbar included (the owner's call: a stack parked at the bottom edge is
/// on screen), and is not wholly on the work areas after, is moved rigidly
/// by the least that puts it back: up, left, or along to one display,
/// whichever is shortest, clear of the taskbar. A group already partly off
/// the displays before is where the person put it, and is left there. A
/// drag is not affected.
pub fn lift_onto_displays(
    before: &Layout,
    after: &Layout,
    graph: &WindowGraph,
    monitors: &[MonitorInfo],
    only: &dyn Fn(&[WindowId]) -> bool,
) -> Layout {
    let works: Vec<Rect> = monitors.iter().map(|m| m.work).collect();
    let displays: Vec<Rect> = monitors.iter().map(|m| m.rect).collect();
    let on = |l: &Layout, comp: &[WindowId], areas: &[Rect]| {
        comp.iter().filter_map(|id| l.get(id)).all(|r| {
            let covered: i64 = areas
                .iter()
                .map(|n| {
                    let w = (r.right().min(n.right()) - r.x.max(n.x)).max(0) as i64;
                    let h = (r.bottom().min(n.bottom()) - r.y.max(n.y)).max(0) as i64;
                    w * h
                })
                .sum();
            covered >= r.w as i64 * r.h as i64
        })
    };
    let mut out = after.clone();
    let ids: Vec<WindowId> = after.keys().copied().collect();
    for comp in graph.components(&ids) {
        if !only(&comp) || !on(before, &comp, &displays) || on(after, &comp, &works) {
            continue;
        }
        let Some(bounds) = bond::bounds(after, &comp) else {
            continue;
        };
        let Some((dx, dy)) = works
            .iter()
            .map(|w| contain_translation(bounds, merged_rect_for(&works, *w, bounds)))
            .min_by_key(|(dx, dy)| dx.abs() + dy.abs())
        else {
            continue;
        };
        bond::translate_group(&mut out, &comp, dx, dy);
    }
    out
}

/// The scale a layout was saved at, read from Main's width (D182): Main is
/// never resized, so it is `CHROME_W` logical pixels at the chrome zoom.
pub fn saved_scale(layout: &Layout, zoom: f64) -> Option<f64> {
    let w = layout.get(&MAIN)?.w;
    (w > 0).then(|| w as f64 / (CHROME_W * zoom))
}

/// Re-derive the whole layout in place: expand, re-derive, rescue, keep in
/// reach, re-collapse. The same dance `save` does, because a shaded height is
/// a state to reapply, not a size to scale. X's one scale changing (D182)
/// comes through here. Returns the new layout.
fn rederive_state(
    s: &mut WmState,
    new_zoom: f64,
    rederive: impl FnOnce(&Layout, &WindowGraph) -> (Layout, WindowGraph),
) -> Layout {
    let (monitors, fallback) = (s.monitors.clone(), s.scale);
    let (layout, graph, unshaded) = rederive_whole(
        s,
        new_zoom,
        rederive,
        &|_, r: &Rect| monitor_at(&monitors, r.x, r.y).map_or(fallback, |m| m.scale),
        platform::platform().confines_to_work_area(),
    );
    s.unshaded_h.extend(unshaded);
    s.graph = graph;
    s.layout = layout.clone();
    layout
}

/// `rederive_state` without the state changed: the new layout and graph,
/// and the unshaded heights of the shaded windows. `strip` is the scale a
/// shaded window's strip is drawn at, from where it lands. A re-zoom (#47)
/// tries it more than once (D188), so it changes nothing itself. `confine`
/// is whether the window manager keeps windows wholly in the work area.
fn rederive_whole(
    s: &WmState,
    new_zoom: f64,
    rederive: impl FnOnce(&Layout, &WindowGraph) -> (Layout, WindowGraph),
    strip: &dyn Fn(WindowId, &Rect) -> f64,
    confine: bool,
) -> (Layout, WindowGraph, BTreeMap<WindowId, Px>) {
    let mut expanded = s.layout.clone();
    for id in &s.shaded {
        if let Some(h) = s.unshaded_h.get(id).copied() {
            apply_shade(&mut expanded, &s.graph, *id, h);
        }
    }
    let (rederived, graph) = rederive(&expanded, &s.graph);
    // A group that grew may now hang off the display; same rescue as a
    // topology change, so it comes back rigidly with its bonds intact.
    let mut layout = rescue_layout(&rederived, &graph, &s.monitors);
    layout = keep_layout_in_reach(&layout, &graph, &s.monitors, new_zoom, confine);
    let mut unshaded = BTreeMap::new();
    for id in &s.shaded {
        let Some(r) = layout.get(id).copied() else {
            continue;
        };
        unshaded.insert(*id, r.h);
        apply_shade(
            &mut layout,
            &graph,
            *id,
            bond::d40::physical(SHADE_H * new_zoom, strip(*id, &r)),
        );
    }
    (layout, graph, unshaded)
}

// ---- rescue -----------------------------------------------------------------

/// Does this rect put any of itself on a display?
///
/// The test is intersection, not containment: a window half off the right-hand
/// edge is reachable and must not be dragged back by a well-meaning rescue.
pub fn is_on_screen(r: Rect, monitors: &[MonitorInfo]) -> bool {
    monitors.iter().any(|m| {
        let n = m.rect;
        r.x < n.right() && r.right() > n.x && r.y < n.bottom() && r.bottom() > n.y
    })
}

/// The display nearest a rect, by centre-to-centre distance.
pub fn nearest_monitor(monitors: &[MonitorInfo], r: Rect) -> Option<MonitorInfo> {
    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
    monitors
        .iter()
        .min_by_key(|m| {
            let (mx, my) = (m.rect.x + m.rect.w / 2, m.rect.y + m.rect.h / 2);
            // i64 because a virtual desktop several thousand px wide squares
            // into a number an i32 cannot hold.
            let (dx, dy) = ((cx - mx) as i64, (cy - my) as i64);
            dx * dx + dy * dy
        })
        .copied()
}

/// The translation that brings `bounds` inside `m`.
///
/// Clamped to the **top-left**, not centred. A group taller than the display
/// keeps its title bars on screen rather than being centred so that both ends
/// fall off — the top edge is where every grab handle is, so it is the edge
/// worth saving.
///
/// The `max` is what encodes that preference, and it is the whole subtlety
/// here: pulling the bottom edge into view wants a negative dy, keeping the top
/// edge in view wants a non-negative one, and when the group does not fit the
/// second has to win. Taking the minimum instead drags the title bars off the
/// top of the screen, which is exactly the state nothing can recover from.
pub fn contain_translation(bounds: Rect, m: Rect) -> (Px, Px) {
    let dx = if bounds.x < m.x {
        m.x - bounds.x
    } else if bounds.right() > m.right() {
        (m.right() - bounds.right()).max(m.x - bounds.x)
    } else {
        0
    };
    let dy = if bounds.y < m.y {
        m.y - bounds.y
    } else if bounds.bottom() > m.bottom() {
        (m.bottom() - bounds.bottom()).max(m.y - bounds.y)
    } else {
        0
    };
    (dx, dy)
}

/// D57: bring any stranded group back onto a surviving display.
///
/// Every group that has nothing on screen is moved as a **rigid translation**,
/// which is what makes this safe: a rigid move cannot change any relative
/// position, so the rescue provably cannot open a seam. Groups that are still
/// reachable are left exactly where they are — a rescue that tidied up windows
/// the user could still see would be a bug, not a feature.
pub fn rescue_layout(layout: &Layout, graph: &WindowGraph, monitors: &[MonitorInfo]) -> Layout {
    if monitors.is_empty() {
        return layout.clone();
    }
    let mut out = layout.clone();
    for comp in graph.components(&CLASSIC) {
        let Some(bounds) = bond::bounds(&out, &comp) else {
            continue;
        };
        if comp
            .iter()
            .any(|id| out.get(id).is_some_and(|r| is_on_screen(*r, monitors)))
        {
            continue;
        }
        let Some(m) = nearest_monitor(monitors, bounds) else {
            continue;
        };
        let (dx, dy) = contain_translation(bounds, m.rect);
        bond::translate_group(&mut out, &comp, dx, dy);
    }
    out
}

/// #101: can a hand reach this window? Its title bar has to sit inside some
/// display's work area: the bar's whole height, and `REACH_W` of its width.
/// The work area rather than the display, so the taskbar cannot hide it.
pub fn is_reachable(r: Rect, monitors: &[MonitorInfo], title_h: Px, grab: Px) -> bool {
    monitors.iter().any(|m| {
        let w = m.work;
        r.y >= w.y && r.y + title_h <= w.bottom() && r.right().min(w.right()) - r.x.max(w.x) >= grab
    })
}

/// The rigid translation that brings every member of a group within reach
/// (#101, D88), or (0, 0) when they all already are. Toward the display
/// nearest the group, the choice the display rescue makes too. When a group
/// is larger than the work area the top edge wins over the bottom and the
/// left over the right: the windows up there carry the bars a hand goes for.
pub fn reach_clamp(
    layout: &Layout,
    group: &[WindowId],
    monitors: &[MonitorInfo],
    zoom: f64,
) -> (Px, Px) {
    let Some(bounds) = bond::bounds(layout, group) else {
        return (0, 0);
    };
    let Some(m) = nearest_monitor(monitors, bounds) else {
        return (0, 0);
    };
    let title_h = bond::d40::physical(SHADE_H * zoom, m.scale);
    let grab = bond::d40::physical(REACH_W * zoom, m.scale);
    let rects: Vec<Rect> = group
        .iter()
        .filter_map(|id| layout.get(id).copied())
        .collect();
    if rects
        .iter()
        .all(|r| is_reachable(*r, monitors, title_h, grab))
    {
        return (0, 0);
    }
    let w = m.work;
    let min_y = rects.iter().map(|r| r.y).min().unwrap_or(w.y);
    let max_y = rects.iter().map(|r| r.y).max().unwrap_or(w.y);
    let dy = if min_y < w.y {
        w.y - min_y
    } else if max_y + title_h > w.bottom() {
        w.bottom() - title_h - max_y
    } else {
        0
    };
    // Every bar keeps `grab` of its width inside: the shift right some member
    // needs, against the shift right the furthest-right member can still take.
    let need = rects
        .iter()
        .map(|r| w.x + grab - r.right())
        .max()
        .unwrap_or(0);
    let room = rects
        .iter()
        .map(|r| w.right() - grab - r.x)
        .min()
        .unwrap_or(0);
    let dx = if need > 0 {
        need
    } else if room < 0 {
        room
    } else {
        0
    };
    (dx, dy)
}

/// `reach_clamp` for every connected component (#101): launch, the display
/// rescue and a re-zoom, so a layout saved with a bar off the screen heals.
pub fn reach_layout(
    layout: &Layout,
    graph: &WindowGraph,
    monitors: &[MonitorInfo],
    zoom: f64,
) -> Layout {
    keep_layout_in_reach(layout, graph, monitors, zoom, false)
}

/// D182: the rigid translation that keeps every member of a group wholly
/// inside the work area, or (0, 0) when it already is. The area is the work
/// area of the display nearest the group, joined to its neighbours' where the
/// group could slide across the seam (`screen_rect_for`'s walk), so a group
/// may still straddle two displays as Mutter allows. A group larger than the
/// area keeps its top and left edges, where the title bars are.
pub fn confine_clamp(layout: &Layout, group: &[WindowId], monitors: &[MonitorInfo]) -> (Px, Px) {
    let Some(bounds) = bond::bounds(layout, group) else {
        return (0, 0);
    };
    let Some(m) = nearest_monitor(monitors, bounds) else {
        return (0, 0);
    };
    let works: Vec<Rect> = monitors.iter().map(|m| m.work).collect();
    contain_translation(bounds, merged_rect_for(&works, m.work, bounds))
}

/// D182: would this splitter or grip frame take a group that fits inside the
/// work area out of it? Then the frame is refused and the edge stops where
/// the area does, as it would at a window's minimum size. Only where the
/// window manager confines windows, and never for a group that did not fit to
/// begin with, which would otherwise be stuck.
fn grows_out(
    before: &Layout,
    after: &Layout,
    group: &[WindowId],
    monitors: &[MonitorInfo],
) -> bool {
    platform::platform().confines_to_work_area()
        && confine_clamp(before, group, monitors) == (0, 0)
        && confine_clamp(after, group, monitors) != (0, 0)
}

/// The clamp a group gets wherever it moves or grows: within reach (#101,
/// D88), or wholly inside the work area where the window manager would
/// otherwise clamp each window on its own and shear the group (D182).
pub fn keep_in_reach(
    layout: &Layout,
    group: &[WindowId],
    monitors: &[MonitorInfo],
    zoom: f64,
    confine: bool,
) -> (Px, Px) {
    if confine {
        confine_clamp(layout, group, monitors)
    } else {
        reach_clamp(layout, group, monitors, zoom)
    }
}

/// `keep_in_reach` for every connected component: launch, the display
/// rescue and a re-zoom.
pub fn keep_layout_in_reach(
    layout: &Layout,
    graph: &WindowGraph,
    monitors: &[MonitorInfo],
    zoom: f64,
    confine: bool,
) -> Layout {
    let mut out = layout.clone();
    for comp in graph.components(&CLASSIC) {
        let (dx, dy) = keep_in_reach(&out, &comp, monitors, zoom, confine);
        if (dx, dy) != (0, 0) {
            bond::translate_group(&mut out, &comp, dx, dy);
        }
    }
    out
}

/// Read the display topology from the OS.
///
/// D57: `scale_factor()` on a window goes **stale** after a topology change —
/// windows kept reporting 1.5 with only a 1.0 display attached — while a fresh
/// monitor enumeration was correct immediately. So this always re-enumerates
/// and never derives anything from a window.
pub fn read_monitors(app: &AppHandle) -> Vec<MonitorInfo> {
    app.available_monitors()
        .map(|ms| {
            ms.iter()
                .map(|m| MonitorInfo {
                    rect: Rect::new(
                        m.position().x,
                        m.position().y,
                        m.size().width as Px,
                        m.size().height as Px,
                    ),
                    scale: m.scale_factor(),
                    work: {
                        let wa = m.work_area();
                        Rect::new(
                            wa.position.x,
                            wa.position.y,
                            wa.size.width as Px,
                            wa.size.height as Px,
                        )
                    },
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One pass of the display watchdog.
///
/// **Must run on the main thread.** Every Win32 call below targets a window
/// owned by it, so running here makes D54's cross-thread deadlock structurally
/// unreachable rather than merely avoided.
pub fn check_displays(app: &AppHandle) {
    let monitors = read_monitors(app);
    let p = platform::platform();

    // D57: losing a display *minimizes* the group rather than relocating it,
    // and `IsVisible` stays true throughout — so visibility is not the signal
    // and a z-order walk still lists the windows. `IsIconic` is the signal.
    let (handles, topology_changed, by_user) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        (
            CLASSIC.iter().map(|id| s.handle(*id)).collect::<Vec<_>>(),
            monitors != s.monitors,
            s.minimized,
        )
    }; // D54: the lock is gone before is_minimized touches a window.
    let main_iconic = !handles[0].is_none() && p.is_minimized(handles[0]);
    let minimized: Vec<NativeWindow> = handles
        .into_iter()
        .filter(|w| !w.is_none() && p.is_minimized(*w))
        .collect();
    // #86: a group Main's button put in the taskbar is not stranded. The flag
    // is believed only while Main is actually iconic, so a stale one (the
    // group came back some other way) cannot mask a real display loss.
    if !topology_changed && (minimized.is_empty() || (by_user && main_iconic)) {
        return;
    }

    // D59 is why this cannot be left to the user: undecorated windows with no
    // taskbar button have no restore affordance at all. Main keeps a taskbar
    // button as a second way back, but the rescue is the first.
    for w in &minimized {
        p.restore_no_activate(*w);
    }

    let (layout, moved, rescaled) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        s.minimized = false;
        let old_monitors = std::mem::replace(&mut s.monitors, monitors.clone());
        let old_scale = s.scale;
        s.scale = monitors.first().map(|m| m.scale).unwrap_or(s.scale);
        let before = s.layout.clone();
        // D182: where one scale covers the whole desktop, a display coming or
        // going can change it for every window at once. The toolkit follows
        // and redraws at the new scale without resizing anything, so the
        // layout is re-derived for it, positions and sizes both.
        let rescaled = p.one_scale() && (s.scale - old_scale).abs() > 1e-6;
        let rescued = if rescaled {
            let (zoom, new_scale) = (s.zoom(), s.scale);
            eprintln!("wm: the desktop's scale went from {old_scale} to {new_scale}");
            rederive_state(&mut s, zoom, |l, g| {
                rescale_layout(l, g, &old_monitors, &monitors, (old_scale, new_scale), zoom)
            })
        } else {
            let rescued = rescue_layout(&s.layout, &s.graph, &s.monitors);
            keep_layout_in_reach(
                &rescued,
                &s.graph,
                &s.monitors,
                s.zoom(),
                p.confines_to_work_area(),
            )
        };
        let moved: Vec<WindowId> = CLASSIC
            .iter()
            .copied()
            .filter(|id| before.get(id) != rescued.get(id))
            .collect();
        s.layout = rescued.clone();
        (rescued, moved, rescaled)
    }; // D54: lock dropped before the OS is touched.

    // D182: satellites `minimize_group` hid rather than minimised come back
    // with the group, now that the rescue has cleared the flag that would
    // have brought them back.
    if !p.minimises_taskbarless() {
        show_hidden(app);
    }

    if !moved.is_empty() {
        eprintln!(
            "wm: display topology changed, rescued {} window(s) onto a surviving display",
            moved.len()
        );
        if rescaled {
            remap(app, moved);
        } else {
            push_to_os(app, &layout, &moved);
        }
    } else if !minimized.is_empty() {
        // Un-minimizing alone can leave the OS geometry behind the model, so
        // re-assert it (D58: the model and the OS have to be made to agree,
        // and the graph agreeing with itself proves nothing).
        push_to_os(app, &layout, &CLASSIC);
    }
    save_now(app);
    // D191: a display that came, went or changed its scale may have put a
    // window at a scale its size is not in, and a reconcile the minimised
    // group turned away is owed.
    reconcile_later(app);
}

/// Watch for display changes.
///
/// This polls rather than handling `WM_DISPLAYCHANGE` directly — see D62. The
/// interval is deliberately slack: D55 measured topology as changing about once
/// a day, and the failure being guarded against is one where nothing reacts at
/// all, not one where a second matters.
pub fn spawn_display_watch(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let handle = app.clone();
            if app
                .run_on_main_thread(move || check_displays(&handle))
                .is_err()
            {
                break; // app is shutting down
            }
        }
    });
}

// ---- persistence (D33) -------------------------------------------------------

/// The monitor a rect sits on, encoded for the `monitor_id` column.
///
/// The display's own rect, not its device name. What a restore actually needs
/// to know is whether the geometry still lands somewhere real, and the rect
/// answers that directly — while keeping `MonitorInfo` `Copy` and every
/// function above it pure.
fn monitor_id_of(r: Rect, monitors: &[MonitorInfo]) -> Option<String> {
    monitor_at(monitors, r.x, r.y)
        .map(|m| format!("{},{},{}x{}", m.rect.x, m.rect.y, m.rect.w, m.rect.h))
}

pub(crate) fn edge_name(e: Edge) -> &'static str {
    match e {
        Edge::Right => "right",
        Edge::Bottom => "bottom",
        Edge::Left => "left",
        Edge::Top => "top",
    }
}

/// D33: geometry **and** the bond graph survive restart.
///
/// Physical pixels, per the project convention and the schema comment. Written
/// in one transaction so a hard kill mid-write cannot leave the layout and the
/// bonds describing different worlds.
pub fn save(
    conn: &Connection,
    layout: &Layout,
    graph: &WindowGraph,
    shaded: &BTreeSet<WindowId>,
    unshaded_h: &BTreeMap<WindowId, Px>,
    monitors: &[MonitorInfo],
    drawn_at: &BTreeMap<WindowId, f64>,
) -> Result<(), rusqlite::Error> {
    // Store the layout **as if nothing were shaded**, and the shade flags
    // beside it. A shade moves its neighbours as well as changing one height,
    // so writing the collapsed positions next to the expanded heights would
    // save a world that never existed: on the next launch eq's bottom edge and
    // the playlist's top edge would not meet, and register() would correctly
    // drop the bond between them. Expanding first keeps the two halves
    // describing the same layout, and load() re-collapses from it.
    let mut expanded = layout.clone();
    for id in CLASSIC {
        if !shaded.contains(&id) {
            continue;
        }
        if let Some(h) = unshaded_h.get(&id).copied() {
            apply_shade(&mut expanded, graph, id, h);
        }
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM window_layout WHERE window_id IN ('main','eq','playlist')",
        [],
    )?;
    tx.execute("DELETE FROM window_bonds", [])?;

    for id in CLASSIC {
        let Some(r) = expanded.get(&id) else {
            continue;
        };
        let h = r.h;
        tx.execute(
            "INSERT INTO window_layout (window_id, x, y, w, h, shaded, visible, monitor_id, scale)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)",
            rusqlite::params![
                label_of(id),
                r.x,
                r.y,
                r.w,
                h,
                shaded.contains(&id) as i32,
                monitor_id_of(*r, monitors),
                drawn_at.get(&id).copied(),
            ],
        )?;
    }

    for b in &graph.bonds {
        tx.execute(
            "INSERT INTO window_bonds (a, b, edge, span_start, span_end) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![
                label_of(b.a),
                label_of(b.b),
                edge_name(b.edge),
                b.span.0,
                b.span.1
            ],
        )?;
    }
    tx.commit()
}

/// Persist the current layout. Called after every gesture that ends, never
/// per frame — a drag writes once on release, not sixty times a second.
pub fn save_now(app: &AppHandle) {
    let Some(db) = app.try_state::<crate::db::Db>() else {
        return;
    };
    let (layout, graph, shaded, unshaded_h, monitors, double, drawn_at) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        (
            s.layout.clone(),
            s.graph.clone(),
            s.shaded.clone(),
            s.unshaded_h.clone(),
            s.monitors.clone(),
            s.double,
            // D191: kept only where each display has its own scale; where
            // one covers the desktop nothing keeps it, so it is not written.
            if platform::platform().one_scale() {
                BTreeMap::new()
            } else {
                s.drawn_at.clone()
            },
        )
    }; // The wm lock is released before the db lock is taken -- always in that
       // order, so the two can never be acquired against each other.
    let conn = db.0.lock().unwrap();
    if let Err(e) = save(
        &conn,
        &layout,
        &graph,
        &shaded,
        &unshaded_h,
        &monitors,
        &drawn_at,
    ) {
        eprintln!("wm: could not save the window layout: {e}");
    }
    // Beside the layout, never apart from it: the rects only mean what they
    // say at the zoom they were written at (#47).
    if let Err(e) = crate::db::set_setting(&conn, DOUBLE_SETTING, if double { "1" } else { "0" }) {
        eprintln!("wm: could not save the chrome zoom: {e}");
    }
}

/// What a previous session left behind.
pub struct Restored {
    pub layout: Layout,
    pub graph: WindowGraph,
    pub shaded: BTreeSet<WindowId>,
    pub unshaded_h: BTreeMap<WindowId, Px>,
    /// D191: the scale each stored size was derived at, where it was kept.
    pub drawn_at: BTreeMap<WindowId, f64>,
}

/// D33: read back geometry, bonds and shade state.
///
/// Returns `None` if nothing was stored or the rows do not describe all three
/// windows — a partial layout is not worth reconstructing around, and the
/// default stack is a perfectly good answer.
pub fn load(conn: &Connection) -> Option<Restored> {
    let mut layout = Layout::new();
    let mut shaded = BTreeSet::new();
    let mut unshaded_h = BTreeMap::new();
    let mut drawn_at = BTreeMap::new();

    let mut stmt = conn
        .prepare("SELECT window_id, x, y, w, h, shaded, scale FROM window_layout")
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, i32>(4)?,
                row.get::<_, i32>(5)?,
                row.get::<_, Option<f64>>(6)?,
            ))
        })
        .ok()?;

    for row in rows.flatten() {
        let (label, x, y, w, h, is_shaded, scale) = row;
        let Some(id) = id_of(&label) else { continue };
        if let Some(scale) = scale.filter(|s| s.is_finite() && *s > 0.0) {
            drawn_at.insert(id, scale);
        }
        unshaded_h.insert(id, h);
        if is_shaded != 0 {
            shaded.insert(id);
        }
        // The stored height is the unshaded one, so a window that was left
        // collapsed comes back collapsed at the right size rather than at 14px
        // forever. The exact strip height is recomputed on restore.
        layout.insert(id, Rect::new(x, y, w, h));
    }
    if CLASSIC.iter().any(|id| !layout.contains_key(id)) {
        return None;
    }

    let mut graph = WindowGraph::new();
    if let Ok(mut stmt) = conn.prepare("SELECT a, b, edge, span_start, span_end FROM window_bonds")
    {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, i32>(4)?,
            ))
        }) {
            for (a, b, edge, s0, s1) in rows.flatten() {
                let (Some(a), Some(b), Some(edge)) = (id_of(&a), id_of(&b), edge_from_str(&edge))
                else {
                    continue;
                };
                graph.insert(Bond::new(a, b, edge, (s0, s1)));
            }
        }
    }

    Some(Restored {
        layout,
        graph,
        shaded,
        unshaded_h,
        drawn_at,
    })
}

// ---- tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- corner grip ----

    #[test]
    fn grip_snaps_to_the_playlist_grid_and_keeps_the_corner() {
        let o = Rect::new(100, 200, 275, 116);
        // A little past one step each way rounds to one step.
        let r = resize_frame(o, (30, 40), true, true, 1.0, 1.0);
        assert_eq!(r, Rect::new(100, 200, 300, 145));
        // Just under half a step rounds back to none.
        let r = resize_frame(o, (12, 14), true, true, 1.0, 1.0);
        assert_eq!(r, o);
        // Three steps.
        let r = resize_frame(o, (76, 88), true, true, 1.0, 1.0);
        assert_eq!(r, Rect::new(100, 200, 350, 203));
    }

    #[test]
    fn grip_never_goes_under_the_base_size() {
        let o = Rect::new(0, 0, 325, 174);
        let r = resize_frame(o, (-500, -500), true, true, 1.0, 1.0);
        assert_eq!((r.w, r.h), (275, 116));
    }

    #[test]
    fn grip_leaves_a_bonded_edge_alone() {
        let o = Rect::new(0, 0, 275, 116);
        assert_eq!(
            resize_frame(o, (60, 60), false, true, 1.0, 1.0),
            Rect::new(0, 0, 275, 174)
        );
        assert_eq!(
            resize_frame(o, (60, 60), true, false, 1.0, 1.0),
            Rect::new(0, 0, 325, 116)
        );
    }

    #[test]
    fn grip_rounds_once_from_the_logical_base_at_150_percent_and_at_2x() {
        // 150%: one step wide is 300 logical, 450 physical, not 413 + 37.
        let o = Rect::new(0, 0, 413, 174);
        let r = resize_frame(o, (40, 0), true, true, 1.5, 1.0);
        assert_eq!(r.w, bond::d40::stepped(CHROME_W, PLAYLIST_STEP_W, 1, 1.5));
        assert_eq!(r.w, 450);
        // 2x: the grid is 550 + 50n.
        let o = Rect::new(0, 0, 550, 232);
        let r = resize_frame(o, (60, -10), true, true, 1.0, 2.0);
        assert_eq!((r.w, r.h), (600, 232));
    }

    // ---- chrome zoom (#47) ----

    #[test]
    fn rezoom_doubles_a_stack_keeps_it_flush_and_round_trips() {
        let (layout, graph) = initial_layout(1.0, 1.0);
        let (out, g) = rezoom_layout(&layout, &graph, &[], 1.0, 1.0, 2.0);
        assert_eq!(out[&MAIN], Rect::new(120, 120, 550, 232));
        assert_eq!(out[&EQ], Rect::new(120, 352, 550, 232));
        assert_eq!(out[&PLAYLIST], Rect::new(120, 584, 550, 232));
        assert!(bond::violations(&g, &out).is_empty());
        assert_eq!(g.bond_between(MAIN, EQ).unwrap().span, (120, 670));

        let (back, g1) = rezoom_layout(&out, &g, &[], 1.0, 2.0, 1.0);
        assert_eq!(back, layout);
        assert_eq!(g1.bond_between(EQ, PLAYLIST).unwrap().span, (120, 395));
    }

    #[test]
    fn rezoom_keeps_a_row_flush_at_150_percent() {
        // 275 x 1.5 = 412.5 -> 413 each. A scaled offset would put the third
        // window at round(550 x 1.5) = 825, one pixel out of flush; walking
        // the bonds puts it at 413 + 413 = 826 at 1x and 825 + 825 at 2x.
        let scale = 1.5;
        let w = bond::d40::physical(CHROME_W, scale);
        let h = bond::d40::physical(CHROME_H, scale);
        assert_eq!(w, 413);
        let mut layout = Layout::new();
        layout.insert(MAIN, Rect::new(0, 0, w, h));
        layout.insert(EQ, Rect::new(w, 0, w, h));
        layout.insert(PLAYLIST, Rect::new(2 * w, 0, w, h));
        let mut graph = WindowGraph::new();
        graph.insert(Bond::new(MAIN, EQ, Edge::Right, (0, h)));
        graph.insert(Bond::new(EQ, PLAYLIST, Edge::Right, (0, h)));

        let (out, g) = rezoom_layout(&layout, &graph, &[], scale, 1.0, 2.0);
        let w2 = bond::d40::physical(CHROME_W * 2.0, scale);
        assert_eq!(w2, 825);
        assert_eq!(out[&EQ].x, out[&MAIN].right());
        assert_eq!(out[&PLAYLIST].x, out[&EQ].right());
        assert_eq!(out[&PLAYLIST].x, 2 * w2);
        assert!(bond::violations(&g, &out).is_empty());

        let (back, _) = rezoom_layout(&out, &g, &[], scale, 2.0, 1.0);
        assert_eq!(back, layout);
    }

    #[test]
    fn rezoom_keeps_the_playlist_step_count() {
        let (mut layout, graph) = initial_layout(1.0, 1.0);
        // Two steps wider, one taller (D30): 325 x 145.
        let r = layout[&PLAYLIST];
        layout.insert(PLAYLIST, Rect::new(r.x, r.y, 275 + 50, 116 + 29));
        let (out, _) = rezoom_layout(&layout, &graph, &[], 1.0, 1.0, 2.0);
        assert_eq!((out[&PLAYLIST].w, out[&PLAYLIST].h), (550 + 100, 232 + 58));
        let (back, _) = rezoom_layout(&out, &graph, &[], 1.0, 2.0, 1.0);
        assert_eq!(back[&PLAYLIST], layout[&PLAYLIST]);
    }

    #[test]
    fn rezoom_scales_an_offset_along_the_seam_and_leaves_a_loner_put() {
        // eq bonded under main, shifted 50 px right; the playlist is on its
        // own somewhere else. At 2x the shift is 100 and the loner's corner
        // does not move.
        let mut layout = Layout::new();
        layout.insert(MAIN, Rect::new(0, 0, 275, 116));
        layout.insert(EQ, Rect::new(50, 116, 275, 116));
        layout.insert(PLAYLIST, Rect::new(900, 900, 275, 116));
        let mut graph = WindowGraph::new();
        graph.insert(Bond::new(MAIN, EQ, Edge::Bottom, (50, 275)));

        let (out, g) = rezoom_layout(&layout, &graph, &[], 1.0, 1.0, 2.0);
        assert_eq!(out[&MAIN], Rect::new(0, 0, 550, 232));
        assert_eq!(out[&EQ], Rect::new(100, 232, 550, 232));
        assert_eq!(out[&PLAYLIST], Rect::new(900, 900, 550, 232));
        assert_eq!(g.bond_between(MAIN, EQ).unwrap().span, (100, 550));
        assert!(bond::violations(&g, &out).is_empty());
    }

    #[test]
    fn rezoom_walks_a_bond_backwards_from_the_anchor() {
        // The anchor is the top-most window. Here eq is ABOVE main, so the
        // walk from eq reaches main through a bond where main is `a`.
        let mut layout = Layout::new();
        layout.insert(EQ, Rect::new(0, 0, 275, 116));
        layout.insert(MAIN, Rect::new(0, 116, 275, 116));
        layout.insert(PLAYLIST, Rect::new(0, 232, 275, 116));
        let mut graph = WindowGraph::new();
        graph.insert(Bond::new(EQ, MAIN, Edge::Bottom, (0, 275)));
        graph.insert(Bond::new(PLAYLIST, MAIN, Edge::Top, (0, 275)));

        let (out, g) = rezoom_layout(&layout, &graph, &[], 1.0, 1.0, 2.0);
        assert_eq!(out[&EQ], Rect::new(0, 0, 550, 232));
        assert_eq!(out[&MAIN], Rect::new(0, 232, 550, 232));
        assert_eq!(out[&PLAYLIST], Rect::new(0, 464, 550, 232));
        assert!(bond::violations(&g, &out).is_empty());
    }

    #[test]
    fn seam_quantises_on_the_doubled_grid_at_2x() {
        // At 2x the playlist steps are 50 x 58 from a 550 x 232 base.
        let mut s = state_with(&[(MAIN, EQ), (EQ, PLAYLIST)]);
        let (l, g) = initial_layout(1.0, 2.0);
        s.layout = l;
        s.graph = g;
        let b = *s.graph.bond_between(EQ, PLAYLIST).unwrap();
        let bottom = s.layout[&PLAYLIST].bottom();
        // Ask for 232 + 58 x 3 + a bit: lands on exactly three steps.
        let raw = bottom - (232 + 58 * 3) - 20;
        assert_eq!(
            quantize_seam(&s.layout, &b, raw, 1.0, 2.0),
            bottom - (232 + 58 * 3)
        );
    }

    fn nw(n: isize) -> NativeWindow {
        NativeWindow(n)
    }

    /// Three windows, three roots, handles 10/11/12 and roots 90/91/92.
    fn state_with(bonds: &[(WindowId, WindowId)]) -> WmState {
        let mut s = WmState {
            handles: vec![nw(10), nw(11), nw(12)],
            roots: vec![nw(90), nw(91), nw(92)],
            scale: 1.0,
            ..Default::default()
        };
        for (i, id) in CLASSIC.iter().enumerate() {
            s.layout.insert(*id, Rect::new(0, 116 * i as Px, 275, 116));
        }
        for (a, b) in bonds {
            s.graph.insert(Bond::new(*a, *b, Edge::Bottom, (0, 275)));
        }
        s
    }

    #[test]
    fn one_group_gets_one_root() {
        let s = state_with(&[(MAIN, EQ), (EQ, PLAYLIST)]);
        let plan = plan_ownership(&s, Some(MAIN));
        assert_eq!(
            plan.owners,
            vec![(nw(10), nw(90)), (nw(11), nw(90)), (nw(12), nw(90))]
        );
    }

    #[test]
    fn a_split_gives_each_side_its_own_root() {
        // The D41 payoff: breaking a bond is a re-point, not a promotion.
        let s = state_with(&[(MAIN, EQ)]);
        let plan = plan_ownership(&s, Some(MAIN));
        let root_of = |h: NativeWindow| plan.owners.iter().find(|(w, _)| *w == h).unwrap().1;
        assert_eq!(root_of(nw(10)), root_of(nw(11)));
        assert_ne!(root_of(nw(10)), root_of(nw(12)));
    }

    #[test]
    fn no_real_window_ever_owns_another() {
        let s = state_with(&[(MAIN, EQ), (EQ, PLAYLIST)]);
        let plan = plan_ownership(&s, Some(PLAYLIST));
        for (_, owner) in &plan.owners {
            assert!(
                s.roots.contains(owner),
                "a real window became an owner: {owner:?} — that is the star \
                 topology, and it pins the owner to the back of its own group"
            );
        }
    }

    #[test]
    fn the_touched_group_is_raised_last() {
        // Two groups: {main, eq} and {playlist}. Touching the playlist has to
        // put it on top, which means raising it last.
        let s = state_with(&[(MAIN, EQ)]);
        let plan = plan_ownership(&s, Some(PLAYLIST));
        assert_eq!(*plan.raise.last().unwrap(), nw(12));

        // And the touched window last within its group (D193).
        let plan = plan_ownership(&s, Some(MAIN));
        assert_eq!(*plan.raise.last().unwrap(), nw(10));
    }

    #[test]
    fn the_touched_window_is_raised_last_within_its_group() {
        // D193: the owner clicked Main and found it behind the EQ.
        let s = state_with(&[(MAIN, EQ), (EQ, PLAYLIST)]);
        assert_eq!(
            *plan_ownership(&s, Some(MAIN)).raise.last().unwrap(),
            nw(10)
        );
        assert_eq!(*plan_ownership(&s, Some(EQ)).raise.last().unwrap(), nw(11));
    }

    #[test]
    fn an_owner_already_set_is_left_alone() {
        // D193: setting an owner gives the window the owner's scale for a
        // moment, so only the ones that change are set.
        let s = state_with(&[(MAIN, EQ)]);
        let plan = plan_ownership(&s, Some(MAIN));
        let all = owner_changes(&plan, |_| NativeWindow::NONE);
        assert_eq!(all, plan.owners, "nothing owned yet: all of them");
        let same = owner_changes(&plan, |w| {
            plan.owners
                .iter()
                .find(|(h, _)| *h == w)
                .map(|(_, o)| *o)
                .unwrap()
        });
        assert!(same.is_empty(), "every owner already set: none");
        // The playlist owned by some other root: only it is set.
        let moved = owner_changes(&plan, |w| {
            if w == nw(12) {
                nw(99)
            } else {
                plan.owners
                    .iter()
                    .find(|(h, _)| *h == w)
                    .map(|(_, o)| *o)
                    .unwrap()
            }
        });
        let playlist: Vec<_> = plan
            .owners
            .iter()
            .copied()
            .filter(|(h, _)| *h == nw(12))
            .collect();
        assert_eq!(moved, playlist);
    }

    #[test]
    fn a_window_the_owner_moved_off_its_scale_is_put_back_and_a_crossing_is_not() {
        // (before, laid out, after) -> put back from, to.
        let table = [
            // At its scale, and the owner gave it another: put back.
            ((Some(1.5), Some(1.5), Some(1.0)), Some((1.0, 1.5))),
            // Still crossing when the owner was set, and it got there.
            ((Some(1.0), Some(1.5), Some(1.5)), None),
            // Still crossing, not there yet: `reconcile`'s.
            ((Some(1.0), Some(1.5), Some(1.0)), None),
            // Nothing changed.
            ((Some(1.5), Some(1.5), Some(1.5)), None),
            // No laid-out scale recorded: judged from the one it was at.
            ((Some(1.5), None, Some(1.0)), Some((1.0, 1.5))),
            ((Some(1.5), None, Some(1.5)), None),
            // The mirror, laid out at 100 %.
            ((Some(1.0), Some(1.0), Some(1.5)), Some((1.5, 1.0))),
            ((Some(1.5), Some(1.0), Some(1.0)), None),
            ((Some(1.5), Some(1.0), Some(1.5)), None),
            ((Some(1.0), Some(1.0), Some(1.0)), None),
            ((Some(1.0), None, Some(1.5)), Some((1.5, 1.0))),
            // Where the window system gives no scale, nothing.
            ((None, Some(1.5), Some(1.0)), None),
            ((Some(1.5), Some(1.5), None), None),
        ];
        for ((b, t, a), want) in table {
            assert_eq!(put_back(b, t, a), want, "{b:?} {t:?} {a:?}");
        }
    }

    /// D193: a window system that holds a change of scale back until it is
    /// pumped, gives a window put on a display that display's scale, and
    /// gives a window its owner's scale when its owner is set, as the owner's
    /// desk did. A change still held then lands at the next pump, as the
    /// re-dock D193 records was put back after it had.
    #[derive(Default)]
    struct HeldBack {
        monitors: Vec<MonitorInfo>,
        scale: std::cell::RefCell<BTreeMap<NativeWindow, f64>>,
        held: std::cell::RefCell<BTreeMap<NativeWindow, f64>>,
        owner: std::cell::RefCell<BTreeMap<NativeWindow, NativeWindow>>,
    }

    impl OwnerOps for HeldBack {
        fn owner(&self, w: NativeWindow) -> NativeWindow {
            self.owner
                .borrow()
                .get(&w)
                .copied()
                .unwrap_or(NativeWindow::NONE)
        }
        fn own(&self, w: NativeWindow, owner: NativeWindow) {
            self.owner.borrow_mut().insert(w, owner);
            let given = self.scale.borrow().get(&owner).copied();
            if let Some(s) = given {
                self.scale.borrow_mut().insert(w, s);
            }
        }
        fn lift(&self, _w: NativeWindow) {}
        fn scale(&self, w: NativeWindow) -> Option<f64> {
            self.scale.borrow().get(&w).copied()
        }
        fn put(&self, w: NativeWindow, x: i32, y: i32, cx: i32, cy: i32) -> bool {
            let s = judged_scale(&self.monitors, Rect::new(x, y, cx, cy), 1.0);
            self.held.borrow_mut().insert(w, s);
            true
        }
        fn deliver(&self, w: NativeWindow) {
            let held = self.held.borrow_mut().remove(&w);
            if let Some(s) = held {
                self.scale.borrow_mut().insert(w, s);
            }
        }
    }

    /// The demagnetised playlist re-docked into a 150 % group on DISPLAY2,
    /// the snap carrying it across the seam on the release frame.
    fn redock(plan_monitors: Vec<MonitorInfo>) -> (HeldBack, OwnPlan) {
        let (pl, root) = (nw(12), nw(91));
        let fake = HeldBack {
            monitors: desk(),
            ..Default::default()
        };
        fake.scale.borrow_mut().insert(root, 1.0);
        let plan = OwnPlan {
            owners: vec![(pl, root)],
            raise: vec![],
            monitors: plan_monitors,
            rects: vec![(pl, Rect::new(2600, 464, 825, 609))],
            scales: vec![(pl, 1.5)],
        };
        (fake, plan)
    }

    #[test]
    fn a_crossing_is_delivered_before_the_owner_is_set() {
        // The release frame put it on DISPLAY2; the change is held back.
        let (fake, plan) = redock(desk());
        fake.scale.borrow_mut().insert(nw(12), 1.0);
        fake.held.borrow_mut().insert(nw(12), 1.5);
        apply_ownership_on(&fake, &plan);
        assert_eq!(fake.owner(nw(12)), nw(91));
        assert_eq!(fake.scale(nw(12)), Some(1.5));
        // Read before the change was delivered, the scale was 100 % and the
        // root went to DISPLAY1: a 100 % owner for a window on DISPLAY2.
        assert_eq!(fake.scale(nw(91)), Some(1.5));
    }

    #[test]
    fn a_window_the_owner_pulled_off_its_scale_is_put_back() {
        // No display at 150 % in the plan's list (a scale changed in the
        // settings a moment ago): the root stays at 100 % and gives the
        // playlist 100 %; it is put back where it is laid out.
        let (fake, plan) = redock(vec![desk()[0]]);
        fake.scale.borrow_mut().insert(nw(12), 1.5);
        apply_ownership_on(&fake, &plan);
        assert_eq!(fake.scale(nw(12)), Some(1.5));
    }

    #[test]
    fn an_owner_already_set_is_not_set_again() {
        let (fake, plan) = redock(desk());
        fake.scale.borrow_mut().insert(nw(12), 1.5);
        fake.owner.borrow_mut().insert(nw(12), nw(91));
        apply_ownership_on(&fake, &plan);
        assert_eq!(fake.scale(nw(12)), Some(1.5));
        assert_eq!(fake.scale(nw(91)), Some(1.0), "the root was not moved");
    }

    #[test]
    fn a_drag_raises_the_window_grabbed_last_within_its_group() {
        // Dragged by the EQ's title bar: the EQ on top on the release, not
        // Main, the group's first.
        let mut s = state_with(&[(MAIN, EQ), (EQ, PLAYLIST)]);
        let drag = DragState {
            moving: CLASSIC.to_vec(),
            origin_layout: s.layout.clone(),
            origin_cursor: (0, 0),
            origin_unshaded: BTreeMap::new(),
            origin_scales: BTreeMap::new(),
            grabbed: EQ,
        };
        assert_eq!(*finish_drag(&mut s, &drag).raise.last().unwrap(), nw(11));
    }

    #[test]
    fn every_window_is_raised_exactly_once() {
        let s = state_with(&[(MAIN, EQ)]);
        let plan = plan_ownership(&s, Some(MAIN));
        let mut seen = plan.raise.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), plan.raise.len());
        assert_eq!(plan.raise.len(), CLASSIC.len());
    }

    #[test]
    fn no_roots_yet_means_no_plan() {
        // register() has not run. Doing nothing is correct; setting an owner to
        // NativeWindow::NONE would silently clear the topology instead.
        let mut s = state_with(&[(MAIN, EQ)]);
        s.roots.clear();
        assert_eq!(plan_ownership(&s, Some(MAIN)), OwnPlan::default());
    }

    #[test]
    fn only_the_playlist_resizes() {
        // D35: the cursor tells the truth. A main/eq seam is a move handle, not
        // a splitter, because neither side can change size.
        assert!(!is_resizable(MAIN));
        assert!(!is_resizable(EQ));
        assert!(is_resizable(PLAYLIST));
        assert!(!bond::splitter_is_live(
            is_resizable(MAIN),
            is_resizable(EQ)
        ));
        assert!(bond::splitter_is_live(
            is_resizable(EQ),
            is_resizable(PLAYLIST)
        ));
    }

    #[test]
    fn chrome_is_physical_and_rounded_once() {
        // D38/D40: 275 x 1.5 = 412.5, and the toolkit's rounding is not ours to
        // inherit. Also the 2x check: 413 * 2 = 826, but 550 * 1.5 = 825.
        assert_eq!(bond::d40::physical(CHROME_W, 1.5), 413);
        assert_eq!(bond::d40::physical(CHROME_W * 2.0, 1.5), 825);
        assert_ne!(bond::d40::physical(CHROME_W, 1.5) * 2, 825);
    }

    // ---- monitors -----------------------------------------------------------

    fn mon(x: Px, y: Px, w: Px, h: Px, scale: f64) -> MonitorInfo {
        MonitorInfo {
            rect: Rect::new(x, y, w, h),
            scale,
            work: Rect::new(x, y, w, h),
        }
    }

    /// A display with a taskbar along its bottom edge.
    fn mon_tb(x: Px, y: Px, w: Px, h: Px, scale: f64, taskbar: Px) -> MonitorInfo {
        MonitorInfo {
            rect: Rect::new(x, y, w, h),
            scale,
            work: Rect::new(x, y, w, h - taskbar),
        }
    }

    /// Laptop at 150% on the left, external at 100% to its right, sharing the
    /// seam at x = 2560. The stage 6 arrangement.
    fn two_monitors() -> Vec<MonitorInfo> {
        vec![mon(0, 0, 2560, 1440, 1.5), mon(2560, 0, 1920, 1080, 1.0)]
    }

    #[test]
    fn the_threshold_follows_the_cursor_not_the_window() {
        // D51. Same logical 10 px magnet, two different physical answers, and
        // which one applies is decided by where the hand is.
        let ms = two_monitors();
        assert_eq!(scale_at(&ms, 100, 100, 1.0), 1.5);
        assert_eq!(scale_at(&ms, 3000, 100, 1.5), 1.0);
        assert_eq!(
            bond::d40::threshold(SNAP_THRESHOLD, scale_at(&ms, 100, 100, 1.0)),
            15
        );
        assert_eq!(
            bond::d40::threshold(SNAP_THRESHOLD, scale_at(&ms, 3000, 100, 1.0)),
            10
        );
    }

    #[test]
    fn a_point_outside_every_monitor_falls_back() {
        let ms = two_monitors();
        assert_eq!(monitor_at(&ms, -50, -50), None);
        assert_eq!(scale_at(&ms, -50, -50, 1.25), 1.25);
    }

    #[test]
    fn a_shared_monitor_edge_is_not_a_screen_edge() {
        // D53. The seam at x = 2560 is where a naive implementation drops an
        // invisible wall down the middle of a continuous desktop.
        let ms = two_monitors();
        let group = Rect::new(2400, 100, 275, 116);
        let screen = screen_rect_for(&ms, ms[0].rect, group);
        assert_eq!(screen, Rect::new(0, 0, 4480, 1440));

        // Approaching the seam from the left must not snap to it...
        assert_eq!(bond::screen_edge_snap(group, screen, 15), (0, 0));
        // ...but the desktop's genuine right-hand edge still magnetises.
        let group = Rect::new(4200, 100, 275, 116);
        let screen = screen_rect_for(&ms, ms[1].rect, group);
        assert_eq!(bond::screen_edge_snap(group, screen, 15), (5, 0));
    }

    #[test]
    fn a_neighbour_too_short_to_hold_the_group_does_not_hide_the_seam() {
        // The external display is 1080 tall against the laptop's 1440. A group
        // sitting below y = 1080 cannot slide across the seam at all, so for
        // that group the seam is a real edge and snapping to it is correct.
        let ms = two_monitors();
        let low = Rect::new(2200, 1200, 275, 116);
        assert_eq!(screen_rect_for(&ms, ms[0].rect, low), ms[0].rect);
    }

    #[test]
    fn an_l_shaped_desktop_does_not_invent_screen_in_empty_space() {
        // A bounding box over all monitors would hand back the empty quadrant
        // below the right-hand display as somewhere a window may be snapped to.
        let ms = vec![
            mon(0, 0, 1920, 1080, 1.0),
            mon(1920, 0, 1920, 1080, 1.0),
            mon(0, 1080, 1920, 1080, 1.0),
        ];
        let on_right = Rect::new(2000, 100, 275, 116);
        let screen = screen_rect_for(&ms, ms[1].rect, on_right);
        assert_eq!(screen, Rect::new(0, 0, 3840, 1080));
        assert!(screen.bottom() < 2160, "claimed screen where there is none");

        // The same desktop, a group on the left-hand display: there the bottom
        // edge really is an inner seam, and it does get pushed out.
        let on_left = Rect::new(100, 100, 275, 116);
        let screen = screen_rect_for(&ms, ms[0].rect, on_left);
        assert_eq!(screen, Rect::new(0, 0, 3840, 2160));
    }

    #[test]
    fn one_monitor_is_its_own_screen() {
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        let g = Rect::new(100, 100, 275, 116);
        assert_eq!(screen_rect_for(&ms, ms[0].rect, g), ms[0].rect);
        assert_eq!(
            screen_rect_for(&[], Rect::new(0, 0, 800, 600), g),
            Rect::new(0, 0, 800, 600)
        );
    }

    // ---- drag ---------------------------------------------------------------

    /// main and eq bonded and flush; playlist parked off to the right.
    fn drag_layout() -> Layout {
        let mut l = Layout::new();
        l.insert(MAIN, Rect::new(0, 0, 275, 116));
        l.insert(EQ, Rect::new(0, 116, 275, 116));
        l.insert(PLAYLIST, Rect::new(600, 0, 275, 116));
        l
    }

    #[test]
    fn a_group_drag_preserves_offsets_exactly() {
        let origin = drag_layout();
        let out = drag_frame(&origin, &[MAIN, EQ], (37, -12), &[PLAYLIST], 10, None);
        assert_eq!(out[&MAIN], Rect::new(37, -12, 275, 116));
        assert_eq!(out[&EQ], Rect::new(37, 104, 275, 116));
        // The stationary window is exactly where it was.
        assert_eq!(out[&PLAYLIST], origin[&PLAYLIST]);
        // And the bond is still flush.
        assert_eq!(out[&MAIN].bottom(), out[&EQ].y);
    }

    #[test]
    fn every_frame_is_recomputed_from_the_origin() {
        // D40, stated as a test: a thousand frames of a drag land in exactly
        // the same place as one frame of the same total delta. If any frame
        // accumulated onto the previous one this would drift.
        let origin = drag_layout();
        let mut total = (0, 0);
        let mut last = origin.clone();
        for _ in 0..1000 {
            total = (total.0 + 3, total.1 + 1);
            last = drag_frame(&origin, &[MAIN, EQ], total, &[PLAYLIST], 10, None);
        }
        let one_shot = drag_frame(&origin, &[MAIN, EQ], (3000, 1000), &[PLAYLIST], 10, None);
        assert_eq!(last[&MAIN], one_shot[&MAIN]);
        assert_eq!(last[&EQ], one_shot[&EQ]);
        assert!(bond::violations(
            &{
                let mut g = WindowGraph::new();
                g.insert(Bond::new(MAIN, EQ, Edge::Bottom, (0, 275)));
                g
            },
            &last
        )
        .is_empty());
    }

    #[test]
    fn the_magnet_pulls_the_whole_group_flush() {
        // Drag main+eq so main lands 7 px short of the playlist's left edge.
        // Within the 10 px threshold, so it snaps -- and eq comes with it.
        let origin = drag_layout();
        let out = drag_frame(&origin, &[MAIN, EQ], (318, 0), &[PLAYLIST], 10, None);
        assert_eq!(out[&MAIN].right(), out[&PLAYLIST].x, "did not snap flush");
        assert_eq!(out[&EQ].x, out[&MAIN].x, "eq did not come along");
        assert_eq!(
            out[&MAIN].bottom(),
            out[&EQ].y,
            "the group's own bond opened"
        );
    }

    #[test]
    fn outside_the_threshold_nothing_moves_it() {
        let origin = drag_layout();
        let out = drag_frame(&origin, &[MAIN, EQ], (300, 0), &[PLAYLIST], 10, None);
        assert_eq!(out[&MAIN], Rect::new(300, 0, 275, 116));
    }

    #[test]
    fn the_nearer_of_two_candidates_wins() {
        // Two stationary windows within reach at once. Cheapest snap wins, so
        // the window goes where the hand was actually heading.
        let mut origin = Layout::new();
        origin.insert(MAIN, Rect::new(0, 0, 275, 116));
        origin.insert(EQ, Rect::new(283, 0, 275, 116)); // 8 px to the right
        origin.insert(PLAYLIST, Rect::new(-278, 0, 275, 116)); // 3 px to the left
        let out = drag_frame(&origin, &[MAIN], (0, 0), &[EQ, PLAYLIST], 10, None);
        assert_eq!(
            out[&MAIN].x,
            origin[&PLAYLIST].right(),
            "snapped to the far one"
        );

        // Again with the two distances swapped. `others` is walked in order, so
        // above the winner also happened to be the last one probed and "keep
        // whichever came last" would pass too. Here the nearer window is probed
        // first, which is what actually pins the comparison.
        origin.insert(EQ, Rect::new(278, 0, 275, 116)); // 3 px to the right
        origin.insert(PLAYLIST, Rect::new(-283, 0, 275, 116)); // 8 px to the left
        let out = drag_frame(&origin, &[MAIN], (0, 0), &[EQ, PLAYLIST], 10, None);
        assert_eq!(
            out[&MAIN].right(),
            origin[&EQ].x,
            "a later, dearer candidate overwrote the best one"
        );
    }

    #[test]
    fn a_screen_edge_is_a_movement_constraint_only() {
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        let screen = screen_rect_for(&ms, ms[0].rect, Rect::new(0, 0, 275, 232));
        let origin = drag_layout();
        let out = drag_frame(
            &origin,
            &[MAIN, EQ],
            (-6, -4),
            &[PLAYLIST],
            10,
            Some(screen),
        );
        assert_eq!(out[&MAIN], Rect::new(0, 0, 275, 116));
        // No resize, and the group's internal offset is untouched.
        assert_eq!(out[&EQ], Rect::new(0, 116, 275, 116));
    }

    #[test]
    fn a_window_magnet_beats_a_screen_edge() {
        // Both are in range. Bonding to a window is the stronger relationship:
        // it forms a graph edge, the screen edge does not.
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        let screen = screen_rect_for(&ms, ms[0].rect, Rect::new(8, 300, 275, 116));
        let mut origin = Layout::new();
        origin.insert(MAIN, Rect::new(8, 300, 275, 116));
        origin.insert(EQ, Rect::new(291, 300, 275, 116));
        origin.insert(PLAYLIST, Rect::new(1600, 900, 275, 116));
        let out = drag_frame(&origin, &[MAIN], (0, 0), &[EQ, PLAYLIST], 10, Some(screen));
        assert_eq!(
            out[&MAIN].right(),
            origin[&EQ].x,
            "took the screen edge instead"
        );
        assert_ne!(out[&MAIN].x, 0);
    }

    // ---- bond forming -------------------------------------------------------

    #[test]
    fn a_drag_that_lands_flush_forms_a_bond() {
        let mut l = drag_layout();
        l.insert(PLAYLIST, Rect::new(275, 0, 275, 116));
        let bonds = bonds_after_drag(&l, &[PLAYLIST], &[MAIN, EQ]);
        assert_eq!(bonds.len(), 1);
        assert_eq!(bonds[0].pair(), (MAIN, PLAYLIST));
        // Stored canonically: a is the left window whatever order it arrived in.
        assert_eq!(bonds[0].a, MAIN);
        assert_eq!(bonds[0].edge, Edge::Right);
    }

    #[test]
    fn touching_at_a_corner_is_not_a_bond() {
        // Flush on one axis, zero overlap on the other. A bond needs a seam
        // with actual length or the splitter has nothing to grab.
        let mut l = Layout::new();
        l.insert(MAIN, Rect::new(0, 0, 275, 116));
        l.insert(PLAYLIST, Rect::new(275, 116, 275, 116));
        assert!(bonds_after_drag(&l, &[PLAYLIST], &[MAIN]).is_empty());
    }

    #[test]
    fn a_one_pixel_gap_is_not_a_bond() {
        let mut l = drag_layout();
        l.insert(PLAYLIST, Rect::new(276, 0, 275, 116));
        assert!(bonds_after_drag(&l, &[PLAYLIST], &[MAIN, EQ]).is_empty());
    }

    // ---- bodies (#100, D89) -------------------------------------------------

    /// Main with the playlist shaded flush under it, and the EQ elsewhere.
    fn main_over_shaded_playlist() -> Layout {
        let mut l = Layout::new();
        l.insert(MAIN, Rect::new(0, 0, 275, 116));
        l.insert(PLAYLIST, Rect::new(0, 116, 275, 14));
        l.insert(EQ, Rect::new(600, 0, 275, 116));
        l
    }

    #[test]
    fn a_window_dropped_on_a_members_body_stays_loose() {
        // The EQ dropped flush under Main, right on the shaded playlist: exactly
        // flush with Main's bottom, and lying on the playlist. No bond.
        let mut l = main_over_shaded_playlist();
        l.insert(EQ, Rect::new(0, 116, 275, 116));
        assert!(bonds_after_drag(&l, &[EQ], &[MAIN, PLAYLIST]).is_empty());
        // Clear of the playlist, the same edge does bond.
        let mut l = main_over_shaded_playlist();
        l.remove(&PLAYLIST);
        l.insert(EQ, Rect::new(0, 116, 275, 116));
        assert_eq!(bonds_after_drag(&l, &[EQ], &[MAIN]).len(), 1);
    }

    #[test]
    fn a_magnet_never_pulls_a_window_onto_a_body() {
        // Four pixels below Main's bottom edge, over the playlist. The nearer
        // magnet (Main's bottom, cost 4) would land it on the playlist, so the
        // further one wins: flush under the playlist instead, cost 10.
        let mut origin = main_over_shaded_playlist();
        origin.insert(EQ, Rect::new(0, 120, 275, 116));
        let out = drag_frame(&origin, &[EQ], (0, 0), &[MAIN, PLAYLIST], 10, None);
        assert_eq!(out[&EQ].y, 130);
        assert!(bond::overlapping_pairs(&out, &CLASSIC).is_empty());
        // Too deep in a body for any magnet: it stays where it is, loose.
        let mut origin = main_over_shaded_playlist();
        origin.insert(PLAYLIST, Rect::new(0, 116, 275, 116));
        origin.insert(EQ, Rect::new(0, 150, 275, 116));
        let out = drag_frame(&origin, &[EQ], (0, 0), &[MAIN, PLAYLIST], 10, None);
        assert_eq!(out[&EQ].y, 150);
        assert!(bonds_after_drag(&out, &[EQ], &[MAIN, PLAYLIST]).is_empty());
    }

    #[test]
    fn overlapping_pairs_sees_bodies_that_cross_and_not_edges_that_touch() {
        let l = main_over_shaded_playlist();
        assert!(bond::overlapping_pairs(&l, &CLASSIC).is_empty());
        let mut l = l;
        l.insert(EQ, Rect::new(100, 100, 275, 116));
        assert_eq!(
            bond::overlapping_pairs(&l, &CLASSIC),
            vec![(MAIN, EQ), (EQ, PLAYLIST)]
        );
    }

    #[test]
    fn windows_that_both_sat_still_are_left_alone() {
        // main and eq are flush and stationary. Dragging the playlist somewhere
        // unrelated must not re-bond them -- if the user had just demagnetized
        // that seam, silently restoring it would look like the break failed.
        let l = drag_layout();
        let bonds = bonds_after_drag(&l, &[PLAYLIST], &[MAIN, EQ]);
        assert!(bonds.iter().all(|b| b.pair() != (MAIN, EQ)));
    }

    // ---- focus --------------------------------------------------------------

    #[test]
    fn focus_lights_the_whole_group_and_only_that_group() {
        let s = state_with(&[(MAIN, EQ)]);
        let flags = focus_plan(&s, Some(EQ));
        assert_eq!(flags, vec![(MAIN, true), (EQ, true), (PLAYLIST, false)]);
    }

    #[test]
    fn losing_focus_darkens_everything() {
        let s = state_with(&[(MAIN, EQ), (EQ, PLAYLIST)]);
        let flags = focus_plan(&s, None);
        assert!(flags.iter().all(|(_, active)| !active));
    }

    // ---- seams --------------------------------------------------------------

    /// The default stack: main on top, eq under it, playlist under that.
    fn stacked() -> WmState {
        state_with(&[(MAIN, EQ), (EQ, PLAYLIST)])
    }

    // ---- minimise (#86) ----

    #[test]
    fn main_minimises_its_whole_component_main_last() {
        assert_eq!(minimize_plan(&stacked()), vec![EQ, PLAYLIST, MAIN]);
    }

    #[test]
    fn main_off_the_group_minimises_alone() {
        assert_eq!(minimize_plan(&state_with(&[(EQ, PLAYLIST)])), vec![MAIN]);
    }

    #[test]
    fn each_window_knows_which_of_its_edges_are_seams() {
        let s = stacked();
        // Bonds are stored canonically, so the same bond is main's bottom edge
        // and eq's top edge. Reading it from the wrong end is how a seam ends
        // up drawn on the outside of a group.
        assert_eq!(
            edges_for(&s, MAIN),
            Edges {
                bottom: Some(false),
                ..Default::default()
            }
        );
        assert_eq!(
            edges_for(&s, EQ),
            Edges {
                top: Some(false),
                bottom: Some(true),
                ..Default::default()
            }
        );
        assert_eq!(
            edges_for(&s, PLAYLIST),
            Edges {
                top: Some(true),
                ..Default::default()
            }
        );
    }

    #[test]
    fn a_seam_between_two_fixed_windows_is_not_a_splitter() {
        // D35, from the window's point of view. main/eq is bonded but inert as
        // a splitter, so it is offered as a move handle and never as a resize.
        let s = stacked();
        assert_eq!(edges_for(&s, MAIN).bottom, Some(false));
        assert_eq!(edges_for(&s, EQ).bottom, Some(true));
    }

    #[test]
    fn an_unbonded_window_has_no_seams() {
        let mut s = stacked();
        s.graph = WindowGraph::new();
        assert_eq!(edges_for(&s, EQ), Edges::default());
    }

    #[test]
    fn a_seam_is_found_from_either_side() {
        let s = stacked();
        let from_eq = seam_on(&s, EQ, Edge::Bottom).expect("eq has a bottom seam");
        let from_playlist = seam_on(&s, PLAYLIST, Edge::Top).expect("playlist has a top seam");
        assert_eq!(from_eq.pair(), from_playlist.pair());
        assert_eq!(from_eq.pair(), (EQ, PLAYLIST));
        // And an edge with nothing on it stays empty.
        assert!(seam_on(&s, MAIN, Edge::Top).is_none());
        assert!(seam_on(&s, EQ, Edge::Right).is_none());
    }

    // ---- D30 quantisation ---------------------------------------------------

    fn eq_playlist_bond() -> Bond {
        Bond::new(EQ, PLAYLIST, Edge::Bottom, (0, 275))
    }

    #[test]
    fn the_seam_only_stops_on_a_legal_playlist_height() {
        // D30: every valid playlist size is 116 + 29m tall. The seam jumps.
        let s = stacked();
        let b = eq_playlist_bond();
        let bottom = s.layout[&PLAYLIST].bottom();
        for raw in 150..260 {
            let pos = quantize_seam(&s.layout, &b, raw, 1.0, 1.0);
            let height = bottom - pos;
            assert_eq!(
                (height - 116) % 29,
                0,
                "raw {raw} produced an illegal playlist height {height}"
            );
            assert!(height >= 116, "raw {raw} went under the base height");
        }
    }

    #[test]
    fn quantisation_never_drifts_however_far_the_seam_travels() {
        // D40 at 150%, where 116 * 1.5 = 174 and 29 * 1.5 = 43.5 -- the step is
        // not an integer, so anything that adds a rounded physical step to a
        // previous physical value walks off. Recomputing from the logical base
        // does not.
        let s = stacked();
        let b = eq_playlist_bond();
        let bottom = s.layout[&PLAYLIST].bottom();
        for m in 0..40 {
            let want = bond::d40::stepped(CHROME_H, PLAYLIST_STEP_H, m, 1.5);
            // Aim the cursor exactly at the seam position for m steps, plus a
            // pixel of hand tremor in each direction.
            for jitter in [-1, 0, 1] {
                let pos = quantize_seam(&s.layout, &b, bottom - want + jitter, 1.5, 1.0);
                assert_eq!(bottom - pos, want, "step {m} jitter {jitter} drifted");
            }
        }
    }

    #[test]
    fn a_seam_with_nothing_resizable_is_left_where_it_is() {
        let s = stacked();
        let b = Bond::new(MAIN, EQ, Edge::Bottom, (0, 275));
        assert_eq!(quantize_seam(&s.layout, &b, 173, 1.0, 1.0), 173);
    }

    // ---- demagnetize --------------------------------------------------------

    #[test]
    fn breaking_the_middle_of_a_chain_makes_two_groups() {
        // This is the case a flat list of groups cannot represent: the break is
        // in the middle, so one group has to become two and nothing in a flat
        // list knows where the split is.
        let mut s = stacked();
        assert_eq!(s.graph.components(&CLASSIC).len(), 1);
        assert!(s.graph.break_bond(EQ, PLAYLIST));
        let comps = s.graph.components(&CLASSIC);
        assert_eq!(comps.len(), 2);
        assert_eq!(comps[0], vec![MAIN, EQ]);
        assert_eq!(comps[1], vec![PLAYLIST]);

        // D41: each side gets its own hidden root, and no real window becomes
        // an owner.
        let plan = plan_ownership(&s, Some(PLAYLIST));
        let root_of = |id: WindowId| {
            plan.owners
                .iter()
                .find(|(w, _)| *w == s.handle(id))
                .unwrap()
                .1
        };
        assert_eq!(root_of(MAIN), root_of(EQ));
        assert_ne!(root_of(MAIN), root_of(PLAYLIST));
        assert!(plan.owners.iter().all(|(_, o)| s.roots.contains(o)));
    }

    #[test]
    fn a_break_leaves_the_other_seam_alone() {
        let mut s = stacked();
        s.graph.break_bond(EQ, PLAYLIST);
        assert_eq!(edges_for(&s, MAIN).bottom, Some(false));
        assert_eq!(edges_for(&s, EQ).top, Some(false));
        // The broken edge is gone from both sides, not just the one clicked.
        assert_eq!(edges_for(&s, EQ).bottom, None);
        assert_eq!(edges_for(&s, PLAYLIST).top, None);
    }

    #[test]
    fn breaking_a_bond_that_is_not_there_changes_nothing() {
        let mut s = stacked();
        let before = s.graph.bonds.len();
        assert!(!s.graph.break_bond(MAIN, PLAYLIST));
        assert_eq!(s.graph.bonds.len(), before);
    }

    #[test]
    fn a_broken_seam_can_be_re_formed_by_dragging_back() {
        // Stage 5's "rebond carries no stale state", from the app's side: after
        // a break the two windows are still flush, and dragging one back onto
        // the other has to produce a clean single bond rather than a duplicate.
        let mut s = stacked();
        s.graph.break_bond(EQ, PLAYLIST);
        for b in bonds_after_drag(&s.layout, &[PLAYLIST], &[MAIN, EQ]) {
            s.graph.insert(b);
        }
        assert_eq!(s.graph.bonds.len(), 2);
        assert_eq!(s.graph.components(&CLASSIC).len(), 1);
        assert!(bond::violations(&s.graph, &s.layout).is_empty());
    }

    #[test]
    fn a_click_is_not_a_drag_and_re_forms_nothing() {
        // #10: after a demagnetise the windows are still flush. A click on a
        // title bar is a zero-length drag; it must not put the bond back.
        let mut s = stacked();
        s.graph.break_bond(EQ, PLAYLIST);
        let origin = s.layout.clone();
        assert!(bonds_on_release(&origin, &s.layout, &[PLAYLIST], &[MAIN, EQ]).is_empty());
        // The same position reached by a drag that moved does bond.
        let mut away = origin.clone();
        away.get_mut(&PLAYLIST).unwrap().x += 30;
        assert_eq!(
            bonds_on_release(&away, &s.layout, &[PLAYLIST], &[MAIN, EQ]).len(),
            1
        );
    }

    #[test]
    fn the_opening_stack_is_flush_and_bonded_at_any_scale() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let (layout, graph) = initial_layout(scale, 1.0);
            assert_eq!(
                graph.components(&CLASSIC).len(),
                1,
                "scale {scale} came up split"
            );
            assert!(
                bond::violations(&graph, &layout).is_empty(),
                "scale {scale} opened with a gap in the seam"
            );
            // D38/D40: every dimension recomputed from the logical base, so
            // 275 x 1.5 is 413 and not whatever the toolkit would have rounded.
            let w = bond::d40::physical(CHROME_W, scale);
            assert!(layout.values().all(|r| r.w == w));
        }
    }

    #[test]
    fn a_seeded_bond_the_os_disagrees_with_is_dropped() {
        // D58 as register() applies it: the seeded graph is intent, and intent
        // is not evidence. If the OS put a window somewhere else, the bond has
        // to go -- a graph that stays self-consistent while describing a layout
        // existing nowhere is exactly what that decision is about.
        let (mut layout, graph) = initial_layout(1.0, 1.0);
        assert!(bond::violations(&graph, &layout).is_empty());
        let moved = layout[&PLAYLIST].translated(0, 7);
        layout.insert(PLAYLIST, moved);
        let bad = bond::violations(&graph, &layout);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0.pair(), (EQ, PLAYLIST));
    }

    // ---- windowshade (D60/D61) ----------------------------------------------

    #[test]
    fn shading_collapses_in_place_and_the_group_follows() {
        let s = stacked();
        let mut layout = s.layout.clone();
        apply_shade(&mut layout, &s.graph, EQ, 14);

        // The window you clicked does not move, and neither does anything
        // above it. Only the height changes.
        assert_eq!(layout[&MAIN], s.layout[&MAIN]);
        assert_eq!(layout[&EQ].x, s.layout[&EQ].x);
        assert_eq!(layout[&EQ].y, s.layout[&EQ].y);
        assert_eq!(layout[&EQ].h, 14);
        // Everything below slides up by the difference.
        assert_eq!(layout[&PLAYLIST].y, s.layout[&PLAYLIST].y - 102);
        assert!(bond::violations(&s.graph, &layout).is_empty());
    }

    #[test]
    fn shading_the_top_window_pulls_the_whole_stack_up() {
        let s = stacked();
        let mut layout = s.layout.clone();
        apply_shade(&mut layout, &s.graph, MAIN, 14);
        assert_eq!(layout[&MAIN].y, s.layout[&MAIN].y);
        assert_eq!(layout[&EQ].y, s.layout[&EQ].y - 102);
        assert_eq!(layout[&PLAYLIST].y, s.layout[&PLAYLIST].y - 102);
        assert!(bond::violations(&s.graph, &layout).is_empty());
    }

    #[test]
    fn shading_the_bottom_window_moves_nothing_else() {
        let s = stacked();
        let mut layout = s.layout.clone();
        apply_shade(&mut layout, &s.graph, PLAYLIST, 14);
        assert_eq!(layout[&MAIN], s.layout[&MAIN]);
        assert_eq!(layout[&EQ], s.layout[&EQ]);
        assert_eq!(layout[&PLAYLIST].h, 14);
        assert!(bond::violations(&s.graph, &layout).is_empty());
    }

    #[test]
    fn a_shade_and_an_expand_round_trip_exactly() {
        let s = stacked();
        let mut layout = s.layout.clone();
        apply_shade(&mut layout, &s.graph, EQ, 14);
        apply_shade(&mut layout, &s.graph, EQ, 116);
        assert_eq!(
            layout, s.layout,
            "the stack did not come back to where it was"
        );
    }

    #[test]
    fn an_unbonded_window_shades_without_disturbing_anyone() {
        let mut s = stacked();
        s.graph = WindowGraph::new();
        let mut layout = s.layout.clone();
        apply_shade(&mut layout, &s.graph, MAIN, 14);
        assert_eq!(layout[&EQ], s.layout[&EQ]);
        assert_eq!(layout[&PLAYLIST], s.layout[&PLAYLIST]);
    }

    #[test]
    fn expanding_restores_a_resized_playlist_rather_than_the_base_height() {
        // The playlist can sit at any legal D30 size. Coming back from a shade
        // has to return the height it had, not 116 -- silently discarding a
        // resize on a double-click would be data loss the user did not ask for.
        let mut s = stacked();
        s.unshaded_h.insert(PLAYLIST, 174);
        assert_eq!(height_for(&s, PLAYLIST, false), 174);
        assert_eq!(height_for(&s, PLAYLIST, true), 14);
        // A window that has never been shaded falls back to the base height.
        assert_eq!(height_for(&s, EQ, false), 116);
    }

    #[test]
    fn the_shade_strip_is_taller_on_a_scaled_display() {
        // D51: rendered geometry resolves from the window's OWN monitor, and
        // the 14px strip really is physically taller at 150%.
        let mut s = stacked();
        s.monitors = vec![mon(0, 0, 2560, 1440, 1.5), mon(2560, 0, 1920, 1080, 1.0)];
        s.layout.insert(MAIN, Rect::new(100, 100, 413, 174));
        assert_eq!(height_for(&s, MAIN, true), 21);
        s.layout.insert(MAIN, Rect::new(3000, 100, 275, 116));
        assert_eq!(height_for(&s, MAIN, true), 14);
    }

    #[test]
    fn shading_main_floats_the_whole_group() {
        // D61. Topmost is per-window and does not follow ownership, so lifting
        // only main would leave its bonded neighbours behind other apps.
        let mut s = stacked();
        assert!(topmost_set(&s).is_empty());
        s.shaded.insert(MAIN);
        assert_eq!(topmost_set(&s), vec![MAIN, EQ, PLAYLIST]);
    }

    #[test]
    fn a_detached_main_floats_alone() {
        let mut s = stacked();
        s.graph.break_bond(MAIN, EQ);
        s.shaded.insert(MAIN);
        assert_eq!(topmost_set(&s), vec![MAIN]);
    }

    #[test]
    fn shading_anything_other_than_main_floats_nothing() {
        // The mini-player is the Main shade specifically. A shaded equalizer is
        // just a shaded equalizer.
        let mut s = stacked();
        s.shaded.insert(EQ);
        s.shaded.insert(PLAYLIST);
        assert!(topmost_set(&s).is_empty());
    }

    // ---- rescue (D57) -------------------------------------------------------

    #[test]
    fn a_window_hanging_off_the_edge_is_still_on_screen() {
        // Intersection, not containment. A window half off the right-hand edge
        // is reachable, and a rescue that hauled it back would be undoing
        // something the user did on purpose.
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        assert!(is_on_screen(Rect::new(1800, 100, 275, 116), &ms));
        assert!(is_on_screen(Rect::new(-100, 100, 275, 116), &ms));
        assert!(!is_on_screen(Rect::new(-400, 100, 275, 116), &ms));
        // The minimized rect D57 measured on a real cable-pull.
        assert!(!is_on_screen(Rect::new(-32000, -32000, 160, 28), &ms));
    }

    #[test]
    fn nothing_is_on_screen_when_there_are_no_screens() {
        assert!(!is_on_screen(Rect::new(0, 0, 275, 116), &[]));
        assert_eq!(nearest_monitor(&[], Rect::new(0, 0, 1, 1)), None);
    }

    #[test]
    fn the_nearest_surviving_display_wins() {
        let ms = two_monitors();
        // Just off the left-hand display's top-left.
        assert_eq!(
            nearest_monitor(&ms, Rect::new(-500, 0, 275, 116)),
            Some(ms[0])
        );
        // Out beyond the right-hand one.
        assert_eq!(
            nearest_monitor(&ms, Rect::new(5000, 500, 275, 116)),
            Some(ms[1])
        );
    }

    #[test]
    fn a_rescue_is_a_rigid_translation_with_no_bond_violations() {
        // The whole reason the rescue is a translation: a rigid move cannot
        // change any relative position, so it provably cannot open a seam.
        // Measured on the spike as 0 violations; here it is structural.
        let s = stacked();
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        let mut stranded = s.layout.clone();
        bond::translate_group(&mut stranded, &CLASSIC, -32000, -32000);

        let out = rescue_layout(&stranded, &s.graph, &ms);
        assert!(bond::violations(&s.graph, &out).is_empty());
        // Offsets preserved exactly.
        assert_eq!(out[&EQ].y - out[&MAIN].y, 116);
        assert_eq!(out[&PLAYLIST].y - out[&EQ].y, 116);
        // And it landed somewhere real.
        assert!(CLASSIC.iter().all(|id| is_on_screen(out[id], &ms)));
    }

    #[test]
    fn a_rescue_leaves_reachable_groups_alone() {
        let s = stacked();
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        assert_eq!(rescue_layout(&s.layout, &s.graph, &ms), s.layout);
    }

    #[test]
    fn only_the_stranded_group_is_moved() {
        let mut s = stacked();
        s.graph.break_bond(EQ, PLAYLIST);
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        let mut layout = s.layout.clone();
        bond::translate_group(&mut layout, &[PLAYLIST], -32000, -32000);

        let out = rescue_layout(&layout, &s.graph, &ms);
        assert_eq!(
            out[&MAIN], s.layout[&MAIN],
            "an on-screen group was disturbed"
        );
        assert_eq!(out[&EQ], s.layout[&EQ], "an on-screen group was disturbed");
        assert!(is_on_screen(out[&PLAYLIST], &ms));
    }

    #[test]
    fn with_no_displays_at_all_nothing_is_invented() {
        // Every display gone is not a case to guess at: leave the model alone
        // and wait for one to come back.
        let s = stacked();
        assert_eq!(rescue_layout(&s.layout, &s.graph, &[]), s.layout);
    }

    // ---- confined to the work area (D182) -----------------------------------

    /// The GNOME laptop the Linux spike ran on: the dock on the left, the top
    /// bar along the top.
    fn gnome() -> Vec<MonitorInfo> {
        vec![MonitorInfo {
            rect: Rect::new(0, 0, 1920, 1080),
            scale: 1.0,
            work: Rect::new(67, 32, 1853, 1048),
        }]
    }

    #[test]
    fn a_group_past_the_dock_comes_inside_the_work_area_whole() {
        let s = stacked();
        let ms = gnome();
        let mut l = s.layout.clone();
        bond::translate_group(&mut l, &CLASSIC, -15, 100);
        // Reach is satisfied, since every bar keeps a grab inside; on Windows
        // that is where the group stays. Mutter would put each window at
        // x = 67 on its own; the engine moves the group there first.
        assert_eq!(reach_clamp(&l, &CLASSIC, &ms, 1.0), (0, 0));
        assert_eq!(confine_clamp(&l, &CLASSIC, &ms), (67 + 15, 0));
        let out = keep_layout_in_reach(&l, &s.graph, &ms, 1.0, true);
        assert!(CLASSIC.iter().all(|id| out[id].x == 67));
        assert!(bond::violations(&s.graph, &out).is_empty());
        assert_eq!(keep_layout_in_reach(&l, &s.graph, &ms, 1.0, false), l);
    }

    #[test]
    fn a_doubled_stack_near_the_bottom_rises_whole_rather_than_shearing() {
        // The owner's 2x at the bottom right: Mutter left the EQ and the
        // playlist overlapping by 204 px.
        let mut l = Layout::new();
        for (i, id) in CLASSIC.iter().enumerate() {
            l.insert(*id, Rect::new(1370, 820 + 232 * i as Px, 550, 232));
        }
        let s = stacked();
        let out = keep_layout_in_reach(&l, &s.graph, &gnome(), 2.0, true);
        assert_eq!(out[&PLAYLIST].bottom(), 32 + 1048);
        assert_eq!(out[&MAIN].y, 32 + 1048 - 3 * 232);
        assert!(CLASSIC.iter().all(|id| out[id].x == 1370));
        assert!(bond::violations(&s.graph, &out).is_empty());
    }

    #[test]
    fn a_group_may_straddle_two_work_areas_side_by_side() {
        // Stage 6's arrangement as X saw it at scale 2: the panel with its
        // dock and bar, the external display to its right with neither.
        let ms = vec![
            MonitorInfo {
                rect: Rect::new(0, 0, 3840, 2160),
                scale: 2.0,
                work: Rect::new(134, 64, 3706, 2096),
            },
            mon(3840, 0, 5120, 2880, 2.0),
        ];
        let mut l = Layout::new();
        for (i, id) in CLASSIC.iter().enumerate() {
            l.insert(*id, Rect::new(3600, 500 + 232 * i as Px, 550, 232));
        }
        assert_eq!(confine_clamp(&l, &CLASSIC, &ms), (0, 0));
        // Off the far side of the external display it comes back.
        bond::translate_group(&mut l, &CLASSIC, 5500, 0);
        assert_eq!(
            confine_clamp(&l, &CLASSIC, &ms).0,
            3840 + 5120 - (3600 + 5500 + 550)
        );
    }

    // ---- a drag across scales (D187) ----------------------------------------

    /// The owner's Windows desk: DISPLAY1 2560 x 1080 at 100 %, primary, and
    /// DISPLAY2 3840 x 2160 at 150 % to its right.
    fn desk() -> Vec<MonitorInfo> {
        vec![mon(0, 0, 2560, 1080, 1.0), mon(2560, 0, 3840, 2160, 1.5)]
    }

    /// The default stack at `zoom` on DISPLAY1, its playlist `steps` taller,
    /// translated by (dx, dy) as a drag would.
    fn stack_moved(zoom: f64, steps: i32, dx: Px, dy: Px) -> (Layout, Layout, WindowGraph) {
        let (mut origin, g) = initial_layout(1.0, zoom);
        let pl = origin[&PLAYLIST];
        let step = bond::d40::physical(PLAYLIST_STEP_H * zoom, 1.0);
        origin.insert(PLAYLIST, Rect::new(pl.x, pl.y, pl.w, pl.h + steps * step));
        let mut now = origin.clone();
        bond::translate_group(&mut now, &CLASSIC, dx, dy);
        (origin, now, g)
    }

    /// Each window's (where it began, where it is) scale, by the display
    /// holding most of it at each.
    fn by_area(origin: &Layout, now: &Layout) -> BTreeMap<WindowId, (f64, f64)> {
        let at = |r: &Rect| dpi_monitor(&desk(), *r).unwrap().scale;
        CLASSIC
            .iter()
            .map(|id| (*id, (at(&origin[id]), at(&now[id]))))
            .collect()
    }

    fn fit(origin: &Layout, now: &Layout, g: &WindowGraph, zoom: f64) -> Option<Layout> {
        fit_to_displays(
            now,
            &CLASSIC,
            g,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &by_area(origin, now),
            zoom,
        )
        .map(|(l, _)| l)
    }

    /// `settle_scales` over a drag frame, as `drag_layout` runs it: every
    /// window drawn at `from` where the drag began and still, the read-back
    /// not yet in.
    fn settled(
        frame: &Layout,
        g: &WindowGraph,
        from: f64,
        zoom: f64,
    ) -> (BTreeMap<WindowId, f64>, Layout) {
        let now: BTreeMap<WindowId, f64> = CLASSIC.iter().map(|id| (*id, from)).collect();
        let (t, l, _) = settle_scales(&CLASSIC, &now, &desk(), |t| {
            let scales = CLASSIC.iter().map(|id| (*id, (from, t[id]))).collect();
            fit_to_displays(
                frame,
                &CLASSIC,
                g,
                &BTreeSet::new(),
                &BTreeMap::new(),
                &scales,
                zoom,
            )
            .unwrap_or_else(|| (frame.clone(), BTreeMap::new()))
        });
        (t, l)
    }

    #[test]
    fn a_window_split_evenly_goes_to_the_first_display() {
        // The owner's drop at the 50/50 point: Main at x = 2285, 275 on each
        // side. Windows kept it at 100 %; the engine said 150 %. And from
        // 150 % on the same tie, D193's probes found Windows giving 100 %.
        let ms = desk();
        let even = Rect::new(2285, 68, 550, 232);
        assert_eq!(judged_scale(&ms, even, 1.0), 1.0);
        assert_eq!(judged_scale(&ms, even, 1.5), 1.0);
        assert_eq!(dpi_monitor(&ms, even).unwrap().scale, 1.0);
        assert_eq!(judged_scale(&ms, Rect::new(2286, 68, 550, 232), 1.0), 1.5);
        // So the stack dropped there stays the size it is, with no gaps.
        let (_, frame, g) = stack_moved(2.0, 0, 2285 - 120, 0);
        let (t, out) = settled(&frame, &g, 1.0, 2.0);
        assert!(t.values().all(|s| *s == 1.0), "{t:?}");
        assert_eq!(out, frame);
    }

    #[test]
    fn the_playlist_takes_its_scale_from_where_the_re_pack_puts_it() {
        // The owner's sweep left: an 825-wide stack on 150 % brought back far
        // enough that Main and the EQ are mostly on 100 %. Before the re-pack
        // the playlist hangs below DISPLAY1's bottom edge, where only DISPLAY2
        // is; after it, it sits beside them, mostly on DISPLAY1.
        let (start, g) = initial_layout(1.5, 2.0);
        let mut frame = start.clone();
        let pl = frame[&PLAYLIST];
        let step = bond::d40::physical(PLAYLIST_STEP_H * 2.0, 1.5);
        frame.insert(PLAYLIST, Rect::new(pl.x, pl.y, pl.w, pl.h + 3 * step));
        let m = frame[&MAIN];
        bond::translate_group(&mut frame, &CLASSIC, 2100 - m.x, 68 - m.y);
        assert_eq!(dpi_monitor(&desk(), frame[&PLAYLIST]).unwrap().scale, 1.5);
        let (t, out) = settled(&frame, &g, 1.5, 2.0);
        assert!(t.values().all(|s| *s == 1.0), "{t:?}");
        assert_eq!((out[&MAIN].w, out[&MAIN].h), (550, 232));
        assert_eq!(
            (out[&PLAYLIST].w, out[&PLAYLIST].h),
            (
                550,
                bond::d40::stepped(CHROME_H * 2.0, PLAYLIST_STEP_H * 2.0, 3, 1.0)
            )
        );
        assert!(bond::violations(&g, &out).is_empty());
    }

    #[test]
    fn a_re_zoom_on_the_seam_starts_from_the_scale_windows_draws_at() {
        // The owner's break: 1x to 2x at the foot of DISPLAY1, on the seam.
        // Main's top-left is on DISPLAY1 but most of it is on DISPLAY2, so it
        // is drawn at 150 %, 413 wide. The scale under its corner said 100 %:
        // 550, from a 413 read as 100 %, at 2x drawn at 150 %.
        let (mut l, g) = initial_layout(1.5, 1.0);
        let m = l[&MAIN];
        bond::translate_group(&mut l, &CLASSIC, 2560 - 100 - m.x, 1080 - 3 * 174 - m.y);
        let s = WmState {
            layout: l,
            graph: g,
            monitors: desk(),
            scale: 1.0,
            ..Default::default()
        };
        let now: BTreeMap<WindowId, f64> = CLASSIC.iter().map(|id| (*id, 1.5)).collect();
        let (t, out, (g2, _)) = rezoom_settled(&s, &now, &now, 1.0, 2.0);
        assert!(t.values().all(|s| *s == 1.5), "{t:?}");
        for id in CLASSIC {
            assert_eq!((out[&id].w, out[&id].h), (825, 348), "{id:?}");
        }
        assert!(bond::violations(&g2, &out).is_empty());
    }

    #[test]
    fn a_stack_doubled_at_the_foot_of_a_display_is_lifted_back_on() {
        // The owner's 1x to 2x on the seam at DISPLAY1's foot: Main mostly
        // on DISPLAY1, the stack reaching DISPLAY1's bottom edge.
        let (mut l, g) = initial_layout(1.0, 1.0);
        let m = l[&MAIN];
        bond::translate_group(&mut l, &CLASSIC, 2560 - 400 - m.x, 1080 - 3 * 116 - m.y);
        let s = WmState {
            layout: l,
            graph: g,
            monitors: desk(),
            scale: 1.0,
            ..Default::default()
        };
        let now: BTreeMap<WindowId, f64> = CLASSIC.iter().map(|id| (*id, 1.0)).collect();
        let (t, out, (g2, _)) = rezoom_settled(&s, &now, &now, 1.0, 2.0);
        assert!(t.values().all(|s| *s == 1.0), "{t:?}");
        assert_eq!(out[&PLAYLIST].bottom(), 1080, "lifted, not shoved right");
        assert_eq!(out[&MAIN].x, s.layout[&MAIN].x);
        assert!(bond::violations(&g2, &out).is_empty());
    }

    #[test]
    fn a_group_already_partly_off_the_displays_stays_where_it_was_put() {
        let (mut before, g) = initial_layout(1.0, 1.0);
        let m = before[&MAIN];
        bond::translate_group(&mut before, &CLASSIC, 100 - m.x, 1000 - m.y);
        let mut after = before.clone();
        bond::translate_group(&mut after, &[PLAYLIST], 0, 50);
        assert_eq!(
            lift_onto_displays(&before, &after, &g, &desk(), &|_| true),
            after
        );
    }

    #[test]
    fn sizes_are_read_at_the_scale_they_were_laid_out_at_not_the_one_drawn() {
        // The owner's second way into C: the engine has the stack at 150 %,
        // 825 wide; Windows has put it at 100 % before the engine heard. The
        // next drag lays it out from 150 % (D190), so it comes out 550 where
        // reading 825 as 100 % gave a playlist steps too big, and a Main 413
        // wide at 100 %.
        let (mut frame, g) = initial_layout(1.5, 2.0);
        let m = frame[&MAIN];
        bond::translate_group(&mut frame, &CLASSIC, 1200 - m.x, 100 - m.y);
        let now: BTreeMap<WindowId, f64> = CLASSIC.iter().map(|id| (*id, 1.0)).collect();
        let (t, out, _) = settle_scales(&CLASSIC, &now, &desk(), |t| {
            let scales = CLASSIC.iter().map(|id| (*id, (1.5, t[id]))).collect();
            fit_to_displays(
                &frame,
                &CLASSIC,
                &g,
                &BTreeSet::new(),
                &BTreeMap::new(),
                &scales,
                2.0,
            )
            .unwrap()
        });
        assert!(t.values().all(|s| *s == 1.0), "{t:?}");
        for id in CLASSIC {
            assert_eq!((out[&id].w, out[&id].h), (550, 232), "{id:?}");
        }
        assert!(bond::violations(&g, &out).is_empty());
    }

    #[test]
    fn a_stack_parked_over_the_taskbar_is_lifted_clear_of_it() {
        // The owner's call: resting on the screen's bottom edge, over the
        // taskbar, is on screen. Doubled, it is lifted onto the work area.
        let ms = vec![mon_tb(0, 0, 2560, 1080, 1.0, 48)];
        let (mut before, g) = initial_layout(1.0, 1.0);
        let m = before[&MAIN];
        bond::translate_group(&mut before, &CLASSIC, 300 - m.x, 1080 - 3 * 116 - m.y);
        let mut after = before.clone();
        for (i, id) in CLASSIC.iter().enumerate() {
            after.insert(
                *id,
                Rect::new(300, before[&MAIN].y + i as Px * 232, 550, 232),
            );
        }
        let out = lift_onto_displays(&before, &after, &g, &ms, &|_| true);
        assert_eq!(out[&PLAYLIST].bottom(), 1080 - 48);
        assert!(bond::violations(&g, &out).is_empty());
    }

    #[test]
    fn a_size_is_read_at_the_scale_it_was_derived_at_not_the_one_reported() {
        // D190: Windows has the stack at 100 % before the engine heard; the
        // sizes are still the 150 % ones, and are read so.
        let drawn = BTreeMap::from([(MAIN, 1.5), (EQ, 1.5)]);
        let now = BTreeMap::from([(MAIN, 1.0), (EQ, 1.0), (PLAYLIST, 1.0)]);
        assert_eq!(
            laid_out_at_in(&drawn, &now, true),
            BTreeMap::from([(MAIN, 1.5), (EQ, 1.5), (PLAYLIST, 1.0)]),
            "a window not derived yet is at the scale it is drawn at"
        );
        // Where one scale covers the desktop the engine's answer is the only
        // one, and a recorded scale is never consulted.
        assert_eq!(laid_out_at_in(&drawn, &now, false), now);
        assert_eq!(derived_scale_in(&drawn, MAIN, 1.0, true), 1.5);
        assert_eq!(derived_scale_in(&drawn, MAIN, 1.0, false), 1.0);
        assert_eq!(derived_scale_in(&drawn, PLAYLIST, 1.25, true), 1.25);
    }

    #[test]
    fn a_re_zoom_starts_from_the_scale_the_sizes_are_in() {
        // The stack's sizes are 150 %'s (413 wide at 1x) and Windows has
        // already put it at 100 % on DISPLAY1. Doubled from 150 % it is
        // 550 x 232 there; read at 100 % it was 826, a Main 1.5 times too big.
        let (mut l, g) = initial_layout(1.5, 1.0);
        let m = l[&MAIN];
        bond::translate_group(&mut l, &CLASSIC, 300 - m.x, 100 - m.y);
        let s = WmState {
            layout: l,
            graph: g,
            monitors: desk(),
            scale: 1.0,
            ..Default::default()
        };
        let from: BTreeMap<WindowId, f64> = CLASSIC.iter().map(|id| (*id, 1.5)).collect();
        let now: BTreeMap<WindowId, f64> = CLASSIC.iter().map(|id| (*id, 1.0)).collect();
        let (t, out, (g2, _)) = rezoom_settled(&s, &from, &now, 1.0, 2.0);
        assert!(t.values().all(|s| *s == 1.0), "{t:?}");
        for id in CLASSIC {
            assert_eq!((out[&id].w, out[&id].h), (550, 232), "{id:?}");
        }
        assert!(bond::violations(&g2, &out).is_empty());
    }

    /// Main at 100 % on DISPLAY1 and the playlist docked to its right on
    /// DISPLAY2 at 150 %, three steps tall, at 2x: a group across the seam.
    fn straddle() -> (Layout, WindowGraph) {
        let l = Layout::from([
            (MAIN, Rect::new(2010, 100, 550, 232)),
            (PLAYLIST, Rect::new(2560, 100, 825, 609)),
        ]);
        let mut g = WindowGraph::new();
        g.insert(Bond::new(MAIN, PLAYLIST, Edge::Right, (100, 332)));
        (l, g)
    }

    #[test]
    fn a_group_across_the_seam_is_saved_at_its_own_scales_and_left_alone() {
        let (l, g) = straddle();
        let stored = BTreeMap::from([(MAIN, 1.0), (PLAYLIST, 1.5)]);
        assert!(heal_saved_sizes(&l, &g, &desk(), 1.0, 2.0, &stored, &BTreeSet::new()).is_none());
        // A row from before the scale was kept: the playlist's size is one it
        // can have at 150 % and not at Main's 100 %, so it is read as 150 %.
        // Main's width said 100 % for it, and it grew steps every launch.
        assert!(heal_saved_sizes(
            &l,
            &g,
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new()
        )
        .is_none());
        // The mirror, a playlist at 100 % beside Main at 150 %, is not shrunk.
        let mirror = Layout::from([
            (PLAYLIST, Rect::new(2010, 100, 550, 406)),
            (MAIN, Rect::new(2560, 100, 825, 348)),
        ]);
        let mut mg = WindowGraph::new();
        mg.insert(Bond::new(PLAYLIST, MAIN, Edge::Right, (100, 448)));
        assert!(heal_saved_sizes(
            &mirror,
            &mg,
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new()
        )
        .is_none());
    }

    #[test]
    fn a_stored_scale_decides_a_playlist_size_that_fits_two_scales() {
        // 900 x 348 at 2x is one width step at 150 % and seven at 100 %, and
        // no height step at 150 % or two at 100 %: a size both can have.
        let (mut l, g) = straddle();
        l.insert(PLAYLIST, Rect::new(2560, 100, 900, 348));
        assert!(playlist_fits(l[&PLAYLIST], 1.0, 2.0) && playlist_fits(l[&PLAYLIST], 1.5, 2.0));
        // An older row: it fits where it is, so it was sized there. Read at
        // Main's 100 % it grew to 1350 x 522.
        assert!(heal_saved_sizes(
            &l,
            &g,
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new()
        )
        .is_none());
        // Saved at 100 %'s size on DISPLAY2, its row saying so: healed to
        // 150 %, its seven steps and two kept.
        let stored = BTreeMap::from([(MAIN, 1.0), (PLAYLIST, 1.0)]);
        let (out, _) =
            heal_saved_sizes(&l, &g, &desk(), 1.0, 2.0, &stored, &BTreeSet::new()).expect("wrong");
        assert_eq!((out[&PLAYLIST].w, out[&PLAYLIST].h), (1350, 522));
        assert_eq!(out[&MAIN], l[&MAIN]);
    }

    #[test]
    fn a_playlist_alone_saved_at_another_scale_heals_from_its_own_size() {
        // v1.5: the playlist dragged alone onto DISPLAY2 kept 100 %'s
        // 550 x 406. With no Main or EQ to read, nothing healed it.
        let l = Layout::from([(PLAYLIST, Rect::new(3000, 100, 550, 406))]);
        let (out, _) = heal_saved_sizes(
            &l,
            &WindowGraph::new(),
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new(),
        )
        .expect("it was wrong");
        assert_eq!((out[&PLAYLIST].w, out[&PLAYLIST].h), (825, 609));
    }

    #[test]
    fn a_stack_saved_on_a_tie_comes_back_on_the_first_display() {
        // Main split 275 / 275 at x = 2285, which Windows kept at 100 %.
        let (mut l, g) = initial_layout(1.0, 2.0);
        let m = l[&MAIN];
        bond::translate_group(&mut l, &CLASSIC, 2285 - m.x, 100 - m.y);
        let stored: BTreeMap<WindowId, f64> = CLASSIC.iter().map(|id| (*id, 1.0)).collect();
        assert!(heal_saved_sizes(&l, &g, &desk(), 1.0, 2.0, &stored, &BTreeSet::new()).is_none());
        assert_eq!(launch_scale(&desk(), l[&MAIN], Some(1.0), 1.0), 1.0);
        // Whatever it was saved at: Windows gives the first display.
        assert_eq!(launch_scale(&desk(), l[&MAIN], Some(1.5), 1.5), 1.0);
        // An older row the same: the first of the largest is DISPLAY1.
        assert_eq!(launch_scale(&desk(), l[&MAIN], None, 1.5), 1.0);
    }

    #[test]
    fn the_playlist_fits_the_sizes_its_steps_give_at_a_scale() {
        assert!(playlist_fits(Rect::new(0, 0, 825, 609), 1.5, 2.0));
        assert!(!playlist_fits(Rect::new(0, 0, 825, 609), 1.0, 2.0));
        assert!(playlist_fits(Rect::new(0, 0, 550, 406), 1.0, 2.0));
        assert!(!playlist_fits(Rect::new(0, 0, 550, 406), 1.5, 2.0));
    }

    #[test]
    fn a_gesture_beginning_clears_any_other_left_behind() {
        // D191: a splitter whose end never came kept the reconcile waiting.
        let mut s = stacked();
        s.drag = Some(DragState {
            moving: CLASSIC.to_vec(),
            origin_layout: s.layout.clone(),
            origin_cursor: (0, 0),
            origin_unshaded: BTreeMap::new(),
            origin_scales: BTreeMap::new(),
            grabbed: MAIN,
        });
        s.splitter = Some(SplitterState {
            bond: *s.graph.bond_between(MAIN, EQ).unwrap(),
            origin_layout: s.layout.clone(),
        });
        s.resize = Some(ResizeState {
            id: PLAYLIST,
            origin: s.layout[&PLAYLIST],
            origin_cursor: (0, 0),
            w_free: true,
            h_free: true,
        });
        begin_gesture(&mut s);
        assert!(s.splitter.is_none() && s.resize.is_none() && s.drag.is_none());
    }

    #[test]
    fn any_end_ends_every_gesture_and_says_whether_one_was_live() {
        // A seam ended as a move after Rust made it a splitter.
        let mut s = stacked();
        s.splitter = Some(SplitterState {
            bond: *s.graph.bond_between(EQ, PLAYLIST).unwrap(),
            origin_layout: s.layout.clone(),
        });
        assert!(end_gestures(&mut s));
        assert!(s.splitter.is_none());
        assert!(!end_gestures(&mut s), "nothing left to end");
    }

    #[test]
    fn a_start_heard_after_its_own_end_starts_nothing() {
        let mut s = stacked();
        assert!(begins(&s, EQ, 7));
        started(&mut s, EQ, 7);
        assert!(ends(&mut s, EQ, 7));
        assert!(!begins(&s, EQ, 7), "its end came first");
        // Two ends heard before a start that was overtaken by both.
        assert!(ends(&mut s, EQ, 8));
        assert!(ends(&mut s, EQ, 9));
        assert!(!begins(&s, EQ, 8));
        assert!(begins(&s, EQ, 10), "the next press is its own");
        assert!(begins(&s, MAIN, 7), "another window's press");
    }

    #[test]
    fn a_late_end_leaves_the_live_gesture_alone() {
        let mut s = stacked();
        started(&mut s, EQ, 1);
        // Press 2 in Main starts before press 1's end is heard.
        started(&mut s, MAIN, 2);
        assert!(!ends(&mut s, EQ, 1), "press 1's end is late");
        assert_eq!(s.live, Some((MAIN, 2)));
        assert!(ends(&mut s, MAIN, 2));
        assert_eq!(s.live, None);
        // With nothing live, an end still ends whatever Rust started.
        assert!(ends(&mut s, PLAYLIST, 3));
    }

    #[test]
    fn a_start_rust_refuses_does_not_take_the_live_press() {
        // D192: a grip Rust refuses (nothing to resize) is no gesture. The
        // lingering press it would have displaced still ends when its late
        // end comes.
        let mut s = stacked();
        start_if(&mut s, EQ, 1, true);
        assert!(begins(&s, PLAYLIST, 5), "not stale");
        start_if(&mut s, PLAYLIST, 5, false);
        assert_eq!(s.live, Some((EQ, 1)), "refused, so not live");
        assert!(ends(&mut s, EQ, 1));
        start_if(&mut s, PLAYLIST, 6, true);
        assert_eq!(s.live, Some((PLAYLIST, 6)));
    }

    /// The owner's desk with a 48 px taskbar on each display.
    fn desk_tb() -> Vec<MonitorInfo> {
        vec![
            mon_tb(0, 0, 2560, 1080, 1.0, 48),
            mon_tb(2560, 0, 3840, 2160, 1.5, 48),
        ]
    }

    #[test]
    fn following_a_scale_re_derives_that_window_and_closes_its_seams() {
        // The 48 px drag: the stack laid out at 150 % on the seam, Windows
        // has put all three at 100 %.
        let (mut l, g) = initial_layout(1.5, 1.0);
        let m = l[&MAIN];
        bond::translate_group(&mut l, &CLASSIC, 2410 - m.x, 549 - m.y);
        let mut s = WmState {
            layout: l,
            graph: g,
            monitors: desk_tb(),
            scale: 1.0,
            drawn_at: CLASSIC.iter().map(|id| (*id, 1.5)).collect(),
            ..Default::default()
        };
        let now: BTreeMap<WindowId, f64> = CLASSIC.iter().map(|id| (*id, 1.0)).collect();
        let t = std::time::Instant::now();
        let f = follow_scales(&mut s, &now, &BTreeSet::new(), t, true, false);
        assert!(f.rescaled && f.left.is_empty());
        for id in CLASSIC {
            assert_eq!((s.layout[&id].w, s.layout[&id].h), (275, 116), "{id:?}");
        }
        assert_eq!((s.layout[&MAIN].x, s.layout[&MAIN].y), (2410, 549));
        assert!(bond::violations(&s.graph, &s.layout).is_empty());
        assert_eq!(s.drawn_at, now);
        // And once the sizes are in the scale drawn, there is nothing to do.
        assert_eq!(
            follow_scales(&mut s, &now, &BTreeSet::new(), t, true, false),
            Followed::default()
        );
    }

    #[test]
    fn following_lifts_only_a_group_it_grew_off_the_displays() {
        // Main and the EQ at 100 % sizes resting on DISPLAY1's work area,
        // Windows puts them at 150 %: they grow down past the taskbar and are
        // lifted. The playlist, alone and parked over the taskbar, is not
        // theirs to move.
        let (mut l, mut g) = initial_layout(1.0, 1.0);
        g.break_bond(EQ, PLAYLIST);
        let m = l[&MAIN];
        bond::translate_group(&mut l, &[MAIN, EQ], 2300 - m.x, 1032 - 2 * 116 - m.y);
        l.insert(PLAYLIST, Rect::new(500, 1080 - 116, 275, 116));
        let mut s = WmState {
            layout: l.clone(),
            graph: g,
            monitors: desk_tb(),
            scale: 1.0,
            drawn_at: CLASSIC.iter().map(|id| (*id, 1.0)).collect(),
            ..Default::default()
        };
        let now = BTreeMap::from([(MAIN, 1.5), (EQ, 1.5), (PLAYLIST, 1.0)]);
        follow_scales(
            &mut s,
            &now,
            &BTreeSet::new(),
            std::time::Instant::now(),
            true,
            false,
        );
        assert_eq!(s.layout[&EQ].bottom(), 1032, "lifted clear of the taskbar");
        assert_eq!(s.layout[&MAIN].w, 413);
        assert_eq!(s.layout[&PLAYLIST], l[&PLAYLIST], "untouched");
    }

    #[test]
    fn a_window_flipped_six_times_is_left_and_the_others_still_followed() {
        let (mut l, mut g) = initial_layout(1.0, 1.0);
        g.break_bond(EQ, PLAYLIST);
        let p = l[&PLAYLIST];
        l.insert(PLAYLIST, Rect::new(p.x + 900, p.y, p.w, p.h));
        let t = std::time::Instant::now();
        let mut s = WmState {
            layout: l,
            graph: g,
            monitors: desk(),
            scale: 1.0,
            drawn_at: CLASSIC.iter().map(|id| (*id, 1.0)).collect(),
            reconciles: BTreeMap::from([(MAIN, vec![t; 6])]),
            ..Default::default()
        };
        let now = BTreeMap::from([(MAIN, 1.5), (EQ, 1.5), (PLAYLIST, 1.5)]);
        let iconic = BTreeSet::from([PLAYLIST]);
        let f = follow_scales(&mut s, &now, &iconic, t, true, false);
        assert!(f.rescaled);
        assert_eq!(f.left, BTreeSet::from([MAIN, PLAYLIST]));
        assert_eq!(s.drawn_at[&MAIN], 1.0, "the flipped window is left");
        assert_eq!(s.drawn_at[&PLAYLIST], 1.0, "a minimised one waits");
        assert_eq!(s.drawn_at[&EQ], 1.5);
        assert_eq!(s.layout[&EQ].w, 413);
        assert_eq!(s.layout[&MAIN].w, 275);
    }

    #[test]
    fn an_older_row_on_a_tie_comes_back_on_the_first_display() {
        // Saved before the scale was kept: Main's width says 100 %, and the
        // tie goes to DISPLAY1, as Windows did.
        let (mut l, g) = initial_layout(1.0, 2.0);
        let m = l[&MAIN];
        bond::translate_group(&mut l, &CLASSIC, 2285 - m.x, 100 - m.y);
        assert!(heal_saved_sizes(
            &l,
            &g,
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new()
        )
        .is_none());
        let sc = launch_scales(
            &l,
            &g,
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new(),
        );
        assert!(sc.values().all(|v| *v == (Some(1.0), 1.0)), "{sc:?}");
    }

    #[test]
    fn a_group_saved_wrong_whole_heals_its_playlist_with_it() {
        // v1.5's whole stack on DISPLAY2 at 100 % sizes; the playlist's
        // 900 x 348 fits 150 % as well, but its group says 100 %.
        let (mut l, g) = initial_layout(1.0, 2.0);
        let pl = l[&PLAYLIST];
        l.insert(PLAYLIST, Rect::new(pl.x, pl.y, 900, 348));
        bond::translate_group(&mut l, &CLASSIC, 2800, 0);
        let (out, _) = heal_saved_sizes(
            &l,
            &g,
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new(),
        )
        .expect("wrong");
        assert_eq!((out[&MAIN].w, out[&MAIN].h), (825, 348));
        assert_eq!((out[&PLAYLIST].w, out[&PLAYLIST].h), (1350, 522));
    }

    #[test]
    fn a_playlist_whose_scale_cannot_be_told_is_left_untold() {
        // Alone, at a size no display's scale gives it: nothing to heal from,
        // and no scale recorded for it, so it is told again next launch.
        let l = Layout::from([(PLAYLIST, Rect::new(3000, 100, 560, 400))]);
        let g = WindowGraph::new();
        let sc = launch_scales(
            &l,
            &g,
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new(),
        );
        assert_eq!(sc[&PLAYLIST].0, None);
        assert!(heal_saved_sizes(
            &l,
            &g,
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new()
        )
        .is_none());
    }

    #[test]
    fn a_shaded_window_is_judged_where_its_strip_will_be() {
        // D192: a shaded 2x Main at 100 % at (2200, 1000). Its strip, 550 x
        // 28, is mostly on DISPLAY1; its expanded rect, as the row stores
        // it, would hang below DISPLAY1's foot onto DISPLAY2.
        let l = Layout::from([(MAIN, Rect::new(2200, 1000, 550, 232))]);
        let g = WindowGraph::new();
        let shaded = BTreeSet::from([MAIN]);
        let sc = launch_scales(&l, &g, &desk(), 1.0, 2.0, &BTreeMap::new(), &shaded);
        assert_eq!(sc[&MAIN], (Some(1.0), 1.0));
        assert!(heal_saved_sizes(&l, &g, &desk(), 1.0, 2.0, &BTreeMap::new(), &shaded).is_none());
        // Judged expanded, it was 150 %.
        let sc = launch_scales(
            &l,
            &g,
            &desk(),
            1.0,
            2.0,
            &BTreeMap::new(),
            &BTreeSet::new(),
        );
        assert_eq!(sc[&MAIN], (Some(1.0), 1.5));
    }

    #[test]
    fn a_window_going_to_another_scale_is_first_put_at_the_size_it_has() {
        let r = Rect::new(2290, 68, 825, 348);
        assert_eq!(sized_for(r, 1.5, 1.0), Rect::new(2290, 68, 550, 232));
        assert_eq!(sized_for(r, 1.5, 1.5), r);
    }

    #[test]
    fn a_stack_dragged_onto_150_percent_takes_its_size_there() {
        // At 1x the owner saw 275 x 116 on DISPLAY2 where 413 x 174 belongs.
        let (origin, now, g) = stack_moved(1.0, 0, 2800, 200);
        let out = fit(&origin, &now, &g, 1.0).expect("it crossed");
        for id in CLASSIC {
            assert_eq!((out[&id].w, out[&id].h), (413, 174), "{id:?}");
        }
        assert_eq!((out[&MAIN].x, out[&MAIN].y), (now[&MAIN].x, now[&MAIN].y));
        assert!(bond::violations(&g, &out).is_empty());
    }

    #[test]
    fn at_2x_the_playlist_keeps_its_steps_on_the_new_display() {
        // The owner's 2x: 551 x 233 where 825 x 348 belongs.
        let (origin, now, g) = stack_moved(2.0, 3, 2800, 200);
        let out = fit(&origin, &now, &g, 2.0).unwrap();
        assert_eq!((out[&MAIN].w, out[&MAIN].h), (825, 348));
        assert_eq!(
            out[&PLAYLIST].h,
            bond::d40::stepped(CHROME_H * 2.0, PLAYLIST_STEP_H * 2.0, 3, 1.5)
        );
        assert!(bond::violations(&g, &out).is_empty());
    }

    #[test]
    fn coming_back_and_staying_put_change_nothing() {
        let (origin, now, g) = stack_moved(1.0, 0, 300, 50);
        assert_eq!(fit(&origin, &now, &g, 1.0), None, "never left DISPLAY1");
        let (origin, _, g) = stack_moved(1.0, 0, 0, 0);
        assert_eq!(fit(&origin, &origin, &g, 1.0), None);
    }

    #[test]
    fn a_window_takes_the_scale_of_the_display_holding_most_of_it() {
        let ms = desk();
        let mostly_left = Rect::new(2560 - 200, 100, 275, 116);
        let mostly_right = Rect::new(2560 - 70, 100, 275, 116);
        assert_eq!(dpi_monitor(&ms, mostly_left).unwrap().scale, 1.0);
        assert_eq!(dpi_monitor(&ms, mostly_right).unwrap().scale, 1.5);
        assert_eq!(
            dpi_monitor(&ms, Rect::new(-900, 100, 275, 116))
                .unwrap()
                .scale,
            1.0
        );
    }

    #[test]
    fn a_layout_saved_at_the_wrong_size_on_150_percent_heals_at_launch() {
        // What every build before D187 saved after a drag onto DISPLAY2.
        let (origin, now, g) = stack_moved(1.0, 3, 2800, 200);
        let healed = heal_saved_sizes(
            &now,
            &g,
            &desk(),
            1.0,
            1.0,
            &BTreeMap::new(),
            &BTreeSet::new(),
        )
        .expect("it was wrong");
        let fitted = fit(&origin, &now, &g, 1.0).unwrap();
        assert_eq!(healed.0, fitted, "the same sizes a drag there gives now");
        assert_eq!(
            (healed.0[&MAIN].x, healed.0[&MAIN].y),
            (now[&MAIN].x, now[&MAIN].y)
        );
        // And a layout that already fits its displays is left alone.
        assert!(heal_saved_sizes(
            &healed.0,
            &healed.1,
            &desk(),
            1.0,
            1.0,
            &BTreeMap::new(),
            &BTreeSet::new()
        )
        .is_none());
        assert!(heal_saved_sizes(
            &origin,
            &g,
            &desk(),
            1.0,
            1.0,
            &BTreeMap::new(),
            &BTreeSet::new()
        )
        .is_none());
    }

    #[test]
    fn a_shaded_main_crosses_as_a_strip_and_keeps_its_height_for_later() {
        let (origin, mut now, g) = stack_moved(1.0, 0, 2800, 200);
        // Main shaded to its 14 px strip, the rest closed up under it.
        let shaded = BTreeSet::from([MAIN]);
        let unshaded = BTreeMap::from([(MAIN, 116)]);
        apply_shade(&mut now, &g, MAIN, 14);
        let mut start = origin.clone();
        apply_shade(&mut start, &g, MAIN, 14);
        let (out, heights) = fit_to_displays(
            &now,
            &CLASSIC,
            &g,
            &shaded,
            &unshaded,
            &by_area(&start, &now),
            1.0,
        )
        .unwrap();
        assert_eq!(out[&MAIN].h, bond::d40::physical(SHADE_H, 1.5));
        assert_eq!(
            heights[&MAIN], 174,
            "unshading on DISPLAY2 opens to its size there"
        );
        assert_eq!(
            out[&EQ].y,
            out[&MAIN].bottom(),
            "the EQ sits right under the strip"
        );
        assert!(bond::violations(&g, &out).is_empty());
    }

    // ---- one scale for the desktop (D182) -----------------------------------

    #[test]
    fn one_display_to_200_percent_resizes_in_place_and_keeps_its_seams() {
        // X's screen stays the panel's 1920 x 1080 at 200 %, so nothing moves.
        let (mut l, g) = initial_layout(1.0, 1.0);
        // A playlist two steps taller, which must stay two steps taller.
        let pl = l[&PLAYLIST];
        l.insert(PLAYLIST, Rect::new(pl.x, pl.y, pl.w, pl.h + 2 * 29));
        let at1 = [mon(0, 0, 1920, 1080, 1.0)];
        let at2 = [mon(0, 0, 1920, 1080, 2.0)];
        let (out, g2) = rescale_layout(&l, &g, &at1, &at2, (1.0, 2.0), 1.0);
        assert_eq!(out[&MAIN], Rect::new(120, 120, 550, 232));
        assert_eq!(out[&EQ], Rect::new(120, 352, 550, 232));
        assert_eq!(out[&PLAYLIST], Rect::new(120, 584, 550, 232 + 2 * 58));
        assert!(bond::violations(&g2, &out).is_empty());
        let (back, _) = rescale_layout(&out, &g2, &at2, &at1, (2.0, 1.0), 1.0);
        assert_eq!(back, l);
    }

    #[test]
    fn a_display_that_doubles_xs_screen_moves_the_group_with_it() {
        // Stage 6: a 150 % display joined the 100 % panel, X went to scale 2,
        // and the panel became 3840 x 2160 in X's pixels.
        let (l, g) = initial_layout(1.0, 1.0);
        let before = [mon(0, 0, 1920, 1080, 1.0)];
        let after = [mon(0, 0, 3840, 2160, 2.0), mon(3840, 0, 5120, 2880, 2.0)];
        let (out, g2) = rescale_layout(&l, &g, &before, &after, (1.0, 2.0), 1.0);
        assert_eq!(out[&MAIN], Rect::new(240, 240, 550, 232));
        assert!(bond::violations(&g2, &out).is_empty());
        // And the Dell goes again: a window that was on it keeps its place for
        // the rescue, sized for the scale left.
        let mut on_dell = out.clone();
        bond::translate_group(&mut on_dell, &CLASSIC, 4000, 0);
        let (gone, _) = rescale_layout(&on_dell, &g2, &after, &before, (2.0, 1.0), 1.0);
        assert_eq!(gone[&MAIN], Rect::new(4240, 240, 275, 116));
    }

    #[test]
    fn a_fractional_scale_change_keeps_seams_flush() {
        let (l, g) = initial_layout(1.0, 2.0);
        let (out, g2) = rescale_layout(&l, &g, &[], &[], (1.0, 1.25), 2.0);
        assert_eq!(out[&MAIN].w, bond::d40::physical(CHROME_W * 2.0, 1.25));
        assert_eq!((out[&MAIN].x, out[&MAIN].y), (l[&MAIN].x, l[&MAIN].y));
        assert!(bond::violations(&g2, &out).is_empty());
    }

    #[test]
    fn mains_width_says_the_scale_a_layout_was_saved_at() {
        let (l, _) = initial_layout(1.0, 1.0);
        assert_eq!(saved_scale(&l, 1.0), Some(1.0));
        let (l, _) = initial_layout(2.0, 1.0);
        assert_eq!(saved_scale(&l, 1.0), Some(2.0));
        let (l, _) = initial_layout(1.0, 2.0);
        assert_eq!(saved_scale(&l, 2.0), Some(1.0));
        let (l, _) = initial_layout(1.25, 1.0);
        assert!((saved_scale(&l, 1.0).unwrap() - 1.25).abs() < 0.01);
    }

    // ---- reach (#101, D88) --------------------------------------------------

    #[test]
    fn a_title_bar_above_the_work_area_is_pulled_down() {
        let s = stacked();
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        let mut l = s.layout.clone();
        bond::translate_group(&mut l, &CLASSIC, 0, -150);
        // Main's title bar is at y = -30 while the EQ and playlist are still
        // on screen, so the display rescue leaves this alone. The reach rule
        // does not.
        assert_eq!(rescue_layout(&l, &s.graph, &ms), l);
        assert_eq!(reach_clamp(&l, &CLASSIC, &ms, 1.0), (0, -l[&MAIN].y));
        let out = reach_layout(&l, &s.graph, &ms, 1.0);
        assert_eq!(out[&MAIN].y, 0);
        assert!(bond::violations(&s.graph, &out).is_empty());
    }

    #[test]
    fn a_reachable_group_is_left_alone() {
        let s = stacked();
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        assert_eq!(reach_clamp(&s.layout, &CLASSIC, &ms, 1.0), (0, 0));
        assert_eq!(reach_layout(&s.layout, &s.graph, &ms, 1.0), s.layout);
    }

    #[test]
    fn a_group_off_a_side_keeps_a_grab_of_every_title_bar() {
        let s = stacked();
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        let mut l = s.layout.clone();
        bond::translate_group(&mut l, &CLASSIC, 1900, 0);
        let x = l[&MAIN].x;
        assert_eq!(reach_clamp(&l, &CLASSIC, &ms, 1.0), (1920 - 40 - x, 0));
        let mut l = s.layout.clone();
        bond::translate_group(&mut l, &CLASSIC, -400, 0);
        let right = l[&MAIN].right();
        assert_eq!(reach_clamp(&l, &CLASSIC, &ms, 1.0), (40 - right, 0));
    }

    #[test]
    fn a_title_bar_under_the_taskbar_is_pulled_up() {
        let s = stacked();
        let ms = vec![mon_tb(0, 0, 1920, 1080, 1.0, 48)];
        let mut l = s.layout.clone();
        // The playlist's bar lands at y = 1050, under a 48 px taskbar.
        let dy = 1050 - l[&PLAYLIST].y;
        bond::translate_group(&mut l, &CLASSIC, 0, dy);
        assert_eq!(reach_clamp(&l, &CLASSIC, &ms, 1.0), (0, 1032 - 14 - 1050));
    }

    #[test]
    fn a_member_on_each_display_is_reachable_where_it_is() {
        let mut s = stacked();
        s.graph.break_bond(EQ, PLAYLIST);
        let ms = two_monitors();
        let mut l = s.layout.clone();
        bond::translate_group(&mut l, &[PLAYLIST], 3000, 0);
        assert_eq!(reach_layout(&l, &s.graph, &ms, 1.0), l);
    }

    #[test]
    fn the_clamp_moves_only_the_component_that_is_out_of_reach() {
        let mut s = stacked();
        s.graph.break_bond(EQ, PLAYLIST);
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        let mut l = s.layout.clone();
        bond::translate_group(&mut l, &[PLAYLIST], 0, -600);
        let out = reach_layout(&l, &s.graph, &ms, 1.0);
        assert_eq!(out[&MAIN], l[&MAIN]);
        assert_eq!(out[&EQ], l[&EQ]);
        assert_eq!(out[&PLAYLIST].y, 0);
    }

    #[test]
    fn a_group_taller_than_the_display_keeps_its_top_edge() {
        // Clamped rather than centred: centring a 348px stack on a 200px-tall
        // display would push the title bars off the top, where nothing can grab
        // them.
        let short = vec![mon(0, 0, 1920, 200, 1.0)];
        let s = stacked();
        let mut stranded = s.layout.clone();
        bond::translate_group(&mut stranded, &CLASSIC, -32000, -32000);
        let out = rescue_layout(&stranded, &s.graph, &short);
        assert_eq!(out[&MAIN].y, 0, "the top of the group went off screen");
    }

    // ---- persistence (D33) --------------------------------------------------

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        conn
    }

    #[test]
    fn geometry_and_the_bond_graph_both_survive_a_restart() {
        // D33 is explicit that the *graph* persists too, not just the rects.
        // Restoring three windows in the right places with no bonds between
        // them would look identical on the first frame and wrong on the first
        // drag.
        let conn = memory_db();
        let s = stacked();
        save(
            &conn,
            &s.layout,
            &s.graph,
            &s.shaded,
            &s.unshaded_h,
            &s.monitors,
            &s.drawn_at,
        )
        .unwrap();

        let r = load(&conn).expect("nothing came back");
        assert_eq!(r.layout, s.layout);
        assert_eq!(r.graph.components(&CLASSIC).len(), 1);
        assert!(bond::violations(&r.graph, &r.layout).is_empty());
    }

    #[test]
    fn a_broken_bond_stays_broken_across_a_restart() {
        let conn = memory_db();
        let mut s = stacked();
        s.graph.break_bond(EQ, PLAYLIST);
        save(
            &conn,
            &s.layout,
            &s.graph,
            &s.shaded,
            &s.unshaded_h,
            &s.monitors,
            &s.drawn_at,
        )
        .unwrap();

        let r = load(&conn).expect("nothing came back");
        assert_eq!(r.graph.components(&CLASSIC).len(), 2);
        assert!(r.graph.bond_between(EQ, PLAYLIST).is_none());
        assert!(r.graph.bond_between(MAIN, EQ).is_some());
    }

    #[test]
    fn a_shaded_stack_round_trips_through_the_database_exactly() {
        // The bug this pins down: a shade moves its neighbours as well as
        // changing one height, so storing the collapsed positions next to the
        // expanded heights saves a world that never existed. On the next launch
        // eq's bottom edge and the playlist's top edge do not meet, register()
        // correctly drops the bond between them, and the group silently comes
        // back in two pieces.
        let conn = memory_db();
        let mut s = stacked();
        let before = s.layout.clone();

        // Collapse eq the way toggle_shade would.
        s.unshaded_h.insert(EQ, s.layout[&EQ].h);
        s.shaded.insert(EQ);
        let graph = s.graph.clone();
        apply_shade(&mut s.layout, &graph, EQ, 14);
        assert_eq!(s.layout[&PLAYLIST].y, before[&PLAYLIST].y - 102);

        save(
            &conn,
            &s.layout,
            &s.graph,
            &s.shaded,
            &s.unshaded_h,
            &s.monitors,
            &s.drawn_at,
        )
        .unwrap();
        let r = load(&conn).expect("nothing came back");

        // What is stored is the expanded world, and it is flush.
        assert_eq!(r.layout, before);
        assert!(bond::violations(&r.graph, &r.layout).is_empty());

        // Re-collapsing on load reproduces exactly what was on screen.
        let mut restored = r.layout.clone();
        apply_shade(&mut restored, &r.graph, EQ, 14);
        assert_eq!(restored, s.layout);
        assert!(bond::violations(&r.graph, &restored).is_empty());
    }

    #[test]
    fn a_bond_span_follows_the_windows_it_describes() {
        // Nothing reads the span yet, which is exactly why it has to be right:
        // it is persisted, so a stale one gets written to disk and read back as
        // though it meant something.
        let mut s = stacked();
        assert_eq!(s.graph.bonds[0].span, (0, 275));
        bond::translate_group(&mut s.layout, &CLASSIC, 400, 200);
        let layout = s.layout.clone();
        resync_spans(&mut s.graph, &layout);
        assert!(s.graph.bonds.iter().all(|b| b.span == (400, 675)));
    }

    #[test]
    fn a_shaded_window_comes_back_at_the_size_it_had_before_it_collapsed() {
        // The stored height is the *unshaded* one. Persisting 14px would mean a
        // playlist resized to 174 and then shaded is 116 forever after the next
        // restart -- a resize silently destroyed by a restart nobody connected
        // to it.
        let conn = memory_db();
        let mut s = stacked();
        s.unshaded_h.insert(PLAYLIST, 174);
        s.shaded.insert(PLAYLIST);
        s.layout.insert(PLAYLIST, Rect::new(0, 232, 275, 14));
        save(
            &conn,
            &s.layout,
            &s.graph,
            &s.shaded,
            &s.unshaded_h,
            &s.monitors,
            &s.drawn_at,
        )
        .unwrap();

        let r = load(&conn).expect("nothing came back");
        assert!(r.shaded.contains(&PLAYLIST));
        assert_eq!(r.unshaded_h[&PLAYLIST], 174);
        assert_eq!(r.layout[&PLAYLIST].h, 174, "came back collapsed forever");
    }

    #[test]
    fn the_scale_each_size_is_in_survives_a_restart() {
        // D191: and a row without one reads as unknown.
        let conn = memory_db();
        let mut s = stacked();
        s.drawn_at = BTreeMap::from([(MAIN, 1.0), (EQ, 1.0), (PLAYLIST, 1.5)]);
        save(
            &conn,
            &s.layout,
            &s.graph,
            &s.shaded,
            &s.unshaded_h,
            &s.monitors,
            &s.drawn_at,
        )
        .unwrap();
        assert_eq!(load(&conn).expect("nothing came back").drawn_at, s.drawn_at);
        s.drawn_at.remove(&EQ);
        save(
            &conn,
            &s.layout,
            &s.graph,
            &s.shaded,
            &s.unshaded_h,
            &s.monitors,
            &s.drawn_at,
        )
        .unwrap();
        assert_eq!(load(&conn).unwrap().drawn_at, s.drawn_at);
    }

    #[test]
    fn saving_twice_does_not_accumulate_rows() {
        let conn = memory_db();
        let s = stacked();
        for _ in 0..3 {
            save(
                &conn,
                &s.layout,
                &s.graph,
                &s.shaded,
                &s.unshaded_h,
                &s.monitors,
                &s.drawn_at,
            )
            .unwrap();
        }
        let layouts: i64 = conn
            .query_row("SELECT COUNT(*) FROM window_layout", [], |r| r.get(0))
            .unwrap();
        let bonds: i64 = conn
            .query_row("SELECT COUNT(*) FROM window_bonds", [], |r| r.get(0))
            .unwrap();
        assert_eq!(layouts, 3);
        assert_eq!(bonds, 2);
    }

    #[test]
    fn an_empty_database_restores_nothing_rather_than_half_a_layout() {
        assert!(load(&memory_db()).is_none());
    }

    #[test]
    fn a_partial_layout_is_refused() {
        // Two of three windows is not something to reconstruct around. The
        // default stack is a perfectly good answer and a guessed third window
        // is not.
        let conn = memory_db();
        let mut s = stacked();
        s.layout.remove(&PLAYLIST);
        save(
            &conn,
            &s.layout,
            &s.graph,
            &s.shaded,
            &s.unshaded_h,
            &s.monitors,
            &s.drawn_at,
        )
        .unwrap();
        assert!(load(&conn).is_none());
    }

    #[test]
    fn the_monitor_a_window_sat_on_is_recorded() {
        let conn = memory_db();
        let mut s = stacked();
        s.monitors = two_monitors();
        s.layout.insert(MAIN, Rect::new(3000, 100, 275, 116));
        save(
            &conn,
            &s.layout,
            &s.graph,
            &s.shaded,
            &s.unshaded_h,
            &s.monitors,
            &s.drawn_at,
        )
        .unwrap();
        let id: Option<String> = conn
            .query_row(
                "SELECT monitor_id FROM window_layout WHERE window_id = 'main'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(id.as_deref(), Some("2560,0,1920x1080"));
    }

    #[test]
    fn labels_and_entries_are_distinct() {
        let labels: Vec<&str> = CLASSIC.iter().map(|id| label_of(*id)).collect();
        let entries: Vec<&str> = CLASSIC.iter().map(|id| entry_of(*id)).collect();
        for set in [labels, entries] {
            let mut sorted = set.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), set.len());
        }
        // A root label colliding with a window label would make
        // get_webview_window hand back the wrong window and quietly build the
        // star topology D41 exists to avoid.
        for r in ROOT_LABELS {
            assert!(!CLASSIC.iter().any(|id| label_of(*id) == r));
        }
    }
}
