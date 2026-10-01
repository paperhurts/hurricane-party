#!/usr/bin/env python3
"""xsize.py: does the X server / window manager let a plain 275x116 window be
275x116? Creates one with Xlib alone (no GTK, no WebKit), with and without
Motif 'no decorations', maps it, and reads back its geometry."""
import ctypes, ctypes.util, time

X = ctypes.CDLL(ctypes.util.find_library("X11"))
X.XOpenDisplay.restype = ctypes.c_void_p
X.XOpenDisplay.argtypes = [ctypes.c_char_p]
X.XDefaultRootWindow.restype = ctypes.c_ulong
X.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
X.XCreateSimpleWindow.restype = ctypes.c_ulong
X.XCreateSimpleWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                                  ctypes.c_uint, ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_ulong]
X.XMapWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
X.XFlush.argtypes = [ctypes.c_void_p]
X.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
X.XInternAtom.restype = ctypes.c_ulong
X.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
X.XChangeProperty.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong,
                              ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
X.XGetGeometry.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong),
                           ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
                           ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint),
                           ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint)]
X.XMoveResizeWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint]
X.XDestroyWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
X.XStoreName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_char_p]

d = X.XOpenDisplay(None)
root = X.XDefaultRootWindow(d)

def geom(w):
    r, x, y, wd, ht, b, dp = (ctypes.c_ulong(), ctypes.c_int(), ctypes.c_int(), ctypes.c_uint(),
                              ctypes.c_uint(), ctypes.c_uint(), ctypes.c_uint())
    X.XGetGeometry(d, w, ctypes.byref(r), ctypes.byref(x), ctypes.byref(y), ctypes.byref(wd),
                   ctypes.byref(ht), ctypes.byref(b), ctypes.byref(dp))
    return wd.value, ht.value

for undecorated in (False, True):
    w = X.XCreateSimpleWindow(d, root, 700, 120, 275, 116, 0, 0, 0x202020)
    X.XStoreName(d, w, b"xsize probe")
    if undecorated:
        hints = (ctypes.c_long * 5)(2, 0, 0, 0, 0)  # MWM_HINTS_DECORATIONS, decorations = 0
        a = X.XInternAtom(d, b"_MOTIF_WM_HINTS", 0)
        X.XChangeProperty(d, w, a, a, 32, 0, hints, 5)
    X.XMapWindow(d, w); X.XSync(d, 0); time.sleep(1.0); X.XSync(d, 0)
    print(f"undecorated={undecorated}: asked 275x116, mapped {geom(w)}")
    X.XMoveResizeWindow(d, w, 700, 120, 275, 116); X.XSync(d, 0); time.sleep(0.6); X.XSync(d, 0)
    print(f"undecorated={undecorated}: after resize to 275x116 -> {geom(w)}")
    X.XDestroyWindow(d, w); X.XSync(d, 0)
