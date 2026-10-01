#!/bin/bash
export DISPLAY=:0
sleep ${1:-7}
echo "== log"; grep -v "libEGL\|hp-control" ~/spike-run.log | head -20
echo "== windows"
for w in $(xdotool search --name "hurricane-party — " 2>/dev/null); do
  n=$(xdotool getwindowname $w | sed 's/hurricane-party — //')
  g=$(xwininfo -id $w | awk -F: '/Absolute upper-left X/{x=$2}/Absolute upper-left Y/{y=$2}/Width/{w=$2}/Height/{h=$2}/Map State/{m=$2}END{gsub(/ /,"",x);gsub(/ /,"",y);gsub(/ /,"",w);gsub(/ /,"",h);gsub(/ /,"",m);printf "%sx%s at %s,%s %s",w,h,x,y,m}')
  echo "$w $n: $g"
done
