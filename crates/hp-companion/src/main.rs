//! Cap'n Capy: a desktop companion for hurricane-party (#192, v1.1; D147,
//! D153). A separate process on the public control pipe (D22), so a crash in
//! him never stops the music, and the protocol is proven by its first real
//! client.
//!
//! He lives on the player's windows (D154, D155): he stands on a top edge with
//! room, walks along it now and then, dances on the beat while music plays,
//! sleeps when nothing has played for a while, and jumps and falls to the next
//! ledge down when the window under him moves, shades or goes. He goes when
//! the player's windows do. Click him and he leans into the pet; drag him and
//! he hangs by his scruff, kicking, until he is dropped (D156).
//!
//!     hp-companion [--pack <folder>]
//!
//! Without `--pack` he looks for `companions/captain` beside the exe, and in a
//! debug build for the repo's own `skins/companions/captain`.

mod brain;
mod link;
mod pack;
mod perch;
mod platform;
mod sprite;

use brain::{Brain, World};
use hp_control::LayoutInfo;
use link::Msg;
use pack::Pack;
use perch::{Body, Rect};
use sprite::Bgra;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// About 30 steps a second: smooth enough for a walk and a fall; the frame
/// only changes at the pack's own rate, or on the beat.
const TICK_MS: u32 = 33;
/// A stalled tick (the machine asleep, a debugger) is not a long fall.
const MAX_STEP: f32 = 0.1;
/// Monitors change rarely (D55); their work areas are re-read this often, and
/// whenever the layout changes.
const WORK_AREAS_EVERY: Duration = Duration::from_secs(2);

fn main() {
    let dir = pack_dir().unwrap_or_else(|e| fail(&e));
    let pack = Pack::load(&dir).unwrap_or_else(|e| fail(&e));
    eprintln!("hp-companion: {} from {}", pack.name, dir.display());

    platform::init();
    let (tx, rx) = mpsc::channel();
    link::spawn(tx);
    let mut surface = platform::Surface::new().unwrap_or_else(|e| fail(&e));
    let mut captain = Captain::new(pack, rx);
    platform::run(TICK_MS, || captain.tick(&mut surface));
}

fn fail(e: &str) -> ! {
    eprintln!("hp-companion: {e}");
    std::process::exit(1)
}

fn pack_dir() -> Result<PathBuf, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => {}
        ["--pack", dir] => return Ok(PathBuf::from(dir)),
        ["-h"] | ["--help"] => {
            println!("hp-companion [--pack <folder>]");
            std::process::exit(0)
        }
        _ => return Err(format!("unknown arguments {args:?}; try --help")),
    }
    let mut places = Vec::new();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
    {
        places.push(exe_dir.join("companions").join("captain"));
    }
    #[cfg(debug_assertions)]
    places.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skins/companions/captain"));
    places
        .into_iter()
        .find(|d| d.join("companion.json").is_file())
        .ok_or_else(|| "no pack found; pass --pack <folder>".into())
}

struct Captain {
    pack: Pack,
    rx: Receiver<Msg>,
    brain: Brain,
    layout: Option<LayoutInfo>,
    playing: bool,
    work: Vec<Rect>,
    work_read: Instant,
    last_tick: Instant,
    /// Rendered frames by (cell, scale, mirrored).
    frames: HashMap<(u32, u32, bool), Bgra>,
    /// What is on screen now: (x, y, cell, scale, mirrored). Nothing is
    /// redrawn until it changes.
    drawn: Option<(i32, i32, u32, u32, bool)>,
    complained: bool,
    /// `HP_COMPANION_TRACE=1`: print each change of state, for a hand test.
    trace: bool,
    traced: Option<&'static str>,
}

impl Captain {
    fn new(pack: Pack, rx: Receiver<Msg>) -> Captain {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5eed);
        Captain {
            pack,
            rx,
            brain: Brain::new(seed),
            layout: None,
            playing: false,
            work: platform::work_areas(),
            work_read: Instant::now(),
            last_tick: Instant::now(),
            frames: HashMap::new(),
            drawn: None,
            complained: false,
            trace: std::env::var_os("HP_COMPANION_TRACE").is_some_and(|v| v == "1"),
            traced: None,
        }
    }

    fn tick(&mut self, surface: &mut platform::Surface) {
        let dt = self.last_tick.elapsed().as_secs_f32().min(MAX_STEP);
        self.last_tick = Instant::now();

        let (mut moved, mut beat) = (false, false);
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Layout(l) => {
                    self.layout = Some(l);
                    moved = true;
                }
                Msg::Playing(p) => self.playing = p,
                Msg::Beat => beat = true,
                Msg::Gone => {
                    self.layout = None;
                    self.playing = false;
                    moved = true;
                }
            }
        }
        if moved || self.work_read.elapsed() >= WORK_AREAS_EVERY {
            self.work = platform::work_areas();
            self.work_read = Instant::now();
        }

        let Some(layout) = &self.layout else {
            return self.away(surface);
        };
        if perch::shown(layout).next().is_none() {
            return self.away(surface);
        }
        let scale = perch::zoom(layout);
        let body = self.body(scale);
        let ledges = perch::ledges(layout, &self.work, body);
        // Carried, the hand is at his scruff: his feet hang this far below it.
        let hang = body.ay as f32 - brain::SCRUFF * scale as f32;
        for h in surface.hands() {
            self.brain.hand(h, hang);
        }
        self.brain.step(
            dt,
            &World {
                layout,
                ledges: &ledges,
                playing: self.playing,
                beat,
                zoom: scale as f32,
                walk_px_per_sec: self.pack.walk_px_per_sec,
            },
        );
        if !self.brain.is_placed() {
            return self.away(surface);
        }

        let pose = self.brain.pose(&self.pack);
        if self.trace && self.traced != Some(pose.state) {
            eprintln!(
                "hp-companion: {} at {},{}",
                pose.state, pose.feet.0, pose.feet.1
            );
            self.traced = Some(pose.state);
        }
        let (x, y) = (pose.feet.0 - body.ax, pose.feet.1 - body.ay);
        let now = (x, y, pose.cell, scale, pose.flip);
        if self.drawn == Some(now) {
            return;
        }
        let pack = &self.pack;
        let img = self
            .frames
            .entry((pose.cell, scale, pose.flip))
            .or_insert_with(|| sprite::render(pack.cell(pose.cell), scale, pose.flip));
        match surface.present(img, x, y) {
            Ok(()) => {
                self.drawn = Some(now);
                self.complained = false;
            }
            Err(e) if !self.complained => {
                eprintln!("hp-companion: could not draw ({e})");
                self.complained = true;
            }
            Err(_) => {}
        }
    }

    /// No player, or none of its windows showing: he goes with it, and
    /// appears afresh when it is back.
    fn away(&mut self, surface: &mut platform::Surface) {
        self.brain.leave();
        self.drawn = None;
        surface.hide();
    }

    fn body(&self, scale: u32) -> Body {
        let (fw, fh) = self.pack.frame;
        let (ax, ay) = self.pack.anchor;
        Body {
            w: (fw * scale) as i32,
            h: (fh * scale) as i32,
            ax: (ax * scale) as i32,
            ay: ((ay + 1) * scale) as i32,
        }
    }
}
