//! Machinery and trains, for puzzle, factory, sim and adventure games.
//!
//! * `train`: rail clack from the real axle geometry, rolling roar, a diesel or electric
//!   drive, curve squeal, brakes and a horn.
//! * `conveyor` and `press`: a factory line (belt, rollers, motor hum with load) and a
//!   periodic stamping or hydraulic press.
//! * `clock`, `gears`, `music_box` and the `unlock` event: clockwork for puzzles.
//! * `hydraulic`, the `pneumatic` event and `elevator`: pumps, pistons, air and lifts.
//!
//! Everything was measured against CC0 recordings (`target/refs/machines`, see
//! `tools/sound_views.py`); the numbers quoted in the docs below come from them. The common
//! thread is rhythm: a machine is recognised by its beat (the "ta-dum ta-dum" of bogies over
//! rail joints, the tick-tock of an escapement, a press stroke), so every rhythm here comes from
//! the mechanism's geometry and is steady, with variation only in the timbre of each hit.
//! Struck parts ring through [`crate::dsp::Modal`], so no two hits are identical.

use crate::blocks::{hz_coef, mix_seed, settle_coef, Brown, OnePole, Phasor, SlowNoise, BLOCK};
use crate::dsp::{Air, Modal, ModalBank};
use crate::filter::{FilterMode, Svf};
use crate::math::{soft_clip, Rng, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::osc::{Oscillator, Waveform};
use crate::params::{exp, int, lin, GAIN, UNIT};

// ---------------------------------------------------------------------------------------------
// Shared pieces
// ---------------------------------------------------------------------------------------------

/// Per-sample decay factor that falls 60 dB in `t60` seconds.
#[inline]
fn t60_coef(t60: f32, sr: f32) -> f32 {
    (-6.91 / (t60.max(1e-4) * sr)).exp()
}

/// Move `y` toward `target` with separate rise/fall settle times (seconds, ~95 %).
#[inline]
fn slew(y: &mut f32, target: f32, up: f32, down: f32, dt: f32) -> f32 {
    let secs = if target > *y { up } else { down };
    *y += (target - *y) * settle_coef(secs, dt);
    *y
}

#[inline]
fn smoothstep(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A hash of an integer to 0..1, for properties that must repeat (a rail joint's gap, a gear
/// tooth's wear) while looking random.
#[inline]
fn hash01(i: i64, salt: u32) -> f32 {
    let h = mix_seed((i as u32) ^ ((i >> 32) as u32).wrapping_mul(0x27D4_EB2F) ^ salt);
    (h >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// A decaying noise burst through a filter: the contact noise of a hit, a puff of air. `kick`
/// adds to the envelope, so overlapping hits sum.
#[derive(Clone, Copy, Debug, Default)]
struct Burst {
    env: f32,
    decay: f32,
    f: Svf,
}

impl Burst {
    fn set(&mut self, mode: FilterMode, hz: f32, res: f32, t60: f32, sr: f32) {
        self.f.set(mode, hz, res, sr);
        self.decay = t60_coef(t60, sr);
    }

    #[inline]
    fn kick(&mut self, a: f32) {
        self.env += a;
    }

    #[inline]
    fn tick(&mut self, noise: f32) -> f32 {
        if self.env < 1e-7 {
            self.env = 0.0;
            return self.f.tick(0.0);
        }
        let y = self.f.tick(noise * self.env);
        self.env *= self.decay;
        y
    }
}

/// Impulses scheduled inside one control block (sample offset, amplitude).
#[derive(Clone, Copy, Debug)]
struct Hits {
    x: [f32; BLOCK],
}

impl Default for Hits {
    fn default() -> Self {
        Hits { x: [0.0; BLOCK] }
    }
}

impl Hits {
    #[inline]
    fn add(&mut self, at: usize, a: f32) {
        self.x[at.min(BLOCK - 1)] += a;
    }

    fn clear(&mut self) {
        self.x = [0.0; BLOCK];
    }
}

/// A short metallic or wooden click: a [`Modal`] ring plus a contact burst. The workhorse of the
/// clockwork, gears, rollers and latches.
#[derive(Clone, Debug)]
struct Click {
    ring: Modal,
    burst: Burst,
}

impl Click {
    fn new(ratios: &[f32], t60: &[f32], level: &[f32], seed: u32) -> Self {
        Click { ring: Modal::new(ratios, t60, level, seed), burst: Burst::default() }
    }

    /// Per block: ring modes at `hz` (ratio 1) with ring-time factor `decay`; contact noise
    /// band-passed at `noise_hz`, falling 60 dB in `noise_t60` seconds.
    fn set(&mut self, hz: f32, decay: f32, noise_hz: f32, noise_t60: f32, sr: f32) {
        self.ring.set(hz, 0.5, decay, sr);
        self.burst.set(FilterMode::BandPass, noise_hz, 0.25, noise_t60, sr);
    }

    /// `x` is an impulse (0 most samples), `noise` a white noise sample, `click` the level of
    /// the contact noise against the ring.
    #[inline]
    fn tick(&mut self, x: f32, noise: f32, click: f32, variation: f32, sr: f32) -> f32 {
        if x != 0.0 {
            self.burst.kick(x * click);
        }
        self.ring.tick(x, 0.23, variation, sr) + self.burst.tick(noise)
    }

    fn end_block(&mut self) {
        self.ring.end_block();
    }
}

// ---------------------------------------------------------------------------------------------
// Train
// ---------------------------------------------------------------------------------------------

/// Most axles the train tracks: 16 cars of two two-axle bogies.
pub const MAX_AXLES: usize = 64;

/// Axle positions of one car, in metres from its front end: two bogies `bogie_m` in from
/// each end, each with two axles `wheelbase_m` apart.
pub fn car_axles(car_m: f32, bogie_m: f32, wheelbase_m: f32) -> [f32; 4] {
    let b = bogie_m.clamp(0.5 * wheelbase_m, 0.5 * car_m - 0.5 * wheelbase_m);
    let h = 0.5 * wheelbase_m;
    [b - h, b + h, car_m - b - h, car_m - b + h]
}

/// Seconds after the front axle crosses a rail joint at which each axle of a `cars`-car train
/// crosses it, at `speed_mps`: the clack rhythm a listener at that joint hears.
pub fn clack_times(cars: usize, car_m: f32, bogie_m: f32, wheelbase_m: f32, speed_mps: f32) -> Vec<f32> {
    let axles = car_axles(car_m, bogie_m, wheelbase_m);
    let first = axles[0];
    (0..cars.clamp(1, MAX_AXLES / 4)).flat_map(|c| axles.map(|a| (c as f32 * car_m + a - first) / speed_mps.max(1e-3))).collect()
}

model_params! {
    /// A train. The clack is the train's own geometry rolling over jointed rail: every axle
    /// clacks as it crosses a joint (`track/joint_m` apart), two axles per bogie
    /// (`train/wheelbase_m` apart, the "ta-dum") and two bogies per car (`train/bogie_m` in
    /// from each end), so the rhythm follows speed, car length and the number of cars by
    /// itself. `listen/view` 0 rides along at `listen/position` (0 front .. 1 rear): your own
    /// car's bogies are loud, the others fainter and duller. 1 stands at the trackside by a
    /// joint while an endless train rolls past, every axle of every car crossing it.
    ///
    /// Measured (`target/refs/machines/train`): a freight car's axles 150-160 ms apart at about
    /// 40 km/h (1.8 m wheelbase), a car every 1.5 s; each clack 8-10 dB above the roll, a broad
    /// knock centred near 900 Hz that falls back into the roar within 20-30 ms. Onboard the
    /// roar sits under 300 Hz. A curve squeal is a few narrow tones at 3.8-6.2 kHz; a three-chime
    /// horn 322, 390 and 431 Hz with strong harmonics to 4 kHz; a two-tone one 366 and 460 Hz.
    TrainParams / TrainParamId {
        view: "listen/view" = 0.0, int(0, 1);
        seat: "listen/position" = 0.5, UNIT;
        cars: "train/cars" = 6.0, int(1, 16);
        car_m: "train/car_m" = 24.0, lin(6.0, 30.0);
        bogie_m: "train/bogie_m" = 3.5, lin(0.8, 8.0);
        wheelbase_m: "train/wheelbase_m" = 2.5, lin(1.2, 4.0);
        max_kmh: "train/max_kmh" = 140.0, exp(10.0, 320.0);
        joint_m: "track/joint_m" = 20.0, lin(5.0, 40.0);
        clack_level: "clack/level" = 0.8, UNIT;
        clack_hz: "clack/hz" = 900.0, exp(200.0, 4000.0);
        clack_ring: "clack/ring" = 0.35, UNIT;
        thump: "clack/thump" = 0.6, UNIT;
        roll_level: "rolling/level" = 0.6, UNIT;
        roll_hz: "rolling/hz" = 900.0, exp(150.0, 5000.0);
        drive: "drive/kind" = 0.0, int(0, 2);
        drive_level: "drive/level" = 0.6, UNIT;
        diesel_rpm: "drive/diesel_max_rpm" = 900.0, lin(300.0, 2400.0);
        cylinders: "drive/diesel_cylinders" = 12.0, int(4, 20);
        motor_hz: "drive/electric_max_hz" = 1300.0, exp(100.0, 5000.0);
        inverter: "drive/electric_inverter" = 0.5, UNIT;
        squeal_level: "squeal/level" = 0.6, UNIT;
        squeal_hz: "squeal/hz" = 3900.0, exp(800.0, 8000.0);
        brake_level: "brake/level" = 0.6, UNIT;
        air_level: "brake/air" = 0.5, UNIT;
        horn_kind: "horn/kind" = 0.0, int(0, 2);
        horn_level: "horn/level" = 0.7, UNIT;
        horn_pitch: "horn/pitch_semitones" = 0.0, lin(-12.0, 12.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// Horn chords (Hz) and levels: three-chime and two-tone measured on recordings; the whistle's
/// three pipes a minor chord.
const HORNS: [[(f32, f32); 3]; 3] = [
    [(322.0, 0.8), (390.0, 1.0), (431.0, 1.0)],
    [(366.0, 1.0), (460.0, 0.9), (0.0, 0.0)],
    [(587.0, 0.9), (698.0, 1.0), (880.0, 0.7)],
];

/// One axle (or, trackside, one axle and one joint) whose crossings make clacks.
#[derive(Clone, Copy, Debug, Default)]
struct Crosser {
    /// Crossing when `(pos - offset) / period` passes an integer.
    offset: f64,
    period: f64,
    last: i64,
    weight: f32,
    /// Share that is heard bright (near) rather than dull (far).
    bright: f32,
    /// Trackside: the joint is fixed, so its gap is too.
    joint: Option<i64>,
}

pub struct Train {
    sr: f32,
    /// Distance travelled by the front axle, metres.
    pos: f64,
    v: f32,
    crossers: [Crosser; MAX_AXLES],
    n: usize,
    geometry: [f32; 7],
    rng: Rng,
    noise: Noise,
    near: Hits,
    far: Hits,
    knock: Burst,
    knock_far: Burst,
    rail: Modal,
    body: Modal,
    pass_am: f32,
    roar: Svf,
    rumble: Svf,
    brown: Brown,
    cabin: [OnePole; 2],
    // Drive.
    rpm: f32,
    fire: Phasor,
    fire_count: u32,
    pulse_amp: f32,
    exhaust: Svf,
    turbo: f32,
    turbo_ph: Phasor,
    mesh: Phasor,
    mesh_noise: Svf,
    carrier: Phasor,
    carrier_hz: f32,
    // Squeal and brakes.
    squeal_gate: SlowNoise,
    squeal_wobble: SlowNoise,
    squeal_ph: [Phasor; 3],
    squeal_amp: f32,
    flange: Svf,
    shoe: Svf,
    brake_squeal: Phasor,
    stick: SlowNoise,
    brake_prev: f32,
    air: Burst,
    air_hp: OnePole,
    // Horn.
    horn: f32,
    horn_osc: [Oscillator; 3],
    horn_lp: Svf,
    horn_bell: Svf,
    breath: [Svf; 3],
}

impl Train {
    fn rebuild(&mut self, p: &TrainParams) {
        let key = [p.view, p.seat, p.cars, p.car_m, p.bogie_m, p.wheelbase_m, p.joint_m];
        if key == self.geometry {
            return;
        }
        self.geometry = key;
        let axles = car_axles(p.car_m, p.bogie_m, p.wheelbase_m);
        let joint = p.joint_m as f64;
        self.n = 0;
        if p.view < 0.5 {
            // Onboard: every axle of every car crosses every joint; you hear the ones near you.
            let cars = p.cars as usize;
            let length = cars as f32 * p.car_m;
            let me = (p.seat * length).clamp(0.5 * p.car_m.min(length), length - 0.5 * p.car_m.min(length));
            for c in 0..cars {
                for a in axles {
                    let off = c as f32 * p.car_m + a;
                    let d = (off - me).abs();
                    let weight = 1.0 / (1.0 + (d / 8.0) * (d / 8.0));
                    self.crossers[self.n] = Crosser { offset: off as f64, period: joint, last: 0, weight, bright: (1.0 - d / 30.0).clamp(0.0, 1.0), joint: None };
                    self.n += 1;
                }
            }
        } else {
            // Trackside: the joint by the listener and two either side, crossed by every axle of
            // an endless train of identical cars.
            for k in -2i64..=2 {
                let d = (k as f32 * p.joint_m).abs();
                let weight = 1.0 / (1.0 + (d / 6.0) * (d / 6.0));
                for a in axles {
                    let offset = a as f64 + k as f64 * joint;
                    self.crossers[self.n] = Crosser { offset, period: p.car_m as f64, last: 0, weight, bright: (1.0 - d / 25.0).clamp(0.0, 1.0), joint: Some(k) };
                    self.n += 1;
                }
            }
        }
        for c in self.crossers[..self.n].iter_mut() {
            c.last = ((self.pos - c.offset) / c.period).floor() as i64;
        }
    }

    /// Schedule the clacks of this block, for a move from `pos` by `ds` metres.
    fn clacks(&mut self, ds: f64, n: usize, p: &TrainParams) {
        self.near.clear();
        self.far.clear();
        if ds <= 0.0 {
            return;
        }
        // A harder knock at speed: amplitude follows impact speed.
        let hit = (self.v / 25.0).powf(0.6).min(1.6) * p.clack_level;
        let pos1 = self.pos + ds;
        for i in 0..self.n {
            let c = self.crossers[i];
            let idx = ((pos1 - c.offset) / c.period).floor() as i64;
            if idx <= c.last {
                continue;
            }
            self.crossers[i].last = idx;
            let at_pos = idx as f64 * c.period + c.offset;
            let at = (((at_pos - self.pos) / ds) * n as f64).clamp(0.0, (n - 1) as f64) as usize;
            // Each joint has its own gap and height step, heard on every axle that crosses it.
            let joint = c.joint.unwrap_or(idx);
            let gap = 0.55 + 0.45 * hash01(joint, 0x7A11);
            let a = hit * c.weight * gap * self.rng.range(0.85, 1.15);
            self.near.add(at, a * c.bright);
            self.far.add(at, a * (1.0 - c.bright));
        }
    }

    /// Trackside: how close the nearest bogie is to the listener, 0..1 (the roar swells as
    /// each passes).
    fn proximity(&self, p: &TrainParams) -> f32 {
        let car = p.car_m as f64;
        let u = self.pos.rem_euclid(car) as f32;
        let b = car_axles(p.car_m, p.bogie_m, p.wheelbase_m);
        let centres = [0.5 * (b[0] + b[1]), 0.5 * (b[2] + b[3])];
        let mut d = f32::MAX;
        for c in centres {
            let x = (u - c).abs();
            d = d.min(x.min(p.car_m - x));
        }
        1.0 / (1.0 + (d / 3.0) * (d / 3.0))
    }
}

/// Ring of the rail and wheel at a joint (ratios of `clack/hz`), and the car body's thump.
const RAIL_RATIOS: [f32; 5] = [0.78, 1.65, 2.45, 3.4, 4.7];
const RAIL_T60: [f32; 5] = [0.09, 0.07, 0.05, 0.04, 0.03];
const RAIL_LEVEL: [f32; 5] = [1.0, 0.8, 0.6, 0.45, 0.3];
const BODY_RATIOS: [f32; 3] = [1.0, 1.9, 3.1];
const BODY_T60: [f32; 3] = [0.18, 0.1, 0.06];
const BODY_LEVEL: [f32; 3] = [1.0, 0.5, 0.25];

impl Generator for Train {
    type P = TrainParams;
    const NAME: &'static str = "train";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "Train: the ta-dum ta-dum of every axle over the rail joints (tempo from speed, car length and \
        car count), rolling roar, a diesel or electric drive, curve squeal, brakes with air, and a horn.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.0, doc: "Speed, 0 stopped .. 1 = train/max_kmh: clack tempo, roar" },
        InputSpec { name: "throttle", default: 0.3, doc: "Traction effort: diesel revs, electric inverter and motor load" },
        InputSpec { name: "curve", default: 0.0, doc: "Curve tightness: wheel squeal and flange noise" },
        InputSpec { name: "braking", default: 0.0, doc: "Brake application: shoe grind, squeal near a stop, air on apply and release" },
        InputSpec { name: "horn", default: 0.0, doc: "Horn or whistle: hold above 0 to sound it (its level)" },
    ];

    fn presets() -> Vec<(&'static str, TrainParams)> {
        let d = TrainParams::default();
        vec![
            (
                "Freight trackside",
                TrainParams { view: 1.0, cars: 16.0, car_m: 17.0, bogie_m: 2.0, wheelbase_m: 1.8, max_kmh: 100.0, joint_m: 12.0, clack_hz: 1100.0, clack_ring: 0.5, thump: 0.35, roll_hz: 1800.0, drive_level: 0.35, squeal_level: 0.7, ..d },
            ),
            ("Electric commuter", TrainParams { cars: 4.0, car_m: 22.0, bogie_m: 3.2, wheelbase_m: 2.3, drive: 1.0, joint_m: 25.0, horn_kind: 1.0, ..d }),
            (
                "Metro",
                TrainParams { cars: 6.0, car_m: 16.0, bogie_m: 2.4, wheelbase_m: 2.0, max_kmh: 80.0, drive: 1.0, motor_hz: 1800.0, inverter: 0.8, joint_m: 15.0, clack_hz: 1300.0, squeal_level: 0.8, squeal_hz: 2600.0, horn_kind: 1.0, ..d },
            ),
            ("Heritage line", TrainParams { cars: 4.0, car_m: 18.0, bogie_m: 2.8, wheelbase_m: 2.4, max_kmh: 70.0, joint_m: 13.7, clack_level: 1.0, clack_ring: 0.5, thump: 0.8, horn_kind: 2.0, drive_level: 0.4, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        Train {
            sr,
            pos: 0.0,
            v: 0.0,
            crossers: [Crosser::default(); MAX_AXLES],
            n: 0,
            geometry: [f32::NAN; 7],
            rng: Rng::new(mix_seed(0x7_0001)),
            noise: Noise::new(mix_seed(0x7_0002)),
            near: Hits::default(),
            far: Hits::default(),
            knock: Burst::default(),
            knock_far: Burst::default(),
            rail: Modal::new(&RAIL_RATIOS, &RAIL_T60, &RAIL_LEVEL, 0x7_0003),
            body: Modal::new(&BODY_RATIOS, &BODY_T60, &BODY_LEVEL, 0x7_0004),
            pass_am: 0.5,
            roar: Svf::default(),
            rumble: Svf::default(),
            brown: Brown::default(),
            cabin: [OnePole::default(); 2],
            rpm: 0.3,
            fire: Phasor::default(),
            fire_count: 0,
            pulse_amp: 1.0,
            exhaust: Svf::default(),
            turbo: 0.0,
            turbo_ph: Phasor::default(),
            mesh: Phasor::default(),
            mesh_noise: Svf::default(),
            carrier: Phasor::default(),
            carrier_hz: 600.0,
            squeal_gate: SlowNoise::new(0x7_0005),
            squeal_wobble: SlowNoise::new(0x7_0006),
            squeal_ph: [Phasor::default(); 3],
            squeal_amp: 0.0,
            flange: Svf::default(),
            shoe: Svf::default(),
            brake_squeal: Phasor::default(),
            stick: SlowNoise::new(0x7_0007),
            brake_prev: 0.0,
            air: Burst::default(),
            air_hp: OnePole::default(),
            horn: 0.0,
            horn_osc: [Oscillator::new(0x7_0008), Oscillator::new(0x7_0009), Oscillator::new(0x7_000A)],
            horn_lp: Svf::default(),
            horn_bell: Svf::default(),
            breath: [Svf::default(); 3],
        }
    }

    fn snap(&mut self, x: &[f32], p: &TrainParams) {
        self.v = x[0] * p.max_kmh / 3.6;
        self.rpm = 0.35 + 0.65 * x[1];
        self.turbo = x[1];
        self.brake_prev = x[3];
        self.horn = x[4];
    }

    fn block(&mut self, x: &[f32], p: &TrainParams, out: &mut [f32]) {
        let (sr, n) = (self.sr, out.len());
        let dt = n as f32 / sr;
        let (throttle, curve, braking, horn_in) = (x[1], x[2], x[3], x[4]);
        // A train has mass: the game sets the speed, the sound follows within a fraction of a second.
        let v = slew(&mut self.v, x[0] * p.max_kmh / 3.6, 0.3, 0.3, dt);
        let vn = (v / 30.0).min(1.5);
        self.rebuild(p);
        let ds = v as f64 * dt as f64;
        self.clacks(ds, n, p);
        self.pos += ds;

        // Clack voices: a knock (contact), the rail's ring, the body's thump; far axles dull.
        let bright = 0.6 + 0.4 * vn.min(1.0);
        self.knock.set(FilterMode::BandPass, p.clack_hz * bright, 0.3, 0.07, sr);
        self.knock_far.set(FilterMode::LowPass, 450.0, 0.1, 0.09, sr);
        self.rail.set(p.clack_hz * bright, 0.5, 1.0, sr);
        self.body.set(if p.view < 0.5 { 72.0 } else { 95.0 }, 0.5, 1.0, sr);
        let (ring_gain, knock_gain, thump_gain) = (p.clack_ring * 0.5, 2.0, p.thump * 1.0);
        let onboard = p.view < 0.5;
        let cabin = hz_coef(1300.0, sr);

        // Rolling: the roar of wheel on rail, and onboard the body's low rumble.
        let roll_hz = p.roll_hz * (0.3 + 0.7 * vn.min(1.2));
        self.roar.set(FilterMode::LowPass, roll_hz, 0.15, sr);
        self.rumble.set(FilterMode::LowPass, 110.0, 0.3, sr);
        if p.view >= 0.5 {
            let prox = self.proximity(p);
            self.pass_am += (0.7 + 0.3 * prox - self.pass_am) * settle_coef(0.05, dt);
        } else {
            self.pass_am = 1.0;
        }
        let moving = smoothstep(v / 1.5);
        let roar_gain = p.roll_level * vn.powf(1.2) * 0.9 * self.pass_am;
        let rumble_gain = p.roll_level * moving * (0.15 + 0.85 * vn) * if p.view < 0.5 { 0.8 } else { 0.35 };

        // Drive.
        let drive = p.drive.round() as u32;
        let rpm = slew(&mut self.rpm, 0.35 + 0.65 * throttle, 2.5, 3.0, dt);
        let turbo = slew(&mut self.turbo, throttle, 4.0, 5.0, dt);
        let fire_hz = p.diesel_rpm * rpm / 60.0 * p.cylinders;
        self.exhaust.set(FilterMode::LowPass, (2.2 * fire_hz).clamp(120.0, 900.0), 0.45, sr);
        let diesel_gain = if drive == 0 { p.drive_level * (0.35 + 0.65 * throttle) * 0.3 } else { 0.0 };
        let turbo_hz = 1400.0 + 2600.0 * turbo;
        let turbo_gain = diesel_gain * 0.006 * turbo * turbo;
        let speed_frac = (v / (p.max_kmh / 3.6)).clamp(0.0, 1.2);
        let mesh_hz = p.motor_hz * speed_frac;
        self.mesh_noise.set(FilterMode::BandPass, mesh_hz.max(60.0), 0.85, sr);
        let electric_gain = if drive == 1 { p.drive_level * smoothstep(speed_frac * 8.0) * (0.3 + 0.7 * throttle) } else { 0.0 };
        // The inverter sings while it drives: asynchronous at a fixed carrier at low speed, then
        // synchronous steps that jump down as the motor's electrical frequency rises.
        let fe = 4.0 + 140.0 * speed_frac;
        let target = if speed_frac < 0.12 {
            620.0
        } else {
            let ratio = [21.0, 15.0, 9.0, 5.0, 3.0][((speed_frac - 0.12) / 0.18).floor().clamp(0.0, 4.0) as usize];
            fe * ratio
        };
        self.carrier_hz += (target - self.carrier_hz) * settle_coef(0.03, dt);
        let inverter_gain = if drive == 1 { p.drive_level * p.inverter * throttle * smoothstep(speed_frac * 30.0) * 0.12 } else { 0.0 };

        // Curve squeal: stick-slip bursts of narrow tones; flange rub under it.
        let gate = self.squeal_gate.advance(1.3, dt);
        let open = smoothstep((0.5 + 0.5 * gate) * 1.8 * curve.sqrt() - 0.45 + curve * 0.4);
        let squeal_target = p.squeal_level * curve * open * smoothstep(v / 4.0);
        let squeal_amp = slew(&mut self.squeal_amp, squeal_target, 0.08, 0.15, dt);
        let wob = 1.0 + 0.004 * self.squeal_wobble.advance(5.0, dt);
        let squeal_hz = [p.squeal_hz * wob, p.squeal_hz * 0.67 * wob, p.squeal_hz * 1.6 * wob];
        self.flange.set(FilterMode::BandPass, 2500.0, 0.5, sr);
        let flange_gain = p.squeal_level * curve * smoothstep(v / 6.0) * 0.12;

        // Brakes: shoe grind at speed, a squeal as it slows to a stop, air on apply and release.
        self.shoe.set(FilterMode::BandPass, 650.0 + 400.0 * vn.min(1.0), 0.35, sr);
        let shoe_gain = p.brake_level * braking * vn.min(1.0).sqrt() * 0.9;
        let near_stop = smoothstep(1.0 - v / 7.0) * smoothstep(v / 0.4);
        let stick = 0.5 + 0.5 * self.stick.advance(9.0 + 4.0 * v, dt);
        let brake_squeal_gain = p.brake_level * braking * near_stop * (0.4 + 0.6 * stick) * 0.12;
        let db = braking - self.brake_prev;
        self.brake_prev = braking;
        if db.abs() > 1e-4 {
            self.air.kick(p.air_level * db.abs() * if db > 0.0 { 0.6 } else { 1.2 });
        }
        self.air.set(FilterMode::BandPass, 4500.0, 0.2, 1.4, sr);
        let air_hp = hz_coef(1800.0, sr);

        // Horn.
        let horn = slew(&mut self.horn, horn_in, 0.05, 0.12, dt);
        let kind = (p.horn_kind.round() as usize).min(2);
        let scoop = (p.horn_pitch / 12.0).exp2() * (-0.06 * (1.0 - horn.min(1.0))).exp2();
        self.horn_lp.set(FilterMode::LowPass, if kind == 2 { 3500.0 } else { 6000.0 }, 0.1, sr);
        self.horn_bell.set(FilterMode::BandPass, 1100.0, 0.4, sr);
        for (b, (hz, _)) in self.breath.iter_mut().zip(HORNS[kind]) {
            b.set(FilterMode::BandPass, (hz * scoop).max(20.0), 0.9, sr);
        }
        // Onboard, the horn is on the locomotive up ahead and comes through the car body.
        let horn_gain = p.horn_level * horn * if onboard { 0.16 } else { 0.3 };

        let gain = p.gain;
        let mesh_inc = mesh_hz / sr;
        let carrier_inc = self.carrier_hz / sr;
        let fire_inc = fire_hz / sr;
        let turbo_inc = turbo_hz / sr;
        let brake_squeal_inc = p.squeal_hz * 0.55 / sr;
        for (i, o) in out.iter_mut().enumerate() {
            let w = self.noise.white();
            // Clack.
            let (xn, xf) = (self.near.x[i], self.far.x[i]);
            if xn != 0.0 {
                self.knock.kick(xn);
            }
            if xf != 0.0 {
                self.knock_far.kick(xf);
            }
            let ring = self.rail.tick(xn, 0.23, 0.6, sr);
            let thump = self.body.tick(xn + 0.6 * xf, 0.23, 0.4, sr);
            let clack = self.knock.tick(w) * knock_gain + self.knock_far.tick(w) * knock_gain * 1.5 + ring * ring_gain + thump * thump_gain;
            // Rolling.
            let pink = self.noise.pink();
            let roll = self.roar.tick(pink) * roar_gain * 2.5 + self.rumble.tick(self.brown.tick(w)) * rumble_gain;
            // Drive.
            let mut drive_s = 0.0;
            if diesel_gain > 0.0 {
                if self.fire.tick(fire_inc) {
                    self.fire_count = self.fire_count.wrapping_add(1);
                    // One cylinder a little weak: the once-per-rev lope of a big diesel.
                    let lope = if self.fire_count.is_multiple_of(p.cylinders.max(1.0) as u32) { 0.75 } else { 1.0 };
                    self.pulse_amp = lope * self.rng.range(0.85, 1.15);
                }
                let ph = self.fire.phase;
                let pulse = if ph < 0.35 { (core::f32::consts::PI * ph / 0.35).sin() } else { 0.0 } * self.pulse_amp - 0.22;
                self.turbo_ph.tick(turbo_inc);
                drive_s += soft_clip(self.exhaust.tick(pulse * (0.5 + 2.0 * pink.abs()) + 0.3 * pink) * 2.0) * diesel_gain + self.turbo_ph.sin() * turbo_gain;
            }
            if electric_gain > 0.0 || inverter_gain > 0.0 {
                self.mesh.tick(mesh_inc);
                self.carrier.tick(carrier_inc);
                let m = self.mesh.phase * TAU;
                let whine = m.sin() + 0.35 * (2.0 * m).sin();
                let c = self.carrier.phase * TAU;
                drive_s += (whine * 0.15 + self.mesh_noise.tick(w) * 0.25) * electric_gain + (c.sin() + 0.3 * (3.0 * c).sin()) * inverter_gain;
            }
            // Squeal and brakes.
            let mut sq = 0.0;
            if squeal_amp > 1e-4 {
                for ((ph, hz), lvl) in self.squeal_ph.iter_mut().zip(squeal_hz).zip([1.0, 0.4, 0.3]) {
                    ph.tick(hz / sr);
                    sq += ph.sin() * lvl;
                }
                sq *= squeal_amp * 0.11;
            }
            sq += self.flange.tick(w) * flange_gain;
            let mut brake = self.shoe.tick(pink) * shoe_gain;
            if brake_squeal_gain > 1e-5 {
                self.brake_squeal.tick(brake_squeal_inc);
                brake += self.brake_squeal.sin() * brake_squeal_gain;
            }
            brake += self.air_hp.hp(self.air.tick(w), air_hp) * 0.5;
            // Horn.
            let mut horn_s = 0.0;
            if horn_gain > 1e-5 {
                for ((osc, (hz, lvl)), br) in self.horn_osc.iter_mut().zip(HORNS[kind]).zip(self.breath.iter_mut()) {
                    if lvl == 0.0 {
                        continue;
                    }
                    let inc = hz * scoop / sr;
                    horn_s += if kind == 2 {
                        osc.next(Waveform::Triangle, inc, 0.5) * 0.6 + br.tick(w) * 3.0
                    } else {
                        osc.next(Waveform::Pulse, inc, 0.3) * 0.6
                    } * lvl;
                }
                let h = self.horn_lp.tick(horn_s);
                horn_s = soft_clip((h + self.horn_bell.tick(h) * 0.8) * 1.2) * horn_gain;
            }
            let mut track = (clack + roll) * if onboard { 1.0 } else { 1.5 };
            if onboard {
                // The car body: the cabin hears the wheels through the floor, under 1.3 kHz.
                let t = self.cabin[0].lp(track, cabin);
                track = self.cabin[1].lp(t, cabin);
            }
            *o = (track + drive_s + sq + brake + horn_s) * gain;
        }
        self.rail.end_block();
        self.body.end_block();
    }
}

// ---------------------------------------------------------------------------------------------
// Conveyor
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A conveyor belt: the motor's hum and gearbox whine (louder under load), the belt's
    /// rumble, idler rollers ticking once per turn each, the belt splice thumping once per loop,
    /// and items knocking as they ride over the rollers.
    ///
    /// Measured (`target/refs/machines/conveyor`): a drive's hum is a comb of lines 32 Hz apart
    /// peaking at 380-410 Hz; a heavy belt's rumble sits at 90-310 Hz (centroid 370 Hz); the
    /// splice ticks once per loop (5.4 s on a small belt).
    ConveyorParams / ConveyorParamId {
        max_mps: "belt/max_mps" = 1.2, exp(0.1, 6.0);
        length_m: "belt/length_m" = 6.5, lin(1.0, 60.0);
        rumble: "belt/rumble" = 0.6, UNIT;
        rumble_hz: "belt/rumble_hz" = 260.0, exp(60.0, 1500.0);
        splice: "belt/splice" = 0.5, UNIT;
        roller_m: "rollers/diameter_m" = 0.09, lin(0.03, 0.3);
        rollers: "rollers/level" = 0.4, UNIT;
        roller_hz: "rollers/hz" = 1400.0, exp(300.0, 6000.0);
        motor_hz: "motor/rotation_hz" = 32.0, exp(5.0, 120.0);
        motor: "motor/level" = 0.5, UNIT;
        hum_hz: "motor/hum_hz" = 390.0, exp(80.0, 2000.0);
        gear: "motor/gear_whine" = 0.25, UNIT;
        items: "items/level" = 0.4, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

const ROLLERS: usize = 6;
/// Shaft harmonics in the motor hum, and their (fixed, unrelated) phases.
const HUM_PARTIALS: usize = 24;
const HUM_PHASE: [f32; HUM_PARTIALS] = [0.0, 2.1, 4.4, 1.3, 5.8, 3.0, 0.7, 4.9, 2.6, 5.3, 1.8, 3.7, 0.4, 6.0, 2.9, 4.1, 1.1, 5.5, 3.3, 0.2, 4.7, 2.4, 5.9, 1.6];
/// Each roller turns at a slightly different rate (wear, slip), so their ticks drift.
const ROLLER_RATE: [f32; ROLLERS] = [1.0, 1.013, 0.987, 1.027, 0.971, 1.041];

pub struct Conveyor {
    sr: f32,
    spin: f32,
    noise: Noise,
    rng: Rng,
    shaft: Phasor,
    hum: [Svf; 1],
    mesh: Phasor,
    brown: Brown,
    rumble: Svf,
    rumble_bp: Svf,
    rollers: [Phasor; ROLLERS],
    roller: Click,
    splice: Phasor,
    splice_click: Click,
    drift: [f32; HUM_PARTIALS],
    items: Click,
    hits: Hits,
}

impl Generator for Conveyor {
    type P = ConveyorParams;
    const NAME: &'static str = "conveyor";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "Conveyor belt or roller line: motor hum and gear whine with load, belt rumble, rollers ticking, \
        the splice thumping once a loop, items knocking.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.0, doc: "Belt speed, 0 stopped .. 1 = belt/max_mps; the drive spins up and down" },
        InputSpec { name: "load", default: 0.3, doc: "How loaded the belt is: motor strain, rumble, items knocking" },
    ];

    fn presets() -> Vec<(&'static str, ConveyorParams)> {
        let d = ConveyorParams::default();
        vec![
            ("Baggage belt", ConveyorParams { max_mps: 0.6, length_m: 25.0, rumble_hz: 180.0, roller_m: 0.12, rollers: 0.25, roller_hz: 800.0, splice: 0.3, motor_hz: 25.0, items: 0.7, ..d }),
            ("Mine conveyor", ConveyorParams { max_mps: 3.0, length_m: 60.0, rumble: 0.9, rumble_hz: 130.0, roller_m: 0.15, rollers: 0.25, roller_hz: 650.0, splice: 0.7, motor_hz: 25.0, motor: 0.7, hum_hz: 250.0, gear: 0.4, ..d }),
            ("Roller line", ConveyorParams { rumble: 0.25, rollers: 0.9, roller_hz: 1800.0, roller_m: 0.05, splice: 0.0, items: 0.6, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        Conveyor {
            sr,
            spin: 0.0,
            noise: Noise::new(mix_seed(0x7_1001)),
            rng: Rng::new(mix_seed(0x7_1002)),
            shaft: Phasor::default(),
            hum: [Svf::default(); 1],
            mesh: Phasor::default(),
            brown: Brown::default(),
            rumble: Svf::default(),
            rumble_bp: Svf::default(),
            rollers: core::array::from_fn(|k| Phasor { phase: k as f32 / ROLLERS as f32 }),
            roller: Click::new(&[1.0, 2.62, 4.95, 7.6], &[0.05, 0.035, 0.025, 0.015], &[1.0, 0.6, 0.35, 0.2], 0x7_1003),
            splice: Phasor { phase: 0.3 },
            splice_click: Click::new(&[1.0, 1.9, 3.1], &[0.12, 0.07, 0.04], &[1.0, 0.5, 0.25], 0x7_1004),
            drift: [0.0; HUM_PARTIALS],
            items: Click::new(&[1.0, 1.45, 2.3, 3.5], &[0.09, 0.06, 0.04, 0.025], &[1.0, 0.7, 0.4, 0.2], 0x7_1005),
            hits: Hits::default(),
        }
    }

    fn snap(&mut self, x: &[f32], _p: &ConveyorParams) {
        self.spin = x[0];
    }

    fn block(&mut self, x: &[f32], p: &ConveyorParams, out: &mut [f32]) {
        let (sr, n) = (self.sr, out.len());
        let dt = n as f32 / sr;
        let spin = slew(&mut self.spin, x[0], 1.5, 2.5, dt);
        let load = x[1];
        let run = smoothstep(spin * 6.0);
        let v = spin * p.max_mps;
        // Motor: a comb of shaft harmonics shaped by the housing; strains louder with load.
        let shaft_hz = p.motor_hz * spin;
        // The hum: shaft harmonics with fixed, unrelated phases (a smooth comb, not a click train),
        // loudest around `motor/hum_hz`.
        // Each line wanders a little (slip, load ripple), so the hum is not a rigid click train.
        for d in self.drift.iter_mut() {
            *d = (*d + 0.08 * self.rng.next_bipolar()).rem_euclid(TAU);
        }
        let mut comb = [0.0f32; HUM_PARTIALS];
        for (k, a) in comb.iter_mut().enumerate() {
            let f = shaft_hz * (k + 1) as f32;
            let oct = (f.max(1.0) / p.hum_hz).log2();
            *a = if f < sr * 0.45 { 0.25 * (-(k as f32) / 14.0).exp() + (-(oct * oct) * 1.5).exp() } else { 0.0 };
        }
        self.hum[0].set(FilterMode::BandPass, p.hum_hz, 0.3, sr);
        let motor_gain = p.motor * run * (0.45 + 0.55 * load) * 1.1;
        let mesh_hz = shaft_hz * 23.0;
        let gear_gain = p.gear * run * spin * (0.4 + 0.6 * load) * 0.05;
        // Belt rumble.
        self.rumble.set(FilterMode::LowPass, p.rumble_hz * (0.6 + 0.4 * spin), 0.2, sr);
        self.rumble_bp.set(FilterMode::BandPass, p.rumble_hz * 1.2, 0.3, sr);
        let rumble_gain = p.rumble * spin.powf(0.8) * (0.6 + 0.4 * load) * 0.6;
        // Rollers, splice and items.
        let roller_rev = v / (core::f32::consts::PI * p.roller_m);
        self.roller.set(p.roller_hz, 0.5, p.roller_hz * 1.5, 0.006, sr);
        self.splice_click.set(140.0, 1.0, 900.0, 0.03, sr);
        self.items.set(260.0, 1.0, 1200.0, 0.02, sr);
        let roller_amp = p.rollers * (v / 1.5).clamp(0.0, 1.0).sqrt() * 0.25;
        let splice_amp = p.splice * (0.4 + 0.6 * (v / 1.5).min(1.0)) * (0.7 + 0.3 * load);
        let item_p = p.items * load * (0.5 + 2.5 * spin) / sr;
        self.hits.clear();
        let (shaft_inc, mesh_inc, splice_inc) = (shaft_hz / sr, mesh_hz / sr, v / p.length_m / sr);
        let gain = p.gain * 0.8;
        for (i, o) in out.iter_mut().enumerate() {
            let w = self.noise.white();
            self.shaft.tick(shaft_inc);
            let ph = self.shaft.phase * TAU;
            let mut hum = self.hum[0].tick(w) * 0.15;
            for (k, a) in comb.iter().enumerate() {
                if *a > 0.0 {
                    hum += a * ((k + 1) as f32 * ph + HUM_PHASE[k] + self.drift[k]).sin() * 0.12;
                }
            }
            self.mesh.tick(mesh_inc);
            let whine = self.mesh.sin() + 0.3 * (2.0 * TAU * self.mesh.phase).sin();
            let b = self.brown.tick(w);
            let rumble = self.rumble.tick(b) * 1.5 + self.rumble_bp.tick(self.noise.pink()) * 0.6;
            let mut r = 0.0;
            for (k, roll) in self.rollers.iter_mut().enumerate() {
                if roll.tick(roller_rev * ROLLER_RATE[k] / sr) {
                    r += roller_amp * self.rng.range(0.4, 1.0);
                }
            }
            let mut s = 0.0;
            if self.splice.tick(splice_inc) {
                s = splice_amp;
            }
            let item = if self.rng.next_f32() < item_p { p.items * self.rng.next_f32().powf(2.0) * 0.6 } else { 0.0 };
            self.hits.x[i] = item;
            let clicks = self.roller.tick(r, w, 0.6, 0.6, sr) + self.splice_click.tick(s, w, 1.0, 0.4, sr) * 0.8 + self.items.tick(item, w, 1.2, 0.8, sr);
            *o = (hum * motor_gain + whine * gear_gain + rumble * rumble_gain + clicks) * gain;
        }
        self.roller.end_block();
        self.splice_click.end_block();
        self.items.end_block();
    }
}

// ---------------------------------------------------------------------------------------------
// Press
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A press working in a steady cycle. Each stroke: the clutch clanks, the ram slides down,
    /// lands (a thud and the ring of the die, weighted by `force`), slides back up and an air
    /// blast clears the part. Under it the flywheel hums, sagging a little at each hit, or a
    /// hydraulic pump whines and labours while it presses.
    ///
    /// Measured (`target/refs/machines/press`): a hydraulic press pump is a strong line at 442 Hz
    /// with harmonics, centroid 2.4 kHz from the oil's hiss; its strokes 12 s apart.
    PressParams / PressParamId {
        min_spm: "press/min_strokes_per_minute" = 12.0, exp(1.0, 60.0);
        max_spm: "press/max_strokes_per_minute" = 60.0, exp(2.0, 200.0);
        land: "press/stroke_at" = 0.45, lin(0.15, 0.8);
        impact: "impact/level" = 0.8, UNIT;
        ring_hz: "impact/ring_hz" = 620.0, exp(150.0, 4000.0);
        ring: "impact/ring" = 1.0, exp(0.2, 4.0);
        thud_hz: "impact/thud_hz" = 55.0, exp(25.0, 300.0);
        clutch: "clutch/level" = 0.5, UNIT;
        slide: "slide/level" = 0.3, UNIT;
        air: "air/level" = 0.4, UNIT;
        flywheel: "flywheel/level" = 0.5, UNIT;
        flywheel_hz: "flywheel/hz" = 28.0, exp(5.0, 120.0);
        pump: "pump/level" = 0.0, UNIT;
        pump_hz: "pump/hz" = 442.0, exp(80.0, 2000.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

const DIE_RATIOS: [f32; 6] = [1.0, 1.58, 2.31, 3.17, 4.4, 5.9];
const DIE_T60: [f32; 6] = [0.7, 0.5, 0.38, 0.27, 0.18, 0.12];
const DIE_LEVEL: [f32; 6] = [1.0, 0.8, 0.7, 0.5, 0.35, 0.25];

pub struct Press {
    sr: f32,
    noise: Noise,
    rng: Rng,
    cycle: f32,
    rate: f32,
    die: Modal,
    thud: Modal,
    crunch: Burst,
    clutch: Click,
    air: Burst,
    air_hp: OnePole,
    slide: Svf,
    fly: Phasor,
    fly_lp: Svf,
    sag: f32,
    pump: Phasor,
    pump_bp: Svf,
    press_env: f32,
}

impl Press {
    /// Crossed `at` (a cycle fraction) going from `a` to `b`?
    #[inline]
    fn crossed(a: f32, b: f32, at: f32) -> bool {
        if b >= a {
            a < at && b >= at
        } else {
            a < at || b >= at
        }
    }
}

impl Generator for Press {
    type P = PressParams;
    const NAME: &'static str = "press";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "Stamping, forging or hydraulic press in a steady cycle: clutch, ram, a heavy landing with the die \
        ringing, return and air blast, over a flywheel hum or a labouring pump.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "rate", default: 0.5, doc: "Strokes per minute, 0 stopped (idling) .. 1 = press/max_strokes_per_minute" },
        InputSpec { name: "force", default: 0.7, doc: "How hard it lands: a light stamp .. a full-tonnage blow" },
    ];

    fn presets() -> Vec<(&'static str, PressParams)> {
        let d = PressParams::default();
        vec![
            (
                "Hydraulic press",
                PressParams { min_spm: 2.0, max_spm: 10.0, land: 0.5, impact: 0.35, ring_hz: 420.0, ring: 0.6, thud_hz: 45.0, clutch: 0.0, slide: 0.5, air: 0.6, flywheel: 0.0, pump: 0.8, ..d },
            ),
            ("Drop forge", PressParams { max_spm: 40.0, land: 0.3, impact: 1.0, ring_hz: 380.0, ring: 1.6, thud_hz: 38.0, clutch: 0.2, slide: 0.1, air: 0.0, flywheel: 0.3, ..d }),
            ("Desk stamp", PressParams { min_spm: 20.0, max_spm: 120.0, impact: 0.6, ring_hz: 2400.0, ring: 0.25, thud_hz: 140.0, clutch: 0.2, slide: 0.15, air: 0.0, flywheel: 0.0, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        Press {
            sr,
            noise: Noise::new(mix_seed(0x7_2001)),
            rng: Rng::new(mix_seed(0x7_2002)),
            cycle: 0.9,
            rate: 0.5,
            die: Modal::new(&DIE_RATIOS, &DIE_T60, &DIE_LEVEL, 0x7_2003),
            thud: Modal::new(&[1.0, 1.52, 2.4], &[0.4, 0.25, 0.12], &[1.0, 0.5, 0.25], 0x7_2004),
            crunch: Burst::default(),
            clutch: Click::new(&[1.0, 1.7, 2.9, 4.2], &[0.12, 0.08, 0.05, 0.03], &[1.0, 0.7, 0.5, 0.3], 0x7_2005),
            air: Burst::default(),
            air_hp: OnePole::default(),
            slide: Svf::default(),
            fly: Phasor::default(),
            fly_lp: Svf::default(),
            sag: 0.0,
            pump: Phasor::default(),
            pump_bp: Svf::default(),
            press_env: 0.0,
        }
    }

    fn snap(&mut self, x: &[f32], _p: &PressParams) {
        self.rate = x[0];
    }

    fn block(&mut self, x: &[f32], p: &PressParams, out: &mut [f32]) {
        let (sr, n) = (self.sr, out.len());
        let dt = n as f32 / sr;
        let rate = slew(&mut self.rate, x[0], 0.5, 0.5, dt);
        let force = x[1];
        let spm = if rate > 0.01 { p.min_spm * (p.max_spm / p.min_spm).max(1.0).powf(rate) } else { 0.0 };
        let inc = spm / 60.0 / sr;
        self.die.set(p.ring_hz, 0.5, p.ring, sr);
        self.thud.set(p.thud_hz, 0.5, 1.0, sr);
        self.crunch.set(FilterMode::BandPass, 1100.0, 0.2, 0.12, sr);
        self.clutch.set(1500.0, 1.0, 2500.0, 0.01, sr);
        self.air.set(FilterMode::BandPass, 5500.0, 0.15, 0.35, sr);
        self.fly_lp.set(FilterMode::LowPass, (p.flywheel_hz * 8.0).max(180.0), 0.2, sr);
        self.pump_bp.set(FilterMode::BandPass, p.pump_hz * 2.0, 0.3, sr);
        let air_hp = hz_coef(1500.0, sr);
        let land = p.land;
        let gain = p.gain * 0.9;
        // The flywheel sags at each blow and recovers; the pump drops in pitch while pressing.
        self.sag *= (-dt / 0.6).exp();
        let fly_inc = p.flywheel_hz * (1.0 - self.sag) / sr;
        let fly_gain = p.flywheel * 0.5;
        for o in out.iter_mut() {
            let w = self.noise.white();
            let a = self.cycle;
            self.cycle = (self.cycle + inc).fract();
            let b = self.cycle;
            let (mut die_x, mut thud_x, mut clutch_x) = (0.0, 0.0, 0.0);
            if inc > 0.0 {
                if Press::crossed(a, b, 0.02) {
                    clutch_x = p.clutch * self.rng.range(0.8, 1.0) * 0.5;
                }
                if Press::crossed(a, b, land) {
                    let hit = (0.3 + 0.7 * force) * self.rng.range(0.9, 1.1);
                    die_x = p.impact * hit * 0.22;
                    thud_x = p.impact * hit * 0.5;
                    self.crunch.kick(p.impact * hit * 0.8);
                    self.sag = (self.sag + 0.04 * force).min(0.1);
                }
                if Press::crossed(a, b, (land + 0.38).min(0.97)) {
                    self.air.kick(p.air * 0.8);
                }
            }
            // The ram sliding: down before the landing, back up after it.
            let travel = if inc > 0.0 {
                if b < land {
                    (b / land * core::f32::consts::PI).sin()
                } else {
                    ((b - land) / (1.0 - land) * core::f32::consts::PI).sin() * 0.7
                }
            } else {
                0.0
            };
            let pressing = if inc > 0.0 { smoothstep(1.0 - ((b - land) / 0.12).abs()) } else { 0.0 };
            self.press_env += (pressing - self.press_env) * 0.0005;
            self.slide.set(FilterMode::BandPass, 700.0 + 500.0 * travel, 0.3, sr);
            let slide = self.slide.tick(w) * travel * p.slide * 0.25;
            let impact = self.die.tick(die_x, 0.23, 0.5, sr) + self.thud.tick(thud_x, 0.23, 0.3, sr) + self.crunch.tick(w) * 0.6;
            let clutch = self.clutch.tick(clutch_x, w, 0.5, 0.5, sr);
            let air = self.air_hp.hp(self.air.tick(w), air_hp) * 0.6;
            self.fly.tick(fly_inc);
            let f = self.fly.phase * TAU;
            // The flywheel: a low turning hum under the rumble of its bearings and belt.
            let fly = ((f.sin() + 0.5 * (2.0 * f).sin() + 0.25 * (3.0 * f).sin()) * 0.35 + 1.4 * self.fly_lp.tick(self.noise.pink())) * fly_gain;
            let mut pump = 0.0;
            if p.pump > 0.0 {
                let load = 0.3 + 0.7 * self.press_env.max(pressing * force);
                self.pump.tick(p.pump_hz * (1.0 - 0.05 * load) / sr);
                let q = self.pump.phase * TAU;
                pump = ((q.sin() + 0.5 * (2.0 * q).sin() + 0.3 * (3.0 * q).sin() + 0.15 * (4.0 * q).sin()) * 0.5 + self.pump_bp.tick(w) * 0.6 + self.noise.pink() * 0.9) * p.pump * load * 0.3;
            }
            *o = (impact + clutch + air + slide + fly + pump) * gain;
        }
        self.die.end_block();
        self.thud.end_block();
        self.clutch.end_block();
    }
}

// ---------------------------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A clock's escapement. Each beat is a little cluster of clicks (`escapement/clicks`,
    /// `escapement/spacing_ms` apart: the tooth landing on the pallet, then the lock), beats
    /// alternate tick and tock (the two pallets: a small timing asymmetry and a pitch offset),
    /// and the case rings under them.
    ///
    /// Measured (`target/refs/machines/clock`): a wall clock at 1 beat/s, three clicks 24-28 ms
    /// apart (the second loudest), its case ringing at 360 and 560 Hz for 100-200 ms; an alarm
    /// clock at 5 beats/s alternating 212/188 ms, very bright (peaks 5.2-8 kHz, a click fading
    /// 20 dB in 7 ms) with a second click 24 ms later and 20-35 dB down; a mantel clock at 2.6
    /// beats/s, two clicks 30-50 ms apart, mostly its wooden case (316-507 Hz).
    ClockParams / ClockParamId {
        beats: "escapement/beats_per_second" = 1.0, exp(0.25, 10.0);
        beat_error: "escapement/beat_error" = 0.03, lin(0.0, 0.3);
        clicks: "escapement/clicks" = 3.0, int(1, 4);
        spacing_ms: "escapement/spacing_ms" = 26.0, lin(3.0, 80.0);
        after: "escapement/after" = 0.8, UNIT;
        tock: "escapement/tock_semitones" = -1.5, lin(-12.0, 12.0);
        tick_hz: "tick/hz" = 3800.0, exp(500.0, 12000.0);
        tick_ring: "tick/ring" = 1.0, exp(0.2, 5.0);
        case_hz: "case/hz" = 360.0, exp(60.0, 3000.0);
        case_level: "case/level" = 0.6, UNIT;
        case_ring: "case/ring" = 1.0, exp(0.2, 5.0);
        movement: "movement/level" = 0.3, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

const TICK_RATIOS: [f32; 7] = [0.87, 1.05, 1.12, 1.36, 1.42, 2.06, 2.14];
const TICK_T60: [f32; 7] = [0.035, 0.03, 0.03, 0.025, 0.025, 0.02, 0.02];
const TICK_LEVEL: [f32; 7] = [0.4, 0.55, 0.55, 0.8, 0.85, 1.0, 0.9];
const CASE_RATIOS: [f32; 4] = [1.0, 1.55, 2.3, 3.4];
const CASE_T60: [f32; 4] = [0.25, 0.18, 0.12, 0.08];
const CASE_LEVEL: [f32; 4] = [1.0, 0.7, 0.4, 0.3];
const MAX_PENDING: usize = 8;

pub struct Clock {
    sr: f32,
    noise: Noise,
    rng: Rng,
    /// Beats until the next beat; which pallet it is.
    wait: f32,
    tock: bool,
    /// Sub-clicks waiting: (samples left, amplitude, tock).
    pending: [(f32, f32, bool); MAX_PENDING],
    tick: Click,
    tock_click: Click,
    case: Modal,
    /// The case's noisy body: wood or tin shaken by each click.
    body: Burst,
    /// The running movement's faint whir between ticks.
    whir: Svf,
}

impl Clock {
    fn schedule(&mut self, delay: f32, amp: f32, tock: bool) {
        if let Some(slot) = self.pending.iter_mut().find(|q| q.1 == 0.0) {
            *slot = (delay, amp, tock);
        }
    }
}

impl Generator for Clock {
    type P = ClockParams;
    const NAME: &'static str = "clock";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "Clock escapement: a steady tick-tock of little click clusters over the ringing case. \
        Wall, alarm, mantel, grandfather clock or pocket watch.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.5, doc: "Tempo: 0 stopped, 0.5 its own rate, 1 twice as fast" },
        InputSpec { name: "tension", default: 1.0, doc: "Spring wind: 1 fully wound; running down it ticks softer and less evenly, 0 stops" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.1;

    fn presets() -> Vec<(&'static str, ClockParams)> {
        let d = ClockParams::default();
        vec![
            ("Alarm clock", ClockParams { beats: 5.0, beat_error: 0.06, clicks: 2.0, spacing_ms: 24.0, after: 0.12, tock: -1.0, tick_hz: 5200.0, tick_ring: 0.7, case_hz: 1100.0, case_level: 0.25, case_ring: 0.6, ..d }),
            ("Mantel clock", ClockParams { beats: 2.6, beat_error: 0.04, clicks: 2.0, spacing_ms: 40.0, after: 0.7, tick_hz: 2500.0, case_hz: 410.0, case_level: 0.9, case_ring: 1.4, ..d }),
            ("Grandfather clock", ClockParams { beats: 1.0, beat_error: 0.02, clicks: 2.0, spacing_ms: 35.0, after: 0.5, tock: -2.5, tick_hz: 2000.0, tick_ring: 1.4, case_hz: 180.0, case_level: 0.9, case_ring: 2.5, ..d }),
            ("Pocket watch", ClockParams { beats: 5.0, beat_error: 0.02, clicks: 2.0, spacing_ms: 8.0, after: 0.4, tock: 0.5, tick_hz: 7000.0, tick_ring: 0.5, case_hz: 2600.0, case_level: 0.25, case_ring: 0.5, gain: 0.8, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        Clock {
            sr,
            noise: Noise::new(mix_seed(0x7_3001)),
            rng: Rng::new(mix_seed(0x7_3002)),
            wait: 0.0,
            tock: false,
            pending: [(0.0, 0.0, false); MAX_PENDING],
            tick: Click::new(&TICK_RATIOS, &TICK_T60, &TICK_LEVEL, 0x7_3003),
            tock_click: Click::new(&TICK_RATIOS, &TICK_T60, &TICK_LEVEL, 0x7_3004),
            case: Modal::new(&CASE_RATIOS, &CASE_T60, &CASE_LEVEL, 0x7_3005),
            body: Burst::default(),
            whir: Svf::default(),
        }
    }

    fn block(&mut self, x: &[f32], p: &ClockParams, out: &mut [f32]) {
        let sr = self.sr;
        let (speed, tension) = (x[0], x[1]);
        // A clock keeps its rate until the spring is nearly spent, then stumbles and stops.
        let wound = smoothstep(tension * 6.0);
        let tempo = p.beats * 2.0 * speed * wound;
        let level = 0.35 + 0.65 * tension;
        let unsteady = 0.25 * (1.0 - smoothstep(tension * 4.0));
        self.tick.set(p.tick_hz, p.tick_ring, p.tick_hz * 1.4, 0.004 * p.tick_ring, sr);
        let tock_hz = p.tick_hz * (p.tock / 12.0).exp2();
        self.tock_click.set(tock_hz, p.tick_ring, tock_hz * 1.4, 0.004 * p.tick_ring, sr);
        self.case.set(p.case_hz, 0.5, p.case_ring, sr);
        self.body.set(FilterMode::BandPass, p.case_hz * 1.3, 0.35, 0.16 * p.case_ring, sr);
        self.whir.set(FilterMode::BandPass, p.tick_hz * 0.5, 0.1, sr);
        let whir_gain = p.movement * 0.03 * smoothstep(tempo * 4.0);
        let clicks = p.clicks as usize;
        let spacing = p.spacing_ms * 1e-3 * sr;
        let gain = p.gain * 0.45;
        for o in out.iter_mut() {
            if tempo > 0.01 {
                // Counted in beats, so a change of tempo applies at once.
                self.wait -= tempo / sr;
                if self.wait <= 0.0 {
                    // Tick and tock take slightly different times (the beat error).
                    let e = if self.tock { 1.0 - p.beat_error } else { 1.0 + p.beat_error };
                    let jitter = 1.0 + unsteady * self.rng.next_bipolar();
                    self.wait = (self.wait + e * jitter).max(0.0);
                    let tock = self.tock;
                    self.tock = !self.tock;
                    let amp = level * self.rng.range(0.92, 1.0);
                    for k in 0..clicks {
                        let lvl = if k == 0 { 1.0 } else { p.after * [1.0, 0.45, 0.25][k - 1] };
                        let at = k as f32 * spacing * self.rng.range(0.9, 1.1);
                        self.schedule(at, amp * lvl, tock);
                    }
                }
            }
            let (mut xt, mut xk) = (0.0, 0.0);
            for q in self.pending.iter_mut() {
                if q.1 != 0.0 {
                    q.0 -= 1.0;
                    if q.0 <= 0.0 {
                        if q.2 {
                            xk += q.1;
                        } else {
                            xt += q.1;
                        }
                        q.1 = 0.0;
                    }
                }
            }
            let w = self.noise.white();
            let clicks_s = self.tick.tick(xt * 0.5, w, 1.2, 0.4, sr) + self.tock_click.tick(xk * 0.45, w, 1.2, 0.4, sr);
            if xt + xk != 0.0 {
                self.body.kick((xt + xk) * p.case_level);
            }
            let case = self.case.tick((xt + xk) * p.case_level * 0.2, 0.23, 0.3, sr) + self.body.tick(w) * 1.5 + self.whir.tick(self.noise.pink()) * whir_gain;
            *o = (clicks_s + case) * gain;
        }
        self.tick.end_block();
        self.tock_click.end_block();
        self.case.end_block();
    }
}

// ---------------------------------------------------------------------------------------------
// Gears
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A gear train turning: each stage's teeth mesh as clicks (heard one by one when slow, a
    /// buzzing whine when fast), every tooth with its own wear so a revolution repeats; a pawl
    /// clicking over a ratchet (with its little bounce); and the whir of a fast governor.
    /// `gears/ratio` above 1 speeds each stage up (clockwork, a wind-up toy's governor), below 1
    /// slows it (a winch, a mill). `gears/grouping` bunches the teeth to one side, which gives
    /// the lope of a toy's mechanism.
    ///
    /// Measured (`target/refs/machines/gears`, `musicbox`): a wind-up toy running is a repeating
    /// three-click pattern (gaps of 30, 30 and 60 ms) under a 4-10 kHz whir; winding a spring is a
    /// ratchet click every 100-230 ms, each with a faint bounce 25 ms later.
    GearsParams / GearsParamId {
        max_rps: "gears/max_rev_per_second" = 1.5, exp(0.05, 50.0);
        teeth: "gears/teeth" = 12.0, int(3, 64);
        stages: "gears/stages" = 2.0, int(1, 4);
        ratio: "gears/ratio" = 3.0, lin(0.2, 8.0);
        grouping: "gears/grouping" = 0.0, UNIT;
        wear: "gears/wear" = 0.4, UNIT;
        hz: "gears/hz" = 2600.0, exp(150.0, 9000.0);
        ring: "gears/ring" = 1.0, exp(0.2, 5.0);
        level: "gears/level" = 0.7, UNIT;
        ratchet_teeth: "ratchet/teeth" = 16.0, int(4, 60);
        ratchet: "ratchet/level" = 0.0, UNIT;
        whir: "whir/level" = 0.15, UNIT;
        whir_hz: "whir/hz" = 5000.0, exp(500.0, 12000.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

const GEAR_STAGES: usize = 4;
const GEAR_RATIOS: [f32; 5] = [1.0, 1.48, 2.21, 3.05, 4.4];
const GEAR_T60: [f32; 5] = [0.06, 0.045, 0.035, 0.025, 0.02];
const GEAR_LEVEL: [f32; 5] = [1.0, 0.8, 0.6, 0.45, 0.3];

pub struct Gears {
    sr: f32,
    noise: Noise,
    rng: Rng,
    spin: f32,
    /// Per stage: revolution phase and the next tooth to sound.
    phase: [f32; GEAR_STAGES],
    next: [u32; GEAR_STAGES],
    click: [Click; GEAR_STAGES],
    ratchet_phase: f32,
    ratchet: Click,
    bounce: (f32, f32),
    whir: Svf,
    whir_env: f32,
}

impl Gears {
    /// Where tooth `k` of `teeth` sits on the wheel (0..1), with teeth bunched by `grouping`.
    #[inline]
    fn tooth_at(k: u32, teeth: u32, grouping: f32) -> f32 {
        let even = k as f32 / teeth as f32;
        even * (1.0 - grouping / teeth as f32)
    }
}

impl Generator for Gears {
    type P = GearsParams;
    const NAME: &'static str = "gears";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "Gear train: teeth meshing (separate clicks when slow, a whine when fast, each tooth worn its own way), \
        a ratchet pawl, a governor's whir. Clockwork, iron gates, winches, wind-up toys.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.0, doc: "Rotation, 0 still .. 1 = gears/max_rev_per_second on the first wheel" },
        InputSpec { name: "load", default: 0.3, doc: "Torque: harder, louder tooth contacts" },
    ];

    fn presets() -> Vec<(&'static str, GearsParams)> {
        let d = GearsParams::default();
        vec![
            ("Iron gears", GearsParams { max_rps: 0.6, teeth: 20.0, stages: 2.0, ratio: 0.45, wear: 0.6, hz: 420.0, ring: 2.5, level: 1.0, whir: 0.0, ..d }),
            ("Ratchet winch", GearsParams { max_rps: 0.5, teeth: 24.0, stages: 1.0, hz: 1500.0, level: 0.25, ratchet_teeth: 24.0, ratchet: 1.0, whir: 0.0, ..d }),
            ("Wind-up toy", GearsParams { max_rps: 8.3, teeth: 3.0, stages: 1.0, grouping: 1.0, wear: 0.2, hz: 4500.0, ring: 0.4, level: 0.8, whir: 0.25, whir_hz: 6500.0, ..d }),
            ("Wooden mill", GearsParams { max_rps: 0.3, teeth: 16.0, stages: 2.0, ratio: 0.4, wear: 0.8, hz: 380.0, ring: 0.5, level: 1.0, whir: 0.0, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        Gears {
            sr,
            noise: Noise::new(mix_seed(0x7_4001)),
            rng: Rng::new(mix_seed(0x7_4002)),
            spin: 0.0,
            phase: [0.0; GEAR_STAGES],
            next: [1; GEAR_STAGES],
            click: core::array::from_fn(|k| Click::new(&GEAR_RATIOS, &GEAR_T60, &GEAR_LEVEL, 0x7_4010 + k as u32)),
            ratchet_phase: 0.0,
            ratchet: Click::new(&[1.0, 1.9, 3.3, 4.8], &[0.05, 0.035, 0.025, 0.015], &[1.0, 0.8, 0.5, 0.3], 0x7_4003),
            bounce: (0.0, 0.0),
            whir: Svf::default(),
            whir_env: 0.0,
        }
    }

    fn snap(&mut self, x: &[f32], _p: &GearsParams) {
        self.spin = x[0];
    }

    fn block(&mut self, x: &[f32], p: &GearsParams, out: &mut [f32]) {
        let (sr, n) = (self.sr, out.len());
        let dt = n as f32 / sr;
        let spin = slew(&mut self.spin, x[0], 0.15, 0.25, dt);
        let load = x[1];
        let stages = (p.stages as usize).clamp(1, GEAR_STAGES);
        let teeth = p.teeth as u32;
        let mut rps = [0.0f32; GEAR_STAGES];
        let mut amp = [0.0f32; GEAR_STAGES];
        let mut fastest = 0.0f32;
        for k in 0..stages {
            rps[k] = spin * p.max_rps * p.ratio.powi(k as i32);
            let mesh = rps[k] * teeth as f32;
            fastest = fastest.max(mesh);
            // Many contacts a second blend into a whine: keep the power steady as they speed up.
            amp[k] = p.level * (0.4 + 0.6 * load) * (25.0 / mesh.max(25.0)).sqrt() * 0.6 / (1.0 + 0.3 * k as f32);
            // Faster wheels are smaller and higher.
            self.click[k].set(p.hz * 1.25f32.powi(k as i32), p.ring, p.hz * 1.6, 0.003, sr);
        }
        // A pawl is small spring steel: bright whatever the wheels are (winding clicks centre near 6.7 kHz).
        self.ratchet.set(4200.0 * (p.hz / 2600.0).powf(0.3), 0.6, 7000.0, 0.003, sr);
        self.whir.set(FilterMode::BandPass, p.whir_hz, 0.45, sr);
        let whir_target = p.whir * smoothstep(spin * 3.0) * spin;
        let ratchet_teeth = p.ratchet_teeth as u32;
        let ratchet_amp = p.ratchet * (0.5 + 0.5 * load) * 0.5;
        let gain = p.gain * 0.8;
        for o in out.iter_mut() {
            let w = self.noise.white();
            let mut y = 0.0;
            for k in 0..stages {
                let mut hit = 0.0;
                if rps[k] > 0.0 {
                    self.phase[k] += rps[k] / sr;
                    if self.phase[k] >= 1.0 {
                        self.phase[k] -= 1.0;
                        // The first tooth of the next turn.
                        let tooth_wear = 1.0 - p.wear * hash01(k as i64 * 64, 0x6EA2);
                        hit += amp[k] * tooth_wear * self.rng.range(0.9, 1.0);
                        self.next[k] = 1;
                    }
                    while self.next[k] < teeth && self.phase[k] >= Gears::tooth_at(self.next[k], teeth, p.grouping) {
                        let tooth_wear = 1.0 - p.wear * hash01(k as i64 * 64 + self.next[k] as i64, 0x6EA2);
                        hit += amp[k] * tooth_wear * self.rng.range(0.9, 1.0);
                        self.next[k] += 1;
                    }
                }
                y += self.click[k].tick(hit, w, 1.5, 0.35, sr);
            }
            // Ratchet on the first wheel: a pawl drop, then its bounce 25 ms later.
            let mut r = 0.0;
            if ratchet_amp > 0.0 && rps[0] > 0.0 {
                self.ratchet_phase += rps[0] * ratchet_teeth as f32 / sr;
                if self.ratchet_phase >= 1.0 {
                    self.ratchet_phase -= 1.0;
                    r = ratchet_amp * self.rng.range(0.85, 1.0);
                    self.bounce = (0.025 * sr * self.rng.range(0.8, 1.2), r * 0.15);
                }
            }
            if self.bounce.1 > 0.0 {
                self.bounce.0 -= 1.0;
                if self.bounce.0 <= 0.0 {
                    r += self.bounce.1;
                    self.bounce.1 = 0.0;
                }
            }
            y += self.ratchet.tick(r, w, 2.2, 0.4, sr);
            self.whir_env += (whir_target - self.whir_env) * 0.001;
            if self.whir_env > 1e-5 {
                // The governor's whir flutters with the fastest mesh.
                let flutter = 0.7 + 0.3 * (TAU * self.phase[stages - 1] * fastest.min(400.0) / rps[stages - 1].max(1e-3)).sin();
                y += self.whir.tick(w) * self.whir_env * flutter * 0.35;
            }
            *o = y * gain;
        }
        for c in self.click.iter_mut() {
            c.end_block();
        }
        self.ratchet.end_block();
    }
}

// ---------------------------------------------------------------------------------------------
// Music box
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A music box: a pinned cylinder plucking the teeth of a steel comb. Each tooth is a
    /// nearly pure tone that rings for a second or two (higher teeth shorter) with a faint
    /// cantilever overtone at 6.27x, and every pluck has a tiny scrape of the pin. As the spring
    /// runs down (`wind`) the tune slows, stumbles and stops; the pitch stays, as in a real one.
    /// The lid muffles it when closed.
    ///
    /// Tunes (`tune/kind`): 0 lullaby, 1 waltz, 2 carillon (originals written for Brusverk) and
    /// 3 a random pentatonic melody that never repeats.
    ///
    /// Measured (`target/refs/machines/musicbox`): notes 520-1760 Hz, overtones 25-35 dB down,
    /// nothing above 4 kHz but the pluck.
    MusicBoxParams / MusicBoxParamId {
        tune: "tune/kind" = 0.0, int(0, 3);
        tempo: "tune/notes_per_second" = 5.0, exp(0.5, 16.0);
        root_hz: "tune/root_hz" = 523.25, exp(130.0, 2000.0);
        ring: "comb/ring" = 1.0, exp(0.2, 4.0);
        overtone: "comb/overtone" = 0.15, UNIT;
        pluck: "comb/pluck" = 0.3, UNIT;
        mechanism: "mechanism/level" = 0.15, UNIT;
        unsteady: "mechanism/unsteady" = 0.15, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// The comb: a major scale over two octaves (semitones above the root).
const COMB: [f32; 15] = [0.0, 2.0, 4.0, 5.0, 7.0, 9.0, 11.0, 12.0, 14.0, 16.0, 17.0, 19.0, 21.0, 23.0, 24.0];
const R: i8 = -1;
/// Tunes as (melody, bass) per step, in comb teeth; -1 is a rest.
const TUNES: [(&[i8], &[i8]); 3] = [
    (
        &[11, R, 9, R, 7, R, 9, 11, 12, R, 11, R, 9, R, R, R, 11, R, 9, R, 7, R, 9, 11, 9, R, 8, R, 7, R, R, R],
        &[0, R, R, R, R, R, R, R, 3, R, R, R, R, R, R, R, 0, R, R, R, R, R, R, R, 4, R, R, R, 0, R, R, R],
    ),
    (
        &[7, R, 9, 11, R, 11, 12, R, 11, 9, R, R, 8, R, 10, 12, R, 12, 11, R, 9, 7, R, R],
        &[0, R, 4, R, 4, R, 3, R, 5, R, 5, R, 4, R, 6, R, 6, R, 0, R, 4, R, 2, R],
    ),
    (&[7, 9, 11, 14, 11, 9, 7, 4, 8, 10, 12, 13, 12, 10, 8, 6], &[0, R, R, R, R, R, R, R, 4, R, R, R, R, R, R, R]),
];
/// Teeth of a pentatonic scale, for the generated tune.
const PENTA: [i8; 8] = [7, 8, 9, 11, 12, 14, 9, 11];

pub struct MusicBox {
    sr: f32,
    noise: Noise,
    rng: Rng,
    teeth: ModalBank,
    over: ModalBank,
    key: (f32, f32),
    step: usize,
    /// Steps until the next one.
    wait: f32,
    walk: usize,
    pluck: Burst,
    pluck_hp: OnePole,
    lid: [OnePole; 2],
    whir: Svf,
    tick: Click,
}

impl MusicBox {
    fn retune(&mut self, p: &MusicBoxParams) {
        let key = (p.root_hz, p.ring);
        if key == self.key {
            return;
        }
        self.key = key;
        self.teeth.set_len(COMB.len());
        self.over.set_len(COMB.len());
        for (k, semi) in COMB.iter().enumerate() {
            let hz = p.root_hz * (semi / 12.0).exp2();
            // Short, stiff teeth ring for less time.
            let t60 = (p.ring * 2.2 * (500.0 / hz).sqrt()).clamp(0.05, 8.0);
            self.teeth.tune(k, hz, t60, self.sr);
            self.over.tune(k, hz * 6.27, t60 * 0.2, self.sr);
        }
    }

    /// Pluck tooth `k` with `a`, in this sample.
    fn pluck_tooth(&mut self, k: usize, a: f32, overtone: f32) {
        let k = k.min(COMB.len() - 1);
        if self.teeth.is_tuned(k) {
            self.teeth.set_gain(k, self.teeth.gain(k) + a);
            self.teeth.set_live(k, true);
        }
        if self.over.is_tuned(k) {
            self.over.set_gain(k, self.over.gain(k) + a * overtone);
            self.over.set_live(k, true);
        }
    }
}

impl Generator for MusicBox {
    type P = MusicBoxParams;
    const NAME: &'static str = "music_box";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "Music box: a pinned cylinder plucking a steel comb. It slows, stumbles and stops as the spring runs \
        down; the lid muffles it. Three original tunes and a generated one.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "wind", default: 1.0, doc: "Spring: 1 fully wound .. 0 run down (slower, then stopped)" },
        InputSpec { name: "lid", default: 1.0, doc: "Lid: 0 closed (muffled) .. 1 open" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.2;

    fn presets() -> Vec<(&'static str, MusicBoxParams)> {
        let d = MusicBoxParams::default();
        vec![
            ("Waltz", MusicBoxParams { tune: 1.0, tempo: 4.5, ..d }),
            ("Carillon", MusicBoxParams { tune: 2.0, tempo: 6.0, root_hz: 698.5, ..d }),
            ("Endless", MusicBoxParams { tune: 3.0, tempo: 3.5, root_hz: 440.0, ring: 1.4, ..d }),
            ("Toy", MusicBoxParams { tempo: 6.5, root_hz: 880.0, ring: 0.6, overtone: 0.3, pluck: 0.5, mechanism: 0.35, unsteady: 0.35, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        MusicBox {
            sr,
            noise: Noise::new(mix_seed(0x7_5001)),
            rng: Rng::new(mix_seed(0x7_5002)),
            teeth: ModalBank::new(),
            over: ModalBank::new(),
            key: (f32::NAN, f32::NAN),
            step: 0,
            wait: 0.0,
            walk: 2,
            pluck: Burst::default(),
            pluck_hp: OnePole::default(),
            lid: [OnePole::default(); 2],
            whir: Svf::default(),
            tick: Click::new(&[1.0, 1.7, 2.6], &[0.03, 0.02, 0.015], &[1.0, 0.6, 0.3], 0x7_5003),
        }
    }

    fn block(&mut self, x: &[f32], p: &MusicBoxParams, out: &mut [f32]) {
        let sr = self.sr;
        let (wind, lid) = (x[0], x[1]);
        self.retune(p);
        // Fully wound it plays at its tempo; as the spring runs out it drags, then stops.
        let rate = if wind < 0.01 { 0.0 } else { p.tempo * (0.12 + 0.88 * smoothstep(wind * 2.5)) };
        let unsteady = p.unsteady * (0.06 + 0.5 * (1.0 - smoothstep(wind * 3.0)));
        self.pluck.set(FilterMode::HighPass, 3500.0, 0.1, 0.006, sr);
        self.whir.set(FilterMode::BandPass, 6500.0, 0.3, sr);
        self.tick.set(2200.0, 0.6, 3000.0, 0.003, sr);
        let lid_coef = hz_coef(900.0 * (14.0f32).powf(lid), sr);
        let lid_gain = 0.45 + 0.55 * lid;
        let kind = (p.tune.round() as usize).min(3);
        let gain = p.gain * 0.45 * lid_gain;
        let whir_gain = p.mechanism * (rate / p.tempo.max(0.5)) * 0.04;
        let pluck_hp = hz_coef(2500.0, sr);
        for o in out.iter_mut() {
            let (mut tick, mut excite) = (0.0, 0.0);
            if rate > 0.0 {
                // Counted in steps, so the tempo follows the spring at once.
                self.wait -= rate / sr;
                if self.wait <= 0.0 {
                    self.wait = (self.wait + 1.0 + unsteady * self.rng.next_bipolar()).max(0.0);
                    // Clear last pluck's excitation gains, then pluck this step's teeth.
                    for k in 0..COMB.len() {
                        self.teeth.set_gain(k, 0.0);
                        self.over.set_gain(k, 0.0);
                    }
                    let (mel, bass) = if kind < 3 {
                        let (m, b) = TUNES[kind];
                        let i = self.step % m.len();
                        (m[i], b[i % b.len()])
                    } else {
                        // A pentatonic walk: small steps, the odd rest, a bass every eight.
                        let r = self.rng.next_f32();
                        let mv = if r < 0.35 { -1 } else if r < 0.7 { 1 } else if r < 0.85 { 2 } else { -2 };
                        self.walk = (self.walk as i32 + mv).clamp(0, PENTA.len() as i32 - 1) as usize;
                        let m = if self.rng.next_f32() < 0.18 { R } else { PENTA[self.walk] };
                        let b = if self.step.is_multiple_of(8) { [0, 3, 4, 0][(self.rng.next_u32() % 4) as usize] } else { R };
                        (m, b)
                    };
                    for (tooth, a) in [(mel, 1.0), (bass, 0.7)] {
                        if tooth >= 0 {
                            let a = a * self.rng.range(0.85, 1.0);
                            self.pluck_tooth(tooth as usize, a, p.overtone);
                            self.pluck.kick(a * p.pluck * 0.4);
                            excite = 1.0;
                        }
                    }
                    // The cylinder's drive ticks every other step.
                    if self.step.is_multiple_of(2) {
                        tick = p.mechanism * 0.15;
                    }
                    self.step = self.step.wrapping_add(1);
                }
            }
            let w = self.noise.white();
            let mut y = self.teeth.tick(excite) * 0.5 + self.over.tick(excite) * 0.5;
            y += self.pluck_hp.hp(self.pluck.tick(w), pluck_hp) + self.tick.tick(tick, w, 0.5, 0.5, sr);
            y += self.whir.tick(w) * whir_gain;
            let l = self.lid[0].lp(y, lid_coef);
            *o = self.lid[1].lp(l, lid_coef) * gain;
        }
        self.teeth.energy(1e-6);
        self.over.energy(1e-6);
        self.tick.end_block();
    }
}

// ---------------------------------------------------------------------------------------------
// Unlock (event)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A mechanism unlocking, in order: the tumblers or dial clicking into place, a short run of
    /// gears, the bolt sliding back, and the clunk as it hits its stop; then, if set, a seal's
    /// hiss and a chime (the puzzle is solved). `size` scales it from a padlock to a vault door:
    /// lower, slower and heavier.
    ///
    /// Measured (`target/refs/machines/gears`): a vault dial's clicks 50-150 ms apart, bright
    /// (1-10 kHz), each with a faint second click 25 ms later.
    UnlockParams / UnlockParamId {
        tumblers: "lock/tumblers" = 4.0, int(0, 12);
        spacing_ms: "lock/spacing_ms" = 90.0, lin(20.0, 400.0);
        click_hz: "lock/click_hz" = 3500.0, exp(500.0, 10000.0);
        gears: "mechanism/gears" = 0.3, UNIT;
        gear_ms: "mechanism/gear_ms" = 300.0, lin(0.0, 2000.0);
        slide: "bolt/slide" = 0.4, UNIT;
        slide_ms: "bolt/slide_ms" = 220.0, lin(0.0, 3000.0);
        clunk: "bolt/clunk" = 0.8, UNIT;
        clunk_hz: "bolt/clunk_hz" = 300.0, exp(40.0, 1500.0);
        hiss: "seal/hiss" = 0.0, UNIT;
        chime: "chime/level" = 0.5, UNIT;
        chime_hz: "chime/hz" = 1568.0, exp(300.0, 4000.0);
        variation: "shape/variation" = 0.5, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

const CLUNK_RATIOS: [f32; 5] = [1.0, 2.3, 3.9, 5.6, 7.9];
const CLUNK_T60: [f32; 5] = [0.35, 0.22, 0.15, 0.1, 0.07];
const CLUNK_LEVEL: [f32; 5] = [1.0, 0.7, 0.5, 0.35, 0.2];
const BAR_RATIOS: [f32; 3] = [1.0, 2.76, 5.4];
const BAR_T60: [f32; 3] = [1.6, 0.6, 0.25];
const BAR_LEVEL: [f32; 3] = [1.0, 0.35, 0.15];
const MAX_TUMBLERS: usize = 12;

/// Distance for the machine events: quieter and duller, as for the other events.
fn far(air: &mut Air, distance: f32, sr: f32) {
    air.set(distance, 18000.0, 400.0, 1.0 - 0.75 * distance, sr);
}

pub struct Unlock {
    sr: f32,
    noise: Noise,
    rng: Rng,
    t: f32,
    /// Event times (seconds) for this trigger.
    tumbler_at: [f32; MAX_TUMBLERS],
    tumbler_amp: [f32; MAX_TUMBLERS],
    n_tumblers: usize,
    gear_from: f32,
    gear_to: f32,
    slide_from: f32,
    slide_to: f32,
    clunk_at: f32,
    end: f32,
    power: f32,
    scale: f32,
    pitch: f32,
    gear_phase: f32,
    tumbler: Click,
    gear: Click,
    clunk: Modal,
    thud: Burst,
    slide: Svf,
    stick: SlowNoise,
    hiss: Burst,
    hiss_hp: OnePole,
    chime: Modal,
    air: Air,
    active: bool,
    fired: [bool; 3],
}

impl Unlock {
    fn scale_of(size: f32) -> f32 {
        crate::dsp::size_scale(size)
    }
}

impl Generator for Unlock {
    type P = UnlockParams;
    const NAME: &'static str = "unlock";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "A mechanism unlocking: tumblers clicking into place, gears, the bolt sliding back and its clunk, \
        then a seal's hiss or a chime. Puzzle box, door lock, padlock, vault, stone door.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "power", default: 1.0, doc: "How firmly it works: level and weight" },
        InputSpec { name: "distance", default: 0.0, doc: "0 = on top of the listener, 1 = far off (duller, quieter)" },
        InputSpec { name: "size", default: 0.5, doc: "0 tiny (higher, quicker) .. 1 huge (lower, slower, heavier)" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const ONE_SHOT: bool = true;
    const TRANSPOSES: bool = true;

    fn presets() -> Vec<(&'static str, UnlockParams)> {
        let d = UnlockParams::default();
        vec![
            ("Door lock", UnlockParams { tumblers: 2.0, spacing_ms: 120.0, click_hz: 2800.0, gears: 0.0, slide: 0.6, slide_ms: 180.0, clunk: 0.9, clunk_hz: 220.0, chime: 0.0, ..d }),
            ("Padlock", UnlockParams { tumblers: 3.0, spacing_ms: 60.0, click_hz: 4200.0, gears: 0.0, slide: 0.2, slide_ms: 40.0, clunk: 0.7, clunk_hz: 900.0, chime: 0.0, ..d }),
            (
                "Vault door",
                UnlockParams { tumblers: 8.0, spacing_ms: 140.0, click_hz: 2500.0, gears: 0.8, gear_ms: 1200.0, slide: 0.8, slide_ms: 1500.0, clunk: 1.0, clunk_hz: 70.0, hiss: 0.7, chime: 0.0, ..d },
            ),
            ("Stone door", UnlockParams { tumblers: 0.0, click_hz: 800.0, gears: 0.6, gear_ms: 800.0, slide: 1.0, slide_ms: 2500.0, clunk: 1.0, clunk_hz: 50.0, chime: 0.0, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        Unlock {
            sr,
            noise: Noise::new(mix_seed(0x7_6001)),
            rng: Rng::new(mix_seed(0x7_6002)),
            t: 0.0,
            tumbler_at: [0.0; MAX_TUMBLERS],
            tumbler_amp: [0.0; MAX_TUMBLERS],
            n_tumblers: 0,
            gear_from: 0.0,
            gear_to: 0.0,
            slide_from: 0.0,
            slide_to: 0.0,
            clunk_at: 0.0,
            end: 0.0,
            power: 1.0,
            scale: 1.0,
            pitch: 1.0,
            gear_phase: 0.0,
            tumbler: Click::new(&[1.0, 1.32, 1.81, 2.4, 3.1], &[0.03, 0.025, 0.02, 0.015, 0.012], &[1.0, 0.8, 0.7, 0.5, 0.35], 0x7_6003),
            gear: Click::new(&GEAR_RATIOS, &GEAR_T60, &GEAR_LEVEL, 0x7_6004),
            clunk: Modal::new(&CLUNK_RATIOS, &CLUNK_T60, &CLUNK_LEVEL, 0x7_6005),
            thud: Burst::default(),
            slide: Svf::default(),
            stick: SlowNoise::new(0x7_6006),
            hiss: Burst::default(),
            hiss_hp: OnePole::default(),
            chime: Modal::new(&BAR_RATIOS, &BAR_T60, &BAR_LEVEL, 0x7_6007),
            air: Air::default(),
            active: false,
            fired: [false; 3],
        }
    }

    fn trigger(&mut self, x: &[f32], p: &UnlockParams) {
        let (power, distance, size) = (x[0], x[1], x[2]);
        let v = p.variation.clamp(0.0, 1.0);
        if v <= 0.0 {
            self.rng = Rng::new(mix_seed(0x7_6002));
        }
        let s = Unlock::scale_of(size);
        let slow = s.sqrt();
        self.scale = s;
        self.power = power;
        let mut t = 0.01;
        self.n_tumblers = (p.tumblers as usize).min(MAX_TUMBLERS);
        for k in 0..self.n_tumblers {
            self.tumbler_at[k] = t;
            self.tumbler_amp[k] = self.rng.range(0.7, 1.0) * (0.85 + 0.15 * (k as f32 / self.n_tumblers.max(1) as f32));
            t += p.spacing_ms * 1e-3 * slow * (1.0 + v * 0.5 * self.rng.next_bipolar());
        }
        if self.n_tumblers > 0 {
            t += 0.05 * slow;
        }
        self.gear_from = t;
        if p.gears > 0.0 {
            t += p.gear_ms * 1e-3 * slow * (1.0 + v * 0.2 * self.rng.next_bipolar());
        }
        self.gear_to = t;
        self.slide_from = t;
        t += p.slide_ms * 1e-3 * slow * (1.0 + v * 0.2 * self.rng.next_bipolar());
        self.slide_to = t;
        self.clunk_at = t;
        self.end = t;
        self.t = 0.0;
        self.fired = [false; 3];
        far(&mut self.air, distance, self.sr);
        self.active = true;
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        self.pitch = ratio;
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn length(p: &UnlockParams) -> Option<f32> {
        let slow = Unlock::scale_of(1.0).sqrt();
        let timeline = 0.01 + p.tumblers * p.spacing_ms * 1e-3 * slow * 1.5 + 0.05 * slow + p.gear_ms * 1e-3 * slow * 1.2 + p.slide_ms * 1e-3 * slow * 1.2;
        let tail = [0.35 * Unlock::scale_of(1.0).powf(0.6) * 1.5, if p.hiss > 0.0 { 2.2 } else { 0.0 }, if p.chime > 0.0 { 0.15 + 1.6 * 1.6 } else { 0.0 }, 0.2].into_iter().fold(0.0, f32::max);
        Some(timeline + tail + 0.4)
    }

    fn block(&mut self, _x: &[f32], p: &UnlockParams, out: &mut [f32]) {
        let sr = self.sr;
        // Size and the host's pitch both move every frequency.
        let s = self.scale / self.pitch;
        let pw = self.power;
        let v = p.variation;
        self.tumbler.set(p.click_hz / s, 1.0, p.click_hz * 1.3 / s, 0.004, sr);
        self.gear.set(p.click_hz * 0.6 / s, 1.0, p.click_hz * 0.8 / s, 0.003, sr);
        self.clunk.set(p.clunk_hz / s, 0.5, s.powf(0.6), sr);
        self.thud.set(FilterMode::LowPass, (p.clunk_hz * 1.5 / s).max(60.0), 0.2, 0.08 * s.sqrt(), sr);
        self.hiss.set(FilterMode::BandPass, 5000.0, 0.15, 1.4, sr);
        self.chime.set(p.chime_hz, 0.5, 1.0, sr);
        let slide_hz = (p.clunk_hz * 5.0 / s).clamp(250.0, 3000.0);
        let hiss_hp = hz_coef(1200.0, sr);
        let level = pw.powf(0.8) * s.powf(0.25);
        let gain = p.gain * 0.9;
        let dt = 1.0 / sr;
        let gear_rate = 45.0 / s.sqrt();
        for o in out.iter_mut() {
            let t = self.t;
            self.t += dt;
            let w = self.noise.white();
            let mut xt = 0.0;
            for k in 0..self.n_tumblers {
                if self.tumbler_amp[k] > 0.0 && t >= self.tumbler_at[k] {
                    xt += self.tumbler_amp[k] * level * 0.5;
                    self.tumbler_amp[k] = 0.0;
                }
            }
            let mut xg = 0.0;
            if p.gears > 0.0 && t >= self.gear_from && t < self.gear_to {
                let u = (t - self.gear_from) / (self.gear_to - self.gear_from).max(1e-3);
                // The gears run up to speed and slow at the end.
                self.gear_phase += gear_rate * (0.4 + 0.6 * (core::f32::consts::PI * u).sin()) * dt;
                if self.gear_phase >= 1.0 {
                    self.gear_phase -= 1.0;
                    xg = p.gears * level * self.rng.range(0.5, 1.0) * 0.3;
                }
            }
            let mut y = self.tumbler.tick(xt, w, 0.6, 0.5 * v, sr) + self.gear.tick(xg, w, 0.5, 0.4, sr);
            if t >= self.slide_from && t < self.slide_to && p.slide > 0.0 {
                let u = (t - self.slide_from) / (self.slide_to - self.slide_from).max(1e-3);
                let env = smoothstep(u * 6.0) * smoothstep((1.0 - u) * 10.0);
                self.slide.set(FilterMode::BandPass, slide_hz * (0.8 + 0.4 * u), 0.35, sr);
                let stick = 0.55 + 0.45 * self.stick.advance(35.0 / s.sqrt(), dt);
                y += self.slide.tick(self.noise.pink()) * env * stick * p.slide * level * 1.6;
            }
            let mut xc = 0.0;
            if !self.fired[0] && t >= self.clunk_at {
                self.fired[0] = true;
                xc = p.clunk * level * 0.6;
                self.thud.kick(p.clunk * level * 1.2);
                if p.hiss > 0.0 {
                    self.hiss.kick(p.hiss * level * 0.5);
                }
            }
            let mut xch = 0.0;
            if !self.fired[1] && t >= self.clunk_at + 0.15 {
                self.fired[1] = true;
                xch = p.chime * pw * 0.25;
            }
            y += self.clunk.tick(xc, 0.23, 0.5 * v, sr) + self.thud.tick(w) * 0.8 + self.hiss_hp.hp(self.hiss.tick(w), hiss_hp) * 0.7;
            y += self.chime.tick(xch, 0.23, 0.2 * v, sr);
            *o = self.air.tick(y) * gain;
        }
        let energy = self.clunk.end_block() + self.chime.end_block();
        self.tumbler.end_block();
        self.gear.end_block();
        if self.t > self.end + 0.2 && energy < 1e-5 && self.hiss.env < 1e-5 && self.thud.env < 1e-6 {
            self.active = false;
            self.air.reset();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Pneumatic (event)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Compressed air working a piston: the solenoid valve clicks, the piston runs and clacks
    /// against its stop `piston/ms` later, and the exhaust lets go with a hiss that falls away
    /// over `exhaust/secs` (seconds for a big pressure release). Air hiss is the one bright
    /// sound in this family: the recordings of air releases are flat to 12 kHz.
    ///
    /// Measured (`target/refs/machines/hydraulic`): an air release with its centroid near 12 kHz
    /// and 98 % of its energy above 2.5 kHz, a resonance at 308 Hz in the pipe, decaying over 3-4 s.
    PneumaticParams / PneumaticParamId {
        valve: "valve/click" = 0.5, UNIT;
        clack: "piston/clack" = 0.7, UNIT;
        piston_ms: "piston/ms" = 45.0, lin(5.0, 1000.0);
        clack_hz: "piston/hz" = 1200.0, exp(100.0, 5000.0);
        hiss: "exhaust/level" = 0.8, UNIT;
        hiss_hz: "exhaust/hz" = 7000.0, exp(500.0, 14000.0);
        hiss_secs: "exhaust/secs" = 0.3, exp(0.05, 8.0);
        pipe: "exhaust/pipe" = 0.2, UNIT;
        pipe_hz: "exhaust/pipe_hz" = 310.0, exp(80.0, 3000.0);
        variation: "shape/variation" = 0.5, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Pneumatic {
    sr: f32,
    noise: Noise,
    rng: Rng,
    t: f32,
    clack_at: f32,
    clack_amp: f32,
    hiss_env: f32,
    hiss_decay: f32,
    hiss_attack: f32,
    hiss_peak: f32,
    hiss_bp: Svf,
    hiss_hp: OnePole,
    pipe_bp: Svf,
    valve: Click,
    piston: Modal,
    thud: Burst,
    air: Air,
    scale: f32,
    pitch: f32,
    active: bool,
}

impl Generator for Pneumatic {
    type P = PneumaticParams;
    const NAME: &'static str = "pneumatic";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "Pneumatic piston or pressure release: the valve's click, the piston's clack at its stop, \
        and the exhaust's hiss (short for a piston, seconds for a big release).";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "power", default: 1.0, doc: "Air pressure: level and length of the hiss" },
        InputSpec { name: "distance", default: 0.0, doc: "0 = on top of the listener, 1 = far off (duller, quieter)" },
        InputSpec { name: "size", default: 0.5, doc: "0 small (higher, quicker) .. 1 big (lower, slower)" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const ONE_SHOT: bool = true;
    const TRANSPOSES: bool = true;

    fn presets() -> Vec<(&'static str, PneumaticParams)> {
        let d = PneumaticParams::default();
        vec![
            ("Pressure release", PneumaticParams { valve: 0.2, clack: 0.0, hiss: 1.0, hiss_hz: 9000.0, hiss_secs: 3.5, pipe: 0.4, ..d }),
            ("Air tool", PneumaticParams { valve: 0.3, clack: 0.4, piston_ms: 20.0, clack_hz: 2200.0, hiss_hz: 5000.0, hiss_secs: 0.12, ..d }),
            ("Heavy cylinder", PneumaticParams { valve: 0.6, clack: 1.0, piston_ms: 300.0, clack_hz: 400.0, hiss_hz: 4000.0, hiss_secs: 0.8, ..d }),
            ("Bus door", PneumaticParams { valve: 0.3, clack: 0.8, piston_ms: 700.0, clack_hz: 250.0, hiss: 0.7, hiss_hz: 6000.0, hiss_secs: 0.6, pipe: 0.3, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        Pneumatic {
            sr,
            noise: Noise::new(mix_seed(0x7_7001)),
            rng: Rng::new(mix_seed(0x7_7002)),
            t: 0.0,
            clack_at: 0.0,
            clack_amp: 0.0,
            hiss_env: 0.0,
            hiss_decay: 1.0,
            hiss_attack: 0.0,
            hiss_peak: 0.0,
            hiss_bp: Svf::default(),
            hiss_hp: OnePole::default(),
            pipe_bp: Svf::default(),
            valve: Click::new(&[1.0, 1.6, 2.5], &[0.02, 0.015, 0.01], &[1.0, 0.7, 0.4], 0x7_7003),
            piston: Modal::new(&[1.0, 2.1, 3.4, 4.9], &[0.15, 0.1, 0.07, 0.05], &[1.0, 0.6, 0.4, 0.25], 0x7_7004),
            thud: Burst::default(),
            air: Air::default(),
            scale: 1.0,
            pitch: 1.0,
            active: false,
        }
    }

    fn trigger(&mut self, x: &[f32], p: &PneumaticParams) {
        let (power, distance, size) = (x[0], x[1], x[2]);
        let v = p.variation.clamp(0.0, 1.0);
        if v <= 0.0 {
            self.rng = Rng::new(mix_seed(0x7_7002));
            self.noise = Noise::new(mix_seed(0x7_7001));
        }
        let s = crate::dsp::size_scale(size);
        self.scale = s;
        self.t = 0.0;
        self.clack_at = p.piston_ms * 1e-3 * s.sqrt() * (1.0 + v * 0.15 * self.rng.next_bipolar());
        self.clack_amp = p.clack * power * s.powf(0.25) * self.rng.range(0.85, 1.0) * 0.5;
        self.hiss_peak = p.hiss * power.powf(0.7) * 0.5 * (1.0 + v * 0.15 * self.rng.next_bipolar());
        self.hiss_env = 0.0;
        self.hiss_attack = 0.0;
        let secs = p.hiss_secs * (0.6 + 0.4 * power) * s.sqrt() * (1.0 + v * 0.2 * self.rng.next_bipolar());
        self.hiss_decay = t60_coef(secs, self.sr);
        self.valve.ring.strike(0.23, v);
        far(&mut self.air, distance, self.sr);
        self.active = true;
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        self.pitch = ratio;
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn length(p: &PneumaticParams) -> Option<f32> {
        let slow = crate::dsp::size_scale(1.0).sqrt();
        Some(p.piston_ms * 1e-3 * slow * 1.15 + (p.hiss_secs * slow * 1.2 * 1.35).max(0.3 * 1.5) + 0.4)
    }

    fn block(&mut self, _x: &[f32], p: &PneumaticParams, out: &mut [f32]) {
        let sr = self.sr;
        let s = self.scale / self.pitch;
        self.hiss_bp.set(FilterMode::BandPass, p.hiss_hz / s.powf(0.3), 0.15, sr);
        self.pipe_bp.set(FilterMode::BandPass, p.pipe_hz / s.powf(0.5), 0.85, sr);
        self.valve.set(2600.0 / s.powf(0.3), 1.0, 3500.0, 0.003, sr);
        self.piston.set(p.clack_hz / s, 0.5, s.powf(0.5), sr);
        self.thud.set(FilterMode::LowPass, (p.clack_hz * 0.4 / s).max(60.0), 0.2, 0.06, sr);
        let hp = hz_coef(1500.0 / s.powf(0.3), sr);
        let attack = 1.0 / (0.008 * sr);
        let gain = p.gain;
        let dt = 1.0 / sr;
        for o in out.iter_mut() {
            let t = self.t;
            self.t += dt;
            let w = self.noise.white();
            let xv = if t == 0.0 { p.valve * 0.4 } else { 0.0 };
            let mut xc = 0.0;
            if self.clack_amp > 0.0 && t >= self.clack_at {
                xc = self.clack_amp;
                self.thud.kick(self.clack_amp * 1.4);
                self.clack_amp = 0.0;
            }
            // The exhaust: a fast rise, then an exponential fall.
            if self.hiss_attack < 1.0 {
                self.hiss_attack = (self.hiss_attack + attack).min(1.0);
                self.hiss_env = self.hiss_peak * smoothstep(self.hiss_attack);
            } else {
                self.hiss_env *= self.hiss_decay;
            }
            let hiss = (self.hiss_hp.hp(w, hp) * 0.5 + self.hiss_bp.tick(w) * 0.8 + self.pipe_bp.tick(w) * p.pipe * 1.5) * self.hiss_env;
            let y = self.valve.tick(xv, w, 0.6, p.variation, sr) + self.piston.tick(xc, 0.23, p.variation, sr) + self.thud.tick(w) + hiss;
            *o = self.air.tick(y) * gain;
        }
        let energy = self.piston.end_block();
        self.valve.end_block();
        if self.clack_amp == 0.0 && self.hiss_attack >= 1.0 && self.hiss_env < 1e-5 && energy < 1e-5 && self.thud.env < 1e-6 && self.t > 0.05 {
            self.active = false;
            self.air.reset();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Hydraulic
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A hydraulic system: the pump's tonal whine (always running, labouring and sagging in pitch
    /// under load), oil hissing through the valve while the cylinder moves, the cylinder's groan
    /// under load, the relief valve whistling when it pushes against a stop, and the solenoid
    /// valve's clunk as motion starts and stops.
    ///
    /// Measured (`target/refs/machines/hydraulic`, `press`): a scissor lift's pump at 346 Hz with
    /// harmonics at 688, 1034 and 1380 Hz (centroid 820 Hz); a press pump at 442 Hz.
    HydraulicParams / HydraulicParamId {
        pump_hz: "pump/hz" = 442.0, exp(60.0, 2000.0);
        pump: "pump/level" = 0.5, UNIT;
        harmonics: "pump/harmonics" = 0.5, UNIT;
        sag: "pump/sag" = 0.06, lin(0.0, 0.3);
        idle: "pump/idle" = 0.35, UNIT;
        flow: "flow/level" = 0.5, UNIT;
        flow_hz: "flow/hz" = 2500.0, exp(300.0, 9000.0);
        relief: "relief/level" = 0.4, UNIT;
        relief_hz: "relief/hz" = 3200.0, exp(500.0, 8000.0);
        creak: "creak/level" = 0.3, UNIT;
        valve: "valve/level" = 0.5, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Hydraulic {
    sr: f32,
    noise: Noise,
    pump: Phasor,
    pump_noise: Svf,
    flow: Svf,
    relief: Phasor,
    relief_bp: Svf,
    wobble: SlowNoise,
    creak: Phasor,
    creak_gate: SlowNoise,
    creak_lp: Svf,
    moving: bool,
    valve: Click,
    thunk: Modal,
    labour: f32,
}

impl Generator for Hydraulic {
    type P = HydraulicParams;
    const NAME: &'static str = "hydraulic";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "Hydraulics: a pump whining and labouring under load, oil hiss while it moves, cylinder groan, \
        relief-valve whistle against a stop, valve clunks. Lifts, presses, robot arms.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "motion", default: 0.0, doc: "How fast the cylinder moves (the valve's opening); clunks on start and stop" },
        InputSpec { name: "load", default: 0.3, doc: "Pressure: the pump labours and sags, the cylinder groans; high load with no motion opens the relief valve" },
    ];

    fn presets() -> Vec<(&'static str, HydraulicParams)> {
        let d = HydraulicParams::default();
        vec![
            ("Scissor lift", HydraulicParams { pump_hz: 346.0, harmonics: 0.6, flow: 0.35, flow_hz: 1800.0, relief: 0.2, creak: 0.4, ..d }),
            ("Robot arm", HydraulicParams { pump_hz: 620.0, pump: 0.4, harmonics: 0.3, sag: 0.15, idle: 0.15, flow: 0.7, flow_hz: 3500.0, creak: 0.1, valve: 0.7, ..d }),
            ("Sci-fi servo", HydraulicParams { pump_hz: 760.0, pump: 0.6, harmonics: 0.9, sag: 0.25, idle: 0.05, flow: 0.3, flow_hz: 5000.0, relief: 0.0, creak: 0.0, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        Hydraulic {
            sr,
            noise: Noise::new(mix_seed(0x7_8001)),
            pump: Phasor::default(),
            pump_noise: Svf::default(),
            flow: Svf::default(),
            relief: Phasor::default(),
            relief_bp: Svf::default(),
            wobble: SlowNoise::new(0x7_8002),
            creak: Phasor::default(),
            creak_gate: SlowNoise::new(0x7_8003),
            creak_lp: Svf::default(),
            moving: false,
            valve: Click::new(&[1.0, 1.8, 2.9], &[0.04, 0.03, 0.02], &[1.0, 0.6, 0.3], 0x7_8004),
            thunk: Modal::new(&[1.0, 1.7], &[0.15, 0.08], &[1.0, 0.4], 0x7_8005),
            labour: 0.3,
        }
    }

    fn snap(&mut self, x: &[f32], _p: &HydraulicParams) {
        self.labour = (x[0].max(0.7 * x[1])).min(1.0);
        self.moving = x[0] > 0.05;
    }

    fn block(&mut self, x: &[f32], p: &HydraulicParams, out: &mut [f32]) {
        let (sr, n) = (self.sr, out.len());
        let dt = n as f32 / sr;
        let (motion, load) = (x[0], x[1]);
        let labour = slew(&mut self.labour, motion.max(0.7 * load).min(1.0), 0.15, 0.4, dt);
        let pump_hz = p.pump_hz * (1.0 - p.sag * load * (0.5 + 0.5 * motion));
        let slope = 2.4 - 1.8 * p.harmonics;
        let amps: [f32; 5] = core::array::from_fn(|k| if pump_hz * ((k + 1) as f32) < sr * 0.45 { ((k + 1) as f32).powf(-slope) } else { 0.0 });
        let pump_gain = p.pump * (p.idle + (1.0 - p.idle) * labour) * 0.22;
        self.pump_noise.set(FilterMode::BandPass, pump_hz * 3.0, 0.4, sr);
        self.flow.set(FilterMode::BandPass, p.flow_hz * (0.7 + 0.6 * motion), 0.3, sr);
        let flow_gain = p.flow * motion.sqrt() * (0.3 + 0.7 * load.sqrt()) * 0.5;
        let relief_on = smoothstep((load - 0.75) / 0.2) * (1.0 - smoothstep(motion * 5.0));
        let wob = self.wobble.advance(6.0, dt);
        let relief_inc = p.relief_hz * (1.0 + 0.01 * wob) / sr;
        self.relief_bp.set(FilterMode::BandPass, p.relief_hz, 0.8, sr);
        let relief_gain = p.relief * relief_on * 0.12;
        let gate = smoothstep(0.5 + 0.8 * self.creak_gate.advance(2.5, dt));
        let creak_gain = p.creak * motion.min(1.0) * load * gate * 0.25;
        self.creak_lp.set(FilterMode::LowPass, 500.0, 0.5, sr);
        let creak_inc = (55.0 + 50.0 * load) * (1.0 + 0.05 * wob) / sr;
        self.valve.set(1600.0, 1.0, 2200.0, 0.004, sr);
        self.thunk.set(85.0, 0.5, 1.0, sr);
        // Solenoid valve: click on start; on stop, a click and the thunk of the pressure spike.
        let (mut xv, mut xt) = (0.0, 0.0);
        if !self.moving && motion > 0.06 {
            self.moving = true;
            xv = p.valve * 0.4;
        } else if self.moving && motion < 0.03 {
            self.moving = false;
            xv = p.valve * 0.3;
            xt = p.valve * (0.4 + 0.6 * load) * 0.8;
        }
        let pump_inc = pump_hz / sr;
        let gain = p.gain;
        for (i, o) in out.iter_mut().enumerate() {
            let w = self.noise.white();
            self.pump.tick(pump_inc);
            let ph = self.pump.phase * TAU;
            let mut tone = 0.0;
            for (k, a) in amps.iter().enumerate() {
                tone += a * ((k + 1) as f32 * ph).sin();
            }
            let pump = (tone + self.pump_noise.tick(w) * 0.8) * pump_gain;
            let flow = self.flow.tick(w) * flow_gain;
            let mut relief = 0.0;
            if relief_gain > 1e-5 {
                self.relief.tick(relief_inc);
                relief = (self.relief.sin() * 0.6 + self.relief_bp.tick(w) * 2.0) * relief_gain;
            }
            let mut creak = 0.0;
            if creak_gain > 1e-5 {
                self.creak.tick(creak_inc);
                creak = self.creak_lp.tick(self.creak.phase * 2.0 - 1.0 + 0.3 * w) * creak_gain;
            }
            let (cv, ct) = if i == 0 { (xv, xt) } else { (0.0, 0.0) };
            let clicks = self.valve.tick(cv, w, 0.5, 0.4, sr) + self.thunk.tick(ct, 0.23, 0.3, sr);
            *o = (pump + flow + relief + creak + clicks) * gain;
        }
        self.valve.end_block();
        self.thunk.end_block();
    }
}

// ---------------------------------------------------------------------------------------------
// Elevator
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A lift: the traction motor's low hum and the inverter's whine, the car's rumble and the
    /// rush of air in the shaft, a whoosh as each landing passes (`car/floor_m` apart) and the
    /// guide shoes knocking over the rail joints, so the ride has a rhythm that follows its
    /// speed. The brake clicks open as it starts and clunks shut as it stops, with a chime on
    /// arrival. Feed the door's position (0 shut .. 1 open) and the door motor whirs, its rollers
    /// rumble, and it thuds at either end.
    ///
    /// Measured (`target/refs/machines/elevator`): the ride is low and steady (centroid 150-250 Hz,
    /// hum lines at 35, 59 and 97 Hz); a door is a 2-7 kHz rolling rush over a low rumble.
    ElevatorParams / ElevatorParamId {
        max_mps: "car/max_mps" = 1.6, exp(0.2, 10.0);
        floor_m: "car/floor_m" = 3.2, lin(2.0, 6.0);
        rail_m: "car/rail_m" = 5.0, lin(1.0, 10.0);
        rumble: "car/rumble" = 0.5, UNIT;
        floors: "car/floor_whoosh" = 0.4, UNIT;
        rails: "car/rail_knock" = 0.3, UNIT;
        motor_hz: "motor/hz" = 48.0, exp(10.0, 300.0);
        motor: "motor/level" = 0.5, UNIT;
        inverter: "motor/inverter" = 0.15, UNIT;
        brake: "brake/level" = 0.5, UNIT;
        door: "door/level" = 0.6, UNIT;
        door_hz: "door/motor_hz" = 220.0, exp(50.0, 1500.0);
        chime: "chime/level" = 0.5, UNIT;
        chime_hz: "chime/hz" = 988.0, exp(300.0, 4000.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Elevator {
    sr: f32,
    noise: Noise,
    rng: Rng,
    v: f32,
    pos: f64,
    travelled: f32,
    moving: bool,
    motor: Phasor,
    inverter: Phasor,
    brown: Brown,
    rumble: Svf,
    rush: Svf,
    whoosh: crate::dsp::Envelope,
    whoosh_bp: Svf,
    rail: Click,
    brake: Click,
    thud: Modal,
    chime: Modal,
    door_prev: f32,
    door_speed: f32,
    door_motor: Phasor,
    door_roll: Svf,
    door_low: Svf,
}

impl Generator for Elevator {
    type P = ElevatorParams;
    const NAME: &'static str = "elevator";
    const CATEGORY: &'static str = "machines";
    const DOC: &'static str = "Lift ride: motor hum, rumble, a whoosh at every landing and knocks over the rail joints, brake click \
        and clunk, an arrival chime, and a door driven by its position.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.0, doc: "Travel speed, 0 stopped .. 1 = car/max_mps (either direction)" },
        InputSpec { name: "door", default: 0.0, doc: "Door position, 0 shut .. 1 open: moving it runs the door motor" },
    ];

    fn presets() -> Vec<(&'static str, ElevatorParams)> {
        let d = ElevatorParams::default();
        vec![
            ("Freight lift", ElevatorParams { max_mps: 0.5, rumble: 0.8, rails: 0.8, motor_hz: 30.0, motor: 0.9, inverter: 0.0, brake: 0.8, door: 0.9, door_hz: 120.0, chime: 0.0, ..d }),
            ("Old cage lift", ElevatorParams { max_mps: 0.8, rumble: 0.6, floors: 0.2, rails: 0.9, motor_hz: 25.0, motor: 0.7, inverter: 0.0, brake: 0.9, door_hz: 160.0, chime: 0.0, ..d }),
            ("Space station", ElevatorParams { max_mps: 4.0, rumble: 0.2, floors: 0.6, rails: 0.0, motor_hz: 70.0, inverter: 0.5, chime: 0.7, chime_hz: 1568.0, ..d }),
        ]
    }

    fn new(sr: f32) -> Self {
        Elevator {
            sr,
            noise: Noise::new(mix_seed(0x7_9001)),
            rng: Rng::new(mix_seed(0x7_9002)),
            v: 0.0,
            pos: 0.0,
            travelled: 0.0,
            moving: false,
            motor: Phasor::default(),
            inverter: Phasor::default(),
            brown: Brown::default(),
            rumble: Svf::default(),
            rush: Svf::default(),
            whoosh: crate::dsp::Envelope::default(),
            whoosh_bp: Svf::default(),
            rail: Click::new(&[1.0, 1.9, 2.8], &[0.08, 0.05, 0.03], &[1.0, 0.5, 0.3], 0x7_9003),
            brake: Click::new(&[1.0, 1.55, 2.4, 3.6], &[0.06, 0.05, 0.03, 0.02], &[1.0, 0.7, 0.5, 0.3], 0x7_9004),
            thud: Modal::new(&[1.0, 1.6, 2.7], &[0.25, 0.12, 0.06], &[1.0, 0.5, 0.25], 0x7_9005),
            chime: Modal::new(&BAR_RATIOS, &BAR_T60, &BAR_LEVEL, 0x7_9006),
            door_prev: 0.0,
            door_speed: 0.0,
            door_motor: Phasor::default(),
            door_roll: Svf::default(),
            door_low: Svf::default(),
        }
    }

    fn snap(&mut self, x: &[f32], p: &ElevatorParams) {
        self.v = x[0] * p.max_mps;
        self.moving = x[0] > 0.02;
        self.door_prev = x[1];
        self.door_speed = 0.0;
    }

    fn block(&mut self, x: &[f32], p: &ElevatorParams, out: &mut [f32]) {
        let (sr, n) = (self.sr, out.len());
        let dt = n as f32 / sr;
        let (speed_in, door) = (x[0], x[1]);
        let v = slew(&mut self.v, speed_in * p.max_mps, 1.0, 1.0, dt);
        let vf = (v / p.max_mps).clamp(0.0, 1.2);
        // Start and stop: the brake lifts with a click, sets with a clunk, the chime on arrival.
        let (mut xb, mut xth, mut xch) = (0.0, 0.0, 0.0);
        if !self.moving && speed_in > 0.02 {
            self.moving = true;
            self.travelled = 0.0;
            xb = p.brake * 0.5;
            xth = p.brake * 0.3;
        } else if self.moving && vf < 0.01 && speed_in < 0.01 {
            self.moving = false;
            xb = p.brake * 0.3;
            xth = p.brake * 0.7;
            if self.travelled > 1.5 {
                xch = p.chime * 0.25;
            }
        }
        // Landings and rail joints pass at the car's speed.
        let ds = v as f64 * dt as f64;
        let floor = (self.pos / p.floor_m as f64).floor();
        let rail = ((self.pos + 1.3) / p.rail_m as f64).floor();
        self.pos += ds;
        self.travelled += ds as f32;
        if (self.pos / p.floor_m as f64).floor() != floor {
            self.whoosh.trigger(p.floors * (0.3 + 0.7 * vf.min(1.0)));
        }
        let mut xr = 0.0;
        if ((self.pos + 1.3) / p.rail_m as f64).floor() != rail {
            xr = p.rails * (0.3 + 0.7 * vf.min(1.0)) * self.rng.range(0.6, 1.0) * 0.4;
        }
        let run = smoothstep(vf * 8.0);
        let motor_hz = p.motor_hz * (0.25 + 0.75 * vf);
        let motor_gain = p.motor * run * (0.5 + 0.5 * vf) * 0.2;
        let inverter_hz = 2200.0 + 1200.0 * vf;
        let inverter_gain = p.inverter * run * 0.02;
        self.rumble.set(FilterMode::LowPass, 230.0, 0.3, sr);
        self.rush.set(FilterMode::BandPass, 420.0, 0.2, sr);
        self.whoosh_bp.set(FilterMode::BandPass, 650.0, 0.25, sr);
        let rumble_gain = p.rumble * run * (0.3 + 0.7 * vf) * 0.9;
        let rush_gain = p.rumble * vf.powf(1.5) * 1.1;
        self.rail.set(380.0, 1.0, 900.0, 0.01, sr);
        self.brake.set(1700.0, 1.0, 2500.0, 0.004, sr);
        self.thud.set(95.0, 0.5, 1.0, sr);
        self.chime.set(p.chime_hz, 0.5, 1.0, sr);
        // The door: its motor runs while the position changes; it thuds at either end.
        let dv = (door - self.door_prev) / dt;
        let reached_end = (door <= 0.002 && self.door_prev > 0.002) || (door >= 0.998 && self.door_prev < 0.998);
        let shut = door <= 0.002;
        self.door_prev = door;
        let ds_door = slew(&mut self.door_speed, (dv.abs() / 0.6).min(1.5), 0.05, 0.08, dt);
        let door_gain = p.door * smoothstep(ds_door * 3.0);
        self.door_roll.set(FilterMode::BandPass, 3500.0, 0.15, sr);
        self.door_low.set(FilterMode::LowPass, 200.0, 0.3, sr);
        let door_inc = p.door_hz * (0.6 + 0.6 * ds_door.min(1.0)) / sr;
        if reached_end {
            xth += p.door * if shut { 0.6 } else { 0.25 };
            xr += p.door * if shut { 0.3 } else { 0.1 };
        }
        let (motor_inc, inverter_inc) = (motor_hz / sr, inverter_hz / sr);
        let gain = p.gain;
        let dts = 1.0 / sr;
        for (i, o) in out.iter_mut().enumerate() {
            let w = self.noise.white();
            self.motor.tick(motor_inc);
            let m = self.motor.phase * TAU;
            let motor = (m.sin() + 0.6 * (2.0 * m).sin() + 0.3 * (3.0 * m).sin()) * motor_gain;
            self.inverter.tick(inverter_inc);
            let inverter = self.inverter.sin() * inverter_gain;
            let pink = self.noise.pink();
            let ride = self.rumble.tick(self.brown.tick(w)) * rumble_gain + self.rush.tick(pink) * rush_gain;
            let wh = self.whoosh.tick(dts, 0.12, 0.05, 0.35, 0.0, 0.3);
            let whoosh = self.whoosh_bp.tick(pink) * wh * 1.2;
            let first = i == 0;
            let clicks = self.rail.tick(if first { xr } else { 0.0 }, w, 0.8, 0.5, sr)
                + self.brake.tick(if first { xb } else { 0.0 }, w, 0.5, 0.4, sr)
                + self.thud.tick(if first { xth } else { 0.0 }, 0.23, 0.3, sr)
                + self.chime.tick(if first { xch } else { 0.0 }, 0.23, 0.0, sr);
            let mut door_s = 0.0;
            if door_gain > 1e-4 {
                self.door_motor.tick(door_inc);
                let d = self.door_motor.phase * TAU;
                door_s = ((d.sin() + 0.4 * (2.0 * d).sin()) * 0.08 + self.door_roll.tick(w) * 0.25 + self.door_low.tick(pink) * 0.6) * door_gain;
            }
            *o = (motor + inverter + ride + whoosh + clicks + door_s) * gain;
        }
        self.rail.end_block();
        self.brake.end_block();
        self.thud.end_block();
        self.chime.end_block();
    }
}
