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

use std::sync::Mutex;

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindowBuilder,
};

use crate::bond::{self, Bond, Edge, Layout, Px, Rect, WindowGraph, WindowId};
use crate::platform::{self, NativeWindow};

// ---- geometry ---------------------------------------------------------------

/// Logical chrome geometry, 1x. `windows.md`'s inventory. Double for 2x mode.
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
    monitor_at(monitors, x, y).map(|m| m.scale).unwrap_or(fallback)
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
        let Some(n) = monitors.iter().map(|m| m.rect).find(|n| {
            n.x == r.right() && n.y <= group.y && n.bottom() >= group.bottom()
        }) else {
            break;
        };
        r.w = n.right() - r.x;
    }
    for _ in 0..limit {
        let Some(n) = monitors.iter().map(|m| m.rect).find(|n| {
            n.right() == r.x && n.y <= group.y && n.bottom() >= group.bottom()
        }) else {
            break;
        };
        r.w = r.right() - n.x;
        r.x = n.x;
    }
    for _ in 0..limit {
        let Some(n) = monitors.iter().map(|m| m.rect).find(|n| {
            n.y == r.bottom() && n.x <= group.x && n.right() >= group.right()
        }) else {
            break;
        };
        r.h = n.bottom() - r.y;
    }
    for _ in 0..limit {
        let Some(n) = monitors.iter().map(|m| m.rect).find(|n| {
            n.bottom() == r.y && n.x <= group.x && n.right() >= group.right()
        }) else {
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
        self.handles.get(id.0 as usize).copied().unwrap_or(NativeWindow::NONE)
    }
}

/// Tauri-managed wrapper. Separate type so `WmState` itself stays plain data
/// that the tests can build without a running app.
#[derive(Default)]
pub struct Wm(pub Mutex<WmState>);

// ---- window creation --------------------------------------------------------

/// Build the three classic windows plus their hidden roots.
///
/// Sizes are declared in `PhysicalSize`, never the logical config keys (D38):
/// `275 x 1.5 = 412.5`, and a logically-sized window inherits a half pixel that
/// the toolkit resolves by its own rounding rule. For a bond model whose whole
/// premise is two windows sitting flush with a hairline seam, that would make
/// "flush" a property of tao's rounding mode.
pub fn build_classic_windows(app: &AppHandle) -> tauri::Result<()> {
    let scale = app.primary_monitor()?.map(|m| m.scale_factor()).unwrap_or(1.0);

    let w = bond::d40::physical(CHROME_W, scale);
    let h = bond::d40::physical(CHROME_H, scale);

    // Start where Winamp does: main on top, EQ under it, playlist under that,
    // all three flush. The bonds are inserted in register() so the group is real
    // from the first frame rather than something the user has to assemble.
    let x0 = bond::d40::physical(120.0, scale);
    let y0 = bond::d40::physical(120.0, scale);

    for (i, id) in CLASSIC.iter().enumerate() {
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
        win.set_position(PhysicalPosition::new(x0, y0 + h * i as Px))?;
        win.set_size(PhysicalSize::new(w as u32, h as u32))?;
        win.show()?;
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

/// Read the windows back out of the OS and seed the state from what is actually
/// on screen, rather than from what we asked for.
///
/// D58 is the reason this reads instead of assuming: the bond graph stays
/// perfectly self-consistent while describing a layout that exists nowhere, so
/// the OS is the authority and the model is the thing that gets corrected.
pub fn register(app: &AppHandle) -> tauri::Result<()> {
    let scale = app.primary_monitor()?.map(|m| m.scale_factor()).unwrap_or(1.0);

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

    let mut graph = WindowGraph::new();
    for (a, b) in [(MAIN, EQ), (EQ, PLAYLIST)] {
        let (Some(ra), Some(rb)) = (layout.get(&a), layout.get(&b)) else {
            continue;
        };
        // Only bond what is genuinely flush. If the OS put a window somewhere
        // other than where we asked, the honest answer is no bond — asserting a
        // bond that is not geometrically true is exactly the D58 failure.
        if ra.bottom() == rb.y {
            let span = (ra.x.max(rb.x), ra.right().min(rb.right()));
            graph.insert(Bond::new(a, b, Edge::Bottom, span));
        }
    }

    let plan = {
        let state = app.state::<Wm>();
        let mut s = state.0.lock().unwrap();
        s.scale = scale;
        s.monitors = monitors;
        s.handles = handles;
        s.roots = roots;
        s.layout = layout;
        s.graph = graph;
        plan_ownership(&s, Some(MAIN))
    }; // D54: lock dropped here, before a single OS call.

    apply_ownership(&plan);
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
                if best.map_or(true, |(c, _, _)| cost < c) {
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
        let active = drag.moving.first().copied();
        plan_ownership(&s, active)
    };
    apply_ownership(&plan);
}

// ---- focus ------------------------------------------------------------------

/// Focus is a group property: when any bonded window has focus, all of them
/// render active. Getting this wrong looks broken immediately.
///
/// Returns the per-window flags so the caller can emit them once the lock is
/// gone.
pub fn focus_plan(state: &WmState, focused: Option<WindowId>) -> Vec<(WindowId, bool)> {
    let group = focused.map(|f| state.graph.component(f)).unwrap_or_default();
    CLASSIC.iter().map(|id| (*id, group.contains(id))).collect()
}

/// A window was clicked or focused: raise its whole group (D42) and tell every
/// classic window whether it should render active.
pub fn focus_group(app: &AppHandle, focused: Option<WindowId>) {
    let (plan, flags) = {
        let state = app.state::<Wm>();
        let s = state.0.lock().unwrap();
        (plan_ownership(&s, focused), focus_plan(&s, focused))
    };
    // Only a genuine focus gain reorders anything. On focus *loss* the app is
    // no longer foreground, so raising would be shoving our windows up through
    // somebody else's stack for no reason — and D56 says it would not reach the
    // top anyway.
    if focused.is_some() {
        apply_ownership(&plan);
    }
    for (id, active) in flags {
        let _ = app.emit_to(label_of(id), "wm:active", active);
    }
}

// ---- tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!bond::splitter_is_live(is_resizable(MAIN), is_resizable(EQ)));
        assert!(bond::splitter_is_live(is_resizable(EQ), is_resizable(PLAYLIST)));
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
        MonitorInfo { rect: Rect::new(x, y, w, h), scale }
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
        assert_eq!(bond::d40::threshold(SNAP_THRESHOLD, scale_at(&ms, 100, 100, 1.0)), 15);
        assert_eq!(bond::d40::threshold(SNAP_THRESHOLD, scale_at(&ms, 3000, 100, 1.0)), 10);
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
        assert_eq!(screen_rect_for(&[], Rect::new(0, 0, 800, 600), g), Rect::new(0, 0, 800, 600));
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
        assert!(bond::violations(&{
            let mut g = WindowGraph::new();
            g.insert(Bond::new(MAIN, EQ, Edge::Bottom, (0, 275)));
            g
        }, &last).is_empty());
    }

    #[test]
    fn the_magnet_pulls_the_whole_group_flush() {
        // Drag main+eq so main lands 7 px short of the playlist's left edge.
        // Within the 10 px threshold, so it snaps -- and eq comes with it.
        let origin = drag_layout();
        let out = drag_frame(&origin, &[MAIN, EQ], (318, 0), &[PLAYLIST], 10, None);
        assert_eq!(out[&MAIN].right(), out[&PLAYLIST].x, "did not snap flush");
        assert_eq!(out[&EQ].x, out[&MAIN].x, "eq did not come along");
        assert_eq!(out[&MAIN].bottom(), out[&EQ].y, "the group's own bond opened");
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
        assert_eq!(out[&MAIN].x, origin[&PLAYLIST].right(), "snapped to the far one");
    }

    #[test]
    fn a_screen_edge_is_a_movement_constraint_only() {
        let ms = vec![mon(0, 0, 1920, 1080, 1.0)];
        let screen = screen_rect_for(&ms, ms[0].rect, Rect::new(0, 0, 275, 232));
        let origin = drag_layout();
        let out = drag_frame(&origin, &[MAIN, EQ], (-6, -4), &[PLAYLIST], 10, Some(screen));
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
        assert_eq!(out[&MAIN].right(), origin[&EQ].x, "took the screen edge instead");
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
