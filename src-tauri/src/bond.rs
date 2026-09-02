//! The bond graph.
//!
//! This is the one module in the spike that is NOT disposable — it gets ported
//! back. So it is clean, it is tested, and it has no dependencies: no Tauri, no
//! Win32, no I/O, no globals. Everything here is pure geometry over fake rects,
//! which is why the whole of stage 5 can be tested without opening a window.
//!
//! **Every coordinate in this module is a physical pixel.** Nothing logical is
//! allowed past the boundary (D40). The only functions that know about logical
//! units at all are the `d40` helpers at the bottom, whose entire job is to
//! convert once, correctly.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub type Px = i32;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

/// A window rectangle in physical pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub x: Px,
    pub y: Px,
    pub w: Px,
    pub h: Px,
}

impl Rect {
    pub fn new(x: Px, y: Px, w: Px, h: Px) -> Self {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> Px {
        self.x + self.w
    }
    pub fn bottom(&self) -> Px {
        self.y + self.h
    }
    pub fn translated(&self, dx: Px, dy: Px) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }
}

/// Which side of a window another window sits against.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Edge {
    Right,
    Bottom,
    Left,
    Top,
}

impl Edge {
    /// A seam between left/right neighbours is a vertical line.
    pub fn is_vertical_seam(self) -> bool {
        matches!(self, Edge::Right | Edge::Left)
    }
}

/// A bond between two windows.
///
/// Stored in **canonical form**: `edge` is always `Right` or `Bottom`, so `a` is
/// always the left/top window and `b` the right/bottom one. That normalisation
/// is what keeps the splitter maths from needing four symmetric cases, and it
/// makes an unordered pair unique so a re-bond cannot silently duplicate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bond {
    pub a: WindowId,
    pub b: WindowId,
    pub edge: Edge,
    /// Overlap interval along the seam (y range for a vertical seam, x range for
    /// a horizontal one), physical px.
    pub span: (Px, Px),
}

impl Bond {
    /// `edge` is the side of `a` that `b` sits against; the result is canonical.
    pub fn new(a: WindowId, b: WindowId, edge: Edge, span: (Px, Px)) -> Bond {
        match edge {
            Edge::Right | Edge::Bottom => Bond { a, b, edge, span },
            Edge::Left => Bond {
                a: b,
                b: a,
                edge: Edge::Right,
                span,
            },
            Edge::Top => Bond {
                a: b,
                b: a,
                edge: Edge::Bottom,
                span,
            },
        }
    }

    pub fn pair(&self) -> (WindowId, WindowId) {
        if self.a <= self.b {
            (self.a, self.b)
        } else {
            (self.b, self.a)
        }
    }

    pub fn touches(&self, id: WindowId) -> bool {
        self.a == id || self.b == id
    }

    pub fn other(&self, id: WindowId) -> Option<WindowId> {
        if self.a == id {
            Some(self.b)
        } else if self.b == id {
            Some(self.a)
        } else {
            None
        }
    }
}

/// The graph. Connected components are groups.
///
/// Deliberately not a flat list of "groups" — stage 5 is where a flat list falls
/// over, because breaking one bond in the middle of a chain has to split a group
/// into two and a flat list has no way to know where the split is.
#[derive(Clone, Debug, Default)]
pub struct WindowGraph {
    pub bonds: Vec<Bond>,
}

impl WindowGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a bond, replacing any existing bond between the same pair.
    /// At most one bond per unordered pair — two windows meet on one seam.
    pub fn insert(&mut self, bond: Bond) {
        let p = bond.pair();
        self.bonds.retain(|x| x.pair() != p);
        self.bonds.push(bond);
    }

    /// Remove the bond between two windows. Returns true if one was there.
    pub fn break_bond(&mut self, a: WindowId, b: WindowId) -> bool {
        let p = if a <= b { (a, b) } else { (b, a) };
        let before = self.bonds.len();
        self.bonds.retain(|x| x.pair() != p);
        self.bonds.len() != before
    }

    /// Remove every bond touching a window. Used when a window leaves entirely.
    pub fn isolate(&mut self, id: WindowId) {
        self.bonds.retain(|x| !x.touches(id));
    }

    pub fn bond_between(&self, a: WindowId, b: WindowId) -> Option<&Bond> {
        let p = if a <= b { (a, b) } else { (b, a) };
        self.bonds.iter().find(|x| x.pair() == p)
    }

    pub fn neighbours(&self, id: WindowId) -> Vec<WindowId> {
        let mut v: Vec<WindowId> = self.bonds.iter().filter_map(|b| b.other(id)).collect();
        v.sort();
        v.dedup();
        v
    }

    /// The connected component containing `id`, always including `id` itself.
    pub fn component(&self, id: WindowId) -> Vec<WindowId> {
        let mut seen = BTreeSet::new();
        let mut q = VecDeque::new();
        seen.insert(id);
        q.push_back(id);
        while let Some(cur) = q.pop_front() {
            for n in self.neighbours(cur) {
                if seen.insert(n) {
                    q.push_back(n);
                }
            }
        }
        seen.into_iter().collect()
    }

    /// All components over a known window set, sorted for determinism.
    /// The window set has to be passed in: an unbonded window has no edges and
    /// so cannot be discovered from the bond list alone.
    pub fn components(&self, all: &[WindowId]) -> Vec<Vec<WindowId>> {
        let mut done: BTreeSet<WindowId> = BTreeSet::new();
        let mut out = vec![];
        let mut ids: Vec<WindowId> = all.to_vec();
        ids.sort();
        for id in ids {
            if done.contains(&id) {
                continue;
            }
            let c = self.component(id);
            for m in &c {
                done.insert(*m);
            }
            out.push(c);
        }
        out
    }
}

// ---------------------------------------------------------------- bond forming

/// The result of probing a moving window against a stationary one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Snap {
    /// Side of the *stationary* window that the moving one attaches to.
    pub edge: Edge,
    pub span: (Px, Px),
    /// Translation to apply to the moving window to sit flush.
    pub dx: Px,
    pub dy: Px,
}

fn overlap(a0: Px, a1: Px, b0: Px, b1: Px) -> Option<(Px, Px)> {
    let lo = a0.max(b0);
    let hi = a1.min(b1);
    if hi > lo {
        Some((lo, hi))
    } else {
        None
    }
}

/// Would `moving` bond to `fixed`?
///
/// The seam axis snaps flush (gap exactly zero). The perpendicular axis snaps to
/// alignment only if it is already within `threshold` — that is what makes a
/// bond feel magnetic instead of teleporting the window.
pub fn probe(moving: Rect, fixed: Rect, threshold: Px) -> Option<Snap> {
    let mut best: Option<(Px, Snap)> = None;

    // vertical seams: moving sits against fixed's right or left edge
    if overlap(moving.y, moving.bottom(), fixed.y, fixed.bottom()).is_some() {
        for (edge, target_x) in [
            (Edge::Right, fixed.right()),
            (Edge::Left, fixed.x - moving.w),
        ] {
            let dx = target_x - moving.x;
            if dx.abs() <= threshold {
                // align tops if they are already close
                let dy = if (fixed.y - moving.y).abs() <= threshold {
                    fixed.y - moving.y
                } else if (fixed.bottom() - moving.bottom()).abs() <= threshold {
                    fixed.bottom() - moving.bottom()
                } else {
                    0
                };
                let m = moving.translated(dx, dy);
                if let Some(span2) = overlap(m.y, m.bottom(), fixed.y, fixed.bottom()) {
                    let cost = dx.abs() + dy.abs();
                    let snap = Snap {
                        edge,
                        span: span2,
                        dx,
                        dy,
                    };
                    if best.is_none_or(|(c, _)| cost < c) {
                        best = Some((cost, snap));
                    }
                }
            }
        }
    }

    // horizontal seams: moving sits against fixed's bottom or top edge
    if overlap(moving.x, moving.right(), fixed.x, fixed.right()).is_some() {
        for (edge, target_y) in [
            (Edge::Bottom, fixed.bottom()),
            (Edge::Top, fixed.y - moving.h),
        ] {
            let dy = target_y - moving.y;
            if dy.abs() <= threshold {
                let dx = if (fixed.x - moving.x).abs() <= threshold {
                    fixed.x - moving.x
                } else if (fixed.right() - moving.right()).abs() <= threshold {
                    fixed.right() - moving.right()
                } else {
                    0
                };
                let m = moving.translated(dx, dy);
                if let Some(span2) = overlap(m.x, m.right(), fixed.x, fixed.right()) {
                    let cost = dx.abs() + dy.abs();
                    let snap = Snap {
                        edge,
                        span: span2,
                        dx,
                        dy,
                    };
                    if best.is_none_or(|(c, _)| cost < c) {
                        best = Some((cost, snap));
                    }
                }
            }
        }
    }

    best.map(|(_, s)| s)
}

pub type Layout = BTreeMap<WindowId, Rect>;

/// Move a whole connected component. Offsets are preserved exactly because a
/// rigid translation cannot change any relative position, so no bond can drift.
pub fn translate_group(layout: &mut Layout, group: &[WindowId], dx: Px, dy: Px) {
    for id in group {
        if let Some(r) = layout.get_mut(id) {
            *r = r.translated(dx, dy);
        }
    }
}

/// Verify every bond in the graph is still geometrically flush.
/// Returns the bonds that are not. Used by the drift tests.
pub fn violations(graph: &WindowGraph, layout: &Layout) -> Vec<(Bond, String)> {
    let mut out = vec![];
    for b in &graph.bonds {
        let (ra, rb) = match (layout.get(&b.a), layout.get(&b.b)) {
            (Some(x), Some(y)) => (*x, *y),
            _ => continue,
        };
        match b.edge {
            Edge::Right => {
                if ra.right() != rb.x {
                    out.push((*b, format!("gap {} on vertical seam", rb.x - ra.right())));
                } else if overlap(ra.y, ra.bottom(), rb.y, rb.bottom()).is_none() {
                    out.push((*b, "no overlap along seam".into()));
                }
            }
            Edge::Bottom => {
                if ra.bottom() != rb.y {
                    out.push((*b, format!("gap {} on horizontal seam", rb.y - ra.bottom())));
                } else if overlap(ra.x, ra.right(), rb.x, rb.right()).is_none() {
                    out.push((*b, "no overlap along seam".into()));
                }
            }
            _ => out.push((*b, "non-canonical bond stored".into())),
        }
    }
    out
}

// ------------------------------------------------------------------- splitter

/// D35: the cursor tells the truth. A splitter is live only where at least one
/// neighbour can actually resize; everywhere else the edge is a move handle.
pub fn splitter_is_live(a_resizable: bool, b_resizable: bool) -> bool {
    a_resizable || b_resizable
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitterOutcome {
    /// Both neighbours changed size around a stationary seam.
    BothResized,
    /// One neighbour is fixed, so it moved and the other absorbed the change.
    OneMoved,
    /// Neither can resize: not a splitter at all, the caller should move the group.
    NotLive,
    /// The requested position would push a window below its minimum.
    Clamped,
}

/// Drive a splitter to an absolute seam position, in physical px.
///
/// The seam is kept flush by construction: both sides are written from the same
/// `pos`, so there is no intermediate state in which a gap or overlap exists in
/// the model. (Whether that survives two non-atomic OS calls is a separate
/// question, and is measured in stage 4.)
pub fn apply_splitter(
    layout: &mut Layout,
    bond: &Bond,
    pos: Px,
    resizable: &dyn Fn(WindowId) -> bool,
    min_size: Px,
) -> SplitterOutcome {
    let (ra, rb) = match (layout.get(&bond.a), layout.get(&bond.b)) {
        (Some(x), Some(y)) => (*x, *y),
        _ => return SplitterOutcome::NotLive,
    };
    let (ar, br) = (resizable(bond.a), resizable(bond.b));
    if !splitter_is_live(ar, br) {
        return SplitterOutcome::NotLive;
    }

    let vertical = bond.edge.is_vertical_seam();
    let (a_lo, a_hi) = if vertical {
        (ra.x, ra.right())
    } else {
        (ra.y, ra.bottom())
    };
    let (b_lo, b_hi) = if vertical {
        (rb.x, rb.right())
    } else {
        (rb.y, rb.bottom())
    };
    debug_assert_eq!(a_hi, b_lo, "bond was not flush before the splitter drag");

    // Clamp so no *resizable* side goes under its minimum. A fixed side imposes
    // no bound at all: it keeps its size and slides, so the seam can travel past
    // where its edge used to be.
    let lo = if ar { a_lo + min_size } else { Px::MIN / 4 };
    let hi = if br { b_hi - min_size } else { Px::MAX / 4 };
    let clamped = pos.clamp(lo.min(hi), lo.max(hi));
    let outcome = if clamped != pos {
        SplitterOutcome::Clamped
    } else if ar && br {
        SplitterOutcome::BothResized
    } else {
        SplitterOutcome::OneMoved
    };

    let (na, nb) = split_sides(ra, rb, vertical, clamped, ar, br);
    layout.insert(bond.a, na);
    layout.insert(bond.b, nb);
    outcome
}

fn split_sides(ra: Rect, rb: Rect, vertical: bool, pos: Px, ar: bool, br: bool) -> (Rect, Rect) {
    let mut na = ra;
    let mut nb = rb;
    if vertical {
        if ar {
            na.w = pos - ra.x; // grows/shrinks in place
        } else {
            na.x = pos - ra.w; // fixed size, so it has to move
        }
        if br {
            nb.w = rb.right() - pos;
            nb.x = pos;
        } else {
            nb.x = pos;
        }
    } else {
        if ar {
            na.h = pos - ra.y;
        } else {
            na.y = pos - ra.h;
        }
        if br {
            nb.h = rb.bottom() - pos;
            nb.y = pos;
        } else {
            nb.y = pos;
        }
    }
    (na, nb)
}

/// Everything reachable from `from` without crossing `bond` — i.e. the rigid
/// body on that side of the seam.
pub fn side_of(graph: &WindowGraph, bond: &Bond, from: WindowId) -> Vec<WindowId> {
    let mut cut = graph.clone();
    cut.break_bond(bond.a, bond.b);
    cut.component(from)
}

/// Splitter drag that respects the rest of the group.
///
/// `apply_splitter` sees only its two neighbours. When one of them is *fixed* it
/// cannot resize, so it slides — and anything bonded to its far side has to
/// slide with it, or the seam they share opens a gap. The two-window form is
/// structurally unable to notice that; the graph is what makes it fixable.
///
/// A neighbour that *resized* does not propagate: its far edge never moved.
pub fn apply_splitter_in_graph(
    layout: &mut Layout,
    graph: &WindowGraph,
    bond: &Bond,
    pos: Px,
    resizable: &dyn Fn(WindowId) -> bool,
    min_size: Px,
) -> SplitterOutcome {
    let before = [
        (bond.a, layout.get(&bond.a).copied()),
        (bond.b, layout.get(&bond.b).copied()),
    ];
    let out = apply_splitter(layout, bond, pos, resizable, min_size);
    if out == SplitterOutcome::NotLive {
        return out;
    }
    for (id, b0) in before {
        let (Some(b0), Some(a1)) = (b0, layout.get(&id).copied()) else {
            continue;
        };
        // only a size-preserving slide drags its neighbours along
        if a1.w != b0.w || a1.h != b0.h {
            continue;
        }
        let (dx, dy) = (a1.x - b0.x, a1.y - b0.y);
        if dx == 0 && dy == 0 {
            continue;
        }
        let mut side = side_of(graph, bond, id);
        side.retain(|s| *s != id);
        translate_group(layout, &side, dx, dy);
    }
    out
}

// ---------------------------------------------------------------- screen edges

/// Screen-edge bonding is a movement constraint only — no resize, no graph node.
/// Returns the translation to apply so the group sits flush against a screen
/// edge it came within `threshold` of.
pub fn screen_edge_snap(group_bounds: Rect, screen: Rect, threshold: Px) -> (Px, Px) {
    let dx = if (group_bounds.x - screen.x).abs() <= threshold {
        screen.x - group_bounds.x
    } else if (group_bounds.right() - screen.right()).abs() <= threshold {
        screen.right() - group_bounds.right()
    } else {
        0
    };
    let dy = if (group_bounds.y - screen.y).abs() <= threshold {
        screen.y - group_bounds.y
    } else if (group_bounds.bottom() - screen.bottom()).abs() <= threshold {
        screen.bottom() - group_bounds.bottom()
    } else {
        0
    };
    (dx, dy)
}

/// Bounding box of a set of windows.
pub fn bounds(layout: &Layout, group: &[WindowId]) -> Option<Rect> {
    let mut it = group.iter().filter_map(|id| layout.get(id));
    let first = *it.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.right(), first.bottom());
    for r in it {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.right());
        y1 = y1.max(r.bottom());
    }
    Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
}

// ------------------------------------------------------------------- D40

/// D40: round once, from logical.
///
/// Every physical value is recomputed from its logical source. Nothing here
/// takes an already-rounded physical number as input, which is the whole point —
/// that is the operation that accumulates error.
pub mod d40 {
    use super::Px;

    /// The only place a logical number becomes a physical one.
    pub fn physical(logical: f64, scale: f64) -> Px {
        (logical * scale).round() as Px
    }

    /// Size after `n` steps, computed fresh from the logical base and step.
    /// NOT `physical(base) + n * physical(step)`.
    pub fn stepped(base_logical: f64, step_logical: f64, n: i32, scale: f64) -> Px {
        physical(base_logical + step_logical * n as f64, scale)
    }

    /// The snap threshold, recomputed from its logical definition every call.
    pub fn threshold(logical: f64, scale: f64) -> Px {
        physical(logical, scale)
    }

    /// The wrong way, kept only so the tests can measure how wrong it is.
    #[cfg(test)]
    pub fn stepped_naive(base_logical: f64, step_logical: f64, n: i32, scale: f64) -> Px {
        physical(base_logical, scale) + n * physical(step_logical, scale)
    }
}

// ==========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    const W: Px = 413; // 1x chrome at 150%
    const H: Px = 174;
    const T: Px = 15; // 10 logical px threshold at 150%

    fn id(n: u32) -> WindowId {
        WindowId(n)
    }
    fn a() -> WindowId {
        id(0)
    }
    fn b() -> WindowId {
        id(1)
    }
    fn c() -> WindowId {
        id(2)
    }

    fn layout3() -> Layout {
        let mut l = Layout::new();
        l.insert(a(), Rect::new(0, 0, W, H));
        l.insert(b(), Rect::new(W, 0, W, H));
        l.insert(c(), Rect::new(2 * W, 0, W, H));
        l
    }

    /// A–B–C in a row, bonded left to right.
    fn chain() -> (WindowGraph, Layout) {
        let l = layout3();
        let mut g = WindowGraph::new();
        g.insert(Bond::new(a(), b(), Edge::Right, (0, H)));
        g.insert(Bond::new(b(), c(), Edge::Right, (0, H)));
        (g, l)
    }

    // ---------------------------------------------------------- bond forming

    #[test]
    fn bonds_within_threshold_and_snaps_flush() {
        let fixed = Rect::new(0, 0, W, H);
        // 9 px short of touching, inside a 15 px threshold
        let moving = Rect::new(W + 9, 0, W, H);
        let s = probe(moving, fixed, T).expect("should bond");
        assert_eq!(s.edge, Edge::Right);
        assert_eq!(s.dx, -9);
        assert_eq!(s.dy, 0);
        let snapped = moving.translated(s.dx, s.dy);
        assert_eq!(snapped.x, fixed.right(), "must be flush, not merely close");
    }

    #[test]
    fn does_not_bond_outside_threshold() {
        let fixed = Rect::new(0, 0, W, H);
        let moving = Rect::new(W + 16, 0, W, H);
        assert!(probe(moving, fixed, T).is_none());
    }

    #[test]
    fn threshold_is_inclusive_at_the_boundary() {
        let fixed = Rect::new(0, 0, W, H);
        assert!(probe(Rect::new(W + 15, 0, W, H), fixed, T).is_some());
        assert!(probe(Rect::new(W + 16, 0, W, H), fixed, T).is_none());
    }

    #[test]
    fn aligns_perpendicular_axis_when_close() {
        let fixed = Rect::new(0, 0, W, H);
        let moving = Rect::new(W + 3, 6, W, H); // 6 px vertical misalignment
        let s = probe(moving, fixed, T).unwrap();
        assert_eq!(s.dy, -6, "tops should align when already within threshold");
    }

    #[test]
    fn leaves_perpendicular_axis_alone_when_far() {
        let fixed = Rect::new(0, 0, W, H);
        let moving = Rect::new(W + 3, 60, W, H); // far out of alignment but still overlapping
        let s = probe(moving, fixed, T).unwrap();
        assert_eq!(s.dy, 0, "should not teleport a deliberately offset window");
    }

    #[test]
    fn near_a_corner_the_nearer_seam_wins() {
        // Dragged toward a corner, both a vertical and a horizontal seam are in
        // range at once. Cheapest wins, and both orders matter: the vertical
        // seam is probed first, so "keep the newest candidate" would take the
        // horizontal one every time and "keep the first" would never take it.
        let fixed = Rect::new(0, 0, W, H);

        // 5 px from the right edge, 6 px from the bottom: the vertical seam.
        let s = probe(Rect::new(W - 5, H - 6, W, H), fixed, T).expect("should bond");
        assert_eq!(s.edge, Edge::Right, "took the dearer horizontal seam");
        assert_eq!((s.dx, s.dy), (5, 0));

        // 6 px from the right edge, 5 px from the bottom: the horizontal seam.
        let s = probe(Rect::new(W - 6, H - 5, W, H), fixed, T).expect("should bond");
        assert_eq!(s.edge, Edge::Bottom, "took the dearer vertical seam");
        assert_eq!((s.dx, s.dy), (0, 5));
    }

    #[test]
    fn no_bond_without_overlap_along_the_seam() {
        let fixed = Rect::new(0, 0, W, H);
        // horizontally adjacent but vertically disjoint
        let moving = Rect::new(W, 5000, W, H);
        assert!(probe(moving, fixed, T).is_none());
    }

    #[test]
    fn bond_is_stored_canonically_whichever_way_it_is_made() {
        let x = Bond::new(a(), b(), Edge::Right, (0, H));
        let y = Bond::new(b(), a(), Edge::Left, (0, H));
        assert_eq!(x, y, "a Left bond must normalise to the same Right bond");
        assert_eq!(y.a, a());
        assert_eq!(y.edge, Edge::Right);
    }

    #[test]
    fn re_bonding_the_same_pair_does_not_duplicate() {
        let mut g = WindowGraph::new();
        g.insert(Bond::new(a(), b(), Edge::Right, (0, H)));
        g.insert(Bond::new(a(), b(), Edge::Right, (0, H)));
        g.insert(Bond::new(b(), a(), Edge::Left, (0, H)));
        assert_eq!(g.bonds.len(), 1);
    }

    // ------------------------------------------------------ components

    #[test]
    fn chain_is_one_component() {
        let (g, _) = chain();
        assert_eq!(g.components(&[a(), b(), c()]), vec![vec![a(), b(), c()]]);
    }

    #[test]
    fn unbonded_windows_are_their_own_components() {
        let g = WindowGraph::new();
        assert_eq!(
            g.components(&[a(), b(), c()]),
            vec![vec![a()], vec![b()], vec![c()]]
        );
    }

    // ------------------------------------------- STAGE 5: the named cases

    /// A–B–C, break the middle. Two independent groups, not one confused one.
    #[test]
    fn stage5_break_middle_of_three_in_a_row() {
        let (mut g, _) = chain();
        assert!(g.break_bond(b(), c()));
        assert_eq!(
            g.components(&[a(), b(), c()]),
            vec![vec![a(), b()], vec![c()]]
        );

        // and the other middle bond, from a fresh chain
        let (mut g2, _) = chain();
        assert!(g2.break_bond(a(), b()));
        assert_eq!(
            g2.components(&[a(), b(), c()]),
            vec![vec![a()], vec![b(), c()]]
        );
    }

    /// Break a bond that is not there. Must be a no-op, not a panic or a
    /// silently corrupted graph.
    #[test]
    fn stage5_breaking_a_nonexistent_bond_is_a_noop() {
        let (mut g, _) = chain();
        assert!(!g.break_bond(a(), c()), "A and C were never bonded");
        assert_eq!(g.bonds.len(), 2);
        assert_eq!(g.components(&[a(), b(), c()]), vec![vec![a(), b(), c()]]);
    }

    /// A–B–C, break the middle, then drag A back onto C.
    /// The question is whether stale state from the old group leaks in.
    #[test]
    fn stage5_rebond_after_break_carries_no_stale_state() {
        let (mut g, mut l) = chain();

        // break A–B: {A} and {B,C}
        g.break_bond(a(), b());
        assert_eq!(
            g.components(&[a(), b(), c()]),
            vec![vec![a()], vec![b(), c()]]
        );

        // drag A around to C's right-hand side, landing 7 px short
        let c_rect = l[&c()];
        let dragged = Rect::new(c_rect.right() + 7, c_rect.y, W, H);
        let s = probe(dragged, c_rect, T).expect("A should bond to C");
        let snapped = dragged.translated(s.dx, s.dy);
        l.insert(a(), snapped);
        g.insert(Bond::new(c(), a(), s.edge, s.span));

        // one component again, and crucially A–B is NOT resurrected
        assert_eq!(g.components(&[a(), b(), c()]), vec![vec![a(), b(), c()]]);
        assert!(
            g.bond_between(a(), b()).is_none(),
            "stale A-B bond leaked back in"
        );
        assert!(g.bond_between(c(), a()).is_some());
        assert_eq!(g.bonds.len(), 2, "exactly B-C and C-A");
        assert!(violations(&g, &l).is_empty(), "{:?}", violations(&g, &l));

        // and breaking the NEW bond splits along the new topology, not the old
        g.break_bond(c(), a());
        assert_eq!(
            g.components(&[a(), b(), c()]),
            vec![vec![a()], vec![b(), c()]]
        );
    }

    /// L shape: A right of B, C below B. Break A–B. Does C stay with B?
    #[test]
    fn stage5_l_shape_break_keeps_the_perpendicular_bond() {
        let mut l = Layout::new();
        let rb = Rect::new(500, 500, W, H);
        l.insert(b(), rb);
        l.insert(a(), Rect::new(rb.right(), rb.y, W, H)); // A right of B
        l.insert(c(), Rect::new(rb.x, rb.bottom(), W, H)); // C below B

        let mut g = WindowGraph::new();
        g.insert(Bond::new(b(), a(), Edge::Right, (rb.y, rb.bottom())));
        g.insert(Bond::new(b(), c(), Edge::Bottom, (rb.x, rb.right())));
        assert_eq!(g.components(&[a(), b(), c()]), vec![vec![a(), b(), c()]]);
        assert!(violations(&g, &l).is_empty());

        g.break_bond(a(), b());
        assert_eq!(
            g.components(&[a(), b(), c()]),
            vec![vec![a()], vec![b(), c()]],
            "C must stay with B; breaking a horizontal bond cannot disturb a vertical one"
        );
        assert!(g.bond_between(b(), c()).is_some());
    }

    /// Breaking a bond while a splitter drag is in flight on the other edge of
    /// the same window. The in-flight splitter must not resurrect the bond or
    /// corrupt the window it was resizing.
    #[test]
    fn stage5_break_during_an_in_flight_splitter_on_the_other_edge() {
        // A | B | C, with B resizable so B|C is a live splitter
        let mut l = layout3();
        let mut g = WindowGraph::new();
        g.insert(Bond::new(a(), b(), Edge::Right, (0, H)));
        g.insert(Bond::new(b(), c(), Edge::Right, (0, H)));
        let resizable = |w: WindowId| w == b() || w == c();

        // splitter drag begins on the B|C seam
        let bc = *g.bond_between(b(), c()).unwrap();
        apply_splitter(&mut l, &bc, 2 * W - 40, &resizable, 50);
        assert!(
            violations(&g, &l).is_empty(),
            "seam should stay flush mid-drag"
        );

        // mid-drag, the A|B bond is broken
        g.break_bond(a(), b());
        assert_eq!(
            g.components(&[a(), b(), c()]),
            vec![vec![a()], vec![b(), c()]]
        );

        // the drag continues against the still-valid bond object
        let outcome = apply_splitter(&mut l, &bc, 2 * W - 80, &resizable, 50);
        assert_eq!(outcome, SplitterOutcome::BothResized);
        assert!(
            violations(&g, &l).is_empty(),
            "in-flight splitter corrupted the surviving graph: {:?}",
            violations(&g, &l)
        );
        assert!(g.bond_between(a(), b()).is_none(), "broken bond came back");
    }

    /// The component result is what drives z-order grouping, so it has to be
    /// correct at every step of a break/rebond sequence, not just at the end.
    #[test]
    fn stage5_components_correct_at_every_step() {
        let (mut g, _) = chain();
        let all = [a(), b(), c()];
        let steps: Vec<Vec<Vec<WindowId>>> = vec![
            g.components(&all),
            {
                g.break_bond(b(), c());
                g.components(&all)
            },
            {
                g.break_bond(a(), b());
                g.components(&all)
            },
            {
                g.insert(Bond::new(a(), c(), Edge::Right, (0, H)));
                g.components(&all)
            },
        ];
        assert_eq!(steps[0], vec![vec![a(), b(), c()]]);
        assert_eq!(steps[1], vec![vec![a(), b()], vec![c()]]);
        assert_eq!(steps[2], vec![vec![a()], vec![b()], vec![c()]]);
        assert_eq!(steps[3], vec![vec![a(), c()], vec![b()]]);
    }

    // ------------------------------------------------------- group move

    #[test]
    fn group_move_preserves_offsets_exactly() {
        let (g, mut l) = chain();
        let group = g.component(a());
        let before = l.clone();
        translate_group(&mut l, &group, 137, -49);
        for id in &group {
            assert_eq!(l[id].x, before[id].x + 137);
            assert_eq!(l[id].y, before[id].y - 49);
        }
        assert!(violations(&g, &l).is_empty());
    }

    /// "No drift after twenty drags" — the stated stage 3 pass criterion.
    #[test]
    fn no_drift_after_twenty_group_drags() {
        let (g, mut l) = chain();
        let group = g.component(a());
        let before = l.clone();
        let moves = [(37, 11), (-9, 43), (120, -77), (-148, 23)];
        for i in 0..20 {
            let (dx, dy) = moves[i % moves.len()];
            translate_group(&mut l, &group, dx, dy);
            assert!(violations(&g, &l).is_empty(), "bond broke on drag {}", i);
        }
        // undo exactly
        let (sx, sy): (Px, Px) = moves
            .iter()
            .cycle()
            .take(20)
            .fold((0, 0), |acc, m| (acc.0 + m.0, acc.1 + m.1));
        translate_group(&mut l, &group, -sx, -sy);
        assert_eq!(l, before, "twenty drags and back should be bit-identical");
    }

    #[test]
    fn moving_one_group_does_not_touch_another() {
        let (mut g, mut l) = chain();
        g.break_bond(a(), b());
        let g1 = g.component(a());
        let c_before = l[&c()];
        translate_group(&mut l, &g1, 500, 500);
        assert_eq!(l[&c()], c_before);
    }

    // -------------------------------------------------------- splitter

    #[test]
    fn splitter_both_resizable_keeps_the_seam_flush() {
        let mut l = layout3();
        let mut g = WindowGraph::new();
        g.insert(Bond::new(a(), b(), Edge::Right, (0, H)));
        let bond = *g.bond_between(a(), b()).unwrap();
        let all = |_: WindowId| true;
        for pos in [W - 60, W - 20, W + 30, W + 90] {
            let out = apply_splitter(&mut l, &bond, pos, &all, 50);
            assert_eq!(out, SplitterOutcome::BothResized);
            assert_eq!(l[&a()].right(), l[&b()].x, "gap/overlap at seam");
            assert_eq!(l[&a()].right(), pos);
            assert!(violations(&g, &l).is_empty());
        }
    }

    #[test]
    fn splitter_with_one_fixed_neighbour_moves_it_instead() {
        let mut l = layout3();
        let mut g = WindowGraph::new();
        g.insert(Bond::new(a(), b(), Edge::Right, (0, H)));
        let bond = *g.bond_between(a(), b()).unwrap();
        let only_b = |w: WindowId| w == b();

        let a_w_before = l[&a()].w;
        let out = apply_splitter(&mut l, &bond, W - 40, &only_b, 50);
        assert_eq!(out, SplitterOutcome::OneMoved);
        assert_eq!(
            l[&a()].w,
            a_w_before,
            "a is fixed, its size must not change"
        );
        assert_eq!(l[&a()].right(), l[&b()].x, "seam still flush");
        assert!(violations(&g, &l).is_empty());
    }

    #[test]
    fn fixed_fixed_edge_is_not_a_splitter() {
        let mut l = layout3();
        let mut g = WindowGraph::new();
        g.insert(Bond::new(a(), b(), Edge::Right, (0, H)));
        let bond = *g.bond_between(a(), b()).unwrap();
        let none = |_: WindowId| false;
        let before = l.clone();
        assert_eq!(
            apply_splitter(&mut l, &bond, W - 40, &none, 50),
            SplitterOutcome::NotLive
        );
        assert_eq!(l, before, "a dead splitter must not move anything");
        assert!(!splitter_is_live(false, false));
        assert!(splitter_is_live(false, true));
    }

    #[test]
    fn splitter_clamps_at_minimum_size() {
        let mut l = layout3();
        let mut g = WindowGraph::new();
        g.insert(Bond::new(a(), b(), Edge::Right, (0, H)));
        let bond = *g.bond_between(a(), b()).unwrap();
        let all = |_: WindowId| true;
        let out = apply_splitter(&mut l, &bond, 10, &all, 50);
        assert_eq!(out, SplitterOutcome::Clamped);
        assert!(l[&a()].w >= 50);
        assert_eq!(l[&a()].right(), l[&b()].x, "clamping must not open a gap");
    }

    /// The bug the real windows found: a fixed neighbour slides, and its far-side
    /// bond opens a gap unless the whole rigid body slides with it.
    #[test]
    fn splitter_propagates_a_sliding_fixed_neighbour_through_the_group() {
        // main | eq | playlist, only playlist resizable (D35)
        let (m, e, pl) = (a(), b(), c());
        let mut l = layout3();
        let mut g = WindowGraph::new();
        g.insert(Bond::new(m, e, Edge::Right, (0, H)));
        g.insert(Bond::new(e, pl, Edge::Right, (0, H)));
        let only_pl = |w: WindowId| w == pl;
        let ep = *g.bond_between(e, pl).unwrap();

        // two-neighbour form: eq slides, main is left behind -> gap
        let mut naive = l.clone();
        apply_splitter(&mut naive, &ep, 2 * W + 274, &only_pl, 50);
        assert!(
            !violations(&g, &naive).is_empty(),
            "the two-window form should have opened a gap; if it did not, this test is stale"
        );

        // graph-aware form: main comes along
        let out = apply_splitter_in_graph(&mut l, &g, &ep, 2 * W + 274, &only_pl, 50);
        assert_eq!(out, SplitterOutcome::OneMoved);
        assert!(violations(&g, &l).is_empty(), "{:?}", violations(&g, &l));
        assert_eq!(l[&m].w, W, "main is fixed, it must slide not stretch");
        assert_eq!(l[&e].w, W, "eq is fixed too");
        assert_eq!(l[&m].x, 274, "main slid by exactly the seam delta");
        assert_eq!(l[&pl].w, W - 274, "playlist absorbed the change");
    }

    /// A resizable neighbour must NOT propagate: its far edge never moved.
    #[test]
    fn splitter_does_not_propagate_a_resize() {
        let (m, e, pl) = (a(), b(), c());
        let mut l = layout3();
        let mut g = WindowGraph::new();
        g.insert(Bond::new(m, e, Edge::Right, (0, H)));
        g.insert(Bond::new(e, pl, Edge::Right, (0, H)));
        let all = |_: WindowId| true;
        let ep = *g.bond_between(e, pl).unwrap();
        let m_before = l[&m];
        apply_splitter_in_graph(&mut l, &g, &ep, 2 * W - 60, &all, 50);
        assert_eq!(l[&m], m_before, "main must not move when eq merely resized");
        assert!(violations(&g, &l).is_empty());
    }

    #[test]
    fn side_of_splits_the_graph_at_the_seam() {
        let (g, _) = chain();
        let ab = *g.bond_between(a(), b()).unwrap();
        assert_eq!(side_of(&g, &ab, a()), vec![a()]);
        assert_eq!(side_of(&g, &ab, b()), vec![b(), c()]);
    }

    #[test]
    fn splitter_on_a_horizontal_seam() {
        let mut l = Layout::new();
        l.insert(a(), Rect::new(0, 0, W, H));
        l.insert(b(), Rect::new(0, H, W, H));
        let mut g = WindowGraph::new();
        g.insert(Bond::new(a(), b(), Edge::Bottom, (0, W)));
        let bond = *g.bond_between(a(), b()).unwrap();
        let all = |_: WindowId| true;
        assert_eq!(
            apply_splitter(&mut l, &bond, H + 30, &all, 50),
            SplitterOutcome::BothResized
        );
        assert_eq!(l[&a()].bottom(), l[&b()].y);
        assert!(violations(&g, &l).is_empty());
    }

    // --------------------------------------------------- screen edges

    #[test]
    fn group_snaps_to_screen_edge_as_a_movement_constraint() {
        let (g, mut l) = chain();
        let group = g.component(a());
        let screen = Rect::new(0, 0, 2560, 1080);
        translate_group(&mut l, &group, 9, 1080 - H - 11);
        let bb = bounds(&l, &group).unwrap();
        let (dx, dy) = screen_edge_snap(bb, screen, T);
        assert_eq!(dx, -9, "left edge should snap flush to x=0");
        assert_eq!(dy, 11, "bottom edge should snap flush to the screen bottom");
        translate_group(&mut l, &group, dx, dy);
        // it is a constraint, not a resize: every window keeps its size
        assert_eq!(l[&a()].w, W);
        assert_eq!(l[&a()].h, H);
        assert!(violations(&g, &l).is_empty());
    }

    #[test]
    fn no_screen_snap_when_far_away() {
        let screen = Rect::new(0, 0, 2560, 1080);
        assert_eq!(
            screen_edge_snap(Rect::new(400, 400, W, H), screen, T),
            (0, 0)
        );
    }

    // -------------------------------------------------------------- D40

    #[test]
    fn d40_threshold_is_recomputed_from_logical() {
        assert_eq!(d40::threshold(10.0, 1.5), 15);
        assert_eq!(d40::threshold(10.0, 1.0), 10);
        assert_eq!(d40::threshold(10.0, 2.0), 20);
    }

    #[test]
    fn d40_rounding_does_not_commute_with_doubling() {
        // the stage 0 finding, pinned as a test
        let one_x = d40::physical(275.0, 1.5);
        let two_x = d40::physical(275.0 * 2.0, 1.5);
        assert_eq!(one_x, 413);
        assert_eq!(two_x, 825);
        assert_ne!(one_x * 2, two_x, "413*2 = 826, not 825");
    }

    #[test]
    fn d40_stepping_from_logical_does_not_drift() {
        // winamp playlist steps: 25 x 29 logical
        let (base, step, scale) = (275.0, 25.0, 1.5);
        for n in 0..=20 {
            let correct = d40::stepped(base, step, n, scale);
            let expected = ((275.0 + 25.0 * n as f64) * 1.5).round() as Px;
            assert_eq!(correct, expected, "step {} drifted", n);
        }
    }

    /// The D40 drift, quantified. 25 logical x 1.5 = 37.5 physical, which rounds
    /// to 38, so the naive form gains half a pixel every step and the error
    /// grows without bound. These are the numbers that go in the findings doc.
    #[test]
    fn d40_naive_stepping_drifts_and_here_is_by_how_much() {
        let (base, step, scale) = (275.0, 25.0, 1.5);

        // at ten steps
        assert_eq!(d40::stepped(base, step, 10, scale), 788); // (275+250)*1.5 = 787.5
        assert_eq!(d40::stepped_naive(base, step, 10, scale), 793); // 413 + 10*38
        assert_eq!(
            d40::stepped_naive(base, step, 10, scale) - d40::stepped(base, step, 10, scale),
            5,
            "drift after ten steps"
        );

        // and it keeps growing: roughly n/2 px after n steps
        assert_eq!(
            d40::stepped_naive(base, step, 20, scale) - d40::stepped(base, step, 20, scale),
            10
        );
        assert_eq!(
            d40::stepped_naive(base, step, 40, scale) - d40::stepped(base, step, 40, scale),
            20
        );

        // the correct form never drifts, by construction
        for n in 0..=40 {
            assert_eq!(
                d40::stepped(base, step, n, scale),
                ((base + step * n as f64) * scale).round() as Px
            );
        }
    }

    #[test]
    fn d40_height_steps_too() {
        let (base, step, scale) = (116.0, 29.0, 1.5);
        assert_eq!(d40::stepped(base, step, 0, scale), 174);
        assert_eq!(d40::stepped(base, step, 1, scale), 218); // (116+29)*1.5 = 217.5 -> 218
        assert_eq!(d40::stepped(base, step, 2, scale), 261); // (116+58)*1.5 = 261.0
    }
}
