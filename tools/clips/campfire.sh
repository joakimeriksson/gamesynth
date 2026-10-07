#!/bin/sh
# Campfire clip: one model file, values changed on a timeline. Run from the repo root.
# Output: clips_out/campfire_720p.mp4 (1280x720) and clips_out/campfire_1080sq.mp4 (1080x1080).
set -e
mkdir -p clips_out
# Timeline must match the segments in tools/clips/campfire.json. gain only levels the
# segments against each other (it is not shown on screen).
cargo run -q -p brusverk-core --example render_timeline --release -- models/campfire.toml clips_out/campfire.wav 22 \
  0:gain=0.4 \
  5:intensity=1/1.5 \
  9.5:crackle_rate=300/0.6 9.5:crackle_hz=6000/0.6 9.5:roar_level=0.05/0.6 9.5:gain=0.25/0.6 \
  14:crackle_rate=4/0.6 14:crackle_ms=20/0.6 14:roar_hz=70/0.6 14:roar_level=1/0.6 14:crackle_hz=2800/0.6 14:gain=0.4/0.6 \
  18.5:crackle_rate=30/0.8 18.5:crackle_ms=4/0.8 18.5:roar_hz=220/0.8 18.5:roar_level=0.6/0.8 18.5:intensity=0.5/0.8
python3 tools/clips/compose.py tools/clips/campfire.json wide clips_out/campfire_720p.mp4
python3 tools/clips/compose.py tools/clips/campfire.json square clips_out/campfire_1080sq.mp4
