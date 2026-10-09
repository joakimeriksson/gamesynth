//! Voices: dialogue babble and creature vocalisations, for RPGs, cozy and indie games.
//!
//! Everything here is one source-filter voice, [`Throat`]: a band-limited pulse train (the vocal
//! folds) with per-cycle jitter, shimmer and subharmonics (roughness), plus breath noise, through
//! three formants (the vocal tract, [`crate::dsp::Formant`]). Pitch contours are piecewise glides
//! ([`Contour`], built on [`crate::dsp::Chirp`]), loudness is [`crate::dsp::Envelope`], series of
//! calls are a [`crate::dsp::Pattern`] and distance is [`crate::dsp::Air`].
//!
//! * `babble`: "Animal Crossing / Undertale" speech from text. The game triggers it once per
//!   character as the dialogue types out, with the character in `letter` (its code / 128). Each
//!   letter is a syllable whose pitch, vowel and consonant follow the letter, so the same text
//!   always sounds the same; `voice/seed`, pitch, formant shift and style make each character's
//!   voice its own, and `mood` / `anger` bend the melody (happy rises, sad falls, angry is harsh).
//! * `creature_growl`, `creature_roar`, `creature_hiss`, `creature_squeak`, `creature_chirp`,
//!   `creature_chatter`: one-shot calls driven by `size` (a mouse at 0 .. a dragon at 1),
//!   `aggression` and `excitement`, on one shared parameter table whose presets are species.
//! * `creature_idle`: continuous breathing (calm .. panting) and purring.
//!
//! Measured on recordings (CC0, Freesound; `target/refs/voice`): growls are vocal fry, irregular
//! glottal pulses grouped into a 25..35 Hz beat over an 70..100 Hz voice, with nearly all their
//! energy under 1 kHz; a purr is 25..29 Hz pulses on both breaths; roars swell for most of a second
//! and fall about an octave; squeaks are 50..150 ms arches at 3..5 kHz; bird chirps fall fast;
//! speech is 4..7 syllables a second, game text blips 9 a second.

use core::marker::PhantomData;

use crate::blocks::{hz_coef, mix_seed, OnePole, Reverb, BLOCK};
use crate::dsp::{size_scale, vowel, Air, Chirp, Envelope, Formant, Pattern, Rhythm, MAX_FORMANTS};
use crate::filter::{FilterMode, Svf};
use crate::math::{soft_clip, Rng, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::osc::{Oscillator, Waveform};
use crate::params::{exp, int, lin, ParamKind, GAIN, UNIT};

/// Time the peak meter needs after the (already faded) end, see `Native::is_finished`.
const SETTLE: f32 = 0.15;
/// The reverb tail fades out over its last 50 ms instead of stopping.
const TAIL_FADE: f32 = 0.05;
/// White noise through a narrow formant keeps a small share of its power; this brings breath up
/// to a level comparable with the voiced source.
const BREATH_GAIN: f32 = 5.0;

// ---------------------------------------------------------------------------------------------
// The voice: source, formants, contour, room
// ---------------------------------------------------------------------------------------------

/// What a [`Throat`] is set to for one block.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    /// Vowel 0 a, 1 e, 2 i, 3 o, 4 u (fractions morph), see [`crate::dsp::vowel`].
    pub vowel: f32,
    /// Formant frequencies times this: > 1 a smaller head, < 1 a bigger one.
    pub shift: f32,
    /// Formant Qs times this.
    pub q: f32,
    /// Subharmonic depth 0..1: all but every `sub_n`th glottal cycle are this much weaker, so the
    /// voice gains a pulse at f0 / `sub_n` (vocal fry, a growl's rattle).
    pub sub: f32,
    pub sub_n: u32,
    /// Per-cycle random period (jitter) and amplitude (shimmer), as fractions.
    pub jitter: f32,
    pub shimmer: f32,
    /// One-pole low-pass on the pulses: a soft voice is dark, a pressed or shouting one bright.
    pub tilt_hz: f32,
    pub wave: Waveform,
}

impl Default for Shape {
    fn default() -> Self {
        Shape { vowel: 0.0, shift: 1.0, q: 1.0, sub: 0.0, sub_n: 2, jitter: 0.0, shimmer: 0.0, tilt_hz: 3000.0, wave: Waveform::Saw }
    }
}

/// Source-filter voice: a band-limited saw (or square) at f0 with per-cycle jitter, shimmer and
/// subharmonics, plus breath noise, through three parallel formants.
#[derive(Clone, Copy, Debug)]
pub struct Throat {
    osc: Oscillator,
    noise: Noise,
    rng: Rng,
    formant: Formant,
    levels: [f32; MAX_FORMANTS],
    tilt: OnePole,
    tilt_coef: f32,
    /// This cycle's period factor and gain.
    period: f32,
    gain: f32,
    cycle: u32,
    sub: f32,
    sub_n: u32,
    jitter: f32,
    shimmer: f32,
    wave: Waveform,
}

impl Throat {
    /// `series`: Klatt's cascade of resonant low-passes, whose spectrum falls steeply above
    /// the formants (big throats, growls); else parallel band-passes (brighter, clearer vowels).
    pub fn new(seed: u32, series: bool) -> Self {
        Throat {
            osc: Oscillator::new(mix_seed(seed)),
            noise: Noise::new(mix_seed(seed ^ 0xB0E5)),
            rng: Rng::new(mix_seed(seed ^ 0x7A11)),
            formant: Formant::new(series),
            levels: [0.0; MAX_FORMANTS],
            tilt: OnePole::default(),
            tilt_coef: 1.0,
            period: 1.0,
            gain: 1.0,
            cycle: 0,
            sub: 0.0,
            sub_n: 2,
            jitter: 0.0,
            shimmer: 0.0,
            wave: Waveform::Saw,
        }
    }

    /// Set the vocal tract and the source's character for this block.
    pub fn set(&mut self, s: &Shape, sr: f32) {
        let v = vowel(s.vowel);
        let top = 0.42 * sr;
        let f = v.map(|(hz, q, _)| ((hz * s.shift).clamp(60.0, top), (q * s.q).max(0.7)));
        // A fourth formant (~3.3 kHz in an adult) keeps a cascade from falling too steeply.
        let f4 = ((3300.0 * s.shift).clamp(60.0, top), (8.0 * s.q).max(0.7));
        self.formant.set(&[f[0], f[1], f[2], f4], sr);
        self.levels = [v[0].2, v[1].2, v[2].2, 0.1];
        self.tilt_coef = hz_coef(s.tilt_hz.min(0.45 * sr), sr);
        self.sub = s.sub.clamp(0.0, 0.95);
        self.sub_n = s.sub_n.max(1);
        self.jitter = s.jitter.clamp(0.0, 0.3);
        self.shimmer = s.shimmer.clamp(0.0, 0.5);
        self.wave = s.wave;
    }

    /// Restart the noise and jitter streams (a repeatable render).
    pub fn reseed(&mut self, seed: u32) {
        self.noise = Noise::new(mix_seed(seed ^ 0xB0E5));
        self.rng = Rng::new(mix_seed(seed ^ 0x7A11));
    }

    /// Back to rest: filters empty, the pulse train at the start of a cycle.
    pub fn reset(&mut self) {
        self.formant.reset();
        self.tilt = OnePole::default();
        self.osc.reset(0.0);
        self.period = 1.0;
        self.gain = 1.0;
        self.cycle = 0;
    }

    /// One sample at `f0` Hz: `voiced` is the level of the pulses, `breath` of the noise.
    #[inline]
    pub fn tick(&mut self, f0: f32, voiced: f32, breath: f32, sr: f32) -> f32 {
        let before = self.osc.phase;
        let s = self.osc.next(self.wave, (f0 * self.period / sr).clamp(0.0, 0.45), 0.5) * self.gain;
        if self.osc.phase < before {
            self.next_cycle();
        }
        let x = self.tilt.lp(s * voiced, self.tilt_coef);
        let n = if breath > 0.0 { self.noise.white() * breath * BREATH_GAIN } else { 0.0 };
        self.formant.tick(x + n, &self.levels)
    }

    fn next_cycle(&mut self) {
        self.cycle = self.cycle.wrapping_add(1);
        self.period = if self.jitter > 0.0 { 1.0 + self.jitter * self.rng.next_bipolar() } else { 1.0 };
        let sub = if self.sub_n > 1 && !self.cycle.is_multiple_of(self.sub_n) { 1.0 - self.sub } else { 1.0 };
        self.gain = sub * if self.shimmer > 0.0 { 1.0 + self.shimmer * self.rng.next_bipolar() } else { 1.0 };
    }
}

/// Most points in a [`Contour`].
const MAX_POINTS: usize = 4;

/// A pitch contour through up to four points, each segment a [`Chirp`] glide (even in octaves),
/// started from wherever the pitch is, so contours chain without jumps.
#[derive(Clone, Copy, Debug, Default)]
pub struct Contour {
    chirp: Chirp,
    /// (seconds after the start, Hz).
    pts: [(f32, f32); MAX_POINTS],
    n: usize,
    seg: usize,
    t: f32,
}

impl Contour {
    /// Glide from `from` Hz through `pts` (seconds after now, rising; Hz).
    pub fn start(&mut self, from: f32, pts: &[(f32, f32)]) {
        self.n = pts.len().clamp(1, MAX_POINTS);
        for (k, &p) in pts.iter().take(self.n).enumerate() {
            self.pts[k] = p;
        }
        self.seg = 0;
        self.t = 0.0;
        self.chirp.start(from, self.pts[0].1, self.pts[0].0.max(1e-4), 1.0);
    }

    #[inline]
    fn advance(&mut self, sr: f32) {
        self.t += 1.0 / sr;
        if self.seg + 1 < self.n && self.t >= self.pts[self.seg].0 {
            let from = self.chirp.freq();
            self.seg += 1;
            let (t1, hz) = self.pts[self.seg];
            self.chirp.start(from, hz, (t1 - self.pts[self.seg - 1].0).max(1e-4), 1.0);
        }
    }

    /// The pitch for this sample (Hz), for a source of its own.
    #[inline]
    pub fn step(&mut self, sr: f32) -> f32 {
        self.advance(sr);
        self.chirp.step(sr)
    }

    /// The contour as a tone: a sine, or FM (modulator at `ratio` times the pitch) for overtones.
    #[inline]
    pub fn tone(&mut self, ratio: f32, index: f32, sr: f32) -> f32 {
        self.advance(sr);
        self.chirp.tick(ratio, index, sr)
    }

    #[inline]
    pub fn freq(&self) -> f32 {
        self.chirp.freq()
    }
}

/// A small room whose tail rings on after the voice and then fades to exact silence.
struct Room {
    reverb: Reverb,
    tail_left: f32,
}

impl Room {
    fn new(sr: f32) -> Self {
        Room { reverb: Reverb::new(sr), tail_left: 0.0 }
    }

    fn clear(&mut self) {
        self.reverb.clear();
        self.tail_left = 0.0;
    }

    fn is_ringing(&self) -> bool {
        self.tail_left > 0.0
    }

    /// Add the room to `out` (the dry block). `sounding`: the voice produced sound this block.
    fn process(&mut self, out: &mut [f32], sounding: bool, send: f32, rt: f32, damping: f32, sr: f32) {
        let n = out.len();
        let start = self.tail_left;
        if sounding {
            self.tail_left = if send > 0.0 { rt } else { 0.0 };
        } else {
            self.tail_left = (self.tail_left - n as f32 / sr).max(0.0);
        }
        if send <= 0.0 || (!sounding && start <= 0.0) {
            return;
        }
        let (a, b) = if sounding { (1.0, 1.0) } else { ((start / TAIL_FADE).min(1.0), (self.tail_left / TAIL_FADE).min(1.0)) };
        let step = (b - a) / n as f32;
        let mut fade = a;
        for o in out.iter_mut() {
            fade += step;
            *o += self.reverb.tick(*o, rt, damping) * send * fade;
        }
    }
}

#[inline]
fn semis(s: f32) -> f32 {
    (s / 12.0).exp2()
}

#[inline]
fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let u = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// Gentle saturation that keeps quiet passages at their level: `amount` 0 is a bypass.
#[inline]
fn rasp(x: f32, amount: f32) -> f32 {
    if amount <= 0.0 {
        return x;
    }
    let k = 1.0 + 3.0 * amount;
    soft_clip(x * k) / k.powf(0.7)
}

/// Distance: duller, quieter (as the material events).
fn far(air: &mut Air, distance: f32, sr: f32) {
    air.set(distance, 18000.0, 400.0, 1.0 - 0.75 * distance, sr);
}

// ---------------------------------------------------------------------------------------------
// Text: letters to syllables
// ---------------------------------------------------------------------------------------------

/// The `letter` input carries a character as its code / 128 (ASCII; see [`letter_code`]).
pub const LETTER_SCALE: f32 = 128.0;

/// The character code a `letter` input value stands for: round(letter * 128), 0..=128.
pub fn letter_code(letter: f32) -> u32 {
    (letter.clamp(0.0, 1.0) * LETTER_SCALE).round() as u32
}

/// The `letter` value for a character: its code / 128 for ASCII, anything else folded into
/// 0..127 (still one fixed syllable per character).
pub fn letter_value(c: char) -> f32 {
    (c as u32 % 128) as f32 / LETTER_SCALE
}

/// How a syllable starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Onset {
    /// Straight into the vowel.
    Open,
    /// A burst at `hz` (lips ~900, tongue tip ~3500, back ~1900); unvoiced ones then aspirate.
    Stop { voiced: bool, hz: f32 },
    /// Hiss at `hz` with resonance `res` and level, voiced or not.
    Fricative { voiced: bool, hz: f32, res: f32, level: f32 },
    /// Breath through the vowel ('h').
    Breath,
    /// A hum that opens into the vowel ('m', 'n').
    Nasal,
    /// The tract glides from this vowel position ('l', 'r', 'w', 'y').
    Glide(f32),
}

/// What a character does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Glyph {
    /// A syllable; `vowel` is the letter's own vowel, or `None` for a consonant (which then
    /// takes a vowel from the voice's seed).
    Syllable { onset: Onset, vowel: Option<f32> },
    /// Space: silent, and the next syllable starts a word (a little stress).
    Space,
    /// Comma, semicolon, colon: silent; a short breath in the melody.
    Comma,
    /// Full stop: the sounding syllable falls, the next starts a new phrase.
    Stop,
    /// Exclamation mark: falls and stresses, new phrase.
    Exclaim,
    /// Question mark: the sounding syllable rises, new phrase.
    Question,
    /// Other punctuation: nothing.
    Silent,
}

const fn stop(voiced: bool, hz: f32) -> Onset {
    Onset::Stop { voiced, hz }
}

const fn fric(voiced: bool, hz: f32, res: f32, level: f32) -> Onset {
    Onset::Fricative { voiced, hz, res, level }
}

/// The letters a character outside the alphabet (digits, control codes, non-ASCII folded or
/// hashed values) borrows its sound from.
const BORROW: &[u8] = b"bdfghklmnprstvwzaeiouaeo";

/// The glyph for an ASCII lowercase letter.
fn letter_glyph(c: u8) -> Option<Glyph> {
    let s = |onset| Some(Glyph::Syllable { onset, vowel: None });
    let v = |vowel| Some(Glyph::Syllable { onset: Onset::Open, vowel: Some(vowel) });
    match c {
        b'a' => v(0.0),
        b'e' => v(1.0),
        b'i' => v(2.0),
        b'o' => v(3.0),
        b'u' => v(4.0),
        b'y' => s(Onset::Glide(2.0)),
        b'b' => s(stop(true, 900.0)),
        b'p' => s(stop(false, 900.0)),
        b'd' => s(stop(true, 3200.0)),
        b't' => s(stop(false, 3600.0)),
        b'g' => s(stop(true, 1800.0)),
        b'k' | b'c' | b'q' => s(stop(false, 1900.0)),
        b's' => s(fric(false, 6500.0, 0.45, 1.0)),
        b'z' => s(fric(true, 6000.0, 0.45, 0.6)),
        b'x' => s(fric(false, 5200.0, 0.35, 0.9)),
        b'f' => s(fric(false, 4500.0, 0.05, 0.45)),
        b'v' => s(fric(true, 4000.0, 0.05, 0.3)),
        b'j' => s(fric(true, 2800.0, 0.35, 0.6)),
        b'h' => s(Onset::Breath),
        b'm' | b'n' => s(Onset::Nasal),
        b'l' => s(Onset::Glide(1.4)),
        b'r' => s(Onset::Glide(3.4)),
        b'w' => s(Onset::Glide(4.0)),
        _ => None,
    }
}

/// What character code `code` does (see [`letter_code`]). Letters speak as themselves (upper
/// and lower case alike); spaces and punctuation are silent but shape the melody; anything else
/// borrows a letter's sound by a fixed hash, so every value of `letter` makes a syllable.
pub fn glyph(code: u32) -> Glyph {
    let c = if (65..=90).contains(&code) { code + 32 } else { code };
    match c {
        32 => return Glyph::Space,
        44 | 58 | 59 => return Glyph::Comma,
        46 => return Glyph::Stop,
        33 => return Glyph::Exclaim,
        63 => return Glyph::Question,
        34..=43 | 45 | 47 | 60..=62 | 64 | 91..=96 | 123..=126 => return Glyph::Silent,
        _ => {}
    }
    if c < 128 {
        if let Some(g) = letter_glyph(c as u8) {
            return g;
        }
    }
    let k = hash(code, 0, 0x60) as usize % BORROW.len();
    letter_glyph(BORROW[k]).unwrap_or(Glyph::Silent)
}

#[inline]
fn hash(code: u32, seed: u32, salt: u32) -> u32 {
    mix_seed(code.wrapping_mul(0x9E37_79B1) ^ seed.wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D))
}

#[inline]
fn hash01(code: u32, seed: u32, salt: u32) -> f32 {
    (hash(code, seed, salt) >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// The vowel a consonant letter takes in a voice with `seed`: an accent. Weighted like speech
/// (more a and e than u).
fn seeded_vowel(code: u32, seed: u32) -> f32 {
    let h = hash01(code, seed, 2);
    match h {
        h if h < 0.28 => 0.0,
        h if h < 0.50 => 1.0,
        h if h < 0.70 => 2.0,
        h if h < 0.88 => 3.0,
        _ => 4.0,
    }
}

/// Intrinsic pitch of a vowel (semitones): close vowels sit a little higher in real speech.
fn vowel_pitch(v: f32) -> f32 {
    [-0.4, 0.0, 0.8, -0.2, 0.6][(v.round() as usize).min(4)]
}

// ---------------------------------------------------------------------------------------------
// babble
// ---------------------------------------------------------------------------------------------

pub const BABBLE_STYLES: [&str; 6] = ["Natural", "Critter", "Robot", "Gruff", "Elder", "Ghost"];

/// How one style speaks.
#[derive(Clone, Copy, Debug)]
struct Style {
    wave: Waveform,
    shift: f32,
    q: f32,
    tilt: f32,
    breath: f32,
    /// Share of the pulses replaced by breath (a whisper).
    whisper: f32,
    sub: f32,
    sub_n: u32,
    jitter: f32,
    shimmer: f32,
    /// Vibrato (Hz, semitones) and tremolo depth.
    vib_hz: f32,
    vib: f32,
    trem: f32,
    /// Syllable length factor, attack, decay (60 dB) and the glide into each syllable's pitch.
    length: f32,
    attack: f32,
    decay: f32,
    glide: f32,
    /// Each syllable starts this many semitones off its pitch and glides in.
    scoop: f32,
    /// Melody range factor and the slope over a syllable (semitones).
    range: f32,
    contour: f32,
    /// How much of the consonants is heard.
    cons: f32,
    drive: f32,
    /// Amplitude modulation (Hz): a ring-modulated, metallic voice.
    ring_hz: f32,
    room: f32,
    room_time: f32,
    level: f32,
}

const NATURAL: Style = Style {
    wave: Waveform::Saw,
    shift: 1.0,
    q: 1.0,
    tilt: 2600.0,
    breath: 0.04,
    whisper: 0.0,
    sub: 0.0,
    sub_n: 2,
    jitter: 0.006,
    shimmer: 0.03,
    vib_hz: 0.0,
    vib: 0.0,
    trem: 0.0,
    length: 1.0,
    attack: 0.012,
    decay: 0.07,
    glide: 0.03,
    scoop: -1.0,
    range: 1.0,
    contour: -0.6,
    cons: 1.0,
    drive: 0.0,
    ring_hz: 0.0,
    room: 0.0,
    room_time: 0.6,
    level: 1.0,
};

const STYLES: [Style; 6] = [
    NATURAL,
    // Critter: a small, quick, bright voice whose syllables bloop down into their pitch.
    Style { shift: 1.25, q: 1.1, tilt: 4200.0, breath: 0.02, jitter: 0.004, length: 0.72, attack: 0.006, decay: 0.05, glide: 0.022, scoop: 3.0, range: 1.4, contour: 1.0, cons: 0.35, level: 1.12, ..NATURAL },
    // Robot: a square wave, steady and stepped, sharp formants, ring-modulated.
    Style {
        wave: Waveform::Square,
        q: 1.8,
        tilt: 7000.0,
        breath: 0.0,
        jitter: 0.0,
        shimmer: 0.0,
        length: 0.9,
        attack: 0.004,
        decay: 0.03,
        glide: 0.002,
        scoop: 0.0,
        range: 0.8,
        contour: 0.0,
        cons: 0.5,
        ring_hz: 70.0,
        room_time: 0.4,
        level: 1.87,
        ..NATURAL
    },
    // Gruff: low, dark and rough (subharmonics, jitter), a little rasp.
    Style { shift: 0.86, q: 0.8, tilt: 1300.0, breath: 0.18, sub: 0.35, jitter: 0.025, shimmer: 0.1, length: 1.1, attack: 0.015, decay: 0.08, glide: 0.035, scoop: -2.0, range: 0.7, contour: -1.0, drive: 0.35, level: 1.11, ..NATURAL },
    // Elder: slower, breathy, with a tremor in pitch and level.
    Style { shift: 0.96, q: 0.9, tilt: 1800.0, breath: 0.28, jitter: 0.02, shimmer: 0.12, vib_hz: 5.8, vib: 0.45, trem: 0.22, length: 1.3, attack: 0.02, decay: 0.1, glide: 0.04, scoop: -1.5, range: 0.8, contour: -1.2, cons: 0.8, level: 1.03, ..NATURAL },
    // Ghost: a whisper through whistling formants, sinking and wavering, in a large hall.
    Style {
        shift: 1.1,
        q: 1.7,
        tilt: 900.0,
        breath: 0.55,
        whisper: 0.8,
        vib_hz: 4.3,
        vib: 0.5,
        trem: 0.15,
        length: 1.7,
        attack: 0.05,
        decay: 0.25,
        glide: 0.08,
        scoop: -3.0,
        contour: -2.0,
        cons: 0.4,
        room: 0.5,
        room_time: 2.4,
        level: 2.17,
        ..NATURAL
    },
];

/// Seconds of one syllable at speed 1, before style and mood: per-letter babble at a typing
/// rate of 10..12 characters a second.
const SYLLABLE: f32 = 0.085;
/// A gap longer than this (or `.`, `!`, `?`) starts a new phrase.
const PHRASE_GAP: f32 = 0.6;
/// Output level of the voice (calibrated so `babble` sits with `ui_type`).
const BABBLE_GAIN: f32 = 0.95;

model_params! {
    /// A babbling voice. `voice/style`: 0 natural, 1 critter, 2 robot, 3 gruff, 4 elder, 5 ghost.
    /// `voice/seed` is the character: it decides each letter's pitch and the vowels of the
    /// consonants, so two characters with the same pitch still speak different melodies.
    BabbleParams / BabbleParamId {
        style: "voice/style" = 0.0, ParamKind::Enum(&BABBLE_STYLES);
        pitch: "voice/pitch_hz" = 165.0, exp(50.0, 900.0);
        shift: "voice/formant_shift" = 1.1, exp(0.6, 2.0);
        range: "voice/range_semitones" = 6.0, lin(0.0, 16.0);
        seed: "voice/seed" = 0.0, int(0, 99);
        speed: "voice/speed" = 1.0, exp(0.5, 2.0);
        breath: "voice/breath" = 0.2, UNIT;
        consonants: "voice/consonants" = 0.5, UNIT;
        variation: "shape/variation" = 0.3, UNIT;
        room: "space/room" = 0.15, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

fn style_index(p: &BabbleParams) -> usize {
    (p.style.round().max(0.0) as usize).min(STYLES.len() - 1)
}

fn babble_preset(style: f32, pitch: f32, shift: f32, range: f32, seed: f32, speed: f32) -> BabbleParams {
    BabbleParams { style, pitch, shift, range, seed, speed, ..Default::default() }
}

/// Mood as (happy, sad), each 0..1, from the `mood` input (0 sad, 0.5 neutral, 1 happy).
fn moods(mood: f32) -> (f32, f32) {
    (((mood - 0.5) * 2.0).max(0.0), ((0.5 - mood) * 2.0).max(0.0))
}

/// Speed multiplier for a mood: happy and angry speak faster, sad slower.
fn mood_speed(happy: f32, sad: f32, anger: f32) -> f32 {
    1.0 + 0.12 * happy - 0.26 * sad + 0.1 * anger
}

pub struct Babble {
    sr: f32,
    throat: Throat,
    contour: Contour,
    env: Envelope,
    fric_env: Envelope,
    asp_env: Envelope,
    fric: Svf,
    fric_norm: f32,
    burst: Svf,
    burst_amp: f32,
    burst_k: f32,
    burst_norm: f32,
    noise: Noise,
    rng: Rng,
    air: Air,
    /// The top of a voice falls away above ~6 kHz (scaled with the formants).
    top: OnePole,
    top_coef: f32,
    room: Room,
    // The syllable.
    style: usize,
    syl_t: f32,
    voice_at: f32,
    voice_pending: bool,
    peak: f32,
    hold: f32,
    nasal: bool,
    fric_hold: f32,
    asp_hold: f32,
    vowel_from: f32,
    vowel_to: f32,
    vowel_tau: f32,
    shape: Shape,
    breath: f32,
    cons: f32,
    drive: f32,
    // Prosody across syllables.
    since: f32,
    phrase_t: f32,
    in_phrase: bool,
    word_start: bool,
    decline: f32,
    bend: f32,
    bend_target: f32,
    vib_phase: f32,
    ring_phase: f32,
    active: bool,
    pitch_ratio: f32,
}

impl Babble {
    fn vowel_now(&self) -> f32 {
        self.vowel_to + (self.vowel_from - self.vowel_to) * (-self.syl_t / self.vowel_tau).exp()
    }

    fn sounding(&self) -> bool {
        !self.env.is_idle() || self.voice_pending || !self.fric_env.is_idle() || !self.asp_env.is_idle() || self.burst_amp > 1e-5
    }

    fn punctuate(&mut self, g: Glyph) {
        let sounding = self.sounding();
        match g {
            Glyph::Space => self.word_start = true,
            Glyph::Comma => {
                self.word_start = true;
                self.phrase_t *= 0.5;
            }
            Glyph::Stop | Glyph::Exclaim | Glyph::Question => {
                self.in_phrase = false;
                if sounding {
                    self.bend_target = match g {
                        Glyph::Question => 5.0,
                        Glyph::Exclaim => -1.0,
                        _ => -2.0,
                    };
                    // Hold the last syllable a little so the bend is heard.
                    if g == Glyph::Question {
                        self.hold += 0.06;
                    }
                }
            }
            _ => {}
        }
    }

    fn syllable(&mut self, onset: Onset, own_vowel: Option<f32>, code: u32, x: &[f32], p: &BabbleParams) {
        let (power, mood, anger) = (x[0], x[3], x[4]);
        let st = STYLES[style_index(p)];
        self.style = style_index(p);
        let v = p.variation.clamp(0.0, 1.0);
        let (happy, sad) = moods(mood);
        let seed = p.seed.round().max(0.0) as u32;
        let legato = self.sounding();
        let interval = self.since;
        if !self.in_phrase || !legato || interval > PHRASE_GAP {
            self.in_phrase = true;
            self.phrase_t = 0.0;
            self.word_start = true;
            self.bend = 0.0;
        }
        self.bend_target = 0.0;
        self.since = 0.0;
        // Letters typed faster than a syllable squeeze their consonants.
        let pace = if legato { (interval / 0.12).clamp(0.35, 1.0) } else { 1.0 };
        let speed = p.speed * mood_speed(happy, sad, anger);
        let mut len = SYLLABLE * st.length / speed * (1.0 + 0.1 * v * self.rng.next_bipolar());
        if legato {
            // Letters typed faster than a syllable shorten it, so each letter is still heard as
            // its own beat (the level dips between them, as between spoken syllables).
            len = len.min(0.6 * interval.max(0.03) + st.decay * 0.15);
        }
        let ctime = pace / speed.sqrt();

        // Melody: the letter's place in the voice's range, intrinsic vowel pitch, word stress,
        // the phrase's decline, mood.
        let target_vowel = own_vowel.unwrap_or_else(|| seeded_vowel(code, seed));
        let range = p.range * st.range * (1.0 + 0.5 * happy - 0.5 * sad + 0.3 * anger);
        let mut s = (hash01(code, seed, 1) - 0.5) * range + vowel_pitch(target_vowel);
        s += 4.0 * happy - 3.5 * sad + 2.0 * anger;
        if self.word_start {
            s += 1.2;
        }
        self.decline = 1.5 + 2.0 * sad - 1.2 * happy;
        s -= (self.decline * self.phrase_t).clamp(0.0, 4.0);
        s += v * 0.5 * self.rng.next_bipolar();
        let slope = st.contour + 3.0 * happy - 3.0 * sad - 1.5 * anger;
        let hz = p.pitch * semis(s);
        let from = if st.scoop > 0.0 || !legato { hz * semis(st.scoop) } else { self.contour.freq() };
        let glide = st.glide.min(0.5 * len);
        self.contour.start(from, &[(glide, hz), (len.max(glide + 0.01), hz * semis(slope))]);

        // The tract: from the onset's position (or where it was) into the vowel.
        self.vowel_from = match onset {
            Onset::Glide(g) => g,
            Onset::Nasal => 4.0,
            _ if legato => self.vowel_now(),
            _ => target_vowel,
        };
        self.vowel_to = target_vowel;
        self.vowel_tau = 0.03 * ctime;
        self.syl_t = 0.0;

        // The consonant.
        self.cons = p.consonants * st.cons * 2.0;
        self.nasal = onset == Onset::Nasal;
        self.voice_at = 0.0;
        self.fric_hold = 0.0;
        self.asp_hold = 0.0;
        let shift = p.shift * st.shift * self.pitch_ratio;
        self.top_coef = hz_coef((6000.0 * shift).min(0.45 * self.sr), self.sr);
        match onset {
            Onset::Stop { voiced, hz } => {
                let res = 0.3;
                self.burst.set(FilterMode::BandPass, (hz * shift).min(0.42 * self.sr), res, self.sr);
                self.burst_norm = 2.0 - 1.98 * res;
                self.burst_amp = if voiced { 0.5 } else { 1.0 };
                self.burst_k = (-6.91 / (0.012 * self.sr)).exp();
                if !voiced {
                    self.voice_at = 0.035 * ctime;
                    self.asp_hold = 0.025 * ctime;
                } else {
                    self.voice_at = 0.006 * ctime;
                }
            }
            Onset::Fricative { voiced, hz, res, level } => {
                self.fric.set(FilterMode::BandPass, (hz * shift.sqrt()).min(0.42 * self.sr), res, self.sr);
                self.fric_norm = (2.0 - 1.98 * res) * level;
                self.fric_hold = 0.04 * ctime;
                self.voice_at = if voiced { 0.02 * ctime } else { 0.05 * ctime };
            }
            Onset::Breath => {
                self.asp_hold = 0.035 * ctime;
                self.voice_at = 0.04 * ctime;
            }
            _ => {}
        }
        if self.fric_hold > 0.0 {
            self.fric_env.trigger(1.0);
        }
        if self.asp_hold > 0.0 {
            self.asp_env.trigger(1.0);
        }

        // Loudness and the voice's character for this syllable.
        let stress = if self.word_start { 1.19 } else { 1.0 };
        self.word_start = false;
        self.peak = (0.25 + 0.75 * power) * stress * (1.0 + 0.25 * happy - 0.2 * sad + 0.4 * anger) * (1.0 + 0.1 * v * self.rng.next_bipolar());
        self.hold = (len - st.attack - self.voice_at).max(0.01);
        self.breath = (p.breath * 0.5 + st.breath + 0.25 * sad).min(1.0);
        self.drive = (st.drive + 0.6 * anger).min(1.0);
        self.shape = Shape {
            vowel: self.vowel_from,
            shift,
            q: st.q * (1.0 + 0.3 * anger),
            sub: st.sub + 0.45 * anger,
            sub_n: st.sub_n,
            jitter: st.jitter + 0.03 * anger,
            shimmer: st.shimmer,
            tilt_hz: st.tilt * (1.0 + 0.3 * happy + 1.5 * anger) * (0.6 + 0.4 * power) * self.pitch_ratio,
            wave: st.wave,
        };
        if self.voice_at > 0.01 {
            // An unvoiced consonant interrupts the voice.
            self.env.release();
            self.voice_pending = true;
        } else {
            self.voice_pending = false;
            self.env.trigger(self.peak);
        }
        self.active = true;
    }

    fn length(p: &BabbleParams) -> f32 {
        let st = &STYLES[style_index(p)];
        let slowest = p.speed * mood_speed(0.0, 1.0, 0.0);
        let len = SYLLABLE * st.length / slowest * (1.0 + 0.1 * p.variation);
        let tail = if p.room * 0.6 + st.room > 0.0 { st.room_time } else { 0.0 };
        // The longest consonant, the syllable, a held question, its decay, the room.
        0.06 + len + 0.06 + st.decay + 0.02 + tail + 2.0 * BLOCK as f32 / 8000.0 + SETTLE
    }
}

impl Generator for Babble {
    type P = BabbleParams;
    const NAME: &'static str = "babble";
    const CATEGORY: &'static str = "voice";
    const DOC: &'static str = "Dialogue babble: trigger once per character as the text types out, with the character in `letter` \
        (its code / 128). Letters speak as themselves, so the same text always sounds the same; the seed, pitch, formants and style make each \
        character's voice. Happy rises, sad falls, angry is harsh; `?` lifts the last syllable.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "power", default: 1.0, doc: "Whisper .. shout: level and brightness" },
        InputSpec { name: "distance", default: 0.0, doc: "0 = the speaker is right here, 1 = far off (duller, quieter)" },
        InputSpec { name: "letter", default: 97.0 / 128.0, doc: "The character, as its code / 128 (`ord(c) % 128 / 128.0`): letters speak, space and punctuation shape the melody" },
        InputSpec { name: "mood", default: 0.5, doc: "0 sad (low, falling, slow) .. 0.5 neutral .. 1 happy (high, rising, lively)" },
        InputSpec { name: "anger", default: 0.0, doc: "0 calm .. 1 angry: harsh, rough, loud, falling" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const ONE_SHOT: bool = true;
    const TRANSPOSES: bool = true;

    fn presets() -> Vec<(&'static str, BabbleParams)> {
        vec![
            ("Critter", babble_preset(1.0, 340.0, 1.35, 7.0, 7.0, 1.2)),
            ("Kid", babble_preset(0.0, 290.0, 1.3, 7.0, 3.0, 1.1)),
            ("Robot", babble_preset(2.0, 130.0, 1.0, 7.0, 11.0, 1.0)),
            ("Gruff", babble_preset(3.0, 95.0, 0.85, 4.0, 5.0, 0.9)),
            ("Elder", BabbleParams { breath: 0.3, ..babble_preset(4.0, 150.0, 1.0, 5.0, 9.0, 0.85) }),
            ("Ghost", BabbleParams { room: 0.6, ..babble_preset(5.0, 230.0, 1.15, 6.0, 13.0, 0.8) }),
            ("Giant", babble_preset(3.0, 62.0, 0.7, 3.0, 21.0, 0.75)),
        ]
    }

    fn new(sr: f32) -> Self {
        Babble {
            sr,
            throat: Throat::new(0xBAB1, false),
            contour: Contour::default(),
            env: Envelope::default(),
            fric_env: Envelope::default(),
            asp_env: Envelope::default(),
            fric: Svf::default(),
            fric_norm: 1.0,
            burst: Svf::default(),
            burst_amp: 0.0,
            burst_k: 0.0,
            burst_norm: 1.0,
            noise: Noise::new(mix_seed(0xBAB2)),
            rng: Rng::new(mix_seed(0xBAB3)),
            air: Air::default(),
            top: OnePole::default(),
            top_coef: 1.0,
            room: Room::new(sr),
            style: 0,
            syl_t: 0.0,
            voice_at: 0.0,
            voice_pending: false,
            peak: 0.0,
            hold: 0.0,
            nasal: false,
            fric_hold: 0.0,
            asp_hold: 0.0,
            vowel_from: 0.0,
            vowel_to: 0.0,
            vowel_tau: 0.03,
            shape: Shape::default(),
            breath: 0.0,
            cons: 0.0,
            drive: 0.0,
            since: 0.0,
            phrase_t: 0.0,
            in_phrase: false,
            word_start: true,
            decline: 1.5,
            bend: 0.0,
            bend_target: 0.0,
            vib_phase: 0.0,
            ring_phase: 0.0,
            active: false,
            pitch_ratio: 1.0,
        }
    }

    fn trigger(&mut self, x: &[f32], p: &BabbleParams) {
        let code = letter_code(x[2]);
        if p.variation <= 0.0 && !self.sounding() {
            // No variation means none at all: the same text renders the same samples.
            self.rng = Rng::new(mix_seed(0xBAB3));
            self.noise = Noise::new(mix_seed(0xBAB2));
            self.throat.reseed(0xBAB1);
            self.throat.reset();
            self.room.clear();
            self.air.reset();
        }
        far(&mut self.air, x[1], self.sr);
        match glyph(code) {
            Glyph::Syllable { onset, vowel } => self.syllable(onset, vowel, code, x, p),
            g => self.punctuate(g),
        }
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn length(p: &BabbleParams) -> Option<f32> {
        Some(Self::length(p))
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        self.pitch_ratio = ratio;
    }

    fn block(&mut self, _x: &[f32], p: &BabbleParams, out: &mut [f32]) {
        let n = out.len();
        let sr = self.sr;
        let dt = 1.0 / sr;
        if !self.active {
            out.iter_mut().for_each(|s| *s = 0.0);
            return;
        }
        let st = STYLES[self.style];
        let secs = n as f32 * dt;
        self.since += secs;
        if self.in_phrase {
            self.phrase_t += secs;
        }
        let sounding = self.sounding();
        if sounding {
            let mut shape = self.shape;
            shape.vowel = self.vowel_now();
            self.throat.set(&shape, sr);
            self.bend += (self.bend_target - self.bend) * (1.0 - (-secs / 0.05).exp());
            self.vib_phase = (self.vib_phase + st.vib_hz * secs).fract();
            let wobble = (TAU * self.vib_phase).sin();
            let mult = semis(self.bend + st.vib * wobble) * self.pitch_ratio;
            let trem = 1.0 - st.trem * (0.5 + 0.5 * wobble);
            let voiced = 1.0 - st.whisper;
            let gain = BABBLE_GAIN * st.level * p.gain;
            for o in out.iter_mut() {
                self.syl_t += dt;
                if self.voice_pending && self.syl_t >= self.voice_at {
                    self.voice_pending = false;
                    self.env.trigger(self.peak);
                }
                let f = self.contour.step(sr) * mult;
                let e = self.env.tick(dt, st.attack, self.hold, st.decay, 0.0, 0.015) * trem;
                let hum = if self.nasal { 0.45 + 0.55 * smoothstep(0.02, 0.05, self.syl_t) } else { 1.0 };
                let asp = self.asp_env.tick(dt, 0.004, self.asp_hold, 0.02, 0.0, 0.02) * self.cons * 0.25;
                let mut y = self.throat.tick(f, e * hum * voiced, e * self.breath * 0.3 + asp, sr);
                let fr = self.fric_env.tick(dt, 0.008, self.fric_hold, 0.03, 0.0, 0.02);
                if fr > 0.0 {
                    y += self.fric.tick(self.noise.white()) * self.fric_norm * fr * self.cons * 0.35;
                }
                if self.burst_amp > 1e-5 {
                    y += self.burst.tick(self.noise.white()) * self.burst_norm * self.burst_amp * self.cons * 0.5;
                    self.burst_amp *= self.burst_k;
                }
                if st.ring_hz > 0.0 {
                    self.ring_phase = (self.ring_phase + st.ring_hz * dt).fract();
                    y *= 0.6 + 0.4 * (TAU * self.ring_phase).sin();
                }
                let y = self.top.lp(y, self.top_coef);
                *o = self.air.tick(rasp(y, self.drive)) * gain;
            }
        } else {
            out.iter_mut().for_each(|s| *s = 0.0);
        }
        let send = p.room * 0.6 + st.room;
        self.room.process(out, sounding, send, st.room_time, 0.4, sr);
        if !sounding && !self.room.is_ringing() {
            // At rest: every filter and phase back to where a new voice starts.
            self.active = false;
            self.throat.reset();
            self.air.reset();
            self.room.clear();
            self.burst_amp = 0.0;
            self.top = OnePole::default();
            self.fric.reset();
            self.burst.reset();
            self.vib_phase = 0.0;
            self.ring_phase = 0.0;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Creatures
// ---------------------------------------------------------------------------------------------

/// What makes the sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// The [`Throat`]: pulses and breath through formants.
    Throat,
    /// A pure tone along the contour (FM for overtones): squeaks, bird chirps.
    Tone,
}

/// One call, at size 0.5 (a dog or wolf), aggression and excitement 0.5.
#[derive(Clone, Copy, Debug)]
pub struct Call {
    pub source: Source,
    /// Pitch (Hz) and how it falls with size: f0 / scale^size_exp, scale 0.25 (mouse) .. 4 (dragon).
    pub f0: f32,
    pub size_exp: f32,
    /// Pitch contour: (fraction of the syllable, semitones), first point at 0.
    pub contour: [(f32, f32); 4],
    /// Syllable length in seconds and how it grows with size (scale^size_secs).
    pub secs: f32,
    pub size_secs: f32,
    /// Envelope: attack and hold as fractions of the syllable, decay (60 dB) in seconds.
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    /// Vowel at the start, at 40 % and at the end.
    pub vowel: [f32; 3],
    /// Formants as a cascade (dark, steep above the formants) rather than in parallel.
    pub series: bool,
    pub q: f32,
    pub sub: f32,
    pub sub_n: u32,
    pub jitter: f32,
    pub breath: f32,
    /// Hiss (noise above `hiss_hz`), with its own envelope: attack and hold (s), decay (s).
    pub hiss: f32,
    pub hiss_hz: f32,
    pub hiss_env: [f32; 3],
    /// Source brightness (low-pass Hz) at size 0.5.
    pub tilt: f32,
    /// Pulsing "grr-rr" (depth at full excitement), at about 6 Hz.
    pub pulse: f32,
    /// Syllables at excitement 0 and 1, their rate per second, and each one's pitch step.
    pub count: [f32; 2],
    pub rate: f32,
    pub step: f32,
    /// Tone overtones (FM index).
    pub fm: f32,
    pub level: f32,
}

const CALL: Call = Call {
    source: Source::Throat,
    f0: 100.0,
    size_exp: 0.85,
    contour: [(0.0, 0.0), (1.0, 0.0), (1.0, 0.0), (1.0, 0.0)],
    secs: 1.0,
    size_secs: 0.3,
    attack: 0.15,
    hold: 0.6,
    decay: 0.3,
    vowel: [3.0, 3.0, 3.0],
    series: false,
    q: 0.7,
    sub: 0.0,
    sub_n: 2,
    jitter: 0.02,
    breath: 0.3,
    hiss: 0.0,
    hiss_hz: 3000.0,
    hiss_env: [0.01, 0.05, 0.1],
    tilt: 2000.0,
    pulse: 0.0,
    count: [1.0, 1.0],
    rate: 4.0,
    step: 0.0,
    fm: 0.0,
    level: 1.0,
};

/// A creature call: a name and a [`Call`]. [`Creature`] turns it into a generator.
pub trait CreatureCall: Send + 'static {
    const NAME: &'static str;
    const DOC: &'static str;
    const CALL: Call;
}

macro_rules! creature_call {
    ($ty:ident, $name:literal, $doc:literal, $call:expr) => {
        pub struct $ty;
        impl CreatureCall for $ty {
            const NAME: &'static str = $name;
            const DOC: &'static str = $doc;
            const CALL: Call = $call;
        }
    };
}

creature_call!(Growl, "creature_growl", "A low, rough growl (vocal fry: pulses grouped into a rattle) about a second long; aggression snarls, excitement pulses it.",
    Call {
        f0: 95.0,
        contour: [(0.0, -2.0), (0.3, 1.0), (0.8, 0.0), (1.0, -3.0)],
        secs: 1.1,
        attack: 0.15,
        hold: 0.6,
        decay: 0.35,
        vowel: [4.0, 3.6, 4.0],
        series: true,
        q: 0.55,
        sub: 0.5,
        sub_n: 3,
        jitter: 0.02,
        breath: 0.06,
        tilt: 800.0,
        pulse: 0.6,
        level: 1.19,
        ..CALL
    });
creature_call!(Roar, "creature_roar", "A roar: a swelling, open-mouthed call that peaks and falls about an octave, a lion's at size 0.75, a dragon's at 1.",
    Call {
        f0: 230.0,
        contour: [(0.0, -6.0), (0.35, 0.0), (0.7, -2.0), (1.0, -11.0)],
        secs: 1.6,
        size_secs: 0.35,
        attack: 0.35,
        hold: 0.2,
        decay: 0.9,
        vowel: [3.2, 0.2, 3.0],
        q: 0.7,
        sub: 0.3,
        sub_n: 2,
        jitter: 0.03,
        breath: 0.18,
        tilt: 2400.0,
        level: 1.26,
        ..CALL
    });
creature_call!(Hiss, "creature_hiss", "A hiss: a spat onset into a long breath of noise, with a growl under it as aggression rises.",
    Call {
        f0: 150.0,
        contour: [(0.0, 0.0), (1.0, -2.0), (1.0, -2.0), (1.0, -2.0)],
        secs: 0.9,
        size_secs: 0.25,
        attack: 0.08,
        hold: 0.6,
        decay: 0.3,
        vowel: [3.8, 3.8, 3.8],
        sub: 0.4,
        sub_n: 2,
        jitter: 0.05,
        breath: 0.0,
        hiss: 1.0,
        hiss_hz: 1400.0,
        hiss_env: [0.03, 0.55, 0.3],
        tilt: 600.0,
        level: 0.62,
        ..CALL
    });
creature_call!(Squeak, "creature_squeak", "A squeak: a short tonal arch, a mouse's at size 0 (4..5 kHz); excitement squeaks two or three times.",
    Call {
        source: Source::Tone,
        f0: 1300.0,
        size_exp: 0.85,
        contour: [(0.0, 0.0), (0.4, 4.0), (1.0, -2.0), (1.0, -2.0)],
        secs: 0.11,
        size_secs: 0.2,
        attack: 0.15,
        hold: 0.55,
        decay: 0.035,
        count: [1.0, 3.0],
        rate: 7.0,
        step: -0.8,
        fm: 1.0,
        level: 0.69,
        ..CALL
    });
creature_call!(ChirpCall, "creature_chirp", "A bird-like chirp: a fast falling sweep; excitement chirps up to four times.",
    Call {
        source: Source::Tone,
        f0: 1900.0,
        size_exp: 0.6,
        contour: [(0.0, 8.0), (0.15, 6.0), (1.0, 0.0), (1.0, 0.0)],
        secs: 0.07,
        size_secs: 0.2,
        attack: 0.08,
        hold: 0.45,
        decay: 0.03,
        count: [1.0, 4.0],
        rate: 9.0,
        step: -0.7,
        fm: 0.6,
        level: 0.68,
        ..CALL
    });
creature_call!(Chatter, "creature_chatter", "Small-critter chatter: a quick burst of clicky little syllables (a squirrel, an imp); excitement makes it longer and faster.",
    Call {
        f0: 520.0,
        size_exp: 0.8,
        contour: [(0.0, 3.0), (1.0, -2.0), (1.0, -2.0), (1.0, -2.0)],
        secs: 0.045,
        size_secs: 0.2,
        attack: 0.1,
        hold: 0.4,
        decay: 0.03,
        vowel: [1.6, 2.0, 1.4],
        q: 1.0,
        sub: 0.1,
        jitter: 0.02,
        breath: 0.25,
        hiss: 0.5,
        hiss_hz: 3500.0,
        hiss_env: [0.004, 0.008, 0.02],
        tilt: 4000.0,
        count: [3.0, 10.0],
        rate: 14.0,
        step: 0.0,
        level: 1.74,
        ..CALL
    });

model_params! {
    /// A creature's voice, shared by every `creature_*` call; the presets are species. Size,
    /// aggression and excitement are inputs, so one species can be a pup or a giant.
    CreatureParams / CreatureParamId {
        pitch: "body/pitch_semitones" = 0.0, lin(-24.0, 24.0);
        throat: "body/throat" = 1.0, exp(0.5, 2.0);
        rough: "voice/roughness" = 0.5, UNIT;
        breath: "voice/breath" = 0.5, UNIT;
        length: "shape/length" = 1.0, exp(0.5, 2.0);
        variation: "shape/variation" = 0.5, UNIT;
        room: "space/room" = 0.25, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

fn species(pitch: f32, throat: f32, rough: f32, breath: f32, length: f32, room: f32) -> CreatureParams {
    CreatureParams { pitch, throat, rough, breath, length, room, ..Default::default() }
}

/// Species presets (at size 0.5; `size` still scales them).
fn species_presets() -> Vec<(&'static str, CreatureParams)> {
    vec![
        ("Cat", species(7.0, 1.4, 0.3, 0.6, 0.9, 0.2)),
        ("Big cat", species(-4.0, 0.8, 0.6, 0.5, 1.1, 0.3)),
        ("Bear", species(-8.0, 0.7, 0.8, 0.6, 1.2, 0.3)),
        ("Dragon", species(-10.0, 0.6, 0.9, 0.7, 1.4, 0.5)),
        ("Critter", species(4.0, 1.5, 0.15, 0.3, 0.8, 0.15)),
        ("Monster", species(-5.0, 0.75, 1.0, 0.8, 1.1, 0.35)),
    ]
}

/// Highest tone a call reaches (Hz).
const TONE_TOP: f32 = 9000.0;
/// Output level of the creature calls (calibrated against the other one-shot events).
const CREATURE_GAIN: f32 = 2.0;

/// Renders any [`Call`]; one per generator instance.
pub struct CreatureVoice {
    sr: f32,
    seed: u32,
    throat: Throat,
    /// The same voice through cascade formants, for calls that want it.
    dark: Throat,
    contour: Contour,
    env: Envelope,
    hiss_env: Envelope,
    hiss: Svf,
    hiss_hp: OnePole,
    hiss_hp_coef: f32,
    hiss_lp: [OnePole; 2],
    hiss_lp_coef: f32,
    noise: Noise,
    rng: Rng,
    pattern: Pattern,
    rhythm: Rhythm,
    air: Air,
    room: Room,
    // This call.
    call: Call,
    shape: Shape,
    f0: f32,
    secs: f32,
    depth: f32,
    level: f32,
    voiced: f32,
    breath: f32,
    hiss_level: f32,
    pulse: f32,
    pulse_hz: f32,
    pulse_phase: f32,
    drive: f32,
    /// This syllable's FM index (less on high tones).
    fm: f32,
    /// Seconds since the trigger, the series window, syllables so far, seconds into this one.
    t: f32,
    window: f32,
    fired: u32,
    syl_t: f32,
    room_send: f32,
    room_time: f32,
    active: bool,
    pitch_ratio: f32,
}

/// Creature call inputs, in order.
const CREATURE_INPUTS: &[InputSpec] = &[
    InputSpec { name: "power", default: 1.0, doc: "A murmur .. full voice: level and brightness" },
    InputSpec { name: "distance", default: 0.0, doc: "0 = right here, 1 = far off (duller, quieter)" },
    InputSpec { name: "size", default: 0.5, doc: "0 a mouse .. 0.25 a cat .. 0.5 a wolf .. 0.75 a lion or bear .. 1 a dragon: lower, longer, bigger throat" },
    InputSpec { name: "aggression", default: 0.5, doc: "0 calm .. 1 furious: rougher, harsher, more breath, louder" },
    InputSpec { name: "excitement", default: 0.5, doc: "0 lazy .. 1 agitated: higher, quicker, more syllables, more pitch movement" },
];

impl CreatureVoice {
    pub fn new(sr: f32, seed: u32) -> Self {
        CreatureVoice {
            sr,
            seed,
            throat: Throat::new(seed, false),
            dark: Throat::new(seed, true),
            contour: Contour::default(),
            env: Envelope::default(),
            hiss_env: Envelope::default(),
            hiss: Svf::default(),
            hiss_hp: OnePole::default(),
            hiss_hp_coef: 0.0,
            hiss_lp: [OnePole::default(); 2],
            hiss_lp_coef: 1.0,
            noise: Noise::new(mix_seed(seed ^ 0x4155)),
            rng: Rng::new(mix_seed(seed ^ 0x5A5A)),
            pattern: Pattern::new(seed ^ 0x9A77, None),
            rhythm: Rhythm { rate: 4.0, count: 1.0, gap: 60.0, jitter: 0.0, variation: 0.0 },
            air: Air::default(),
            room: Room::new(sr),
            call: CALL,
            shape: Shape::default(),
            f0: 100.0,
            secs: 1.0,
            depth: 1.0,
            level: 0.0,
            voiced: 1.0,
            breath: 0.0,
            hiss_level: 0.0,
            pulse: 0.0,
            pulse_hz: 6.0,
            pulse_phase: 0.0,
            drive: 0.0,
            fm: 0.0,
            t: 0.0,
            window: 0.0,
            fired: 0,
            syl_t: 0.0,
            room_send: 0.0,
            room_time: 0.8,
            active: false,
            pitch_ratio: 1.0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn set_pitch_ratio(&mut self, ratio: f32) {
        self.pitch_ratio = ratio;
    }

    fn room_time(p: &CreatureParams) -> f32 {
        0.5 + 1.5 * p.room
    }

    pub fn trigger(&mut self, call: &Call, p: &CreatureParams, x: &[f32]) {
        let (power, distance, size, aggr, exc) = (x[0], x[1], x[2], x[3], x[4]);
        let v = p.variation.clamp(0.0, 1.0);
        if v <= 0.0 {
            self.rng = Rng::new(mix_seed(self.seed ^ 0x5A5A));
            self.noise = Noise::new(mix_seed(self.seed ^ 0x4155));
            self.throat.reseed(self.seed);
            self.dark.reseed(self.seed);
            self.pattern = Pattern::new(self.seed ^ 0x9A77, None);
            if !self.active {
                self.room.clear();
                self.throat.reset();
                self.dark.reset();
            }
        }
        let s = size_scale(size);
        let rough = 0.4 + 1.2 * p.rough;
        let breathy = 0.3 + 1.4 * p.breath;
        self.call = *call;
        self.f0 = call.f0 * semis(p.pitch + 6.0 * (exc - 0.5) + v * 0.6 * self.rng.next_bipolar()) / s.powf(call.size_exp);
        self.secs = call.secs * s.powf(call.size_secs) * p.length * (1.3 - 0.6 * exc) * (1.0 + 0.15 * v * self.rng.next_bipolar());
        self.depth = 0.75 + 0.5 * exc;
        let open = 0.35 * aggr;
        self.shape = Shape {
            vowel: call.vowel[0],
            shift: p.throat / s.powf(0.6),
            q: call.q * (1.0 + 0.3 * aggr),
            sub: ((call.sub + 0.35 * aggr) * rough).min(0.9),
            sub_n: call.sub_n,
            jitter: (call.jitter * (1.0 + 1.5 * aggr) * rough).min(0.25),
            shimmer: 0.1 * rough,
            tilt_hz: call.tilt * (1.0 + aggr) * (0.6 + 0.4 * power),
            wave: Waveform::Saw,
        };
        // Aggression opens the mouth (towards "a") and presses the voice.
        self.call.vowel = call.vowel.map(|vw| vw + (0.0 - vw) * open);
        self.breath = (call.breath + 0.12 * aggr) * breathy;
        self.voiced = if call.hiss > 0.0 { 0.6 + 1.6 * aggr } else { 1.0 };
        self.hiss_level = call.hiss * (0.6 + 0.8 * aggr);
        let hiss_hz = (call.hiss_hz * (1.0 + 0.6 * aggr) * p.throat / s.powf(0.4)).min(0.4 * self.sr);
        self.hiss.set(FilterMode::BandPass, (2.2 * hiss_hz).min(0.42 * self.sr), 0.2, self.sr);
        self.hiss_hp_coef = hz_coef(hiss_hz, self.sr);
        self.hiss_lp_coef = hz_coef((3.0 * hiss_hz).min(0.42 * self.sr), self.sr);
        self.pulse = call.pulse * exc;
        self.pulse_hz = 4.5 + 3.0 * exc;
        self.drive = 0.3 * aggr * rough.min(1.0);
        self.level = call.level * (0.25 + 0.75 * power) * (0.7 + 0.5 * aggr) * s.powf(0.15) * (1.0 + 0.15 * v * self.rng.next_bipolar());
        let count = call.count[0] + (call.count[1] - call.count[0]) * exc;
        let rate = call.rate * (0.8 + 0.4 * exc);
        self.rhythm = Rhythm { rate, count: count.round().max(1.0), gap: 60.0, jitter: 0.2 * v, variation: 0.3 * v };
        self.window = count.round().max(1.0) * 1.2 / rate;
        self.pattern.restart();
        self.fired = 0;
        self.t = 0.0;
        self.room_send = 0.5 * p.room;
        self.room_time = Self::room_time(p);
        far(&mut self.air, distance, self.sr);
        self.active = true;
    }

    /// Start syllable `k` of the call with velocity `vel`.
    fn syllable(&mut self, vel: f32, v: f32) {
        let c = &self.call;
        let k = self.fired as f32;
        let base = self.f0 * semis(c.step * k + if k > 0.0 { 1.2 * v * self.rng.next_bipolar() } else { 0.0 }) * self.pitch_ratio;
        // Tones stay under 9 kHz (a mouse squeaks at 4..5), so their overtones never fold over.
        let top = TONE_TOP.min(0.2 * self.sr);
        let at = |i: usize| (c.contour[i].0 * self.secs, (base * semis(c.contour[i].1 * self.depth)).min(top));
        let from = (base * semis(c.contour[0].1 * self.depth)).min(top);
        self.fm = c.fm * (2500.0 / base).min(1.0);
        let pts = [at(1), at(2), at(3)];
        self.contour.start(from, &pts);
        self.env.trigger(vel * self.level);
        if self.hiss_level > 0.0 {
            self.hiss_env.trigger(vel);
        }
        self.syl_t = 0.0;
        self.fired += 1;
    }

    pub fn length(call: &Call, p: &CreatureParams) -> f32 {
        let s = size_scale(1.0).powf(call.size_secs.max(0.0));
        let secs = call.secs * s * p.length * 1.3 * (1.0 + 0.15 * p.variation);
        let decay = call.decay * s * p.length;
        let hiss = call.hiss_env[0] + call.hiss_env[1] * s + call.hiss_env[2];
        let count = call.count[0].max(call.count[1]).round().max(1.0);
        let series = (count - 1.0) * (1.0 + 0.2 * p.variation) / (call.rate * 0.8);
        let tail = if p.room > 0.0 { Self::room_time(p) } else { 0.0 };
        series + secs.max(hiss) + decay + tail + 2.0 * BLOCK as f32 / 8000.0 + SETTLE
    }

    pub fn render(&mut self, p: &CreatureParams, out: &mut [f32]) {
        let sr = self.sr;
        let dt = 1.0 / sr;
        let n = out.len();
        if !self.active {
            out.iter_mut().for_each(|s| *s = 0.0);
            return;
        }
        let v = p.variation.clamp(0.0, 1.0);
        let c = self.call;
        // The tract follows the vowel path of the current syllable.
        let u = (self.syl_t / self.secs.max(1e-3)).clamp(0.0, 1.0);
        let mut shape = self.shape;
        shape.vowel = if u < 0.4 { c.vowel[0] + (c.vowel[1] - c.vowel[0]) * u / 0.4 } else { c.vowel[1] + (c.vowel[2] - c.vowel[1]) * (u - 0.4) / 0.6 };
        shape.shift *= self.pitch_ratio;
        let throat = if c.series { &mut self.dark } else { &mut self.throat };
        throat.set(&shape, sr);
        let attack = c.attack * self.secs;
        let hold = c.hold * self.secs;
        let decay = c.decay * self.secs / c.secs.max(1e-3);
        let hs = self.secs / c.secs.max(1e-3);
        let (h_att, h_hold, h_dec) = (c.hiss_env[0], c.hiss_env[1] * hs, c.hiss_env[2] * hs.sqrt());
        let gain = CREATURE_GAIN * p.gain;
        let mut sounding = false;
        for o in out.iter_mut() {
            self.t += dt;
            self.syl_t += dt;
            let imp = self.pattern.tick(self.t < self.window, &self.rhythm, sr);
            if imp > 0.0 {
                self.syllable(imp, v);
            }
            let e = self.env.tick(dt, attack, hold, decay, 0.0, 0.05);
            let mut y = 0.0;
            if e > 0.0 {
                y = match c.source {
                    Source::Throat => {
                        let f = self.contour.step(sr);
                        let mut a = e;
                        if self.pulse > 0.0 {
                            self.pulse_phase = (self.pulse_phase + self.pulse_hz * dt).fract();
                            a *= 1.0 - self.pulse * (0.5 - 0.5 * (TAU * self.pulse_phase).cos());
                        }
                        let throat = if c.series { &mut self.dark } else { &mut self.throat };
                        throat.tick(f, a * self.voiced, a * self.breath, sr)
                    }
                    Source::Tone => self.contour.tone(1.0, self.fm, sr) * e * 0.35,
                };
            } else if c.source == Source::Throat {
                self.contour.step(sr);
            }
            let h = self.hiss_env.tick(dt, h_att, h_hold, h_dec, 0.0, 0.05);
            if h > 0.0 {
                let w = self.noise.white();
                let hp = self.hiss_hp.hp(w, self.hiss_hp_coef);
                let band = self.hiss_lp[0].lp(0.55 * hp + 0.6 * self.hiss.tick(w), self.hiss_lp_coef);
                let band = self.hiss_lp[1].lp(band, self.hiss_lp_coef);
                y += band * h * self.hiss_level * self.level * 0.8;
            }
            sounding |= e > 0.0 || h > 0.0;
            *o = self.air.tick(rasp(y, self.drive)) * gain;
        }
        sounding |= self.t < self.window && self.fired < self.rhythm.count as u32;
        self.room.process(out, sounding, self.room_send, self.room_time, 0.45, sr);
        if !sounding && !self.room.is_ringing() && self.env.is_idle() && self.hiss_env.is_idle() {
            self.active = false;
            self.throat.reset();
            self.dark.reset();
            self.air.reset();
            self.hiss_hp = OnePole::default();
            self.hiss_lp = [OnePole::default(); 2];
            self.hiss.reset();
            self.room.clear();
        }
        debug_assert!(n <= BLOCK);
    }
}

/// Adapter: any [`CreatureCall`] as a one-shot [`Generator`].
pub struct Creature<C: CreatureCall> {
    voice: CreatureVoice,
    call: PhantomData<C>,
}

impl<C: CreatureCall> Generator for Creature<C> {
    type P = CreatureParams;
    const NAME: &'static str = C::NAME;
    const CATEGORY: &'static str = "creatures";
    const DOC: &'static str = C::DOC;
    const INPUTS: &'static [InputSpec] = CREATURE_INPUTS;
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const ONE_SHOT: bool = true;
    const TRANSPOSES: bool = true;

    fn presets() -> Vec<(&'static str, CreatureParams)> {
        species_presets()
    }

    fn new(sr: f32) -> Self {
        let seed = C::NAME.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
        Creature { voice: CreatureVoice::new(sr, seed), call: PhantomData }
    }

    fn trigger(&mut self, x: &[f32], p: &CreatureParams) {
        self.voice.trigger(&C::CALL, p, x);
    }

    fn is_active(&self) -> bool {
        self.voice.is_active()
    }

    fn length(p: &CreatureParams) -> Option<f32> {
        Some(CreatureVoice::length(&C::CALL, p))
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        self.voice.set_pitch_ratio(ratio);
    }

    fn block(&mut self, _x: &[f32], p: &CreatureParams, out: &mut [f32]) {
        self.voice.render(p, out);
    }
}

// ---------------------------------------------------------------------------------------------
// creature_idle: breathing and purring
// ---------------------------------------------------------------------------------------------

/// Purr pulse rate (Hz) of a cat; measured 25..29 Hz on three recordings.
const PURR_HZ: f32 = 24.0;
const IDLE_GAIN: f32 = 1.2;

/// A creature at rest: breath in and out (calm through the nose .. panting), and a purr on both
/// breaths. Continuous.
pub struct Idle {
    sr: f32,
    throat: Throat,
    rng: Rng,
    room: Room,
    /// Breath cycle phase 0..1 and this cycle's rate and depth factors.
    phase: f32,
    rate_k: f32,
    depth_k: f32,
}

impl Idle {
    /// Level of the breath at cycle phase `ph`: out (0 .. 0.45) then in (0.5 .. 0.88).
    fn breath_window(ph: f32) -> (f32, bool) {
        if ph < 0.45 {
            let w = (core::f32::consts::PI * ph / 0.45).sin();
            (w * w, false)
        } else if (0.5..0.88).contains(&ph) {
            let w = (core::f32::consts::PI * (ph - 0.5) / 0.38).sin();
            (w * w, true)
        } else {
            (0.0, false)
        }
    }

    /// The purr runs through both breaths with short turns between them.
    fn purr_window(ph: f32) -> f32 {
        let edge = |x: f32| smoothstep(0.0, 0.06, x);
        if ph < 0.47 {
            edge(ph) * edge(0.47 - ph)
        } else if ph < 0.97 {
            edge(ph - 0.5) * edge(0.97 - ph)
        } else {
            0.0
        }
    }
}

impl Generator for Idle {
    type P = CreatureParams;
    const NAME: &'static str = "creature_idle";
    const CATEGORY: &'static str = "creatures";
    const DOC: &'static str = "A creature at rest: breathing (calm .. panting with `excitement`) and purring (`purr`), \
        scaled by `size` from a mouse to a dragon. Continuous; use the creature species presets.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "excitement", default: 0.2, doc: "0 asleep (slow, soft) .. 1 panting (fast, loud, open-mouthed)" },
        InputSpec { name: "purr", default: 0.0, doc: "0 breathing only .. 1 a full purr on both breaths" },
        InputSpec { name: "size", default: 0.3, doc: "0 a mouse .. 0.25 a cat .. 1 a dragon: lower and slower" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.3;

    fn presets() -> Vec<(&'static str, CreatureParams)> {
        species_presets()
    }

    fn new(sr: f32) -> Self {
        Idle { sr, throat: Throat::new(0x1D1E, true), rng: Rng::new(mix_seed(0x1D1F)), room: Room::new(sr), phase: 0.0, rate_k: 1.0, depth_k: 1.0 }
    }

    fn block(&mut self, x: &[f32], p: &CreatureParams, out: &mut [f32]) {
        let sr = self.sr;
        let dt = 1.0 / sr;
        let (exc, purr, size) = (x[0], x[1], x[2]);
        let s = size_scale(size);
        let v = p.variation.clamp(0.0, 1.0);
        let rate = (0.22 + 2.4 * exc.powf(1.3)) / (s.powf(0.3) * p.length) * self.rate_k;
        let (_, inhale) = Self::breath_window(self.phase);
        let pant = exc * exc;
        let shape = Shape {
            // Calm breath is through the nose (dark, "u"); panting through an open mouth ("a").
            vowel: 3.8 - 3.3 * pant + (3.9 - (3.8 - 3.3 * pant)) * purr * 0.7,
            shift: p.throat / s.powf(0.6) * if inhale { 1.1 } else { 1.0 },
            q: 0.5,
            sub: 0.0,
            sub_n: 2,
            jitter: 0.04 + 0.06 * p.rough,
            shimmer: 0.15,
            tilt_hz: 500.0 / s.powf(0.5),
            wave: Waveform::Saw,
        };
        self.throat.set(&shape, sr);
        let purr_hz = PURR_HZ * semis(0.1 * p.pitch) / s.powf(0.15);
        let breath_level = (0.15 + 0.85 * exc) * (0.3 + 1.4 * p.breath) * (1.0 - 0.6 * purr) * self.depth_k;
        for o in out.iter_mut() {
            self.phase += rate * dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                self.rate_k = 1.0 + 0.15 * v * self.rng.next_bipolar();
                self.depth_k = 1.0 + 0.25 * v * self.rng.next_bipolar();
            }
            let (w, inh) = Self::breath_window(self.phase);
            let pw = Self::purr_window(self.phase);
            let f = purr_hz * if inh { 1.04 } else { 1.0 };
            let b = w * breath_level * if inh { 0.7 + 0.3 * pant } else { 1.0 } * 0.25;
            let y = self.throat.tick(f, pw * purr * if inh { 0.75 } else { 1.0 }, b, sr);
            *o = y * IDLE_GAIN * p.gain;
        }
        self.room.process(out, true, 0.3 * p.room, 0.5 + 1.5 * p.room, 0.5, sr);
    }
}
