#!/bin/sh
# fetch-sidecars.sh: populate src-tauri/binaries/ for Tauri's externalBin, on
# Linux. The counterpart of fetch-sidecars.ps1 (D184): the same three
# sidecars, the same pins, from the same sidecars release, each archive and
# each program checked against its SHA-256. Bump both scripts together; the
# steps are at the top of fetch-sidecars.ps1.
#
#   yt-dlp  the official standalone executable, yt-dlp_linux (D47)
#   deno    the JS runtime yt-dlp's EJS challenges need (D46)
#   ffmpeg  yt-dlp's own build, linux64 GPL (D133)
#
# Tauri resolves externalBin by appending the target triple, so each file is
# named <tool>-x86_64-unknown-linux-gnu. binaries/ is gitignored.
#
# Usage: tools/fetch-sidecars.sh [--force]
set -eu

MIRROR=https://github.com/paperhurts/hurricane-party/releases/download/sidecars-2026-10
YTDLP_SHA=58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a
DENO_ZIP=deno-x86_64-unknown-linux-gnu.zip
DENO_ZIP_SHA=9100bbd0450c17b82002d3948bb8e7e1953c9e6506e63b586e2daa4042eca7f6
DENO_SHA=4c0a613103bdfea83a752742c698203af537b9c4f6cf12b089657a1b2adc41e0
FFMPEG_TAR=ffmpeg-N-127083-g65a3870462-linux64-gpl.tar.xz
FFMPEG_TAR_SHA=e9c007fa3c41a3dd20539c9cdab42b81ae48836672f6c2a1749b92076f76d645
FFMPEG_SHA=8ebf553fe2f604c2fef17c8681eb477de22bb523eca415f83cb64878061b1189

triple=x86_64-unknown-linux-gnu
root=$(cd "$(dirname "$0")/.." && pwd)
bin="$root/src-tauri/binaries"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
force=${1:-}
mkdir -p "$bin"

sha() { sha256sum "$1" | cut -d' ' -f1; }

# Succeeds when the sidecar needs fetching: absent, or not that exact file.
need() {
    dest="$bin/$1-$triple"
    if [ -f "$dest" ] && [ "$force" != "--force" ]; then
        if [ "$(sha "$dest")" = "$2" ]; then
            echo "  $1 already present, skipping (--force to refetch)"
            return 1
        fi
        echo "  $1 present but not the pinned build, fetching it"
    fi
    return 0
}

# A download that is not the pinned file is refused, not shipped.
fetch() {
    curl -fsSL -o "$tmp/$1" "$MIRROR/$1"
    got=$(sha "$tmp/$1")
    if [ "$got" != "$2" ]; then
        echo "$1: the download's SHA-256 is $got, not the pinned $2" >&2
        exit 1
    fi
}

# Put a program in place, checked against its own pin once it is out.
install_pinned() {
    if [ "$(sha "$1")" != "$3" ]; then
        echo "$2: the program in the pinned archive is not the pinned program" >&2
        exit 1
    fi
    install -m 755 "$1" "$bin/$2-$triple"
}

if need yt-dlp "$YTDLP_SHA"; then
    echo "fetching yt-dlp"
    fetch yt-dlp_linux "$YTDLP_SHA"
    install_pinned "$tmp/yt-dlp_linux" yt-dlp "$YTDLP_SHA"
fi

if need deno "$DENO_SHA"; then
    echo "fetching deno"
    fetch "$DENO_ZIP" "$DENO_ZIP_SHA"
    # unzip is not on every machine; Python is, for gsettings and friends.
    python3 -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extract("deno", sys.argv[2])' \
        "$tmp/$DENO_ZIP" "$tmp/deno-x"
    install_pinned "$tmp/deno-x/deno" deno "$DENO_SHA"
fi

if need ffmpeg "$FFMPEG_SHA"; then
    echo "fetching ffmpeg (D133: yt-dlp's build, ~150 MB)"
    fetch "$FFMPEG_TAR" "$FFMPEG_TAR_SHA"
    mkdir -p "$tmp/ffmpeg-x"
    tar -xJf "$tmp/$FFMPEG_TAR" -C "$tmp/ffmpeg-x" --wildcards '*/bin/ffmpeg'
    install_pinned "$(find "$tmp/ffmpeg-x" -name ffmpeg -type f | head -1)" ffmpeg "$FFMPEG_SHA"
fi

# A dev build runs its sidecars from target/debug (and a local release build
# from target/release), copied there by Tauri's build script, which does not
# run again just because a file in binaries/ changed (#18). Refresh any copy
# that is already there, so the next launch runs the pin.
for flavour in debug release; do
    for name in yt-dlp deno ffmpeg; do
        built="$root/src-tauri/target/$flavour/$name"
        pinned="$bin/$name-$triple"
        if [ -f "$built" ] && [ -f "$pinned" ] && [ "$(sha "$built")" != "$(sha "$pinned")" ]; then
            install -m 755 "$pinned" "$built"
            echo "  refreshed target/$flavour/$name"
        fi
    done
done

echo
echo "src-tauri/binaries/:"
ls -lh "$bin" | awk 'NR > 1 { print "  " $NF "  " $5 }'
