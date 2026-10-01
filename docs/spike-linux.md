# The Linux spike (#187)

**Status:** under way. The owner's calls are D181: Linux first, under XWayland, and this spike before any porting.

**What it answers:** whether the player on Linux is the same app, with three classic windows that bond, or a reduced one. That is the question v0.0 answered for Windows (D45), asked again of a different window system.

**What it produces:** a findings section at the end of this file, with a measured number or a plain yes or no for each stage, and decision rows from it. Code is a means of measuring. Anything worth keeping is ported in its own PR after the verdict, as `bond.rs` was (D66).

**Timebox:** a weekend's worth of sessions. If it is still fighting on the third day, that is the finding.

---

## Where it runs

1. **A first read in WSL**, on the dev machine: Ubuntu 24.04 through WSLg, which gives X11 through XWayland on Weston and draws each window on the Windows desktop. It is cheap and it is here, but WSLg's compositor is neither GNOME nor KDE, so **a pass in WSL is provisional** and a failure may be WSLg's own.
2. **The verdict on the owner's Linux laptop**: a real desktop, real input (D43), and a session running on the laptop. Note the desktop, its version, Wayland or X11, and the scale.

## How it differs from v0.0

The app exists now. The spike builds **the real app for Linux** on the non-Windows stub (`platform/stub.rs`), started with `GDK_BACKEND=x11` (D181). It adds to `platform/` only what a stage needs in order to be measured, on a branch `spike/linux` that is not merged.

Not in the spike: the sidecars, audio and video formats under WebKitGTK and GStreamer, the pipe as a Unix socket, the companion, and packaging. Each is port work after a go. If WebKitGTK cannot draw the classic windows at all, that is a finding and stops the spike.

## Building it in WSL

- **System packages**, once, as the owner (needs `sudo`): the Tauri v2 prerequisites, `rustup`, and `xdotool` and `x11-utils` for measuring.
- **The repo** is cloned into the WSL home from the Windows checkout (`git clone /mnt/c/dev/hurricane-party`), because building on `/mnt/c` is slow.
- **The frontend** is built on Windows (`pnpm build`) and its `dist/` copied over: Ubuntu's Node is too old for Vite. The app runs from `dist/` (`--features tauri/custom-protocol`), not from a dev server.
- **The sidecars** are empty files named for the Linux target, as CI does on Windows.

## The stages

v0.0's stages, in v0.0's order, with v0.0's pass criteria (`spike-v0.0.md`). Each stage gates the next. What is new on Linux:

**0. Skeleton, and the scale gate.** Windows lies about scale to a process that does not declare DPI awareness (D37). X11 has a lie of its own: it has **one scale for the whole screen**, and XWayland on a fractionally scaled Wayland desktop bitmap-scales X clients. So before trusting a number, log `scale_factor()` and `outer_size()` against the screen's physical size, and check that a position set reads back as set. Record the scale and how it is set (`GDK_SCALE`, the desktop's setting).

**1. The drag loop**, the make-or-break. On X11, a window's position belongs to the window manager. Does it honour a `set_position` on every mousemove, or constrain, animate or ignore it? Measure as D39 does: excess over floor, and latency in frames.

**2. Grouped z-order.** Windows does this with owned windows and a hidden root (D41, D42, D56). X11's nearest is `WM_TRANSIENT_FOR`, which keeps a window above its parent, but window managers treat transients as dialogs: they may centre them, attach them, or drop them from the taskbar. The alternatives are a window type such as `_NET_WM_WINDOW_TYPE_UTILITY`, or raising the group together on focus. Pass as v0.0: click any window and all three come up together, focus lands sensibly, and nothing sticks on top of other apps.

**3 to 5. Bonding, the splitter, breaking bonds.** `bond.rs` is pure and already tested, so these check the real app by hand: bonds form at the threshold, a group moves with its offsets exact, a splitter drag shows no gap or overlap, and v0.0's break cases hold, including that z-order follows a split.

**6. Two displays.** One X scale across monitors means mixed scales likely blur one side. Record what happens, and unplug a display with a group on it (D57).

**7. The rest of the trait's window calls**, new for Linux: topmost (D61, `_NET_WM_STATE_ABOVE`), minimising and restoring as a group (D86, D152), and the cursor position (`XQueryPointer`).

---

## Findings

Not yet run.
