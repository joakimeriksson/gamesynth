//! ADSR envelope generator.

use crate::math::onepole_coef;

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AdsrParams {
    /// Seconds from trigger to peak (linear ramp).
    pub attack: f32,
    /// Seconds from peak to sustain level (exponential).
    pub decay: f32,
    /// Sustain level 0..1.
    pub sustain: f32,
    /// Seconds from release to silence (exponential).
    pub release: f32,
}

impl Default for AdsrParams {
    fn default() -> Self {
        AdsrParams { attack: 0.005, decay: 0.1, sustain: 0.8, release: 0.2 }
    }
}

impl AdsrParams {
    pub const fn new(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        AdsrParams { attack, decay, sustain, release }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Linear attack, exponential decay/release. Envelope value is always in [0, 1].
#[derive(Clone, Copy, Debug, Default)]
pub struct Adsr {
    stage: Stage,
    level: f32,
    attack_inc: f32,
    decay_coef: f32,
    release_coef: f32,
    sustain: f32,
}

const SILENCE: f32 = 1e-4;

impl Adsr {
    pub fn trigger(&mut self, p: &AdsrParams, sample_rate: f32) {
        self.attack_inc = if p.attack <= 0.0 { 1.0 } else { 1.0 / (p.attack * sample_rate) };
        self.decay_coef = onepole_coef(p.decay, sample_rate);
        self.release_coef = onepole_coef(p.release, sample_rate);
        self.sustain = p.sustain.clamp(0.0, 1.0);
        // Retriggering from a non-zero level starts the attack from there (no click).
        self.stage = Stage::Attack;
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// Immediately silence the envelope.
    pub fn kill(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
    }

    #[inline]
    pub fn stage(&self) -> Stage {
        self.stage
    }

    #[inline]
    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    #[inline]
    pub fn is_released(&self) -> bool {
        matches!(self.stage, Stage::Release | Stage::Idle)
    }

    #[inline]
    pub fn level(&self) -> f32 {
        self.level
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += self.attack_inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level += (self.sustain - self.level) * self.decay_coef;
                if self.level - self.sustain < SILENCE {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                    if self.sustain <= 0.0 {
                        self.stage = Stage::Idle;
                    }
                }
            }
            Stage::Sustain => {}
            Stage::Release => {
                self.level -= self.level * self.release_coef;
                if self.level < SILENCE {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}
