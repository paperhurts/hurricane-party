# The Linux spike, on a real desktop

`docs/spike-linux.md` on `main` is the plan and the findings so far. This branch
(`spike/linux`) is the app with just enough of `platform/` to measure on Linux,
plus these scripts. It is never merged.

## What WSL already showed

Stages 0 and 1 pass provisionally in WSL's Ubuntu (WSLg, Weston, scale 1.0),
after four fixes on this branch: a GTK size request (GTK3 holds a non-resizable
window to 200 px tall), D58's check waiting until X has mapped the windows,
reading a window's own rect rather than the window manager's frame, and a drag
measured from the press. Stage 3's group move holds offsets exactly. WSLg cannot
answer stages 2 and 6: Windows stacks its windows, and it sees one display.

## Setting up the laptop

1. Packages (Ubuntu or Debian names; the Tauri v2 prerequisites, Rust, and the
   measuring tools):

       sudo apt-get install -y build-essential pkg-config curl wget file libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev rustup xdotool x11-utils

   Node 20.19 or later and pnpm, for the frontend.
2. `git clone https://github.com/paperhurts/hurricane-party && cd hurricane-party && git switch spike/linux`
3. `pnpm install && pnpm build` (the app runs from `dist/`, no dev server)
4. Empty sidecars, as CI does:
   `mkdir -p src-tauri/binaries && for t in yt-dlp deno ffmpeg; do touch src-tauri/binaries/$t-x86_64-unknown-linux-gnu; done`
5. `cd src-tauri && cargo build --features tauri/custom-protocol` (rustup installs the pinned compiler)
6. Run it under XWayland (D181), with its log where `stage1.py` looks:

       GDK_BACKEND=x11 ./target/debug/hurricane-party > ~/spike-run.log 2>&1

## Write down first

The desktop and its version, `echo $XDG_SESSION_TYPE` (wayland or x11), the
display scale, and each monitor's resolution.

## The scripts

- `wins.sh [seconds]`: the app's log and every window's X geometry
- `stage1.py`: stage 1, the drag loop at 200, 800 and 3000 px/s, with D39's numbers and the latency split
- `groupcheck.sh`: a group drag held midway, all three windows read
- `xgrab.sh <out.png> <title regex>`: screenshot X windows from X itself
- `xsize.py`, `gtksize.py`, `gtksize2.py`: the probes behind stage 0's size finding

The scripts inject input with XTEST. Stages 2, 5, 6 and 7 are judged by hand,
with a real mouse (D43): z-order when another app's window is in front, breaking
and re-forming bonds, the splitter, a second display at another scale, topmost,
and minimising as a group.
