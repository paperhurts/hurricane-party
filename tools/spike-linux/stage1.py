#!/usr/bin/env python3
"""stage1.py <title-regex-ish name> : spike-linux.md stage 1, the drag loop.

Presses on a window's title strip with XTEST, drags it right then back at a
fixed speed in 8 ms steps (a 125 Hz mouse), samples the window's real X origin
after every step, releases, and keeps sampling to catch a runaway. Reports
D39's numbers: steady lag, the window's update rate, floor = v / rate, excess
over floor, latency in frames at 60 Hz, and where the window ends up against
where the cursor says it should be."""
import ctypes, ctypes.util, statistics, subprocess, sys, time

X = ctypes.CDLL(ctypes.util.find_library("X11"))
T = ctypes.CDLL(ctypes.util.find_library("Xtst"))
X.XOpenDisplay.restype = ctypes.c_void_p
X.XOpenDisplay.argtypes = [ctypes.c_char_p]
X.XDefaultRootWindow.restype = ctypes.c_ulong
X.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
X.XFlush.argtypes = [ctypes.c_void_p]
X.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
X.XTranslateCoordinates.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int,
                                    ctypes.c_int, ctypes.POINTER(ctypes.c_int),
                                    ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong)]
T.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
T.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]

d = X.XOpenDisplay(None)
root = X.XDefaultRootWindow(d)
name = sys.argv[1] if len(sys.argv) > 1 else "hurricane-party — main"
xid = int(subprocess.check_output(["xdotool", "search", "--name", name]).split()[0])

def origin():
    rx, ry, c = ctypes.c_int(), ctypes.c_int(), ctypes.c_ulong()
    X.XTranslateCoordinates(d, xid, root, 0, 0, ctypes.byref(rx), ctypes.byref(ry), ctypes.byref(c))
    return rx.value, ry.value

X.XQueryPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong] + [ctypes.POINTER(ctypes.c_ulong)] * 2 + \
                           [ctypes.POINTER(ctypes.c_int)] * 4 + [ctypes.POINTER(ctypes.c_uint)]

def pointer():
    a, b = ctypes.c_ulong(), ctypes.c_ulong()
    rx, ry, wx, wy, m = ctypes.c_int(), ctypes.c_int(), ctypes.c_int(), ctypes.c_int(), ctypes.c_uint()
    X.XQueryPointer(d, root, a, b, rx, ry, wx, wy, m)
    return rx.value, ry.value

def settle(x, y, timeout=2.0):
    """Wait until X reports the pointer where it was put. Under XWayland with
    -enable-ei-portal (GNOME), XTEST goes through the compositor and the first
    events of a burst can take a few hundred ms to come back; a press before
    then lands where the pointer was, not where it was sent."""
    end = time.perf_counter() + timeout
    while pointer() != (int(round(x)), int(round(y))) and time.perf_counter() < end:
        time.sleep(0.005)
    time.sleep(0.1)

INJ = []
def move(x, y):
    T.XTestFakeMotionEvent(d, -1, int(round(x)), int(round(y)), 0); X.XFlush(d)
    INJ.append((time.time_ns() // 1000, int(round(x))))

def button(down):
    T.XTestFakeButtonEvent(d, 1, 1 if down else 0, 0); X.XFlush(d)

OBS = []
def run(speed, dist=600, step_ms=8):
    INJ.clear(); OBS.clear(); t_start = time.time_ns() // 1000
    wx0, wy0 = origin()
    gx, gy = wx0 + 60, wy0 + 6          # the title strip, left of the buttons
    move(gx, gy); settle(gx, gy); button(True); time.sleep(0.15)
    samples = []                         # (t, cursor dx, window dx)
    t0 = time.perf_counter()
    path = [(+1, dist), (-1, dist)]
    cx = gx
    for sign, length in path:
        n = max(1, int(length / (speed * step_ms / 1000)))
        per = sign * length / n
        for _ in range(n):
            cx += per
            move(cx, gy)
            nxt = time.perf_counter() + step_ms / 1000
            while time.perf_counter() < nxt:
                ox = origin()[0]
                samples.append((time.perf_counter() - t0, cx - gx, ox - wx0))
                OBS.append((time.time_ns() // 1000, ox))
                time.sleep(0.001)
    released = time.perf_counter() - t0
    button(False)
    after = []
    end = time.perf_counter() + 0.4
    while time.perf_counter() < end:
        after.append((time.perf_counter() - t0, origin()[0] - wx0)); time.sleep(0.002)
    # steady state: the middle 60% of the outbound leg
    out = [s for s in samples if s[1] > 0 and s[0] < released / 2]
    lo, hi = int(len(out) * 0.2), int(len(out) * 0.8)
    mid = out[lo:hi]
    lags = [c - w for _, c, w in mid]
    lag = statistics.mean(lags) if lags else float("nan")
    # update rate: distinct window positions per second over the mid section
    changes = sum(1 for a, b in zip(mid, mid[1:]) if a[2] != b[2])
    span = (mid[-1][0] - mid[0][0]) if len(mid) > 1 else 1
    rate = changes / span if span else float("nan")
    floor = speed / rate if rate else float("nan")
    frames = (lag / speed) / (1 / 60)
    final_w = after[-1][1]
    moved_after = len({w for _, w in after}) - 1
    print(f"v={speed:>5} px/s  lag {lag:6.1f} px  rate {rate:5.1f}/s  floor {floor:5.1f}  "
          f"excess {lag - floor:6.1f}  latency {frames:4.2f} frames  "
          f"end offset {final_w - 0:+d} (cursor back at 0)  moves after release {moved_after}")

    # the split: injection -> drag_move in Rust -> X shows the window there
    if speed == 800:
        import os, bisect
        log = [l.split() for l in open(os.path.expanduser("~/spike-run.log")) if l.startswith("spike-drag ")]
        rust = [(int(a[1]), int(a[2])) for a in log if int(a[1]) >= t_start]
        d1, d2 = [], []
        inj_t = [t for t, _ in INJ]
        for tr, cxr in rust:
            cands = [t for t, c in INJ if c == cxr and t <= tr]
            if cands: d1.append((tr - max(cands)) / 1000)
            target = wx0 + (cxr - gx)
            seen = [t for t, w in OBS if w == target and tr <= t <= tr + 200_000]
            if seen: d2.append((min(seen) - tr) / 1000)
        q = lambda v: (statistics.median(v), sorted(v)[int(len(v) * 0.9)]) if v else (float("nan"),) * 2
        print(f"   split at 800 px/s over {len(rust)} moves: input->Rust median {q(d1)[0]:.1f} ms (p90 {q(d1)[1]:.1f}), "
              f"Rust->X median {q(d2)[0]:.1f} ms (p90 {q(d2)[1]:.1f})")
    time.sleep(0.3)

for v in (200, 800, 3000):
    run(v)
