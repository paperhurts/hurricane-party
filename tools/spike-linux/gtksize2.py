#!/usr/bin/env python3
"""gtksize2.py: can a NON-resizable GTK3 window be 275x116? Try a size request on
the window, then on its child, the two places a Rust fix could put one."""
import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib
cases = ["window size_request", "child size_request", "geometry hints min=max"]
out = []
def run(i=0):
    if i == len(cases):
        print("\n".join(out)); Gtk.main_quit(); return False
    how = cases[i]
    w = Gtk.Window(); w.set_decorated(False); w.set_default_size(1, 1); w.resize(275, 116)
    w.set_resizable(False)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL); w.add(box)
    if how == "window size_request": w.set_size_request(275, 116)
    if how == "child size_request": box.set_size_request(275, 116)
    if how == "geometry hints min=max":
        from gi.repository import Gdk
        g = Gdk.Geometry(); g.min_width = g.max_width = 275; g.min_height = g.max_height = 116
        w.set_geometry_hints(None, g, Gdk.WindowHints.MIN_SIZE | Gdk.WindowHints.MAX_SIZE)
    w.move(1100, 120); w.show_all()
    def read():
        out.append(f"non-resizable + {how}: got {w.get_allocated_width()}x{w.get_allocated_height()}")
        w.destroy(); GLib.idle_add(run, i + 1); return False
    GLib.timeout_add(900, read); return False
GLib.idle_add(run); Gtk.main()
