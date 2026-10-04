//! Noise sources.

use crate::math::Rng;

/// Pink (1/f) noise via Paul Kellet's refined filter method applied to white noise.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pink {
    b: [f32; 7],
}

impl Pink {
    #[inline]
    pub fn next(&mut self, white: f32) -> f32 {
        let b = &mut self.b;
        b[0] = 0.99886 * b[0] + white * 0.0555179;
        b[1] = 0.99332 * b[1] + white * 0.0750759;
        b[2] = 0.96900 * b[2] + white * 0.153852;
        b[3] = 0.86650 * b[3] + white * 0.3104856;
        b[4] = 0.55000 * b[4] + white * 0.5329522;
        b[5] = -0.7616 * b[5] - white * 0.0168980;
        let out = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + white * 0.5362;
        b[6] = white * 0.115926;
        out * 0.11
    }

    pub fn reset(&mut self) {
        self.b = [0.0; 7];
    }
}

/// White + pink noise generator with its own PRNG stream.
#[derive(Clone, Copy, Debug)]
pub struct Noise {
    rng: Rng,
    pink: Pink,
}

impl Noise {
    pub fn new(seed: u32) -> Self {
        Noise { rng: Rng::new(seed), pink: Pink::default() }
    }

    #[inline]
    pub fn white(&mut self) -> f32 {
        self.rng.next_bipolar()
    }

    #[inline]
    pub fn pink(&mut self) -> f32 {
        let w = self.rng.next_bipolar();
        self.pink.next(w)
    }
}

impl Default for Noise {
    fn default() -> Self {
        Noise::new(0xA5A5_1234)
    }
}
