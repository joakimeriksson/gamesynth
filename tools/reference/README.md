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

What they show: a skate stride is about a second of broad noise, level from 500 Hz to 4 kHz,
that flutters by a third. A puck on the boards is a knock at 500 Hz to 1 kHz over a low thud,
with almost nothing above 2 kHz. The goal horn is three horns near 140, 203 and 257 Hz whose
harmonics fall off like a sawtooth's. The buzzer is harmonics of 126 Hz between 250 Hz and
1.3 kHz. A hockey crowd booing peaks at 500 Hz to 1 kHz with under 2 % above 2.5 kHz.
