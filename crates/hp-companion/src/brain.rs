//! What he does, moment to moment. Pure: time, the ledges, the window he is
//! standing on and what the OS says about it, whether music is playing and
//! whether a beat just landed go in; his feet, his facing and his state come
//! out. The pack turns that into a frame (`pose`). The seven states are the
//! app's, fixed (`purricane.md`).
//!
//! - **idle** by default, with a walk now and then to somewhere else on the
//!   ledge he is on, across a seam if two windows sit side by side.
//! - **dance** while music plays: a frame per beat from the viz stream, and
//!   between beats the groove the last ones set, so a song with a soft kick
//!   or none still gets him dancing (#222).
//! - **sleep** once nothing has played for a while; music wakes him.
//! - **startle**: when the window under him moves, shades, hides or goes, or
//!   the stretch he stands on stops being a ledge, he jumps and falls to the
//!   next ledge below, the floor above the taskbar at the bottom of it all.
//!   When another window is in front of his, where his feet are, he is
//!   standing on nothing: he jumps and falls straight to the floor (D166).
//! - **pet**: a click on him. He leans into it for a moment, awake again.
//! - **carry**: a drag. He hangs from the hand by his scruff, kicking, and
//!   when he is let go he falls to the ledge below where he was dropped.
//!
//! When the window he stands on is minimised, he is minimised with it: not
//! drawn, and back where he stood when it is restored (D166).
//!
//! A jolt or a pet wakes him and starts the quiet over, so he does not doze
//! off again the moment he lands.

use crate::pack::Pack;
use crate::perch::{self, Ledge};
use hp_control::{LayoutInfo, WindowRect};

/// Nothing playing for this long and he curls up.
pub const SLEEP_AFTER: f32 = 30.0;
/// How long he stands between walks, at random within.
pub const IDLE_FOR: (f32, f32) = (5.0, 14.0);
/// A walk goes at least this far (1x pixels), or it is not worth the frames.
pub const MIN_WALK: f32 = 48.0;
/// While music plays he dances the whole time (#222, D175). A beat the player
/// finds is a step; between them he keeps the groove the last ones set, and
/// until there are any, this one: a step every 0.55 s, about 110 a minute.
pub const GROOVE: f32 = 0.55;
/// The groove is folded into this range by halving and doubling, so a
/// detector that hears only every other kick still gives him the right time,
/// and a busy hi-hat does not make him frantic.
pub const STEP_RANGE: (f32, f32) = (0.35, 0.9);
/// A beat this soon after a step he took on his own is the same beat,
/// arriving late: it moves his timing, not his feet, so he never steps twice.
pub const SAME_BEAT: f32 = 0.2;
/// The gaps between beats a groove is taken from, the newest this many.
const GAPS: usize = 8;
/// A gap outside this is a break or a stutter, not a tempo.
const GAP_RANGE: (f32, f32) = (0.2, 2.0);
/// Gravity and the startle's hop, in 1x pixels per second (squared).
pub const GRAVITY: f32 = 2400.0;
pub const HOP: f32 = 420.0;
/// He holds the landing frame this long before standing.
pub const LANDING: f32 = 0.3;
/// A pet lasts this long, whatever the pack's frame count.
pub const PET_FOR: f32 = 1.5;
/// A press that moves further than this (physical pixels, either axis) is a
/// drag; less is a click. Windows' own drag threshold is the same size.
pub const DRAG_PX: i32 = 4;
/// Carried, the hand holds him this far (1x pixels) below the top of his
/// sprite: the scruff, where the carry frames stretch his jacket up to.
pub const SCRUFF: f32 = 4.0;
/// Something in front of his window where his feet are, for this long, and he
/// is standing on nothing (D166). Long enough for a window dragged across him.
pub const COVERED_FOR: f32 = 0.4;
/// How far into his window's top (physical pixels) the OS is asked what is
/// there: under his feet, and clear of his own sprite, which ends at them.
pub const UNDERFOOT: i32 = 2;

/// `Air` with `floor` falls straight to the floor, past every window's top.
/// `Minimised`: his window is, and so is he, not drawn, `along` its top from
/// its left edge, which is where he comes back when it does (D166).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Idle { until: f32 },
    Walk { to: f32 },
    Dance,
    Sleep,
    Air { vy: f32, floor: bool },
    Landing { until: f32 },
    Pet { until: f32 },
    Carry,
    Minimised { along: i32 },
}

/// The pointer on him, in screen pixels. His window turns its mouse messages
/// into these, and only his own opaque pixels get them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hand {
    Down(i32, i32),
    Move(i32, i32),
    Up(i32, i32),
    /// The mouse was taken away mid-press (another window captured it).
    Cancel,
}

/// A press on him: where it started, and whether it has become a drag.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Grab {
    from: (i32, i32),
    carrying: bool,
}

/// The window under his feet as it was when he landed, so a move is noticed.
#[derive(Debug, Clone, PartialEq)]
struct Under {
    id: String,
    rect: (i32, i32, i32, i32),
    shaded: bool,
}

impl Under {
    fn of(w: &WindowRect) -> Under {
        Under {
            id: w.id.clone(),
            rect: (w.x, w.y, w.w, w.h),
            shaded: w.shaded,
        }
    }
}

/// The player's window he stands on or is minimised with, for the OS to look
/// at (`Brain::perch`): its id and rectangle from the layout, and the point
/// just under his feet on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Spot<'a> {
    pub id: &'a str,
    pub rect: (i32, i32, i32, i32),
    pub at: (i32, i32),
}

/// What the OS says about that window, which the layout cannot: it has no
/// z-order, and a minimised window is `visible: false` there just as a hidden
/// one is (D148). Both false when there is no such window, or it cannot be
/// found, and then he behaves as he did before D166.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    /// Another window is in front of it at the spot under his feet.
    pub covered: bool,
    pub minimised: bool,
}

/// What the world looks like this tick.
pub struct World<'a> {
    pub layout: &'a LayoutInfo,
    pub ledges: &'a [Ledge],
    pub playing: bool,
    /// A beat arrived since the last tick.
    pub beat: bool,
    /// The player's zoom (1 or 2): speeds and distances scale with it.
    pub zoom: f32,
    /// The pack's walking speed at 1x.
    pub walk_px_per_sec: f32,
    /// What the OS says about the window he is on, as `perch` asked it.
    pub seen: Seen,
}

pub struct Brain {
    /// His feet: x of the contact point, y of the edge under it.
    feet: (f32, f32),
    left: bool,
    mode: Mode,
    since: f32,
    clock: f32,
    quiet_since: Option<f32>,
    /// Steps danced, which is the dance frame; when the last was taken; when
    /// the player last found a beat; the gaps between its latest beats.
    steps: u32,
    last_step: f32,
    last_beat: f32,
    gaps: Vec<f32>,
    /// `None` on a floor or in the air.
    under: Option<Under>,
    placed: bool,
    grab: Option<Grab>,
    /// When the OS first said something was in front of his window.
    covered_since: Option<f32>,
    rng: u64,
}

/// Which frame to draw and where: `feet` is where the anchor goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub state: &'static str,
    pub cell: u32,
    pub flip: bool,
    pub feet: (i32, i32),
}

impl Brain {
    pub fn new(seed: u64) -> Brain {
        Brain {
            feet: (0.0, 0.0),
            left: false,
            mode: Mode::Idle { until: 0.0 },
            since: 0.0,
            clock: 0.0,
            quiet_since: Some(0.0),
            steps: 0,
            last_step: 0.0,
            last_beat: f32::NEG_INFINITY,
            gaps: Vec::new(),
            under: None,
            placed: false,
            grab: None,
            covered_since: None,
            rng: seed | 1,
        }
    }

    /// The player's windows are gone: next time they show, he appears afresh.
    pub fn leave(&mut self) {
        self.placed = false;
        self.under = None;
        self.grab = None;
        self.covered_since = None;
        self.set(Mode::Idle { until: 0.0 });
    }

    /// The pointer, as his window heard it. `hang` is how far below the hand
    /// his feet are while he is carried, in screen pixels.
    pub fn hand(&mut self, h: Hand, hang: f32) {
        if !self.is_shown() {
            return;
        }
        match h {
            Hand::Down(x, y) => {
                self.grab = Some(Grab {
                    from: (x, y),
                    carrying: false,
                })
            }
            Hand::Move(x, y) => {
                let Some(g) = self.grab else { return };
                let far = (x - g.from.0).abs() > DRAG_PX || (y - g.from.1).abs() > DRAG_PX;
                if !g.carrying && far {
                    self.grab = Some(Grab {
                        carrying: true,
                        ..g
                    });
                    self.under = None;
                    self.set(Mode::Carry);
                }
                if self.mode == Mode::Carry {
                    self.feet = (x as f32, y as f32 + hang);
                }
            }
            Hand::Up(..) => match self.grab.take() {
                Some(Grab { carrying: true, .. }) => self.set(Mode::Air {
                    vy: 0.0,
                    floor: false,
                }),
                Some(_) if !matches!(self.mode, Mode::Air { .. }) => {
                    self.wake();
                    self.set(Mode::Pet {
                        until: self.clock + PET_FOR,
                    });
                }
                _ => {}
            },
            Hand::Cancel => {
                if let Some(Grab { carrying: true, .. }) = self.grab.take() {
                    self.set(Mode::Air {
                        vy: 0.0,
                        floor: false,
                    });
                }
            }
        }
    }

    /// Start the quiet over: whatever woke him, he stays up a while.
    fn wake(&mut self) {
        if self.quiet_since.is_some() {
            self.quiet_since = Some(self.clock);
        }
    }

    pub fn is_placed(&self) -> bool {
        self.placed
    }

    /// Placed, and not minimised with his window: drawn, and there to touch.
    pub fn is_shown(&self) -> bool {
        self.placed && !matches!(self.mode, Mode::Minimised { .. })
    }

    /// The player's window to ask the OS about: the one he stands on, or is
    /// minimised with. `None` on a floor, in the air and in the hand.
    pub fn perch(&self) -> Option<Spot<'_>> {
        let u = self.under.as_ref().filter(|_| self.placed)?;
        let (x, y) = self.feet();
        Some(Spot {
            id: &u.id,
            rect: u.rect,
            at: (x, y + UNDERFOOT),
        })
    }

    #[cfg(test)]
    pub fn state(&self) -> &'static str {
        match self.mode {
            Mode::Idle { .. } => "idle",
            Mode::Walk { .. } => "walk",
            Mode::Dance => "dance",
            Mode::Sleep => "sleep",
            Mode::Air { .. } | Mode::Landing { .. } => "startle",
            Mode::Pet { .. } => "pet",
            Mode::Carry => "carry",
            Mode::Minimised { .. } => "minimised",
        }
    }

    pub fn feet(&self) -> (i32, i32) {
        (self.feet.0.round() as i32, self.feet.1.round() as i32)
    }

    pub fn step(&mut self, dt: f32, w: &World) {
        self.clock += dt;
        if w.beat {
            self.heard_beat();
        }
        if w.playing {
            self.quiet_since = None;
        } else if self.quiet_since.is_none() {
            self.quiet_since = Some(self.clock);
        }

        // His window minimised under him: so is he, until it is back.
        if w.seen.minimised && self.is_shown() {
            if let Some(u) = &self.under {
                let along = self.feet().0 - u.rect.0;
                self.grab = None;
                self.covered_since = None;
                self.set(Mode::Minimised { along });
            }
        }
        if let Mode::Minimised { along } = self.mode {
            if !self.back(along, w) {
                return;
            }
        }
        // None of the player's windows showing: standing on them, he goes
        // with them (D154). On the floor he is detached from them, and stays
        // out front until he is put back on a window (D168); falling or held,
        // he is on his way there.
        if perch::shown(w.layout).next().is_none() && (!self.placed || self.under.is_some()) {
            self.leave();
            return;
        }

        if !self.placed {
            let Some((x, y)) = perch::spawn(w.layout, w.ledges) else {
                return;
            };
            self.feet = (x as f32, y as f32);
            self.under = self.window_under(w);
            self.placed = true;
            let rest = self.rest();
            self.set(Mode::Idle { until: rest });
        }

        let grounded = !matches!(self.mode, Mode::Air { .. } | Mode::Carry);
        if grounded && self.ground_gone(w) {
            self.startle(w, false);
        } else if grounded && self.under.is_some() && w.seen.covered {
            // Another window in front of his: he is standing on nothing.
            let since = *self.covered_since.get_or_insert(self.clock);
            if self.clock - since >= COVERED_FOR {
                self.startle(w, true);
            }
        } else {
            self.covered_since = None;
        }

        match self.mode {
            Mode::Air { vy, floor } => self.fall(dt, vy, floor, w),
            Mode::Carry | Mode::Minimised { .. } => {}
            Mode::Landing { until } | Mode::Pet { until } => {
                if self.clock >= until {
                    let rest = self.rest();
                    self.set(Mode::Idle { until: rest });
                }
            }
            _ if w.playing => {
                if self.mode == Mode::Dance {
                    self.keep_groove();
                } else {
                    self.set(Mode::Dance);
                    self.last_step = self.clock;
                }
            }
            Mode::Dance => {
                let rest = self.rest();
                self.set(Mode::Idle { until: rest });
            }
            Mode::Sleep => {}
            Mode::Idle { until } => {
                let quiet = self.quiet_since.map(|t| self.clock - t).unwrap_or(0.0);
                if quiet >= SLEEP_AFTER {
                    self.set(Mode::Sleep);
                } else if self.clock >= until {
                    self.start_walk(w);
                }
            }
            Mode::Walk { to } => {
                let step = w.walk_px_per_sec * w.zoom * dt;
                let x = self.feet.0;
                if (to - x).abs() <= step {
                    self.feet.0 = to;
                    self.under = self.window_under(w);
                    let rest = self.rest();
                    self.set(Mode::Idle { until: rest });
                } else {
                    self.feet.0 += step * (to - x).signum();
                    self.under = self.window_under(w);
                }
            }
        }
    }

    /// The frame for now, from the pack's states.
    pub fn pose(&self, pack: &Pack) -> Pose {
        let (state, cell) = match self.mode {
            Mode::Air { .. } => ("startle", first(pack, "startle")),
            Mode::Landing { .. } => ("startle", last(pack, "startle")),
            Mode::Pet { .. } => ("pet", timed(pack, "pet", self.clock - self.since)),
            Mode::Carry => ("carry", timed(pack, "carry", self.clock - self.since)),
            Mode::Dance => {
                let d = &pack.state("dance").frames;
                ("dance", d[self.steps as usize % d.len()])
            }
            // Minimised he is not drawn; idle is a frame every pack has.
            Mode::Idle { .. } | Mode::Minimised { .. } => ("idle", timed(pack, "idle", self.clock)),
            Mode::Walk { .. } => ("walk", timed(pack, "walk", self.clock - self.since)),
            Mode::Sleep => ("sleep", timed(pack, "sleep", self.clock)),
        };
        // He faces the way he went, but dancing and asleep he is shown as
        // drawn: a shout's words and a sleeper's z's read the right way
        // round, where a mirror spells them backwards (D167).
        let as_drawn = matches!(self.mode, Mode::Dance | Mode::Sleep);
        Pose {
            state,
            cell,
            flip: self.left && !as_drawn,
            feet: self.feet(),
        }
    }

    fn set(&mut self, m: Mode) {
        self.mode = m;
        self.since = self.clock;
    }

    /// A beat from the player: a step, unless he has just taken this one on
    /// his own, and a gap for the groove.
    fn heard_beat(&mut self) {
        let gap = self.clock - self.last_beat;
        if (GAP_RANGE.0..=GAP_RANGE.1).contains(&gap) {
            if self.gaps.len() == GAPS {
                self.gaps.remove(0);
            }
            self.gaps.push(gap);
        }
        self.last_beat = self.clock;
        if self.clock - self.last_step >= SAME_BEAT {
            self.steps = self.steps.wrapping_add(1);
        }
        self.last_step = self.clock;
    }

    /// Dancing with no beat due yet: a step of his own when the groove says
    /// one is. Timed from where the step belonged, not the tick that noticed,
    /// so the groove does not drift.
    fn keep_groove(&mut self) {
        let every = self.groove();
        if self.clock - self.last_step >= every {
            self.steps = self.steps.wrapping_add(1);
            self.last_step += every;
            if self.clock - self.last_step >= every {
                self.last_step = self.clock;
            }
        }
    }

    /// The time between steps: the middle of the latest gaps between beats,
    /// folded into `STEP_RANGE`, or `GROOVE` until there are three.
    fn groove(&self) -> f32 {
        if self.gaps.len() < 3 {
            return GROOVE;
        }
        let mut g = self.gaps.clone();
        g.sort_by(f32::total_cmp);
        let mut t = g[g.len() / 2];
        while t > STEP_RANGE.1 {
            t /= 2.0;
        }
        while t < STEP_RANGE.0 {
            t *= 2.0;
        }
        t
    }

    fn rest(&mut self) -> f32 {
        self.clock + IDLE_FOR.0 + self.rand() * (IDLE_FOR.1 - IDLE_FOR.0)
    }

    /// xorshift64*: enough to pick a moment and a place, and seedable in tests.
    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn ledge<'a>(&self, w: &'a World) -> Option<&'a Ledge> {
        let (x, y) = self.feet();
        w.ledges.iter().find(|l| l.y == y && l.holds(x))
    }

    fn window_under(&self, w: &World) -> Option<Under> {
        let (x, y) = self.feet();
        perch::shown(w.layout)
            .find(|win| win.y == y && x >= win.x && x < win.x + win.w)
            .map(Under::of)
    }

    /// Whether what he stands on has gone out from under him.
    fn ground_gone(&self, w: &World) -> bool {
        if self.ledge(w).is_none() {
            return true;
        }
        match &self.under {
            None => false,
            Some(u) => match perch::shown(w.layout).find(|win| win.id == u.id) {
                None => true,
                Some(win) => Under::of(win) != *u,
            },
        }
    }

    /// A jolt: he hops and falls, to the first ledge his feet pass, or with
    /// `floor` straight to the floor.
    fn startle(&mut self, w: &World, floor: bool) {
        self.set(Mode::Air {
            vy: -HOP * w.zoom,
            floor,
        });
        self.under = None;
        self.covered_since = None;
        self.wake();
    }

    /// Minimised with his window: whether that is over, with him back where
    /// he stood on it, or gone with it to appear afresh. The layout is up to
    /// 50 ms behind the OS, so he waits until both say the window is up.
    fn back(&mut self, along: i32, w: &World) -> bool {
        if w.seen.minimised {
            return false;
        }
        let Some(id) = self.under.as_ref().map(|u| u.id.clone()) else {
            self.leave();
            return true;
        };
        let Some(win) = w.layout.windows.iter().find(|win| win.id == id) else {
            self.leave();
            return true;
        };
        if !perch::shown(w.layout).any(|s| s.id == id) {
            return false;
        }
        // As far along its top as before, within its width if that changed,
        // on the stretch of it that is a ledge nearest there.
        let want = win.x + along.clamp(0, win.w - 1);
        let x = w
            .ledges
            .iter()
            .filter(|l| l.y == win.y && l.on.contains(&id))
            .map(|l| (l.x0.max(win.x), l.x1.min(win.x + win.w) - 1))
            .filter(|(lo, hi)| lo <= hi)
            .map(|(lo, hi)| want.clamp(lo, hi))
            .min_by_key(|x| (x - want).abs());
        let Some(x) = x else {
            // No room on it any more (something bonded on top, say).
            self.leave();
            return true;
        };
        self.feet = (x as f32, win.y as f32);
        self.under = self.window_under(w);
        let rest = self.rest();
        self.set(Mode::Idle { until: rest });
        true
    }

    fn start_walk(&mut self, w: &World) {
        let Some(l) = self.ledge(w) else { return };
        let (x0, x1) = (l.x0 as f32, (l.x1 - 1) as f32);
        let min = MIN_WALK * w.zoom;
        if x1 - x0 < min {
            let rest = self.rest();
            self.set(Mode::Idle { until: rest });
            return;
        }
        let x = self.feet.0;
        let mut to = x0 + self.rand() * (x1 - x0);
        if (to - x).abs() < min {
            to = if x - x0 > x1 - x { x - min } else { x + min };
        }
        let to = to.clamp(x0, x1).round();
        self.left = to < x;
        self.set(Mode::Walk { to });
    }

    fn fall(&mut self, dt: f32, vy: f32, floor: bool, w: &World) {
        let vy = vy + GRAVITY * w.zoom * dt;
        let (x, y0) = (self.feet.0.round() as i32, self.feet.1);
        let y1 = y0 + vy * dt;
        if vy > 0.0 {
            // The first ledge his feet pass on the way down.
            let land = w
                .ledges
                .iter()
                .filter(|l| !floor || l.is_floor())
                .filter(|l| l.holds(x) && (l.y as f32) >= y0 && (l.y as f32) <= y1)
                .min_by_key(|l| l.y);
            if let Some(l) = land {
                self.feet.1 = l.y as f32;
                self.under = self.window_under(w);
                self.set(Mode::Landing {
                    until: self.clock + LANDING,
                });
                return;
            }
            // Past the lowest floor, or beside every floor: onto the nearest one.
            let below = w
                .ledges
                .iter()
                .any(|l| (!floor || l.is_floor()) && l.holds(x) && l.y as f32 > y1);
            if !below {
                if let Some(f) = perch::floor_under(w.ledges, x) {
                    self.feet = (x.clamp(f.x0, f.x1 - 1) as f32, f.y as f32);
                    self.under = None;
                    self.set(Mode::Landing {
                        until: self.clock + LANDING,
                    });
                    return;
                }
            }
        }
        self.feet.1 = y1;
        self.mode = Mode::Air { vy, floor };
    }
}

fn first(pack: &Pack, state: &str) -> u32 {
    pack.state(state).frames[0]
}

fn last(pack: &Pack, state: &str) -> u32 {
    *pack.state(state).frames.last().expect("a state has frames")
}

fn timed(pack: &Pack, state: &str, t: f32) -> u32 {
    let s = pack.state(state);
    let n = (t.max(0.0) * s.fps.unwrap_or(4.0)) as usize;
    if s.looping {
        s.frames[n % s.frames.len()]
    } else {
        s.frames[n.min(s.frames.len() - 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perch::tests::{layout, win, B, SCREEN};
    use crate::perch::{ledges, Rect};
    use std::path::Path;

    fn captain() -> Pack {
        Pack::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skins/companions/captain"))
            .unwrap()
    }

    struct Sim {
        brain: Brain,
        layout: LayoutInfo,
        work: Vec<Rect>,
        playing: bool,
        /// What the OS says about his window, as the platform would.
        seen: Seen,
    }

    impl Sim {
        fn new(layout: LayoutInfo) -> Sim {
            Sim {
                brain: Brain::new(7),
                layout,
                work: vec![SCREEN],
                playing: false,
                seen: Seen::default(),
            }
        }

        fn run(&mut self, secs: f32, beat_every: Option<f32>) {
            let dt = 1.0 / 30.0;
            let mut t = 0.0;
            let mut next_beat = 0.0;
            while t < secs {
                let ls = ledges(&self.layout, &self.work, B);
                let beat = beat_every.is_some_and(|p| {
                    let hit = t >= next_beat;
                    if hit {
                        next_beat += p;
                    }
                    hit
                });
                let w = World {
                    layout: &self.layout,
                    ledges: &ls,
                    playing: self.playing,
                    beat,
                    zoom: 1.0,
                    walk_px_per_sec: 24.0,
                    seen: self.seen,
                };
                self.brain.step(dt, &w);
                t += dt;
            }
        }
    }

    fn one_window() -> LayoutInfo {
        layout(vec![win("main", 500, 400, 275, 116)])
    }

    #[test]
    fn he_appears_idle_on_main() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        assert_eq!(s.brain.state(), "idle");
        assert_eq!(s.brain.feet(), (583, 400));
    }

    #[test]
    fn now_and_then_he_walks_and_stays_on_his_ledge() {
        let mut s = Sim::new(one_window());
        let mut walked = false;
        for _ in 0..60 {
            s.run(1.0, None);
            walked |= s.brain.state() == "walk";
            let (x, y) = s.brain.feet();
            assert_eq!(y, 400, "never off the edge");
            assert!((500..775).contains(&x), "never past the ends: {x}");
            if s.brain.state() == "sleep" {
                break;
            }
        }
        assert!(walked, "a walk within the first half minute");
    }

    #[test]
    fn walking_left_mirrors_him() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.brain.feet.0 = 760.0;
        s.brain.set(Mode::Idle { until: 0.0 });
        s.run(0.1, None);
        assert_eq!(s.brain.state(), "walk");
        assert!(
            s.brain.pose(&captain()).flip,
            "the frames face right; left is a mirror"
        );
    }

    #[test]
    fn dancing_and_asleep_he_is_shown_as_drawn_so_words_read_the_right_way_round() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.brain.left = true;
        s.brain.set(Mode::Idle { until: 1e9 });
        assert!(
            s.brain.pose(&captain()).flip,
            "standing, he faces the way he went"
        );
        s.brain.set(Mode::Dance);
        let dancing = s.brain.pose(&captain());
        assert_eq!(dancing.state, "dance");
        assert!(
            !dancing.flip,
            "a shout in a speech bubble is not spelt backwards"
        );
        s.brain.steps += 1;
        assert!(
            !s.brain.pose(&captain()).flip,
            "nor on the next step, so he does not turn on each one"
        );
        s.brain.set(Mode::Sleep);
        assert!(!s.brain.pose(&captain()).flip, "nor a sleeper's z's");
    }

    #[test]
    fn he_sleeps_when_nothing_plays_and_music_wakes_him() {
        let mut s = Sim::new(one_window());
        s.run(SLEEP_AFTER + 20.0, None);
        assert_eq!(s.brain.state(), "sleep");
        s.playing = true;
        s.run(0.1, Some(0.5));
        assert_eq!(s.brain.state(), "dance");
    }

    #[test]
    fn he_dances_a_frame_per_beat_and_keeps_dancing_when_the_beats_stop() {
        let pack = captain();
        let mut s = Sim::new(one_window());
        s.playing = true;
        s.run(0.2, Some(0.5));
        let a = s.brain.pose(&pack);
        s.run(0.5, Some(0.5));
        let b = s.brain.pose(&pack);
        assert_eq!((a.state, b.state), ("dance", "dance"));
        assert_ne!(a.cell, b.cell, "the next beat, the next frame");
        // A long break: he keeps dancing, a frame at a time, never idle.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..16 {
            s.run(0.5, None);
            let p = s.brain.pose(&pack);
            assert_eq!(p.state, "dance", "music on: dancing (#222)");
            seen.insert(p.cell);
        }
        assert!(seen.len() > 1, "and not frozen on one pose: {seen:?}");
        s.playing = false;
        s.run(0.1, None);
        assert_eq!(s.brain.state(), "idle", "music off: he stands");
    }

    #[test]
    fn a_song_with_no_beat_to_find_still_gets_him_dancing() {
        let mut s = Sim::new(one_window());
        s.playing = true;
        s.run(11.0, None);
        assert_eq!(s.brain.pose(&captain()).state, "dance");
        let expect = 11.0 / GROOVE;
        let got = s.brain.steps as f32;
        assert!(
            (got - expect).abs() <= 1.5,
            "about {expect} steps, took {got}"
        );
    }

    #[test]
    fn steady_beats_are_one_step_each_never_two() {
        let mut s = Sim::new(one_window());
        s.playing = true;
        // A beat every 0.5 s, some a tick late, for 10 s.
        s.run(10.0, Some(0.5));
        let got = s.brain.steps;
        assert!((19..=21).contains(&got), "20 beats, {got} steps");
    }

    #[test]
    fn in_a_break_he_keeps_the_last_songs_groove() {
        let mut s = Sim::new(one_window());
        s.playing = true;
        s.run(4.0, Some(0.7));
        assert!(
            (s.brain.groove() - 0.7).abs() < 0.05,
            "{}",
            s.brain.groove()
        );
        let before = s.brain.steps;
        s.run(7.0, None);
        let taken = s.brain.steps - before;
        assert!((9..=11).contains(&taken), "7 s at 0.7 s: {taken} steps");
    }

    #[test]
    fn a_detector_that_hears_every_other_kick_gives_him_the_half_time_back() {
        let mut s = Sim::new(one_window());
        s.playing = true;
        s.run(12.0, Some(1.4));
        assert!(
            (s.brain.groove() - 0.7).abs() < 0.05,
            "{}",
            s.brain.groove()
        );
        // A hi-hat at 0.2 s is doubled up to a step he can take.
        let mut t = Sim::new(one_window());
        t.playing = true;
        t.run(3.0, Some(0.21));
        assert!(t.brain.groove() >= STEP_RANGE.0, "{}", t.brain.groove());
    }

    #[test]
    fn a_nudge_under_him_is_a_hop_and_he_lands_back_on_it() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.layout = layout(vec![win("main", 540, 400, 275, 116)]);
        s.run(0.05, None);
        assert_eq!(s.brain.state(), "startle");
        assert!(s.brain.feet().1 < 400, "the hop goes up first");
        s.run(1.0, None);
        assert_eq!(s.brain.feet(), (583, 400), "Main is still under him");
        s.run(LANDING + 0.1, None);
        assert_eq!(s.brain.state(), "idle");
    }

    #[test]
    fn a_window_pulled_out_from_under_him_drops_him_to_the_floor() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.layout = layout(vec![win("main", 1200, 400, 275, 116)]);
        s.run(2.0, None);
        assert_eq!(
            s.brain.feet(),
            (583, 1032),
            "down past everything to the floor"
        );
    }

    #[test]
    fn shading_main_under_him_is_a_jolt_and_he_lands_back_on_it() {
        let lower = win("library", 400, 700, 600, 300);
        let mut s = Sim::new(layout(vec![win("main", 500, 400, 275, 116), lower.clone()]));
        s.run(0.1, None);
        let mut main = win("main", 500, 400, 275, 116);
        main.shaded = true;
        main.h = 14;
        s.layout = layout(vec![main, lower]);
        s.run(0.05, None);
        assert_eq!(
            s.brain.state(),
            "startle",
            "shading Main under him is a jolt"
        );
        s.run(2.0, None);
        assert_eq!(
            s.brain.feet().1,
            400,
            "the hop lands him back on Main's top, which stayed"
        );
    }

    #[test]
    fn a_hidden_window_drops_him_onto_the_next_ledge_down() {
        // The EQ switched off, the library to the tray: hidden, not
        // minimised, which only the OS can tell apart (D166).
        let lower = win("library", 400, 700, 600, 300);
        let mut s = Sim::new(layout(vec![win("main", 500, 400, 275, 116), lower.clone()]));
        s.run(0.1, None);
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main, lower]);
        s.run(2.0, None);
        assert_eq!(s.brain.feet().1, 700, "onto the library's top");
    }

    /// Main at (500, 400) and the library below it, whose top he falls past
    /// on the way to the floor.
    fn main_over_the_library() -> LayoutInfo {
        layout(vec![
            win("main", 500, 400, 275, 116),
            win("library", 400, 700, 600, 300),
        ])
    }

    #[test]
    fn the_os_is_asked_about_the_spot_just_under_his_feet() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        let spot = s.brain.perch().expect("on Main");
        assert_eq!(spot.id, "main");
        assert_eq!(spot.rect, (500, 400, 275, 116));
        assert_eq!(spot.at, (583, 400 + UNDERFOOT));
    }

    #[test]
    fn covered_past_the_debounce_he_falls_to_the_floor() {
        let mut s = Sim::new(main_over_the_library());
        s.run(0.1, None);
        s.seen.covered = true;
        s.run(COVERED_FOR - 0.1, None);
        assert_eq!(s.brain.state(), "idle", "not yet: it may be passing");
        assert_eq!(s.brain.feet(), (583, 400));
        s.run(0.2, None);
        assert_eq!(s.brain.state(), "startle", "standing on nothing");
        assert!(s.brain.perch().is_none(), "nothing to ask about in the air");
        s.seen.covered = false;
        s.run(2.0, None);
        assert_eq!(
            s.brain.feet(),
            (583, 1032),
            "straight past the library's top to the floor"
        );
        s.run(LANDING + 0.1, None);
        assert_eq!(s.brain.state(), "idle");
    }

    #[test]
    fn covered_briefly_he_stays() {
        let mut s = Sim::new(main_over_the_library());
        s.run(0.1, None);
        for _ in 0..6 {
            s.seen.covered = true;
            s.run(COVERED_FOR - 0.15, None);
            s.seen.covered = false;
            s.run(0.1, None);
        }
        assert_ne!(s.brain.state(), "startle", "a window dragged across him");
        assert_eq!(s.brain.feet().1, 400, "still on Main");
    }

    #[test]
    fn covered_on_the_floor_in_the_air_or_in_the_hand_is_nothing() {
        // No room on Main at the top of the screen: he is on the floor.
        let mut s = Sim::new(layout(vec![win("main", 0, 0, 275, 116)]));
        s.run(0.1, None);
        assert_eq!(s.brain.feet(), (137, 1032));
        assert!(s.brain.perch().is_none(), "the floor is no window");
        s.seen.covered = true;
        s.run(1.0, None);
        assert_ne!(s.brain.state(), "startle");
        assert_eq!(s.brain.feet(), (137, 1032));

        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.brain.hand(Hand::Down(583, 370), HANG);
        s.brain.hand(Hand::Move(900, 200), HANG);
        s.seen.covered = true;
        s.run(1.0, None);
        assert_eq!(s.brain.state(), "carry", "held, he is not standing");
        assert_eq!(s.brain.feet(), (900, 260));
    }

    #[test]
    fn his_window_minimised_he_goes_with_it_and_comes_back_where_he_stood() {
        let mut s = Sim::new(main_over_the_library());
        s.run(0.1, None);
        s.brain.feet.0 = 700.0;
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main, win("library", 400, 700, 600, 300)]);
        s.seen.minimised = true;
        s.run(2.0, None);
        assert_eq!(s.brain.state(), "minimised", "not dropped to the library");
        assert!(s.brain.is_placed() && !s.brain.is_shown());
        assert_eq!(s.brain.perch().map(|p| p.id), Some("main"), "still his");

        // Restored: the OS says so first, the layout up to 50 ms later.
        s.seen.minimised = false;
        s.run(0.05, None);
        assert!(!s.brain.is_shown(), "waiting for the layout to agree");
        s.layout = main_over_the_library();
        s.run(0.1, None);
        assert!(s.brain.is_shown());
        assert_eq!(s.brain.state(), "idle");
        assert_eq!(s.brain.feet(), (700, 400), "where he stood");
    }

    #[test]
    fn restored_narrower_he_is_kept_on_it() {
        let lib = win("library", 100, 300, 900, 400);
        let mut s = Sim::new(layout(vec![lib.clone()]));
        s.run(0.1, None);
        s.brain.feet.0 = 950.0;
        let mut down = lib.clone();
        down.visible = false;
        s.layout = layout(vec![down]);
        s.seen.minimised = true;
        s.run(0.5, None);
        assert_eq!(s.brain.state(), "minimised");
        s.seen.minimised = false;
        s.layout = layout(vec![win("library", 100, 300, 600, 400)]);
        s.run(0.1, None);
        assert_eq!(s.brain.feet(), (699, 300), "at its end, not past it");
    }

    #[test]
    fn with_every_window_minimised_he_still_comes_back_where_he_stood() {
        // Main's minimise takes the group (D86) and nothing else is showing:
        // he is minimised with his window, not sent off to appear afresh.
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.brain.feet.0 = 720.0;
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main]);
        s.seen.minimised = true;
        s.run(1.0, None);
        assert!(s.brain.is_placed() && !s.brain.is_shown());
        s.seen.minimised = false;
        s.layout = one_window();
        s.run(0.1, None);
        assert_eq!(s.brain.feet(), (720, 400), "not 30% along, afresh");
    }

    #[test]
    fn every_window_hidden_still_sends_him_off() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main]);
        s.run(0.1, None);
        assert!(!s.brain.is_placed(), "he goes with the player (D154)");
        s.layout = one_window();
        s.run(0.1, None);
        assert_eq!(s.brain.feet(), (583, 400), "and appears afresh");
    }

    #[test]
    fn his_window_gone_while_minimised_he_appears_afresh() {
        let mut s = Sim::new(main_over_the_library());
        s.run(0.1, None);
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main, win("library", 400, 700, 600, 300)]);
        s.seen.minimised = true;
        s.run(0.5, None);
        s.seen.minimised = false;
        s.layout = layout(vec![win("library", 400, 700, 600, 300)]);
        s.run(0.1, None);
        assert!(s.brain.is_shown());
        assert_eq!(s.brain.feet(), (580, 700), "30% along the library");
    }

    #[test]
    fn on_the_floor_a_minimise_changes_nothing() {
        // Main at the top of the screen with no room above it, so he is on
        // the floor; the library shows too, so the player is still there.
        let mut s = Sim::new(layout(vec![
            win("main", 0, 0, 275, 116),
            win("library", 1000, 0, 600, 300),
        ]));
        s.run(0.1, None);
        assert_eq!(s.brain.feet(), (137, 1032));
        let mut main = win("main", 0, 0, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main, win("library", 1000, 0, 600, 300)]);
        s.seen.minimised = true;
        s.run(1.0, None);
        assert!(s.brain.is_shown(), "on the floor he is his own");
        assert_ne!(s.brain.state(), "startle");
        assert_eq!(s.brain.feet(), (137, 1032));
    }

    #[test]
    fn on_the_floor_he_stays_out_front_with_every_window_down_until_put_back_on_one() {
        let mut s = Sim::new(layout(vec![win("main", 0, 0, 275, 116)]));
        s.run(0.1, None);
        assert_eq!(s.brain.feet(), (137, 1032), "on the floor");
        let mut main = win("main", 0, 0, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main]);
        s.run(2.0, None);
        assert!(s.brain.is_shown(), "detached, he stays out front (D168)");
        assert_eq!(s.brain.feet().1, 1032, "and on the floor");
        // The player comes back: he stays where he is, on the floor.
        s.layout = layout(vec![win("main", 0, 0, 275, 116)]);
        s.run(0.5, None);
        assert_eq!(s.brain.feet().1, 1032, "not whisked back onto a window");
    }

    #[test]
    fn falling_to_the_floor_as_every_window_goes_he_lands_and_stays() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        // Main closes under him: he jumps and falls, with nothing left showing.
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main]);
        s.brain.set(Mode::Air {
            vy: 0.0,
            floor: true,
        });
        s.brain.under = None;
        s.run(2.0, None);
        assert!(s.brain.is_shown(), "he lands, and stays");
        assert_eq!(s.brain.feet().1, 1032);
    }

    #[test]
    fn standing_on_the_floor_is_not_a_jolt() {
        let mut s = Sim::new(layout(vec![win("main", 0, 0, 275, 116)]));
        s.run(0.1, None);
        assert_eq!(s.brain.feet().1, 1032);
        s.layout = layout(vec![win("main", 10, 0, 275, 116)]);
        s.run(0.2, None);
        assert_ne!(
            s.brain.state(),
            "startle",
            "Main moving does not shake the floor"
        );
    }

    /// Where the hand is while carrying him at 1x: feet this far below it.
    const HANG: f32 = 64.0 - SCRUFF;

    #[test]
    fn a_click_is_a_pet_and_then_he_stands() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.brain.hand(Hand::Down(583, 370), HANG);
        s.brain.hand(Hand::Up(584, 371), HANG);
        s.run(0.1, None);
        assert_eq!(s.brain.state(), "pet");
        assert_eq!(s.brain.pose(&captain()).state, "pet");
        s.run(PET_FOR, None);
        assert_eq!(s.brain.state(), "idle");
        assert_eq!(s.brain.feet(), (583, 400), "a pet does not move him");
    }

    #[test]
    fn a_pet_wakes_him_and_he_stays_up_a_while() {
        let mut s = Sim::new(one_window());
        s.run(SLEEP_AFTER + 20.0, None);
        assert_eq!(s.brain.state(), "sleep");
        let (x, y) = s.brain.feet();
        s.brain.hand(Hand::Down(x, y - 30), HANG);
        s.brain.hand(Hand::Up(x, y - 30), HANG);
        s.run(PET_FOR + 1.0, None);
        assert_ne!(s.brain.state(), "sleep", "not straight back to sleep");
        s.run(SLEEP_AFTER, None);
        assert_eq!(
            s.brain.state(),
            "sleep",
            "until the quiet has been long enough again"
        );
    }

    #[test]
    fn a_jolt_wakes_him_and_he_stays_up_a_while() {
        let lower = win("library", 400, 700, 600, 300);
        let mut s = Sim::new(layout(vec![win("main", 500, 400, 275, 116), lower.clone()]));
        s.run(SLEEP_AFTER + 20.0, None);
        assert_eq!(s.brain.state(), "sleep");
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main, lower]);
        s.run(3.0, None);
        assert_eq!(s.brain.feet().1, 700);
        assert_ne!(s.brain.state(), "sleep", "the fall woke him");
    }

    #[test]
    fn a_drag_carries_him_by_the_scruff() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.brain.hand(Hand::Down(583, 370), HANG);
        s.brain.hand(Hand::Move(600, 360), HANG);
        assert_eq!(s.brain.state(), "carry");
        s.brain.hand(Hand::Move(900, 200), HANG);
        s.run(0.5, None);
        assert_eq!(s.brain.feet(), (900, 260), "hanging under the hand");
        assert_eq!(s.brain.pose(&captain()).state, "carry");
        assert_eq!(s.brain.state(), "carry", "no gravity while held");
    }

    #[test]
    fn let_go_he_falls_to_the_ledge_below_where_he_was_dropped() {
        let lib = win("library", 800, 700, 600, 300);
        let mut s = Sim::new(layout(vec![win("main", 500, 400, 275, 116), lib]));
        s.run(0.1, None);
        s.brain.hand(Hand::Down(583, 370), HANG);
        s.brain.hand(Hand::Move(1000, 300), HANG);
        s.brain.hand(Hand::Up(1000, 300), HANG);
        s.run(2.0, None);
        assert_eq!(
            s.brain.feet(),
            (1000, 700),
            "onto the library, where he was dropped"
        );
        s.brain.hand(Hand::Down(1000, 670), HANG);
        s.brain.hand(Hand::Move(1700, 100), HANG);
        s.brain.hand(Hand::Up(1700, 100), HANG);
        s.run(2.0, None);
        assert_eq!(s.brain.feet(), (1700, 1032), "over nothing: the floor");
    }

    #[test]
    fn a_wobble_smaller_than_a_drag_is_still_a_pet() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.brain.hand(Hand::Down(583, 370), HANG);
        s.brain.hand(Hand::Move(583 + DRAG_PX, 370 - DRAG_PX), HANG);
        s.brain.hand(Hand::Up(583 + DRAG_PX, 370), HANG);
        assert_eq!(s.brain.state(), "pet");
    }

    #[test]
    fn a_carry_the_mouse_is_taken_from_drops_him() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.brain.hand(Hand::Down(583, 370), HANG);
        s.brain.hand(Hand::Move(583, 300), HANG);
        s.brain.hand(Hand::Cancel, HANG);
        s.run(2.0, None);
        assert_eq!(s.brain.feet(), (583, 400), "straight down onto Main again");
    }

    #[test]
    fn a_click_while_he_falls_does_nothing_but_a_drag_catches_him() {
        let mut s = Sim::new(one_window());
        s.run(0.1, None);
        s.layout = layout(vec![win("main", 1200, 400, 275, 116)]);
        s.run(0.2, None);
        assert_eq!(s.brain.state(), "startle");
        s.brain.hand(Hand::Down(583, 500), HANG);
        s.brain.hand(Hand::Up(583, 500), HANG);
        assert_eq!(s.brain.state(), "startle", "no pet in mid-air");
        s.brain.hand(Hand::Down(583, 500), HANG);
        s.brain.hand(Hand::Move(583, 520), HANG);
        assert_eq!(s.brain.state(), "carry", "caught");
    }
}
