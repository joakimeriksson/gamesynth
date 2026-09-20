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

/// Scramble a seed so that consecutive seeds (voice 0, 1, 2…) give unrelated streams;
/// xorshift's first outputs are close to linear in the seed.
#[inline]
pub fn mix_seed(seed: u32) -> u32 {
    let mut x = seed.wrapping_add(0x9E37_79B9);
    x = (x ^ (x >> 16)).wrapping_mul(0x85EB_CA6B);
    x = (x ^ (x >> 13)).wrapping_mul(0xC2B2_AE35);
    x ^ (x >> 16)
}

/// Poisson impulse generator ("dust"): sparse random impulses at an average rate.
#[derive(Clone, Copy, Debug)]
pub struct Dust {
    rng: Rng,
}

impl Dust {
    pub fn new(seed: u32) -> Self {
        Dust { rng: Rng::new(mix_seed(seed)) }
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
        let mut rng = Rng::new(mix_seed(seed));
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

/// Small mono reverb (four damped combs into two all-passes). Gives one-shots a space to
/// ring out in; cheap enough to run per emitter.
#[derive(Clone, Debug)]
pub struct Reverb {
    combs: [DelayLine; 4],
    damp: [f32; 4],
    allpass: [DelayLine; 2],
    /// Second all-pass pair, for the side (difference) signal of the stereo output.
    side_allpass: [DelayLine; 2],
    /// Keeps sub-bass out of the combs, whose low modes would ring as a pitched "boing".
    send_hp: OnePole,
    send_coef: f32,
    sr: f32,
}

const COMB_MS: [f32; 4] = [29.7, 37.1, 41.1, 43.7];
const ALLPASS_MS: [f32; 2] = [5.0, 1.7];
const SIDE_ALLPASS_MS: [f32; 2] = [5.9, 2.3];

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let line = |ms: f32| DelayLine::new((ms * 0.001 * sample_rate) as usize + 2);
        Reverb { combs: COMB_MS.map(line), damp: [0.0; 4], allpass: ALLPASS_MS.map(line), side_allpass: SIDE_ALLPASS_MS.map(line), send_hp: OnePole::default(), send_coef: hz_coef(220.0, sample_rate), sr: sample_rate }
    }

    pub fn clear(&mut self) {
        self.combs.iter_mut().chain(self.allpass.iter_mut()).chain(self.side_allpass.iter_mut()).for_each(|l| l.clear());
        self.damp = [0.0; 4];
        self.send_hp = OnePole::default();
    }

    /// Wet signal for input `x`. `rt60` is the decay time in seconds, `damping` 0..1 darkens
    /// the tail.
    #[inline]
    pub fn tick(&mut self, x: f32, rt60: f32, damping: f32) -> f32 {
        let (sum, _) = self.combs(x, rt60, damping);
        Self::diffuse(&mut self.allpass, &ALLPASS_MS, sum * 0.25, self.sr)
    }

    /// Stereo wet signal as `(mid, side)`: left = mid + side, right = mid - side. `mid` is
    /// exactly what [`Reverb::tick`] returns, so a width of zero is the mono reverb. The side
    /// signal sums the same four combs with alternating signs, which decorrelates the channels
    /// for the price of two short all-passes.
    #[inline]
    pub fn tick_stereo(&mut self, x: f32, rt60: f32, damping: f32) -> (f32, f32) {
        let (sum, alt) = self.combs(x, rt60, damping);
        let sr = self.sr;
        (Self::diffuse(&mut self.allpass, &ALLPASS_MS, sum * 0.25, sr), Self::diffuse(&mut self.side_allpass, &SIDE_ALLPASS_MS, alt * 0.25, sr))
    }

    /// Run the comb bank; returns (sum, alternating-sign sum) of the comb outputs.
    #[inline]
    fn combs(&mut self, x: f32, rt60: f32, damping: f32) -> (f32, f32) {
        let keep = 1.0 - damping.clamp(0.0, 0.95);
        let x = self.send_hp.hp(x, self.send_coef);
        let (mut sum, mut alt, mut sign) = (0.0, 0.0, 1.0);
        for ((comb, damp), ms) in self.combs.iter_mut().zip(self.damp.iter_mut()).zip(COMB_MS) {
            let g = 10f32.powf(-3.0 * ms * 0.001 / rt60.max(0.05));
            let y = comb.read(ms * 0.001 * self.sr);
            *damp += (y - *damp) * keep;
            comb.write(x + *damp * g);
            sum += y;
            alt += y * sign;
            sign = -sign;
        }
        (sum, alt)
    }

    #[inline]
    fn diffuse(lines: &mut [DelayLine; 2], ms: &[f32; 2], mut y: f32, sr: f32) -> f32 {
        for (line, ms) in lines.iter_mut().zip(ms) {
            let v = -0.5 * y + line.read(ms * 0.001 * sr);
            line.write(y + 0.5 * v);
            y = v;
        }
        y
    }
}
