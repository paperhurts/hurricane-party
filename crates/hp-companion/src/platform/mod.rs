//! Everything that talks to the OS, and the only place `cfg(windows)` appears
//! (the same rule as the player's `src-tauri/src/platform/`).
//!
//! `Surface` is his window: transparent, always on top, never activated, and
//! clickable only where he is (#192, D156). It also asks the OS what the
//! layout cannot about the player's window he is on: whether it is minimised,
//! and whether another window is in front of it under his feet (D166).
//! `pipe_server` is the player's process, from its end of the control pipe.
//! `run` is the message loop, calling back on a timer. `work_areas` is each
//! monitor minus its taskbar, in physical pixels.

#[cfg(windows)]
mod windows_impl;
#[cfg(windows)]
pub use windows_impl::{init, only_one, pipe_server, run, work_areas, Leave, Surface};

#[cfg(not(windows))]
mod stub;
#[cfg(not(windows))]
pub use stub::{init, only_one, pipe_server, run, work_areas, Leave, Surface};
