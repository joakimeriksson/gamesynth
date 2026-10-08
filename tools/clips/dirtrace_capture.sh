#!/bin/sh
# Record game footage from Dirtrace (~/work/dirtrace, a Godot game whose sound is all Brusverk) with
# Godot's movie maker: rendered offline at a locked 60 fps with the game's own audio mix, no screen
# capture. The player's vehicle is on autopilot and the music is off. Run from the repo root.
#   plugger: the Muscle V8 (the race start: countdown, GO, the pack launching)
#   rig:     the War rig V8 (canyon and jump at 31.5-36.5 s on Rustback)
set -e
mkdir -p clips_out
for v in plugger:1800 rig:2700; do
  name=${v%%:*}; frames=${v#*:}
  DR_AUTOPILOT=1 DR_VEHICLE=$name DR_TRACK=rustback DR_NO_MUSIC=1 \
    godot --path "$HOME/work/dirtrace" --write-movie "$PWD/clips_out/dirtrace_$name.avi" \
    --fixed-fps 60 --quit-after "$frames" res://scenes/race.tscn
done
