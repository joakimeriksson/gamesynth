# Reference material for engine work

| File | What | Source and licence |
|---|---|---|
| `rover_v8.ogg` | 3.5 litre Rover V8 in a 1979 Range Rover: idle (835 rpm), three blips, a hold at 2992 rpm | [Wikimedia Commons, File:Rover-v8-rr79.ogg](https://commons.wikimedia.org/wiki/File:Rover-v8-rr79.ogg), public domain |
| `rover_v8_script.csv` | Throttle and revs over time approximating that clip (from its level envelope, anchored at the two measured speeds) | derived |
| `drive_script.csv` | A short drive for auditioning any engine: idle, two blips, three gears flat out, lift-off | ours |

`tools/engine_analysis.py` measures a recording (cycle frequency, level per harmonic of the
720 degree cycle, "rumble share", third-octave levels); `tools/fit_engine.py` fits the `piston`
generator's parameters to it. The `piston` defaults and its "Stock V8" preset come from that fit.
Scripts are played with the `render_params` example (`script=...`).
