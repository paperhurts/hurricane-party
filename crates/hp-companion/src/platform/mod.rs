//! Everything that talks to the OS, and the only place `cfg(windows)` appears
//! (the same rule as the player's `src-tauri/src/platform/`).
//!
//! `Surface` is his window: transparent, always on top, never activated, and
//! click-through (#192). `run` is the message loop, calling back on a timer.
//! `work_areas` is each monitor minus its taskbar, in physical pixels.

#[cfg(windows)]
mod windows_impl;
#[cfg(windows)]
pub use windows_impl::{init, only_one, run, work_areas, Leave, Surface};

#[cfg(not(windows))]
mod stub;
#[cfg(not(windows))]
pub use stub::{init, only_one, run, work_areas, Leave, Surface};
