#!/bin/sh
# Motion-designed V8 + Dirtrace clip (same audio and cut as v8_game, see FILMING.md). Run from the repo root.
# Needs clips_out/v8_mid.wav + v8_mid.csv (the engine render) and the Dirtrace captures
# clips_out/dirtrace_plugger.avi and dirtrace_rig.avi. Set CLIPS=/path/to/clips_out to read them from
# elsewhere (e.g. a worktree). Output: $CLIPS/design/v8_game_720p.mp4, v8_game_1080sq.mp4 and compare.png.
set -e
CLIPS=${CLIPS:-clips_out}
export CLIPS
mkdir -p "$CLIPS/design"
python3 tools/clips/compose.py tools/clips/v8_mid_design.json wide "$CLIPS/design/v8_mid_720p.mp4"
python3 tools/clips/compose.py tools/clips/v8_mid_design.json square "$CLIPS/design/v8_mid_sq.mp4"
python3 tools/clips/design.py tools/clips/v8_game_design.json wide "$CLIPS/design/v8_game_720p.mp4"
python3 tools/clips/design.py tools/clips/v8_game_design.json square "$CLIPS/design/v8_game_1080sq.mp4"
# Before/after sheet against the plain cut, if it is there.
if [ -f "$CLIPS/v8_game_720p.mp4" ]; then
  python3 tools/clips/compare.py "$CLIPS/v8_game_720p.mp4" "$CLIPS/design/v8_game_720p.mp4" \
    "$CLIPS/v8_game_1080sq.mp4" "$CLIPS/design/v8_game_1080sq.mp4" "$CLIPS/design/compare.png"
fi
