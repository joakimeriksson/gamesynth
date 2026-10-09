#!/bin/sh
# Transcribe every decoded reference of a species (first max_s seconds of each).
#   sh tools/birds/batch.sh <species> <out_dir> <fmin> <fmax> [thresh_db] [win] [min_gap_ms] [min_dur_ms] [max_s]
D=/Users/joakimeriksson/work/gamesynth/target/refs/decoded/birds/$1
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$2"
for f in "$D"/*.wav; do
  b=$(basename "$f" .wav)
  echo "== $b"
  sh "$HERE/py.sh" transcribe.py "$f" 0 "${9:-60}" "$3" "$4" "$2/$1_$b" "${5:-14}" "${6:-512}" "${7:-6}" "${8:-8}" | cut -c1-260
done
