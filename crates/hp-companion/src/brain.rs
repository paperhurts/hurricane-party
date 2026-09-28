//! What he does, moment to moment. Pure: time, the ledges, the window he is
//! standing on, whether music is playing and whether a beat just landed go in;
//! his feet, his facing and his state come out. The pack turns that into a
//! frame (`pose`). The seven states are the app's, fixed (`purricane.md`).
//!
//! - **idle** by default, with a walk now and then to somewhere else on the
//!   ledge he is on, across a seam if two windows sit side by side.
//! - **dance** while music plays: a frame per beat from the viz stream, and
//!   idle when the beats go quiet.
//! - **sleep** once nothing has played for a while; music wakes him.
//! - **startle**: when the window under him moves, shades, hides or goes, or
//!   the stretch he stands on stops being a ledge, he jumps and falls to the
//!   next ledge below, the floor above the taskbar at the bottom of it all.
//! - **pet**: a click on him. He leans into it for a moment, awake again.
//! - **carry**: a drag. He hangs from the hand by his scruff, kicking, and
//!   when he is let go he falls to the ledge below where he was dropped.
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
/// Beats this stale and the dance goes back to standing.
pub const BEAT_STALE: f32 = 1.5;
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Idle { until: f32 },
    Walk { to: f32 },
    Dance,
    Sleep,
    Air { vy: f32 },
    Landing { until: f32 },
    Pet { until: f32 },
    Carry,
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
}

pub struct Brain {
    /// His feet: x of the contact point, y of the edge under it.
    feet: (f32, f32),
    left: bool,
    mode: Mode,
    since: f32,
    clock: f32,
    quiet_since: Option<f32>,
    beats: u32,
    last_beat: f32,
    /// `None` on a floor or in the air.
    under: Option<Under>,
    placed: bool,
    grab: Option<Grab>,
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
            beats: 0,
            last_beat: f32::NEG_INFINITY,
            under: None,
            placed: false,
            grab: None,
            rng: seed | 1,
        }
    }

    /// The player's windows are gone: next time they show, he appears afresh.
    pub fn leave(&mut self) {
        self.placed = false;
        self.under = None;
        self.grab = None;
    }

    /// The pointer, as his window heard it. `hang` is how far below the hand
    /// his feet are while he is carried, in screen pixels.
    pub fn hand(&mut self, h: Hand, hang: f32) {
        if !self.placed {
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
                Some(Grab { carrying: true, .. }) => self.set(Mode::Air { vy: 0.0 }),
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
                    self.set(Mode::Air { vy: 0.0 });
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
        }
    }

    pub fn feet(&self) -> (i32, i32) {
        (self.feet.0.round() as i32, self.feet.1.round() as i32)
    }

    pub fn step(&mut self, dt: f32, w: &World) {
        self.clock += dt;
        if w.beat {
            self.beats = self.beats.wrapping_add(1);
            self.last_beat = self.clock;
        }
        if w.playing {
            self.quiet_since = None;
        } else if self.quiet_since.is_none() {
            self.quiet_since = Some(self.clock);
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
            self.set(Mode::Air { vy: -HOP * w.zoom });
            self.under = None;
            self.wake();
        }

        match self.mode {
            Mode::Air { vy } => self.fall(dt, vy, w),
            Mode::Carry => {}
            Mode::Landing { until } | Mode::Pet { until } => {
                if self.clock >= until {
                    let rest = self.rest();
                    self.set(Mode::Idle { until: rest });
                }
            }
            _ if w.playing => {
                if self.mode != Mode::Dance {
                    self.set(Mode::Dance);
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
            Mode::Dance if self.clock - self.last_beat <= BEAT_STALE => {
                let d = &pack.state("dance").frames;
                ("dance", d[self.beats as usize % d.len()])
            }
            Mode::Dance | Mode::Idle { .. } => ("idle", timed(pack, "idle", self.clock)),
            Mode::Walk { .. } => ("walk", timed(pack, "walk", self.clock - self.since)),
            Mode::Sleep => ("sleep", timed(pack, "sleep", self.clock)),
        };
        Pose {
            state,
            cell,
            flip: self.left,
            feet: self.feet(),
        }
    }

    fn set(&mut self, m: Mode) {
        self.mode = m;
        self.since = self.clock;
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

    fn fall(&mut self, dt: f32, vy: f32, w: &World) {
        let vy = vy + GRAVITY * w.zoom * dt;
        let (x, y0) = (self.feet.0.round() as i32, self.feet.1);
        let y1 = y0 + vy * dt;
        if vy > 0.0 {
            // The first ledge his feet pass on the way down.
            let land = w
                .ledges
                .iter()
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
            let below = w.ledges.iter().any(|l| l.holds(x) && l.y as f32 > y1);
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
        self.mode = Mode::Air { vy };
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
    }

    impl Sim {
        fn new(layout: LayoutInfo) -> Sim {
            Sim {
                brain: Brain::new(7),
                layout,
                work: vec![SCREEN],
                playing: false,
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
    fn he_sleeps_when_nothing_plays_and_music_wakes_him() {
        let mut s = Sim::new(one_window());
        s.run(SLEEP_AFTER + 20.0, None);
        assert_eq!(s.brain.state(), "sleep");
        s.playing = true;
        s.run(0.1, Some(0.5));
        assert_eq!(s.brain.state(), "dance");
    }

    #[test]
    fn he_dances_a_frame_per_beat_and_stands_when_the_beats_stop() {
        let pack = captain();
        let mut s = Sim::new(one_window());
        s.playing = true;
        s.run(0.2, Some(0.5));
        let a = s.brain.pose(&pack);
        s.run(0.5, Some(0.5));
        let b = s.brain.pose(&pack);
        assert_eq!((a.state, b.state), ("dance", "dance"));
        assert_ne!(a.cell, b.cell, "the next beat, the next frame");
        s.run(BEAT_STALE + 0.5, None);
        assert_eq!(s.brain.pose(&pack).state, "idle", "quiet passage: standing");
        s.playing = false;
        s.run(0.1, None);
        assert_eq!(s.brain.state(), "idle");
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
    fn a_minimised_window_drops_him_onto_the_next_ledge_down() {
        let lower = win("library", 400, 700, 600, 300);
        let mut s = Sim::new(layout(vec![win("main", 500, 400, 275, 116), lower.clone()]));
        s.run(0.1, None);
        let mut main = win("main", 500, 400, 275, 116);
        main.visible = false;
        s.layout = layout(vec![main, lower]);
        s.run(2.0, None);
        assert_eq!(s.brain.feet().1, 700, "onto the library's top");
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
