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
| `powerdust` | **rate** per second, skew (3) | impulses whose sizes follow a power law, see below |
| `impulse` | amp (1), rate (0) | one impulse when a one-shot is triggered, plus `rate` per second |
| `pattern` | **rate**, count (4), gap (1), jitter (0), variation (0), run (1), `euclid = [hits, steps(, rotation)]` | impulses in phrases |
| `ad` | attack (0.005), hold (0), decay (0.3) | click-free envelope fired by impulses in `in` |
| `adsr` | **gate**, attack (0.01), decay (0.2), sustain (0.7), release (0.3) | click-free gated envelope, source |
| `chirp` | **from**, **to**, time (0.1), curve (1), ratio (1), index (0) | triggered pitch glide (sine or FM) |
| `fm` | **freq**, ratio (1), index (1), index_decay (0); `in` optional | two-operator FM |
| `modal` | **freq**, size (0.5), decay (1), position (0.23), variation (0.3), `ratios = [..]`, `t60`, `levels` | struck object: modes with their own ring times |
| `formant` | vowel (0), shift (1), q (1), `freqs`, `qs`, `gains`, `routing` = parallel \| series | voice and creature formants |
| `pan` | pan (0), `channel` = left \| right | equal-power pan for stereo models |
| `distance` | **distance**, rolloff (24), far_hz (1000), delay_ms (0), `max_ms` (200) | level drop, air low-pass, arrival delay |
| `bubbles` | radius_mm (3), spread (1), rise (0.03), damping (1) | air bubbles in water, one per impulse in `in`: brooks, drips, boiling, splashes |

Recipes: **crackle** = `dust → decay → mul(noise) → svf bandpass` (see `campfire.toml`), or
better `powerdust → ad → mul(noise)` (`crackle.toml`); **drops** = `dust → resonators
route="random"` (`rain_on_tent.toml`); **engine inertia** = `slew()` on the throttle
(`jet_lite.toml`); **impact flare** = `slew(hit, 0.01, 0.6)` (`shield.toml`); **struck
objects** = `pattern` or `powerdust → modal` (`wind_chimes.toml`); **calls and song** =
`pattern → chirp` times `pattern → ad` (`songbird.toml`); **voices** = `saw → formant`
(`creature_grunt.toml`); **running water** = `powerdust → bubbles` plus band-passed
pink noise for the wash.

## Building blocks

The nodes below are the shared DSP blocks the native generators are made of (Rust:
`brusverk_core::dsp`), so a model file can build what the hand-written families do.

**Triggers.** `ad`, `chirp`, `modal` and a triggered `fm` fire on a *trigger*: a non-zero
sample after a zero one. One-sample impulses from `dust`, `powerdust`, `pattern` and
`impulse` each fire once; so does the rising edge of a gate. The size of that first sample is
the velocity (`ad` peaks at it, `modal` rings in proportion). Feed the same trigger to a
`chirp` and an `ad` and multiply them to get a shaped syllable.

**`powerdust`: power-law dust.** Random impulses, `rate` per second, each of size `u^skew`
with `u` uniform, so `P(size <= x) = x^(1/skew)`. Skew 1 is uniform, 3 is mostly tiny ticks
with the odd loud snap (crackle, gravel, rain, leaf contacts), 0 makes every impulse 1.
Uniform sizes are the most common reason a texture sounds fake.

```toml
{ id = "grains", type = "powerdust", rate = "40 + 600 * intensity", skew = 3 },
{ id = "click", type = "ad", in = ["grains"], attack = 0.0005, decay = 0.004 },
{ id = "tick", type = "mul", in = ["white"], by = ["click"] },
```

**`modal`: struck objects.** Modes at `freq * ratios`, each with its own ring time `t60`
(seconds to fall 60 dB; one number for all, or a list) and `levels` (linear; an impulse of
size 1 rings a mode at its level). A metal ring and a wood knock can share a bank. `size` 0..1
(0.5 plays as written) moves every mode down in pitch and up in ring time as it grows (two
octaves either way); `decay` multiplies the ring times. Each trigger strikes at `position`
(0 edge .. 0.5 middle; each mode's level follows its shape there), moved by up to
`variation` towards a random point, which also detunes (±0.6 %) and re-times (±30 %) the
modes, so no two hits are alike. A new hit adds to the modes that still ring.

```toml
{ id = "bar", type = "modal", in = ["hits"], freq = 880, ratios = [1, 2.756, 5.404, 8.933],
  t60 = [3, 1.6, 0.8, 0.4], levels = [1, 0.6, 0.35, 0.2], size = "size", variation = 0.5 },
```

`resonators` is unchanged: one shared resonance, good for drops and pipes.

**`chirp` and `fm`.** A chirp glides from `from` to `to` Hz in `time` seconds whenever it is
triggered, evenly in octaves at `curve` 1 (above 1 it lingers and falls late, below 1 it moves
early and settles), then holds `to`; before its first trigger it rests at `to`. Pitches are
read when the glide starts, so `sh()` in them varies each syllable. With `index` > 0 it is an
FM voice (modulator at `ratio` times the pitch): a buzz or warble. `fm` is a free-running
two-operator FM oscillator; give it triggers in `in` and `index_decay` (60 dB time) and every
trigger restarts a bright index that mellows (bells, UI tones). Phases never jump, and the
operators run locked, so ratios 1 and 0.5 add no DC.

```toml
{ id = "song", type = "pattern", rate = 9, count = 5, gap = 1.5, jitter = 0.15, variation = 0.6 },
{ id = "pitch", type = "chirp", in = ["song"], from = "4200 * (1 + 0.1 * sh(2))", to = 2800, time = 0.07, curve = 0.6, ratio = 0.02, index = 1.5 },
{ id = "env", type = "ad", in = ["song"], attack = 0.01, hold = 0.02, decay = 0.07 },
{ id = "syllables", type = "mul", in = ["pitch"], by = ["env"] },
```

**`ad` and `adsr`: click-free envelopes.** Unlike `decay` (instant attack, endless tail),
the attack is an S-curve of at least 0.5 ms from wherever the level is, so onsets and
retriggers never click, and decay and release (their 60 dB time) land on exactly zero, so a
sound ends in true silence with no DC left over. `ad` fires on triggers, holds `hold` seconds
at the peak, then decays. `adsr` is a source: it attacks while `gate` > 0.5, settles to
`sustain`, releases when the gate falls, and re-attacks on every one-shot trigger.

```toml
{ id = "env", type = "adsr", gate = "t < 0.4", attack = 0.03, decay = 0.15, sustain = 0.75, release = 0.12 },
```

**`formant`.** Two to four formants. `vowel` 0 a, 1 e, 2 i, 3 o, 4 u picks a preset (adult
voice; fractions morph between neighbours, so `vowel = "lerp(3, 0, t * 3)"` says "oh-ah"), or
give `freqs = [..]` with `qs` (default 10) and `gains`. `shift` scales the frequencies (a small
creature > 1, a big one < 1) and `q` scales the Qs. `routing = "parallel"` (default) sums
band-passes, each normalised to its gain at its peak; `"series"` is Klatt's cascade of
resonant low-passes, which falls between formants by itself (`gains` unused). Feed it
something buzzy (`saw`, `pulse`, noise).

```toml
{ id = "throat", type = "formant", in = ["buzz"], vowel = "vowel_now", shift = "1.6 - size" },
```

**`pattern`: rhythm and phrasing.** `count` impulses at `rate` per second make a phrase,
then `gap` extra seconds of rest (0 = one continuous stream), while `run` > 0.5. A phrase lasts
`count / rate`, so the silence from its last impulse to the next phrase is `1 / rate + gap`.
`jitter` (0..1 of a step) loosens the timing; `variation` (0..1) rolls each phrase's count and
gap (±50 %) and each impulse's size (down to -50 %). A rest lasts the gap rolled when it
began, or the current gap if that has become shorter, so a busy input cuts a long rest short.
`euclid = [3, 8]` (optionally a rotation third) makes each phrase one cycle of a Euclidean
rhythm at `rate` steps per second: only the hits sound, `count` is unused. In a one-shot, each
trigger restarts the phrase.

```toml
{ id = "knock", type = "pattern", rate = "lerp(2, 9, wind)", count = 3, gap = "lerp(4, 0.3, wind)", jitter = 0.8, variation = 0.9, run = "wind > 0.03" },
{ id = "beat", type = "pattern", rate = 8, gap = 0, euclid = [3, 8] },
```

**`bubbles`: water.** Every impulse in `in` releases an air bubble that rings at its Minnaert
pitch, 3260 / `radius_mm` Hz (a 3 mm bubble sings near 1.1 kHz, a 1 cm one at 330 Hz), as loud
as the impulse. Radii spread log-evenly over ±`spread` octaves around `radius_mm`, small ones
more often. Each dies away at its own natural rate (big bubbles ring longer; `damping` scales
it) and rises in pitch by `rise` (a fraction) over that natural ring. Recordings of brooks,
drips and splashes measure as dense swarms of short pings at a nearly steady pitch, so keep
`rise` small and drive it with many impulses (`powerdust`), not a few long chirps. Up to 16
ring at once.

```toml
{ id = "drops", type = "powerdust", rate = "lerp(20, 600, flow)", skew = 2 },
{ id = "brook", type = "bubbles", in = ["drops"], radius_mm = "lerp(2.5, 5, depth)", spread = 1.2 },
```

**`impulse`.** One impulse of size `amp` on each one-shot trigger (the hit that starts an
`ad`, a `chirp` or a `modal`), plus a steady train at `rate` per second if `rate` > 0.

## Stereo and distance

Models are mono unless `[graph]` also names `out_right`: then `out` is the left channel and
`out_right` the right, and players hear them as stereo. Build each channel with `pan`
nodes, which output one side (`channel`) of an equal-power pan (`left² + right² = 1`; the
centre is -3 dB per side):

```toml
{ id = "l", type = "pan", in = ["bird"], pan = "pan", channel = "left" },
{ id = "r", type = "pan", in = ["bird"], pan = "pan", channel = "right" },
# under [graph]:  out = "l"  and  out_right = "r"
```

The mono render of a stereo model (`render_mono`, and anything that asks for mono) centres
every `pan` and averages the two outputs, so it is the same sound with no width, not a
fold-down. Other nodes are mono: a `reverb` per channel (as in `wind_chimes.toml`) gives each
side its own tail. Files without `out_right` play the same in both channels, exactly as
before.

`distance` (0..1) places a sound: its level falls `rolloff` dB by distance 1, a 12 dB/oct
air low-pass falls from 18 kHz to `far_hz`, and with `delay_ms` it arrives up to that much
later (moving it then bends the pitch, as a real delay does). Add reverb in proportion for
the room.

```toml
{ id = "far", type = "distance", in = ["bird"], distance = "distance", rolloff = 10, far_hz = 3000, delay_ms = 20 },
```

## Limits

No audio feedback between nodes (a loop is a compile error; `comb` and `delay` contain
their own). New DSP primitives still mean Rust: a file composes nodes, it cannot invent
them. At most 256 nodes, 128 signals, 64 params, 16 inputs, 2 s of delay per node, so an
untrusted file (mods) cannot take unbounded memory.

Errors name the section and entry, e.g. `node 'throat' (svf): unknown argument 'cutof'.
Valid: cutoff, resonance, mode`.
