#!/bin/sh
# V8 clip: the piston engine (fitted to a real Rover V8 recording), Muscle V8 preset, driven by a scripted
# throttle, then switched to other presets. Run from the repo root.
# Output: clips_out/v8_720p.mp4 (1280x720) and clips_out/v8_1080sq.mp4 (1080x1080).
set -e
mkdir -p clips_out
# Preset switches must match the segment times in tools/clips/v8.json.
cargo run -q -p brusverk-core --example render_timeline --release -- piston clips_out/v8.wav 24 --trace clips_out/v8.csv \
  0:preset="Muscle V8" 1.2:throttle=0.55/0.08 1.6:throttle=0/0.05 2.6:throttle=0.8/0.08 3.0:throttle=0/0.05 \
  4.0:throttle=1/0.3 5.6:throttle=0/0.05 \
  7:preset="Flat-plane V8" 7.8:throttle=1/0.1 10.4:throttle=0/0.05 11.0:throttle=1/0.05 11.6:throttle=0/0.05 \
  13:preset="War rig V8" 13.8:throttle=1/0.3 16.8:throttle=0/0.05
python3 tools/clips/compose.py tools/clips/v8.json wide clips_out/v8_720p.mp4
python3 tools/clips/compose.py tools/clips/v8.json square clips_out/v8_1080sq.mp4
