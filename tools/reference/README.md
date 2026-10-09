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

## Tyres

`tyres/` holds what the `tyre` generator was tuned against. All are excerpts from recordings
released into the public domain (CC0) on [Freesound](https://freesound.org), cut from its
preview encodes and levelled to about -20 dB RMS (the gain applied is listed).

| File | What | Source | Excerpt |
|---|---|---|---|
| `gravel_roll.ogg` | Tyres rolling on a gravel road | [Tires on Gravel Road 2](https://freesound.org/people/OBXJohn/sounds/251662/) by OBXJohn | 0 to 15 s, -2 dB |
| `gravel_donuts.ogg` | A truck pulling away on gravel, then sliding in circles | [Truck Pulling Away On Gravel Then Doing Donuts](https://freesound.org/people/mwchristian95/sounds/725428/) by mwchristian95 | 6.5 to 18.5 s, +12 dB |
| `gravel_skid.ogg` | Braking to a stop on gravel | [auto car or van stop brake skid gravel short.flac](https://freesound.org/people/kyles/sounds/637161/) by kyles | 0 to 1.15 s, -1 dB |
| `dirt_road.ogg` | Driving fast on a dirt road, microphone at the window (engine and wind in it) | [Car driving at fast speed on dirt road, recorded with microphone through the window](https://freesound.org/people/felix.blume/sounds/685233/) by felix.blume | 20 to 35 s, +5 dB |
| `mud_wheelspin.ogg` | A car stuck in mud, wheels spinning (engine in it; a 1960s optical recording) | [S07-20 car stuck in mud; medium distance engine revs; wheels spin.wav](https://freesound.org/people/craigsmith/sounds/675266/) by craigsmith | 2 to 14 s, +10 dB |
| `mud_pull_away.ogg` | A truck pulling away through mud (engine in it) | [auto truck real start and drive pull away through mud.flac](https://freesound.org/people/kyles/sounds/637197/) by kyles | 4 to 19 s, +4 dB |
| `tarmac_coasting.ogg` | A car coasting on tarmac with the engine off | [Tires car without an engine.](https://freesound.org/people/petruchio_ru/sounds/188507/) by petruchio_ru | 30 to 45 s, +17 dB |
| `tarmac_squeal.ogg` | Tyres screeching on tarmac | [screeching tyres / tires](https://freesound.org/people/johnnydekk/sounds/614627/) by johnnydekk | 28 to 40 s, +2 dB |
| `tarmac_handbrake.ogg` | A handbrake turn on tarmac: the squeal starts at 7.5 s | [AE0090 Volvo 740 GLE handbrake turn 01.flac](https://freesound.org/people/audible-edge/sounds/76804/) by audible-edge | 0 to 10.4 s, +3 dB |

What they show, measured with octave bands: a tyre is a mid-range sound. Rolling on gravel it
is loudest at 500 Hz and 1 kHz, 12 dB down by 2 kHz and 20 dB down by 8 kHz, with 3 to 5 % of
its energy above 2.5 kHz. Sliding on gravel is the same band, louder. Tarmac squeal is a steady
tone near 1.1 kHz with its octave above it. No usable recording of tyres on sand was found, so
sand follows the same rule (soft, low-mid, no hiss) without a reference.

## Hockey

`hockey/` holds what the hockey sounds (`skate`, the `puck_*` events, `goal_horn`, `buzzer` and
the crowd's `Arena` preset) were tuned against: public-domain (CC0) excerpts from
[Freesound](https://freesound.org), cut and levelled like the tyre ones. `skate_script.csv` and
`arena_script.csv` are the input scripts the test bench plays.

| File | What | Source | Excerpt |
|---|---|---|---|
| `skates_strides.ogg` | A skater's strides on ice, close | [Skates on Ice.wav](https://freesound.org/people/Rehanjo/sounds/593623/) by Rehanjo | 0 to 6.6 s, +10 dB |
| `puck_on_boards.ogg` | A puck hitting the boards of an outdoor rink, three times | [hockey outdoor puck hit board clean cu.flac](https://freesound.org/people/kyles/sounds/450719/) by kyles | 0 to 9 s, +0 dB |
| `stick_hit.ogg` | A hockey stick striking | [Hitting with a hockey stick](https://freesound.org/people/Luisa_Sanchez/sounds/813416/) by Luisa_Sanchez | 0.9 to 2.7 s, +27 dB |
| `goal_horn.ogg` | An arena goal horn, then the crowd | [Hockey arena goal horn with crowd applause](https://freesound.org/people/SEF7/sounds/702099/) by SEF7 | 2.5 to 16 s, -5 dB |
| `buzzer.ogg` | A buzzer | [buzzer.wav](https://freesound.org/people/Jared_DiCarlo/sounds/581374/) by Jared_DiCarlo | 0 to 5.5 s, +1 dB |
| `crowd_outrage_boo.ogg` | A hockey crowd: outrage, then booing | [Crowd, large outrage then booing reaction, hockey game, 2011.wav](https://freesound.org/people/TRP/sounds/577098/) by TRP | 0 to 22 s, +7 dB |
| `organ_charge.ogg` | A ballpark organ playing the "Charge!" call | [Baseball cavalry sting short sustain.wav](https://freesound.org/people/vckhaze/sounds/380695/) by vckhaze | whole clip, +0 dB |

What they show: a skate stride is about a second of broad noise, level from 500 Hz to 4 kHz,
that flutters by a third. A puck on the boards is a knock at 500 Hz to 1 kHz over a low thud,
with almost nothing above 2 kHz. The goal horn is three horns near 140, 203 and 257 Hz whose
harmonics fall off like a sawtooth's. The buzzer is harmonics of 126 Hz between 250 Hz and
1.3 kHz. A hockey crowd booing peaks at 500 Hz to 1 kHz with under 2 % above 2.5 kHz. The organ call is sol, do, mi, sol in C sharp (about 213, 283, 358
and 426 Hz as played), each key held so the arpeggio builds the chord; that organ is far brighter
than ours, which was kept to the level of a second, duller organ recording to stay out of the
hiss range.

## Snow and ice

`snow/` holds what the tyre's `snow` input and the wind's `Blizzard` were tuned against:
public-domain (CC0) excerpts from [Freesound](https://freesound.org), cut and levelled like the
others. `snow_script.csv` is the drive the test bench plays: packed snow (sliding at 4 s), powder,
then ice (sliding at 13 s).

| File | What | Source | Excerpt |
|---|---|---|---|
| `tyres_snow_ice.ogg` | Tyres on snow and ice, slow (the squeaks are at 1.2 and 3.7 s) | [tires_snow_ice_slow.wav](https://freesound.org/people/dunebuggy/sounds/71099/) by dunebuggy | 0 to 10.8 s, +5 dB |
| `steps_fresh_snow.ogg` | Footsteps in fresh snow | [footsteps in fresh snow](https://freesound.org/people/florianreichelt/sounds/453168/) by florianreichelt | 0 to 15 s, +14 dB |
| `studded_tyre.ogg` | A car with studded tyres | [Car with studs](https://freesound.org/people/KasperKeskinen/sounds/833165/) by KasperKeskinen | 0 to 5.3 s, +4 dB |
| `snowstorm.ogg` | A snowstorm | [Wind - Snowstorm Sound Effect](https://freesound.org/people/NicknameLarry/sounds/493680/) by NicknameLarry | 0 to 20 s, +12 dB |
| `heavy_snowstorm.ogg` | A heavy snowstorm | [DMP013016 HEAVYSNOWSTORM.wav](https://freesound.org/people/martypinso/sounds/22606/) by martypinso | 0 to 20 s, +11 dB |
| `howling_winter_storm.ogg` | A howling winter storm | [Howling winter storm ambient sounds](https://freesound.org/people/DBlover/sounds/505999/) by DBlover | 0 to 20 s, +4 dB |

What they show: tyres on snow and steps in fresh snow are broad up to about 2 kHz and
low-heavy, with about 5 % of their energy above 2.5 kHz, and cold snow squeaks in short tones
near 1 to 1.5 kHz. A studded tyre is a steady band at 250 Hz and 1 kHz. No recording of a car in
powder was found, so powder is by judgement. Snowstorms are loudest at 125 to 500 Hz, with up to
a tenth of the energy above 2.5 kHz, and they come in shoves.

## Combustion

`combustion/` holds what the `combustion` generator's presets were tuned against in October 2026:
public-domain (CC0) excerpts from [Freesound](https://freesound.org), cut and levelled like the
others. The V8 presets were also measured against the Mustang GT500, Challenger and drag car
clips in `engines/`.

| File | Preset | What | Source | Excerpt |
|---|---|---|---|---|
| `mower_honda.ogg` | Lawnmower | Honda HRB 475 mowing | [Lawn mower Honda HRB 475](https://freesound.org/people/Breviceps/sounds/761922/) by Breviceps | 40 to 50 s, +0 dB |
| `mower_running.ogg` | Lawnmower | a mower running | [Lawnmower](https://freesound.org/people/sweet_niche/sounds/450109/) by sweet_niche | 2 to 12 s, -6 dB |
| `mower_pops.ogg` | Lawnmower | a mower with pops | [motor lawnmower with pops.flac](https://freesound.org/people/kyles/sounds/637696/) by kyles | 3 to 13 s, +7 dB |
| `bmw_twin_idle.ogg` | Motorbike | BMW twin idling in a garage | [motorcycle, bmw, idle, underground, garage.wav](https://freesound.org/people/golovlev.sound/sounds/632219/) by golovlev.sound | 5 to 13 s, +7 dB |
| `bmw_twin_blips.ogg` | Motorbike | the same bike blipped | [motorcycle, bmw, engine rev, underground, garage.wav](https://freesound.org/people/golovlev.sound/sounds/632218/) by golovlev.sound | 0 to 10 s, +2 dB |
| `harley_revs.ogg` | Motorbike | Harley-Davidson Sportster held at revs | [Harley Davidson Sportster - Idling with Engine Revs](https://freesound.org/people/demodave/sounds/502690/) by demodave | 80 to 89 s, +2 dB |
| `cr85_riding.ogg` | Dirt bike 2-stroke | Honda CR85 two-stroke riding | [HONDA_85CR.wav](https://freesound.org/people/Olympia_94/sounds/704792/) by Olympia_94 | 30 to 42 s, -5 dB |
| `scooter_idle_revs.ogg` | Dirt bike 2-stroke | Gilera Runner two-stroke scooter: cold idle, revs | [Gilera Runner 2007 revs backfire cold idle](https://freesound.org/people/lovretta/sounds/140384/) by lovretta | 4 to 16 s, +4 dB |
| `scooter_revs.ogg` | Dirt bike 2-stroke | the same scooter revving | as above | 19 to 30 s, +2 dB |
| `diesel_idle_rev.ogg` | Diesel truck | a diesel idling, then revved | [Diesel engine starting revving and stopping](https://freesound.org/people/chlund/sounds/500193/) by chlund | 2 to 14 s, +6 dB |
| `vw_buggy.ogg` | Buggy flat-four | VW dune buggy starting and driving off (a 1960s recording) | [S07-14 VW dune buggy starts & drives around.wav](https://freesound.org/people/craigsmith/sounds/675200/) by craigsmith | 4 to 16 s, +4 dB |
| `open_header_v8s.ogg` | Blown V8 | open-header V8s idling and revving | [Open header V8 cars idling and revving](https://freesound.org/people/holderall/sounds/432508/) by holderall | 15 to 27 s, -5 dB |
| `worn_v8_idle.ogg` | Rattletrap V8 | a worn engine idling | [worn_engine_idle.flac](https://freesound.org/people/Kevaaq/sounds/203962/) by Kevaaq | whole clip, +4 dB |
| `worn_v8_revs.ogg` | Rattletrap V8 | the same engine revving | [worn_engine_revving.flac](https://freesound.org/people/Kevaaq/sounds/203963/) by Kevaaq | whole clip, +2 dB |

What they show, measured the same way on recordings and renders (firing frequency from a
harmonic comb, the share of energy on the firing harmonics, energy between them, the 1 ms
envelope folded over one firing, octave bands, energy above 2.5 kHz):

- A running engine never falls silent between firings. Folded over one firing, the envelope of
  every recording swings 0.3 to 2 dB. The first `combustion` clicked and went quiet: 4 to 30 dB.
  The muffler and the pipe smear each pulse; `exhaust/muffler` and `exhaust/pipe_*` do that now.
- Cylinders differ, and the difference repeats every cycle, so recordings carry energy between
  the firing harmonics (half orders within 1 to 10 dB of them). The first version had them
  14 to 24 dB down; `engine/fire_spread` gives each cylinder its own level and timing.
- At revs, real engines are noisy, not a clean buzz: 9 to 60 % of the energy below 2 kHz sits on
  the firing harmonics. The first version had 86 to 95 %.
- Diesels clatter: 6 to 29 % of their energy is above 2.5 kHz (`mechanical/clatter`). The
  Mustang and Challenger are the opposite, with almost nothing above 1 kHz.
- Mowers run at 2600 to 3500 rpm, loudest at 125 to 500 Hz.
- Revved in neutral, engines rise in 0.15 to 0.5 s and fall back to idle in 0.3 to 0.8 s.

The presets were fitted to these with a coordinate-descent fitter over the exhaust, pipe,
muffler and noise parameters. The tests in `crates/brusverk-core/tests/combustion.rs` hold the
traits above, and the loudness of every preset's sweep stays within 1 LU of the first version.
