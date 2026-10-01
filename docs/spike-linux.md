# The Linux spike (#187)

**Status:** done. Stages 0 to 7 ran on a real GNOME desktop under XWayland, and the verdict is **go**, with four Linux rules for the port (below). The owner's calls are D181: Linux first, under XWayland, and this spike before any porting.

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

### The verdict, on a real desktop (2026-10-01)

Ubuntu 26.04 LTS, GNOME Shell 50.1 (Mutter), a Wayland session (`XDG_SESSION_TYPE=wayland`), the app under Xwayland 24.1.10 (`GDK_BACKEND=x11`, D181). Mutter's `scale-monitor-framebuffer` and `xwayland-native-scaling` are on, as Ubuntu ships them. One display for stages 0 to 5 and 7: the built-in panel, eDP-1, 1920×1080 at 60 Hz, scale 1.0, which X sees as one 1920×1080 screen at 96 dpi with a work area of 67, 32, 1853×1016 (the Ubuntu dock on the left, the top bar). WebKitGTK 2.52.6. Ubuntu's own Node (22.22) is new enough, so the frontend built on the laptop. Stages 2 to 7 were by hand with a real mouse (D43), with scripts reading X while the owner worked.

What the spike branch gained on the laptop: the X11 calls stages 2 and 7 need (a window's X id from its title, `XRaiseWindow`, `_NET_WM_STATE_ABOVE`, `WM_STATE`, and `WM_TRANSIENT_FOR` behind `SPIKE_Z=transient`, which was not needed), handles re-read once GTK has realized the windows, and `stage1.py` waiting for the pointer before it presses.

- **Injected input needs the owner's consent on GNOME.** Xwayland runs with `-enable-ei-portal`, so XTEST goes through the Remote Desktop portal (libei): a dialog asks first, and events sent before it is answered land later, all at once. Once allowed, the first events of a burst can still come back a few hundred ms late, hence the wait in `stage1.py`. The portal session ended on its own within the hour and XTEST went quiet without asking again, so `groupcheck.sh` did not run; stage 3 was measured by hand instead.
- **Stage 0: a pass, at 100 %, 200 % and 125 %.** WebKitGTK draws the chrome correctly, and the set/read round trip is exact at every scale (put 275×116 at 120,120, reads 275×116 at 120,120, 0.2 to 0.7 s after the show). Mutter claims no `_NET_FRAME_EXTENTS` for the undecorated windows, so WSL's Weston offset does not occur here, and `rect_of` is still right. At **200 %** the X screen stays the panel's 1920×1080, `Xft.dpi` is 192, `scale_factor()` reads 2, and a fresh layout comes up 550×232 and crisp, and a group dragged by hand follows the pointer exactly. At **125 %** Mutter runs X at 2x on a virtual 3072×1728 screen and scales it down by 0.625: the app sees scale 2 and lays out 550×232, which lands at the right size on the panel and did not look soft to the owner. So under Ubuntu's defaults an X client only ever sees an integer scale, and a fractional one is Mutter's downscale.
- **A layout saved at one scale comes back at the wrong size at another.** Not Linux's: `seed_state` restores the stored physical rects as they are, so a stack saved at 100 % opened at 200 % as 276×116 windows drawing 2x content, a quarter of the chrome showing. Windows would do the same after a scale change between sessions. It belongs on `main` whatever the verdict.
- **Stage 1: a pass, in open space.** Against D39:

  | Speed | Lag | Rate | Floor | Excess over floor (≤ 1 px) | Latency (≤ 1.1 frames) |
  |---|---|---|---|---|---|
  | 200 px/s | 1.9 px | 60.2/s | 3.3 | −1.4 | 0.58 |
  | 800 px/s | 6.8 px | 59.3/s | 13.5 | −6.7 | 0.51 |
  | 3000 px/s | 28.8 px | 61.7/s | 48.6 | −19.8 | 0.58 |

  The window ends exactly under the cursor, with no runaway. The split at 800 px/s: input to Rust 4.3 ms median (p90 7.8), Rust to X **1.6 ms** (p90 2.2). Mutter acknowledges a move several times faster than WSLg's Weston (5 to 13 ms).
- **Mutter keeps a client-placed window wholly inside the work area, one window at a time.** This is the finding. Asked by `XMoveWindow` for x = −100 or 10, Mutter gives 67, the dock's edge; x = 1700 or 1800 gives 1645, flush right; y = 0 or 20 gives 32, under the top bar; y = 1000 or 1060 gives 964. Windows lets a classic window hang off a display (D88 asks only that a title bar stay within reach). Two consequences, both seen by hand:
  1. **The engine's layout stops matching the screen.** A group dragged past the dock stops at 67 while `wm` records where it was sent; the next drag starts from there, and the group trails the pointer by the difference for the whole drag. The first stage 1 run, from a group parked at the dock's edge, trailed by a steady 117 px at every speed.
  2. **A bonded group shears.** Each window is clamped on its own, so a group pressed against an edge comes apart: switched to 2x with the stack near the bottom right, the EQ (550×232 at 1370,820) and the playlist (550×232 at 1370,848) came to rest overlapping by 204 px.

  The port has to clamp the whole group to the work area itself, before Mutter does, and so give up hanging off the screen on GNOME. That is a rule, not a fight: the bond math is unchanged.
- **Stage 2: a pass, by raising the group.** D41's hidden roots do not exist on X11: GTK creates no X window for a window that is never shown, so there is nothing to own the group. Raising each member with `XRaiseWindow` on a click (the engine's `raise_group`, D42) is enough. Clicking any member's title or body brought all three in front of another app together, the other app covered all three when clicked, and Alt+Tab lists Main and the library, as on Windows. Mutter raises the clicked window first and the rest follow about 190 ms later, which does not show, since bonded windows do not overlap.
- **Stage 3: a pass.** Recorded by hand over 698 px of dragging: at rest the offsets were exact every time. Mid-drag the three windows came apart 11 times, each time for 2 to 3 ms (the worst 50 px, on a fast flick), because `push_to_os` moves them one after another and a sample can land between two moves. The owner saw it as a slight wobble, more at 2x. Windows moves them the same way; whether it shows there is not measured.
- **Stage 4: a pass.** The playlist resized in its steps (heights 116 to 232 by 29, widths 275 to 425 by 25), every seam closed exactly at the end of each drag, and mid-drag a seam opened 14 times for at most 3 ms (worst 11 px), the same one-after-another move as stage 3.
- **Stage 5: a pass.** Double-clicking the Main/EQ seam broke the bond and Main then moved alone; EQ and the playlist moved together; clicking Main raised Main alone, so the z-order follows a split; dragging Main back onto EQ snapped and bonded, and the three moved as one again.
- **Stage 7: topmost and the cursor pass; minimising as a group does not.**
  - **Topmost:** shading Main put `_NET_WM_STATE_ABOVE` on all three bonded windows, they stayed in front of another app when it was clicked, and unshading cleared it. A loose Main takes only itself, as D61 says.
  - **The cursor:** `XQueryPointer` drove every drag above.
  - **Minimising:** Main's minimise took Main and left the EQ and the playlist on screen, bonded or not. **Mutter does not allow a window that skips the taskbar to be minimised**: `_NET_WM_ALLOWED_ACTIONS` lists `MINIMIZE` for Main and not for the other two, and asking Mutter directly does nothing. On Linux the port should hide (unmap) the satellites while Main is minimised and show them when it comes back, which `watch_restore` already notices. Mutter keeps a minimised X window mapped, so `WM_STATE` is the signal (`is_minimized` reads it), not the map state.
- **Stage 6: crossing displays passes; a change of display set does not resize the windows.** A Dell on HDMI, 3840×2160 at 30 Hz, scale 1.5, to the right of the panel at 1.0.
  - **X has one scale, and with mixed scales Mutter makes it 2.** X saw an 8960×2880 screen: the panel as 3840×2160, drawn at 2x and halved, and the Dell as 5120×2880, drawn at 2x and scaled by 0.75. Every monitor the app reads says scale 2, so there is no scale boundary to cross on Linux, which is the problem D52 and O14 had to solve on Windows. A fresh stack came up 550×232, the round trip exact.
  - **Crossing:** dragged onto the Dell and back, slowly and fast, the group stayed bonded and flush at every rest; mid-drag a seam opened 24 times for at most 3 ms (worst 16 px), as in stage 3. The chrome looked sharp on both displays and the right size beside other apps.
  - **Adding or removing a display can change X's one scale, and the engine does not follow.** Plugging the Dell in under a running app took X from 1 to 2: GTK followed at once and drew 2x content, but the windows stayed 275×116, a quarter of the chrome showing. Taking the Dell away took X from 2 back to 1: the windows stayed 550×232 with 1x content in their top-left quarter. The watchdog (D57, D62) moves a group and never resizes it; on Windows tao rescales a window on `WM_DPICHANGED`, and on X11 nothing does. When the scale changes, the watchdog has to re-derive sizes from the logical base, as the 2x toggle does; the same fix covers a layout saved at another scale.
  - **Unplugging the cable was not seen at all.** With the stack on the Dell and the cable pulled from the laptop's HDMI port, the kernel still reported the connector connected, so GNOME kept the display and the stack sat on a screen nobody could see. That is the laptop's (an Intel HD 630 port holding hotplug high), and it strands any app's windows the same way. Turning the Dell off in Settings stood in for the loss: Mutter first dropped the three windows on top of one another on the panel, and two seconds later the watchdog put them back as a bonded, flush stack on the panel, with the size problem above.

**The verdict.** The drag loop beats D39, groups move, resize, break, re-form and cross displays exactly, and z-order and topmost behave. What GNOME asks of the port is four rules in `wm` and `platform/`, none of which touches `bond.rs`: clamp the whole group to the work area before Mutter clamps each window; raise the group together in place of the hidden roots; hide the satellites while Main is minimised; and re-derive sizes when X's scale changes. **Go: the bond model holds on Linux under XWayland on GNOME, and the port is those four rules, not a reduced app.**
