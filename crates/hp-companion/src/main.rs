//! Cap'n Capy: a desktop companion for hurricane-party (#192, v1.1; D147,
//! D153). A separate process on the public control pipe (D22), so a crash in
//! him never stops the music, and the protocol is proven by its first real
//! client.
//!
//! This first cut stands him on the player's windows and lets him idle: he
//! finds a top edge with room, rides it when the window moves, drops to the
//! floor when there is nowhere else, and goes when the player's windows do.
//! Walking, the beat, sleep, petting and carrying come after.
//!
//!     hp-companion [--pack <folder>]
//!
//! Without `--pack` he looks for `companions/captain` beside the exe, and in a
//! debug build for the repo's own `skins/companions/captain`.

mod link;
mod pack;
mod perch;
mod platform;
mod sprite;

use hp_control::LayoutInfo;
use link::Msg;
use pack::Pack;
use perch::{Body, Rect, Spot};
use sprite::Bgra;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// About 30 checks a second: fast enough to ride a dragged window, and the
/// frame only changes at the pack's own rate.
const TICK_MS: u32 = 33;
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
    layout: Option<LayoutInfo>,
    work: Vec<Rect>,
    work_read: Instant,
    spot: Option<Spot>,
    born: Instant,
    /// Rendered frames by (cell, scale).
    frames: HashMap<(u32, u32), Bgra>,
    /// What is on screen now: (x, y, cell, scale). Nothing is redrawn until it changes.
    drawn: Option<(i32, i32, u32, u32)>,
    complained: bool,
}

impl Captain {
    fn new(pack: Pack, rx: Receiver<Msg>) -> Captain {
        Captain {
            pack,
            rx,
            layout: None,
            work: platform::work_areas(),
            work_read: Instant::now(),
            spot: None,
            born: Instant::now(),
            frames: HashMap::new(),
            drawn: None,
            complained: false,
        }
    }

    fn tick(&mut self, surface: &mut platform::Surface) {
        let mut moved = false;
        while let Ok(m) = self.rx.try_recv() {
            self.layout = match m {
                Msg::Layout(l) => Some(l),
                Msg::Gone => None,
            };
            moved = true;
        }
        if moved || self.work_read.elapsed() >= WORK_AREAS_EVERY {
            self.work = platform::work_areas();
            self.work_read = Instant::now();
        }

        let Some(layout) = &self.layout else {
            return self.away(surface);
        };
        let scale = perch::zoom(layout);
        self.spot = perch::choose(layout, &self.work, self.body(scale), self.spot.as_ref());
        let Some(spot) = &self.spot else {
            return self.away(surface);
        };

        let cell = self.idle_cell();
        let now = (spot.x, spot.y, cell, scale);
        if self.drawn == Some(now) {
            return;
        }
        let pack = &self.pack;
        let img = self
            .frames
            .entry((cell, scale))
            .or_insert_with(|| sprite::render(pack.cell(cell), scale, false));
        match surface.present(img, spot.x, spot.y) {
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

    /// No player, or none of its windows showing: he goes with it.
    fn away(&mut self, surface: &mut platform::Surface) {
        self.spot = None;
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

    fn idle_cell(&self) -> u32 {
        let idle = self.pack.state("idle");
        let fps = idle.fps.unwrap_or(4.0);
        let n = (self.born.elapsed().as_secs_f32() * fps) as usize;
        idle.frames[n % idle.frames.len()]
    }
}
