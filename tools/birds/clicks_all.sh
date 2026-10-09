#!/bin/sh
# Worst high-frequency share and 2nd difference of every WAV in a directory.
#   sh tools/birds/clicks_all.sh <dir>
HERE=$(cd "$(dirname "$0")" && pwd)
for f in "$1"/*.wav; do
  printf '%-22s ' "$(basename "$f" .wav)"
  sh "$HERE/py.sh" clicks.py "$f" | sed -n '2p;$p' | tr '\n' ' '
  echo
done
