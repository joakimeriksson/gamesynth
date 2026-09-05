# gamesynth

A real-time software synthesizer for game audio, written in Rust, with a Godot 4
GDExtension. One engine covers both **procedural sound effects** (sfxr-style, seed based,
no audio files) and **playable instruments** (polyphonic subtractive synth).

```
crates/gamesynth-core    pure Rust DSP (synth voice, SFX presets, jet engine), #![forbid(unsafe_code)]
crates/gamesynth-godot   GDExtension: SynthPatch (Resource), SynthStream (AudioStream)
crates/gamesynth-wasm    C-ABI WebAssembly build of the jet engine (no bindgen)
godot/                   demo project
web/                     Sound Lab: browser test stand running the wasm build (GitHub Pages)
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
use gamesynth_core::*;
let mut synth = Synth::with_patch(48000.0, SfxPreset::Laser.generate(seed));
synth.trigger();                       // or synth.note_on(60.0, 1.0)
let mut out = vec![StereoFrame::default(); 512];
synth.render(&mut out);                // call from your audio callback
```

Audition without Godot (writes WAVs):

```
cargo run -p gamesynth-core --example render_sfx --release -- sfx_out 5
```

## Godot

Build the extension, then open `godot/` in Godot 4.7:

```
cargo build -p gamesynth-godot            # debug; use --release for shipping
godot --path godot                        # or open in the editor
```

`godot/gamesynth.gdextension` points at `target/{debug,release}` relative to the project.
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
var engine := AudioStreamPlayer3D.new()            # per ship; Godot does doppler + distance
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
class. Audition offline: `cargo run -p gamesynth-core --example render_jet --release -- jet_out`
renders an idle → full → boost → damaged spool-down sequence per preset.

### Classes

| Class | Base | Purpose |
|---|---|---|
| `SynthPatch` | `Resource` | All parameters; `from_preset`, `to_json`/`from_json`, `mutate`, `set_param`/`get_param`, `get_param_names` |
| `SynthStream` | `AudioStream` | `patch`, `one_shot`; `from_preset(name, seed)` |
| `SynthStreamPlayback` | `AudioStreamPlayback` | `note_on`, `note_off`, `play_note`, `trigger`, `all_notes_off`, `panic`, `set_pitch_bend`, `set_master_gain`, `set_param`, `set_patch`, `get_active_voices`, `get_peak` |
| `JetEnginePatch` | `Resource` | Engine parameters; `from_preset`, `apply_preset`, `to_json`/`from_json`, `set_param`/`get_param` |
| `JetEngineStream` | `AudioStream` | `patch`, `initial_throttle`, `start_spooled`; `from_preset(name)` |
| `JetEnginePlayback` | `AudioStreamPlayback` | `set_throttle`, `set_boost`, `set_speed`, `set_damage`, `set_state`, `snap_rpm`, `set_param`, `set_patch`, `set_master_gain`, `get_rpm`, `get_peak` |

SFX presets: `Pickup`, `Laser`, `Explosion`, `PowerUp`, `Hit`, `Jump`, `Blip`, `Arrow`, `Shoot`, `Throw`, `Random`.
Jet presets: `Racer`, `Heavy`, `Turbine`, `Scramjet`.

## Web: Sound Lab (WebAssembly)

Live at **https://joakimeriksson.github.io/gamesynth/**. `web/` is a static page that runs
the *same* Rust engine compiled to WebAssembly inside an AudioWorklet, in three tabs:

| Tab | What it does | Godot counterpart |
|---|---|---|
| **Engines** | Jet engine dyno: throttle / boost / speed / damage, RPM gauge, spectrum, presets, tuning | `JetEngineStream` + `pb.set_state(...)` |
| **Sound FX** | sfxr-style presets fired with a seed, mutate, waveform preview, recent list, WAV download | `SynthStream.from_preset(name, seed)` |
| **Instrument** | Playable keyboard (mouse/touch/computer keys), pitch bend, Lead/Bass/Pad/Pluck patches | `SynthStream` with `one_shot = false` |

Every tab has a tuning section generated from the engine's own parameter table and a
**Copy JSON** button whose output `JetEnginePatch.from_json()` / `SynthPatch.from_json()`
accept unchanged. `?tab=sfx` deep-links a tab.

```
rustup target add wasm32-unknown-unknown     # once
./web/build.sh                               # -> web/pkg/gamesynth_wasm.wasm (~220 KB)
python3 -m http.server -d web 8000           # open http://localhost:8000
```

If your day-to-day `cargo` is Homebrew's (no wasm target) and rustup is the keg-only
formula: `CARGO=/opt/homebrew/opt/rustup/bin/cargo ./web/build.sh`.

`crates/gamesynth-wasm` exposes a plain C ABI (`jet_*`, `synth_*`, `gs_meta_json`, …), so
the page has no bindgen glue and the module has zero imports; `web/jet-worklet.js`
instantiates it on the audio thread, one node per tab.

**GitHub Pages:** `.github/workflows/pages.yml` builds the wasm and deploys `web/` on every
push to `main` (it enables Pages itself on the first run).

## Tests

```
cargo test -p gamesynth-core
```
