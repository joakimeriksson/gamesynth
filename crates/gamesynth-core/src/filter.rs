//! State variable filter (Andrew Simper's trapezoidal-integrated SVF).

use crate::math::PI;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum FilterMode {
    Off = 0,
    #[default]
    LowPass = 1,
    HighPass = 2,
    BandPass = 3,
    Notch = 4,
}

impl FilterMode {
    pub const ALL: [FilterMode; 5] = [
        FilterMode::Off,
        FilterMode::LowPass,
        FilterMode::HighPass,
        FilterMode::BandPass,
        FilterMode::Notch,
    ];
    pub const NAMES: [&'static str; 5] = ["Off", "LowPass", "HighPass", "BandPass", "Notch"];

    pub fn from_index(i: usize) -> FilterMode {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1eq: f32,
    ic2eq: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    mode: FilterMode,
}

impl Svf {
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    /// Update coefficients. Intended to be called at control rate (once per block).
    /// `resonance` is 0..1 where 1 is at the edge of self-oscillation.
    pub fn set(&mut self, mode: FilterMode, cutoff_hz: f32, resonance: f32, sample_rate: f32) {
        self.mode = mode;
        let fc = cutoff_hz.clamp(10.0, sample_rate * 0.45);
        let g = (PI * fc / sample_rate).tan();
        let k = 2.0 - 1.98 * resonance.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        self.a1 = a1;
        self.a2 = g * a1;
        self.a3 = g * self.a2;
        self.k = k;
        // Flush denormals that would otherwise linger in the integrators.
        if self.ic1eq.abs() < 1e-15 {
            self.ic1eq = 0.0;
        }
        if self.ic2eq.abs() < 1e-15 {
            self.ic2eq = 0.0;
        }
    }

    #[inline]
    pub fn tick(&mut self, v0: f32) -> f32 {
        if self.mode == FilterMode::Off {
            return v0;
        }
        let v3 = v0 - self.ic2eq;
        let v1 = self.a1 * self.ic1eq + self.a2 * v3;
        let v2 = self.ic2eq + self.a2 * self.ic1eq + self.a3 * v3;
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        match self.mode {
            FilterMode::Off => v0,
            FilterMode::LowPass => v2,
            FilterMode::BandPass => v1,
            FilterMode::HighPass => v0 - self.k * v1 - v2,
            FilterMode::Notch => v0 - self.k * v1,
        }
    }
}
