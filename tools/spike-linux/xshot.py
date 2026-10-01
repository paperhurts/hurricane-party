#!/usr/bin/env python3
"""xshot.py <out.png> <xid>... : grab X windows' own pixels with XGetImage and
lay them out at their root positions on one canvas, so occlusion on the
Windows desktop (WSLg draws through RDP) does not matter. ctypes + zlib only."""
import ctypes, ctypes.util, struct, sys, zlib

X = ctypes.CDLL(ctypes.util.find_library("X11"))
X.XOpenDisplay.restype = ctypes.c_void_p
X.XOpenDisplay.argtypes = [ctypes.c_char_p]
X.XDefaultRootWindow.restype = ctypes.c_ulong
X.XDefaultRootWindow.argtypes = [ctypes.c_void_p]

class XImage(ctypes.Structure):
    _fields_ = [("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int),
                ("format", ctypes.c_int), ("data", ctypes.POINTER(ctypes.c_ubyte)),
                ("byte_order", ctypes.c_int), ("bitmap_unit", ctypes.c_int),
                ("bitmap_bit_order", ctypes.c_int), ("bitmap_pad", ctypes.c_int),
                ("depth", ctypes.c_int), ("bytes_per_line", ctypes.c_int),
                ("bits_per_pixel", ctypes.c_int), ("red_mask", ctypes.c_ulong),
                ("green_mask", ctypes.c_ulong), ("blue_mask", ctypes.c_ulong)]

class XWindowAttributes(ctypes.Structure):
    _fields_ = [("x", ctypes.c_int), ("y", ctypes.c_int), ("width", ctypes.c_int),
                ("height", ctypes.c_int), ("border_width", ctypes.c_int), ("depth", ctypes.c_int),
                ("visual", ctypes.c_void_p), ("root", ctypes.c_ulong), ("class_", ctypes.c_int),
                ("bit_gravity", ctypes.c_int), ("win_gravity", ctypes.c_int),
                ("backing_store", ctypes.c_int), ("backing_planes", ctypes.c_ulong),
                ("backing_pixel", ctypes.c_ulong), ("save_under", ctypes.c_int),
                ("colormap", ctypes.c_ulong), ("map_installed", ctypes.c_int),
                ("map_state", ctypes.c_int), ("all_event_masks", ctypes.c_long),
                ("your_event_mask", ctypes.c_long), ("do_not_propagate_mask", ctypes.c_long),
                ("override_redirect", ctypes.c_int), ("screen", ctypes.c_void_p)]

X.XGetImage.restype = ctypes.POINTER(XImage)
X.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                        ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]
X.XGetWindowAttributes.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(XWindowAttributes)]
X.XTranslateCoordinates.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int,
                                    ctypes.c_int, ctypes.POINTER(ctypes.c_int),
                                    ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong)]

def png(path, w, h, rows):
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    raw = b"".join(b"\x00" + r for r in rows)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))

d = X.XOpenDisplay(None)
root = X.XDefaultRootWindow(d)
grabs = []
for xid in [int(a) for a in sys.argv[2:]]:
    at = XWindowAttributes()
    X.XGetWindowAttributes(d, xid, ctypes.byref(at))
    rx, ry, child = ctypes.c_int(), ctypes.c_int(), ctypes.c_ulong()
    X.XTranslateCoordinates(d, xid, root, 0, 0, ctypes.byref(rx), ctypes.byref(ry), ctypes.byref(child))
    img = X.XGetImage(d, xid, 0, 0, at.width, at.height, 0xFFFFFFFF, 2)  # ZPixmap
    if not img:
        print(f"{xid}: XGetImage failed"); continue
    im = img.contents
    buf = ctypes.string_at(im.data, im.bytes_per_line * im.height)
    px = []
    for y in range(im.height):
        row = buf[y * im.bytes_per_line: y * im.bytes_per_line + im.width * 4]
        px.append(bytes(b for i in range(0, len(row), 4) for b in (row[i + 2], row[i + 1], row[i])))
    grabs.append((rx.value, ry.value, im.width, im.height, px))
    print(f"{xid}: {im.width}x{im.height} at {rx.value},{ry.value} depth {im.depth}")
x0 = min(g[0] for g in grabs); y0 = min(g[1] for g in grabs)
W = max(g[0] + g[2] for g in grabs) - x0; H = max(g[1] + g[3] for g in grabs) - y0
canvas = [bytearray(b"\x40\x40\x40" * W) for _ in range(H)]
for gx, gy, gw, gh, px in grabs:
    for y in range(gh):
        canvas[gy - y0 + y][(gx - x0) * 3:(gx - x0 + gw) * 3] = px[y]
png(sys.argv[1], W, H, [bytes(r) for r in canvas])
print(f"wrote {sys.argv[1]} {W}x{H} from {x0},{y0}")
