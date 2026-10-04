//! Low frequency oscillator, evaluated at control rate.

use crate::math::{Rng, TAU};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum LfoWave {
    #[default]
    Sine = 0,
    Triangle = 1,
    Saw = 2,
    Square = 3,
    SampleHold = 4,
}

impl LfoWave {
    pub const ALL: [LfoWave; 5] =
        [LfoWave::Sine, LfoWave::Triangle, LfoWave::Saw, LfoWave::Square, LfoWave::SampleHold];
    pub const NAMES: [&'static str; 5] = ["Sine", "Triangle", "Saw", "Square", "SampleHold"];

    pub fn from_index(i: usize) -> LfoWave {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LfoParams {
    pub wave: LfoWave,
    /// Rate in Hz.
    pub rate_hz: f32,
    /// Pitch modulation depth in semitones.
    pub pitch: f32,
    /// Filter cutoff modulation depth in octaves.
    pub cutoff: f32,
    /// Tremolo depth 0..1.
    pub amp: f32,
    /// Pulse width modulation depth 0..0.5.
    pub pulse_width: f32,
    /// Seconds for the LFO depth to fade in after note on.
    pub fade_in: f32,
}

impl Default for LfoParams {
    fn default() -> Self {
        LfoParams {
            wave: LfoWave::Sine,
            rate_hz: 5.0,
            pitch: 0.0,
            cutoff: 0.0,
            amp: 0.0,
            pulse_width: 0.0,
            fade_in: 0.0,
        }
    }
}

impl LfoParams {
    pub fn is_active(&self) -> bool {
        self.pitch != 0.0 || self.cutoff != 0.0 || self.amp != 0.0 || self.pulse_width != 0.0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    phase: f32,
    held: f32,
    rng: Rng,
    value: f32,
}

impl Default for Lfo {
    fn default() -> Self {
        Lfo { phase: 0.0, held: 0.0, rng: Rng::new(0x10F0_0001), value: 0.0 }
    }
}

impl Lfo {
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.held = self.rng.next_bipolar();
        self.value = 0.0;
    }

    /// Advance by `dt_secs` and return the new bipolar value in [-1, 1].
    #[inline]
    pub fn advance(&mut self, wave: LfoWave, rate_hz: f32, dt_secs: f32) -> f32 {
        self.phase += rate_hz * dt_secs;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            if wave == LfoWave::SampleHold {
                self.held = self.rng.next_bipolar();
            }
        }
        let t = self.phase;
        self.value = match wave {
            LfoWave::Sine => (t * TAU).sin(),
            LfoWave::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
            LfoWave::Saw => 2.0 * t - 1.0,
            LfoWave::Square => {
                if t < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoWave::SampleHold => self.held,
        };
        self.value
    }

    #[inline]
    pub fn value(&self) -> f32 {
        self.value
    }
}
