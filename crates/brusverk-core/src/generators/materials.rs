//! Material sounds for physics, puzzle, sandbox and sports games: a struck object (`strike`)
//! and an object rolling or sliding (`roll`).
//!
//! Both are modal synthesis. A material is a small table of modes (frequency ratio, level,
//! ring time) measured on recordings of real hits (`tools/impact_measure.py`, recordings in
//! `target/refs/impacts`). A hit is a short force pulse into those modes: its length is the
//! contact time, which a soft floor stretches and a fast hit shortens (Hertz contact), so a
//! soft floor or a gentle touch only reaches the low modes and a hard, fast hit rings the high
//! ones too. Where on the object it is struck changes the mode levels, so no two hits are the
//! same. Size scales every mode down in pitch and up in ring time. Rolling is the same modes
//! fed by many tiny contacts: the bumps of the surface, its rumble, the ball's own wobble, and
//! friction noise when it slides.
//!
//! A strike that is triggered again while it still rings adds to the ringing modes, as a real
//! object does: one playback per object, `trigger()` on every contact.

use crate::blocks::{hz_coef, mix_seed, OnePole, SlowNoise, BLOCK};
use crate::math::{Rng, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::params::{exp, int, lin, GAIN, UNIT};

pub const MAX_MODES: usize = 12;

/// One mode of a material at size 0.5: frequency as a ratio of [`Material::hz`], its level
/// in dB for a unit strike, and its ring time (60 dB) in seconds.
#[derive(Clone, Copy, Debug)]
pub struct Mode {
    pub ratio: f32,
    pub db: f32,
    pub t60: f32,
}

const fn m(ratio: f32, db: f32, t60: f32) -> Mode {
    Mode { ratio, db, t60 }
}

#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub name: &'static str,
    /// Frequency of ratio 1 at size 0.5 and no transposition.
    pub hz: f32,
    pub modes: &'static [Mode],
    /// Contact time against a hard floor at full power, in milliseconds.
    pub contact_ms: f32,
    /// Contact noise (the click or crunch of the hit itself): level relative to the modes,
    /// decay time in ms and top frequency in Hz.
    pub noise: f32,
    pub noise_ms: f32,
    pub noise_hz: f32,
    /// Extra micro-contacts after the first, spread over `grit_ms`: an irregular, rough body.
    pub grit: u32,
    pub grit_ms: f32,
    /// Body noise: the many small high modes that make a real hit's spectrum dense, heard as a
    /// short hiss between half of `hz` and `noise_hz`. Level relative to the modes, ring time in ms.
    pub hiss: f32,
    pub hiss_ms: f32,
    /// Output level, so that every material peaks about the same at full power.
    pub level: f32,
}

/// The materials, in `material/kind` order. Mode tables are from recordings (see the module
/// docs): a plank dropped on the ground, a hollow metal tube, a drinking glass, cobblestones,
/// a plastic storage box, a clay pot lid and plate, a plastic ball.
pub const MATERIALS: [Material; 7] = [
    Material {
        name: "wood",
        hz: 1000.0,
        modes: &[
            m(0.15, -26.0, 0.400),
            m(0.30, -18.0, 0.250),
            m(0.37, -18.0, 0.180),
            m(0.65, -12.0, 0.120),
            m(0.77, -8.0, 0.060),
            m(1.00, 0.0, 0.110),
            m(1.30, -10.0, 0.070),
            m(1.59, -13.0, 0.060),
            m(1.78, -15.0, 0.070),
            m(2.60, -20.0, 0.040),
        ],
        contact_ms: 0.25,
        noise: 2.0,
        noise_ms: 8.0,
        noise_hz: 3000.0,
        grit: 0,
        grit_ms: 0.0,
        hiss: 2.5,
        hiss_ms: 120.0,
        level: 0.394,
    },
    Material {
        name: "metal",
        hz: 500.0,
        modes: &[
            m(0.52, -12.0, 5.000),
            m(1.00, -4.0, 4.500),
            m(1.62, 0.0, 4.500),
            m(2.37, -10.0, 2.800),
            m(3.26, -10.0, 3.500),
            m(4.28, -2.0, 3.000),
            m(5.40, -6.0, 2.000),
            m(6.60, -6.0, 1.500),
            m(9.06, 3.0, 0.800),
            m(11.80, 2.0, 0.500),
            m(15.30, 0.0, 0.350),
            m(19.70, -4.0, 0.250),
        ],
        contact_ms: 0.08,
        noise: 0.4,
        noise_ms: 8.0,
        noise_hz: 14000.0,
        grit: 0,
        grit_ms: 0.0,
        hiss: 0.1,
        hiss_ms: 40.0,
        level: 0.177,
    },
    Material {
        name: "glass",
        hz: 1350.0,
        modes: &[
            m(1.00, -6.0, 0.700),
            m(2.31, 0.0, 0.300),
            m(3.37, 0.0, 0.150),
            m(3.61, 0.0, 0.120),
            m(3.73, 9.0, 0.100),
            m(6.96, -3.0, 0.150),
        ],
        contact_ms: 0.06,
        noise: 0.1,
        noise_ms: 0.6,
        noise_hz: 14000.0,
        grit: 0,
        grit_ms: 0.0,
        hiss: 0.0,
        hiss_ms: 20.0,
        level: 0.155,
    },
    Material {
        name: "stone",
        hz: 600.0,
        modes: &[
            m(0.13, -18.0, 0.250),
            m(0.40, -14.0, 0.150),
            m(0.44, -14.0, 0.120),
            m(1.00, -6.0, 0.180),
            m(1.42, -8.0, 0.120),
            m(1.60, -6.0, 0.050),
            m(1.76, -8.0, 0.120),
            m(2.04, -8.0, 0.100),
            m(2.90, -10.0, 0.060),
            m(4.10, -12.0, 0.040),
            m(5.60, -14.0, 0.030),
        ],
        contact_ms: 0.12,
        noise: 3.0,
        noise_ms: 30.0,
        noise_hz: 1200.0,
        grit: 4,
        grit_ms: 12.0,
        hiss: 0.6,
        hiss_ms: 250.0,
        level: 0.418,
    },
    Material {
        name: "plastic",
        hz: 240.0,
        modes: &[
            m(0.75, -2.0, 0.180),
            m(0.84, -7.0, 0.140),
            m(1.00, 0.0, 0.140),
            m(1.13, -4.0, 0.140),
            m(1.29, 0.0, 0.160),
            m(1.70, -2.0, 0.120),
            m(1.87, -2.0, 0.120),
            m(2.22, 0.0, 0.100),
            m(3.16, -1.0, 0.070),
            m(3.85, -2.0, 0.070),
            m(5.50, -4.0, 0.050),
        ],
        contact_ms: 0.25,
        noise: 2.0,
        noise_ms: 10.0,
        noise_hz: 5000.0,
        grit: 0,
        grit_ms: 0.0,
        hiss: 3.0,
        hiss_ms: 120.0,
        level: 0.140,
    },
    Material {
        name: "ceramic",
        hz: 1230.0,
        modes: &[
            m(0.35, -10.0, 0.250),
            m(1.00, 0.0, 0.320),
            m(1.24, -6.0, 0.200),
            m(1.90, -4.0, 0.220),
            m(2.18, -3.0, 0.220),
            m(2.79, -6.0, 0.200),
            m(3.05, -3.0, 0.200),
            m(4.27, -5.0, 0.180),
            m(6.40, -9.0, 0.120),
        ],
        contact_ms: 0.1,
        noise: 0.35,
        noise_ms: 6.0,
        noise_hz: 10000.0,
        grit: 0,
        grit_ms: 0.0,
        hiss: 3.0,
        hiss_ms: 150.0,
        level: 0.178,
    },
    Material {
        name: "rubber",
        hz: 240.0,
        modes: &[
            m(0.49, -8.0, 0.300),
            m(0.78, -2.0, 0.270),
            m(1.00, 0.0, 0.220),
            m(1.42, -4.0, 0.100),
            m(2.40, 0.0, 0.120),
            m(4.30, -3.0, 0.070),
            m(6.20, -6.0, 0.050),
            m(9.00, -9.0, 0.030),
        ],
        contact_ms: 0.25,
        noise: 2.5,
        noise_ms: 8.0,
        noise_hz: 5000.0,
        grit: 0,
        grit_ms: 0.0,
        hiss: 6.0,
        hiss_ms: 150.0,
        level: 0.151,
    },
];

pub fn material(kind: f32) -> &'static Material {
    &MATERIALS[(kind.round().max(0.0) as usize).min(MATERIALS.len() - 1)]
}

/// The object's scale for a 0..1 `size` input: 0.25 (two octaves up) .. 4 (two octaves down).
#[inline]
pub fn size_scale(size: f32) -> f32 {
    ((size.clamp(0.0, 1.0) - 0.5) * 4.0).exp2()
}

/// A bank of decaying sinusoids, each a complex one-pole resonator (four multiplies a sample).
/// Modes that have died away are skipped.
#[derive(Clone, Copy, Debug)]
struct ModeBank {
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

impl ModeBank {
    const fn new() -> Self {
        ModeBank {
            re: [0.0; MAX_MODES],
            im: [0.0; MAX_MODES],
            c: [0.0; MAX_MODES],
            s: [0.0; MAX_MODES],
            g: [0.0; MAX_MODES],
            live: [false; MAX_MODES],
            n: 0,
        }
    }

    /// Tune mode `k` to `hz` with ring time `t60`. Modes above 0.45 sr are silenced.
    fn tune(&mut self, k: usize, hz: f32, t60: f32, sr: f32) {
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

    fn clear(&mut self) {
        self.re = [0.0; MAX_MODES];
        self.im = [0.0; MAX_MODES];
        self.live = [false; MAX_MODES];
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
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
    fn energy(&mut self, floor: f32) -> f32 {
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

/// A half-sine force pulse, normalised to unit area so a soft (long) contact delivers the same
/// momentum as a hard one, only with less treble.
#[derive(Clone, Copy, Debug, Default)]
struct Pulse {
    /// Samples until it starts.
    wait: i32,
    t: f32,
    len: f32,
    amp: f32,
}

impl Pulse {
    #[inline]
    fn tick(&mut self) -> f32 {
        if self.amp == 0.0 {
            return 0.0;
        }
        if self.wait > 0 {
            self.wait -= 1;
            return 0.0;
        }
        let t = self.t;
        self.t += 1.0;
        if t >= self.len {
            self.amp = 0.0;
            return 0.0;
        }
        // The sample sum of sin(pi (t + 0.5) / len) is about 2 len / pi.
        self.amp * (core::f32::consts::PI * (t + 0.5) / self.len).sin() * (core::f32::consts::FRAC_PI_2 / self.len)
    }
}

const MAX_PULSES: usize = 8;

/// Distance: quieter and duller, as for the other events (air takes the top off first).
#[derive(Clone, Copy, Debug, Default)]
struct Far {
    lp: [OnePole; 2],
    coef: f32,
    gain: f32,
}

impl Far {
    fn set(&mut self, distance: f32, sr: f32) {
        self.coef = hz_coef(18000.0 * (400.0f32 / 18000.0).powf(distance), sr);
        self.gain = 1.0 - 0.75 * distance;
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.lp[0].lp(x, self.coef);
        self.lp[1].lp(y, self.coef) * self.gain
    }
}

// ---------------------------------------------------------------------------------------------
// strike
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A struck object. `material/kind`: 0 wood, 1 metal, 2 glass, 3 stone, 4 plastic,
    /// 5 ceramic, 6 rubber. The presets pick the material.
    StrikeParams / StrikeParamId {
        kind: "material/kind" = 0.0, int(0, 6);
        pitch: "body/pitch_semitones" = 0.0, lin(-24.0, 24.0);
        ring: "body/ring" = 1.0, exp(0.1, 4.0);
        brightness: "body/brightness" = 0.5, UNIT;
        click: "body/click" = 1.0, lin(0.0, 3.0);
        variation: "shape/variation" = 0.5, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

fn kind_preset(kind: f32) -> StrikeParams {
    StrikeParams { kind, ..Default::default() }
}

pub struct Strike {
    sr: f32,
    bank: ModeBank,
    /// Untransposed mode frequencies and ring times of the current hit.
    hz: [f32; MAX_MODES],
    t60: [f32; MAX_MODES],
    pulses: [Pulse; MAX_PULSES],
    noise: Noise,
    noise_env: f32,
    noise_decay: f32,
    noise_lp: [OnePole; 2],
    noise_coef: f32,
    hiss_env: f32,
    hiss_decay: f32,
    hiss_lp: [OnePole; 2],
    hiss_hp: OnePole,
    hiss_coef: f32,
    hiss_hp_coef: f32,
    rng: Rng,
    far: Far,
    active: bool,
    pitch_ratio: f32,
}

impl Strike {
    fn retune(&mut self) {
        for k in 0..self.bank.n {
            let g = self.bank.g[k];
            self.bank.tune(k, self.hz[k] * self.pitch_ratio, self.t60[k], self.sr);
            if self.bank.c[k] != 0.0 || self.bank.s[k] != 0.0 {
                self.bank.g[k] = g;
            }
        }
    }

    fn add_pulse(&mut self, wait: f32, len: f32, amp: f32) {
        let sr = self.sr;
        // A free slot, else replace the weakest.
        let slot = (0..MAX_PULSES)
            .min_by(|&a, &b| self.pulses[a].amp.abs().partial_cmp(&self.pulses[b].amp.abs()).unwrap_or(core::cmp::Ordering::Equal))
            .unwrap_or(0);
        self.pulses[slot] = Pulse { wait: (wait * sr) as i32, t: 0.0, len: (len * sr).max(1.0), amp };
    }

    /// Contact time in seconds for these inputs: material, floor hardness, impact speed.
    pub fn contact_secs(mat: &Material, power: f32, hardness: f32, brightness: f32) -> f32 {
        // Hertz contact shrinks as v^-1/5. The recordings brighten by 0.5..1 octave per 20 dB of
        // hit strength (a gentle hit is also a softer one), so it is steeper here.
        mat.contact_ms * 1e-3 * ((1.0 - hardness) * 5.0).exp2() * power.max(0.02).powf(-0.5) * ((0.5 - brightness) * 3.0).exp2()
    }
}

impl Generator for Strike {
    type P = StrikeParams;
    const NAME: &'static str = "strike";
    const CATEGORY: &'static str = "materials";
    const DOC: &'static str = "A struck object of wood, metal, glass, stone, plastic, ceramic or rubber (modal synthesis): \
        harder hits are louder and brighter, bigger objects lower and longer, a soft floor dulls the attack. Retrigger on every contact.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "power", default: 1.0, doc: "Impact speed: a touch .. a hard throw (level and brightness)" },
        InputSpec { name: "distance", default: 0.0, doc: "0 = on top of the listener, 1 = far off (duller, quieter)" },
        InputSpec { name: "size", default: 0.5, doc: "Object size: 0 tiny (two octaves up, short) .. 1 huge (two octaves down, long)" },
        InputSpec { name: "hardness", default: 0.8, doc: "What it hits: 0 soft (carpet, grass, a hand) .. 1 hard (stone floor)" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const ONE_SHOT: bool = true;
    const TRANSPOSES: bool = true;

    fn presets() -> Vec<(&'static str, StrikeParams)> {
        vec![
            ("Wood", kind_preset(0.0)),
            ("Metal", kind_preset(1.0)),
            ("Glass", kind_preset(2.0)),
            ("Stone", kind_preset(3.0)),
            ("Plastic", kind_preset(4.0)),
            ("Ceramic", kind_preset(5.0)),
            ("Rubber ball", kind_preset(6.0)),
        ]
    }

    fn new(sr: f32) -> Self {
        Strike {
            sr,
            bank: ModeBank::new(),
            hz: [0.0; MAX_MODES],
            t60: [0.0; MAX_MODES],
            pulses: [Pulse::default(); MAX_PULSES],
            noise: Noise::new(mix_seed(0x57_1E)),
            noise_env: 0.0,
            noise_decay: 0.0,
            noise_lp: [OnePole::default(); 2],
            noise_coef: 1.0,
            hiss_env: 0.0,
            hiss_decay: 0.0,
            hiss_lp: [OnePole::default(); 2],
            hiss_hp: OnePole::default(),
            hiss_coef: 1.0,
            hiss_hp_coef: 0.0,
            rng: Rng::new(mix_seed(0x57_1F)),
            far: Far::default(),
            active: false,
            pitch_ratio: 1.0,
        }
    }

    fn trigger(&mut self, x: &[f32], p: &StrikeParams) {
        let (power, distance, size, hardness) = (x[0], x[1], x[2], x[3]);
        let mat = material(p.kind);
        let v = p.variation.clamp(0.0, 1.0);
        if v <= 0.0 {
            self.rng = Rng::new(mix_seed(0x57_1F));
            self.noise = Noise::new(mix_seed(0x57_1E));
        }
        let s = size_scale(size);
        let tune = (p.pitch / 12.0).exp2() / s;
        // A soft floor also holds the object and damps its ring a little.
        let ring = p.ring * s.powf(0.6) * (0.55 + 0.45 * hardness);
        // Where it is struck: each mode's level follows its shape at that point.
        let pos = 0.23 + v * (self.rng.range(0.06, 0.5) - 0.23);
        if !self.active {
            self.bank.clear();
        }
        self.bank.n = mat.modes.len();
        for (k, mode) in mat.modes.iter().enumerate() {
            let shape = 0.25 + 0.75 * (core::f32::consts::PI * (k as f32 + 1.0) * pos).sin().abs();
            self.hz[k] = mat.hz * mode.ratio * tune * (1.0 + v * 0.006 * self.rng.next_bipolar());
            self.t60[k] = (mode.t60 * ring * (1.0 + v * 0.3 * self.rng.next_bipolar())).clamp(0.004, 12.0);
            self.bank.g[k] = (mode.db / 20.0 * core::f32::consts::LN_10).exp() * shape;
            self.bank.live[k] = true;
        }
        self.retune();
        let level = mat.level * power * s.powf(0.3) * (1.0 + v * 0.15 * self.rng.next_bipolar());
        let contact = Self::contact_secs(mat, power, hardness, p.brightness);
        self.add_pulse(0.0, contact, level);
        for _ in 0..mat.grit {
            let wait = self.rng.range(0.0005, mat.grit_ms * 1e-3) * s.sqrt();
            let a = self.rng.next_f32();
            let len = contact * self.rng.range(0.5, 1.5);
            self.add_pulse(wait, len, level * a * a * 0.6);
        }
        // The click of the contact itself: shorter and duller on a soft floor.
        self.noise_env += mat.noise * p.click * level * power.sqrt() * (0.2 + 0.8 * hardness) * 0.5;
        self.noise_decay = (-6.91 / (mat.noise_ms * 1e-3 * (0.5 + 0.5 * s) * self.sr)).exp();
        // A harder hit shakes loose higher frequencies too (the recordings brighten with strength).
        let top = mat.noise_hz * (0.4 + 0.6 * power);
        self.noise_coef = hz_coef(top.min(0.6 / contact) * (p.pitch / 12.0).exp2(), self.sr);
        // The body hiss follows the modes: lower and longer for a bigger object, duller when soft.
        self.hiss_env += mat.hiss * level * (0.3 + 0.7 * hardness);
        self.hiss_decay = (-6.91 / (mat.hiss_ms * 1e-3 * ring.min(4.0) * self.sr)).exp();
        self.hiss_coef = hz_coef((top * tune).min(1.2 / contact).min(0.45 * self.sr), self.sr);
        self.hiss_hp_coef = hz_coef(0.5 * mat.hz * tune, self.sr);
        self.far.set(distance, self.sr);
        self.active = true;
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn length(p: &StrikeParams) -> Option<f32> {
        // The longest mode at the biggest size, at its slowest variation roll, until it is
        // 80 dB down, plus the peak meter's settling.
        let mat = material(p.kind);
        let longest = mat.modes.iter().map(|m| m.t60).fold(0.0, f32::max);
        let t60 = (longest * p.ring * size_scale(1.0).powf(0.6) * (1.0 + p.variation * 0.3)).min(12.0);
        Some(t60 * 1.45 + 0.35)
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        if ratio != self.pitch_ratio {
            self.pitch_ratio = ratio;
            self.retune();
        }
    }

    fn block(&mut self, _x: &[f32], p: &StrikeParams, out: &mut [f32]) {
        let gain = p.gain;
        for o in out.iter_mut() {
            let mut x = 0.0;
            for pulse in self.pulses.iter_mut() {
                x += pulse.tick();
            }
            let mut y = self.bank.tick(x);
            if self.noise_env > 1e-6 {
                let n = self.noise.white() * self.noise_env;
                self.noise_env *= self.noise_decay;
                let n = self.noise_lp[0].lp(n, self.noise_coef);
                y += self.noise_lp[1].lp(n, self.noise_coef);
            }
            if self.hiss_env > 1e-6 {
                let h = self.noise.white() * self.hiss_env;
                self.hiss_env *= self.hiss_decay;
                let h = self.hiss_lp[0].lp(h, self.hiss_coef);
                y += self.hiss_hp.hp(self.hiss_lp[1].lp(h, self.hiss_coef), self.hiss_hp_coef);
            }
            *o = self.far.tick(y) * gain;
        }
        let pending = self.pulses.iter().any(|q| q.amp != 0.0);
        let energy = self.bank.energy(1e-6);
        if !pending && energy < 3e-5 && self.noise_env < 1e-6 && self.hiss_env < 1e-6 {
            self.active = false;
            self.bank.clear();
            self.noise_env = 0.0;
            self.hiss_env = 0.0;
            self.hiss_lp = [OnePole::default(); 2];
            self.hiss_hp = OnePole::default();
            self.noise_lp = [OnePole::default(); 2];
            self.far.lp = [OnePole::default(); 2];
        }
        debug_assert!(out.len() <= BLOCK);
    }
}

// ---------------------------------------------------------------------------------------------
// roll
// ---------------------------------------------------------------------------------------------

model_params! {
    /// An object rolling or sliding over a surface. Same materials as `strike`.
    ///
    /// Measured on recordings of balls rolling (`target/refs/impacts/roll`): the sound is a band
    /// of contact noise whose centre rises with the hardness of ball and floor (a wooden ball on
    /// a wooden floor peaks at 250..500 Hz, a glass marble at 1..2 kHz) and with speed, plus
    /// clacks over the grain of the floor, 10 to 30 dB above the band, plus the ball's own ring.
    RollParams / RollParamId {
        kind: "material/kind" = 0.0, int(0, 6);
        pitch: "body/pitch_semitones" = 0.0, lin(-24.0, 24.0);
        ring: "body/ring" = 1.0, exp(0.1, 4.0);
        modes: "body/modes" = 0.5, UNIT;
        bumps: "roll/bumps" = 0.5, UNIT;
        rumble: "roll/rumble" = 0.5, UNIT;
        rumble_hz: "roll/rumble_hz" = 400.0, exp(80.0, 4000.0);
        wobble: "roll/wobble" = 0.3, UNIT;
        top_speed: "roll/top_speed_ms" = 4.0, exp(0.5, 30.0);
        floor: "floor/hardness" = 0.7, UNIT;
        friction: "slide/friction" = 0.5, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

fn roll_preset(kind: f32, f: impl FnOnce(&mut RollParams)) -> RollParams {
    let mut p = RollParams { kind, ..Default::default() };
    f(&mut p);
    p
}

pub struct Roll {
    sr: f32,
    bank: ModeBank,
    noise: Noise,
    rng: Rng,
    /// Shape of a bump (the contact pulse), the contact band, sliding friction.
    bump: [OnePole; 2],
    click_hp: OnePole,
    band: [OnePole; 2],
    band_hp: OnePole,
    slide_hp: OnePole,
    slide_lp: OnePole,
    chatter: OnePole,
    phase: f32,
    turn: f32,
    /// The floor is not even: the contact swells and fades a few times a second.
    uneven: SlowNoise,
    /// Material, size, ring and pitch the modes are tuned for.
    tuned: (f32, f32, f32, f32),
}

impl Generator for Roll {
    type P = RollParams;
    const NAME: &'static str = "roll";
    const CATEGORY: &'static str = "materials";
    const DOC: &'static str = "An object rolling or sliding: a ball, a barrel, a marble, a crate dragged along. \
        Contact noise and clacks over the floor's grain, ringing the modes of strike's materials; wobble, and friction when it slides.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.0, doc: "Rolling speed, 0 still .. 1 = roll/top_speed_ms" },
        InputSpec { name: "surface", default: 0.4, doc: "0 smooth (polished floor) .. 1 rough (gravel, cobbles)" },
        InputSpec { name: "size", default: 0.5, doc: "Object size, as for strike: 0 a marble .. 1 a barrel" },
        InputSpec { name: "slide", default: 0.0, doc: "0 rolling .. 1 sliding (skidding, dragged): friction noise" },
    ];

    fn presets() -> Vec<(&'static str, RollParams)> {
        vec![
            ("Wooden ball", roll_preset(0.0, |p| {
                p.bumps = 0.7;
                p.modes = 0.25;
            })),
            ("Steel ball", roll_preset(1.0, |p| {
                p.ring = 0.5;
                p.modes = 0.7;
                p.rumble_hz = 700.0;
            })),
            ("Glass marble", roll_preset(2.0, |p| {
                p.top_speed = 2.0;
                p.rumble_hz = 350.0;
                p.bumps = 0.3;
            })),
            ("Boulder", roll_preset(3.0, |p| {
                p.bumps = 0.9;
                p.rumble = 0.7;
                p.wobble = 0.6;
                p.rumble_hz = 250.0;
            })),
            ("Plastic ball", roll_preset(4.0, |_| {})),
            ("Barrel", roll_preset(1.0, |p| {
                p.ring = 0.4;
                p.wobble = 0.5;
                p.rumble_hz = 300.0;
                p.pitch = -5.0;
            })),
            ("Bowling ball", roll_preset(3.0, |p| {
                p.rumble = 0.8;
                p.bumps = 0.3;
                p.ring = 0.5;
                p.pitch = -7.0;
                p.rumble_hz = 1500.0;
                p.floor = 0.9;
            })),
        ]
    }

    fn new(sr: f32) -> Self {
        Roll {
            sr,
            bank: ModeBank::new(),
            noise: Noise::new(mix_seed(0x8011)),
            rng: Rng::new(mix_seed(0x8012)),
            bump: [OnePole::default(); 2],
            click_hp: OnePole::default(),
            band: [OnePole::default(); 2],
            band_hp: OnePole::default(),
            slide_hp: OnePole::default(),
            slide_lp: OnePole::default(),
            chatter: OnePole::default(),
            phase: 0.0,
            turn: 1.0,
            uneven: SlowNoise::new(0x8013),
            tuned: (-1.0, -1.0, -1.0, 0.0),
        }
    }

    fn block(&mut self, x: &[f32], p: &RollParams, out: &mut [f32]) {
        let sr = self.sr;
        let (speed, surface, size, slide) = (x[0], x[1], x[2], x[3]);
        let mat = material(p.kind);
        let s = size_scale(size);
        // Retune when the material, size, ring or pitch has moved (sizes quantised to 1/64).
        let key = (p.kind.round(), (size * 64.0).round(), p.ring, p.pitch);
        if key != self.tuned {
            self.tuned = key;
            let tune = (p.pitch / 12.0).exp2() / s;
            // The floor holds the object: it rings shorter while it rolls than when struck.
            let ring = p.ring * s.powf(0.6) * 0.5;
            self.bank.n = mat.modes.len();
            for (k, mode) in mat.modes.iter().enumerate() {
                self.bank.tune(k, mat.hz * mode.ratio * tune, (mode.t60 * ring).clamp(0.004, 6.0), sr);
                if self.bank.c[k] != 0.0 || self.bank.s[k] != 0.0 {
                    self.bank.g[k] = (mode.db / 20.0 * core::f32::consts::LN_10).exp() * mat.level;
                }
            }
        }
        // Every mode stays in the loop while the object moves (energy() retires silent ones).
        if speed > 1e-4 {
            for k in 0..self.bank.n {
                self.bank.live[k] = self.bank.g[k] != 0.0;
            }
        }
        let v = speed * p.top_speed;
        // Rotation rate of a ball 6 cm across times the size scale: out-of-roundness, seams.
        let rot_inc = v / (core::f32::consts::PI * 0.06 * s) / sr;
        // Harder ball and floor: a smaller, stiffer contact and a higher band.
        let hard = (0.25 / mat.contact_ms).sqrt().clamp(0.7, 2.0) * (0.6 + 0.8 * p.floor);
        let band_hz = (p.rumble_hz * (0.4 + 1.2 * speed) * hard / s.powf(0.3)).clamp(60.0, 0.4 * sr);
        let band_coef = hz_coef(band_hz, sr);
        let band_hp = hz_coef(band_hz * 0.25, sr);
        // Two one-poles on white noise leave about sqrt(coef / 4) of its level.
        let band_norm = (4.0 / band_coef).sqrt() * 0.5;
        let band_amp = p.rumble * speed.powf(1.3) * s.powf(0.4) * (0.6 + 0.4 * surface) * 0.25;
        // Bumps: one per grain of the floor passed; a rough floor has bigger, sparser grains.
        // Their sizes follow a power law, so a few clack well above the rest.
        let grain_m = 0.02 + 0.08 * surface;
        let bump_p = (v / grain_m * (0.2 + 0.8 * p.bumps) / sr).min(0.5);
        let bump_amp = speed.powf(1.2) * (0.1 + 1.4 * surface) * p.bumps;
        let bump_coef = hz_coef(band_hz * 1.5, sr);
        // An impulse through two one-poles peaks at about coef / e: heard directly, a clack
        // peaks at its drawn size; into the modes it is a unit-area push.
        let click_gain = 1.0 / (0.37 * bump_coef);
        let modes_amp = p.modes * 2.0;
        // Sliding: friction noise with stick-slip chatter.
        let slide_amp = slide * speed.sqrt() * p.friction * 0.25;
        let slide_hp = hz_coef(300.0, sr);
        let slide_lp = hz_coef(2500.0 + 4000.0 * p.friction, sr);
        let chatter_coef = hz_coef(20.0 + 60.0 * speed, sr);
        let wobble = p.wobble;
        let gain = p.gain;
        // Up to +-8 dB of slow swell on a rough floor, as in the recordings.
        let swell = (self.uneven.advance(2.0 + 4.0 * speed, out.len() as f32 / sr) * (0.4 + 0.5 * surface)).exp2().powf(1.3);
        for o in out.iter_mut() {
            self.phase += rot_inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                // Each turn lands a little differently.
                self.turn = 0.4 + 0.6 * self.rng.next_f32();
            }
            let roll_env = (1.0 + wobble * self.turn * (TAU * self.phase).sin()) * swell;
            let mut ex = 0.0;
            if self.rng.next_f32() < bump_p {
                let a = self.rng.next_f32();
                ex = a * a * a * a * bump_amp * roll_env;
            }
            let ex = self.bump[0].lp(ex, bump_coef);
            let ex = self.bump[1].lp(ex, bump_coef);
            let click = self.click_hp.hp(ex, band_hp) * click_gain;
            let w = self.noise.white();
            let b = self.band[0].lp(w, band_coef);
            let b = self.band[1].lp(b, band_coef);
            let b = self.band_hp.hp(b, band_hp) * band_amp * band_norm * roll_env;
            let mut drive = ex + b * 0.2;
            let mut y = click + b;
            if slide_amp > 0.0 {
                let ch = self.chatter.lp(self.noise.white(), chatter_coef) * 4.0;
                let f = self.slide_lp.lp(self.slide_hp.hp(self.noise.white(), slide_hp), slide_lp) * slide_amp * (0.3 + ch.abs());
                drive += f * 0.5;
                y += f;
            }
            y += self.bank.tick(drive * modes_amp);
            *o = y * gain;
        }
        self.bank.energy(1e-7);
    }
}
