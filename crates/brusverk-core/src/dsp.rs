//! Higher-level DSP building blocks shared by the generators and the model-file graph: the
//! pieces that make textures, impacts, calls and placement sound real.
//!
//! * [`power_event`] / [`PowerDust`]: Poisson events whose sizes follow a power law.
//! * [`ModalBank`] / [`Modal`]: ringing modes with their own frequency, ring time and level.
//! * [`Chirp`] and [`FmOp`]: a triggered pitch glide and a two-operator FM voice.
//! * [`Envelope`]: click-free AD (triggered) and ADSR (gated) envelopes.
//! * [`Formant`]: two to four formants, parallel or Klatt-style series, with vowel presets.
//! * [`Pattern`]: impulses in phrases, with jitter, variation and Euclidean rhythms.
//! * [`equal_power`], [`balance`] and [`Air`]: stereo placement and distance.
//!
//! Every block allocates (if at all) in its constructor; `tick` never allocates, locks or
//! blocks. Triggered blocks share one convention ([`Onset`]): a trigger is a non-zero sample
//! after a zero one, so one-sample impulses (dust, patterns) and the rising edge of a gate
//! both fire once, and the size of that first sample is the trigger's velocity.

use crate::blocks::{hz_coef, mix_seed, OnePole};
use crate::filter::{FilterMode, Svf};
use crate::math::{Rng, PI, TAU};

/// The object's scale for a 0..1 `size`: 0.25 (two octaves up) .. 4 (two octaves down), with
/// 0.5 as written. Modes are divided by it in pitch and ring `scale^0.6` times longer.
#[inline]
pub fn size_scale(size: f32) -> f32 {
    ((size.clamp(0.0, 1.0) - 0.5) * 4.0).exp2()
}

// ---------------------------------------------------------------------------------------------
// Triggers
// ---------------------------------------------------------------------------------------------

/// Below this a trigger input counts as silent (a ramped gate rarely lands on exact zero).
pub const ONSET_FLOOR: f32 = 1e-6;

/// Onset detector: fires on a non-zero sample that follows a zero one.
#[derive(Clone, Copy, Debug, Default)]
pub struct Onset {
    high: bool,
}

impl Onset {
    /// The velocity (|x|) when `x` starts a trigger, else `None`.
    #[inline]
    pub fn tick(&mut self, x: f32) -> Option<f32> {
        let high = x.abs() > ONSET_FLOOR;
        let fire = high && !self.high;
        self.high = high;
        if fire {
            Some(x.abs())
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Power-law dust
// ---------------------------------------------------------------------------------------------

/// One draw of power-law dust from `rng`: with probability `p` an event of size `u^skew`
/// (`u` uniform in 0..1), else `None`. Sizes then follow `P(size <= x) = x^(1 / skew)`: at
/// skew 1 they are uniform, at 3 most are tiny and a few are big (crackle, gravel, drops), at
/// 0 they are all 1. Uniform sizes are the most common reason a texture sounds fake.
#[inline]
pub fn power_event(rng: &mut Rng, p: f32, skew: f32) -> Option<f32> {
    if rng.next_f32() < p {
        Some(rng.next_f32().powf(skew))
    } else {
        None
    }
}

/// Poisson impulses with power-law sizes; see [`power_event`].
#[derive(Clone, Copy, Debug)]
pub struct PowerDust {
    rng: Rng,
}

impl PowerDust {
    pub fn new(seed: u32) -> Self {
        PowerDust { rng: Rng::new(mix_seed(seed)) }
    }

    /// `p` is the per-sample probability (rate / sample rate). Returns 0 or the event size.
    #[inline]
    pub fn tick(&mut self, p: f32, skew: f32) -> f32 {
        power_event(&mut self.rng, p, skew.max(0.0)).unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------------------------------------
// Modal synthesis
// ---------------------------------------------------------------------------------------------

/// Most modes a [`ModalBank`] holds.
pub const MAX_MODES: usize = 16;

/// A bank of decaying sinusoids, each a complex one-pole resonator (four multiplies a sample).
/// An impulse of size `a` rings mode `k` as a sine of amplitude `a * gain(k)` that starts at
/// zero (no click). Input adds to whatever is still ringing. Modes that have died away are
/// skipped until something wakes them.
#[derive(Clone, Copy, Debug)]
pub struct ModalBank {
    re: [f32; MAX_MODES],
    im: [f32; MAX_MODES],
    /// Pole: radius times cos / sin of the step angle.
    c: [f32; MAX_MODES],
    s: [f32; MAX_MODES],
    /// Excitation gain of each mode.
    g: [f32; MAX_MODES],
    live: [bool; MAX_MODES],
    n: usize,
}

impl Default for ModalBank {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalBank {
    pub const fn new() -> Self {
        ModalBank {
            re: [0.0; MAX_MODES],
            im: [0.0; MAX_MODES],
            c: [0.0; MAX_MODES],
            s: [0.0; MAX_MODES],
            g: [0.0; MAX_MODES],
            live: [false; MAX_MODES],
            n: 0,
        }
    }

    /// Number of modes in use.
    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn set_len(&mut self, n: usize) {
        self.n = n.min(MAX_MODES);
    }

    /// Tune mode `k` to `hz` with ring time `t60` (seconds to fall 60 dB). Modes outside
    /// 0 < hz < 0.45 sr are silenced (their gain is zeroed).
    pub fn tune(&mut self, k: usize, hz: f32, t60: f32, sr: f32) {
        if !(hz > 0.0 && hz < 0.45 * sr) {
            self.c[k] = 0.0;
            self.s[k] = 0.0;
            self.g[k] = 0.0;
            return;
        }
        let r = (-6.91 / (t60.max(1e-3) * sr)).exp();
        let w = TAU * hz / sr;
        self.c[k] = r * w.cos();
        self.s[k] = r * w.sin();
    }

    /// Whether mode `k` is in band (see [`ModalBank::tune`]).
    #[inline]
    pub fn is_tuned(&self, k: usize) -> bool {
        self.c[k] != 0.0 || self.s[k] != 0.0
    }

    #[inline]
    pub fn gain(&self, k: usize) -> f32 {
        self.g[k]
    }

    #[inline]
    pub fn set_gain(&mut self, k: usize, g: f32) {
        self.g[k] = g;
    }

    /// Mark mode `k` as ringing (it is computed) or not (skipped).
    #[inline]
    pub fn set_live(&mut self, k: usize, live: bool) {
        self.live[k] = live;
    }

    pub fn clear(&mut self) {
        self.re = [0.0; MAX_MODES];
        self.im = [0.0; MAX_MODES];
        self.live = [false; MAX_MODES];
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let mut y = 0.0;
        for k in 0..self.n {
            if !self.live[k] {
                continue;
            }
            let (re, im) = (self.re[k], self.im[k]);
            self.re[k] = self.c[k] * re - self.s[k] * im + self.g[k] * x;
            self.im[k] = self.s[k] * re + self.c[k] * im;
            y += self.im[k];
        }
        y
    }

    /// Sum of mode amplitudes; retires modes that have fallen below `floor`.
    pub fn energy(&mut self, floor: f32) -> f32 {
        let mut sum = 0.0;
        for k in 0..self.n {
            if !self.live[k] {
                continue;
            }
            let a = self.re[k].abs() + self.im[k].abs();
            if a < floor {
                self.live[k] = false;
                self.re[k] = 0.0;
                self.im[k] = 0.0;
            } else {
                sum += a;
            }
        }
        sum
    }
}

/// A struck object: a [`ModalBank`] tuned from a table of modes (frequency ratio, ring time,
/// level) and driven by an excitation signal.
///
/// * `size` (0..1, 0.5 as written) moves every mode down in pitch and up in ring time.
/// * Each trigger ([`Onset`]) picks where the object is struck: mode `k`'s level follows its
///   shape `0.25 + 0.75 |sin(pi (k + 1) pos)|` there, and `variation` moves the point and
///   detunes (±0.6 %) and re-times (±30 %) the modes a little, so no two hits are the same.
/// * A new hit adds to the modes that still ring; nothing restarts.
#[derive(Clone, Debug)]
pub struct Modal {
    bank: ModalBank,
    ratio: [f32; MAX_MODES],
    t60: [f32; MAX_MODES],
    level: [f32; MAX_MODES],
    /// This strike's level (shape), detune and ring factor per mode.
    gain: [f32; MAX_MODES],
    detune: [f32; MAX_MODES],
    ring: [f32; MAX_MODES],
    onset: Onset,
    rng: Rng,
    /// (freq, size, decay) the bank is tuned for.
    tuned: (f32, f32, f32),
    awake: bool,
}

impl Modal {
    /// Up to [`MAX_MODES`] modes; `t60` and `level` are cycled if shorter than `ratios`.
    pub fn new(ratios: &[f32], t60: &[f32], level: &[f32], seed: u32) -> Self {
        let n = ratios.len().min(MAX_MODES);
        let pick = |v: &[f32], k: usize, d: f32| if v.is_empty() { d } else { v[k % v.len()] };
        let mut m = Modal {
            bank: ModalBank::new(),
            ratio: [0.0; MAX_MODES],
            t60: [0.0; MAX_MODES],
            level: [0.0; MAX_MODES],
            gain: [0.0; MAX_MODES],
            detune: [1.0; MAX_MODES],
            ring: [1.0; MAX_MODES],
            onset: Onset::default(),
            rng: Rng::new(mix_seed(seed)),
            tuned: (f32::NAN, f32::NAN, f32::NAN),
            awake: false,
        };
        m.bank.set_len(n);
        for (k, &ratio) in ratios.iter().enumerate().take(n) {
            m.ratio[k] = ratio;
            m.t60[k] = pick(t60, k, 1.0);
            m.level[k] = pick(level, k, 1.0);
        }
        m.strike(0.23, 0.0);
        m
    }

    pub fn len(&self) -> usize {
        self.bank.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bank.is_empty()
    }

    /// Retune for this block when `freq` (Hz of ratio 1), `size` or `decay` (ring-time
    /// multiplier) moved.
    pub fn set(&mut self, freq: f32, size: f32, decay: f32, sr: f32) {
        let key = (freq, size, decay);
        if key != self.tuned {
            self.tuned = key;
            self.retune(sr);
        }
    }

    fn retune(&mut self, sr: f32) {
        let (freq, size, decay) = self.tuned;
        let s = size_scale(size);
        let (tune, ring) = (freq / s, decay.max(0.0) * s.powf(0.6));
        for k in 0..self.bank.len() {
            let t60 = (self.t60[k] * ring * self.ring[k]).clamp(0.004, 30.0);
            self.bank.tune(k, self.ratio[k] * tune * self.detune[k], t60, sr);
            self.bank.set_gain(k, if self.bank.is_tuned(k) { self.gain[k] } else { 0.0 });
        }
    }

    /// Roll a new strike: position `pos` (0..0.5 from the edge to the middle) moved by up to
    /// `variation` (0..1) towards a random point.
    pub fn strike(&mut self, pos: f32, variation: f32) {
        let v = variation.clamp(0.0, 1.0);
        let pos = pos + v * (self.rng.range(0.06, 0.5) - pos);
        for k in 0..self.bank.len() {
            let shape = 0.25 + 0.75 * (PI * (k as f32 + 1.0) * pos).sin().abs();
            self.gain[k] = self.level[k] * shape;
            self.detune[k] = 1.0 + v * 0.006 * self.rng.next_bipolar();
            self.ring[k] = 1.0 + v * 0.3 * self.rng.next_bipolar();
        }
    }

    /// One sample: `x` excites the modes; a trigger in `x` first rolls a new strike.
    #[inline]
    pub fn tick(&mut self, x: f32, pos: f32, variation: f32, sr: f32) -> f32 {
        if self.onset.tick(x).is_some() {
            self.strike(pos, variation);
            if self.tuned.0.is_finite() {
                self.retune(sr);
            }
        }
        if x != 0.0 && !self.awake {
            self.awake = true;
            for k in 0..self.bank.len() {
                self.bank.live[k] |= self.bank.g[k] != 0.0;
            }
        }
        self.bank.tick(x)
    }

    /// Call once per block: retires modes that have rung out, so silence is cheap.
    pub fn end_block(&mut self) -> f32 {
        self.awake = false;
        self.bank.energy(1e-7)
    }

    pub fn clear(&mut self) {
        self.bank.clear();
        self.onset = Onset::default();
        self.awake = false;
    }
}

// ---------------------------------------------------------------------------------------------
// Chirp and FM
// ---------------------------------------------------------------------------------------------

/// A two-operator FM voice: a sine carrier whose phase is modulated by a sine at `ratio` times
/// its frequency, `index` radians deep. Both phases start at zero and run locked, so the
/// waveform stays odd-symmetric and sidebands that fold onto 0 Hz (ratio 1 or 0.5) add no DC.
#[derive(Clone, Copy, Debug, Default)]
pub struct FmOp {
    carrier: f32,
    modulator: f32,
}

impl FmOp {
    /// `inc` is the carrier frequency over the sample rate.
    #[inline]
    pub fn tick(&mut self, inc: f32, ratio: f32, index: f32) -> f32 {
        self.carrier = (self.carrier + inc).fract();
        // The modulator always runs, so it stays locked to the carrier when the index returns.
        self.modulator = (self.modulator + inc * ratio).fract();
        if index == 0.0 {
            return (TAU * self.carrier).sin();
        }
        (TAU * self.carrier + index * (TAU * self.modulator).sin()).sin()
    }

    pub fn reset(&mut self) {
        *self = FmOp::default();
    }
}

/// A triggered pitch glide: after [`Chirp::start`] the frequency moves from `from` to `to` Hz
/// in `time` seconds, evenly in octaves when `curve` is 1; above 1 it lingers at the start and
/// falls late, below 1 it moves early and settles. Then it holds `to`. The oscillator is an
/// [`FmOp`], so a chirp can buzz or warble; its phase never jumps, so a retrigger is click-free.
#[derive(Clone, Copy, Debug)]
pub struct Chirp {
    op: FmOp,
    from: f32,
    octaves: f32,
    /// Seconds since the start, and the glide time.
    t: f32,
    time: f32,
    curve: f32,
}

impl Default for Chirp {
    fn default() -> Self {
        Chirp { op: FmOp::default(), from: 440.0, octaves: 0.0, t: 0.0, time: 0.1, curve: 1.0 }
    }
}

impl Chirp {
    pub fn start(&mut self, from: f32, to: f32, time: f32, curve: f32) {
        let (from, to) = (from.clamp(1.0, 24000.0), to.clamp(1.0, 24000.0));
        self.from = from;
        self.octaves = (to / from).log2();
        self.t = 0.0;
        self.time = time.max(1e-4);
        self.curve = curve.clamp(0.05, 20.0);
    }

    /// Current frequency in Hz.
    #[inline]
    pub fn freq(&self) -> f32 {
        let u = (self.t / self.time).min(1.0);
        let u = if self.curve == 1.0 { u } else { u.powf(self.curve) };
        self.from * (self.octaves * u).exp2()
    }

    #[inline]
    pub fn tick(&mut self, ratio: f32, index: f32, sr: f32) -> f32 {
        let inc = (self.freq() / sr).min(0.45);
        self.t += 1.0 / sr;
        self.op.tick(inc, ratio, index)
    }
}

// ---------------------------------------------------------------------------------------------
// Envelopes
// ---------------------------------------------------------------------------------------------

/// Decays aim this fraction of the peak below their target, so they land on it (or on zero) in
/// finite time with no step: the end of a sound is exactly silent, with no tail of DC.
const ENV_EPS: f32 = 1e-3;
/// Shortest attack: half a millisecond is the fastest onset that does not click.
pub const MIN_ATTACK: f32 = 5e-4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Idle,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

/// Click-free envelope for AD (triggered: attack, hold, decay to zero) and ADSR (gated).
///
/// The attack is an S-curve from wherever the level is to the peak (zero slope at both ends),
/// so a retrigger mid-sound never jumps. Decay and release are exponential with their time
/// as the 60 dB time, and land exactly on the sustain level or zero.
#[derive(Clone, Copy, Debug, Default)]
pub struct Envelope {
    stage: Stage,
    level: f32,
    from: f32,
    peak: f32,
    /// Seconds into the current stage.
    pos: f32,
}

impl Envelope {
    /// Start the attack towards `peak` from the current level.
    pub fn trigger(&mut self, peak: f32) {
        self.from = self.level;
        self.peak = peak.max(0.0);
        self.stage = Stage::Attack;
        self.pos = 0.0;
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    #[inline]
    pub fn level(&self) -> f32 {
        self.level
    }

    #[inline]
    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn reset(&mut self) {
        *self = Envelope::default();
    }

    /// One sample, `dt` = 1 / sample rate. Times in seconds; `sustain` 0..1 of the peak (0 for
    /// an AD envelope, which then ends after its decay).
    #[inline]
    pub fn tick(&mut self, dt: f32, attack: f32, hold: f32, decay: f32, sustain: f32, release: f32) -> f32 {
        let fall = |level: &mut f32, target: f32, secs: f32, floor: f32| -> bool {
            *level += (target - *level) * (1.0 - (-6.91 * dt / secs.max(1e-3)).exp());
            if *level <= floor {
                *level = floor;
                true
            } else {
                false
            }
        };
        match self.stage {
            Stage::Idle | Stage::Sustain => {}
            Stage::Attack => {
                self.pos += dt;
                let u = self.pos / attack.max(MIN_ATTACK);
                if u >= 1.0 {
                    self.level = self.peak;
                    self.pos = 0.0;
                    self.stage = Stage::Hold;
                } else {
                    self.level = self.from + (self.peak - self.from) * u * u * (3.0 - 2.0 * u);
                }
            }
            Stage::Hold => {
                self.pos += dt;
                if self.pos >= hold {
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                let s = sustain.clamp(0.0, 1.0) * self.peak;
                if fall(&mut self.level, s - ENV_EPS * self.peak, decay, s) {
                    self.stage = if s > 0.0 { Stage::Sustain } else { Stage::Idle };
                }
            }
            Stage::Release => {
                let target = -ENV_EPS * self.peak.max(self.level);
                if fall(&mut self.level, target, release, 0.0) {
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

// ---------------------------------------------------------------------------------------------
// Formants
// ---------------------------------------------------------------------------------------------

/// Most formants a [`Formant`] filter holds.
pub const MAX_FORMANTS: usize = 4;

/// Vowel presets a, e, i, o, u: three formants as (Hz, bandwidth Hz, level dB) for an adult
/// voice (after Peterson & Barney), the levels relative to F1 and flattened for a buzzy source.
pub const VOWELS: [[(f32, f32, f32); 3]; 5] = [
    [(730.0, 80.0, 0.0), (1090.0, 90.0, -3.0), (2440.0, 120.0, -12.0)],
    [(530.0, 60.0, 0.0), (1840.0, 100.0, -8.0), (2480.0, 120.0, -12.0)],
    [(270.0, 60.0, 0.0), (2290.0, 100.0, -12.0), (3010.0, 120.0, -12.0)],
    [(570.0, 70.0, 0.0), (840.0, 80.0, -5.0), (2410.0, 120.0, -18.0)],
    [(300.0, 60.0, 0.0), (870.0, 80.0, -10.0), (2240.0, 120.0, -22.0)],
];

/// Formants of vowel `v` (0 a, 1 e, 2 i, 3 o, 4 u; fractions morph between neighbours, in log
/// frequency) as (Hz, Q, linear level).
pub fn vowel(v: f32) -> [(f32, f32, f32); 3] {
    let v = if v.is_finite() { v.clamp(0.0, (VOWELS.len() - 1) as f32) } else { 0.0 };
    let (i, f) = ((v.floor() as usize).min(VOWELS.len() - 2), v - v.floor().min((VOWELS.len() - 2) as f32));
    core::array::from_fn(|k| {
        let (a, b) = (VOWELS[i][k], VOWELS[i + 1][k]);
        let hz = a.0 * (b.0 / a.0).powf(f);
        let bw = a.1 + (b.1 - a.1) * f;
        let db = a.2 + (b.2 - a.2) * f;
        (hz, hz / bw, crate::math::db_to_gain(db))
    })
}

/// SVF resonance for a Q (the SVF's damping is `k = 1 / Q = 2 - 1.98 res`).
#[inline]
fn q_to_res(q: f32) -> f32 {
    ((2.0 - 1.0 / q.clamp(0.5, 40.0)) / 1.98).clamp(0.0, 1.0)
}

/// Two to four formants. Parallel: a band-pass per formant, each normalised to unity gain at
/// its peak, times its level. Series (Klatt's cascade): resonant low-passes in a row, which
/// give the natural fall between formants by themselves; normalised so F1 peaks near 1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Formant {
    f: [Svf; MAX_FORMANTS],
    norm: [f32; MAX_FORMANTS],
    n: usize,
    series: bool,
}

impl Formant {
    pub fn new(series: bool) -> Self {
        Formant { series, ..Default::default() }
    }

    /// Set the formants for this block: (Hz, Q) each, at most [`MAX_FORMANTS`].
    pub fn set(&mut self, formants: &[(f32, f32)], sr: f32) {
        self.n = formants.len().min(MAX_FORMANTS);
        let mode = if self.series { FilterMode::LowPass } else { FilterMode::BandPass };
        for (k, &(hz, q)) in formants.iter().take(self.n).enumerate() {
            let res = q_to_res(q);
            self.f[k].set(mode, hz, res, sr);
            self.norm[k] = 2.0 - 1.98 * res;
        }
    }

    /// One sample; `levels` weights the formants in parallel mode (ignored in series).
    #[inline]
    pub fn tick(&mut self, x: f32, levels: &[f32; MAX_FORMANTS]) -> f32 {
        if self.series {
            let mut y = x;
            for f in self.f[..self.n].iter_mut() {
                y = f.tick(y);
            }
            return y * self.norm[0];
        }
        let mut y = 0.0;
        for ((f, norm), level) in self.f[..self.n].iter_mut().zip(&self.norm).zip(levels) {
            y += f.tick(x) * norm * level;
        }
        y
    }

    pub fn reset(&mut self) {
        self.f.iter_mut().for_each(|f| f.reset());
    }
}

// ---------------------------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------------------------

/// Whether step `i` of a Euclidean rhythm of `hits` in `steps` (rotated by `rot`) sounds.
#[inline]
pub fn euclid(i: u32, hits: u32, steps: u32, rot: u32) -> bool {
    let steps = steps.max(1);
    (((i + rot) % steps) * hits.min(steps)) % steps < hits.min(steps)
}

/// Rhythm settings for one [`Pattern::tick`].
#[derive(Clone, Copy, Debug)]
pub struct Rhythm {
    /// Events (steps) per second inside a phrase.
    pub rate: f32,
    /// Events per phrase (rounded, at least 1). Ignored with a Euclidean rhythm.
    pub count: f32,
    /// Extra seconds of silence between phrases; 0 runs phrases together in one stream.
    pub gap: f32,
    /// Timing jitter, 0..1 of a step.
    pub jitter: f32,
    /// 0..1: each phrase rolls its count (±50 %) and gap (±50 %); each event its level (-50 %).
    pub variation: f32,
}

/// Impulses in phrases: `count` events at `rate` per second, then `gap` seconds of rest, again
/// and again while it runs. A phrase lasts `count / rate` seconds, so the silence from a
/// phrase's last event to the next phrase's first is `1 / rate + gap`. With a Euclidean rhythm
/// (`hits` of `steps`) a phrase is one cycle of the steps and only the hits sound.
///
/// Rests follow the game: a rest lasts the gap rolled when it began, or the current gap if that
/// has since become shorter, so a long rest rolled while an input was low ends as soon as the
/// input rises; a rising gap never stretches a rest already under way.
#[derive(Clone, Copy, Debug)]
pub struct Pattern {
    rng: Rng,
    /// Samples since the last step.
    since: f32,
    step: u32,
    /// This phrase's count and the next wait's timing, as factors rolled by variation/jitter.
    count_k: f32,
    jitter_k: f32,
    gap_k: f32,
    /// The gap in samples when the current rest began.
    gap_at_rest: f32,
    resting: bool,
    running: bool,
    /// (hits, steps, rotation); steps 0 = plain phrases.
    euclid: (u32, u32, u32),
}

impl Pattern {
    pub fn new(seed: u32, euclid: Option<(u32, u32, u32)>) -> Self {
        Pattern {
            rng: Rng::new(mix_seed(seed)),
            since: 0.0,
            step: 0,
            count_k: 1.0,
            jitter_k: 1.0,
            gap_k: 1.0,
            gap_at_rest: 0.0,
            resting: false,
            running: false,
            euclid: euclid.unwrap_or((0, 0, 0)),
        }
    }

    /// Start a new phrase at the next sample.
    pub fn restart(&mut self) {
        self.running = false;
    }

    /// Steps in the current phrase.
    fn steps(&self, r: &Rhythm) -> u32 {
        if self.euclid.1 > 0 {
            self.euclid.1
        } else {
            (r.count * self.count_k).round().clamp(1.0, 10_000.0) as u32
        }
    }

    /// A factor of 1 ± `amount` / 2, or exactly 1 when there is no randomness.
    fn roll(&mut self, amount: f32) -> f32 {
        if amount > 0.0 {
            1.0 + 0.5 * amount.min(1.0) * self.rng.next_bipolar()
        } else {
            1.0
        }
    }

    /// Sound step `self.step` and move on; returns its impulse.
    fn fire(&mut self, r: &Rhythm, sr: f32) -> f32 {
        let v = r.variation.clamp(0.0, 1.0);
        let hit = self.euclid.1 == 0 || euclid(self.step, self.euclid.0, self.euclid.1, self.euclid.2);
        let level = if v > 0.0 { 1.0 - 0.5 * v * self.rng.next_f32() } else { 1.0 };
        self.step += 1;
        self.jitter_k = self.roll(r.jitter);
        self.resting = self.step >= self.steps(r);
        if self.resting {
            self.step = 0;
            self.count_k = self.roll(v);
            self.gap_k = self.roll(v);
            self.gap_at_rest = r.gap.max(0.0) * self.gap_k * sr;
        }
        if hit {
            level
        } else {
            0.0
        }
    }

    /// One sample: the impulse (0 or its level) of a pattern that plays while `run`.
    #[inline]
    pub fn tick(&mut self, run: bool, r: &Rhythm, sr: f32) -> f32 {
        if !run {
            self.running = false;
            return 0.0;
        }
        if !self.running {
            // A new phrase: its first step sounds at once.
            self.running = true;
            self.step = 0;
            self.since = 0.0;
            self.count_k = self.roll(r.variation.clamp(0.0, 1.0));
            return self.fire(r, sr);
        }
        self.since += 1.0;
        let interval = sr / r.rate.clamp(0.01, sr * 0.25);
        let need = if self.resting { interval + (r.gap.max(0.0) * self.gap_k * sr).min(self.gap_at_rest) } else { interval * self.jitter_k };
        if self.since < need.max(1.0) {
            return 0.0;
        }
        // Keep the fraction, so steps average out to the exact rate.
        self.since = (self.since - need).clamp(0.0, 1.0);
        self.fire(r, sr)
    }
}

// ---------------------------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------------------------

/// Equal-power pan: (left, right) gains for `pan` -1 (left) .. 1 (right), with
/// `left² + right² = 1` everywhere; the centre is -3 dB in each channel.
#[inline]
pub fn equal_power(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * (PI * 0.25);
    (a.cos().max(0.0), a.sin().max(0.0))
}

/// Balance-law pan: centre is (1, 1), so a width of zero leaves the mono signal untouched.
#[inline]
pub fn balance(pan: f32) -> (f32, f32) {
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}

/// Air between source and listener: a level drop and two one-pole low-passes (12 dB/oct)
/// whose cutoff falls with distance, since air and obstacles take the top off first.
#[derive(Clone, Copy, Debug, Default)]
pub struct Air {
    lp: [OnePole; 2],
    coef: f32,
    gain: f32,
}

impl Air {
    /// Cutoff moves from `near_hz` at distance 0 to `far_hz` at 1, evenly in octaves.
    #[inline]
    pub fn set(&mut self, distance: f32, near_hz: f32, far_hz: f32, gain: f32, sr: f32) {
        self.coef = hz_coef(near_hz * (far_hz / near_hz).powf(distance), sr);
        self.gain = gain;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.lp[0].lp(x, self.coef);
        self.lp[1].lp(y, self.coef) * self.gain
    }

    pub fn reset(&mut self) {
        self.lp = [OnePole::default(); 2];
    }
}
