//! Not Windows: compiles, draws nothing. The player is Windows-first, and so is
//! he; a macOS or Linux companion is #187's question, not this file's.

use crate::brain::{Seen, Spot};
use crate::perch::Rect;
use crate::sprite::Bgra;

pub fn init() {}

pub fn only_one() -> bool {
    true
}

pub fn pipe_server(_pipe: &std::fs::File) -> Option<u32> {
    None
}

pub struct Surface;

impl Surface {
    pub fn new() -> Result<Surface, String> {
        Ok(Surface)
    }

    pub fn present(&mut self, _img: &Bgra, _x: i32, _y: i32) -> Result<(), String> {
        Ok(())
    }

    pub fn hide(&mut self) {}

    pub fn hands(&mut self) -> Vec<crate::brain::Hand> {
        Vec::new()
    }

    pub fn player(&mut self, _pid: u32) {}

    /// Nothing to see: he behaves as he did before D166.
    pub fn look(&mut self, _spot: &Spot) -> Seen {
        Seen::default()
    }
}

pub fn work_areas() -> Vec<Rect> {
    vec![Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1040,
    }]
}

pub fn run(interval_ms: u32, mut tick: impl FnMut()) {
    loop {
        tick();
        std::thread::sleep(std::time::Duration::from_millis(interval_ms as u64));
    }
}

/// Nothing to hear on a platform with no player to ask.
pub struct Leave;

impl Leave {
    pub fn new() -> Leave {
        Leave
    }

    pub fn asked(&self) -> bool {
        false
    }
}
