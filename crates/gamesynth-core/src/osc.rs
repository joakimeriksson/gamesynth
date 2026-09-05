//! Band-limited oscillators (PolyBLEP) plus noise waveforms.

use crate::math::TAU;
use crate::noise::Noise;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum Waveform {
    Sine = 0,
    Triangle = 1,
    #[default]
    Saw = 2,
    Square = 3,
    /// Square with variable duty cycle (`pulse_width`).
    Pulse = 4,
    WhiteNoise = 5,
    PinkNoise = 6,
}

impl Waveform {
    pub const ALL: [Waveform; 7] = [
        Waveform::Sine,
        Waveform::Triangle,
        Waveform::Saw,
        Waveform::Square,
        Waveform::Pulse,
        Waveform::WhiteNoise,
        Waveform::PinkNoise,
    ];
    pub const NAMES: [&'static str; 7] =
        ["Sine", "Triangle", "Saw", "Square", "Pulse", "WhiteNoise", "PinkNoise"];

    pub fn from_index(i: usize) -> Waveform {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn is_noise(self) -> bool {
        matches!(self, Waveform::WhiteNoise | Waveform::PinkNoise)
    }
}

/// PolyBLEP residual for a discontinuity at phase 0 (phase `t` in [0,1), phase increment `dt`).
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// Single oscillator. Phase is kept in [0, 1).
#[derive(Clone, Copy, Debug)]
pub struct Oscillator {
    pub phase: f32,
    noise: Noise,
}

impl Oscillator {
    pub fn new(seed: u32) -> Self {
        Oscillator { phase: 0.0, noise: Noise::new(seed) }
    }

    pub fn reset(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }

    /// Produce one sample. `dt` is frequency / sample_rate; `pw` is the pulse width in (0, 1).
    #[inline]
    pub fn next(&mut self, wave: Waveform, dt: f32, pw: f32) -> f32 {
        let t = self.phase;
        let dt = dt.min(0.5);
        let out = match wave {
            Waveform::Sine => (t * TAU).sin(),
            // Naive triangle: harmonics fall off at 1/n^2 so aliasing is negligible for
            // game audio, and it keeps the waveform exact at very low frequencies.
            Waveform::Triangle => 4.0 * (t - 0.5).abs() - 1.0,
            Waveform::Saw => 2.0 * t - 1.0 - poly_blep(t, dt),
            Waveform::Square => {
                let mut v = if t < 0.5 { 1.0 } else { -1.0 };
                v += poly_blep(t, dt);
                v -= poly_blep((t + 0.5).rem_euclid(1.0), dt);
                v
            }
            Waveform::Pulse => {
                let pw = pw.clamp(0.02, 0.98);
                let mut v = if t < pw { 1.0 } else { -1.0 };
                v += poly_blep(t, dt);
                v -= poly_blep((t + 1.0 - pw).rem_euclid(1.0), dt);
                // Remove the DC offset introduced by an asymmetric duty cycle.
                v - (2.0 * pw - 1.0)
            }
            Waveform::WhiteNoise => self.noise.white(),
            Waveform::PinkNoise => self.noise.pink(),
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        out
    }
}

impl Default for Oscillator {
    fn default() -> Self {
        Oscillator::new(0xC0FF_EE01)
    }
}
