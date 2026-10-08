# Filming Brusverk demo clips

Working notes for 15-30 s social clips (X, LinkedIn). If a session dies, resume from here.
The brief came from a Claude session on Joakim's other Mac on 2026-10-06. Work is done on this
Mac (macOS 26.6, arm64).

## Deliverable

- MP4, H.264 + AAC 192k, 48 kHz stereo, 1280x720 @ 60 fps, 15-30 s.
- Plus a 1080x1080 square variant for the LinkedIn feed.
- Feeds autoplay muted, so the sound must be visible (spectrum/waveform) and the clip
  must show a "SOUND ON" cue.
- Pitch: "Game sound without sound files". Show a value change, then the different
  sound that comes out.

## This machine (checked 2026-10-06)

| Thing | Finding |
|---|---|
| ffmpeg | 7.1 (Homebrew), **has libfreetype → drawtext works**; showspectrum/showwaves/showcqt present |
| OBS | 27.2.4, too old for per-application audio capture (needs OBS 30+); browser audio would need BlackHole |
| Loopback | BlackHole2ch installed |
| Repo | ~/work/gamesynth |

## Route: B, offline render + ffmpeg compositing (chosen)

No screen capture and no OBS. Everything is scripted and repeatable:

1. `crates/brusverk-core/examples/render_timeline.rs` renders a model (TOML file or
   native generator) to a stereo WAV while moving inputs/params on a timeline:
   `t:name=value[/glide]`. Moves happen on one running instance, so nothing clicks.
2. `tools/clips/compose.py <clip.json> <wide|square> <out.mp4>` builds the video:
   - a code panel with the model's values; changed lines are highlighted per segment;
   - a live scrolling log-frequency spectrogram (5 s window) and a waveform;
   - captions, an orange SOUND ON pill, the wordmark, and an end card;
   - site colours (`web/lab.html`) and fonts (IBM Plex Mono, Saira Condensed, OFL,
     fetched once into `target/clip-fonts`).
3. One script per clip, e.g. `sh tools/clips/campfire.sh` (run from the repo root, ~30 s).
   Output goes to `clips_out/`.

The audio is the engine's own render, bit-exact and not re-recorded, which fits the
"no audio files" pitch. The timeline in the `.sh` file must match the segment times in the
`.json` file.

Why not route A: OBS 27 can't capture one app's audio, so it would need BlackHole plus a
Multi-Output Device, and a hand-performed or JS-driven take. The lab UI would look nice, but
route B already shows the claim (the value change) more legibly.

## Scripts

### 1. V8: "This muscle V8 isn't a recording" (built, 24 s, the lead clip)

Joakim's call (2026-10-06): the V8 is the lead and the Muscle V8 opens. The `piston` generator
is fitted to a real Rover V8 recording and Joakim has approved it by ear. `sh tools/clips/v8.sh`.
The code panel shows `SoundGenerator.create("piston")`, a live throttle bar with the engine's
RPM (from the render's `--trace`), and the params that each preset changes.

| t | Engine (preset) | Throttle | Caption |
|---|---|---|---|
| 0 | Muscle V8 | idle, blips at 1.2 and 2.6, pull at 4.0, lift at 5.6 (burble) | This muscle V8 isn't a recording. / Firing order, two exhaust banks, pipes in metres. |
| 7 | Flat-plane V8 | to the 8500 limiter, lift, stab | Swap the crankshaft. / Flat-plane: it screams to the limiter. |
| 13 | War rig V8 | pull 13.8-16.8, lift (engine brake, blow-off) | Or build a war rig. / 3.4 m stacks, a turbo, an engine brake. |
| 19.5 | end card over the war rig idling | | Game sound without sound files. / repo URL / Rust · Godot 4 · WebAssembly |

Preset switches are fresh instances crossfaded over 50 ms. Loudness is −13.0 LUFS and the peak
is 0.955.

### 1b. V8 + Dirtrace: the lead clip now (built, 25.4 s)

Game footage around a shorter engine panel, so the clip is not only an engine
(`tools/clips/v8_game.json`, joined by `tools/clips/assemble.py`):

| t | Piece | Caption |
|---|---|---|
| 0-3.4 | Dirtrace race start (plugger, Muscle V8): countdown, GO!, the pack launches | Every engine you hear is synthesized live. |
| 3.4-16.9 | Engine panel, `tools/clips/v8_mid.json` (Muscle → flat-plane → war rig, 13.5 s) | as before |
| 16.9-21.9 | Dirtrace war rig: canyon, then a jump | Same war rig, in the game. |
| 21.9-25.4 | End card over the rig | Game sound without sound files. |

- Footage comes from `sh tools/clips/dirtrace_capture.sh`: Godot's movie maker records it
  offline at a locked 60 fps with the game's own audio. It runs with `DR_AUTOPILOT`, `DR_NO_MUSIC`
  (the music is .ogg; every other sound is a Brusverk generator, and the logs show no fallback)
  and the Rustback track.
- AI races are not deterministic, so a new capture differs from this one; keep the `.avi` files
  in `clips_out/` (gitignored), or re-pick the timestamps after a new capture.
- Game audio is turned down 2 dB to sit with the panel; the whole clip is −13 LUFS.
- Outputs: `clips_out/v8_game_720p.mp4` and `v8_game_1080sq.mp4`, both also in
  ~/Movies/brusverk-clips.

### 1c. V8 + Dirtrace, motion-designed (built 2026-10-08, same cut and audio as 1b)

`CLIPS=clips_out sh tools/clips/v8_game_design.sh` writes `clips_out/design/v8_game_720p.mp4`,
`v8_game_1080sq.mp4` and a before/after sheet `compare.png` (about 2 minutes).
Direction, "live signal": sound is shown as a signal everywhere, and every motion is something a
scope or a telemetry readout would do. The orange trace that draws the waveform also does the cuts.

- Cuts: a scope-trace sweep (xfade custom + a glowing orange line), 0.2 s, starting on the sound cut.
- Brand: a badge (symbol, wordmark, SOUND ON with a live mini waveform of the game audio) top-centre
  on game footage from frame 0, between the position list and the minimap. The end card has the full
  logo: the symbol pops in and sends three sound arcs out, then the wordmark, the claim, the typed URL.
- Game footage: a slow push-in (a 3 % settle on the cut back, none on frame 0, so the thumbnail
  HUD is whole), a grade towards the panel's navy, a vignette. The lower third has the panel's orange
  "changed line" marker, slides in and types its sub line.
- Panel (`compose.py --motion`, spec `v8_mid_design.json`): captions slide and type in; changed
  values flash and roll in, cascading down the lines; a segmented rev meter with peak hold and the
  redline at `engine/max_rpm` (it moves with the preset; the number turns red at the limiter);
  a glow on the spectrogram and waveform, its empty start in panel colour instead of black.
- Audio: design.py runs assemble.py's audio graph, so the AAC stream is bit-identical to 1b's
  (checked by md5 of the copied stream); −13.0 LUFS, peak −0.4 dBFS.

### 2. Campfire: parked

A 22 s campfire clip was built (`sh tools/clips/campfire.sh`, spec `tools/clips/campfire.json`)
and rejected: "The campfire is crap compared to the V8". It only comes back after the campfire
has been **tuned against a real campfire recording**. Reference recordings for it and other
realistic sounds are being collected into `target/refs/` (REPORT.md there).

### 3. Seed roulette (proposed, not built)

SFX presets: same preset, seeds 0..N, about 2 s each, with the seed number large on screen.
render_timeline does not do SFX patches yet.

## Checklist

- [x] Route chosen (B)
- [x] V8 clip, 1280x720@60 and 1080x1080@60 (`clips_out/`, copied to ~/Movies/brusverk-clips)
- [x] Joakim has watched the V8 + Dirtrace clip; the motion-designed cut (1c) approved 2026-10-08: "much cooler"
- [ ] Campfire tuned against a recording (then maybe re-cut its clip)
- [x] Reported back to the other session (first report was about the campfire draft; delivery unconfirmed)
- [x] Commit render_timeline + tools/clips
