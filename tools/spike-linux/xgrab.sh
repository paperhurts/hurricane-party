#!/bin/bash
# xgrab.sh <out.png> <title regex>: capture matching X windows
export DISPLAY=:0
D="$(dirname "$0")"
ids=$(xdotool search --name "$2" | tr "\n" " ")
python3 "$D/xshot.py" "$1" $ids
