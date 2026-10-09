//! Footsteps and body foley: one generator per walker.
//!
//! **How a game drives it.** `footstep` is a continuous generator that stays silent until the
//! foot lands: the game calls `trigger()` at each footfall (from an animation event or its own
//! stride timer) and sets the inputs the step is made of: `surface` under the foot, `speed`,
//! `weight`, `shoe`. Each step takes the inputs as they are at that instant, so switching
//! `surface` changes the next step and leaves the tail of the last one alone. Steps overlap on a
//! small voice pool, so a run or a landing never cuts the step before it. Without animation
//! events, set `pace` and the walker steps by itself (`pace` x 4 steps per second).
//!
//! It is continuous rather than a one-shot because a one-shot playback ends after each step,
//! and every new playback would start from the same seed: the first step would be identical
//! every time, which is the very flaw this is for. One long-lived player per walker keeps one
//! random stream, alternates left and right feet, and costs nothing between steps.
//!
//! **Surface is an input, not a preset.** The ground changes under a walker all the time (the
//! game raycasts it at each footfall), and a preset would overwrite the walker's tuning
//! (`master/gain`, cloth, brightness) and the step still ringing out. The presets describe the
//! walker instead.
//!
//! **Tuned against recordings** (CC0, Freesound; see `tests/foley.rs` for what each surface must
//! keep). A footstep is a mid-to-low sound: on every dry surface the step's spectral centroid
//! is 300 Hz to 1.1 kHz and under 6 % of its energy is above 2.5 kHz. What tells surfaces apart
//! is the shape in time: stone falls 20 dB in about 13 ms, grass is a short dull thump, wood
//! adds a low floor boom, gravel and snow keep crunching for 100-250 ms after the heel (snow
//! denser and softer, and it stops when the foot lifts), metal rings with partials near 0.6 to
//! 3 kHz for a quarter of a second. Shallow water and mud are the bright ones: splash and
//! squelch put 60-77 % of the recordings' energy above 2.5 kHz (we stay near 50 %). Walking, the
//! toe lands 60-140 ms after the heel, and step peaks spread by 3-5 dB.
//!
//! **Level.** At the default inputs a walk's median step peaks near -11 dBFS on every surface
//! (stone and wood 2 dB hotter, as they are peakier), its loudest steps near -4 dBFS where
//! `rock_hit` at full power peaks; running adds about 5 dB, a heavy walker 2 to 3, a landing
//! more, and the output limiter catches the rest.
//!
//! Also here: a landing (`land`: both feet at once), and a cloth swish as the other leg swings
//! through (`cloth/level`, louder with speed).

use core::sync::atomic::{AtomicU32, Ordering};

use crate::blocks::{hz_coef, mix_seed, OnePole, SlowNoise};
use crate::filter::{FilterMode, Svf};
use crate::math::{Rng, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::params::{exp, lin, GAIN, UNIT};

model_params! {
    /// Tuning of one walker. Surface, speed, weight and shoe are inputs (they change per step).
    FootstepParams / FootstepParamId {
        variation: "step/variation" = 0.5, UNIT;
        brightness: "step/brightness" = 0.5, UNIT;
        roll: "step/roll" = 1.0, lin(0.5, 2.0);
        thump: "body/thump" = 1.0, GAIN;
        crunch: "ground/crunch" = 1.0, GAIN;
        ring: "ground/ring" = 1.0, GAIN;
        splash: "ground/splash" = 1.0, GAIN;
        scuff: "foot/scuff" = 1.0, GAIN;
        cloth_level: "cloth/level" = 0.1, UNIT;
        cloth_hz: "cloth/hz" = 1400.0, exp(400.0, 5000.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// The surfaces, in `surface` input order: the input is the index / 10 (gravel 0, grass 0.1,
/// dirt 0.2, wood 0.3, stone 0.4, snow 0.5, metal 0.6, water 0.7, mud 0.8), rounded to the
/// nearest, so new surfaces can be appended without moving these.
pub const SURFACES: [&str; 9] = ["gravel", "grass", "dirt", "wood", "stone", "snow", "metal", "water", "mud"];

/// What the ground does with a step. Times in seconds (decays to -60 dB), frequencies in Hz.
#[derive(Clone, Copy, Debug)]
struct Material {
    /// The impact: a noise burst through a resonant low-pass; a plate has no low end, so its
    /// burst is high-passed first (`body_hp`, 0 = off). `give` is how long loose ground takes to
    /// yield under the heel (the burst's attack).
    body_hp: f32,
    give: f32,
    body_hz: f32,
    body_res: f32,
    body_decay: f32,
    body: f32,
    /// Weight of the body arriving: a decaying low sine (a floor boom on wood).
    thump_hz: f32,
    thump_decay: f32,
    thump: f32,
    /// Hard sole on a hard floor: a short click, scaled by `shoe` hardness.
    click_hz: f32,
    click: f32,
    /// Toe strike level relative to the heel, and how slowly the foot rolls onto it.
    toe: f32,
    roll: f32,
    /// Crunch: grains per second at its height, level, band, grain decay, how long the ground
    /// keeps settling, how long it takes to build up, and how few grains are loud (1 = even).
    grains: f32,
    crunch: f32,
    grain_lo: f32,
    grain_hi: f32,
    grain_res: f32,
    grain_s: f32,
    crunch_len: f32,
    crunch_rise: f32,
    /// 1: the crunch dies away; higher: it holds, then stops when the foot lifts (snow).
    crunch_shape: f32,
    skew: f32,
    /// Modes the impact excites: (Hz, T60, gain).
    modes: [(f32, f32, f32); 4],
    ring: f32,
    /// The sole sliding as the foot rolls off.
    scuff_hz: f32,
    scuff: f32,
    /// Water: splash and droplets; mud: the suck as the foot pulls out.
    splash: f32,
    suck: f32,
    /// Level trim in dB, so that a walk on each surface peaks where the others do.
    trim_db: f32,
}

const NO_MODES: [(f32, f32, f32); 4] = [(100.0, 0.05, 0.0); 4];

const M: Material = Material {
    body_hp: 0.0,
    give: 0.0,
    body_hz: 800.0,
    body_res: 0.2,
    body_decay: 0.05,
    body: 1.0,
    thump_hz: 85.0,
    thump_decay: 0.08,
    thump: 0.5,
    click_hz: 2500.0,
    click: 0.0,
    toe: 0.6,
    roll: 1.0,
    grains: 0.0,
    crunch: 0.0,
    grain_lo: 500.0,
    grain_hi: 3000.0,
    grain_res: 0.4,
    grain_s: 0.004,
    crunch_len: 0.2,
    crunch_rise: 0.02,
    crunch_shape: 1.0,
    skew: 2.0,
    modes: NO_MODES,
    ring: 0.0,
    scuff_hz: 1200.0,
    scuff: 0.2,
    splash: 0.0,
    suck: 0.0,
    trim_db: 0.0,
};

const MATERIALS: [Material; 9] = [
    // Gravel: stones shift and grind for 100-200 ms after the heel, a few loud ones among many.
    Material { give: 0.012, body_hz: 800.0, body: 0.45, thump: 0.25, toe: 0.3, grains: 700.0, crunch: 1.0, grain_lo: 300.0, grain_hi: 2000.0, grain_s: 0.004, crunch_len: 0.07, crunch_rise: 0.025, skew: 2.5, scuff_hz: 900.0, scuff: 0.15, click: 0.1, trim_db: 0.4, ..M },
    // Grass (a lawn): a dull short thump with a faint swish of blades.
    Material { give: 0.003, body_hz: 320.0, body_res: 0.1, body_decay: 0.045, body: 1.0, thump_hz: 120.0, thump_decay: 0.08, thump: 0.9, toe: 0.4, roll: 1.2, grains: 1500.0, crunch: 0.12, grain_lo: 1000.0, grain_hi: 5000.0, grain_s: 0.0015, crunch_len: 0.08, crunch_rise: 0.01, skew: 1.0, scuff_hz: 1500.0, scuff: 0.06, trim_db: -2.4, ..M },
    // Dirt (packed earth): a thud at heel and toe, a little grit.
    Material { body_hz: 360.0, body_res: 0.15, body_decay: 0.06, body: 1.0, thump_hz: 105.0, thump: 0.7, toe: 0.8, roll: 1.1, click: 0.2, click_hz: 1800.0, grains: 220.0, crunch: 0.2, grain_lo: 400.0, grain_hi: 3000.0, grain_s: 0.003, crunch_len: 0.16, skew: 2.0, scuff_hz: 700.0, scuff: 0.25, trim_db: -3.6, ..M },
    // Wood floor: a knock, the boom of the boards and a few short body modes.
    Material { body_hz: 480.0, body_res: 0.3, body_decay: 0.035, body: 1.2, thump_hz: 100.0, thump_decay: 0.12, thump: 0.4, click_hz: 2200.0, click: 0.7, toe: 0.6, modes: [(110.0, 0.45, 0.6), (260.0, 0.1, 0.7), (540.0, 0.07, 0.4), (1010.0, 0.04, 0.2)], ring: 0.5, scuff_hz: 1300.0, scuff: 0.12, trim_db: -2.9, ..M },
    // Stone and concrete: the sharpest and shortest; grit under the sole.
    Material { body_hz: 600.0, body_res: 0.25, body_decay: 0.02, body: 1.0, thump_hz: 110.0, thump_decay: 0.05, thump: 0.35, click_hz: 2600.0, click: 1.0, toe: 0.35, roll: 0.9, grains: 120.0, crunch: 0.06, grain_lo: 1000.0, grain_hi: 3500.0, grain_s: 0.0015, crunch_len: 0.05, crunch_rise: 0.005, scuff_hz: 1500.0, scuff: 0.04, trim_db: 1.4, ..M },
    // Snow: a dense soft crunch that builds as the snow compacts, mostly under 3 kHz.
    Material { give: 0.008, body_hz: 380.0, body_res: 0.1, body_decay: 0.06, body: 1.6, thump_hz: 100.0, thump: 1.2, toe: 0.6, roll: 1.3, grains: 2200.0, crunch: 0.35, grain_lo: 200.0, grain_hi: 2000.0, grain_res: 0.3, grain_s: 0.005, crunch_len: 0.15, crunch_rise: 0.025, crunch_shape: 3.0, skew: 1.2, scuff_hz: 700.0, scuff: 0.25, trim_db: -3.8, ..M },
    // Metal plate or grating: struck partials ringing a quarter of a second, a little rattle.
    Material { body_hp: 350.0, body_hz: 800.0, body_res: 0.3, body_decay: 0.16, body: 1.6, thump_hz: 110.0, thump: 0.12, click_hz: 2800.0, click: 0.7, toe: 0.4, grains: 60.0, crunch: 0.12, grain_lo: 700.0, grain_hi: 2400.0, grain_res: 0.85, grain_s: 0.006, crunch_len: 0.15, modes: [(590.0, 0.3, 0.7), (1080.0, 0.36, 1.0), (1790.0, 0.27, 0.7), (2870.0, 0.18, 0.5)], ring: 0.3, scuff_hz: 1500.0, scuff: 0.12, trim_db: -10.4, ..M },
    // Shallow water: a splash with droplets over a low slosh.
    Material { give: 0.01, body_hz: 330.0, body_res: 0.1, body_decay: 0.07, body: 0.4, thump_hz: 110.0, thump: 0.4, toe: 0.6, roll: 1.2, grains: 1800.0, crunch: 1.3, grain_lo: 1200.0, grain_hi: 9000.0, grain_res: 0.3, grain_s: 0.004, crunch_len: 0.07, crunch_rise: 0.012, skew: 2.0, scuff: 0.0, splash: 1.0, trim_db: -7.4, ..M },
    // Mud: a wet slap, squelch and the suck of the boot pulling out.
    Material { give: 0.008, body_hz: 420.0, body_res: 0.2, body_decay: 0.07, body: 0.6, thump_hz: 90.0, thump: 0.3, toe: 0.7, roll: 1.3, grains: 900.0, crunch: 0.8, grain_lo: 2000.0, grain_hi: 9000.0, grain_res: 0.5, grain_s: 0.003, crunch_len: 0.07, skew: 1.8, scuff: 0.0, splash: 0.2, suck: 1.0, trim_db: -1.5, ..M },
];

// Layer levels, set by measuring renders against the recordings.
const BODY_GAIN: f32 = 6.0;
const THUMP_GAIN: f32 = 0.55;
const CLICK_GAIN: f32 = 1.4;
const CRUNCH_GAIN: f32 = 2.4;
const RING_GAIN: f32 = 0.6;
const SCUFF_GAIN: f32 = 0.8;
const SPLASH_GAIN: f32 = 1.4;
const DROP_GAIN: f32 = 0.12;
const SUCK_GAIN: f32 = 0.1;
const CLOTH_GAIN: f32 = 0.2;

const VOICES: usize = 4;

/// A two-pole resonator: one ringing mode, or a decaying sine when struck.
#[derive(Clone, Copy, Debug, Default)]
struct Mode {
    y1: f32,
    y2: f32,
    a1: f32,
    a2: f32,
    /// sin(w): striking with `a * s` rings at amplitude `a`.
    s: f32,
}

impl Mode {
    fn set(&mut self, hz: f32, t60: f32, sr: f32) {
        let w = TAU * hz.clamp(20.0, sr * 0.45) / sr;
        let r = (-6.9 / (t60.max(0.003) * sr)).exp();
        self.a1 = 2.0 * r * w.cos();
        self.a2 = -r * r;
        self.s = w.sin();
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = x + self.a1 * self.y1 + self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = if y.abs() < 1e-20 { 0.0 } else { y };
        y
    }
}

/// A water droplet: a sine ping whose pitch rises as the bubble closes.
#[derive(Clone, Copy, Debug, Default)]
struct Drop {
    re: f32,
    im: f32,
    c: f32,
    s: f32,
    hz: f32,
    env: f32,
    decay: f32,
}

/// One footfall, from the heel strike until it has rung out.
#[derive(Clone, Copy, Debug)]
struct Step {
    on: bool,
    /// Seconds since the heel strike (negative while waiting inside the block).
    t: f32,
    mat: usize,
    // Rolled at the strike.
    level: f32,
    tone: f32,
    toe_at: f32,
    toe_level: f32,
    toe_done: bool,
    attack: f32,
    click: f32,
    thump: f32,
    crunch_rate: f32,
    crunch_rise: f32,
    crunch_len: f32,
    scuff_at: f32,
    scuff_len: f32,
    scuff: f32,
    splash: f32,
    suck_at: f32,
    cloth_at: f32,
    cloth: f32,
    length: f32,
    // State.
    ex: f32,
    ex_soft: f32,
    click_env: f32,
    body_lo: OnePole,
    body: Svf,
    click_bp: Svf,
    thump_mode: Mode,
    grain_env: f32,
    grain_soft: f32,
    grain: [Svf; 2],
    grain_next: usize,
    modes: [Mode; 4],
    scuff_bp: Svf,
    splash_hp: Svf,
    slosh: Svf,
    drops: [Drop; 2],
    drop_next: usize,
    suck: Drop,
    cloth_bp: Svf,
}

fn idle() -> Step {
    Step {
        on: false,
        t: 0.0,
        mat: 0,
        level: 0.0,
        tone: 1.0,
        toe_at: 0.1,
        toe_level: 0.0,
        toe_done: true,
        attack: 0.5,
        click: 0.0,
        thump: 0.0,
        crunch_rate: 0.0,
        crunch_rise: 0.02,
        crunch_len: 0.2,
        scuff_at: 0.1,
        scuff_len: 0.1,
        scuff: 0.0,
        splash: 0.0,
        suck_at: 0.2,
        cloth_at: 0.1,
        cloth: 0.0,
        length: 0.5,
        ex: 0.0,
        ex_soft: 0.0,
        click_env: 0.0,
        body_lo: OnePole::default(),
        body: Svf::default(),
        click_bp: Svf::default(),
        thump_mode: Mode::default(),
        grain_env: 0.0,
        grain_soft: 0.0,
        grain: [Svf::default(); 2],
        grain_next: 0,
        modes: [Mode::default(); 4],
        scuff_bp: Svf::default(),
        splash_hp: Svf::default(),
        slosh: Svf::default(),
        drops: [Drop::default(); 2],
        drop_next: 0,
        suck: Drop::default(),
        cloth_bp: Svf::default(),
    }
}

/// Footsteps for one walker. See the module docs.
pub struct Footstep {
    sr: f32,
    noise: Noise,
    rng: Rng,
    steps: [Step; VOICES],
    /// Which foot lands next (0 left, 1 right), and this walker's fixed difference between them.
    foot: usize,
    feet: [(f32, f32, f32); 2],
    /// `pace` stepping: position in the stride, and this stride's length jitter.
    phase: f32,
    stride: f32,
    flutter: SlowNoise,
}

/// Every instance gets its own random stream, so two walkers never step in unison.
static INSTANCES: AtomicU32 = AtomicU32::new(0);

impl Footstep {
    fn fire(&mut self, x: &[f32], p: &FootstepParams, offset: f32) {
        let (speed, weight, shoe, land) = (x[1], x[3], x[4], x[5]);
        let mat_index = ((x[2] * 10.0 + 0.5) as usize).min(MATERIALS.len() - 1);
        let mat = &MATERIALS[mat_index];
        let v = p.variation * 2.0;
        // Take a free voice, or the one that has sounded longest.
        let k = (0..VOICES).find(|&k| !self.steps[k].on).unwrap_or_else(|| {
            (0..VOICES).max_by(|&a, &b| self.steps[a].t.total_cmp(&self.steps[b].t)).unwrap_or(0)
        });
        // This walker's uneven feet, as much as `step/variation` allows.
        let (fl, ft, fr) = self.feet[self.foot];
        let (foot_level, foot_tone, foot_roll) = (1.0 + (fl - 1.0) * v, 1.0 + (ft - 1.0) * v, 1.0 + (fr - 1.0) * v);
        self.foot ^= 1;
        let r = &mut self.rng;
        // A rough unit gaussian (the sum of three uniforms).
        let gauss = |r: &mut Rng| (r.next_bipolar() + r.next_bipolar() + r.next_bipolar()).clamp(-2.5, 2.5);
        let landing = land > 0.02;
        let mass = 0.55 + 0.9 * weight;
        let force = (0.3 + 1.1 * speed) * mass.powf(0.7) * (1.0 + 1.3 * land);
        // Steps spread by about 3 dB at the default variation.
        let level = force * foot_level * (v * 0.5 * gauss(r)).exp2() * (mat.trim_db / 20.0 * core::f32::consts::LOG2_10).exp2();
        // Heavier is lower; harder and faster is brighter.
        let bright = (2.0 * (p.brightness - 0.5)).exp2();
        let tone = bright * (1.15 - 0.35 * weight) * (0.85 + 0.3 * speed) * foot_tone * (v * 0.18 * gauss(r)).exp2();
        // Walking, the toe lands 60-140 ms after the heel; running, the foot strikes nearly flat.
        let gap = (0.105 - 0.075 * speed) * mat.roll * p.roll * foot_roll * (1.0 + v * 0.12 * gauss(r));
        let (toe_at, toe_level) = if landing {
            // Both feet, a few milliseconds apart.
            (r.range(0.006, 0.028), r.range(0.6, 0.95))
        } else {
            (gap.max(0.012), (mat.toe * (0.7 + 0.5 * speed) * (v * 0.6 * gauss(r)).exp2()).clamp(0.1, 1.2))
        };
        let hard = shoe * shoe * shoe;
        let scuff_roll = r.range(0.2, 1.0);
        let scuff = if landing { 0.0 } else { mat.scuff * p.scuff * (0.35 + 0.9 * speed) * (1.0 - v * 0.5 + v * scuff_roll) };
        let s = &mut self.steps[k];
        *s = Step {
            on: true,
            t: -offset,
            mat: mat_index,
            level: level * p.gain,
            tone,
            toe_at,
            toe_level,
            toe_done: false,
            // A running foot strikes harder: loose ground gives way sooner.
            attack: 1.0 - (-1.0 / ((0.0035 - 0.003 * shoe + mat.give * (1.2 - 0.7 * speed)) * self.sr)).exp(),
            click: mat.click * hard * (0.6 + 0.6 * speed),
            thump: mat.thump * p.thump * mass * (1.0 + 1.5 * land),
            crunch_rate: mat.grains * (0.75 + 0.5 * weight) * (0.8 + 0.6 * speed) * (1.0 + 0.5 * land) * (1.0 + v * 0.25 * r.next_bipolar()),
            crunch_rise: mat.crunch_rise * (1.2 - 0.7 * speed),
            crunch_len: mat.crunch_len * (0.85 + 0.3 * weight) * (1.15 - 0.5 * speed) * (1.0 + 0.6 * land),
            scuff_at: toe_at + r.range(0.0, 0.03),
            scuff_len: 0.04 + 0.06 * r.next_f32(),
            scuff,
            splash: mat.splash * p.splash * (0.7 + 0.5 * speed) * (1.0 + 0.8 * land),
            suck_at: 0.16 + 0.12 * r.next_f32() - 0.08 * speed,
            cloth_at: 0.08 + 0.12 * (1.0 - speed) + 0.04 * r.next_f32(),
            cloth: p.cloth_level * (0.4 + 0.9 * speed) * (1.0 + 1.5 * land) * r.range(0.5, 1.0),
            ex: 0.0,
            ex_soft: 0.0,
            click_env: 0.0,
            grain_env: 0.0,
            grain_soft: 0.0,
            drop_next: 0,
            ..idle()
        };
        // Long enough for the slowest layer to ring out.
        let ring_t = if mat.ring > 0.0 { mat.modes.iter().map(|m| m.1).fold(0.0, f32::max) } else { 0.0 };
        s.length = (toe_at + (mat.crunch_len * 1.6 + 0.05).max(ring_t + 0.05).max(mat.thump_decay * 1.5 + 0.05)).max(s.cloth_at + 0.35);
        if mat.suck > 0.0 {
            s.length = s.length.max(s.suck_at + 0.2);
        }
        if mat.splash > 0.0 {
            s.length = s.length.max(0.4);
        }
        let sr = self.sr;
        s.body.set(FilterMode::LowPass, mat.body_hz * tone, mat.body_res, sr);
        s.click_bp.set(FilterMode::BandPass, mat.click_hz * tone.sqrt() * (0.8 + 0.4 * shoe), 0.4, sr);
        s.thump_mode.set(mat.thump_hz * (1.2 - 0.35 * weight) * (1.0 + v * 0.06 * gauss(r)), mat.thump_decay * (0.8 + 0.4 * weight), sr);
        for (m, &(hz, t60, _)) in s.modes.iter_mut().zip(&mat.modes) {
            // Every plank and plate is a little different, and where the foot lands moves the modes.
            m.set(hz * (1.0 + v * 0.05 * r.next_bipolar()), t60 * (1.0 + v * 0.2 * r.next_bipolar()), sr);
        }
        s.scuff_bp.set(FilterMode::BandPass, mat.scuff_hz * tone, 0.3, sr);
        s.splash_hp.set(FilterMode::HighPass, 900.0 * tone.sqrt(), 0.15, sr);
        s.slosh.set(FilterMode::LowPass, 450.0 * tone, 0.3, sr);
        s.cloth_bp.set(FilterMode::BandPass, p.cloth_hz * (0.85 + 0.3 * r.next_f32()), 0.15, sr);
        // Heel strike.
        s.ex = 1.0;
        s.click_env = 1.0;
        let a = s.thump * s.thump_mode.s;
        s.thump_mode.y1 += a;
    }

    fn render_step(&mut self, k: usize, p: &FootstepParams, flutter: f32, out: &mut [f32]) {
        let sr = self.sr;
        let dt = 1.0 / sr;
        let s = &mut self.steps[k];
        let mat = &MATERIALS[s.mat];
        let level = s.level;
        let ex_decay = (-6.9 / (mat.body_decay * s.tone.sqrt().recip() * sr)).exp();
        let click_decay = (-6.9 / (0.012 * sr)).exp();
        let body_hp = if mat.body_hp > 0.0 { hz_coef(mat.body_hp * s.tone, sr) } else { 0.0 };
        let body_gain = mat.body * BODY_GAIN * (s.tone.recip()).sqrt();
        let click_gain = s.click * CLICK_GAIN;
        let crunch_gain = mat.crunch * p.crunch * CRUNCH_GAIN;
        let grain_decay = (-1.0 / (mat.grain_s * sr)).exp();
        let grain_attack = 1.0 - (-1.0 / (0.0008 * sr)).exp();
        // A noise burst of time constant tau samples rings a mode at about sqrt(tau / 2) times
        // what a unit impulse does; divide that out so `ring` is the modes' level next to the body.
        let tau = mat.body_decay / s.tone.sqrt() * sr / 6.9;
        let ring_gain = mat.ring * p.ring * RING_GAIN / (0.5 * tau).sqrt();
        let splash_gain = s.splash * SPLASH_GAIN;
        let suck_gain = mat.suck * p.splash * SUCK_GAIN;
        let cloth_gain = s.cloth * CLOTH_GAIN;
        // Droplet pitches rise, and the suck falls, at control rate.
        for d in s.drops.iter_mut().chain(core::iter::once(&mut s.suck)) {
            if d.env > 1e-4 {
                let w = TAU * d.hz / sr;
                (d.c, d.s) = (w.cos(), w.sin());
            }
        }
        let block = out.len() as f32 * dt;
        let rise = (block * 9.0).exp2();
        for d in s.drops.iter_mut() {
            d.hz = (d.hz * rise).min(sr * 0.4);
        }
        s.suck.hz = (s.suck.hz * (block * -2.5).exp2()).max(60.0);
        // The slow envelopes, at the middle of this block (a block is under a millisecond).
        let tm = (s.t + 0.5 * block).max(0.0);
        let scuff_env = if s.scuff > 0.0 && tm > s.scuff_at && tm < s.scuff_at + s.scuff_len {
            (((tm - s.scuff_at) / s.scuff_len) * core::f32::consts::PI).sin() * s.scuff * SCUFF_GAIN
        } else {
            0.0
        };
        // Crunch: grains fire at a rate that builds over `crunch_rise`, settles over
        // `crunch_len`, and gets a second push from the toe and the scuff.
        let density = if crunch_gain > 0.0 {
            let build = 1.0 - (-tm / s.crunch_rise.max(0.002)).exp();
            let settle = (-(tm / s.crunch_len).powf(mat.crunch_shape)).exp();
            let toe = if tm > s.toe_at { 0.4 * s.toe_level * (-(tm - s.toe_at) / (0.5 * s.crunch_len)).exp() } else { 0.0 };
            build * settle + toe + 0.3 * scuff_env / SCUFF_GAIN
        } else {
            0.0
        };
        let grain_p = s.crunch_rate * density / sr;
        let grain_scale = density.min(1.0).sqrt();
        // Water: the slosh under the splash, and droplets falling back.
        let (slosh_env, drop_p) = if splash_gain > 0.0 {
            let sl = (1.0 - (-tm / 0.03).exp()) * (-tm / 0.12).exp();
            (sl * 0.9 * splash_gain, 250.0 * (-tm / 0.08).exp() * s.splash / sr)
        } else {
            (0.0, 0.0)
        };
        let splash_env = if splash_gain > 0.0 { (1.0 - (-tm / 0.01).exp()) * (-tm / 0.06).exp() * 0.3 * splash_gain } else { 0.0 };
        // Trousers and sleeves swishing as the other leg swings through.
        let cloth_env = if cloth_gain > 0.0 && tm > s.cloth_at && tm < s.cloth_at + 0.3 {
            let e = (((tm - s.cloth_at) / 0.3) * core::f32::consts::PI).sin();
            e * e * flutter * cloth_gain
        } else {
            0.0
        };
        if suck_gain > 0.0 && tm >= s.suck_at && s.suck.decay == 0.0 {
            // The boot pulling out of mud: a falling low chirp.
            let hz = self.rng.range(180.0, 320.0);
            let w0 = TAU * hz / sr;
            s.suck = Drop { re: 1.0, im: 0.0, c: w0.cos(), s: w0.sin(), hz, env: self.rng.range(0.5, 1.0), decay: (-6.9 / (0.12 * sr)).exp() };
        }
        for o in out.iter_mut() {
            s.t += dt;
            if s.t < 0.0 {
                continue;
            }
            if !s.toe_done && s.t >= s.toe_at {
                s.toe_done = true;
                s.ex = s.ex.max(s.toe_level);
                s.click_env = s.click_env.max(s.toe_level * 0.8);
                let a = s.thump * 0.45 * s.toe_level * s.thump_mode.s;
                s.thump_mode.y1 += a;
            }
            let w = self.noise.white();
            // Pink feeds the grains, scuff and cloth, so their tops fall away as in recordings.
            let pk = self.noise.pink() * 3.0;
            s.ex_soft += (s.ex - s.ex_soft) * s.attack;
            s.ex *= ex_decay;
            let excite = w * s.ex_soft;
            let impact = s.body.tick(if body_hp > 0.0 { s.body_lo.hp(excite, body_hp) } else { excite });
            let mut y = impact * body_gain + s.thump_mode.tick(0.0) * THUMP_GAIN;
            if click_gain > 0.0 && s.click_env > 1e-4 {
                y += s.click_bp.tick(w * s.click_env) * click_gain;
                s.click_env *= click_decay;
            }
            if ring_gain > 0.0 {
                let mut ring = 0.0;
                for (m, &(_, _, g)) in s.modes.iter_mut().zip(&mat.modes) {
                    ring += m.tick(excite * m.s) * g;
                }
                y += ring * ring_gain;
            }
            if scuff_env > 0.0 {
                // The sole sliding as the foot rolls off.
                y += s.scuff_bp.tick(pk) * scuff_env;
            }
            if crunch_gain > 0.0 {
                if self.rng.next_f32() < grain_p {
                    // A few big stones among many small ones, the big ones under the heel and toe.
                    let a = self.rng.next_f32().powf(mat.skew) * grain_scale;
                    s.grain_env = s.grain_env.max(a);
                    s.grain_next ^= 1;
                    let hz = mat.grain_lo * (mat.grain_hi / mat.grain_lo).powf(self.rng.next_f32()) * s.tone;
                    s.grain[s.grain_next].set(FilterMode::BandPass, hz, mat.grain_res, sr);
                }
                if s.grain_env > 1e-4 || s.grain_soft > 1e-4 {
                    s.grain_soft += (s.grain_env - s.grain_soft) * grain_attack;
                    let burst = pk * s.grain_soft;
                    let (a, b) = if s.grain_next == 0 { (burst, 0.0) } else { (0.0, burst) };
                    y += (s.grain[0].tick(a) + s.grain[1].tick(b)) * crunch_gain;
                    s.grain_env *= grain_decay;
                }
            }
            if splash_gain > 0.0 {
                y += s.splash_hp.tick(pk) * splash_env + s.slosh.tick(w) * slosh_env;
                if self.rng.next_f32() < drop_p {
                    s.drop_next ^= 1;
                    let d = &mut s.drops[s.drop_next];
                    d.hz = self.rng.range(900.0, 3200.0) * s.tone.sqrt();
                    let w0 = TAU * d.hz / sr;
                    (d.c, d.s, d.re, d.im) = (w0.cos(), w0.sin(), 1.0, 0.0);
                    d.env = self.rng.range(0.3, 1.0);
                    d.decay = (-6.9 / (self.rng.range(0.012, 0.035) * sr)).exp();
                }
                for d in s.drops.iter_mut() {
                    if d.env > 1e-4 {
                        (d.re, d.im) = (d.re * d.c - d.im * d.s, d.re * d.s + d.im * d.c);
                        y += d.im * d.env * DROP_GAIN * splash_gain;
                        d.env *= d.decay;
                    }
                }
            }
            if s.suck.env > 1e-4 {
                let d = &mut s.suck;
                (d.re, d.im) = (d.re * d.c - d.im * d.s, d.re * d.s + d.im * d.c);
                y += d.im * d.env * suck_gain;
                d.env *= d.decay;
            }
            if cloth_env > 0.0 {
                y += s.cloth_bp.tick(pk) * cloth_env;
            }
            *o += y * level;
        }
        if s.t > s.length {
            s.on = false;
        }
    }
}

impl Generator for Footstep {
    type P = FootstepParams;
    const NAME: &'static str = "footstep";
    const CATEGORY: &'static str = "foley";
    const DOC: &'static str = "Footsteps on gravel, grass, dirt, wood, stone, snow, metal, shallow water and mud. One per walker: trigger() at each footfall (or set pace), surface/speed/weight/shoe per step; land for jumps; cloth swish.";
    // `pace` comes first so that a generic input sweep (the test bench, the web lab's first
    // slider) walks; a game that triggers its own steps leaves it at 0.
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "pace", default: 0.0, doc: "0: steps only on trigger(). Above 0 the walker steps by itself at pace x 4 steps/s (0.45 walk, 0.7 run)" },
        InputSpec { name: "speed", default: 0.3, doc: "0 a slow walk, 0.3 walking, 0.7 running, 1 sprinting: impact force, scuff, heel-to-toe roll" },
        InputSpec {
            name: "surface",
            default: 0.0,
            doc: "Ground under the next step, index / 10: 0 gravel, 0.1 grass, 0.2 dirt, 0.3 wood, 0.4 stone, 0.5 snow, 0.6 metal, 0.7 shallow water, 0.8 mud",
        },
        InputSpec { name: "weight", default: 0.5, doc: "0 a child or small creature, 0.5 an adult, 1 a heavy walker: louder, lower, longer crunch" },
        InputSpec { name: "shoe", default: 0.5, doc: "0 bare feet or soft sneakers, 0.5 boots, 1 hard leather soles or heels (clicks on hard floors)" },
        InputSpec { name: "land", default: 0.0, doc: "Above 0, steps are landings: both feet at once, with the force of the fall (0.3 a hop, 1 a big drop). Set it before trigger() and back to 0 for the next step" },
    ];
    // Steps read their inputs at the instant they land; smoothing would blur a surface change.
    const INPUT_SMOOTH_SECS: f32 = 0.0;

    fn presets() -> Vec<(&'static str, FootstepParams)> {
        vec![
            // First person: the microphone at the walker's own feet.
            ("Close up", FootstepParams { brightness: 0.6, scuff: 1.4, cloth_level: 0.45, ..Default::default() }),
            // Sneaking: placed carefully, rolled slowly, nothing scuffs.
            ("Sneaking", FootstepParams { roll: 1.6, thump: 0.6, scuff: 0.2, crunch: 0.7, cloth_level: 0.1, gain: 0.6, ..Default::default() }),
            // A big heavy walker: darker, more weight, longer roll.
            ("Heavy", FootstepParams { brightness: 0.35, roll: 1.3, thump: 1.7, crunch: 1.3, ring: 1.3, cloth_level: 0.2, ..Default::default() }),
            // Nylon jacket and trousers: a bright swish with every step.
            ("Rain jacket", FootstepParams { cloth_level: 0.7, cloth_hz: 2600.0, ..Default::default() }),
            // Crowds and far-off walkers: duller and evener, so many of them do not add up to hiss.
            ("Background", FootstepParams { brightness: 0.3, variation: 0.35, scuff: 0.5, cloth_level: 0.0, gain: 0.7, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        let id = INSTANCES.fetch_add(1, Ordering::Relaxed);
        let seed = mix_seed(0xF007_0000 ^ id.wrapping_mul(0x9E37_79B9));
        let mut rng = Rng::new(mix_seed(seed ^ 0x51));
        // Nobody walks perfectly evenly: one foot lands a little harder, lower, slower to roll.
        let feet = [(1.0, 1.0, 1.0), (rng.range(0.8, 0.95), rng.range(0.93, 1.02), rng.range(1.0, 1.12))];
        Footstep {
            sr,
            noise: Noise::new(mix_seed(seed ^ 0x52)),
            rng,
            steps: [idle(); VOICES],
            foot: 0,
            feet,
            phase: 0.0,
            stride: 1.0,
            flutter: SlowNoise::new(seed ^ 0x53),
        }
    }

    fn trigger(&mut self, x: &[f32], p: &FootstepParams) {
        self.fire(x, p, 0.0);
    }

    fn block(&mut self, x: &[f32], p: &FootstepParams, out: &mut [f32]) {
        out.iter_mut().for_each(|s| *s = 0.0);
        let block_secs = out.len() as f32 / self.sr;
        let pace = x[0];
        if pace > 0.0 {
            let rate = pace * 4.0 / self.stride;
            self.phase += rate * block_secs;
            if self.phase >= 1.0 {
                // Land at the right sample inside this block.
                let late = ((self.phase - 1.0) / rate).min(block_secs);
                self.phase = (self.phase - 1.0).min(0.99);
                let v = p.variation * 2.0;
                // Even strides, but not a metronome: about 2 % jitter, and the other foot's
                // stride a little different.
                self.stride = (1.0 + v * 0.02 * self.rng.next_bipolar()) * if self.foot == 1 { 1.0 + v * 0.015 } else { 1.0 };
                self.fire(x, p, block_secs - late);
            }
        }
        let flutter = 1.0 + 0.7 * self.flutter.advance(30.0, block_secs);
        for k in 0..VOICES {
            if self.steps[k].on {
                self.render_step(k, p, flutter, out);
            }
        }
    }
}
