//! Small numeric helpers shared by the DSP modules.

pub use core::f32::consts::{PI, TAU};

/// Convert a (fractional) MIDI note number to frequency in Hz (A4 = 69 = 440 Hz).
#[inline]
pub fn midi_to_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// Frequency ratio for a pitch offset in semitones.
#[inline]
pub fn semitones_to_ratio(semitones: f32) -> f32 {
    (semitones / 12.0).exp2()
}

/// Decibels to linear gain.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Cheap tanh-like saturator (Padé approximant). Exact-ish on [-3, 3], hard clipped outside.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// One-pole smoothing coefficient that reaches ~99.9% of its target in `time_secs`.
#[inline]
pub fn onepole_coef(time_secs: f32, sample_rate: f32) -> f32 {
    if time_secs <= 0.0 {
        1.0
    } else {
        1.0 - (-6.9078 / (time_secs * sample_rate)).exp()
    }
}

/// Tiny xorshift32 PRNG. Deterministic, allocation free, good enough for audio noise and
/// preset randomisation. Not for anything security related.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        // xorshift must never be seeded with zero.
        Rng(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform in [-1, 1).
    #[inline]
    pub fn next_bipolar(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }

    /// Uniform in [lo, hi).
    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    /// True with probability `p`.
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }
}

impl Default for Rng {
    fn default() -> Self {
        Rng::new(0x1234_5678)
    }
}

/// Master output limiter: identity below `LIMIT_KNEE`, then soft saturation that never
/// exceeds ±1.0. Keeps hot patches from hard-clipping in the host engine.
pub const LIMIT_KNEE: f32 = 0.7;

#[inline]
pub fn limit(y: f32) -> f32 {
    let a = y.abs();
    if a <= LIMIT_KNEE {
        y
    } else {
        let over = (a - LIMIT_KNEE) / (1.0 - LIMIT_KNEE);
        (LIMIT_KNEE + (1.0 - LIMIT_KNEE) * soft_clip(over)).copysign(y)
    }
}
