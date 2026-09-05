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
    let mut r = start;
    // Each side walks independently. The guard bounds a loop that already
    // cannot cycle, since every step strictly grows `r` in one direction.
    let limit = monitors.len() + 1;

    for _ in 0..limit {
        let Some(n) = monitors
            .iter()
            .map(|m| m.rect)
            .find(|n| n.x == r.right() && n.y <= group.y && n.bottom() >= group.bottom())
        else {
            break;
        };
        r.w = n.right() - r.x;
    }
    for _ in 0..limit {
        let Some(n) = monitors
            .iter()
            .map(|m| m.rect)
            .find(|n| n.right() == r.x && n.y <= group.y && n.bottom() >= group.bottom())
        else {
            break;
        };
        r.w = r.right() - n.x;
        r.x = n.x;
    }
    for _ in 0..limit {
        let Some(n) = monitors
            .iter()
            .map(|m| m.rect)
            .find(|n| n.y == r.bottom() && n.x <= group.x && n.right() >= group.right())
        else {
            break;
        };
        r.h = n.bottom() - r.y;
    }
    for _ in 0..limit {
        let Some(n) = monitors
            .iter()
            .map(|m| m.rect)
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
    /// Set for the duration of a corner-grip resize of the playlist.
    pub resize: Option<ResizeState>,
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

    let (mut layout, graph, shaded, unshaded_h) = match restored {
        Some(r) => (r.layout, r.graph, r.shaded, r.unshaded_h),
        None => {
            let (l, g) = initial_layout(scale, zoom);
            (l, g, BTreeSet::new(), BTreeMap::new())
        }
    };

    // D33 again, and the reason the column records a monitor at all: a layout
    // saved on a display that is no longer attached must not restore into empty
    // space. Same rigid-translation rescue the display watchdog uses, so a
    // group that comes back does so with its bonds intact.
    layout = rescue_layout(&layout, &graph, &monitors);

    // Re-collapse whatever was left shaded. The stored height is the *unshaded*
    // one, so this is a fresh collapse from a known-good size rather than a
    // 14 px rect remembered from last time — which is what keeps a resized
    // playlist from being lost across a restart.
    for id in CLASSIC {
        if !shaded.contains(&id) {
            continue;
        }
        let at = layout
            .get(&id)
            .and_then(|r| monitor_at(&monitors, r.x, r.y))
            .map(|m| m.scale)
            .unwrap_or(scale);
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
    s.monitors = monitors;
    s.layout = layout;
    s.graph = graph;
    s.shaded = shaded;
    s.unshaded_h = unshaded_h;
    Ok(())
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
    let (seeded, zoom) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        (s.layout.clone(), s.zoom())
    };

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

        // D52: position, THEN size. Crossing a DPI boundary is not a pure move
        // — Windows sends WM_DPICHANGED and tao rescales to preserve logical
        // size, so size-then-position leaves the window 1.5x too big.
        let r = seeded[id];
        win.set_position(PhysicalPosition::new(r.x, r.y))?;
        win.set_size(PhysicalSize::new(r.w as u32, r.h as u32))?;
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

    Ok(())
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
    Ok(())
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
        })
        .collect();

    let mut handles = Vec::with_capacity(CLASSIC.len());
    let mut layout = Layout::new();
    for id in CLASSIC {
        let Some(win) = app.get_webview_window(label_of(id)) else {
            continue;
        };
        handles.push(platform::handle_of(&win));
        let p = win.outer_position()?;
        let s = win.outer_size()?;
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
        s.layout = layout;
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
    }; // D54: lock dropped here, before a single OS call.

    apply_ownership(&plan);
    emit_state(app);
    Ok(())
}

// ---- z-order ----------------------------------------------------------------

/// What to do about ownership, computed under the lock. Performing it is a
/// separate step that must run with the lock released (D54).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OwnPlan {
    /// `(window, owner)` pairs.
    pub owners: Vec<(NativeWindow, NativeWindow)>,
    /// Windows to force to the top, in order, bottom group first.
    pub raise: Vec<NativeWindow>,
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
    let raise = order
        .iter()
        .flat_map(|c| c.iter().map(|id| state.handle(*id)))
        .collect();

    OwnPlan { owners, raise }
}

/// Perform an [`OwnPlan`]. **Must be called with no lock held** (D54).
pub fn apply_ownership(plan: &OwnPlan) {
    let p = platform::platform();
    for (w, owner) in &plan.owners {
        p.set_owner(*w, *owner);
    }
    for w in &plan.raise {
        p.raise_no_activate(*w);
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
    let state = app.state::<Wm>();
    let mut s = state.0.lock().unwrap();
    let moving = s.graph.component(id);
    let origin_layout = s.layout.clone();
    s.drag = Some(DragState {
        moving,
        origin_layout,
        origin_cursor: cursor,
    });
}

/// One drag frame, driven by the webview pointermove that the compositor has
/// already coalesced to one per display frame (O15).
pub fn drag_move(app: &AppHandle) {
    let cursor = platform::platform().cursor_pos();

    let (layout, moving) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        let Some(drag) = s.drag.clone() else {
            return;
        };

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

        let layout = drag_frame(
            &drag.origin_layout,
            &drag.moving,
            total,
            &others,
            threshold,
            screen,
        );
        s.layout = layout.clone();
        (layout, drag.moving)
    }; // D54: lock released before the OS is touched.

    push_to_os(app, &layout, &moving);
}

/// End a drag: form whatever bonds the final position earned, then re-apply the
/// ownership topology so the new group shape is real in the z-order too.
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

pub fn drag_end(app: &AppHandle) {
    let plan = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        let Some(drag) = s.drag.take() else {
            return;
        };

        let others: Vec<WindowId> = CLASSIC
            .iter()
            .copied()
            .filter(|c| !drag.moving.contains(c))
            .collect();
        for b in bonds_after_drag(&s.layout, &drag.moving, &others) {
            s.graph.insert(b);
        }
        let layout = s.layout.clone();
        resync_spans(&mut s.graph, &layout);
        let active = drag.moving.first().copied();
        plan_ownership(&s, active)
    };
    apply_ownership(&plan);
    emit_state(app);
    save_now(app);
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

        let scale = scale_at(&s.monitors, cursor.0, cursor.1, s.scale);
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
        s.layout = layout.clone();
        (layout, s.graph.component(sp.bond.a))
    };

    push_to_os(app, &layout, &touched);
}

pub fn splitter_end(app: &AppHandle) {
    {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        s.splitter = None;
    }
    save_now(app);
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
        // D51: rendered geometry resolves from the window's own monitor.
        let scale = scale_at(&s.monitors, r.origin.x, r.origin.y, s.scale);
        let delta = (cursor.0 - r.origin_cursor.0, cursor.1 - r.origin_cursor.1);
        let rect = resize_frame(r.origin, delta, r.w_free, r.h_free, scale, s.zoom());
        s.layout.insert(r.id, rect);
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
        if s.resize.take().is_none() {
            return;
        }
        let layout = s.layout.clone();
        resync_spans(&mut s.graph, &layout);
    }
    emit_state(app);
    save_now(app);
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
    let scale = state
        .layout
        .get(&id)
        .and_then(|r| monitor_at(&state.monitors, r.x, r.y))
        .map(|m| m.scale)
        .unwrap_or(state.scale);
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
        s.layout = layout.clone();

        // Everything in the component may have moved, plus the window itself.
        let moved = s.graph.component(id);
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
/// would be rude. Restores a minimized member first, because `set_focus`
/// never does (#39, D59). Leaves `focused` alone: the chrome should render
/// active only where the OS focus actually is.
pub fn raise_group(app: &AppHandle, id: WindowId) {
    let (plan, handles) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        let comp = s.graph.component(id);
        (
            plan_ownership(&s, Some(id)),
            comp.iter().map(|w| s.handle(*w)).collect::<Vec<_>>(),
        )
    }; // D54: the lock is gone before any Win32 call.
    let p = platform::platform();
    for w in handles
        .iter()
        .filter(|w| !w.is_none() && p.is_minimized(**w))
    {
        p.restore_no_activate(*w);
    }
    apply_ownership(&plan);
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
    let scale_of = |r: &Rect| {
        monitor_at(monitors, r.x, r.y)
            .map(|m| m.scale)
            .unwrap_or(fallback_scale)
    };

    let mut sizes: BTreeMap<WindowId, (Px, Px)> = BTreeMap::new();
    for (id, r) in layout {
        let scale = scale_of(r);
        let size = if is_resizable(*id) {
            let steps = |px: Px, base: f64, step: f64| {
                (((px as f64 / scale) - base * old_zoom) / (step * old_zoom))
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
        let ar = layout[&anchor];
        let (aw, ah) = sizes[&anchor];
        out.insert(anchor, Rect::new(ar.x, ar.y, aw, ah));

        let mut queue = VecDeque::from([anchor]);
        while let Some(p) = queue.pop_front() {
            let p_old = layout[&p];
            let p_new = out[&p];
            let scale = scale_of(&p_old);
            // An offset along the seam, re-derived from the logical base.
            let along =
                |old: Px| bond::d40::physical((old as f64 / scale / old_zoom) * new_zoom, scale);
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
    let (layout, zoom) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        if s.double == on {
            return;
        }
        let old_zoom = s.zoom();
        let new_zoom = if on { 2.0 } else { 1.0 };

        // Expand, re-derive, re-collapse: the same dance `save` does, because
        // a shaded height is a state to reapply, not a size to scale.
        let mut expanded = s.layout.clone();
        for id in &s.shaded {
            if let Some(h) = s.unshaded_h.get(id).copied() {
                apply_shade(&mut expanded, &s.graph, *id, h);
            }
        }
        let (rezoomed, graph) = rezoom_layout(
            &expanded,
            &s.graph,
            &s.monitors,
            s.scale,
            old_zoom,
            new_zoom,
        );
        // A group that doubled may now hang off the display; same rescue as a
        // topology change, so it comes back rigidly with its bonds intact.
        let mut layout = rescue_layout(&rezoomed, &graph, &s.monitors);

        s.double = on;
        let shaded: Vec<WindowId> = s.shaded.iter().copied().collect();
        for id in shaded {
            let Some(r) = layout.get(&id).copied() else {
                continue;
            };
            s.unshaded_h.insert(id, r.h);
            let at = monitor_at(&s.monitors, r.x, r.y)
                .map(|m| m.scale)
                .unwrap_or(s.scale);
            apply_shade(
                &mut layout,
                &graph,
                id,
                bond::d40::physical(SHADE_H * new_zoom, at),
            );
        }
        s.graph = graph;
        s.layout = layout.clone();
        (layout, new_zoom)
    }; // D54: the lock is gone before any window call.

    for id in CLASSIC {
        if let Some(win) = app.get_webview_window(label_of(id)) {
            let _ = win.set_zoom(zoom);
        }
    }
    push_to_os(app, &layout, &CLASSIC);
    emit_state(app);
    save_now(app);
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
    let (handles, topology_changed) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        (
            CLASSIC.iter().map(|id| s.handle(*id)).collect::<Vec<_>>(),
            monitors != s.monitors,
        )
    }; // D54: the lock is gone before is_minimized touches a window.
    let minimized: Vec<NativeWindow> = handles
        .into_iter()
        .filter(|w| !w.is_none() && p.is_minimized(*w))
        .collect();
    if !topology_changed && minimized.is_empty() {
        return;
    }

    // D59 is why this cannot be left to the user: undecorated windows with no
    // taskbar button have no restore affordance at all. Main keeps a taskbar
    // button as a second way back, but the rescue is the first.
    for w in &minimized {
        p.restore_no_activate(*w);
    }

    let (layout, moved) = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        s.monitors = monitors.clone();
        s.scale = monitors.first().map(|m| m.scale).unwrap_or(s.scale);
        let rescued = rescue_layout(&s.layout, &s.graph, &s.monitors);
        let moved: Vec<WindowId> = CLASSIC
            .iter()
            .copied()
            .filter(|id| s.layout.get(id) != rescued.get(id))
            .collect();
        s.layout = rescued.clone();
        (rescued, moved)
    }; // D54: lock dropped before the OS is touched.

    if !moved.is_empty() {
        eprintln!(
            "wm: display topology changed, rescued {} window(s) onto a surviving display",
            moved.len()
        );
        push_to_os(app, &layout, &moved);
    } else if !minimized.is_empty() {
        // Un-minimizing alone can leave the OS geometry behind the model, so
        // re-assert it (D58: the model and the OS have to be made to agree,
        // and the graph agreeing with itself proves nothing).
        push_to_os(app, &layout, &CLASSIC);
    }
    save_now(app);
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

fn edge_name(e: Edge) -> &'static str {
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
            "INSERT INTO window_layout (window_id, x, y, w, h, shaded, visible, monitor_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            rusqlite::params![
                label_of(id),
                r.x,
                r.y,
                r.w,
                h,
                shaded.contains(&id) as i32,
                monitor_id_of(*r, monitors),
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
    let (layout, graph, shaded, unshaded_h, monitors, double) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        (
            s.layout.clone(),
            s.graph.clone(),
            s.shaded.clone(),
            s.unshaded_h.clone(),
            s.monitors.clone(),
            s.double,
        )
    }; // The wm lock is released before the db lock is taken -- always in that
       // order, so the two can never be acquired against each other.
    let conn = db.0.lock().unwrap();
    if let Err(e) = save(&conn, &layout, &graph, &shaded, &unshaded_h, &monitors) {
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

    let mut stmt = conn
        .prepare("SELECT window_id, x, y, w, h, shaded FROM window_layout")
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
            ))
        })
        .ok()?;

    for row in rows.flatten() {
        let (label, x, y, w, h, is_shaded) = row;
        let Some(id) = id_of(&label) else { continue };
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

        let plan = plan_ownership(&s, Some(MAIN));
        assert_eq!(*plan.raise.last().unwrap(), nw(11));
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
        )
        .unwrap();

        let r = load(&conn).expect("nothing came back");
        assert!(r.shaded.contains(&PLAYLIST));
        assert_eq!(r.unshaded_h[&PLAYLIST], 174);
        assert_eq!(r.layout[&PLAYLIST].h, 174, "came back collapsed forever");
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
