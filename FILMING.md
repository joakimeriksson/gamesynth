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
- [ ] Joakim has watched/listened to the V8 clip
- [ ] Campfire tuned against a recording (then maybe re-cut its clip)
- [x] Reported back to the other session (first report was about the campfire draft; delivery unconfirmed)
- [x] Commit render_timeline + tools/clips
