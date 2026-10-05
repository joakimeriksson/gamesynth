<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="web/assets/brusverk-logo-dark.png">
    <img src="web/assets/brusverk-logo-light.png" alt="Brusverk: procedural audio for games" width="320">
  </picture>
</p>

# Brusverk

**Procedural audio for games.** (Swedish: *brus* is noise, *verk* a works, as in a mill.)

A real-time software synthesizer for game audio, written in Rust, with a Godot 4
GDExtension. One engine covers both **procedural sound effects** (sfxr-style, seed based,
no audio files) and **playable instruments** (polyphonic subtractive synth).

```
crates/brusverk-core    pure Rust DSP (synth voice, SFX presets, generator library, graph models), #![forbid(unsafe_code)]
crates/brusverk-godot   GDExtension: SynthPatch (Resource), SynthStream (AudioStream)
crates/brusverk-wasm    C-ABI WebAssembly build of the jet engine (no bindgen)
godot/                   demo project
models/                  sound generators defined as TOML files (format: models/README.md)
web/                     the site: landing page with playable demos, and the Sound Lab (GitHub Pages)
```

## Engine

Per voice: 3 oscillators (sine / triangle / PolyBLEP saw, square, pulse / white & pink
noise) → state-variable filter (LP/HP/BP/notch) → amp. Two ADSRs (amp, filter), one LFO
(pitch, cutoff, tremolo, PWM), and sfxr-style modulators: pitch slide + acceleration, arp
jump, vibrato, pulse-width sweep, cutoff sweep. Master FX: drive, bit crusher, delay, plus
a soft limiter so output never exceeds ±1.0. Up to 32 voices, legato mono mode with glide.

Real-time guarantees: `Synth::render` never allocates, locks or blocks. `Patch` and
`Command` are `Copy` plain data, so they cross to the audio thread through a lock-free
queue. Allocation only happens in `Synth::new`.

Every parameter has a stable name (`"filter/cutoff_hz"`, `"osc1/wave"`, …) with a range
and kind (`ParamId`), which drives both the Godot inspector and `set_param` APIs.

```rust
use brusverk_core::*;
let mut synth = Synth::with_patch(48000.0, SfxPreset::Laser.generate(seed));
synth.trigger();                       // or synth.note_on(60.0, 1.0)
let mut out = vec![StereoFrame::default(); 512];
synth.render(&mut out);                // call from your audio callback
```

Audition without Godot (writes WAVs):

```
cargo run -p brusverk-core --example render_sfx --release -- sfx_out 5
cargo run -p brusverk-core --example render_models --release -- models_out   # every generator, inputs swept
```

To review sounds by eye, `tools/spectrograms.py models_out sheet.png wind_default rain_default …`
renders log-frequency spectrograms with an RMS trace (needs matplotlib and scipy). Fixed
horizontal stripes mean a tonal, chime-like sound; black vertical gaps mean dropouts; a
saturated bottom edge means too much sub.

## Godot

Build the extension, then open `godot/` in Godot 4.7:

```
cargo build -p brusverk-godot            # debug; use --release for shipping
godot --path godot                        # or open in the editor
```

`godot/brusverk.gdextension` points at `target/{debug,release}` relative to the project.
Godot registers the extension when it first imports the project (opening it in the editor
does this; headless: `godot --headless --path godot --import`).

Headless smoke test of the extension (exercises the mix path through `mix_audio()`):

```
godot --headless --path godot -s tests/smoke.gd
```

Note: if a non-one-shot `SynthStream` is still playing when the game quits, Godot prints a
harmless "1 ObjectDB instance was leaked at exit" warning — the engine unregisters
extension classes before the audio server drops still-playing playbacks. Stop the player
before quitting if you want a clean exit log.

### Sound effects

```gdscript
var player := AudioStreamPlayer2D.new()   # 2D/3D spatialization and buses just work
add_child(player)
player.stream = SynthStream.from_preset("Explosion", 1234)   # same seed = same sound
player.play()                              # one_shot: fires and finishes like a sample
```

Or make a `SynthStream` in the inspector, give it a `SynthPatch`, pick `sfx/preset` and
`sfx/seed`, press **Generate**, and tweak any parameter. Save the patch as `.tres`.

### Instruments

```gdscript
var stream := SynthStream.new()
stream.patch = load("res://sounds/lead.tres")
stream.one_shot = false                   # stay alive, wait for notes
player.stream = stream
player.play()

var pb := player.get_stream_playback() as SynthStreamPlayback
pb.note_on(60, 0.8)                        # MIDI note, velocity 0..1
pb.note_off(60)
pb.play_note(67, 1.0, 0.25)                # auto-releases after 0.25 s
pb.set_pitch_bend(0.5)                     # -1..1 × patch pitch/bend_range
pb.set_param("filter/cutoff_hz", 2000)     # this playback only
```

Editing a `SynthPatch` resource (inspector or `set_param`) updates every playing stream
that uses it, live. `AudioStreamPlayer.pitch_scale` transposes the synth instead of
resampling.

### Jet engines (vehicles)

A continuous, procedural turbine engine — no loops, no samples — driven from your ship's
physics every frame. Whine, roar, tube resonance, intake hiss, afterburner, wind and damage
layers all follow an internal RPM model with separate spool-up / spool-down inertia.

```gdscript
var engine := AudioStreamPlayer3D.new()            # per ship; distance, and Doppler via pitch_scale
engine.stream = JetEngineStream.from_preset("Racer")  # Racer, Heavy, Turbine, Scramjet
add_child(engine)
engine.play()

func _physics_process(_dt):
    var pb := engine.get_stream_playback() as JetEnginePlayback
    pb.set_state(throttle, boost, speed / max_speed, damage)   # all 0..1
    rpm_gauge.value = pb.get_rpm()                            # idle_rpm .. boost_rpm
```

`JetEnginePatch` exposes ~25 parameters (`whine/hz`, `roar/hz`, `tube/feedback`,
`spool/up`, …) in the inspector with an **Apply preset** button; save as `.tres` per ship
class. Audition offline: `cargo run -p brusverk-core --example render_jet --release -- jet_out`
renders an idle → full → boost → damaged spool-down sequence per preset.

### Generators: wind, rain, fire, engines, crowds…

Every continuous sound shares one shape: **inputs** your game sets each frame (0..1 game
state) and **params** a designer tunes (ranges, presets, inspector sliders). There are two
tiers behind the same `SoundGenerator` class:

| Tier | What | Cost |
|---|---|---|
| **Native** (53) | continuous: `jet` `hover` `combustion` `piston` `motor` `rotor` `scrape` `beam` `tyre` · `wind` `rain` `fire` `stream` `ocean` · `electric` `drone` `crowd` `radio` `siren` · `skate`; events: weapons `laser` `plasma` `cannon` `rocket` `mine_drop` `mine_blast` `explosion` `emp` `quake`, ship `impact` `shield_hit` `shield_up` `boost` `airbrake`, race UI `lock_on` `pickup` `beep` `finish`, a struck `bell`, off-road `metal_crash` `mud_splash` `suspension_thud` `debris` `rock_hit`, and hockey `puck_stick` `puck_boards` `puck_glass` `puck_post` `puck_pad` `goal_horn` `buzzer` `organ_charge` `organ_lets_go`. Each with presets (V8 muscle, Tin roof, Ship destroyed, Heavy cannon, Go…) | every continuous generator at once uses a fraction of one core; idle events cost nothing |
| **Model files** | your own, as TOML/JSON: a graph of nodes plus control formulas. See [`models/README.md`](models/README.md) and the examples in `models/` (`mine_armed` proximity ticker, `rocket_flight` for the projectile, `recharge`, `checkpoint`, `shield`…) | about 2.5x a native generator |

```gdscript
var gen := SoundGenerator.create("combustion")       # or SoundGenerator.from_file("res://sounds/shield.toml")
gen.preset = "V8 muscle"
player.stream = gen                                   # AudioStreamPlayer / 2D / 3D
player.play()

func _physics_process(_dt):
    var pb := player.get_stream_playback() as SoundGeneratorPlayback
    pb.set_inputs({"throttle": throttle, "load": load})
```

**Event sounds** (the `fx` generators, or a model file with `one_shot = true`) are layered,
stereo one-shots: an explosion is a crack, a swept body, a sub drop, rumble and a debris tail into a
small reverb. `play()` fires them, the player emits `finished` when the tail has rung out, and
**every trigger is slightly different** (`shape/variation`), which is what keeps the tenth
explosion from sounding canned. The game passes `power` and `distance` at the moment it happens:

```gdscript
var boom := SoundGenerator.create("explosion")          # once; share it between pooled players
boom.preset = "Ship destroyed"

func explode(power: float, distance: float) -> void:
    boom.set_start_input("power", power)                # glancing hit .. full blast
    boom.set_start_input("distance", distance)          # 0 = point blank, 1 = far off
    free_player().play()                                 # one player per overlapping copy
```

`pb.trigger()` refires a running one (auto-cannon bursts). Macro params bend a recipe without
rewriting it: `shape/size`, `shape/pitch_semitones`, `shape/brightness`, `shape/punch`,
`space/amount`, `space/tail`, `space/width`.

Events are **stereo**: cracks and subs stay centred, noise bodies, debris and the reverb tail
are decorrelated, chime partials are placed across the image. `space/width` 0 is mono and
sample-identical to the mono render, 0.5 is each effect's designed width (wide for `explosion`
and `shield_up`, nearly centred for `beep` and `pickup`), 1.0 doubles it. Stereo costs 30-50%
more than mono, about 0.3% of a desktop core per sounding event.

They end like samples: when the tail has rung out, `mix()` returns no frames, Godot drops the
playback and the player emits `finished`. `get_length()` reports an upper bound on one trigger,
tail included (measured by an offline render for model files). The player's `pitch_scale`
transposes events, as it does for `SynthStream`. A new native effect is a table of `Layer`s in
`generators/fx.rs`, not new DSP.

**Doppler.** On continuous generators, model files and `JetEngineStream`, `pitch_scale`
plays the sound faster or slower: every frequency in it moves together, resonances and noise
colour included, so a game can set it every frame from the source's and listener's velocities
(or switch on Godot's own Doppler tracking, which arrives the same way). It is a cubic
resampler that ramps the ratio across each block, costs nothing until the ratio first leaves
1, and accepts 0.25 to 4.

**Wheeled vehicles.** `combustion` normally revs with its own inertia from `throttle`. A geared
vehicle knows its RPM, so turn on `engine/external_rpm` and feed `rpm` (0..1 between idle and
max RPM) every frame: upshifts drop the revs, `engine/rev_limiter` bounces off the redline,
clutch-in falls to idle. `throttle` then only sets how hard it burns (lift-off backfires with
`exhaust/backfire`). Further inputs: `damage` (misfires, a dead cylinder, rattle, exhaust-leak
hiss; `damage/wear` sets a baseline) and `boost` (blower whine at `blower/ratio` x crank, more
drive). `engine/cycle` switches to two-stroke, `engine/cam_lope` gives a lumpy idle. Presets
`Blown V8`, `Buggy flat-four`, `Dirt bike 2-stroke`, `Rattletrap V8`. `pb.get_rpm()` reads
the revs back. At their defaults all of this is off and the engine sounds as it always did.

**A physical piston engine.** `piston` takes the same inputs as `combustion` (and the same
`engine/*` names, external rpm, limiter, backfire, damage and blower), so switching is a change
of generator name, but it is built the way an engine makes sound. `engine/layout` picks the
firing order and which exhaust bank each cylinder fires into: a cross-plane V8 (L R R L R L L R)
gets its rumble from the uneven pulses per bank, a flat-plane V8 fires each bank evenly and
screams; also inline four and six, boxer four, V-twin, single and V12. Each firing is a blowdown pulse
(`pulse/degrees`, `pulse/turbulence`, `pulse/steepening`) sent down a header and an exhaust pipe
that are real quarter-wave resonators: `exhaust/header_m` and `exhaust/length_m` are metres,
and their formants stay put while the revs sweep through them. `exhaust/muffling` goes from
open headers to a quiet saloon, `exhaust/crossover` is the H/X-pipe, `exhaust/unequal_ms`
and `exhaust/interference` set how much of the bank rumble reaches the tailpipe (unequal
headers are the boxer burble), `exhaust/overrun_burble` pops on a closed throttle. Around the
exhaust: `intake/level` and `intake/hz`, `mechanical/valvetrain`, `mechanical/block`,
`mechanical/fan`. The banks go left and right (`stereo/width`; 0 is the mono sound).

The defaults were fitted to a recording of a real 3.5 litre Rover V8
([`tools/reference`](tools/reference)) at idle and at 3000 rpm: third-octave levels within
about 4 dB on average, and the share of half-order "rumble" harmonics 0.19 and 0.92 against
0.21 and 0.92 measured. To fit your own recording: `tools/engine_analysis.py` measures it,
`tools/fit_engine.py recording.wav 0.3:1.9:835:0 10:13.5:2992:0.25` searches the parameters.
`render_params` renders any generator from the command line, with inputs scripted over time:

```sh
cargo run -p brusverk-core --release --example render_params -- piston drive.wav 20 \
    "preset=Muscle V8" engine/external_rpm=1 script=tools/reference/drive_script.csv
```

**Heavy engines.** For trucks and anything bigger, `piston` has what makes a big diesel sound
big. `turbo/level` adds a turbocharger: the whistle (`turbo/hz` at full speed) spools with
throttle and revs, lags behind by `turbo/lag_s`, and a lift-off dumps the boost
(`turbo/blowoff`). `engine/jake_brake` is the compression-release brake: 0.3 s after the
throttle closes (so not on a gear change) every cylinder barks into the pipe until the revs are
back near idle; the game switches it by setting the parameter. `mechanical/clatter` is injection
knock, most obvious at idle, and `engine/cycle` = two-stroke runs the firing order every turn.
All are off by default, so the engines above are unchanged.

Presets: `Stock V8`, `Muscle V8`, `Blown V8`, `Flat-plane V8`, `Boxer rumble`, `Inline four`,
`Diesel six`, `V-twin`, `Thumper`, and the heavy ones: `Heavy truck` (big-rig turbo six with an
engine brake), `War rig V8` (huge diesel V8 on open stacks), `Two-stroke diesel V8` (blown,
fires every turn) and `Tank V12`. It costs a few times what `combustion` does (two pipes,
per-sample filters), still a small fraction of a core.

`tyre` is one per vehicle (sum its wheels into the inputs): `speed`, `slip`, `load` and
`surface` (0 packed dirt, 0.25 gravel, 0.5 sand, 0.75 mud, 1 rock/tarmac; values in between
blend neighbours). Under every surface are the road rumble, the tyre's own mid-range roar
(`road/roar`) and the hum of knobbly tread. On top: gravel crunches and knocks stones against
the underside, sand is a soft rush, mud squelches, slaps and sucks, and rock or tarmac squeals
when the tyre slides.

It was tuned against recordings ([`tools/reference/tyres`](tools/reference)), which show that a
tyre is a mid-range sound: on gravel it is loudest between 500 Hz and 1 kHz and has only 3 to
5 % of its energy above 2.5 kHz. An earlier version had its gravel and sand layers up at 2.5 to
5 kHz and read as hiss or rain in a race; if you carried overrides for that (`gravel/hz`,
`gravel/crunch`, `gravel/stones`, a low-pass on the bus), try without them.

**Ice hockey.** `skate` is one skater's blades: `speed`, `push` (1 while striding), `edge`
(carving or stopping) and `surface` (fresh to cut-up ice). Each stride is its own "shhk", gliding
is nearly silent, a stop is a thick dull spray. The default keeps its top down so that several
skaters for a whole match do not add up to hiss; the `Close up` preset is what a microphone at
the boards records. The events: `puck_stick` (presets `Pass`, `Receive`, `Wrist`, `Slap`,
`Poke`), `puck_boards` (`Body check`, `Light`), `puck_glass`, `puck_post` (`Crossbar`,
`Glancing`), `puck_pad` (`Glove`, `Blocker`), `goal_horn` (`Ship's horn`, `Short blast`),
`buzzer` (`Shot clock`, `Long`), and two arena-organ stings: `organ_charge` (the six-note
"Charge!" call) and `organ_lets_go` (two chords, then two claps).

`crowd` has an indoor `Arena` preset (the cheer near 1 kHz, a short slap of room) and four
inputs for sport: raise `groan` for one "ooohh" of about a second, hold `boo`, hold `chant` for
clap, clap, clap-clap-clap at `chant/bpm`, and hold `goal` for a roar that swells for a second,
holds and takes a few seconds to settle. At 0 they do nothing, and the older presets are
unchanged. All of these were tuned against recordings in
[`tools/reference/hockey`](tools/reference).

`gen.get_input_names()` tells you what a model wants; params appear in the inspector under
their groups and reach running playbacks live. Start from a native generator; move to a
model file when you need something the library does not have; ask for a native port if a
file model ends up on many simultaneous emitters.

In Rust both tiers are a `Box<dyn Model>`: `generators::create("rain", sr)` or
`GraphModel::from_text(toml, sr)`, then `set_input` / `render_mono`. Adding a native
generator is one `model_params!` table plus a `Generator::block` function.

### Classes

| Class | Base | Purpose |
|---|---|---|
| `SynthPatch` | `Resource` | All parameters; `from_preset`, `to_json`/`from_json`, `mutate`, `set_param`/`get_param`, `get_param_names` |
| `SynthStream` | `AudioStream` | `patch`, `one_shot`; `from_preset(name, seed)` |
| `SynthStreamPlayback` | `AudioStreamPlayback` | `note_on`, `note_off`, `play_note`, `trigger`, `all_notes_off`, `panic`, `set_pitch_bend`, `set_master_gain`, `set_param`, `set_patch`, `get_active_voices`, `get_peak` |
| `JetEnginePatch` | `Resource` | Engine parameters; `from_preset`, `apply_preset`, `to_json`/`from_json`, `set_param`/`get_param` |
| `JetEngineStream` | `AudioStream` | `patch`, `initial_throttle`, `start_spooled`; `from_preset(name)` |
| `JetEnginePlayback` | `AudioStreamPlayback` | `set_throttle`, `set_boost`, `set_speed`, `set_damage`, `set_state`, `snap_rpm`, `set_param`, `set_patch`, `set_master_gain`, `get_rpm`, `get_peak` |
| `SoundGenerator` | `AudioStream` | `generator`, `config_file`, `config`, `preset`, `start_snapped`, params as properties; `create`, `from_file`, `is_one_shot`, `set_start_input`, `get_generator_names`, `get_input_names`, `get_input_default`, `get_param_names`, `get_preset_names`, `set_param`/`get_param`, `get_params_json`/`set_params_json`, `get_error`, `is_native` |
| `SoundGeneratorPlayback` | `AudioStreamPlayback` | `trigger`, `get_rpm`, `set_input`, `set_inputs`, `set_input_index`, `get_input_index`, `get_input_names`, `set_param`, `load_preset`, `snap`, `get_peak` |

SFX presets: `Pickup`, `Laser`, `Explosion`, `PowerUp`, `Hit`, `Jump`, `Blip`, `Arrow`, `Shoot`, `Throw`, `Random`.
Jet presets: `Racer`, `Heavy`, `Turbine`, `Scramjet`.

## Web: landing page and Sound Lab (WebAssembly)

Live at **https://joakimeriksson.github.io/gamesynth/** (the Sound Lab is at
[`/lab.html`](https://joakimeriksson.github.io/gamesynth/lab.html)). `web/` is a static site. The
Sound Lab runs the *same* Rust engine compiled to WebAssembly inside an AudioWorklet, in four tabs:

| Tab | What it does | Godot counterpart |
|---|---|---|
| **Engines** | Jet engine dyno: throttle / boost / speed / damage, RPM gauge, spectrum, presets, tuning | `JetEngineStream` + `pb.set_state(...)` |
| **Generators** | The whole generator library with input sliders, presets and tuning; for model files a live editor (edit, Ctrl+Enter, hear it) with a node reference | `SoundGenerator` + `pb.set_input(...)` |
| **Sound FX** | sfxr-style presets fired with a seed, mutate, waveform preview, recent list, WAV download | `SynthStream.from_preset(name, seed)` |
| **Instrument** | Playable keyboard (mouse/touch/computer keys), pitch bend, Lead/Bass/Pad/Pluck patches | `SynthStream` with `one_shot = false` |

Every tab has a tuning section generated from the engine's own parameter table and a
**Copy JSON** button whose output `JetEnginePatch.from_json()` / `SynthPatch.from_json()`
accept unchanged. `lab.html?tab=sfx` deep-links a tab, `lab.html?gen=piston&preset=Heavy%20truck`
a generator and preset, `lab.html?model=campfire` a model file.

The landing page (`web/index.html`) plays ten short demos. Each is a script in `web/demos.js`:
which generators to run and how their inputs move over time. The page runs the script through
the wasm engine faster than real time, paints the spectrogram and plays the result, so adding a
demo is adding an entry to that list.

```
rustup target add wasm32-unknown-unknown     # once
./web/build.sh                               # -> web/pkg/brusverk_wasm.wasm (about 1 MB)
python3 -m http.server -d web 8000           # open http://localhost:8000 (the lab is /lab.html)
```

If your day-to-day `cargo` is Homebrew's (no wasm target) and rustup is the keg-only
formula: `CARGO=/opt/homebrew/opt/rustup/bin/cargo ./web/build.sh`.

`crates/brusverk-wasm` exposes a plain C ABI (`jet_*`, `synth_*`, `model_*`, `gs_meta_json`, …), so
the page has no bindgen glue and the module has zero imports; `web/jet-worklet.js`
instantiates it on the audio thread, one node per tab.

**GitHub Pages:** `.github/workflows/pages.yml` builds the wasm and deploys `web/` on every
push to `main` (it enables Pages itself on the first run).

## Test bench

```
tools/test_dashboard.py            # everything -> dashboard/index.html (open it in a browser)
tools/test_dashboard.py --sounds   # only re-render and re-review the sounds
```

One page with every test surface (core suites, clippy, the wasm build exercised as the web lab
uses it, the web lab itself in headless Chrome with the sound of every tab **measured**
(`tools/web_audio_check.mjs [url]`, which also works against the deployed site), the headless
Godot smoke test and a scene-tree test that plays one-shots through real
`AudioStreamPlayer`s and awaits `finished`), release-build speed figures, and a **sound review**:
each generator and model file rendered with a standard 10 s input sweep, shown as a
spectrogram with a play button, and scored by detectors for faults found by reading
spectrograms. Continuous sounds: *Bounded*, *Follows input*, *No dropouts*, *Not a chime* (noise-like sounds
ringing at fixed pitches; calibrated at 0.9 dB for the shipped rain vs 6.6 dB for rain forced
to one pitch), *Audible*, *Presets in range*. Event sounds are fired four times (full power
twice, weak, distant) and checked for *Fires*, *Rings out*, *Varies* between identical
triggers, *Responds to power*, *Distance dulls* and *Stereo* (a real image, not out of phase).
A **Stereo: before and after** section plays the same triggers at width 0 and at the designed
width, with the measured L/R correlation and CPU cost. Sounds that legitimately break a rule are marked
exempt with the reason. The script exits non-zero when anything needs attention, so it can
gate CI. Needs numpy, scipy, matplotlib; uses node, godot and ffmpeg when present.

## Tests

```
cargo test -p brusverk-core
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Sounds you generate with Brusverk are yours; the licence covers the code.

The reference recording in `tools/reference/` is in the public domain (see the README there).
