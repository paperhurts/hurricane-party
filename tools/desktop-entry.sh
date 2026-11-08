#!/bin/sh
# desktop-entry.sh: give a Linux build of the player its icon in the dock and
# a place in the app grid (#268). GNOME takes a window's icon from the
# .desktop file whose name or StartupWMClass matches its WM_CLASS, which is
# "hurricane-party"; a build run straight from target/ has none, so the dock
# showed a generic one. This writes one for this user, pointing at a build in
# this checkout, run under XWayland as D181 asks.
#
# Linux only: Windows takes the icon from the exe. A packaged Linux build
# would install its own entry; this is for the build in this checkout.
#
# Usage: tools/desktop-entry.sh [path/to/hurricane-party]   (default: the debug build)
#        tools/desktop-entry.sh --remove
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
apps="$data/applications"
entry="$apps/hurricane-party.desktop"
icons="$data/icons/hicolor"

refresh() {
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$icons" || true
}

if [ "${1:-}" = "--remove" ]; then
  rm -f "$entry" "$icons/256x256/apps/hurricane-party.png" "$icons/512x512/apps/hurricane-party.png"
  refresh
  echo "removed $entry"
  exit 0
fi

bin=${1:-"$root/src-tauri/target/debug/hurricane-party"}
case $bin in /*) ;; *) bin="$(pwd)/$bin" ;; esac
if [ ! -x "$bin" ]; then
  echo "no build at $bin: build it first (README, Linux), or pass its path" >&2
  exit 1
fi

mkdir -p "$apps" "$icons/256x256/apps" "$icons/512x512/apps"
cp "$root/src-tauri/icons/128x128@2x.png" "$icons/256x256/apps/hurricane-party.png"
cp "$root/src-tauri/icons/icon.png" "$icons/512x512/apps/hurricane-party.png"

# Exec's argument is quoted, as the Desktop Entry spec asks of a path that may
# hold a space; a `"`, `` ` ``, `$` or `\` in it is escaped with a backslash.
quoted=$(printf '%s' "$bin" | sed 's/[\\"`$]/\\&/g')
cat > "$entry" <<EOF
[Desktop Entry]
Type=Application
Name=hurricane-party
Comment=Offline media library and player
Exec=env GDK_BACKEND=x11 "$quoted"
Icon=hurricane-party
Terminal=false
Categories=AudioVideo;Audio;Video;Player;
StartupWMClass=hurricane-party
EOF
refresh
echo "wrote $entry for $bin"
