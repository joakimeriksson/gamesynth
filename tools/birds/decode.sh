#!/bin/sh
# Re-decode the bird reference MP3s (target/refs/birds/<species>/*.mp3) to 48 kHz mono WAVs in
# target/refs/decoded/birds/<species>/, which the other scripts read. Delete them again after.
#   sh tools/birds/decode.sh
R=/Users/joakimeriksson/work/gamesynth/target/refs
for mp3 in "$R"/birds/*/*.mp3; do
  sp=$(basename "$(dirname "$mp3")")
  mkdir -p "$R/decoded/birds/$sp"
  out="$R/decoded/birds/$sp/$(basename "$mp3" .mp3).wav"
  [ -f "$out" ] || ffmpeg -v error -y -i "$mp3" -ac 1 -ar 48000 -sample_fmt s16 "$out"
done
