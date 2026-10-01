#!/bin/bash
export DISPLAY=:0
pos() { for n in main eq playlist; do w=$(xdotool search --name "hurricane-party — $n" | head -1); printf "%s %s  " $n "$(xwininfo -id $w | awk -F: '/Absolute upper-left X/{x=$2}/Absolute upper-left Y/{y=$2}END{gsub(/ /,"",x);gsub(/ /,"",y);print x","y}')"; done; echo; }
echo -n "before:   "; pos
xdotool mousemove 180 126 mousedown 1
for i in $(seq 1 30); do xdotool mousemove_relative 10 4; sleep 0.016; done
sleep 0.3; echo -n "held +300,+120: "; pos
for i in $(seq 1 30); do xdotool mousemove_relative -- -10 -4; sleep 0.016; done
sleep 0.2; xdotool mouseup 1; sleep 0.3; echo -n "after:    "; pos
