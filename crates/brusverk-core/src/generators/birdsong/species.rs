//! The species songbook: each bird's song as a score, transcribed from CC0 recordings.
//!
//! Nothing here is audio. Each species was transcribed from several CC0 field recordings on
//! Freesound (listed with their URLs in `target/refs/birds/SOURCES.tsv`; the analysis scripts are
//! in `tools/birds/`): the fundamental of every syllable was tracked with a 1 ms hop, the
//! syllables segmented and clustered into the species' repertoire, and the song grammar (order,
//! repeats, song length, pauses) measured, with their spread. What is kept is the score:
//!
//! * [`Syllable`]: a few keypoints `(time 0..1, Hz, amplitude)` per syllable type, joined by
//!   straight lines in log-frequency, plus a second contour where the bird sings with both
//!   sides of its syrinx at once.
//! * [`Timbre`]: harmonics, fast FM and AM, noise, roughness and formants of the voice.
//! * [`Phrase`] and [`Song`]: how syllables repeat and follow each other, with ranges that every
//!   rendition draws from.
//!
//! A species is a few hundred numbers. Species marked `invented` (the tropical voices) were
//! composed, not transcribed; no recordings back them.

/// A keypoint of a syllable: time as a fraction 0..1 of the syllable, frequency in Hz, and
/// amplitude 0..1 (the loudest syllable of a species peaks near 1).
pub type Key = (f32, f32, f32);

/// One syllable type.
#[derive(Clone, Copy, Debug)]
pub struct Syllable {
    /// Seconds.
    pub dur: f32,
    pub keys: &'static [Key],
    /// The syrinx's other side, sounding at the same time; empty for one voice.
    pub keys2: &'static [Key],
    /// Index into [`Species::timbres`].
    pub timbre: u8,
}

/// The sound of a voice.
#[derive(Clone, Copy, Debug)]
pub struct Timbre {
    /// Harmonics 2, 3 and 4 relative to the fundamental (tonal voices).
    pub harm: [f32; 3],
    /// Fast frequency modulation: rate in Hz, depth in octaves (warble, buzz).
    pub fm: (f32, f32),
    /// Fast amplitude modulation: rate in Hz, depth 0..1 (rattle, tremolo).
    pub am: (f32, f32),
    /// Share of band-limited noise in the voice, 0 (pure tone) .. 1 (all noise).
    pub noise: f32,
    /// Width of the noise band as a fraction of its centre (0: the default, about 0.6 for a
    /// tonal voice and 1.2 around a formant voice's first formant).
    pub band: f32,
    /// Roughness: random jitter of pitch and level, 0 (clean) .. 1 (rasping).
    pub rough: f32,
    /// Depth of the species' micro-modulation ([`Micro`]) on this voice, as a multiple (0 is
    /// taken as 1). A blackbird's warbled notes carry several times the FM of its plain ones.
    pub wobble: f32,
    /// Formants `(Hz, bandwidth Hz, level)` that shape a full harmonic series (crow, gull, the
    /// owl's "ke-wick"). Empty: the voice is the fundamental plus `harm`.
    pub formants: &'static [(f32, f32, f32)],
    /// A knock on wood (woodpecker drumming): each syllable strikes a modal bank tuned by its
    /// first keypoint instead of sounding a voice.
    pub knock: bool,
}

impl Timbre {
    pub const PURE: Timbre = Timbre { harm: [0.0; 3], fm: (0.0, 0.0), am: (0.0, 0.0), noise: 0.0, band: 0.0, rough: 0.0, wobble: 0.0, formants: &[], knock: false };
}

/// A unit of one or more syllables, repeated.
#[derive(Clone, Copy, Debug)]
pub struct Phrase {
    /// Syllables of the unit and their onsets in seconds from the unit's start.
    pub unit: &'static [(u8, f32)],
    /// Repeats, drawn evenly from this range (inclusive).
    pub reps: (u8, u8),
    /// Seconds from one unit's start to the next, at the first and at the last repeat (a trill
    /// that speeds up has the second shorter).
    pub period: (f32, f32),
    /// Pitch change over the phrase, octaves from the first to the last repeat.
    pub drift: f32,
    /// Level of the last repeat relative to the first.
    pub swell: f32,
    /// Seconds of silence after the phrase.
    pub gap: f32,
}

/// One step of a song: a phrase drawn from `phrases`, present with probability `p`.
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub phrases: &'static [u8],
    pub p: f32,
}

/// A song type: its slots in order, sung `laps` times through (each lap draws again).
#[derive(Clone, Copy, Debug)]
pub struct Song {
    pub slots: &'static [Slot],
    pub laps: (u8, u8),
    pub weight: f32,
}

/// How much renditions vary (scaled by the `variety` param, 0.5 = as measured).
#[derive(Clone, Copy, Debug)]
pub struct Variation {
    /// Octaves (about one standard deviation) of a whole song's pitch.
    pub pitch: f32,
    /// Fraction of a whole song's tempo.
    pub tempo: f32,
    /// Octaves of each syllable's own pitch.
    pub note: f32,
    /// Octaves between individuals of the species.
    pub individual: f32,
}

/// The fine modulation that keeps a real note alive, measured as the residual of each pitch and
/// level track from its 41 ms smoothing: quasi-periodic FM `cents` deep (rms) around `rate`
/// Hz, plus a slow drift, and level flutter `flutter` dB (rms).
#[derive(Clone, Copy, Debug)]
pub struct Micro {
    pub rate: f32,
    pub cents: f32,
    pub flutter: f32,
}

/// When a species sings, for the chorus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hours {
    Day,
    Night,
    Both,
}

#[derive(Clone, Copy, Debug)]
pub struct Species {
    pub name: &'static str,
    pub latin: &'static str,
    /// Composed, not transcribed (no recordings behind it).
    pub invented: bool,
    pub syllables: &'static [Syllable],
    pub timbres: &'static [Timbre],
    pub phrases: &'static [Phrase],
    pub songs: &'static [Song],
    /// Alarm or excited calls, sung instead of songs as `excitement` rises; empty: none.
    pub alarm: &'static [Song],
    /// Silence between songs: median seconds and spread (log-normal sigma).
    pub pause: (f32, f32),
    pub alarm_pause: (f32, f32),
    /// Songs per bout (0 = no bouts) and the longer silence after a bout.
    pub bout: (u8, u8),
    pub bout_pause: (f32, f32),
    /// Chance that the next song is the same song type (eventual variety; 0 = always new).
    pub repeat: f32,
    pub var: Variation,
    pub micro: Micro,
    /// Loudness relative to the others.
    pub level: f32,
    pub hours: Hours,
}

// ---------------------------------------------------------------------------------------------
// Helpers for writing the tables.
// ---------------------------------------------------------------------------------------------

const fn syl(dur: f32, keys: &'static [Key], timbre: u8) -> Syllable {
    Syllable { dur, keys, keys2: &[], timbre }
}

const fn duet(dur: f32, keys: &'static [Key], keys2: &'static [Key], timbre: u8) -> Syllable {
    Syllable { dur, keys, keys2, timbre }
}

/// A phrase repeated `reps` times at `period` (constant), then `gap` of silence.
const fn rep(unit: &'static [(u8, f32)], reps: (u8, u8), period: (f32, f32), gap: f32) -> Phrase {
    Phrase { unit, reps, period, drift: 0.0, swell: 1.0, gap }
}

/// A single unit, sung once.
const fn once(unit: &'static [(u8, f32)], gap: f32) -> Phrase {
    Phrase { unit, reps: (1, 1), period: (0.0, 0.0), drift: 0.0, swell: 1.0, gap }
}

const fn tone(harm: [f32; 3]) -> Timbre {
    Timbre { harm, ..Timbre::PURE }
}

const fn slot(phrases: &'static [u8], p: f32) -> Slot {
    Slot { phrases, p }
}

const fn song(slots: &'static [Slot], laps: (u8, u8), weight: f32) -> Song {
    Song { slots, laps, weight }
}

// ---------------------------------------------------------------------------------------------
// Common blackbird (Turdus merula)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 811988 (richwise, isolated, close) and 725332 (Sacha.Julien), with
// 725058/547901/813087 for the song structure. A song is 2.5-4 s: a low, fluting motif of
// slurred whistles at 1.35-3 kHz (often with the syrinx's other side sounding a weak note an
// octave-ish higher), then a quiet, squeaky twitter of harmonic-rich sweeps and noisy squeaks
// at 2-7 kHz. Every song differs from the last (no song-type repeats: eventual variety 0);
// pauses 3-10 s, median 5.5 s.

const BLACKBIRD_SYL: &[Syllable] = &[
    // 0: the long opening slur, 2.0 -> 2.8 -> 1.35 kHz, upper side 4.5 -> 3.0 kHz.
    duet(0.6, &[(0.0, 2000.0, 0.0), (0.06, 2150.0, 0.8), (0.25, 2600.0, 0.95), (0.45, 2800.0, 1.0), (0.7, 2300.0, 0.9), (0.92, 1450.0, 0.7), (1.0, 1350.0, 0.0)],
        &[(0.0, 4500.0, 0.0), (0.15, 4500.0, 0.07), (0.5, 4100.0, 0.06), (0.85, 3200.0, 0.04), (1.0, 3000.0, 0.0)], 4),
    // 1: 2.7 kHz note with a 5 kHz note on the other side.
    duet(0.26, &[(0.0, 2550.0, 0.0), (0.15, 2700.0, 1.0), (0.8, 2650.0, 0.8), (1.0, 2500.0, 0.0)],
        &[(0.0, 5000.0, 0.0), (0.2, 5100.0, 0.4), (0.8, 4950.0, 0.3), (1.0, 4800.0, 0.0)], 4),
    // 2: soft high note 5.1 -> 4.3 kHz.
    syl(0.26, &[(0.0, 5100.0, 0.0), (0.1, 5100.0, 0.55), (0.6, 4700.0, 0.5), (1.0, 4300.0, 0.0)], 0),
    // 3: a hook 3.05 kHz stepping down to 2.4 kHz.
    syl(0.22, &[(0.0, 3050.0, 0.0), (0.1, 3000.0, 0.8), (0.3, 2400.0, 0.9), (0.9, 2400.0, 0.8), (1.0, 2350.0, 0.0)], 0),
    // 4: a broad 2.75 kHz note, both sides a few hundred Hz apart.
    duet(0.28, &[(0.0, 2500.0, 0.0), (0.15, 2750.0, 0.9), (0.6, 2800.0, 1.0), (1.0, 2600.0, 0.0)],
        &[(0.0, 2950.0, 0.0), (0.3, 3050.0, 0.45), (1.0, 2900.0, 0.0)], 4),
    // 5: the liquid step 2.5 -> 1.9 kHz.
    syl(0.4, &[(0.0, 2550.0, 0.0), (0.1, 2500.0, 0.9), (0.4, 2450.0, 0.9), (0.5, 1950.0, 0.85), (0.9, 1900.0, 0.7), (1.0, 1850.0, 0.0)], 0),
    // 6: low flute at 1.7 kHz.
    syl(0.26, &[(0.0, 1650.0, 0.0), (0.2, 1700.0, 0.8), (0.8, 1750.0, 0.7), (1.0, 1700.0, 0.0)], 0),
    // 7: a dip 2.2 -> 1.8 -> 1.9 kHz.
    syl(0.24, &[(0.0, 2100.0, 0.0), (0.2, 2200.0, 0.9), (0.5, 1800.0, 0.8), (0.7, 1900.0, 0.9), (1.0, 1850.0, 0.0)], 0),
    // 8: a hook from 2.95 kHz down to 2.2 and back to 2.6 kHz.
    syl(0.27, &[(0.0, 2950.0, 0.0), (0.15, 2900.0, 0.8), (0.4, 2200.0, 0.9), (0.6, 2600.0, 0.8), (1.0, 2550.0, 0.0)], 0),
    // 9: a long rise 1.95 -> 2.4 kHz.
    syl(0.4, &[(0.0, 1950.0, 0.0), (0.1, 2000.0, 0.8), (0.8, 2400.0, 1.0), (1.0, 2350.0, 0.0)], 4),
    // 10: a wavering 2.25 -> 2.85 kHz whistle.
    syl(0.45, &[(0.0, 2250.0, 0.0), (0.25, 2450.0, 0.9), (0.45, 2300.0, 0.85), (0.7, 2700.0, 1.0), (0.85, 2850.0, 0.8), (1.0, 2800.0, 0.0)], 4),
    // 11: twitter: a harmonic-rich sweep up to 2.7 kHz and down to 1.9 kHz.
    syl(0.45, &[(0.0, 1800.0, 0.0), (0.08, 2700.0, 0.55), (0.3, 2400.0, 0.6), (0.85, 2000.0, 0.4), (1.0, 1900.0, 0.0)], 1),
    // 12: twitter: a noisy squeak 3 -> 4.8 kHz.
    syl(0.25, &[(0.0, 3000.0, 0.0), (0.1, 4500.0, 0.45), (0.5, 4800.0, 0.55), (1.0, 4200.0, 0.0)], 2),
    // 13: twitter: a short squeaky chirp 4.3 -> 3.3 kHz.
    syl(0.07, &[(0.0, 4300.0, 0.0), (0.2, 4100.0, 0.5), (1.0, 3300.0, 0.0)], 1),
    // 14: a quick rise 1.7 -> 2.1 kHz.
    syl(0.15, &[(0.0, 1700.0, 0.0), (0.7, 2100.0, 0.9), (1.0, 2050.0, 0.0)], 0),
    // 15: alarm rattle element: a loud sweep 7.8 -> 4.2 kHz (Freesound 835517, Kanny100).
    syl(0.11, &[(0.0, 7800.0, 0.0), (0.1, 7600.0, 0.8), (0.6, 5500.0, 0.9), (1.0, 4200.0, 0.0)], 3),
];

pub const BLACKBIRD: Species = Species {
    name: "Blackbird",
    latin: "Turdus merula",
    invented: false,
    syllables: BLACKBIRD_SYL,
    timbres: &[
        // The fluting notes: harmonics 40-60 dB down (median -46 dB on 811988's notes).
        tone([0.005, 0.002, 0.0]),
        Timbre { harm: [0.2, 0.08, 0.03], noise: 0.06, ..Timbre::PURE },
        Timbre { harm: [0.12, 0.05, 0.02], noise: 0.12, band: 0.3, rough: 0.2, ..Timbre::PURE },
        Timbre { harm: [0.1, 0.03, 0.0], noise: 0.35, band: 0.4, ..Timbre::PURE },
        // 4: the warbled flute: fast, irregular FM around 50 Hz, about 120 cents rms (3.5
        // semitones peak to peak), measured on the long notes of 811988 (`tools/birds/fm.py`).
        // Irregular, not a sine: the real notes show a smeared band, not a zigzag.
        Timbre { harm: [0.005, 0.002, 0.0], wobble: 4.5, ..Timbre::PURE },
    ],
    phrases: &[
        // Legato: notes follow each other within 0-30 ms (median 17 ms in 811988, 44 % joined),
        // and a note on one side of the syrinx may start before the other's has ended.
        once(&[(0, 0.0)], 0.04),
        once(&[(1, 0.0), (2, 0.22)], 0.01),
        once(&[(3, 0.0)], 0.0),
        once(&[(4, 0.0)], 0.012),
        once(&[(5, 0.0)], 0.0),
        once(&[(6, 0.0), (7, 0.25)], 0.01),
        once(&[(8, 0.0)], 0.02),
        once(&[(9, 0.0)], -0.02),
        once(&[(10, 0.0)], 0.03),
        once(&[(14, 0.0), (3, 0.15)], 0.0),
        once(&[(11, 0.0)], 0.0),
        rep(&[(13, 0.0)], (3, 6), (0.075, 0.07), 0.05),
        once(&[(12, 0.0)], 0.0),
        // 13: the alarm rattle, 6-10 sweeps every 0.135 s.
        Phrase { unit: &[(15, 0.0)], reps: (6, 10), period: (0.14, 0.13), drift: -0.02, swell: 1.0, gap: 0.0 },
    ],
    songs: &[song(&[slot(&[0, 5, 7, 8], 1.0), slot(&[1, 2, 3, 4, 5, 6, 9], 1.0), slot(&[1, 2, 3, 4, 6, 8, 9], 1.0), slot(&[1, 2, 3, 4, 5, 6], 0.9), slot(&[1, 2, 3, 4, 6, 9], 0.75), slot(&[2, 3, 4, 5], 0.5), slot(&[10, 11, 12], 0.9), slot(&[11, 12], 0.35)], (1, 1), 1.0)],
    alarm: &[song(&[slot(&[13], 1.0)], (1, 1), 1.0)],
    pause: (5.5, 0.4),
    alarm_pause: (1.2, 0.4),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 0.0,
    var: Variation { pitch: 0.04, tempo: 0.07, note: 0.03, individual: 0.12 },
    micro: Micro { rate: 65.0, cents: 32.0, flutter: 3.20 },
    level: 0.9,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// European robin (Erithacus rubecula)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 671058 (Andrew.soundscape), with 725331 (Sacha.Julien), 402381
// (freemaster2) and 276560 (tubbers). Songs of 1.5-3 s, each different: often a thin, high
// opening (7.6-9 kHz, wavering), then a tumble of quick slurs, two-voiced down-strokes and
// sustained buzzy notes at 2.3-5 kHz, ending in a twitter or a fast trill (20-25 notes a
// second). Pauses 2-6 s.

const ROBIN_SYL: &[Syllable] = &[
    // 0: high wavering whistle 8.3-8.9 kHz.
    syl(0.27, &[(0.0, 8300.0, 0.0), (0.1, 8700.0, 0.7), (0.5, 8900.0, 0.9), (0.85, 8500.0, 0.8), (1.0, 8100.0, 0.0)], 1),
    // 1: high rising 7.8 -> 8.7 kHz.
    syl(0.18, &[(0.0, 7800.0, 0.0), (0.2, 8000.0, 0.8), (0.7, 8700.0, 1.0), (1.0, 8500.0, 0.0)], 1),
    // 2: a two-voiced down-stroke 4.9 -> 3.5 kHz over 4.5 -> 3.0 kHz.
    duet(0.08, &[(0.0, 4900.0, 0.0), (0.2, 4700.0, 1.0), (1.0, 3500.0, 0.0)], &[(0.0, 4500.0, 0.0), (0.2, 4300.0, 0.6), (1.0, 3000.0, 0.0)], 0),
    // 3: a short flat 4.4 kHz note.
    syl(0.09, &[(0.0, 4300.0, 0.0), (0.2, 4400.0, 1.0), (0.8, 4400.0, 0.9), (1.0, 4300.0, 0.0)], 0),
    // 4: down to a sustained, buzzing 3 kHz.
    syl(0.25, &[(0.0, 4000.0, 0.0), (0.08, 3800.0, 0.9), (0.25, 3100.0, 1.0), (0.9, 3000.0, 0.9), (1.0, 2950.0, 0.0)], 2),
    // 5, 6: twitter "V" and "^" notes.
    syl(0.05, &[(0.0, 4800.0, 0.0), (0.4, 2700.0, 1.0), (1.0, 4200.0, 0.0)], 0),
    syl(0.055, &[(0.0, 2800.0, 0.0), (0.5, 3800.0, 1.0), (1.0, 2900.0, 0.0)], 0),
    // 7: a down hook 4.4 -> 2.3 kHz.
    syl(0.06, &[(0.0, 4400.0, 0.0), (0.2, 4100.0, 1.0), (1.0, 2300.0, 0.0)], 0),
    // 8: a high "tsee" 9 -> 7.9 kHz, falling away at the end.
    syl(0.12, &[(0.0, 9000.0, 0.0), (0.15, 8300.0, 1.0), (0.5, 8000.0, 0.9), (0.85, 7850.0, 0.6), (1.0, 7500.0, 0.0)], 0),
    // 9: an up-slur 2.3 -> 4.1 kHz.
    syl(0.09, &[(0.0, 2300.0, 0.0), (0.15, 2400.0, 0.6), (0.85, 4000.0, 1.0), (1.0, 4100.0, 0.0)], 0),
    // 10: a falling high whistle 8.6 -> 7.6 kHz.
    syl(0.15, &[(0.0, 8600.0, 0.0), (0.1, 8500.0, 0.9), (1.0, 7600.0, 0.0)], 0),
    // 11: a two-voiced S-curve, 6.5 -> 3.6 -> 6.9 kHz.
    duet(0.2, &[(0.0, 6500.0, 0.0), (0.15, 5000.0, 0.8), (0.35, 3600.0, 1.0), (0.7, 6200.0, 0.9), (0.85, 6900.0, 0.8), (1.0, 6300.0, 0.0)],
        &[(0.0, 5500.0, 0.0), (0.35, 3400.0, 0.5), (0.75, 5000.0, 0.5), (1.0, 4700.0, 0.0)], 0),
    // 12: a trill element near 4.6 kHz.
    syl(0.035, &[(0.0, 4600.0, 0.0), (0.3, 4800.0, 1.0), (1.0, 4000.0, 0.0)], 0),
    // 13: a low trill element near 3 kHz.
    syl(0.035, &[(0.0, 3200.0, 0.0), (0.4, 2800.0, 1.0), (1.0, 3100.0, 0.0)], 0),
    // 14: a sustained, slightly falling 3.3 -> 3.1 kHz note (671058 at 3.45 s and 21.1 s).
    syl(0.3, &[(0.0, 3400.0, 0.0), (0.08, 3350.0, 0.9), (0.5, 3200.0, 1.0), (0.9, 3100.0, 0.8), (1.0, 3050.0, 0.0)], 0),
    // 15: a buzzy 4.4 kHz note with a dip (671058 at 3.15 s).
    syl(0.2, &[(0.0, 4600.0, 0.0), (0.1, 4500.0, 0.9), (0.4, 4250.0, 1.0), (0.6, 4450.0, 0.9), (1.0, 4400.0, 0.0)], 2),
];

pub const ROBIN: Species = Species {
    name: "Robin",
    latin: "Erithacus rubecula",
    invented: false,
    syllables: ROBIN_SYL,
    timbres: &[
        tone([0.005, 0.001, 0.0]),
        Timbre { harm: [0.004, 0.0, 0.0], wobble: 2.0, ..Timbre::PURE },
        Timbre { harm: [0.005, 0.001, 0.0], am: (90.0, 0.4), wobble: 1.5, ..Timbre::PURE },
    ],
    phrases: &[
        once(&[(0, 0.0)], 0.03),
        once(&[(1, 0.0)], 0.02),
        once(&[(10, 0.0)], 0.02),
        rep(&[(8, 0.0)], (2, 3), (0.14, 0.14), 0.03),
        rep(&[(2, 0.0)], (2, 3), (0.1, 0.1), 0.02),
        once(&[(4, 0.0)], 0.03),
        rep(&[(7, 0.0)], (2, 4), (0.075, 0.07), 0.02),
        once(&[(11, 0.0)], 0.02),
        rep(&[(5, 0.0), (6, 0.06)], (2, 4), (0.13, 0.125), 0.02),
        rep(&[(9, 0.0)], (2, 3), (0.12, 0.115), 0.02),
        rep(&[(12, 0.0)], (4, 7), (0.045, 0.04), 0.02),
        rep(&[(13, 0.0)], (3, 6), (0.05, 0.05), 0.02),
        rep(&[(3, 0.0)], (1, 3), (0.11, 0.11), 0.02),
        once(&[(14, 0.0)], 0.03),
        once(&[(15, 0.0)], 0.02),
    ],
    songs: &[
        song(&[slot(&[0, 1, 2, 3], 0.9), slot(&[4, 5, 6, 7, 2, 14], 1.0), slot(&[13, 14, 5], 0.85), slot(&[4, 6, 7, 11, 12, 9], 0.9), slot(&[8, 9, 12, 6, 5, 1, 14], 0.8), slot(&[10, 11, 8, 3], 0.6)], (1, 1), 1.0),
        song(&[slot(&[3, 2, 1], 0.6), slot(&[9, 7, 4, 13], 1.0), slot(&[14, 5, 13], 0.85), slot(&[5, 12, 6], 0.7), slot(&[4, 7, 8, 0], 0.6), slot(&[11, 10], 0.8)], (1, 1), 0.6),
    ],
    alarm: &[],
    pause: (2.6, 0.35),
    alarm_pause: (1.0, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 0.0,
    var: Variation { pitch: 0.04, tempo: 0.08, note: 0.025, individual: 0.1 },
    micro: Micro { rate: 65.0, cents: 45.0, flutter: 3.40 },
    level: 0.75,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Common chaffinch (Fringilla coelebs)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 332851 (Travesia_Sonora) and 725328 (Sacha.Julien); 122618 (calls,
// not song) was not used. A song is 2-2.8 s: two or three trills, each lower than the
// last, then a flourish that sweeps 7-8.5 kHz down to 2 kHz and ends in a buzzy "kiss-yoo".
// Song type A (332851): fast down-sweeps 6.3 -> 3.5 kHz every 58 ms, a 7 -> 2.7 kHz bridge,
// rising hooks at 3.4-3.9 kHz every 75 ms, then three-note syllables every 125 ms. Type B
// (725328): a 5.7 kHz whistle, then trills at 100, 90 and 55 ms that fall from 5.3 to 2.5 kHz:
// the descending trill speeds up. A bird sings one type several times before switching. Songs
// every 8-10 s (pauses 5-8 s).

const CHAFFINCH_SYL: &[Syllable] = &[
    // Type A. 0: the first trill's down-sweep, 6.4 -> 3.5 kHz, ending in a slight curl.
    syl(0.032, &[(0.0, 6400.0, 0.0), (0.1, 6200.0, 0.9), (0.5, 4600.0, 1.0), (0.82, 3700.0, 0.7), (1.0, 3500.0, 0.0)], 0),
    // 1: the bridge, a long sweep 7 -> 2.7 kHz.
    syl(0.15, &[(0.0, 7000.0, 0.0), (0.08, 6800.0, 1.0), (0.4, 4400.0, 0.9), (0.8, 3100.0, 0.5), (1.0, 2700.0, 0.0)], 0),
    // 2: the hook of the second trill: a fast fall 5 -> 2.85 kHz that curls back up to 4 kHz.
    syl(0.045, &[(0.0, 5000.0, 0.0), (0.06, 4800.0, 0.7), (0.3, 3000.0, 1.0), (0.42, 2850.0, 0.95), (0.75, 3700.0, 0.9), (1.0, 4050.0, 0.0)], 0),
    // 3, 4: the third trill's unit: a thin tick near 4.7 kHz, then a j-shaped stroke 4.8 -> 3.35
    // kHz that turns up briefly and falls to 2.9 kHz.
    syl(0.009, &[(0.0, 4600.0, 0.0), (0.4, 4800.0, 0.6), (1.0, 4750.0, 0.0)], 0),
    syl(0.06, &[(0.0, 4800.0, 0.0), (0.1, 4700.0, 0.8), (0.45, 3500.0, 1.0), (0.6, 3350.0, 0.95), (0.75, 3550.0, 0.8), (1.0, 2900.0, 0.0)], 0),
    // 5-7: the flourish: a long sweep, a low hook and the buzzy end note.
    syl(0.14, &[(0.0, 7000.0, 0.0), (0.06, 6800.0, 0.9), (0.35, 4600.0, 1.0), (0.75, 3000.0, 0.9), (1.0, 2500.0, 0.0)], 0),
    syl(0.03, &[(0.0, 2450.0, 0.0), (0.3, 2700.0, 0.8), (0.7, 2750.0, 0.7), (1.0, 2400.0, 0.0)], 0),
    syl(0.18, &[(0.0, 5000.0, 0.0), (0.08, 4800.0, 0.9), (0.4, 3600.0, 1.0), (0.8, 2500.0, 0.7), (1.0, 2100.0, 0.0)], 1),
    // Type B (8-13).
    syl(0.3, &[(0.0, 5900.0, 0.0), (0.1, 5800.0, 0.8), (0.8, 5600.0, 0.7), (1.0, 5500.0, 0.0)], 0),
    duet(0.07, &[(0.0, 5400.0, 0.0), (0.2, 5300.0, 0.9), (0.7, 4800.0, 0.8), (1.0, 4500.0, 0.0)], &[(0.0, 4400.0, 0.0), (0.3, 4300.0, 0.5), (1.0, 4100.0, 0.0)], 0),
    syl(0.06, &[(0.0, 6500.0, 0.0), (0.15, 6200.0, 0.9), (0.6, 4500.0, 1.0), (0.85, 3900.0, 0.8), (1.0, 3800.0, 0.0)], 0),
    syl(0.04, &[(0.0, 4600.0, 0.0), (0.15, 4400.0, 1.0), (0.6, 3100.0, 0.85), (0.8, 2700.0, 0.6), (1.0, 2500.0, 0.0)], 0),
    syl(0.1, &[(0.0, 8500.0, 0.0), (0.1, 8000.0, 0.8), (0.5, 5500.0, 1.0), (1.0, 4000.0, 0.0)], 0),
    syl(0.1, &[(0.0, 2300.0, 0.0), (0.2, 2500.0, 0.8), (0.9, 3700.0, 1.0), (1.0, 3700.0, 0.0)], 0),
];

pub const CHAFFINCH: Species = Species {
    name: "Chaffinch",
    latin: "Fringilla coelebs",
    invented: false,
    syllables: CHAFFINCH_SYL,
    // A tone with a little breath: the recordings' strokes are smeared (mostly the wood's echo,
    // which `space/reverb` supplies), but each also carries a faint noise band.
    timbres: &[
        Timbre { harm: [0.016, 0.0055, 0.0], noise: 0.04, band: 0.25, ..Timbre::PURE },
        Timbre { harm: [0.03, 0.01, 0.0], am: (70.0, 0.75), noise: 0.1, ..Timbre::PURE },
    ],
    phrases: &[
        // Type A.
        Phrase { unit: &[(0, 0.0)], reps: (5, 7), period: (0.06, 0.056), drift: -0.05, swell: 1.0, gap: 0.02 },
        once(&[(1, 0.0)], 0.02),
        Phrase { unit: &[(2, 0.0)], reps: (4, 6), period: (0.077, 0.072), drift: -0.05, swell: 1.0, gap: 0.03 },
        Phrase { unit: &[(3, 0.0), (4, 0.012)], reps: (5, 7), period: (0.125, 0.115), drift: -0.06, swell: 1.1, gap: 0.02 },
        once(&[(5, 0.0), (6, 0.13), (7, 0.27)], 0.0),
        // Type B.
        once(&[(8, 0.0)], 0.04),
        Phrase { unit: &[(9, 0.0)], reps: (3, 5), period: (0.1, 0.1), drift: -0.05, swell: 1.0, gap: 0.0 },
        Phrase { unit: &[(10, 0.0)], reps: (6, 10), period: (0.095, 0.085), drift: -0.1, swell: 1.0, gap: 0.0 },
        Phrase { unit: &[(11, 0.0)], reps: (6, 10), period: (0.06, 0.052), drift: -0.08, swell: 1.1, gap: 0.02 },
        once(&[(12, 0.0), (13, 0.18), (7, 0.32)], 0.0),
    ],
    songs: &[
        song(&[slot(&[0], 1.0), slot(&[1], 0.8), slot(&[2], 1.0), slot(&[3], 1.0), slot(&[4], 1.0)], (1, 1), 1.0),
        song(&[slot(&[5], 0.6), slot(&[6], 1.0), slot(&[7], 1.0), slot(&[8], 1.0), slot(&[9], 1.0)], (1, 1), 1.0),
    ],
    alarm: &[],
    pause: (7.0, 0.3),
    alarm_pause: (1.0, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 0.8,
    var: Variation { pitch: 0.025, tempo: 0.05, note: 0.01, individual: 0.08 },
    micro: Micro { rate: 95.0, cents: 30.0, flutter: 3.20 },
    level: 0.9,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Great tit (Parus major)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 418102 (straget), 389404 (BeeProductive) and 346066
// (pulswelle), with 790795 and 849589. Two-note units repeated 3-9 times ("tea-cher"),
// near-pure tones. Type A (418102): a 2.9-3.0 kHz note, then a 3.95 kHz note 0.15 s later,
// every 0.44 s. Type B (389404): a sharp up-sweep into 4.2-4.5 kHz, then a 4.9 kHz note
// 0.145 s later, every 0.33 s. Type C (346066): 4.2, 4.2, 5.7 and 7 kHz notes every 0.55 s,
// later on 3.7 kHz. Songs 1.5-4 s, pauses 2-5 s; a bird repeats one type through a bout. The
// alarm is a churring rattle near 4 kHz, about 22 pulses a second (418102 at 13.8 s).

const GREAT_TIT_SYL: &[Syllable] = &[
    syl(0.125, &[(0.0, 2750.0, 0.0), (0.15, 2900.0, 0.75), (0.5, 3000.0, 0.85), (0.85, 2950.0, 0.55), (1.0, 2850.0, 0.0)], 0),
    syl(0.09, &[(0.0, 3900.0, 0.0), (0.15, 3950.0, 1.0), (0.7, 3990.0, 0.95), (1.0, 3960.0, 0.0)], 0),
    syl(0.11, &[(0.0, 3500.0, 0.0), (0.08, 4500.0, 1.0), (0.16, 4250.0, 0.9), (0.6, 4200.0, 0.85), (1.0, 4150.0, 0.0)], 0),
    syl(0.06, &[(0.0, 4900.0, 0.0), (0.2, 4950.0, 0.7), (0.8, 4900.0, 0.6), (1.0, 4880.0, 0.0)], 0),
    syl(0.12, &[(0.0, 4150.0, 0.0), (0.15, 4200.0, 1.0), (0.85, 4260.0, 0.9), (1.0, 4250.0, 0.0)], 0),
    syl(0.08, &[(0.0, 6000.0, 0.0), (0.15, 5800.0, 0.6), (1.0, 5550.0, 0.0)], 0),
    syl(0.09, &[(0.0, 7300.0, 0.0), (0.1, 7100.0, 0.45), (0.6, 6700.0, 0.35), (1.0, 6550.0, 0.0)], 0),
    syl(0.12, &[(0.0, 3760.0, 0.0), (0.15, 3700.0, 1.0), (0.85, 3700.0, 0.85), (1.0, 3690.0, 0.0)], 0),
    // 8: the churring alarm rattle.
    syl(0.9, &[(0.0, 4000.0, 0.0), (0.05, 4100.0, 0.8), (0.9, 3900.0, 0.7), (1.0, 3800.0, 0.0)], 1),
];

pub const GREAT_TIT: Species = Species {
    name: "Great tit",
    latin: "Parus major",
    invented: false,
    syllables: GREAT_TIT_SYL,
    timbres: &[tone([0.012, 0.002, 0.0]), Timbre { harm: [0.05, 0.015, 0.0], am: (22.0, 0.9), noise: 0.6, band: 0.3, rough: 0.2, ..Timbre::PURE }],
    phrases: &[
        rep(&[(0, 0.0), (1, 0.15)], (3, 6), (0.44, 0.43), 0.0),
        rep(&[(2, 0.0), (3, 0.145)], (5, 9), (0.33, 0.32), 0.0),
        rep(&[(4, 0.0), (4, 0.17), (5, 0.35), (6, 0.4)], (2, 4), (0.55, 0.55), 0.0),
        rep(&[(7, 0.0), (7, 0.17), (5, 0.35), (6, 0.4)], (1, 3), (0.55, 0.55), 0.0),
        rep(&[(8, 0.0)], (1, 3), (1.3, 1.3), 0.0),
    ],
    songs: &[
        song(&[slot(&[0], 1.0)], (1, 1), 1.0),
        song(&[slot(&[1], 1.0)], (1, 1), 0.8),
        song(&[slot(&[2], 1.0), slot(&[3], 0.7)], (1, 1), 0.6),
    ],
    alarm: &[song(&[slot(&[4], 1.0)], (1, 1), 1.0)],
    pause: (3.0, 0.35),
    alarm_pause: (1.5, 0.4),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 0.85,
    var: Variation { pitch: 0.02, tempo: 0.06, note: 0.008, individual: 0.08 },
    micro: Micro { rate: 55.0, cents: 10.0, flutter: 2.00 },
    level: 0.9,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Common cuckoo (Cuculus canorus)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 688844 (Elanor1995) and 377213 (jaromarsalek); 517779-81 (hard,
// synthetic-sounding notes) and 274771 (noisy, unclear) were not used. 94 syllables: the
// "cuck" glides up 600 -> 675 Hz and sits near 655 Hz for 100-190 ms; the "oo" starts 0.285 s
// (sd 0.01) later at 505-520 Hz, a major third (4.2 semitones) lower and 4 dB quieter. Calls
// repeat every 1.05-1.2 s (a little slower at the start of a bout), 6-20 at a time, then
// 10-15 s of silence. Now and then a "cuck-oo-oo" (the third note 0.15 s after the second),
// which excited birds give more often.

const CUCKOO_SYL: &[Syllable] = &[
    syl(0.21, &[(0.0, 600.0, 0.0), (0.06, 660.0, 0.7), (0.15, 675.0, 1.0), (0.34, 660.0, 0.7), (0.57, 652.0, 0.35), (0.8, 648.0, 0.1), (1.0, 645.0, 0.0)], 0),
    syl(0.18, &[(0.0, 503.0, 0.0), (0.25, 520.0, 0.65), (0.6, 515.0, 0.45), (1.0, 505.0, 0.0)], 0),
];

pub const CUCKOO: Species = Species {
    name: "Cuckoo",
    latin: "Cuculus canorus",
    invented: false,
    syllables: CUCKOO_SYL,
    timbres: &[tone([0.02, 0.003, 0.0])],
    phrases: &[
        Phrase { unit: &[(0, 0.0), (1, 0.285)], reps: (3, 9), period: (1.2, 1.07), drift: 0.0, swell: 1.0, gap: 0.6 },
        once(&[(0, 0.0), (1, 0.285), (1, 0.44)], 0.62),
        rep(&[(0, 0.0), (1, 0.285)], (2, 8), (1.08, 1.08), 0.0),
    ],
    songs: &[song(&[slot(&[0], 1.0), slot(&[1], 0.25), slot(&[2], 0.7)], (1, 1), 1.0)],
    alarm: &[song(&[slot(&[1], 1.0), slot(&[1], 0.6)], (1, 1), 1.0)],
    pause: (11.0, 0.35),
    alarm_pause: (1.5, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 1.0,
    var: Variation { pitch: 0.015, tempo: 0.06, note: 0.006, individual: 0.12 },
    micro: Micro { rate: 40.0, cents: 12.0, flutter: 0.80 },
    level: 0.9,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Common wood pigeon (Columba palumbus)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 814890 (Sadiquecat) and 607224 (D4XX); 517793-95 (hard onsets,
// a steady 0.65 kHz tone, no five-note phrase) were not used. The five-note phrase "coo-COOO-coo,
// coo-coo": notes of 0.30, 0.58, 0.42, 0.24 and 0.38 s starting at 0, 0.48, 1.16, 1.80 and
// 2.12 s, hoarse, near 330-560 Hz (fundamental about 440 Hz, rising at each onset). The phrase
// comes every 2.8 s, 3-5 times, often closed by one more short "coo"; then long silence.

const WOOD_PIGEON_SYL: &[Syllable] = &[
    syl(0.38, &[(0.0, 340.0, 0.0), (0.15, 420.0, 0.7), (0.4, 445.0, 1.0), (0.85, 440.0, 0.8), (1.0, 420.0, 0.0)], 0),
    syl(0.62, &[(0.0, 360.0, 0.0), (0.08, 430.0, 0.8), (0.3, 460.0, 1.0), (0.8, 450.0, 0.9), (1.0, 420.0, 0.0)], 0),
    syl(0.46, &[(0.0, 480.0, 0.0), (0.1, 470.0, 0.9), (0.7, 430.0, 0.8), (1.0, 400.0, 0.0)], 0),
    syl(0.26, &[(0.0, 330.0, 0.0), (0.25, 400.0, 0.7), (0.6, 420.0, 0.8), (1.0, 410.0, 0.0)], 0),
    syl(0.4, &[(0.0, 490.0, 0.0), (0.1, 480.0, 0.9), (0.6, 440.0, 0.85), (1.0, 400.0, 0.0)], 0),
];

pub const WOOD_PIGEON: Species = Species {
    name: "Wood pigeon",
    latin: "Columba palumbus",
    invented: false,
    syllables: WOOD_PIGEON_SYL,
    timbres: &[Timbre { harm: [0.035, 0.001, 0.0], noise: 0.02, rough: 0.1, ..Timbre::PURE }],
    phrases: &[
        rep(&[(0, 0.0), (1, 0.48), (2, 1.16), (3, 1.80), (4, 2.12)], (3, 5), (2.8, 2.75), 0.3),
        once(&[(3, 0.0)], 0.0),
    ],
    songs: &[song(&[slot(&[0], 1.0), slot(&[1], 0.75)], (1, 1), 1.0)],
    alarm: &[],
    pause: (14.0, 0.4),
    alarm_pause: (2.0, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 1.0,
    var: Variation { pitch: 0.02, tempo: 0.05, note: 0.01, individual: 0.1 },
    micro: Micro { rate: 50.0, cents: 90.0, flutter: 1.80 },
    level: 0.7,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Eurasian collared dove (Streptopelia decaocto)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 610563 (randomthoughts7), with 516091 (Karola3206) and 607243.
// "coo-COOO-coo": 0.28 s (rising 500 -> 548 Hz), 0.42 s (flat 545 Hz, the long one, starting
// 0.34 s after the first) and 0.30 s (falling 550 -> 500 Hz, at 0.99 s); the triplet repeats
// every 1.75 s (sd 0.02), 3-10 times. Pure, soft tones.

const COLLARED_DOVE_SYL: &[Syllable] = &[
    syl(0.28, &[(0.0, 495.0, 0.0), (0.1, 520.0, 0.8), (0.3, 548.0, 1.0), (0.75, 545.0, 0.9), (1.0, 520.0, 0.0)], 0),
    syl(0.42, &[(0.0, 500.0, 0.0), (0.08, 535.0, 0.85), (0.3, 546.0, 0.9), (0.8, 538.0, 0.8), (1.0, 510.0, 0.0)], 0),
    syl(0.30, &[(0.0, 515.0, 0.0), (0.08, 545.0, 0.95), (0.2, 551.0, 1.0), (0.8, 500.0, 0.6), (1.0, 490.0, 0.0)], 0),
];

pub const COLLARED_DOVE: Species = Species {
    name: "Collared dove",
    latin: "Streptopelia decaocto",
    invented: false,
    syllables: COLLARED_DOVE_SYL,
    timbres: &[tone([0.03, 0.0004, 0.0])],
    phrases: &[rep(&[(0, 0.0), (1, 0.34), (2, 0.99)], (3, 10), (1.75, 1.75), 0.0)],
    songs: &[song(&[slot(&[0], 1.0)], (1, 1), 1.0)],
    alarm: &[],
    pause: (12.0, 0.5),
    alarm_pause: (2.0, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 1.0,
    var: Variation { pitch: 0.015, tempo: 0.04, note: 0.005, individual: 0.08 },
    micro: Micro { rate: 45.0, cents: 9.0, flutter: 0.30 },
    level: 0.6,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Carrion crow (Corvus corone)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 181088 (cfrooos), 741366 (Mish7913, hooded crow, the same
// species group), 476139 (cupido-1) and 577265. A caw is 0.3-0.45 s of harsh, broadband sound:
// a harmonic series on a 270-400 Hz fundamental that rises a little and falls, rough (jittery,
// noisy), with energy to 6-7 kHz and formant peaks near 1.7 and 2.8 kHz. Caws come in series
// of 2-8, every 0.53-0.75 s; series are 5-15 s apart. Agitated crows caw faster.

const CROW_SYL: &[Syllable] = &[
    syl(0.33, &[(0.0, 330.0, 0.0), (0.08, 380.0, 0.9), (0.35, 400.0, 1.0), (0.75, 370.0, 0.85), (1.0, 330.0, 0.0)], 0),
    syl(0.42, &[(0.0, 270.0, 0.0), (0.06, 300.0, 0.9), (0.5, 310.0, 1.0), (0.85, 290.0, 0.8), (1.0, 260.0, 0.0)], 0),
];

pub const CROW: Species = Species {
    name: "Crow",
    latin: "Corvus corone",
    invented: false,
    syllables: CROW_SYL,
    timbres: &[Timbre {
        noise: 0.22,
        rough: 0.38,
        formants: &[(1750.0, 600.0, 1.0), (1150.0, 500.0, 0.5), (2600.0, 600.0, 0.2), (5100.0, 900.0, 0.05)],
        ..Timbre::PURE
    }],
    phrases: &[
        Phrase { unit: &[(0, 0.0)], reps: (3, 7), period: (0.58, 0.53), drift: -0.03, swell: 0.9, gap: 0.0 },
        Phrase { unit: &[(1, 0.0)], reps: (2, 5), period: (0.72, 0.68), drift: -0.03, swell: 0.9, gap: 0.0 },
        Phrase { unit: &[(0, 0.0)], reps: (5, 10), period: (0.4, 0.34), drift: 0.0, swell: 1.0, gap: 0.0 },
    ],
    songs: &[song(&[slot(&[0, 1], 1.0)], (1, 1), 1.0)],
    alarm: &[song(&[slot(&[2], 1.0)], (1, 1), 1.0)],
    pause: (8.0, 0.5),
    alarm_pause: (1.5, 0.4),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 0.6,
    var: Variation { pitch: 0.03, tempo: 0.07, note: 0.02, individual: 0.15 },
    micro: Micro { rate: 50.0, cents: 40.0, flutter: 2.40 },
    level: 0.62,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Tawny owl (Strix aluco)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 745208 (Patrick_Corra), 450214 (Marcuspepsi), 655380
// (faxfaxfax), 688869 (Elanor1995) and 735744 (Vrymaa, female). The male's hoot: a long
// "hoooo" (0.7-1.25 s, 750 -> 920 -> 760 Hz), 3.2-4.4 s of silence, a short "hu" (0.11-0.3 s,
// ~800 Hz), then 0.71 s (sd 0.005) after it a 1.3-1.7 s tremolo "hu-hu-hoooo" that wavers
// 790-930 Hz about ten times a second for its first half, then holds near 880 Hz. The
// fundamental sits at 650-960 Hz across the five birds, with almost no harmonics. The female's
// "ke-wick" (the excited call) is harsh: 0.3 s, rising 780 -> 1100 Hz, strongest near 2.2 kHz.

const TAWNY_OWL_SYL: &[Syllable] = &[
    syl(0.9, &[(0.0, 760.0, 0.0), (0.06, 890.0, 0.8), (0.25, 920.0, 1.0), (0.55, 900.0, 0.85), (0.8, 840.0, 0.5), (1.0, 760.0, 0.0)], 0),
    syl(0.15, &[(0.0, 830.0, 0.0), (0.25, 800.0, 0.8), (0.8, 780.0, 0.5), (1.0, 770.0, 0.0)], 0),
    syl(1.55, &[
        (0.0, 820.0, 0.0), (0.03, 920.0, 0.6), (0.065, 840.0, 0.7), (0.1, 925.0, 0.85), (0.135, 835.0, 0.75), (0.17, 925.0, 0.9), (0.2, 845.0, 0.85),
        (0.235, 915.0, 1.0), (0.27, 850.0, 0.85), (0.31, 900.0, 1.0), (0.36, 880.0, 1.0), (0.6, 885.0, 1.0), (0.85, 870.0, 0.7), (1.0, 840.0, 0.0),
    ], 1),
    syl(0.32, &[(0.0, 780.0, 0.0), (0.15, 850.0, 0.8), (0.35, 1080.0, 1.0), (0.6, 1120.0, 0.85), (1.0, 1050.0, 0.0)], 2),
];

pub const TAWNY_OWL: Species = Species {
    name: "Tawny owl",
    latin: "Strix aluco",
    invented: false,
    syllables: TAWNY_OWL_SYL,
    timbres: &[
        Timbre { harm: [0.03, 0.0007, 0.0], noise: 0.03, ..Timbre::PURE },
        Timbre { harm: [0.03, 0.0007, 0.0], noise: 0.03, wobble: 1.5, ..Timbre::PURE },
        Timbre { noise: 0.3, rough: 0.4, formants: &[(2200.0, 700.0, 1.0), (1000.0, 400.0, 0.7), (3100.0, 800.0, 0.5)], ..Timbre::PURE },
    ],
    phrases: &[
        once(&[(0, 0.0)], 3.4),
        once(&[(1, 0.0), (2, 0.71)], 0.0),
        Phrase { unit: &[(3, 0.0)], reps: (2, 5), period: (1.0, 0.8), drift: 0.0, swell: 1.0, gap: 0.0 },
    ],
    songs: &[song(&[slot(&[0], 1.0), slot(&[1], 1.0)], (1, 1), 1.0)],
    alarm: &[song(&[slot(&[2], 1.0)], (1, 1), 1.0)],
    pause: (5.0, 0.4),
    alarm_pause: (2.0, 0.4),
    bout: (2, 6),
    bout_pause: (30.0, 0.5),
    repeat: 1.0,
    var: Variation { pitch: 0.015, tempo: 0.06, note: 0.005, individual: 0.12 },
    micro: Micro { rate: 40.0, cents: 14.0, flutter: 1.60 },
    level: 1.0,
    hours: Hours::Night,
};

// ---------------------------------------------------------------------------------------------
// House sparrow (Passer domesticus)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 383160 (BeeProductive) and 404663 (straget), with 670176 and
// 737211. The "chilp": 0.1-0.13 s, harmonic-rich (strong 2nd and 3rd harmonics) and a bit
// noisy, an inverted V from about 2.6 kHz up to 4.7 kHz and down to 2.8 kHz; the "cheep"
// falls from 6.5 to 2.9 kHz. A male chirps on and on, one every 0.3-1.5 s; excited birds
// chatter (a rattle of short chirps every 90 ms).

const HOUSE_SPARROW_SYL: &[Syllable] = &[
    syl(0.11, &[(0.0, 5600.0, 0.0), (0.12, 6000.0, 0.8), (0.4, 4700.0, 1.0), (0.6, 4900.0, 0.8), (0.85, 3600.0, 0.6), (1.0, 3300.0, 0.0)], 0),
    syl(0.13, &[(0.0, 6500.0, 0.0), (0.1, 6000.0, 0.8), (0.35, 4700.0, 1.0), (0.75, 3300.0, 0.8), (1.0, 2900.0, 0.0)], 0),
    syl(0.05, &[(0.0, 3000.0, 0.0), (0.3, 4200.0, 0.8), (0.7, 4000.0, 0.7), (1.0, 3200.0, 0.0)], 0),
];

pub const HOUSE_SPARROW: Species = Species {
    name: "House sparrow",
    latin: "Passer domesticus",
    invented: false,
    syllables: HOUSE_SPARROW_SYL,
    timbres: &[Timbre { harm: [0.03, 0.001, 0.0], noise: 0.2, rough: 0.25, ..Timbre::PURE }],
    phrases: &[
        once(&[(0, 0.0)], 0.07),
        once(&[(1, 0.0)], 0.1),
        once(&[(2, 0.0), (0, 0.08)], 0.12),
        once(&[(1, 0.0)], 0.35),
        rep(&[(2, 0.0)], (6, 12), (0.09, 0.085), 0.0),
    ],
    songs: &[song(&[slot(&[0, 1, 2, 3], 1.0)], (2, 7), 1.0)],
    alarm: &[song(&[slot(&[4], 1.0)], (1, 1), 1.0)],
    pause: (1.5, 0.5),
    alarm_pause: (0.8, 0.4),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 1.0,
    var: Variation { pitch: 0.03, tempo: 0.1, note: 0.03, individual: 0.1 },
    micro: Micro { rate: 120.0, cents: 60.0, flutter: 3.00 },
    level: 0.7,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Herring gull (Larus argentatus)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 538016 and 538015 (Canardo55), with 818829 and 73497. The long
// call ("laugh"): a 0.4 s opening note near 1.2 kHz, then 8-14 "kyow" notes of 0.19 s every
// 0.33 -> 0.30 s, each arching 850 -> 1180 -> 880 Hz; the mew: one 1.2 s note at 950 Hz. A
// harmonic-rich, nasal voice: the 2nd harmonic (near 1.9 kHz) is stronger than the fundamental,
// with energy to 6 kHz.

const HERRING_GULL_SYL: &[Syllable] = &[
    syl(1.2, &[(0.0, 1150.0, 0.0), (0.03, 1000.0, 0.9), (0.2, 960.0, 1.0), (0.8, 940.0, 0.95), (1.0, 900.0, 0.0)], 0),
    syl(0.4, &[(0.0, 1050.0, 0.0), (0.08, 1250.0, 0.9), (0.5, 1200.0, 1.0), (1.0, 1150.0, 0.0)], 1),
    syl(0.29, &[(0.0, 800.0, 0.0), (0.07, 1050.0, 0.8), (0.18, 1180.0, 1.0), (0.45, 1060.0, 0.8), (0.75, 920.0, 0.35), (1.0, 850.0, 0.0)], 1),
];

pub const HERRING_GULL: Species = Species {
    name: "Herring gull",
    latin: "Larus argentatus",
    invented: false,
    syllables: HERRING_GULL_SYL,
    timbres: &[Timbre {
        noise: 0.08,
        rough: 0.15,
        formants: &[(1900.0, 800.0, 1.0), (1000.0, 300.0, 0.45), (3600.0, 900.0, 0.35), (5500.0, 1200.0, 0.08)],
        ..Timbre::PURE
    }, Timbre {
        noise: 0.08,
        rough: 0.15,
        formants: &[(1100.0, 500.0, 1.0), (2300.0, 900.0, 0.15), (3500.0, 900.0, 0.08), (5000.0, 1200.0, 0.03)],
        ..Timbre::PURE
    }],
    phrases: &[
        once(&[(1, 0.0)], 0.12),
        Phrase { unit: &[(2, 0.0)], reps: (8, 14), period: (0.33, 0.3), drift: -0.05, swell: 0.8, gap: 0.0 },
        once(&[(0, 0.0)], 0.0),
        Phrase { unit: &[(2, 0.0)], reps: (3, 6), period: (0.25, 0.23), drift: 0.0, swell: 1.0, gap: 0.0 },
    ],
    songs: &[song(&[slot(&[0], 1.0), slot(&[1], 1.0)], (1, 1), 1.0), song(&[slot(&[2], 1.0)], (1, 2), 0.6)],
    alarm: &[song(&[slot(&[3], 1.0)], (1, 1), 1.0)],
    pause: (8.0, 0.6),
    alarm_pause: (1.5, 0.4),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 0.3,
    var: Variation { pitch: 0.03, tempo: 0.06, note: 0.015, individual: 0.12 },
    micro: Micro { rate: 50.0, cents: 14.0, flutter: 1.40 },
    level: 1.0,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Common nightingale (Luscinia megarhynchos)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 521035 (smand), with 395101 (Uocca), 456208 (tsidilin) and 516160
// (Lupsi). Songs of 2-4 s with 1-3 s between them, each built from one to three of a large
// repertoire of strophes, every song different: the slow crescendo "piu piu piu" (a rise from
// 0.9 to 2 kHz held for 0.3 s, 3-5 times every 0.5 s, louder each time), whistles answered by
// a broadband click, fast "jug-jug" series of clicks (sweeps from 9 to 1.5 kHz in 12 ms, 20 a
// second), "chuck" pairs over a 1 kHz blip every 0.3 s, flat 2.5 kHz notes with a rising
// flick, a thin high 7.3 -> 6.3 kHz whistle, harsh broadband rattles (0.1 s, 3-5 times every
// 0.27 s), and a closing down-sweep from 8.7 to 3.5 kHz. The loudest of these birds.

const NIGHTINGALE_SYL: &[Syllable] = &[
    // 0: the crescendo note: up from 0.9 kHz, held at 2 kHz, harmonics at 4 and 6 kHz.
    syl(0.36, &[(0.0, 900.0, 0.0), (0.12, 1850.0, 0.36), (0.2, 1980.0, 0.45), (0.6, 1960.0, 0.27), (1.0, 1940.0, 0.0)], 0),
    // 1: the click: a sweep from 9 to 1.5 kHz in 12 ms.
    syl(0.014, &[(0.0, 9000.0, 0.0), (0.15, 7500.0, 1.0), (0.6, 3500.0, 0.8), (1.0, 1500.0, 0.0)], 1),
    // 2: "chuck" stroke 4.2 -> 1.5 kHz with a noisy tail.
    syl(0.02, &[(0.0, 4200.0, 0.0), (0.15, 3800.0, 1.0), (0.7, 1900.0, 0.8), (1.0, 1500.0, 0.0)], 1),
    // 3: low blip near 1 kHz.
    syl(0.025, &[(0.0, 900.0, 0.0), (0.3, 1050.0, 0.6), (1.0, 950.0, 0.0)], 0),
    // 4: flat 2.5 kHz note.
    syl(0.15, &[(0.0, 2600.0, 0.0), (0.1, 2550.0, 0.9), (0.9, 2500.0, 0.8), (1.0, 2500.0, 0.0)], 0),
    // 5: rising flick 2.5 -> 4.3 kHz.
    syl(0.07, &[(0.0, 2500.0, 0.0), (0.2, 2700.0, 0.9), (1.0, 4300.0, 0.0)], 0),
    // 6: high thin whistle 7.3 -> 6.3 kHz with a wobble.
    syl(0.3, &[(0.0, 7300.0, 0.0), (0.1, 7000.0, 0.7), (0.4, 6300.0, 0.7), (0.65, 6600.0, 0.6), (1.0, 6400.0, 0.0)], 0),
    // 7: harsh rattle burst.
    syl(0.1, &[(0.0, 700.0, 0.0), (0.1, 720.0, 1.0), (0.85, 680.0, 0.9), (1.0, 650.0, 0.0)], 2),
    // 8: the closing down-sweep 8.7 -> 3.5 kHz.
    syl(0.12, &[(0.0, 8700.0, 0.0), (0.1, 8400.0, 1.0), (0.6, 5000.0, 0.8), (1.0, 3500.0, 0.0)], 0),
    // 9: soft "tuk": short, broad, 1.5-4.5 kHz.
    syl(0.012, &[(0.0, 4500.0, 0.0), (0.3, 3200.0, 0.7), (1.0, 1600.0, 0.0)], 1),
];

pub const NIGHTINGALE: Species = Species {
    name: "Nightingale",
    latin: "Luscinia megarhynchos",
    invented: false,
    syllables: NIGHTINGALE_SYL,
    timbres: &[
        tone([0.008, 0.002, 0.0]),
        Timbre { harm: [0.02, 0.004, 0.0], noise: 0.45, ..Timbre::PURE },
        Timbre { noise: 0.5, rough: 0.3, am: (75.0, 0.7), formants: &[(2500.0, 1500.0, 1.0), (4500.0, 2000.0, 0.8), (7000.0, 2500.0, 0.5)], ..Timbre::PURE },
    ],
    phrases: &[
        Phrase { unit: &[(0, 0.0)], reps: (3, 5), period: (0.52, 0.5), drift: 0.0, swell: 2.2, gap: 0.08 },
        rep(&[(0, 0.0), (1, 0.3)], (2, 4), (0.53, 0.53), 0.05),
        Phrase { unit: &[(1, 0.0)], reps: (6, 14), period: (0.056, 0.05), drift: 0.0, swell: 1.0, gap: 0.05 },
        rep(&[(2, 0.0), (2, 0.04), (3, 0.09)], (4, 7), (0.3, 0.29), 0.05),
        rep(&[(4, 0.0), (5, 0.16)], (2, 3), (0.6, 0.6), 0.05),
        once(&[(6, 0.0)], 0.1),
        rep(&[(7, 0.0)], (3, 5), (0.27, 0.26), 0.05),
        once(&[(8, 0.0)], 0.0),
        rep(&[(9, 0.0)], (5, 8), (0.12, 0.12), 0.05),
    ],
    songs: &[song(&[slot(&[8, 5, 4], 0.5), slot(&[0, 1, 3, 4, 6, 0, 4], 1.0), slot(&[2, 3, 6, 1, 5, 4], 0.6), slot(&[7, 5], 0.6)], (1, 1), 1.0)],
    alarm: &[],
    pause: (2.0, 0.4),
    alarm_pause: (1.0, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 0.0,
    var: Variation { pitch: 0.03, tempo: 0.06, note: 0.015, individual: 0.06 },
    micro: Micro { rate: 55.0, cents: 15.0, flutter: 2.40 },
    level: 0.8,
    hours: Hours::Both,
};

// ---------------------------------------------------------------------------------------------
// Eurasian skylark (Alauda arvensis)
// ---------------------------------------------------------------------------------------------
// Transcribed from Freesound 387426 (Kinoton, a single bird), with 399221 (Veridiansunrise),
// 277149 and 244357. One unbroken warble for tens of seconds to minutes: 293 syllables in 30 s,
// a syllable every 70 ms (median onset interval), with no pause longer than 0.6 s. Short motifs
// are repeated 2-4 times and then dropped: fast down-strokes from 7 to 2.5 kHz, arches from
// 2.2 up to 4.7 kHz and back, thin arches near 9 kHz, buzzy two-voiced warbles at 3 and 5 kHz
// (frequency-modulated about 60 times a second), low rattles alternating 2.8/3.2 kHz, and
// slides down onto a 2.6 kHz plateau. Most energy 2.5-6 kHz.

const SKYLARK_SYL: &[Syllable] = &[
    // 0: a down-stroke 7 -> 2.5 kHz.
    syl(0.016, &[(0.0, 6000.0, 0.0), (0.15, 5600.0, 1.0), (0.7, 3600.0, 0.8), (1.0, 3000.0, 0.0)], 0),
    // 1: an arch 2.2 -> 4.7 -> 3.6 kHz.
    syl(0.14, &[(0.0, 2200.0, 0.0), (0.15, 2600.0, 0.7), (0.4, 4100.0, 0.95), (0.6, 4650.0, 1.0), (0.8, 4350.0, 0.8), (1.0, 3600.0, 0.0)], 0),
    // 2: a short high chirp at 5.5 kHz.
    syl(0.03, &[(0.0, 5300.0, 0.0), (0.3, 5600.0, 0.9), (1.0, 5400.0, 0.0)], 0),
    // 3: a slide down onto a 2.6 kHz plateau.
    syl(0.15, &[(0.0, 3800.0, 0.0), (0.1, 3600.0, 0.9), (0.2, 3050.0, 0.95), (0.3, 2760.0, 1.0), (0.45, 2640.0, 0.95), (0.9, 2600.0, 0.8), (1.0, 2550.0, 0.0)], 0),
    // 4: a buzzy two-voiced warble.
    duet(0.15, &[(0.0, 5200.0, 0.0), (0.1, 5000.0, 0.9), (0.9, 4600.0, 0.8), (1.0, 4500.0, 0.0)], &[(0.0, 3000.0, 0.0), (0.1, 3100.0, 0.6), (1.0, 2800.0, 0.0)], 1),
    // 5: a thin arch near 9 kHz.
    syl(0.08, &[(0.0, 7600.0, 0.0), (0.25, 8650.0, 0.5), (0.45, 9000.0, 0.6), (0.65, 8700.0, 0.5), (1.0, 7600.0, 0.0)], 0),
    // 6: a short 3.3 kHz note.
    syl(0.03, &[(0.0, 3600.0, 0.0), (0.2, 3500.0, 0.8), (1.0, 3400.0, 0.0)], 0),
    // 7: a quick up-stroke 2.4 -> 5 kHz.
    syl(0.025, &[(0.0, 2400.0, 0.0), (0.3, 3000.0, 0.8), (1.0, 5000.0, 0.0)], 0),
    // 8: a trill element at 4.6 kHz.
    syl(0.019, &[(0.0, 4800.0, 0.0), (0.4, 4650.0, 1.0), (1.0, 4300.0, 0.0)], 0),
    // 9: a low rattle 2.8/3.2 kHz.
    syl(0.12, &[(0.0, 3200.0, 0.0), (0.1, 3000.0, 0.9), (0.9, 2900.0, 0.8), (1.0, 2850.0, 0.0)], 2),
];

pub const SKYLARK: Species = Species {
    name: "Skylark",
    latin: "Alauda arvensis",
    invented: false,
    syllables: SKYLARK_SYL,
    timbres: &[
        tone([0.008, 0.0001, 0.0]),
        Timbre { harm: [0.008, 0.0, 0.0], am: (65.0, 0.7), wobble: 2.5, ..Timbre::PURE },
        Timbre { harm: [0.008, 0.0001, 0.0], am: (30.0, 0.8), wobble: 2.5, ..Timbre::PURE },
    ],
    phrases: &[
        rep(&[(0, 0.0)], (3, 6), (0.075, 0.07), 0.06),
        rep(&[(0, 0.0), (5, 0.025), (1, 0.06)], (2, 4), (0.3, 0.29), 0.04),
        rep(&[(4, 0.0)], (1, 3), (0.22, 0.22), 0.06),
        rep(&[(3, 0.0)], (2, 4), (0.23, 0.22), 0.04),
        rep(&[(8, 0.0), (7, 0.03)], (4, 8), (0.08, 0.075), 0.06),
        rep(&[(9, 0.0)], (1, 2), (0.15, 0.15), 0.04),
        rep(&[(2, 0.0), (6, 0.05)], (2, 4), (0.13, 0.13), 0.06),
        rep(&[(0, 0.0), (0, 0.035), (3, 0.08)], (2, 3), (0.3, 0.3), 0.04),
        // 8, 9: short notes and trill elements at 3-5 kHz, then a breath (0.5-0.9 s).
        rep(&[(6, 0.0), (8, 0.06), (2, 0.1)], (2, 4), (0.17, 0.16), 0.06),
        once(&[(8, 0.0)], 0.7),
    ],
    songs: &[song(&[slot(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 8], 1.0), slot(&[9], 0.07)], (25, 80), 1.0)],
    alarm: &[],
    pause: (12.0, 0.5),
    alarm_pause: (1.0, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 1.0,
    var: Variation { pitch: 0.02, tempo: 0.05, note: 0.04, individual: 0.08 },
    micro: Micro { rate: 85.0, cents: 10.0, flutter: 2.00 },
    level: 0.8,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Great spotted woodpecker (Dendrocopos major), drumming
// ---------------------------------------------------------------------------------------------
// Measured on Freesound 428146 (naturenotesuk) and 867542 (zachrau): a roll of 0.55-0.65 s,
// about 20 strikes that speed up from 30-35 ms apart to 22-27 ms, alternating loud and soft
// (about 8 dB: the beak's rebound), with the last third fading by 20 dB. Rolls come every 6-9 s.
// The knock rings at the branch's modes, 1.03 kHz strongest with 0.69, 1.32 and 0.28 kHz
// (it depends on the tree: the second bird's branch rings at 0.8-1.5 kHz). Not song: each
// syllable strikes a modal bank tuned to its first keypoint.

const WOODPECKER_SYL: &[Syllable] = &[
    syl(0.02, &[(0.0, 1030.0, 1.0)], 0),
    syl(0.02, &[(0.0, 1030.0, 0.4)], 0),
];

pub const WOODPECKER: Species = Species {
    name: "Woodpecker",
    latin: "Dendrocopos major",
    invented: false,
    syllables: WOODPECKER_SYL,
    timbres: &[Timbre { knock: true, ..Timbre::PURE }],
    phrases: &[
        Phrase { unit: &[(0, 0.0), (1, 0.029)], reps: (7, 10), period: (0.068, 0.048), drift: 0.0, swell: 1.0, gap: 0.0 },
        Phrase { unit: &[(0, 0.0), (1, 0.023)], reps: (2, 4), period: (0.046, 0.046), drift: 0.0, swell: 0.12, gap: 0.0 },
    ],
    songs: &[song(&[slot(&[0], 1.0), slot(&[1], 1.0)], (1, 1), 1.0)],
    alarm: &[],
    pause: (7.0, 0.25),
    alarm_pause: (1.0, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 1.0,
    var: Variation { pitch: 0.01, tempo: 0.05, note: 0.005, individual: 0.25 },
    micro: Micro { rate: 30.0, cents: 0.0, flutter: 0.00 },
    level: 1.0,
    hours: Hours::Day,
};

// ---------------------------------------------------------------------------------------------
// Invented tropical voices (no recordings: composed for the `birds` "Tropical" preset)
// ---------------------------------------------------------------------------------------------

const TROPICAL_WHISTLER_SYL: &[Syllable] = &[
    syl(0.45, &[(0.0, 1500.0, 0.0), (0.2, 1600.0, 0.9), (0.6, 2300.0, 1.0), (1.0, 2000.0, 0.0)], 0),
    syl(0.3, &[(0.0, 2400.0, 0.0), (0.2, 2300.0, 0.9), (1.0, 1400.0, 0.0)], 0),
    syl(0.2, &[(0.0, 1800.0, 0.0), (0.4, 2600.0, 1.0), (1.0, 2500.0, 0.0)], 0),
];

/// Invented: an oriole-like whistler, slow pure curving whistles (composed, not transcribed).
pub const TROPICAL_WHISTLER: Species = Species {
    name: "Tropical whistler",
    latin: "(invented)",
    invented: true,
    syllables: TROPICAL_WHISTLER_SYL,
    timbres: &[tone([0.06, 0.02, 0.0])],
    phrases: &[once(&[(0, 0.0)], 0.15), once(&[(1, 0.0)], 0.12), rep(&[(2, 0.0)], (2, 3), (0.32, 0.3), 0.1)],
    songs: &[song(&[slot(&[0, 2], 1.0), slot(&[1, 2], 0.8), slot(&[1], 0.5)], (1, 1), 1.0)],
    alarm: &[],
    pause: (4.0, 0.4),
    alarm_pause: (1.0, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 0.7,
    var: Variation { pitch: 0.04, tempo: 0.06, note: 0.02, individual: 0.2 },
    micro: Micro { rate: 45.0, cents: 12.0, flutter: 0.80 },
    level: 0.9,
    hours: Hours::Day,
};

const TROPICAL_BABBLER_SYL: &[Syllable] = &[
    syl(0.08, &[(0.0, 1200.0, 0.0), (0.3, 1500.0, 1.0), (1.0, 1100.0, 0.0)], 0),
    syl(0.05, &[(0.0, 900.0, 0.0), (0.3, 1100.0, 0.8), (1.0, 950.0, 0.0)], 0),
    syl(0.03, &[(0.0, 3500.0, 0.0), (0.4, 4200.0, 1.0), (1.0, 3000.0, 0.0)], 1),
];

/// Invented: a cackling, laughing babbler for tropical forests (composed, not transcribed).
pub const TROPICAL_BABBLER: Species = Species {
    name: "Tropical babbler",
    latin: "(invented)",
    invented: true,
    syllables: TROPICAL_BABBLER_SYL,
    timbres: &[
        Timbre { noise: 0.15, rough: 0.3, formants: &[(1400.0, 600.0, 1.0), (2800.0, 900.0, 0.5)], ..Timbre::PURE },
        tone([0.1, 0.02, 0.0]),
    ],
    phrases: &[
        Phrase { unit: &[(0, 0.0), (1, 0.09)], reps: (4, 9), period: (0.2, 0.15), drift: 0.15, swell: 1.2, gap: 0.05 },
        rep(&[(2, 0.0)], (6, 12), (0.06, 0.055), 0.0),
    ],
    songs: &[song(&[slot(&[0], 1.0), slot(&[1], 0.5)], (1, 1), 1.0)],
    alarm: &[],
    pause: (6.0, 0.5),
    alarm_pause: (1.0, 0.3),
    bout: (0, 0),
    bout_pause: (0.0, 0.0),
    repeat: 0.5,
    var: Variation { pitch: 0.05, tempo: 0.08, note: 0.02, individual: 0.2 },
    micro: Micro { rate: 45.0, cents: 15.0, flutter: 1.20 },
    level: 0.9,
    hours: Hours::Day,
};

/// Every species, in the order of the `bird/species` param.
pub const SPECIES: &[&Species] = &[
    &BLACKBIRD, &ROBIN, &CHAFFINCH, &GREAT_TIT, &CUCKOO, &WOOD_PIGEON, &COLLARED_DOVE, &CROW, &TAWNY_OWL, &HOUSE_SPARROW, &HERRING_GULL,
    &NIGHTINGALE, &SKYLARK, &WOODPECKER, &TROPICAL_WHISTLER, &TROPICAL_BABBLER,
];

/// Names of [`SPECIES`], for the param's enum.
pub const NAMES: &[&str] = &[
    "Blackbird", "Robin", "Chaffinch", "Great tit", "Cuckoo", "Wood pigeon", "Collared dove", "Crow", "Tawny owl", "House sparrow", "Herring gull",
    "Nightingale", "Skylark", "Woodpecker", "Tropical whistler", "Tropical babbler",
];

/// Index of the species called `name` (case-insensitive).
pub fn index_of(name: &str) -> Option<usize> {
    NAMES.iter().position(|n| n.eq_ignore_ascii_case(name))
}
