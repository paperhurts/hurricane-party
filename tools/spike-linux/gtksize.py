#!/usr/bin/env python3
"""gtksize.py: a 275x116 undecorated GTK3 window, resizable and not, empty and
holding a WebKit2 WebView, the way tao + wry build one. Prints the size X gives it."""
import gi, sys
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib
try:
    gi.require_version("WebKit2", "4.1"); from gi.repository import WebKit2
except Exception as e:
    WebKit2 = None; print("no WebKit2 gir:", e)
cases = [(r, wv) for r in (True, False) for wv in (False, True) if (WebKit2 or not wv)]
out = []
def run(i=0):
    if i == len(cases):
        print("\n".join(out)); Gtk.main_quit(); return False
    resizable, with_webview = cases[i]
    w = Gtk.Window(); w.set_decorated(False); w.set_default_size(1, 1); w.resize(275, 116)
    w.set_resizable(resizable)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL); w.add(box)
    if with_webview:
        v = WebKit2.WebView(); box.pack_start(v, True, True, 0)
        mn, nat = v.get_preferred_height(); out.append(f"  webview preferred height: min {mn} natural {nat}")
    w.move(1100, 120); w.show_all()
    def read():
        a = w.get_allocated_width(), w.get_allocated_height()
        out.append(f"resizable={resizable} webview={with_webview}: asked 275x116, got {a[0]}x{a[1]}")
        w.destroy(); GLib.idle_add(run, i + 1); return False
    GLib.timeout_add(900, read); return False
GLib.idle_add(run); Gtk.main()
