#!/bin/sh
# Overview spectrograms for every decoded reference of a species.
#   sh tools/birds/ov.sh <species> <out_dir> [fmax] [row_s] [max_s]
D=/Users/joakimeriksson/work/gamesynth/target/refs/decoded/birds/$1
PY=/Users/joakimeriksson/work/gamesynth/target/refs/.venv/bin/python
HERE=$(dirname "$0")
mkdir -p "$2"
for f in "$D"/*.wav; do
  b=$(basename "$f" .wav)
  $PY -I "$HERE/overview.py" "$f" "$2/$1_$b.png" "${3:-10000}" "${4:-10}" "${5:-60}"
done
