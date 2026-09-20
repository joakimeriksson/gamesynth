//! Small reusable DSP blocks shared by the generators and the graph runtime.

use crate::math::{Rng, TAU};

/// Number of samples per control block in generators and graph models.
pub const BLOCK: usize = 32;

/// One-pole coefficient for a cutoff in Hz.
#[inline]
pub fn hz_coef(hz: f32, sample_rate: f32) -> f32 {
    1.0 - (-TAU * hz.max(0.0) / sample_rate).exp()
}

/// One-pole coefficient that settles (~95%) in `secs` when applied once per `dt` seconds.
#[inline]
pub fn settle_coef(secs: f32, dt: f32) -> f32 {
    if secs <= 0.0 {
        1.0
    } else {
        1.0 - (-3.0 * dt / secs).exp()
    }
}

/// Poisson impulse generator ("dust"): sparse random impulses at an average rate.
#[derive(Clone, Copy, Debug)]
pub struct Dust {
    rng: Rng,
}

impl Dust {
    pub fn new(seed: u32) -> Self {
        Dust { rng: Rng::new(seed) }
    }

    /// `p` is the per-sample probability (rate / sample_rate). Returns 0 or an amplitude 0.3..1.
    #[inline]
    pub fn tick(&mut self, p: f32) -> f32 {
        if self.rng.next_f32() < p {
            self.rng.range(0.3, 1.0)
        } else {
            0.0
        }
    }

    #[inline]
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }
}

/// Smooth random control signal in [-1, 1]: random targets joined by cosine segments.
#[derive(Clone, Copy, Debug)]
pub struct SlowNoise {
    rng: Rng,
    phase: f32,
    a: f32,
    b: f32,
}

impl SlowNoise {
    pub fn new(seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        let (a, b) = (rng.next_bipolar(), rng.next_bipolar());
        SlowNoise { rng, phase: 0.0, a, b }
    }

    /// Advance by `dt` seconds at `rate_hz` new targets per second.
    #[inline]
    pub fn advance(&mut self, rate_hz: f32, dt: f32) -> f32 {
        self.phase += rate_hz.max(0.0) * dt;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            self.a = self.b;
            self.b = self.rng.next_bipolar();
        }
        let t = 0.5 - 0.5 * (self.phase * core::f32::consts::PI).cos();
        self.a + (self.b - self.a) * t
    }
}

/// Brown (red) noise: leaky integrated white noise, roughly unit level.
#[derive(Clone, Copy, Debug, Default)]
pub struct Brown {
    y: f32,
}

impl Brown {
    #[inline]
    pub fn tick(&mut self, white: f32) -> f32 {
        self.y = (self.y + 0.02 * white) / 1.02;
        self.y * 3.5
    }
}

/// One-pole low-pass state; high-pass is `x - lp`.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePole {
    y: f32,
}

impl OnePole {
    #[inline]
    pub fn lp(&mut self, x: f32, coef: f32) -> f32 {
        self.y += (x - self.y) * coef;
        if self.y.abs() < 1e-20 {
            self.y = 0.0;
        }
        self.y
    }

    #[inline]
    pub fn hp(&mut self, x: f32, coef: f32) -> f32 {
        x - self.lp(x, coef)
    }
}

/// Fractional delay line with linear interpolation. Allocates once at construction.
#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        DelayLine { buf: vec![0.0; max_samples.max(4) + 4], pos: 0 }
    }

    pub fn max_delay(&self) -> f32 {
        (self.buf.len() - 3) as f32
    }

    /// Read `delay` samples back (clamped to the line length).
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        let d = delay.clamp(1.0, (len - 3) as f32);
        let di = d as usize;
        let frac = d - di as f32;
        let i0 = (self.pos + len - di) % len;
        let i1 = (i0 + len - 1) % len;
        self.buf[i0] + (self.buf[i1] - self.buf[i0]) * frac
    }

    #[inline]
    pub fn write(&mut self, x: f32) {
        self.buf[self.pos] = if x.is_finite() { x } else { 0.0 };
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
    }

    pub fn clear(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
    }
}

/// Phase accumulator in [0, 1). Returns true on wrap.
#[derive(Clone, Copy, Debug, Default)]
pub struct Phasor {
    pub phase: f32,
}

impl Phasor {
    #[inline]
    pub fn tick(&mut self, inc: f32) -> bool {
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn sin(&self) -> f32 {
        (self.phase * TAU).sin()
    }
}
