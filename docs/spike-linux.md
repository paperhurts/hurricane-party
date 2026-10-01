# The Linux spike (#187)

**Status:** the first read in WSL is done (stages 0, 1 and 3 pass provisionally, below); the verdict on the laptop is next. The owner's calls are D181: Linux first, under XWayland, and this spike before any porting.

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

### The first read, in WSL (2026-10-01)

Ubuntu 24.04 in WSL2, through WSLg: Weston's window manager, XWayland, one 2560×1080 screen at 96 dpi (scale 1.0). WebKitGTK 2.52.6, GTK 3.24.41. Input by XTEST. The code is on `spike/linux`, with the scripts in `tools/spike-linux/`.

- **It builds as it is.** The app compiles for Linux on the stub, with warnings as errors, and WebKitGTK draws the classic chrome correctly.
- **Stage 0: a provisional pass, after three fixes.**
  1. **GTK3 holds a non-resizable window to at least 200 px tall**, so the classic windows came up 275×200 and overlapped. A plain X window, or a resizable GTK one, gets 275×116. A size request on the window itself fixes it (`platform::hold_size`). Making the windows resizable does not: on Linux, tauri-runtime-wry starts a native resize from any press within 5 px of a resizable undecorated window's edge, which is the seam. That is D43 again.
  2. **X keeps no geometry for a window that is not mapped, and answers a move later, not at once.** D58's startup check read 0×0, or tao's default 800×600, and dropped every bond. It now waits until the windows are shown and three reads in a row agree, about 0.6 s at startup.
  3. **tao's outer geometry adds the window manager's `_NET_FRAME_EXTENTS`.** Weston claims 38, 38, 59 and 38 px even for an undecorated window, so a position read back 38 and 59 px from where it was set. The engine now reads a window's own rect on Linux (`platform::rect_of`).

  With the three fixes, the round trip closes exactly and the bonds survive. Not tested: any scale but 1.0.
- **Stage 1: a provisional pass, after a fix that is not Linux's alone.** A drag took its origin from the cursor when `drag_start` ran, which is after the first move. The group trailed the pointer by one input step for the whole drag: 2, 6 and 24 px at 200, 800 and 3000 px/s. It now measures from the press (`wm_press`), and a drag's release makes one last move from the real cursor. Two runs after the fix, against D39:

  | Speed | Excess over floor (≤ 1 px) | Latency (≤ 1.1 frames) |
  |---|---|---|
  | 200 px/s | −0.8, −0.6 | 0.79, 0.84 |
  | 800 px/s | 1.7, −2.6 | 1.22, 0.93 |

  The window ends exactly under the cursor at every speed, with no runaway, and updates arrive at the frame rate (53 to 62 a second). The latency splits into input to Rust, about 4 ms, and Rust to X, 5 to 13 ms (median). The second half is the window manager acknowledging the move, so it is Weston's, and the real desktop has to answer it.
- **Stage 3, the group move:** held midway, all three windows had moved by exactly the drag. After six round trips of 1,200 px, they were back to the pixel.
- **Not answerable in WSL:** stage 2, because Windows stacks WSLg's windows, not X; stage 6, because WSLg sees one display; and the scale gate at anything but 1.0. These, and stages 4, 5 and 7 by real input, are for the laptop.
- **Two fixes are not Linux's alone.** Windows has the same drag origin (smaller, since its IPC is faster) and the same dropped last move. Both belong on `main` whatever the verdict.
