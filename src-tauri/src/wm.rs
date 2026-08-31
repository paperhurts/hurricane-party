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

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

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
        WebviewWindowBuilder::new(app, label, WebviewUrl::App("main.html".into()))
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
