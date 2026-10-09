//! Water interaction, for adventure, open-world and sailing games: things falling into water
//! (`splash`), a swimmer (`swim`), a small boat (`boat`), the world under the surface
//! (`underwater`), and the small water of puzzles: drips and leaks (`drip`) and a vessel being
//! filled (`pour`).
//!
//! **Bubbles are the voice of water.** Almost every sound here is made of air bubbles ringing
//! at their Minnaert pitch ([`crate::dsp::Bubbles`]: 3.26 / radius, so a 3 mm bubble sings
//! near 1.1 kHz) plus the splash noise of water hitting water. Recordings (CC0, Freesound;
//! listed in `tests/water.rs`) were measured before anything was built, and show:
//!
//! * A stone plops in about 30 ms: a bright entry crack (the first 30 ms centre near 3-8 kHz),
//!   then the cavity behind it pinches off into one big bubble that rings low (190-280 Hz for a
//!   fist-sized stone) for 40-70 ms. What follows is 20-35 dB down: a few short pings
//!   (median 11 ms, 200 Hz - 2 kHz, nearly steady pitch) and droplets falling back.
//! * A body or a big object is long and broad: the level falls 20 dB only after 1.4-1.6 s, the
//!   octave bands from 125 Hz to 4 kHz lie within 2-3 dB of each other, the onset is a low
//!   whump (centroid 230-280 Hz) and the tail is spray (centroid 2-3 kHz), with surges as the
//!   crown and the sheet of water fall back.
//! * A drip into standing water is a click, then a 1.1-1.7 kHz ping of 20-50 ms that rises a
//!   little (about +0.1 octave), and often a second, smaller one 150 ms later (the drop thrown
//!   up by the rebound jet).
//! * Filling a bottle, the air column above the water rings 15-30 dB over the splash noise; it
//!   rose from 430 Hz to 1.9 kHz as a 25 cm bottle filled, slowly at first and fast at the end
//!   (a quarter-wave tube getting shorter), with its third harmonic near 3x.
//! * Under water the world is dark: centroid 110-240 Hz, 15-40 dB down at 1 kHz, nothing above
//!   2 kHz, apart from bubbles and clicks close by.
//! * A boat's hull slaps are low (event centroid 450-650 Hz, most energy at 125-250 Hz), come in
//!   clusters of two or three per wave, and a wake is a swelling rush centred near 1.3 kHz.
//! * Swimming strokes are 100-300 ms splashes with a sharp hand entry, 1-1.6 per second.
//!
//! **Dunking other sounds.** A generator here cannot process another generator's output: models
//! have no audio input. [`Muffle`] is a host-side filter (like [`crate::resample::Resampler`])
//! that a Rust host can run any buffer through, and [`muffle_hz`] gives the cutoff for a game
//! engine's own low-pass (a Godot bus `AudioEffectLowPassFilter`), so the whole mix goes under
//! with the `underwater` ambience.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::blocks::{hz_coef, mix_seed, Brown, Dust, OnePole, Reverb, SlowNoise, BLOCK};
use crate::dsp::{minnaert_hz, Air, Bubbles, ModalBank};
use crate::filter::{FilterMode, Svf};
use crate::math::{db_to_gain, Rng};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::params::{exp, int, lin, GAIN, UNIT};

static INSTANCES: AtomicU32 = AtomicU32::new(0);

// Output levels, set by measuring renders (LUFS, mono) against `stream` and `ocean` (about
// -16.5 LUFS at their defaults) and against the recordings' peak-to-loudness ratios:
// * underwater, a full-screen bed like the ocean: -17 LUFS;
// * swimming, hull slaps and oars are peaky (recordings: peaks 25-30 dB over their loudness),
//   so they are set by their peaks, near -3 to -5 dBFS: about -29 LUFS for a crawl, -26 for a
//   boat at anchor in a chop, -20 for a fast wake;
// * a bottle filling: -22 LUFS; a dripping tap's drops peak near -5 dBFS.
const SWIM_LEVEL: f32 = 3.0;
const BOAT_LEVEL: f32 = 5.6;
const UNDERWATER_LEVEL: f32 = 1.9;
const DRIP_LEVEL: f32 = 1.8;
const POUR_LEVEL: f32 = 1.5;

/// A different seed for every generator instance, so two swimmers or two leaks never play the
/// same pattern.
fn instance_seed(base: u32) -> u32 {
    let id = INSTANCES.fetch_add(1, Ordering::Relaxed);
    mix_seed(base ^ id.wrapping_mul(0x9E37_79B9))
}

#[inline]
fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let u = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// A noise envelope: a soft attack into an exponential decay, retriggers add. Cheap enough to
/// run several per voice (no transcendental functions per sample).
#[derive(Clone, Copy, Debug, Default)]
struct Burst {
    env: f32,
    target: f32,
    ka: f32,
    kd: f32,
}

impl Burst {
    fn hit(&mut self, amp: f32, attack: f32, t60: f32, sr: f32) {
        self.target += amp;
        self.ka = 1.0 - (-1.0 / (attack.max(2e-4) * sr)).exp();
        self.kd = (-6.91 / (t60.max(1e-3) * sr)).exp();
    }

    #[inline]
    fn tick(&mut self) -> f32 {
        self.env += (self.target - self.env) * self.ka;
        self.target *= self.kd;
        self.env
    }

    fn idle(&self) -> bool {
        self.env < 2e-5 && self.target < 2e-5
    }

    fn quiet(&mut self) {
        if self.idle() {
            self.env = 0.0;
            self.target = 0.0;
        }
    }
}

/// A scheduled event: fires `kind` with `amp` after `wait` seconds.
#[derive(Clone, Copy, Debug, Default)]
struct Due {
    wait: f32,
    kind: u8,
    amp: f32,
    on: bool,
}

const QUEUE: usize = 16;

#[derive(Clone, Copy, Debug)]
struct Queue {
    q: [Due; QUEUE],
}

impl Queue {
    fn new() -> Self {
        Queue { q: [Due::default(); QUEUE] }
    }

    fn push(&mut self, wait: f32, kind: u8, amp: f32) {
        if let Some(d) = self.q.iter_mut().find(|d| !d.on) {
            *d = Due { wait: wait.max(0.0), kind, amp, on: true };
        }
    }

    /// Move time on by `dt` and return the next event that is due (call until `None`).
    fn pop(&mut self, dt: f32) -> Option<(u8, f32)> {
        for d in self.q.iter_mut().filter(|d| d.on) {
            if d.wait <= dt {
                d.on = false;
                return Some((d.kind, d.amp));
            }
        }
        None
    }

    fn advance(&mut self, dt: f32) {
        for d in self.q.iter_mut().filter(|d| d.on) {
            d.wait -= dt;
        }
    }

    fn pending(&self) -> bool {
        self.q.iter().any(|d| d.on)
    }

    fn clear(&mut self) {
        self.q = [Due::default(); QUEUE];
    }
}

/// A swarm of bubbles being born at `rate` per second (set per block), with radii spread
/// log-evenly between `r_lo` and `r_hi` mm and skewed towards small ones (`skew`), and power-law
/// loudness. Feeds a [`Bubbles`] pool.
#[derive(Clone, Copy, Debug)]
struct Swarm {
    rng: Rng,
    rate: f32,
    r_lo: f32,
    r_hi: f32,
    skew: f32,
    amp: f32,
    rise: f32,
    damping: f32,
}

impl Swarm {
    fn new(seed: u32) -> Self {
        Swarm { rng: Rng::new(mix_seed(seed)), rate: 0.0, r_lo: 1.0, r_hi: 4.0, skew: 1.5, amp: 0.0, rise: 0.03, damping: 1.0 }
    }

    /// Per sample: maybe spawn a bubble into `pool`.
    #[inline]
    fn tick(&mut self, pool: &mut Bubbles, sr: f32) {
        if self.rate <= 0.0 || self.rng.next_f32() >= self.rate / sr {
            return;
        }
        let u = self.rng.next_f32().powf(self.skew);
        let r = self.r_lo * (self.r_hi / self.r_lo).powf(u);
        // Bigger bubbles are louder; on top of that most are faint and a few stand out.
        let a = self.amp * (r / self.r_hi).sqrt() * (0.15 + 0.85 * self.rng.next_f32().powi(2));
        pool.spawn(minnaert_hz(r), a, self.rise * self.rng.range(0.5, 1.5), self.damping, sr);
    }
}

/// Distance as for the other events: level and an air low-pass from 18 kHz down to 400 Hz.
fn far(air: &mut Air, distance: f32, sr: f32) {
    air.set(distance, 18000.0, 400.0, 1.0 - 0.75 * distance, sr);
}

// ---------------------------------------------------------------------------------------------
// Muffle: dunking any sound
// ---------------------------------------------------------------------------------------------

/// How deep the listener's head is under water, 0..1, as the share of "under" (0 below 0.35,
/// 1 above 0.65, smooth between), as every generator here reads its `submerge` input.
#[inline]
pub fn submerged(submerge: f32) -> f32 {
    smoothstep(0.35, 0.65, submerge)
}

/// The low-pass cutoff that puts a sound under water at `submerge` (0..1): 18 kHz above the
/// surface down to 450 Hz under it, evenly in octaves. Use it with two cascaded 12 dB/oct
/// low-passes (as [`Muffle`] does), or a Godot bus `AudioEffectLowPassFilter` at 24 dB/oct.
#[inline]
pub fn muffle_hz(submerge: f32) -> f32 {
    18000.0 * (450.0f32 / 18000.0).powf(submerged(submerge))
}

/// The level change of a sound heard through the surface: 0 dB above it, -10 dB under it.
#[inline]
pub fn muffle_db(submerge: f32) -> f32 {
    -10.0 * submerged(submerge)
}

/// Puts any mono buffer under water: two 12 dB/oct low-passes at [`muffle_hz`] and the level
/// of [`muffle_db`]. A host-side helper (no model has an audio input): run a generator's output
/// through it, block by block, with the same `submerge` the `underwater` ambience gets.
/// Changing `submerge` moves the cutoff once per call, so call it with blocks of ~512 samples
/// or less for smooth dives.
#[derive(Clone, Copy, Debug)]
pub struct Muffle {
    f: [Svf; 2],
    gain: f32,
    sr: f32,
}

impl Muffle {
    pub fn new(sample_rate: f32) -> Self {
        Muffle { f: [Svf::default(); 2], gain: 1.0, sr: sample_rate }
    }

    pub fn process(&mut self, buf: &mut [f32], submerge: f32) {
        let hz = muffle_hz(submerge);
        let target = db_to_gain(muffle_db(submerge));
        for f in self.f.iter_mut() {
            f.set(FilterMode::LowPass, hz, 0.0, self.sr);
        }
        let n = buf.len().max(1) as f32;
        let step = (target - self.gain) / n;
        for s in buf.iter_mut() {
            self.gain += step;
            let y = self.f[0].tick(*s);
            *s = self.f[1].tick(y) * self.gain;
        }
        self.gain = target;
    }
}

// ---------------------------------------------------------------------------------------------
// splash
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Something entering water: an entry crack, the cavity's big bubble, the rush of water
    /// thrown up and falling back, a cloud of small bubbles and droplets.
    SplashParams / SplashParamId {
        crack: "crack/level" = 1.0, GAIN;
        bloop: "bloop/level" = 1.0, GAIN;
        body: "body/level" = 1.0, GAIN;
        bubbles: "bubbles/level" = 1.0, GAIN;
        droplets: "droplets/level" = 1.0, GAIN;
        brightness: "shape/brightness" = 0.5, UNIT;
        pitch: "shape/pitch_semitones" = 0.0, lin(-12.0, 12.0);
        variation: "shape/variation" = 0.5, UNIT;
        room: "space/amount" = 0.0, UNIT;
        room_time: "space/tail" = 1.6, lin(0.3, 4.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// The pitch of the cavity's bubble for an object of `size` (0..1) entering at `power`: a
/// pebble's plip near 1.2 kHz, a thrown stone's plop near 280 Hz (190-280 Hz measured), about
/// 160 Hz for a person and 100 Hz for something huge. Faster entries go deeper, a little lower.
pub fn bloop_hz(size: f32, power: f32) -> f32 {
    const AT: [(f32, f32); 5] = [(0.0, 1500.0), (0.2, 600.0), (0.35, 380.0), (0.6, 210.0), (1.0, 130.0)];
    let s = size.clamp(0.0, 1.0);
    let k = AT.iter().rposition(|a| a.0 <= s).unwrap_or(0).min(AT.len() - 2);
    let ((s0, f0), (s1, f1)) = (AT[k], AT[k + 1]);
    f0 * (f1 / f0).powf((s - s0) / (s1 - s0)) * (-0.4 * power.clamp(0.0, 1.0)).exp2()
}

/// The radius range (mm) of the bubbles a splash of `size` leaves behind: 0.8 mm up to 3 mm for
/// a pebble and 15 mm for something huge.
pub fn splash_bubble_radii(size: f32) -> (f32, f32) {
    (0.8, 3.0 + 12.0 * size.clamp(0.0, 1.0))
}

const S_BLOOP: u8 = 0;
const S_SURGE: u8 = 1;
const S_JET: u8 = 2;

pub struct Splash {
    sr: f32,
    rng: Rng,
    noise: Noise,
    queue: Queue,
    crack: Burst,
    crack_lp: Svf,
    crack_hp: OnePole,
    crack_hp_coef: f32,
    slap: Burst,
    slap_lp: Svf,
    whump: Burst,
    whump_lp: Svf,
    whump_hp: Svf,
    rush: Burst,
    rush_lp: Svf,
    rush_hp: Svf,
    tick: Burst,
    tick_hp: OnePole,
    bloops: Bubbles,
    pool: Bubbles,
    cloud: Swarm,
    cloud_decay: f32,
    drops: Swarm,
    drop_dust: Dust,
    drop_rate: f32,
    /// Droplets fall from `drop_at` for `drop_len` seconds; `t` is the time since the trigger.
    drop_at: f32,
    drop_len: f32,
    t: f32,
    // Rolled at the trigger.
    size: f32,
    power: f32,
    rush_t60: f32,
    far: Air,
    reverb: Box<Reverb>,
    room: f32,
    active: bool,
    pitch_ratio: f32,
}

impl Splash {
    fn surge(&mut self, amp: f32) {
        let (sr, s) = (self.sr, self.size);
        self.rush.hit(amp, 0.02, self.rush_t60 * 0.6, sr);
        self.whump.hit(amp * 0.5, 0.012, 0.15 + 0.3 * s, sr);
        self.cloud.rate += amp * (40.0 + 300.0 * s);
    }
}

impl Generator for Splash {
    type P = SplashParams;
    const NAME: &'static str = "splash";
    const CATEGORY: &'static str = "water";
    const DOC: &'static str = "Something entering water: a pebble's plip, a stone's plop, a dive, a belly flop, a big object. \
        An entry crack, the deep bubble of the cavity, the rush of water thrown up and falling back, small bubbles and droplets.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "power", default: 1.0, doc: "Entry speed: dropped gently .. thrown or jumped from high up" },
        InputSpec { name: "distance", default: 0.0, doc: "0 = at the listener, 1 = far off (duller, quieter)" },
        InputSpec { name: "size", default: 0.35, doc: "0 a pebble, 0.35 a stone, 0.6 a person, 1 a huge object: lower, longer, more water thrown up" },
        InputSpec { name: "flat", default: 0.2, doc: "How it meets the water: 0 clean (a dive, a pointed stone: a deep bloop) .. 1 flat (a belly flop: a slap)" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const ONE_SHOT: bool = true;
    const TRANSPOSES: bool = true;

    fn presets() -> Vec<(&'static str, SplashParams)> {
        vec![
            // Inputs set what fell in; presets set where it is heard.
            ("Bright", SplashParams { brightness: 0.8, crack: 1.3, droplets: 1.3, ..Default::default() }),
            ("Deep", SplashParams { brightness: 0.3, bloop: 1.4, crack: 0.7, pitch: -4.0, ..Default::default() }),
            ("Indoor pool", SplashParams { room: 0.45, room_time: 2.2, brightness: 0.6, ..Default::default() }),
            ("Cartoon", SplashParams { bloop: 1.8, bubbles: 1.5, body: 0.6, pitch: 3.0, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        let seed = instance_seed(0x5B1A_5400);
        Splash {
            sr,
            rng: Rng::new(mix_seed(seed ^ 1)),
            noise: Noise::new(mix_seed(seed ^ 2)),
            queue: Queue::new(),
            crack: Burst::default(),
            crack_lp: Svf::default(),
            crack_hp: OnePole::default(),
            crack_hp_coef: 0.0,
            slap: Burst::default(),
            slap_lp: Svf::default(),
            whump: Burst::default(),
            whump_lp: Svf::default(),
            whump_hp: Svf::default(),
            rush: Burst::default(),
            rush_lp: Svf::default(),
            rush_hp: Svf::default(),
            tick: Burst::default(),
            tick_hp: OnePole::default(),
            bloops: Bubbles::new(),
            pool: Bubbles::new(),
            cloud: Swarm::new(seed ^ 3),
            cloud_decay: 1.0,
            drops: Swarm::new(seed ^ 4),
            drop_dust: Dust::new(seed ^ 5),
            drop_rate: 0.0,
            drop_at: 0.0,
            drop_len: 0.0,
            t: 0.0,
            size: 0.0,
            power: 0.0,
            rush_t60: 1.0,
            far: Air::default(),
            reverb: Box::new(Reverb::new(sr)),
            room: 0.0,
            active: false,
            pitch_ratio: 1.0,
        }
    }

    fn trigger(&mut self, x: &[f32], p: &SplashParams) {
        let (power, distance, size, flat) = (x[0], x[1], x[2], x[3]);
        let sr = self.sr;
        let v = p.variation.clamp(0.0, 1.0);
        let roll = |rng: &mut Rng, amount: f32| 1.0 + v * amount * rng.next_bipolar();
        let s = (size + v * 0.04 * self.rng.next_bipolar()).clamp(0.0, 1.0);
        let pw = power.max(0.02);
        self.size = s;
        self.power = pw;
        let tune = (p.pitch / 12.0).exp2() * self.pitch_ratio;
        self.bloops.pitch = tune;
        self.pool.pitch = tune;
        let bright = (2.0 * p.brightness - 1.0).exp2();
        if !self.active {
            self.queue.clear();
            self.bloops.clear();
            self.pool.clear();
            self.far.reset();
            self.reverb.clear();
        }
        self.t = 0.0;
        // How much water it throws up: nothing for a pebble, everything for a big object.
        let mass = smoothstep(0.12, 0.75, s);
        // Set against `mud_splash` and `rock_hit` (see `tests/water.rs`): a full-power big splash
        // is about as loud as `mud_splash`, and every size peaks near -2 to -4 dBFS. Small
        // things are quieter, but not as much quieter as their energy would make them.
        let level = pw.powf(0.8) * 1.4 * (1.0 + 1.6 * (1.0 - s) * (1.0 - s));

        // The entry crack: short and bright for a pebble, a longer, deeper slap when flat.
        let crack_t60 = 0.006 + 0.03 * s * (0.3 + flat);
        let crack = p.crack * 0.9 * level * (0.45 + 0.55 * flat) * (1.6 - 1.2 * s);
        self.crack.hit(crack * roll(&mut self.rng, 0.25), 0.0004, crack_t60, sr);
        let crack_hz = (9000.0 * (-2.0 * s).exp2() * bright * (0.6 + 0.4 * pw) * tune).min(0.45 * sr);
        self.crack_lp.set(FilterMode::LowPass, crack_hz, 0.1, sr);
        self.crack_hp_coef = hz_coef(250.0 * tune, sr);
        // A belly flop's slap: the body of the crack, low and loud.
        self.slap.hit(p.crack * 1.4 * level * flat * flat * mass, 0.0015, 0.05 + 0.15 * s, sr);
        self.slap_lp.set(FilterMode::LowPass, 700.0 * (-s).exp2() * tune, 0.2, sr);

        // The cavity pinches off into one big bubble (a body makes a few): the plop.
        let bloops = 1 + (s * 2.5 + 0.5 * self.rng.next_f32()) as usize;
        // A pebble barely opens a cavity: its plip is small next to its crack.
        let clean = (1.0 - 0.6 * flat) * (0.35 + 0.65 * smoothstep(0.0, 0.3, s));
        for k in 0..bloops {
            let wait = 0.012 + 0.08 * s + k as f32 * self.rng.range(0.03, 0.12) * (0.5 + s) + v * 0.02 * self.rng.next_f32();
            let a = p.bloop * clean * level * (1.0 - 0.55 * mass) * if k == 0 { 1.0 } else { self.rng.range(0.25, 0.6) };
            self.queue.push(wait, S_BLOOP, a * roll(&mut self.rng, 0.2));
        }

        // The water thrown up: a low whump, then the rush of the crown and sheet, which falls
        // back in surges.
        self.rush_t60 = (0.3 + 3.2 * s.powf(1.1)) * (0.7 + 0.3 * pw) * roll(&mut self.rng, 0.2);
        let body = p.body * level * mass;
        self.whump.hit(body * 1.2 * (0.5 + 0.9 * s), 0.012, 0.2 + 0.5 * s, sr);
        self.whump_lp.set(FilterMode::LowPass, 420.0 * (-1.2 * s).exp2() * tune, 0.3, sr);
        self.whump_hp.set(FilterMode::HighPass, 60.0 * tune, 0.1, sr);
        // Even a stone throws up a little crown of water.
        let crown = p.body * level * (0.05 + 0.95 * mass);
        // A big mass of water takes a moment to rise, so its rush peaks late.
        self.rush.hit(crown * 0.55, 0.015 + 0.1 * s * s, (self.rush_t60 * (0.3 + 0.7 * mass)).max(0.15), sr);
        self.rush_lp.set(FilterMode::LowPass, (11000.0 * bright * tune).min(0.45 * sr), 0.05, sr);
        self.rush_hp.set(FilterMode::HighPass, 220.0 * (-1.3 * s).exp2() * tune, 0.05, sr);
        // The crown collapses, the sheet and then the column fall back: surges spread over the
        // first second of a big splash (the recordings stay within 7 dB of their peak for
        // about 0.6 s).
        let surges = (s * 4.5 * pw) as usize;
        for k in 0..surges {
            let wait = (k as f32 + self.rng.range(0.3, 1.0)) / surges as f32 * 0.9 * (0.4 + s);
            self.queue.push(wait, S_SURGE, body * self.rng.range(0.25, 0.7) * 0.55);
        }

        // Small bubbles torn off as it goes in, thinning out.
        let (r_lo, r_hi) = splash_bubble_radii(s);
        self.cloud.r_lo = r_lo;
        self.cloud.r_hi = r_hi;
        self.cloud.amp = p.bubbles * 0.08 * level;
        self.cloud.rate = (25.0 + 260.0 * s) * pw;
        self.cloud_decay = (-(BLOCK as f32) / ((0.08 + 0.5 * s) * sr)).exp();
        // Droplets fall back once the thrown water comes down.
        self.drop_at = (0.06 + 0.35 * s * pw.sqrt()) * roll(&mut self.rng, 0.2);
        self.drop_len = 0.25 + 1.1 * s;
        self.drop_rate = p.droplets * (15.0 + 150.0 * s * mass) * pw;
        self.drops.r_lo = 1.0;
        self.drops.r_hi = 3.5;
        self.drops.amp = p.droplets * 0.035 * level;
        self.drops.rate = 0.0;
        // A small object throws a jet back up, whose drop falls back a moment later.
        if s < 0.45 && self.rng.next_f32() < 0.8 {
            self.queue.push(self.rng.range(0.12, 0.3) * (0.7 + s), S_JET, p.droplets * level * self.rng.range(0.3, 0.6));
        }
        self.room = p.room;
        far(&mut self.far, distance, sr);
        self.active = true;
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn length(p: &SplashParams) -> Option<f32> {
        // The rush of the biggest splash with surges, until it is 90 dB down, plus the room.
        let rush = (0.3 + 3.2) * (1.0 + 0.2 * p.variation);
        Some(0.6 * 1.5 + rush * 1.35 + p.room * p.room_time * 1.6 + 0.35)
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        self.pitch_ratio = ratio;
    }

    fn block(&mut self, _x: &[f32], p: &SplashParams, out: &mut [f32]) {
        let sr = self.sr;
        let dt = out.len() as f32 / sr;
        while let Some((kind, amp)) = self.queue.pop(dt) {
            match kind {
                S_BLOOP => {
                    let hz = bloop_hz(self.size, self.power) * self.rng.range(0.85, 1.15);
                    // Big cavities ring a little longer than a free bubble.
                    self.bloops.spawn(hz, amp * 0.4, 0.04, 0.7, sr);
                    self.crack.hit(amp * 0.08, 0.001, 0.008, sr);
                }
                S_SURGE => self.surge(amp),
                _ => {
                    // The jet's drop: a tick and a high plip.
                    self.tick.hit(amp * 0.25, 0.0003, 0.004, sr);
                    let hz = minnaert_hz(self.rng.range(1.6, 3.2));
                    self.pool.spawn(hz, amp * 0.18, 0.04, 0.6, sr);
                }
            }
        }
        self.queue.advance(dt);
        self.cloud.rate *= self.cloud_decay;
        if self.cloud.rate < 0.5 {
            self.cloud.rate = 0.0;
        }
        let dropping = if self.t >= self.drop_at && self.t < self.drop_at + self.drop_len {
            let u = (self.t - self.drop_at) / self.drop_len;
            (core::f32::consts::PI * u.sqrt()).sin()
        } else {
            0.0
        };
        let drop_p = self.drop_rate * dropping / sr;
        self.drops.rate = 0.4 * self.drop_rate * dropping;
        self.t += dt;
        let tick_coef = hz_coef(1500.0 * self.pitch_ratio, sr);
        let (room, rt) = (self.room, p.room_time);
        for o in out.iter_mut() {
            let w = self.noise.white();
            let pink = self.noise.pink();
            self.cloud.tick(&mut self.pool, sr);
            self.drops.tick(&mut self.pool, sr);
            let d = self.drop_dust.tick(drop_p);
            if d > 0.0 {
                let a = d * d * d * 0.06 * self.drops.amp / 0.035;
                self.tick.hit(a, 0.0003, 0.004, sr);
            }
            let crack = self.crack_hp.hp(self.crack_lp.tick(w), self.crack_hp_coef) * self.crack.tick();
            let slap = self.slap_lp.tick(w) * self.slap.tick();
            let whump = self.whump_lp.tick(pink);
            let whump = self.whump_hp.tick(whump) * self.whump.tick();
            let rush = self.rush_hp.tick(self.rush_lp.tick(pink)) * self.rush.tick() * 2.2;
            let tick = self.tick_hp.hp(w, tick_coef) * self.tick.tick();
            let bubbles = self.bloops.tick() + self.pool.tick();
            let mut y = crack + slap * 1.6 + whump * 1.8 + rush + tick + bubbles;
            if room > 0.0 {
                y += self.reverb.tick(y, rt, 0.5) * room * 1.5;
            }
            *o = self.far.tick(y) * p.gain;
        }
        self.bloops.end_block(1e-5);
        self.pool.end_block(1e-5);
        for b in [&mut self.crack, &mut self.slap, &mut self.whump, &mut self.rush, &mut self.tick] {
            b.quiet();
        }
        let busy = [&self.crack, &self.slap, &self.whump, &self.rush, &self.tick].iter().any(|b| !b.idle());
        let tail = room > 0.0 && self.t < self.drop_at + self.drop_len + rt * 1.6;
        if !busy && !self.queue.pending() && self.bloops.active() == 0 && self.pool.active() == 0 && self.t > self.drop_at + self.drop_len && !tail {
            self.active = false;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// swim
// ---------------------------------------------------------------------------------------------

model_params! {
    /// One swimmer. `stroke/style`: 0 crawl (hand entries, a flutter kick), 1 breaststroke (both
    /// arms sweep, a frog kick, a long glide), 2 dog paddle (quick small paddles).
    SwimParams / SwimParamId {
        style: "stroke/style" = 0.0, int(0, 2);
        entry: "stroke/entry" = 1.0, GAIN;
        pull: "stroke/pull" = 1.0, GAIN;
        drips: "stroke/drips" = 1.0, GAIN;
        kick: "kick/level" = 0.7, GAIN;
        kick_rate: "kick/per_stroke" = 3.0, lin(1.0, 6.0);
        tread: "tread/level" = 1.0, GAIN;
        variation: "stroke/variation" = 0.5, UNIT;
        room: "space/amount" = 0.0, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

const W_ENTRY: u8 = 0;
const W_PULL: u8 = 1;
const W_EXIT: u8 = 2;
const W_KICK: u8 = 3;
const W_SCULL: u8 = 4;

pub struct Swim {
    sr: f32,
    rng: Rng,
    noise: Noise,
    queue: Queue,
    phase: f32,
    kick_phase: f32,
    scull_phase: f32,
    period: f32,
    entry: Burst,
    entry_lp: Svf,
    entry_hp: OnePole,
    splash: Burst,
    splash_bp: Svf,
    pull: Burst,
    pull_lp: Svf,
    kick: Burst,
    kick_bp: Svf,
    tick: Burst,
    tick_hp: OnePole,
    drop_dust: Dust,
    drop_rate: f32,
    pool: Bubbles,
    swarm: Swarm,
    gurgle: Swarm,
    swell: SlowNoise,
    reverb: Box<Reverb>,
}

impl Swim {
    /// One stroke, with this effort: the hand goes in, pulls, comes out and drips.
    fn stroke(&mut self, effort: f32, style: usize, period: f32, p: &SwimParams) {
        let v = p.variation;
        let a = (0.35 + 0.65 * effort) * (1.0 + 0.25 * v * self.rng.next_bipolar());
        match style {
            // Breaststroke: no hand entry; a big sweep, the kick after it, a quiet recovery.
            1 => {
                self.queue.push(0.0, W_PULL, a * 2.2);
                self.queue.push(0.35 * period, W_KICK, a * 2.0);
                self.queue.push(0.55 * period, W_EXIT, a * 0.5);
            }
            // Dog paddle: small quick slaps and pulls.
            2 => {
                self.queue.push(0.0, W_ENTRY, a * 0.5);
                self.queue.push(0.02, W_PULL, a * 0.6);
                self.queue.push(0.45 * period, W_EXIT, a * 0.35);
            }
            _ => {
                self.queue.push(0.0, W_ENTRY, a);
                self.queue.push(0.03, W_PULL, a);
                self.queue.push(0.6 * period.min(1.2), W_EXIT, a * 0.8);
            }
        }
    }

    fn fire(&mut self, kind: u8, a: f32, p: &SwimParams) {
        let sr = self.sr;
        match kind {
            W_ENTRY => {
                self.entry.hit(p.entry * a * 0.5, 0.0005, 0.02 + 0.02 * a, sr);
                self.splash.hit(p.entry * a * 0.45, 0.006, 0.18, sr);
                self.swarm.rate += 220.0 * a;
            }
            W_PULL => {
                self.pull.hit(p.pull * a * 0.5, 0.12, 0.55, sr);
                self.gurgle.rate += 50.0 * a;
            }
            W_EXIT => {
                self.splash.hit(p.entry * a * 0.3, 0.02, 0.22, sr);
                self.drop_rate += 45.0 * a * p.drips;
            }
            W_KICK => {
                self.kick.hit(p.kick * a * 0.35, 0.01, 0.09, sr);
                self.swarm.rate += 25.0 * a;
            }
            _ => {
                // Treading: a slow scull and the water lapping around the shoulders.
                self.pull.hit(p.tread * a * 1.1, 0.18, 0.7, sr);
                self.splash.hit(p.tread * a * 0.35, 0.05, 0.25, sr);
                self.gurgle.rate += 20.0 * a;
                self.drop_rate += 6.0 * a;
            }
        }
    }
}

impl Generator for Swim {
    type P = SwimParams;
    const NAME: &'static str = "swim";
    const CATEGORY: &'static str = "water";
    const DOC: &'static str = "A swimmer: hand entries, the pull, drips off the arm and the kick (crawl, breaststroke or dog paddle), \
        or treading water. Strokes by themselves at pace, or trigger() one per stroke from the animation.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "pace", default: 0.0, doc: "0: strokes only on trigger(). Above 0 the swimmer strokes by itself, pace x 2 arm strokes per second (0.5 cruising, 1 sprinting)" },
        InputSpec { name: "effort", default: 0.5, doc: "0 gentle and quiet .. 1 thrashing: louder entries, more spray, a harder kick" },
        InputSpec { name: "tread", default: 0.0, doc: "0 swimming .. 1 treading water in place: slow sculls and lapping instead of strokes" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.1;

    fn presets() -> Vec<(&'static str, SwimParams)> {
        vec![
            ("Breaststroke", SwimParams { style: 1.0, kick: 1.0, ..Default::default() }),
            ("Dog paddle", SwimParams { style: 2.0, kick: 0.9, kick_rate: 2.0, drips: 0.6, ..Default::default() }),
            ("Indoor pool", SwimParams { room: 0.4, ..Default::default() }),
            ("Lake, far", SwimParams { entry: 0.6, drips: 0.5, kick: 0.4, gain: 0.6, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        let seed = instance_seed(0x5717_0000);
        Swim {
            sr,
            rng: Rng::new(mix_seed(seed ^ 1)),
            noise: Noise::new(mix_seed(seed ^ 2)),
            queue: Queue::new(),
            phase: 0.0,
            kick_phase: 0.0,
            scull_phase: 0.0,
            period: 1.0,
            entry: Burst::default(),
            entry_lp: Svf::default(),
            entry_hp: OnePole::default(),
            splash: Burst::default(),
            splash_bp: Svf::default(),
            pull: Burst::default(),
            pull_lp: Svf::default(),
            kick: Burst::default(),
            kick_bp: Svf::default(),
            tick: Burst::default(),
            tick_hp: OnePole::default(),
            drop_dust: Dust::new(seed ^ 3),
            drop_rate: 0.0,
            pool: Bubbles::new(),
            swarm: Swarm { r_lo: 0.9, r_hi: 4.0, amp: 0.05, ..Swarm::new(seed ^ 4) },
            gurgle: Swarm { r_lo: 3.0, r_hi: 9.0, amp: 0.05, skew: 1.0, ..Swarm::new(seed ^ 5) },
            swell: SlowNoise::new(seed ^ 6),
            reverb: Box::new(Reverb::new(sr)),
        }
    }

    fn trigger(&mut self, x: &[f32], p: &SwimParams) {
        let style = p.style.round() as usize;
        self.stroke(x[1], style, self.period, p);
    }

    fn block(&mut self, x: &[f32], p: &SwimParams, out: &mut [f32]) {
        let sr = self.sr;
        let dt = out.len() as f32 / sr;
        let (pace, effort, tread) = (x[0], x[1], x[2]);
        let style = p.style.round() as usize;
        let swim = 1.0 - tread;
        // Strokes per second: crawl counts each arm, breaststroke whole cycles.
        let rate = match style {
            1 => pace * 1.0,
            2 => pace * 4.0,
            _ => pace * 2.0,
        };
        if rate > 0.0 {
            self.period = (1.0 / rate).min(2.0);
        }
        if pace > 0.0 && swim > 0.05 {
            self.phase += rate * dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                let jitter = 1.0 + 0.06 * p.variation * self.rng.next_bipolar();
                self.phase = (self.phase * jitter).min(0.5);
                self.stroke(effort * swim, style, self.period, p);
            }
            // The crawl's flutter kick runs under the strokes; breaststroke kicks once per stroke.
            if style != 1 {
                self.kick_phase += rate * p.kick_rate * 0.5 * dt;
                if self.kick_phase >= 1.0 {
                    self.kick_phase -= 1.0;
                    let a = swim * effort.powf(1.5) * self.rng.range(0.4, 1.0);
                    self.fire(W_KICK, a, p);
                }
            }
        }
        if tread > 0.05 {
            // Sculling, about 0.8 times a second, quicker when working hard.
            self.scull_phase += (0.6 + 0.6 * effort + 0.4 * pace) * dt;
            if self.scull_phase >= 1.0 {
                self.scull_phase -= 1.0;
                let a = tread * (0.4 + 0.6 * effort) * self.rng.range(0.6, 1.0);
                self.fire(W_SCULL, a, p);
            }
        }
        while let Some((kind, a)) = self.queue.pop(dt) {
            self.fire(kind, a, p);
        }
        self.queue.advance(dt);
        let swell = 1.0 + 0.3 * self.swell.advance(0.7, dt);
        self.entry_lp.set(FilterMode::LowPass, 4500.0 + 3000.0 * effort, 0.1, sr);
        self.splash_bp.set(FilterMode::BandPass, 1300.0 * swell, 0.05, sr);
        self.pull_lp.set(FilterMode::BandPass, 520.0 * swell, 0.25, sr);
        self.kick_bp.set(FilterMode::BandPass, 900.0, 0.1, sr);
        let (hp_coef, tick_coef) = (hz_coef(350.0, sr), hz_coef(1800.0, sr));
        let k = (-(BLOCK as f32) / (0.06 * sr)).exp();
        self.swarm.rate *= k;
        self.gurgle.rate *= (-(BLOCK as f32) / (0.3 * sr)).exp();
        self.drop_rate *= (-(BLOCK as f32) / (0.25 * sr)).exp();
        self.swarm.amp = 0.04 + 0.04 * effort;
        let drop_p = self.drop_rate / sr;
        let room = p.room;
        for o in out.iter_mut() {
            let w = self.noise.white();
            let pink = self.noise.pink();
            self.swarm.tick(&mut self.pool, sr);
            self.gurgle.tick(&mut self.pool, sr);
            let d = self.drop_dust.tick(drop_p);
            if d > 0.0 {
                self.tick.hit(d * d * d * 0.12, 0.0003, 0.004, sr);
                if self.rng.next_f32() < 0.4 {
                    let hz = minnaert_hz(self.rng.range(1.2, 3.5));
                    self.pool.spawn(hz, d * d * 0.05, 0.03, 0.8, sr);
                }
            }
            let entry = self.entry_hp.hp(self.entry_lp.tick(w), hp_coef) * self.entry.tick();
            let splash = self.splash_bp.tick(pink) * self.splash.tick() * 2.2;
            let pull = self.pull_lp.tick(pink) * self.pull.tick() * 1.6;
            let kick = self.kick_bp.tick(pink) * self.kick.tick() * 2.0;
            let tick = self.tick_hp.hp(w, tick_coef) * self.tick.tick();
            let mut y = entry + splash + pull + kick + tick + self.pool.tick();
            if room > 0.0 {
                y += self.reverb.tick(y, 2.2, 0.45) * room * 1.5;
            }
            *o = y * p.gain * SWIM_LEVEL;
        }
        self.pool.end_block(1e-5);
        for b in [&mut self.entry, &mut self.splash, &mut self.pull, &mut self.kick, &mut self.tick] {
            b.quiet();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// boat
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A small boat: waves slapping the hull, the bow wash, a wooden hull creaking, oars.
    /// An outboard motor is `combustion` (preset `Dirt bike 2-stroke`, or `motor`) layered on top.
    BoatParams / BoatParamId {
        hull_hz: "hull/hz" = 170.0, exp(60.0, 600.0);
        hull_size: "hull/size" = 0.5, UNIT;
        slap: "slap/level" = 1.0, GAIN;
        slap_rate: "slap/waves_per_second" = 0.8, exp(0.1, 4.0);
        wash: "wash/level" = 1.0, GAIN;
        wash_hz: "wash/hz" = 700.0, exp(250.0, 4000.0);
        lap: "lap/level" = 0.6, GAIN;
        creak: "creak/level" = 0.5, GAIN;
        creak_hz: "creak/hz" = 420.0, exp(150.0, 1500.0);
        oars: "oars/level" = 1.0, GAIN;
        row_rate: "oars/strokes_per_second" = 0.5, lin(0.2, 1.2);
        gain: "master/gain" = 1.0, GAIN;
    }
}

const B_SLAP: u8 = 0;
const B_CATCH: u8 = 1;
const B_RELEASE: u8 = 2;
const B_CREAK: u8 = 3;

const CREAK_MODES: [(f32, f32, f32); 4] = [(1.0, 0.06, 1.0), (2.31, 0.04, 0.6), (4.12, 0.03, 0.4), (6.7, 0.02, 0.25)];
const KNOCK_MODES: [(f32, f32, f32); 3] = [(1.0, 0.09, 1.0), (2.28, 0.05, 0.5), (3.9, 0.03, 0.3)];

pub struct Boat {
    sr: f32,
    rng: Rng,
    noise: Noise,
    queue: Queue,
    wave_phase: f32,
    wave_len: f32,
    row_phase: f32,
    slap: Burst,
    slap_bp: Svf,
    slosh_lp: Svf,
    knock: ModalBank,
    knock_tuned: f32,
    knock_kick: f32,
    clunk_kick: f32,
    wash_bp: Svf,
    wash_hp: OnePole,
    lap_bp: Svf,
    swell: SlowNoise,
    pull: Burst,
    pull_lp: Svf,
    splash: Burst,
    splash_bp: Svf,
    tick: Burst,
    tick_hp: OnePole,
    drop_dust: Dust,
    drop_rate: f32,
    pool: Bubbles,
    fizz: Swarm,
    gurgle: Swarm,
    lap: Swarm,
    clunk: ModalBank,
    creak: ModalBank,
    creak_tuned: f32,
    /// Stick-slip of the creak: impulses per second gliding from `creak_from` to `creak_to`.
    creak_t: f32,
    creak_len: f32,
    creak_from: f32,
    creak_to: f32,
    creak_amp: f32,
    creak_acc: f32,
}

impl Boat {
    fn fire(&mut self, kind: u8, a: f32, p: &BoatParams) {
        let sr = self.sr;
        let big = p.hull_size;
        match kind {
            B_SLAP => {
                self.slap.hit(a, 0.0015, 0.1 + 0.25 * big, sr);
                let hz = p.hull_hz * self.rng.range(0.85, 1.18);
                self.slap_bp.set(FilterMode::BandPass, hz, 0.55, sr);
                for (k, m) in KNOCK_MODES.iter().enumerate() {
                    self.knock.set_gain(k, m.2 * self.rng.range(0.5, 1.0));
                    self.knock.set_live(k, true);
                }
                self.knock_kick += a * 0.04;
                self.gurgle.rate += 25.0 * a;
            }
            B_CATCH => {
                // The blade goes in: a plip, a little splash, the oar knocks in its lock.
                let hz = minnaert_hz(self.rng.range(5.0, 9.0));
                self.pool.spawn(hz, a * 0.12, 0.03, 0.8, sr);
                self.splash.hit(a * 0.18, 0.004, 0.12, sr);
                self.pull.hit(a * 0.55, 0.12, 0.7, sr);
                self.gurgle.rate += 60.0 * a;
                self.clunk_kick += a * 0.03;
                for k in 0..KNOCK_MODES.len() {
                    self.clunk.set_live(k, true);
                }
            }
            B_RELEASE => {
                self.splash.hit(a * 0.3, 0.015, 0.2, sr);
                self.drop_rate += 50.0 * a;
                self.clunk_kick += a * 0.02;
                for k in 0..KNOCK_MODES.len() {
                    self.clunk.set_live(k, true);
                }
            }
            _ => {
                if self.creak_t >= self.creak_len {
                    self.creak_t = 0.0;
                    self.creak_len = self.rng.range(0.2, 0.6);
                    self.creak_from = self.rng.range(15.0, 40.0);
                    self.creak_to = self.creak_from * self.rng.range(0.6, 2.2);
                    self.creak_amp = a;
                }
            }
        }
    }
}

impl Generator for Boat {
    type P = BoatParams;
    const NAME: &'static str = "boat";
    const CATEGORY: &'static str = "water";
    const DOC: &'static str = "A small boat on the water: waves slapping the hull (faster into the waves), the bow wash rising with speed, \
        a wooden hull creaking, and oar strokes. Layer `combustion` for an outboard.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.0, doc: "0 drifting .. 1 full speed: the wash rises, and the boat meets the waves more often and harder" },
        InputSpec { name: "waves", default: 0.4, doc: "0 a flat calm .. 1 choppy: how hard and how many waves slap the hull" },
        InputSpec { name: "row", default: 0.0, doc: "0 no rowing; above 0 the oars stroke by themselves, harder and quicker as it rises" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.3;

    fn presets() -> Vec<(&'static str, BoatParams)> {
        vec![
            ("Canoe", BoatParams { hull_hz: 260.0, hull_size: 0.3, creak: 0.15, oars: 0.8, row_rate: 0.7, wash_hz: 1200.0, ..Default::default() }),
            ("Sailboat", BoatParams { hull_hz: 110.0, hull_size: 0.8, slap_rate: 0.6, wash: 1.3, wash_hz: 750.0, creak: 0.7, creak_hz: 300.0, oars: 0.0, ..Default::default() }),
            ("Motorboat hull", BoatParams { hull_hz: 150.0, hull_size: 0.6, creak: 0.0, oars: 0.0, wash: 1.5, wash_hz: 1400.0, slap_rate: 1.2, ..Default::default() }),
            ("Moored", BoatParams { slap_rate: 1.5, lap: 1.2, wash: 0.3, oars: 0.0, creak: 0.5, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        let seed = instance_seed(0xB0A7_0000);
        Boat {
            sr,
            rng: Rng::new(mix_seed(seed ^ 1)),
            noise: Noise::new(mix_seed(seed ^ 2)),
            queue: Queue::new(),
            wave_phase: 0.5,
            wave_len: 1.0,
            row_phase: 0.95,
            slap: Burst::default(),
            slap_bp: Svf::default(),
            slosh_lp: Svf::default(),
            knock: ModalBank::new(),
            knock_tuned: 0.0,
            knock_kick: 0.0,
            clunk_kick: 0.0,
            wash_bp: Svf::default(),
            wash_hp: OnePole::default(),
            lap_bp: Svf::default(),
            swell: SlowNoise::new(seed ^ 3),
            pull: Burst::default(),
            pull_lp: Svf::default(),
            splash: Burst::default(),
            splash_bp: Svf::default(),
            tick: Burst::default(),
            tick_hp: OnePole::default(),
            drop_dust: Dust::new(seed ^ 4),
            drop_rate: 0.0,
            pool: Bubbles::new(),
            fizz: Swarm { r_lo: 0.6, r_hi: 2.5, amp: 0.02, ..Swarm::new(seed ^ 5) },
            gurgle: Swarm { r_lo: 3.0, r_hi: 10.0, amp: 0.02, skew: 1.0, ..Swarm::new(seed ^ 6) },
            lap: Swarm { r_lo: 1.5, r_hi: 6.0, amp: 0.025, ..Swarm::new(seed ^ 7) },
            clunk: ModalBank::new(),
            creak: ModalBank::new(),
            creak_tuned: 0.0,
            creak_t: 1.0,
            creak_len: 0.0,
            creak_from: 20.0,
            creak_to: 20.0,
            creak_amp: 0.0,
            creak_acc: 0.0,
        }
    }

    fn block(&mut self, x: &[f32], p: &BoatParams, out: &mut [f32]) {
        let sr = self.sr;
        let dt = out.len() as f32 / sr;
        let (speed, waves, row) = (x[0], x[1], x[2]);
        if self.knock_tuned != p.hull_hz {
            self.knock_tuned = p.hull_hz;
            self.knock.set_len(KNOCK_MODES.len());
            self.clunk.set_len(KNOCK_MODES.len());
            for (k, &(ratio, t60, _)) in KNOCK_MODES.iter().enumerate() {
                self.knock.tune(k, p.hull_hz * 1.9 * ratio, t60 * (0.6 + p.hull_size), sr);
                self.clunk.tune(k, 380.0 * ratio, t60 * 0.5, sr);
                self.clunk.set_gain(k, KNOCK_MODES[k].2);
            }
        }
        if self.creak_tuned != p.creak_hz {
            self.creak_tuned = p.creak_hz;
            self.creak.set_len(CREAK_MODES.len());
            for (k, &(ratio, t60, level)) in CREAK_MODES.iter().enumerate() {
                self.creak.tune(k, p.creak_hz * ratio, t60, sr);
                self.creak.set_gain(k, level);
            }
        }
        // Waves meet the hull at the wave rate, more often the faster it goes into them. Each
        // wave slaps two or three times as it runs along the hull.
        let encounter = p.slap_rate * (1.0 + 1.5 * speed);
        self.wave_phase += dt / self.wave_len;
        if self.wave_phase >= 1.0 {
            self.wave_phase -= 1.0;
            self.wave_len = (1.0 + 0.3 * self.rng.next_bipolar()) / encounter;
            let force = waves.powf(0.8) * (0.6 + 0.4 * speed) * (0.35 + 0.65 * self.rng.next_f32().powf(0.7));
            if force > 0.01 {
                let n = 1 + (self.rng.next_f32() * 2.6) as usize;
                let mut at = 0.0;
                let mut a = force * p.slap;
                for _ in 0..n {
                    self.queue.push(at, B_SLAP, a);
                    at += self.rng.range(0.06, 0.22);
                    a *= self.rng.range(0.35, 0.8);
                }
                if self.rng.next_f32() < p.creak * waves * 0.7 {
                    self.queue.push(self.rng.range(0.0, 0.3), B_CREAK, p.creak * (0.4 + 0.6 * waves));
                }
            }
        }
        if row > 0.02 && p.oars > 0.0 {
            let rate = p.row_rate * (0.5 + 0.5 * row);
            self.row_phase += rate * dt;
            if self.row_phase >= 1.0 {
                self.row_phase -= 1.0;
                let a = p.oars * (0.45 + 0.55 * row) * self.rng.range(0.8, 1.0);
                self.queue.push(0.0, B_CATCH, a);
                self.queue.push(0.5 / rate.max(0.3), B_RELEASE, a);
                if self.rng.next_f32() < p.creak * 1.2 {
                    self.queue.push(self.rng.range(0.05, 0.25), B_CREAK, p.creak * 0.6 * a);
                }
            }
        } else {
            self.row_phase = 0.95;
        }
        while let Some((kind, a)) = self.queue.pop(dt) {
            self.fire(kind, a, p);
        }
        self.queue.advance(dt);
        let swell = self.swell.advance(0.35, dt);
        let wash = p.wash * speed.powf(1.5) * (1.0 + 0.45 * swell) * 0.32;
        self.wash_bp.set(FilterMode::BandPass, p.wash_hz * (0.6 + 0.7 * speed) * (1.0 + 0.1 * swell), 0.12, sr);
        let wash_hp = hz_coef(200.0, sr);
        self.slosh_lp.set(FilterMode::BandPass, 650.0, 0.05, sr);
        self.pull_lp.set(FilterMode::BandPass, 450.0, 0.2, sr);
        self.splash_bp.set(FilterMode::BandPass, 1400.0, 0.05, sr);
        self.fizz.rate = 400.0 * speed * speed * p.wash;
        self.lap.rate = p.lap * (2.0 + 10.0 * waves) * (1.0 - 0.6 * speed);
        // The water is never still against a hull: a soft wash of lapping under the slaps.
        self.lap_bp.set(FilterMode::BandPass, 550.0 * (1.0 + 0.2 * swell), 0.1, sr);
        let lapping = p.lap * (0.15 + waves) * (1.0 + 0.5 * swell) * 0.012;
        self.gurgle.rate *= (-(BLOCK as f32) / (0.35 * sr)).exp();
        self.drop_rate *= (-(BLOCK as f32) / (0.3 * sr)).exp();
        let drop_p = self.drop_rate / sr;
        let tick_coef = hz_coef(1800.0, sr);
        // The creak: stick-slip impulses gliding in rate, into wooden modes.
        let creak_rate = if self.creak_t < self.creak_len {
            let u = self.creak_t / self.creak_len;
            self.creak_t += dt;
            let env = (core::f32::consts::PI * u).sin();
            (self.creak_from * (self.creak_to / self.creak_from).powf(u), env * self.creak_amp)
        } else {
            (0.0, 0.0)
        };
        for o in out.iter_mut() {
            let w = self.noise.white();
            let pink = self.noise.pink();
            self.fizz.tick(&mut self.pool, sr);
            self.gurgle.tick(&mut self.pool, sr);
            self.lap.tick(&mut self.pool, sr);
            let d = self.drop_dust.tick(drop_p);
            if d > 0.0 {
                self.tick.hit(d * d * d * 0.08, 0.0003, 0.004, sr);
                if self.rng.next_f32() < 0.4 {
                    let hz = minnaert_hz(self.rng.range(1.2, 3.5));
                    self.pool.spawn(hz, d * d * 0.04, 0.03, 0.8, sr);
                }
            }
            let mut stick = 0.0;
            if creak_rate.1 > 0.0 {
                self.creak_acc += creak_rate.0 / sr;
                if self.creak_acc >= 1.0 {
                    self.creak_acc -= 1.0;
                    stick = creak_rate.1 * self.rng.range(0.5, 1.0) * 0.05;
                }
            }
            let s = self.slap.tick();
            let slap = (self.slap_bp.tick(w) * 1.5 + self.slosh_lp.tick(pink) * 0.6) * s;
            let knock = self.knock.tick(self.knock_kick);
            self.knock_kick = 0.0;
            let wash = self.wash_hp.hp(self.wash_bp.tick(pink), wash_hp) * wash + self.lap_bp.tick(pink) * lapping;
            let pull = self.pull_lp.tick(pink) * self.pull.tick();
            let splash = self.splash_bp.tick(pink) * self.splash.tick() * 2.0;
            let tick = self.tick_hp.hp(w, tick_coef) * self.tick.tick();
            let creak = self.creak.tick(stick);
            let clunk = self.clunk.tick(self.clunk_kick);
            self.clunk_kick = 0.0;
            let y = slap + knock + wash + pull + splash + tick + self.pool.tick() + creak + clunk;
            *o = y * p.gain * BOAT_LEVEL;
        }
        self.pool.end_block(1e-5);
        self.knock.energy(1e-6);
        self.clunk.energy(1e-6);
        self.creak.energy(1e-6);
        for k in 0..CREAK_MODES.len() {
            self.creak.set_live(k, true);
        }
        for b in [&mut self.slap, &mut self.pull, &mut self.splash, &mut self.tick] {
            b.quiet();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// underwater
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Under the surface: a dark rumble, your own breath bubbling up, the swirl of moving, far
    /// clicks; above it, water lapping at the ears. Crossing the surface plunges or breaks out.
    UnderwaterParams / UnderwaterParamId {
        rumble: "rumble/level" = 1.0, GAIN;
        rumble_hz: "rumble/hz" = 320.0, exp(60.0, 1000.0);
        breath: "breath/level" = 1.0, GAIN;
        breath_period: "breath/period_s" = 4.0, lin(1.5, 12.0);
        motion: "motion/level" = 1.0, GAIN;
        clicks: "clicks/per_second" = 4.0, exp(0.1, 200.0);
        clicks_level: "clicks/level" = 0.5, GAIN;
        surface: "surface/level" = 1.0, GAIN;
        plunge: "plunge/level" = 1.0, GAIN;
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Underwater {
    sr: f32,
    rng: Rng,
    noise: Noise,
    brown: Brown,
    rumble: [Svf; 2],
    rumble_hp: OnePole,
    swell: SlowNoise,
    drift: SlowNoise,
    motion_lp: Svf,
    breath_t: f32,
    breath_on: f32,
    pool: Bubbles,
    exhale: Swarm,
    rush: Burst,
    rush_lp: Svf,
    lap: Swarm,
    lap_bp: Svf,
    click_dust: Dust,
    click: Burst,
    click_bp: Svf,
    tick: Burst,
    tick_hp: OnePole,
    drop_dust: Dust,
    drop_rate: f32,
    ear: [Svf; 2],
    under: bool,
    started: bool,
}

impl Generator for Underwater {
    type P = UnderwaterParams;
    const NAME: &'static str = "underwater";
    const CATEGORY: &'static str = "water";
    const DOC: &'static str = "The world under the surface: a dark rumble, your breath bubbling up, the swirl of moving, far clicks. \
        submerge 0 is the head just above the water (lapping at the ears); crossing 0.5 plunges in or breaks the surface.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "submerge", default: 1.0, doc: "0 head above the surface .. 1 under: drive it from the camera's depth; it darkens through 0.35-0.65" },
        InputSpec { name: "breath", default: 0.3, doc: "Breathing out: 0 holding your breath .. 1 bursts of bubbles every breath/period_s, more and bigger" },
        InputSpec { name: "motion", default: 0.3, doc: "0 floating still .. 1 swimming hard: the water rushing past the ears" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.1;

    fn presets() -> Vec<(&'static str, UnderwaterParams)> {
        vec![
            ("Sea", UnderwaterParams { rumble_hz: 260.0, clicks: 60.0, clicks_level: 0.4, ..Default::default() }),
            ("Pool", UnderwaterParams { rumble: 0.5, rumble_hz: 420.0, clicks: 0.3, breath_period: 3.0, ..Default::default() }),
            ("Deep", UnderwaterParams { rumble: 1.3, rumble_hz: 160.0, clicks: 1.0, clicks_level: 0.3, breath_period: 6.0, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        let seed = instance_seed(0x0DE7_0000);
        Underwater {
            sr,
            rng: Rng::new(mix_seed(seed ^ 1)),
            noise: Noise::new(mix_seed(seed ^ 2)),
            brown: Brown::default(),
            rumble: [Svf::default(); 2],
            rumble_hp: OnePole::default(),
            swell: SlowNoise::new(seed ^ 3),
            drift: SlowNoise::new(seed ^ 4),
            motion_lp: Svf::default(),
            breath_t: 0.0,
            breath_on: 0.0,
            pool: Bubbles::new(),
            exhale: Swarm { r_lo: 2.0, r_hi: 12.0, amp: 0.12, skew: 1.0, ..Swarm::new(seed ^ 5) },
            rush: Burst::default(),
            rush_lp: Svf::default(),
            lap: Swarm { r_lo: 1.0, r_hi: 4.0, amp: 0.04, ..Swarm::new(seed ^ 6) },
            lap_bp: Svf::default(),
            click_dust: Dust::new(seed ^ 7),
            click: Burst::default(),
            click_bp: Svf::default(),
            tick: Burst::default(),
            tick_hp: OnePole::default(),
            drop_dust: Dust::new(seed ^ 8),
            drop_rate: 0.0,
            ear: [Svf::default(); 2],
            under: true,
            started: false,
        }
    }

    fn snap(&mut self, x: &[f32], _p: &UnderwaterParams) {
        self.under = x[0] >= 0.5;
        self.started = true;
    }

    fn block(&mut self, x: &[f32], p: &UnderwaterParams, out: &mut [f32]) {
        let sr = self.sr;
        let dt = out.len() as f32 / sr;
        let (submerge, breath, motion) = (x[0], x[1], x[2]);
        let u = submerged(submerge);
        let under = submerge >= 0.5;
        if !self.started {
            self.started = true;
            self.under = under;
        }
        if under != self.under {
            self.under = under;
            if under {
                // Plunging in: a rush of bubbles past the ears and a low whoosh.
                self.exhale.rate += 600.0 * p.plunge;
                self.rush.hit(0.5 * p.plunge, 0.03, 0.8, sr);
            } else {
                // Breaking the surface: water pouring off the head, then drips.
                self.rush.hit(0.25 * p.plunge, 0.01, 0.4, sr);
                self.drop_rate += 120.0 * p.plunge;
            }
        }
        // Breathing out: a burst of bubbles every period, rising past the face.
        if breath > 0.01 && u > 0.5 {
            self.breath_t += dt;
            if self.breath_t >= p.breath_period * (1.2 - 0.4 * breath) {
                self.breath_t = self.rng.range(-0.4, 0.4);
                self.breath_on = self.rng.range(0.6, 1.2);
            }
        }
        if self.breath_on > 0.0 {
            self.breath_on -= dt;
            self.exhale.rate = self.exhale.rate.max(p.breath * (60.0 + 140.0 * breath));
            self.exhale.r_hi = 6.0 + 8.0 * breath;
        }
        self.exhale.rate *= (-(BLOCK as f32) / (0.25 * sr)).exp();
        self.exhale.amp = 0.1 * u.max(0.3);
        self.drop_rate *= (-(BLOCK as f32) / (0.4 * sr)).exp();
        // Above the surface: water lapping and plipping around the head.
        self.lap.rate = p.surface * 30.0 * (1.0 - u);
        let swell = (0.5 * self.swell.advance(0.15, dt)).exp2();
        let drift = self.drift.advance(0.4, dt);
        let rumble_hz = p.rumble_hz * (1.0 + 0.15 * drift);
        for f in self.rumble.iter_mut() {
            f.set(FilterMode::LowPass, rumble_hz, 0.1, sr);
        }
        self.motion_lp.set(FilterMode::LowPass, 300.0 + 500.0 * motion, 0.3, sr);
        self.rush_lp.set(FilterMode::LowPass, 900.0 + 2500.0 * (1.0 - u), 0.1, sr);
        self.lap_bp.set(FilterMode::BandPass, 2200.0, 0.05, sr);
        self.click_bp.set(FilterMode::BandPass, 2000.0, 0.3, sr);
        let rumble = p.rumble * u * 1.6 * swell;
        let moving = p.motion * motion * (0.6 + 0.4 * swell) * (0.1 + 0.9 * u) * 0.8;
        let click_p = p.clicks * u / sr;
        let click_gain = p.clicks_level * 0.3;
        let lap_noise = p.surface * (1.0 - u) * 0.12 * swell;
        let drop_p = self.drop_rate / sr;
        let tick_coef = hz_coef(1800.0, sr);
        let hp70 = hz_coef(70.0, sr);
        let muffle_hz = 18000.0 * (2500.0f32 / 18000.0).powf(u);
        for f in self.ear.iter_mut() {
            f.set(FilterMode::LowPass, muffle_hz, 0.0, sr);
        }
        for o in out.iter_mut() {
            let w = self.noise.white();
            let pink = self.noise.pink();
            let br = self.brown.tick(w);
            self.exhale.tick(&mut self.pool, sr);
            self.lap.tick(&mut self.pool, sr);
            if self.click_dust.tick(click_p) > 0.0 {
                let a = self.rng.next_f32();
                self.click.hit(a * a * click_gain, 0.0002, 0.003, sr);
            }
            let d = self.drop_dust.tick(drop_p);
            if d > 0.0 {
                self.tick.hit(d * d * 0.06, 0.0003, 0.004, sr);
                let hz = minnaert_hz(self.rng.range(1.5, 4.0));
                self.pool.spawn(hz, d * d * 0.04, 0.03, 0.8, sr);
            }
            let low = self.rumble[0].tick(pink + 0.3 * br);
            let low = self.rumble_hp.hp(self.rumble[1].tick(low), hp70) * rumble;
            let swirl = self.motion_lp.tick(pink) * moving;
            let rush = self.rush_lp.tick(pink) * self.rush.tick();
            let clicks = self.click_bp.tick(w) * self.click.tick();
            let lap = self.lap_bp.tick(pink) * lap_noise;
            let tick = self.tick_hp.hp(w, tick_coef) * self.tick.tick();
            // What is in the water with you reaches the ear dulled (above 2.5 kHz little gets
            // through), but far less than sound from above the surface does.
            let near = clicks + lap + tick + self.pool.tick() * (1.0 - 0.5 * u) + rush;
            let near = self.ear[0].tick(near);
            let near = self.ear[1].tick(near);
            *o = (low + swirl + near) * p.gain * UNDERWATER_LEVEL;
        }
        self.pool.end_block(1e-5);
        for b in [&mut self.rush, &mut self.click, &mut self.tick] {
            b.quiet();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// drip
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Drips and leaks: each drop a click, a bubble's ping when it lands in water, and often a
    /// second smaller drop thrown back up by the rebound.
    DripParams / DripParamId {
        radius: "drops/bubble_mm" = 2.4, exp(0.8, 8.0);
        regular: "drops/regularity" = 0.7, UNIT;
        ring: "drops/ring" = 0.35, exp(0.1, 3.0);
        chance: "drops/bubble_chance" = 0.8, UNIT;
        rebound: "drops/rebound" = 0.6, UNIT;
        click: "drops/click" = 1.0, GAIN;
        room: "space/amount" = 0.0, UNIT;
        room_time: "space/tail" = 1.5, lin(0.3, 5.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// Drips per second at `rate` 0..1: one every 5 s up to 20 a second (evenly in octaves).
pub fn drip_rate(rate: f32) -> f32 {
    0.2 * 100.0f32.powf(rate.clamp(0.0, 1.0))
}

/// The radius (mm) of the bubble a drop of `size` (0..1) leaves, for `drops/bubble_mm` at 0.5.
pub fn drip_radius(param_mm: f32, size: f32) -> f32 {
    param_mm * (2.0 * (size.clamp(0.0, 1.0) - 0.5)).exp2()
}

pub struct Drip {
    sr: f32,
    rng: Rng,
    noise: Noise,
    queue: Queue,
    phase: f32,
    next: f32,
    pool: Bubbles,
    click: Burst,
    click_lp: Svf,
    click_hp: OnePole,
    spray: Burst,
    spray_bp: Svf,
    reverb: Box<Reverb>,
}

impl Drip {
    fn drop(&mut self, x: &[f32], p: &DripParams, amp: f32) {
        let sr = self.sr;
        let (size, pool) = (x[1], x[2]);
        let s = size * (0.96 + 0.08 * self.rng.next_f32());
        let a = amp * (0.5 + 0.5 * s) * self.rng.range(0.75, 1.0);
        // The impact: a click (sharper on a hard floor) and, on a hard floor, a little spray.
        self.click.hit(p.click * a * (0.15 + 0.2 * (1.0 - pool)), 0.0002, 0.0025 + 0.003 * s, sr);
        self.spray.hit(a * 0.18 * (1.0 - pool) * (1.0 - pool), 0.001, 0.02 + 0.02 * s, sr);
        // Into water, the drop pulls down a bubble that rings and rises a little in pitch.
        if self.rng.next_f32() < p.chance * pool.sqrt() {
            let r = drip_radius(p.radius, s) * self.rng.range(0.9, 1.1);
            self.pool.spawn(minnaert_hz(r), a * 0.45 * pool.sqrt(), 0.025, p.ring, sr);
        }
    }
}

impl Generator for Drip {
    type P = DripParams;
    const NAME: &'static str = "drip";
    const CATEGORY: &'static str = "water";
    const DOC: &'static str = "Drips and leaks, from a slow drip to a fast leak: a click, a bubble's ping into water and the rebound drop. \
        Leave rate at 0 and trigger() one drop per drip, or set rate.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "rate", default: 0.3, doc: "0: drops only on trigger(). Above 0: one every 5 s at 0.01 up to 20 a second at 1 (evenly in octaves)" },
        InputSpec { name: "size", default: 0.5, doc: "Drop size: 0 fine (higher, softer) .. 1 fat (lower, louder)" },
        InputSpec { name: "pool", default: 1.0, doc: "Where it lands: 0 a hard wet floor (a tick) .. 1 standing water (a plink)" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.05;

    fn presets() -> Vec<(&'static str, DripParams)> {
        vec![
            ("Leaky tap", DripParams { radius: 2.2, regular: 0.9, rebound: 0.7, ring: 0.25, ..Default::default() }),
            ("Cave", DripParams { radius: 3.0, regular: 0.4, ring: 0.8, room: 0.6, room_time: 3.5, ..Default::default() }),
            ("Bucket", DripParams { radius: 3.6, ring: 0.7, rebound: 0.4, room: 0.15, room_time: 0.5, ..Default::default() }),
            ("Ceiling leak", DripParams { regular: 0.6, chance: 0.5, radius: 2.0, rebound: 0.3, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        let seed = instance_seed(0xD819_0000);
        Drip {
            sr,
            rng: Rng::new(mix_seed(seed ^ 1)),
            noise: Noise::new(mix_seed(seed ^ 2)),
            queue: Queue::new(),
            phase: 0.0,
            next: 1.0,
            pool: Bubbles::new(),
            click: Burst::default(),
            click_lp: Svf::default(),
            click_hp: OnePole::default(),
            spray: Burst::default(),
            spray_bp: Svf::default(),
            reverb: Box::new(Reverb::new(sr)),
        }
    }

    fn trigger(&mut self, x: &[f32], p: &DripParams) {
        self.drop(x, p, 1.0);
        if self.rng.next_f32() < p.rebound {
            let a = self.rng.range(0.3, 0.6);
            self.queue.push(self.rng.range(0.11, 0.2), 0, a);
        }
    }

    fn block(&mut self, x: &[f32], p: &DripParams, out: &mut [f32]) {
        let sr = self.sr;
        let dt = out.len() as f32 / sr;
        let rate = x[0];
        if rate > 0.0 {
            self.phase += drip_rate(rate) * dt;
            if self.phase >= self.next {
                self.phase = 0.0;
                // A tap drips like a clock; a cave ceiling at random.
                let irr = 1.0 - p.regular;
                self.next = if self.rng.next_f32() < irr * 0.5 { -(1.0 - self.rng.next_f32()).ln().min(4.0) } else { 1.0 + 0.6 * irr * self.rng.next_bipolar() };
                self.trigger(x, p);
            }
        }
        while let Some((_, a)) = self.queue.pop(dt) {
            // The rebound drop: smaller, a higher ping.
            let x2 = [x[0], x[1] * 0.5, x[2]];
            self.drop(&x2, p, a);
        }
        self.queue.advance(dt);
        let pool = x[2];
        self.click_lp.set(FilterMode::LowPass, 5000.0 + 4000.0 * (1.0 - pool), 0.1, sr);
        self.spray_bp.set(FilterMode::BandPass, 3500.0, 0.1, sr);
        let hp = hz_coef(700.0, sr);
        let (room, rt) = (p.room, p.room_time);
        for o in out.iter_mut() {
            let w = self.noise.white();
            let click = self.click_hp.hp(self.click_lp.tick(w), hp) * self.click.tick();
            let spray = self.spray_bp.tick(w) * self.spray.tick();
            let mut y = click + spray + self.pool.tick();
            if room > 0.0 {
                y += self.reverb.tick(y, rt, 0.35) * room * 2.0;
            }
            *o = y * p.gain * DRIP_LEVEL;
        }
        self.pool.end_block(1e-5);
        self.click.quiet();
        self.spray.quiet();
    }
}

// ---------------------------------------------------------------------------------------------
// pour
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Pouring into a vessel: the splash and bubbles of the stream, and the air column above the
    /// water ringing higher as it fills (a quarter-wave tube getting shorter).
    PourParams / PourParamId {
        empty_hz: "vessel/empty_hz" = 430.0, exp(80.0, 3000.0);
        rise: "vessel/rise" = 4.5, lin(1.2, 8.0);
        neck: "vessel/neck" = 1.0, UNIT;
        resonance: "vessel/resonance" = 0.94, lin(0.5, 0.99);
        ring: "vessel/level" = 1.0, GAIN;
        splash: "stream/splash" = 1.0, GAIN;
        bubbles: "stream/bubbles" = 1.0, GAIN;
        bubble_mm: "stream/bubble_mm" = 2.0, exp(0.6, 8.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// The pitch of the air above the water in a vessel that rings at `empty_hz` empty and `rise`
/// times higher full, at `fill` 0..1. An open glass is a quarter-wave tube getting shorter
/// (`neck` 0: the pitch goes as 1 / the air's length); a bottle is a Helmholtz resonator
/// (`neck` 1: as 1 / sqrt of the air's volume). Either way it rises slowly at first and fast
/// near the top. The bottle we measured fits `neck` 1: 430 Hz empty, 590 half full, 800 at
/// three quarters, 1.9 kHz nearly full.
pub fn pour_hz(empty_hz: f32, rise: f32, neck: f32, fill: f32) -> f32 {
    let e = 1.0 - 0.5 * neck.clamp(0.0, 1.0);
    let k = 1.0 - rise.max(1.0).powf(-1.0 / e);
    empty_hz * (1.0 - fill.clamp(0.0, 1.0) * k).powf(-e)
}

pub struct Pour {
    sr: f32,
    noise: Noise,
    pool: Bubbles,
    swarm: Swarm,
    gush: SlowNoise,
    flutter: SlowNoise,
    splash_bp: Svf,
    tube: [Svf; 2],
    dust: Dust,
    tick: Burst,
    tick_hp: OnePole,
    hz: f32,
}

impl Generator for Pour {
    type P = PourParams;
    const NAME: &'static str = "pour";
    const CATEGORY: &'static str = "water";
    const DOC: &'static str = "Pouring into a glass, a bottle or a bucket: the stream's splash and bubbles, and the vessel ringing higher as it fills.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "flow", default: 0.6, doc: "0 stopped .. 1 a thick gush" },
        InputSpec { name: "fill", default: 0.0, doc: "How full the vessel is, 0 empty .. 1 brim: its pitch rises as it fills" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.08;

    fn presets() -> Vec<(&'static str, PourParams)> {
        vec![
            ("Glass", PourParams { empty_hz: 900.0, rise: 2.5, neck: 0.0, resonance: 0.85, ring: 0.8, bubble_mm: 1.6, ..Default::default() }),
            ("Jug", PourParams { empty_hz: 300.0, rise: 3.5, neck: 0.5, resonance: 0.9, bubble_mm: 2.5, ..Default::default() }),
            ("Bucket", PourParams { empty_hz: 170.0, rise: 2.0, neck: 0.0, resonance: 0.8, ring: 0.6, splash: 1.3, bubble_mm: 3.5, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Pour {
            sr,
            noise: Noise::new(0x9002_0001),
            pool: Bubbles::new(),
            swarm: Swarm { skew: 1.2, ..Swarm::new(0x9002_0002) },
            gush: SlowNoise::new(0x9002_0003),
            flutter: SlowNoise::new(0x9002_0004),
            splash_bp: Svf::default(),
            tube: [Svf::default(); 2],
            dust: Dust::new(0x9002_0005),
            tick: Burst::default(),
            tick_hp: OnePole::default(),
            hz: 0.0,
        }
    }

    fn block(&mut self, x: &[f32], p: &PourParams, out: &mut [f32]) {
        let sr = self.sr;
        let dt = out.len() as f32 / sr;
        let (flow, fill) = (x[0], x[1]);
        // The stream gushes and wavers, and the water level sloshes the pitch a little.
        let gush = 1.0 + 0.35 * self.gush.advance(3.0, dt);
        let wobble = 1.0 + 0.01 * self.flutter.advance(6.0, dt);
        self.hz = pour_hz(p.empty_hz, p.rise, p.neck, fill) * wobble;
        let res = p.resonance;
        self.tube[0].set(FilterMode::BandPass, self.hz, res, sr);
        self.tube[1].set(FilterMode::BandPass, (self.hz * 3.0).min(0.45 * sr), res * 0.97, sr);
        self.splash_bp.set(FilterMode::BandPass, 1400.0 * (1.0 + 0.5 * flow), 0.05, sr);
        // The water falls a shorter way as the vessel fills, so the splash quietens.
        let fall = 1.0 - 0.8 * fill;
        let on = smoothstep(0.0, 0.08, flow);
        let level = on * flow.sqrt() * fall * gush;
        self.swarm.r_lo = p.bubble_mm * 0.5;
        self.swarm.r_hi = p.bubble_mm * 2.0;
        self.swarm.amp = p.bubbles * 0.05 * on;
        self.swarm.rate = 600.0 * flow * gush * on;
        let tick_p = 40.0 * flow * on / sr;
        let tick_coef = hz_coef(1200.0, sr);
        // A narrow band-pass rings at its peak with gain about 1/(2 - 1.98 res) times less
        // energy; normalise so the ring's level stays put as the resonance changes.
        let norm = (2.0 - 1.98 * res).sqrt() * 5.5;
        let (ring, splash) = (p.ring * norm, p.splash * 0.35 * level);
        for o in out.iter_mut() {
            let pink = self.noise.pink();
            let w = self.noise.white();
            self.swarm.tick(&mut self.pool, sr);
            let d = self.dust.tick(tick_p);
            if d > 0.0 {
                self.tick.hit(d * d * 0.1 * fall, 0.0005, 0.01, sr);
            }
            let tick = self.tick_hp.hp(w, tick_coef) * self.tick.tick();
            let bubbles = self.pool.tick();
            let source = self.splash_bp.tick(pink) * splash + bubbles + tick;
            let tube = self.tube[0].tick(source) + 0.5 * self.tube[1].tick(source);
            *o = (source + tube * ring) * p.gain * POUR_LEVEL;
        }
        self.pool.end_block(1e-5);
        self.tick.quiet();
    }
}
