# Model files

A model file defines a continuous sound (wind, a machine, a force field…) without writing
Rust. It is TOML (or the same structure as JSON) with five sections. Files compile into an
allocation-free DSP graph; expect roughly **2.5x the CPU of a native generator**
(`jet_lite`: 211x real time vs 516x for the hand-written `jet`, single core, release build).

Try and edit them live in the web lab's **Generators** tab, then save the text next to your
game: `SoundGenerator.from_file("res://sounds/shield.toml")`.

```toml
[model]
name = "steam_vent"           # shown in tools; also category, doc, one_shot

[inputs]                      # game state, 0..1, set every frame by the game
pressure = { default = 0.5, doc = "Leak to full blast", smooth = 0.05 }

[params]                      # designer tuning: inspector sliders, presets
pipe_hz = { default = 900, min = 150, max = 4000, scale = "exp", group = "pipe" }
gain    = { default = 1.0, min = 0, max = 2, group = "master" }

[signals]                     # control-rate formulas, evaluated every 32 samples
level = "pow(pressure, 1.5) * (0.7 + 0.3 * sh(14))"

[graph]
nodes = [
  { id = "white", type = "noise" },
  { id = "throat", type = "svf", mode = "bandpass", in = ["white"], cutoff = "pipe_hz * (0.7 + 0.8 * pressure)", resonance = 0.7 },
  { id = "out", type = "gain", in = ["throat"], gain = "gain * level" },
]
out = "out"

[presets.narrow_pipe]         # named overrides of params
pipe_hz = 2400
```

* Inputs, params and signals share one namespace and must be identifiers, because formulas
  refer to them by name. A param is published as `group/name` (`pipe/pipe_hz`).
* Any node argument that takes a number also takes a formula string.
* `in = [...]` sums its sources; a source can carry a gain: `{ from = "hiss", gain = "0.3 * pressure" }`.
  Gains and oscillator frequencies are ramped across each block, so modulation is click-free.
* Signals and nodes may be listed in any order; dependencies are sorted for you.
* The output always passes a soft limiter, so a model can never exceed ±1.0.
* **Event sounds:** set `one_shot = true` under `[model]`. The model is silent (and free) until
  triggered, then formulas see `t`, the seconds since the trigger, and `rnd`, a 0..1 value
  re-rolled on every trigger. `exp(-t / 0.3)` is an envelope, `lerp(900, 90, clamp(t * 6, 0, 1))`
  a pitch dive, `1 + 0.02 * (rnd - 0.5)` makes each one a little different. It counts as
  finished once its level falls 60 dB. See `checkpoint.toml`.

## Formulas

Operators `+ - * / ^ < >`, unary minus, parentheses. Names: your inputs, params and
signals, plus `sr`, `pi`, `tau`, and `t` / `rnd` (time since the last trigger, per-trigger random).

| Pure | |
|---|---|
| `abs sqrt exp exp2 ln sin cos floor` | one argument |
| `db(x)` `midi(note)` | decibels to gain, MIDI note to Hz |
| `min(a,b) max(a,b) pow(a,b)` | |
| `clamp(x,lo,hi) lerp(a,b,t) smoothstep(lo,hi,x) select(cond,a,b)` | |

| Stateful (each call site keeps its own state) | |
|---|---|
| `slew(x, up_s, down_s)` | inertia with separate rise/fall times (engine spool, rev) |
| `lag(x, secs)` | symmetric smoothing |
| `noise(hz)` | smooth random -1..1 (gusts, drift, flutter) |
| `sh(hz)` | stepped random -1..1 (sputter) |
| `lfo(hz)` `ramp(hz)` | sine -1..1, sawtooth 0..1 |

## Nodes

| Type | Arguments (default; **bold** = required) | |
|---|---|---|
| `noise` | `color` = white \| pink \| brown | source |
| `sine` `saw` `tri` | **freq** | source, band-limited |
| `pulse` | **freq**, width (0.5) | source |
| `dust` | **rate** per second | random impulses: drops, sparks, clicks |
| `dc` | **value** | constant, e.g. the `1` in `1 + depth * lfo` |
| `svf` | **cutoff**, resonance (0.2), `mode` = lowpass \| highpass \| bandpass \| notch | 12 dB/oct |
| `lowpass` `highpass` | **cutoff** | gentle 6 dB/oct |
| `comb` | **ms**, feedback (0.5), mix (1), `max_ms` (50) | tube/pipe resonance, flanging |
| `delay` | **ms**, feedback (0.3), mix (0.3), `max_ms` (1000) | echo |
| `resonators` | `freqs = [..]`, resonance (0.95), scale (1), `route` = all \| random, `spread` (0) | ringing bank; `random` sends each impulse to one resonator, detuned by up to ±`spread` octaves so drops never sound like a chime |
| `decay` | ms (5) | impulses → decaying envelope |
| `mul` | `by = [..]` | `sum(in) * sum(by)`: envelopes, tremolo, ring mod |
| `drive` | amount (0.5) | soft saturation |
| `crush` | bits (8), downsample (1) | lo-fi |
| `gain` | **gain** | smoothed |
| `mix` | | sum of inputs |
| `reverb` | time (1.2), mix (0.3), damping (0.4) | small room; gives events a tail (its send is high-passed so subs never ring) |
| `limiter` | | soft limiter |

Recipes: **crackle** = `dust → decay → mul(noise) → svf bandpass` (see `campfire.toml`);
**drops** = `dust → resonators route="random"` (`rain_on_tent.toml`); **engine inertia** =
`slew()` on the throttle (`jet_lite.toml`); **impact flare** = `slew(hit, 0.01, 0.6)`
(`shield.toml`).

## Limits

No audio feedback between nodes (a loop is a compile error; `comb` and `delay` contain
their own). New DSP primitives still mean Rust: a file composes nodes, it cannot invent
them. At most 256 nodes, 128 signals, 64 params, 16 inputs, 2 s of delay per node, so an
untrusted file (mods) cannot take unbounded memory.

Errors name the section and entry, e.g. `node 'throat' (svf): unknown argument 'cutof'.
Valid: cutoff, resonance, mode`.
