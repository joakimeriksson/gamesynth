//! Master effects applied after voice summing: drive, bit crusher, delay.

use crate::math::soft_clip;
use crate::patch::FxParams;

/// Longest supported delay time in seconds (sizes the delay line at construction).
pub const MAX_DELAY_SECS: f32 = 1.0;

pub struct Effects {
    delay: Vec<f32>,
    delay_pos: usize,
    hold: f32,
    hold_count: f32,
    sample_rate: f32,
}

impl Effects {
    pub fn new(sample_rate: f32) -> Self {
        let len = ((MAX_DELAY_SECS * sample_rate) as usize).max(2);
        Effects { delay: vec![0.0; len], delay_pos: 0, hold: 0.0, hold_count: 0.0, sample_rate }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn reset(&mut self) {
        self.delay.iter_mut().for_each(|s| *s = 0.0);
        self.delay_pos = 0;
        self.hold = 0.0;
        self.hold_count = 0.0;
    }

    /// Process a mono block in place. Allocation free.
    pub fn process(&mut self, p: &FxParams, buf: &mut [f32]) {
        if p.drive > 0.0 {
            let pre = 1.0 + p.drive * 15.0;
            let post = 1.0 - 0.5 * p.drive;
            for s in buf.iter_mut() {
                *s = soft_clip(*s * pre) * post;
            }
        }

        let crush_bits = p.bit_depth >= 1.0;
        let crush_rate = p.downsample > 1.0;
        if crush_bits || crush_rate {
            let levels = (p.bit_depth.clamp(1.0, 16.0) - 1.0).exp2();
            for s in buf.iter_mut() {
                let mut x = *s;
                if crush_rate {
                    self.hold_count += 1.0;
                    if self.hold_count >= p.downsample {
                        self.hold_count -= p.downsample;
                        self.hold = x;
                    }
                    x = self.hold;
                }
                if crush_bits {
                    x = (x * levels).round() / levels;
                }
                *s = x;
            }
        }

        if p.delay_time > 0.0 && p.delay_mix > 0.0 {
            let len = self.delay.len();
            let d = ((p.delay_time * self.sample_rate) as usize).clamp(1, len - 1);
            let fb = p.delay_feedback.clamp(0.0, 0.98);
            for s in buf.iter_mut() {
                let read = (self.delay_pos + len - d) % len;
                let r = self.delay[read];
                self.delay[self.delay_pos] = *s + r * fb;
                self.delay_pos += 1;
                if self.delay_pos == len {
                    self.delay_pos = 0;
                }
                *s += r * p.delay_mix;
            }
        }
    }
}
