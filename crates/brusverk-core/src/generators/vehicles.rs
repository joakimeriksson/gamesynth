//! Vehicle and machine generators: jet, anti-grav hover, combustion engine, electric motor,
//! rotor.

use crate::blocks::{hz_coef, settle_coef, DelayLine, Dust, OnePole, Phasor, SlowNoise};
use crate::filter::{FilterMode, Svf};
use crate::jet::{JetEngine, JetParams, JetPreset};
use crate::math::{soft_clip, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::osc::{Oscillator, Waveform};
use crate::params::{exp, int, lin, ParamKind, GAIN, UNIT};

#[inline]
fn fade_in(x: f32) -> f32 {
    let t = (x * 12.0).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Move `y` toward `target` with separate rise/fall settle times.
#[inline]
fn slew(y: &mut f32, target: f32, up: f32, down: f32, dt: f32) -> f32 {
    let secs = if target > *y { up } else { down };
    *y += (target - *y) * settle_coef(secs, dt);
    *y
}

// ---------------------------------------------------------------------------------------------
// Jet (wraps the dedicated engine in `crate::jet`)
// ---------------------------------------------------------------------------------------------

/// Turbine / jet engine. See [`crate::jet`] for the sound model.
pub struct Jet {
    engine: JetEngine,
}

impl Generator for Jet {
    type P = JetParams;
    const NAME: &'static str = "jet";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Turbine engine with spool inertia, afterburner, wind and damage sputter.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "throttle", default: 0.0, doc: "Engine throttle; RPM follows with spool inertia" },
        InputSpec { name: "boost", default: 0.0, doc: "Afterburner amount" },
        InputSpec { name: "speed", default: 0.0, doc: "Airspeed, drives the wind layer" },
        InputSpec { name: "damage", default: 0.0, doc: "Random sputtering drop-outs" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const LIMIT: bool = false;

    fn presets() -> Vec<(&'static str, JetParams)> {
        JetPreset::ALL.iter().skip(1).map(|p| (p.name(), p.params())).collect()
    }

    fn new(sample_rate: f32) -> Self {
        Jet { engine: JetEngine::new(sample_rate, JetParams::default()) }
    }

    fn snap(&mut self, x: &[f32], p: &JetParams) {
        self.engine.set_params(*p);
        self.engine.set_throttle(x[0]);
        self.engine.set_boost(x[1]);
        self.engine.set_speed(x[2]);
        self.engine.snap_rpm();
    }

    fn rpm(&self) -> Option<f32> {
        Some(self.engine.rpm())
    }

    fn block(&mut self, x: &[f32], p: &JetParams, out: &mut [f32]) {
        self.engine.set_params(*p);
        self.engine.set_throttle(x[0]);
        self.engine.set_boost(x[1]);
        self.engine.set_speed(x[2]);
        self.engine.set_damage(x[3]);
        self.engine.render_mono(out);
    }
}

// ---------------------------------------------------------------------------------------------
// Hover (anti-gravity craft)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Anti-gravity field: pulsing detuned core hum, sub, airy shimmer through a slow flanger.
    HoverParams / HoverParamId {
        base_hz: "core/hz" = 62.0, exp(30.0, 200.0);
        detune: "core/detune_cents" = 9.0, lin(0.0, 40.0);
        core_level: "core/level" = 0.7, UNIT;
        sub_level: "core/sub" = 0.5, UNIT;
        response: "core/response" = 0.35, lin(0.02, 3.0);
        pulse_hz: "field/pulse_hz" = 9.0, lin(1.0, 40.0);
        pulse_depth: "field/pulse_depth" = 0.35, UNIT;
        shimmer_level: "field/shimmer" = 0.35, UNIT;
        shimmer_hz: "field/shimmer_hz" = 2400.0, exp(500.0, 9000.0);
        sweep_ms: "field/sweep_ms" = 2.2, lin(0.3, 10.0);
        sweep_feedback: "field/sweep_feedback" = 0.6, lin(0.0, 0.9);
        strain_drive: "strain/drive" = 0.6, UNIT;
        gain: "master/gain" = 0.9, GAIN;
    }
}

pub struct Hover {
    sr: f32,
    thrust: f32,
    osc: [Oscillator; 2],
    sub: Phasor,
    pulse: Phasor,
    rough: Phasor,
    noise: Noise,
    core_lp: Svf,
    shimmer: [Svf; 2],
    drift: [SlowNoise; 3],
    sweep: DelayLine,
}

impl Generator for Hover {
    type P = HoverParams;
    const NAME: &'static str = "hover";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Anti-gravity craft field: pulsing hum, sub and sweeping shimmer.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "thrust", default: 0.3, doc: "Field power; raises pitch, brightness and pulse rate" },
        InputSpec { name: "height", default: 0.5, doc: "0 = hugging the ground (more sub, faster pulse)" },
        InputSpec { name: "strain", default: 0.0, doc: "Overload growl, e.g. hard cornering or damage" },
    ];

    fn presets() -> Vec<(&'static str, HoverParams)> {
        vec![
            ("Heavy hauler", HoverParams { base_hz: 41.0, pulse_hz: 5.0, sub_level: 0.8, shimmer_level: 0.2, response: 0.8, ..Default::default() }),
            ("Scout", HoverParams { base_hz: 96.0, pulse_hz: 16.0, shimmer_level: 0.55, shimmer_hz: 3600.0, response: 0.15, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Hover {
            sr,
            thrust: 0.0,
            osc: [Oscillator::new(0x40_0001), Oscillator::new(0x40_0002)],
            sub: Phasor::default(),
            pulse: Phasor::default(),
            rough: Phasor::default(),
            noise: Noise::new(0x40_0003),
            core_lp: Svf::default(),
            shimmer: [Svf::default(); 2],
            drift: [SlowNoise::new(0x40_0004), SlowNoise::new(0x40_0005), SlowNoise::new(0x40_0006)],
            sweep: DelayLine::new((0.02 * sr) as usize),
        }
    }

    fn snap(&mut self, x: &[f32], _p: &HoverParams) {
        self.thrust = x[0];
    }

    fn block(&mut self, x: &[f32], p: &HoverParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let t = slew(&mut self.thrust, x[0], p.response, p.response * 1.5, dt);
        let (height, strain) = (x[1], x[2]);
        let f = p.base_hz * (1.0 + 0.9 * t);
        let inc = [f / sr, f * (p.detune / 1200.0).exp2() / sr];
        self.core_lp.set(FilterMode::LowPass, 200.0 + 2500.0 * t * t, 0.3, sr);
        let d = [self.drift[0].advance(0.3, dt), self.drift[1].advance(0.23, dt), self.drift[2].advance(0.17, dt)];
        self.shimmer[0].set(FilterMode::BandPass, p.shimmer_hz * (1.0 + 0.3 * d[0]), 0.9, sr);
        self.shimmer[1].set(FilterMode::BandPass, p.shimmer_hz * 1.5 * (1.0 + 0.3 * d[1]), 0.9, sr);
        let pulse_inc = p.pulse_hz * (0.6 + 0.8 * t) * (1.3 - 0.5 * height) / sr;
        let sub_gain = p.sub_level * (1.0 + 0.8 * (1.0 - height));
        let shimmer_gain = p.shimmer_level * (0.2 + 0.8 * t) * 1.5;
        let sweep = p.sweep_ms * (1.0 + 0.5 * d[2]) * 0.001 * sr;
        let drive = 1.0 + strain * p.strain_drive * 8.0;
        let level = (0.25 + 0.75 * t) * p.gain * 0.5;
        for s in out.iter_mut() {
            let core = self.osc[0].next(Waveform::Saw, inc[0], 0.5) + self.osc[1].next(Waveform::Saw, inc[1], 0.5);
            self.sub.tick(inc[0] * 0.5);
            self.pulse.tick(pulse_inc);
            self.rough.tick(31.0 / sr);
            let am = 1.0 - p.pulse_depth * (0.5 + 0.5 * self.pulse.sin());
            let w = self.noise.white();
            let shimmer = self.shimmer[0].tick(w) + self.shimmer[1].tick(w);
            let bus = (self.core_lp.tick(core) * 0.5 * p.core_level + self.sub.sin() * sub_gain) * am + shimmer * shimmer_gain;
            let y = bus + self.sweep.read(sweep) * p.sweep_feedback;
            self.sweep.write(y);
            let rough = 1.0 - 0.4 * strain * (0.5 + 0.5 * self.rough.sin());
            *s = soft_clip(y * drive) / (1.0 + 0.25 * (drive - 1.0)) * rough * level;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Combustion engine
// ---------------------------------------------------------------------------------------------

pub const ENGINE_CYCLES: [&str; 2] = ["Four-stroke", "Two-stroke"];
pub const OFF_ON: [&str; 2] = ["Off", "On"];

model_params! {
    /// Piston engine: per-cylinder firing pulses with combustion noise through exhaust resonances.
    ///
    /// Revs either follow `throttle` with rev_up / rev_down inertia, or, with
    /// `engine/external_rpm` on, come straight from the `rpm` input (a geared vehicle knows its
    /// RPM: shifts, limiter bounces, clutch-in). The parameters after `master/gain` were added
    /// later; at their defaults the engine sounds exactly as before.
    CombustionParams / CombustionParamId {
        cylinders: "engine/cylinders" = 4.0, int(1, 12);
        idle_rpm: "engine/idle_rpm" = 900.0, lin(300.0, 3000.0);
        max_rpm: "engine/max_rpm" = 7000.0, lin(2000.0, 16000.0);
        rev_up: "engine/rev_up" = 0.8, lin(0.05, 6.0);
        rev_down: "engine/rev_down" = 1.6, lin(0.05, 6.0);
        roughness: "engine/roughness" = 0.3, UNIT;
        pulse_sharp: "engine/pulse_sharpness" = 8.0, lin(2.0, 30.0);
        noise_level: "engine/combustion_noise" = 0.4, UNIT;
        exhaust_hz: "exhaust/hz" = 140.0, exp(40.0, 800.0);
        exhaust_res: "exhaust/resonance" = 0.7, lin(0.0, 0.95);
        drive: "exhaust/drive" = 0.4, UNIT;
        burble: "exhaust/overrun_burble" = 0.5, UNIT;
        intake_level: "intake/level" = 0.3, UNIT;
        gain: "master/gain" = 0.9, GAIN;
        external_rpm: "engine/external_rpm" = 0.0, ParamKind::Enum(&OFF_ON);
        cycle: "engine/cycle" = 0.0, ParamKind::Enum(&ENGINE_CYCLES);
        lope: "engine/cam_lope" = 0.0, UNIT;
        limiter: "engine/rev_limiter" = 0.0, UNIT;
        backfire: "exhaust/backfire" = 0.0, UNIT;
        blower_level: "blower/level" = 0.0, UNIT;
        blower_ratio: "blower/ratio" = 16.0, lin(4.0, 40.0);
        boost_drive: "boost/drive" = 0.5, UNIT;
        wear: "damage/wear" = 0.0, UNIT;
        misfire: "damage/misfire" = 0.6, UNIT;
        rattle: "damage/rattle" = 0.5, UNIT;
        leak: "damage/exhaust_leak" = 0.5, UNIT;
    }
}

/// Firing-interval pattern of a lumpy cam: alternating early and late cylinders.
const LOPE_TIMING: [f32; 12] = [0.9, -0.6, 0.3, -1.0, 0.7, -0.2, 1.0, -0.8, 0.4, -0.5, 0.6, -0.3];
const LOPE_LEVEL: [f32; 12] = [0.6, -0.4, 1.0, -0.7, 0.2, 0.8, -0.5, 0.3, -0.9, 0.5, -0.2, 0.7];

pub struct Combustion {
    sr: f32,
    rev: f32,
    fire: Phasor,
    cyl: usize,
    amp: f32,
    /// Cycle-to-cycle timing variation of the current firing.
    timing: f32,
    cyl_gain: [f32; 12],
    noise: Noise,
    dust: Dust,
    dc: OnePole,
    exhaust: [Svf; 2],
    body: OnePole,
    intake: Svf,
    // Added features draw from their own random stream, so the original sound is unchanged
    // while they are off.
    rng: crate::math::Rng,
    /// Throttle a moment ago, for detecting a sharp lift-off.
    thr_slow: f32,
    backfire_cooldown: f32,
    pops_left: u32,
    pop_wait: f32,
    bang: f32,
    limiter_phase: f32,
    rattle_env: f32,
    rattle: Svf,
    leak: Svf,
    bang_pipe: [Svf; 2],
    blower: [Phasor; 2],
    /// Two-stroke idle "ring-ding": skip alternate firings at low throttle.
    skip_next: bool,
    /// Smoothed inputs, in `INPUTS` order.
    x: [f32; 5],
}

impl Combustion {
    /// Current engine speed in RPM.
    pub fn rpm(&self, p: &CombustionParams) -> f32 {
        p.idle_rpm + (p.max_rpm - p.idle_rpm) * self.rev
    }

    /// Current revs as 0..1 between idle and max RPM (the scale of the `rpm` input).
    pub fn rev(&self) -> f32 {
        self.rev
    }
}

impl Generator for Combustion {
    type P = CombustionParams;
    const NAME: &'static str = "combustion";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Piston engine: cylinders, rev inertia or game-driven RPM, exhaust resonance, backfires, blower, damage.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "throttle", default: 0.0, doc: "Combustion intensity; also drives the revs (with inertia) unless engine/external_rpm is on" },
        InputSpec { name: "load", default: 0.3, doc: "Engine load: louder, fuller, more intake" },
        InputSpec { name: "rpm", default: 0.0, doc: "Revs 0..1 between idle_rpm and max_rpm, used when engine/external_rpm is on (gears, limiter, clutch)" },
        InputSpec { name: "damage", default: 0.0, doc: "Wrecked engine: misfires, a dead cylinder, rattle, exhaust-leak hiss, louder backfires" },
        InputSpec { name: "boost", default: 0.0, doc: "Nitro / blower boost: supercharger whine and extra drive" },
    ];
    // Inputs are smoothed in `block`: 50 ms for throttle and load (as before), 12 ms for a
    // game-driven rpm so it follows shifts and limiter bounces crisply.
    const INPUT_SMOOTH_SECS: f32 = 0.0;

    fn presets() -> Vec<(&'static str, CombustionParams)> {
        vec![
            ("V8 muscle", CombustionParams { cylinders: 8.0, idle_rpm: 700.0, max_rpm: 6200.0, exhaust_hz: 95.0, exhaust_res: 0.8, roughness: 0.45, drive: 0.6, ..Default::default() }),
            ("Motorbike", CombustionParams { cylinders: 2.0, idle_rpm: 1300.0, max_rpm: 11000.0, rev_up: 0.4, rev_down: 0.9, exhaust_hz: 210.0, pulse_sharp: 12.0, ..Default::default() }),
            ("Diesel truck", CombustionParams { cylinders: 6.0, idle_rpm: 600.0, max_rpm: 2800.0, rev_up: 2.2, rev_down: 2.8, exhaust_hz: 70.0, noise_level: 0.7, roughness: 0.5, burble: 0.1, ..Default::default() }),
            ("Lawnmower", CombustionParams { cylinders: 1.0, idle_rpm: 1600.0, max_rpm: 3600.0, exhaust_hz: 260.0, exhaust_res: 0.5, roughness: 0.6, noise_level: 0.6, gain: 1.4, ..Default::default() }),
            ("Blown V8", CombustionParams {
                cylinders: 8.0, idle_rpm: 750.0, max_rpm: 6500.0, exhaust_hz: 88.0, exhaust_res: 0.82, roughness: 0.5, noise_level: 0.45, drive: 0.72,
                burble: 0.7, lope: 0.75, limiter: 0.6, backfire: 0.55, blower_level: 0.55, blower_ratio: 14.0, ..Default::default()
            }),
            ("Buggy flat-four", CombustionParams {
                cylinders: 4.0, idle_rpm: 1000.0, max_rpm: 7500.0, rev_up: 0.5, rev_down: 0.9, exhaust_hz: 185.0, exhaust_res: 0.55, roughness: 0.4,
                pulse_sharp: 14.0, noise_level: 0.68, drive: 0.62, intake_level: 0.5, lope: 0.3, limiter: 0.5, backfire: 0.3, ..Default::default()
            }),
            ("Dirt bike 2-stroke", CombustionParams {
                cylinders: 1.0, cycle: 1.0, idle_rpm: 1800.0, max_rpm: 12000.0, rev_up: 0.25, rev_down: 0.5, exhaust_hz: 320.0, exhaust_res: 0.78,
                roughness: 0.45, pulse_sharp: 16.0, noise_level: 0.72, drive: 0.5, limiter: 0.7, backfire: 0.2, gain: 1.15, ..Default::default()
            }),
            ("Rattletrap V8", CombustionParams {
                cylinders: 8.0, idle_rpm: 600.0, max_rpm: 5200.0, exhaust_hz: 82.0, exhaust_res: 0.65, roughness: 0.85, noise_level: 0.6, drive: 0.66,
                burble: 0.8, lope: 0.4, limiter: 0.4, backfire: 0.7, wear: 0.35, rattle: 0.7, ..Default::default()
            }),
        ]
    }

    fn new(sr: f32) -> Self {
        let mut rng = crate::math::Rng::new(0x41_0001);
        let mut cyl_gain = [1.0; 12];
        cyl_gain.iter_mut().for_each(|g| *g = rng.range(0.72, 1.0));
        Combustion {
            sr,
            rev: 0.0,
            fire: Phasor::default(),
            cyl: 0,
            amp: 1.0,
            timing: 1.0,
            cyl_gain,
            noise: Noise::new(0x41_0002),
            dust: Dust::new(0x41_0003),
            dc: OnePole::default(),
            exhaust: [Svf::default(); 2],
            body: OnePole::default(),
            intake: Svf::default(),
            rng: crate::math::Rng::new(crate::blocks::mix_seed(0x41_0004)),
            thr_slow: 0.0,
            backfire_cooldown: 0.0,
            pops_left: 0,
            pop_wait: 0.0,
            bang: 0.0,
            limiter_phase: 0.0,
            rattle_env: 0.0,
            rattle: Svf::default(),
            leak: Svf::default(),
            bang_pipe: [Svf::default(); 2],
            blower: [Phasor::default(); 2],
            skip_next: false,
            x: [0.0, 0.3, 0.0, 0.0, 0.0],
        }
    }

    fn snap(&mut self, x: &[f32], p: &CombustionParams) {
        self.x.copy_from_slice(x);
        self.rev = if p.external_rpm >= 0.5 { x[2] } else { x[0] };
        self.thr_slow = x[0];
    }

    fn rpm(&self) -> Option<f32> {
        Some(self.rev)
    }

    fn block(&mut self, x: &[f32], p: &CombustionParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        for (k, (sm, target)) in self.x.iter_mut().zip(x).enumerate() {
            *sm += (*target - *sm) * settle_coef(if k == 2 { 0.012 } else { 0.05 }, dt);
        }
        let x = self.x;
        let (throttle, load, damage_in, boost) = (x[0], x[1], x[3], x[4]);
        let external = p.external_rpm >= 0.5;
        let rev = if external {
            self.rev = x[2];
            self.rev
        } else {
            slew(&mut self.rev, throttle, p.rev_up, p.rev_down, dt)
        };
        let damage = (damage_in + p.wear).min(1.0);
        let two_stroke = p.cycle >= 0.5;
        let n_cyl = (p.cylinders.round() as usize).clamp(1, 12);
        let fire_hz = self.rpm(p) / 60.0 * n_cyl as f32 * if two_stroke { 1.0 } else { 0.5 };
        let inc = fire_hz / sr;
        self.exhaust[0].set(FilterMode::BandPass, p.exhaust_hz * (0.7 + 0.9 * rev), p.exhaust_res, sr);
        // The second resonance follows the firing fundamental so the tone stays full at high revs.
        self.exhaust[1].set(FilterMode::BandPass, fire_hz.max(p.exhaust_hz * 1.5), p.exhaust_res * 0.8, sr);
        self.intake.set(FilterMode::BandPass, 300.0 + 1500.0 * rev, 0.4, sr);
        let (dc_coef, body_coef) = (hz_coef(20.0, sr), hz_coef(1200.0, sr));
        // Lifting off at high revs: unburnt fuel pops in the exhaust.
        let overrun = (rev - throttle - 0.15).max(0.0) * p.burble;
        let pop_p = 40.0 * overrun / sr;
        let drive = 1.0 + p.drive * 5.0 + boost * p.boost_drive * 4.0;
        let intake_gain = p.intake_level * (0.2 + 0.8 * throttle) * 1.5 * (1.0 + boost);
        let level = (0.35 + 0.65 * load.max(throttle * 0.5)) * (0.7 + 0.6 * rev) * p.gain * 1.1 * (1.0 + 0.25 * boost);

        // ---- added features (all inert at their defaults) ----
        // Backfire: a sharp lift-off at speed sets off one to three bangs in the exhaust.
        self.thr_slow += (throttle - self.thr_slow) * settle_coef(0.2, dt);
        self.backfire_cooldown = (self.backfire_cooldown - dt).max(0.0);
        if p.backfire > 0.0 && self.pops_left == 0 && self.backfire_cooldown == 0.0 && self.thr_slow - throttle > 0.3 && rev > 0.35 {
            self.backfire_cooldown = 0.7;
            if self.rng.chance((p.backfire * (0.7 + 0.6 * damage)).min(1.0)) {
                self.pops_left = 1 + self.rng.next_u32() % 3;
                self.pop_wait = self.rng.range(0.0, 0.05);
            }
        }
        // Rev limiter: bouncing off the redline cuts the ignition in bursts. Only with a
        // game-driven rpm: when throttle drives the revs, max revs just means flat out.
        let limiting = external && p.limiter > 0.0 && rev > 0.97 && throttle > 0.6;
        if limiting {
            self.limiter_phase = (self.limiter_phase + dt * (9.0 + 5.0 * self.rng.next_f32())).fract();
        }
        let cut = limiting && self.limiter_phase < 0.25 + 0.45 * p.limiter;
        let misfire_p = damage.powf(1.5) * 0.35 * p.misfire;
        // The last cylinder dies as the engine gets wrecked.
        let dead = ((damage - 0.5) * 2.0).clamp(0.0, 1.0) * p.misfire;
        let rattle_on = damage * p.rattle > 0.0;
        if rattle_on {
            self.rattle.set(FilterMode::BandPass, 2600.0 * (0.6 * self.rng.next_bipolar()).exp2(), 0.9, sr);
        }
        let leak_gain = damage * p.leak * 0.5;
        if leak_gain > 0.0 {
            self.leak.set(FilterMode::HighPass, 2500.0, 0.3, sr);
        }
        let blower_gain = p.blower_level * (0.25 + 0.4 * throttle + 0.6 * boost) * (0.3 + 0.7 * rev) * 0.12;
        let blower_inc = self.rpm(p) / 60.0 * p.blower_ratio / sr;
        let rattle_decay = (-1.0 / (0.004 * sr)).exp();
        let bang_decay = (-1.0 / (0.03 * sr)).exp();
        let bang_gain = 0.9 * (0.6 + 0.8 * damage) * p.backfire.sqrt();
        if p.backfire > 0.0 {
            // A backfire is a bang in the pipe: a low boom plus a crack at the tailpipe.
            self.bang_pipe[0].set(FilterMode::LowPass, p.exhaust_hz * 3.0, 0.4, sr);
            self.bang_pipe[1].set(FilterMode::BandPass, 1800.0, 0.3, sr);
        }

        for s in out.iter_mut() {
            if self.fire.tick(inc * self.timing) {
                self.cyl = (self.cyl + 1) % n_cyl;
                self.amp = self.cyl_gain[self.cyl] * (1.0 + p.roughness * 0.5 * self.dust.rng().next_bipolar());
                // No two combustion cycles take exactly as long; this is what keeps the upper
                // harmonics from sounding like a clean synth buzz.
                self.timing = 1.0 + (0.015 + 0.06 * p.roughness) * self.dust.rng().next_bipolar();
                if p.lope > 0.0 {
                    // A lumpy cam: an uneven, repeating firing pattern that smooths out with revs.
                    let lumpy = p.lope * (1.0 - rev).max(0.0);
                    self.timing *= 1.0 + 0.22 * lumpy * LOPE_TIMING[self.cyl];
                    self.amp *= 1.0 + 0.4 * lumpy * LOPE_LEVEL[self.cyl];
                }
                if two_stroke && throttle < 0.35 {
                    // Off the pipe a two-stroke fires every other turn, irregularly: ring-ding.
                    if self.skip_next {
                        self.amp = 0.0;
                    }
                    self.skip_next = !self.skip_next && self.rng.chance(0.8 - throttle);
                }
                if cut || (misfire_p > 0.0 && self.rng.chance(misfire_p)) || (dead > 0.0 && self.cyl == n_cyl - 1 && n_cyl > 1) {
                    self.amp *= if cut { 0.0 } else { 1.0 - if self.cyl == n_cyl - 1 { dead } else { 1.0 } };
                }
                if rattle_on && self.rng.chance(damage * p.rattle * 0.6) {
                    self.rattle_env = self.rattle_env.max(self.rng.range(0.4, 1.0));
                }
            }
            let pulse = self.amp * (-self.fire.phase * p.pulse_sharp).exp();
            let w = self.noise.white();
            let bang = pulse * (0.6 + p.noise_level * 1.5 * w) + self.dust.tick(pop_p) * 1.5;
            let mut backfire = 0.0;
            if self.pops_left > 0 || self.bang > 1e-4 {
                if self.pops_left > 0 {
                    self.pop_wait -= 1.0 / sr;
                    if self.pop_wait <= 0.0 {
                        self.bang = 1.0;
                        self.pops_left -= 1;
                        self.pop_wait = self.rng.range(0.04, 0.16);
                    }
                }
                let b = self.bang * (0.4 + 0.6 * w);
                backfire = (self.bang_pipe[0].tick(b) * 2.5 + self.bang_pipe[1].tick(b)) * bang_gain;
                self.bang *= bang_decay;
            }
            let x0 = self.dc.hp(bang, dc_coef);
            let pipe = self.exhaust[0].tick(x0) + 0.8 * self.exhaust[1].tick(x0) + 0.35 * self.body.lp(x0, body_coef);
            let intake = self.intake.tick(self.noise.pink()) * intake_gain * (0.5 + 0.5 * pulse);
            let mut y = soft_clip((pipe + intake) * drive) / (1.0 + 0.2 * (drive - 1.0)) + backfire;
            if rattle_on {
                y += self.rattle.tick(w * self.rattle_env) * 1.5;
                self.rattle_env *= rattle_decay;
            }
            if leak_gain > 0.0 {
                y += self.leak.tick(w) * pulse * leak_gain;
            }
            if blower_gain > 0.0 {
                self.blower[0].tick(blower_inc);
                self.blower[1].tick(blower_inc * 2.0);
                y += (self.blower[0].sin() + 0.45 * self.blower[1].sin()) * blower_gain;
            }
            *s = y * level;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Electric motor / servo
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Electric motor: harmonic whine, PWM inverter tone with sidebands, gear mesh noise.
    MotorParams / MotorParamId {
        max_hz: "motor/max_hz" = 420.0, exp(40.0, 2500.0);
        spin_up: "motor/spin_up" = 0.6, lin(0.02, 6.0);
        spin_down: "motor/spin_down" = 1.2, lin(0.02, 6.0);
        whine_level: "motor/whine" = 0.5, UNIT;
        harmonics: "motor/harmonics" = 0.5, UNIT;
        pwm_hz: "inverter/hz" = 5200.0, exp(1000.0, 12000.0);
        pwm_level: "inverter/level" = 0.15, UNIT;
        gear_ratio: "gears/ratio" = 7.3, lin(1.0, 24.0);
        gear_level: "gears/level" = 0.3, UNIT;
        gain: "master/gain" = 0.9, GAIN;
    }
}

pub struct Motor {
    sr: f32,
    spin: f32,
    rotor: Phasor,
    pwm: Phasor,
    noise: Noise,
    gear: Svf,
}

impl Generator for Motor {
    type P = MotorParams;
    const NAME: &'static str = "motor";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Electric motor or servo: whine harmonics, inverter tone, gear mesh.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.0, doc: "Target speed; the rotor follows with spin inertia" },
        InputSpec { name: "load", default: 0.3, doc: "Torque demand: louder, more gear and inverter noise" },
    ];

    fn presets() -> Vec<(&'static str, MotorParams)> {
        vec![
            ("Servo", MotorParams { max_hz: 900.0, spin_up: 0.08, spin_down: 0.1, gear_ratio: 14.0, gear_level: 0.6, pwm_level: 0.05, ..Default::default() }),
            ("EV drivetrain", MotorParams { max_hz: 650.0, spin_up: 2.5, spin_down: 3.5, pwm_hz: 8000.0, pwm_level: 0.25, gear_ratio: 9.1, ..Default::default() }),
            ("Drone prop", MotorParams { max_hz: 260.0, spin_up: 0.2, spin_down: 0.35, harmonics: 0.9, gear_level: 0.0, pwm_level: 0.1, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Motor { sr, spin: 0.0, rotor: Phasor::default(), pwm: Phasor::default(), noise: Noise::new(0x42_0001), gear: Svf::default() }
    }

    fn snap(&mut self, x: &[f32], _p: &MotorParams) {
        self.spin = x[0];
    }

    fn block(&mut self, x: &[f32], p: &MotorParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let spin = slew(&mut self.spin, x[0], p.spin_up, p.spin_down, dt);
        let load = x[1];
        let f = p.max_hz * spin;
        let slope = 2.0 - 1.5 * p.harmonics;
        const ORDERS: [f32; 5] = [1.0, 2.0, 3.0, 4.0, 6.0];
        let mut amps = [0.0f32; 5];
        for (a, h) in amps.iter_mut().zip(ORDERS) {
            // Drop partials that would alias.
            *a = if f * h < sr * 0.45 { h.powf(-slope) } else { 0.0 };
        }
        self.gear.set(FilterMode::BandPass, (f * p.gear_ratio).clamp(40.0, sr * 0.4), 0.85, sr);
        let gear_gain = p.gear_level * (0.3 + 0.7 * load) * 2.0;
        let pwm_gain = p.pwm_level * (0.3 + 0.7 * load);
        let level = fade_in(spin) * spin.sqrt() * (0.3 + 0.7 * load) * p.gain * 2.5;
        let (inc, pwm_inc) = (f / sr, p.pwm_hz / sr);
        for s in out.iter_mut() {
            self.rotor.tick(inc);
            self.pwm.tick(pwm_inc);
            let ph = self.rotor.phase * TAU;
            let mut whine = 0.0;
            for (a, h) in amps.iter().zip(ORDERS) {
                whine += a * (ph * h).sin();
            }
            let pwm = self.pwm.sin() * (1.0 + 0.5 * ph.sin());
            let gear = self.gear.tick(self.noise.pink());
            *s = (whine * 0.5 * p.whine_level + pwm * pwm_gain + gear * gear_gain) * level;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Rotor (helicopter, propeller, big fan)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Rotor: blade slap, blade-pass thump, chopped downwash noise and a turbine whine.
    ///
    /// The slap was fitted to CC0 recordings of a UH-1 "Huey" and a Chinook (see
    /// `tools/sound_views.py`): every blade pass is a bipolar pressure pulse (a sharp push, then a
    /// longer, shallower pull, about 10 ms in all) wrapped in a short noise burst, 10-13 dB above the
    /// downwash between passes, and nearly the same every time (the envelope repeats with an
    /// autocorrelation of about 0.8). That steady beat is what makes it read as a helicopter.
    RotorParams / RotorParamId {
        blades: "rotor/blades" = 2.0, int(2, 8);
        max_bpf: "rotor/max_blade_hz" = 11.0, exp(4.0, 160.0);
        spin_up: "rotor/spin_up" = 3.0, lin(0.1, 12.0);
        spin_down: "rotor/spin_down" = 5.0, lin(0.1, 12.0);
        sharp: "rotor/sharpness" = 3.0, lin(1.0, 8.0);
        thump_level: "rotor/thump" = 0.2, UNIT;
        chop: "wash/chop" = 0.35, UNIT;
        wash_level: "wash/level" = 0.9, UNIT;
        wash_hz: "wash/hz" = 250.0, exp(100.0, 4000.0);
        turbine_level: "turbine/level" = 0.005, UNIT;
        turbine_hz: "turbine/hz" = 3200.0, exp(500.0, 9000.0);
        gain: "master/gain" = 1.0, GAIN;
        slap_level: "slap/level" = 0.8, UNIT;
        slap_ms: "slap/ms" = 10.0, lin(3.0, 30.0);
        slap_noise: "slap/noise" = 0.35, UNIT;
        slap_hz: "slap/hz" = 900.0, exp(300.0, 8000.0);
        slap_jitter: "slap/jitter" = 0.12, UNIT;
    }
}

pub struct Rotor {
    sr: f32,
    spin: f32,
    pass: Phasor,
    blade: usize,
    blade_amp: f32,
    turbine: [Phasor; 2],
    noise: Noise,
    thump: Svf,
    wash: Svf,
    // Slap: seconds since the last blade pass, this pass's pulse and burst sizes, the burst's
    // attack and decay envelopes, and its own noise so the old layers render exactly as before.
    since: f32,
    pulse_amp: f32,
    burst: [f32; 2],
    slap_noise: Noise,
    slap_lp: Svf,
    rng: crate::math::Rng,
}

/// One blade-slap pressure pulse at `x` = time / pulse length: a push over the first quarter,
/// then a pull three times as long and a third as deep, so it carries no DC.
fn slap_pulse(x: f32) -> f32 {
    if x < 0.25 {
        (x * 4.0 * std::f32::consts::PI).sin()
    } else if x < 1.0 {
        -((x - 0.25) / 0.75 * std::f32::consts::PI).sin() / 3.0
    } else {
        0.0
    }
}

impl Generator for Rotor {
    type P = RotorParams;
    const NAME: &'static str = "rotor";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Helicopter, propeller or fan: blade slap, blade thump, chopped wash, turbine whine.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "rpm", default: 0.0, doc: "Rotor speed target; follows with spin inertia" },
        InputSpec { name: "load", default: 0.5, doc: "Blade pitch / lift: heavier slap and thump" },
    ];

    fn presets() -> Vec<(&'static str, RotorParams)> {
        // Propeller plane and Ceiling fan have no slap and pin the old defaults, so they render
        // exactly as before the slap was added.
        let old = RotorParams {
            blades: 3.0, max_bpf: 22.0, thump_level: 0.7, chop: 0.75, wash_level: 0.6, wash_hz: 500.0, turbine_level: 0.15, slap_level: 0.0, gain: 0.9,
            ..Default::default()
        };
        vec![
            ("Attack chopper", RotorParams { blades: 4.0, max_bpf: 19.0, sharp: 4.5, thump_level: 0.3, turbine_level: 0.015, slap_level: 0.9, slap_ms: 8.0, slap_hz: 1800.0, gain: 1.5, ..Default::default() }),
            ("Propeller plane", RotorParams { blades: 3.0, max_bpf: 95.0, spin_up: 1.5, spin_down: 2.5, sharp: 1.6, wash_hz: 900.0, turbine_level: 0.0, ..old }),
            ("Ceiling fan", RotorParams { blades: 4.0, max_bpf: 9.0, spin_up: 6.0, spin_down: 9.0, thump_level: 0.25, wash_level: 0.35, wash_hz: 300.0, turbine_level: 0.0, ..old }),
        ]
    }

    fn new(sr: f32) -> Self {
        Rotor {
            sr,
            spin: 0.0,
            pass: Phasor::default(),
            blade: 0,
            blade_amp: 1.0,
            turbine: [Phasor::default(); 2],
            noise: Noise::new(0x43_0001),
            thump: Svf::default(),
            wash: Svf::default(),
            since: 1.0,
            pulse_amp: 0.0,
            burst: [0.0; 2],
            slap_noise: Noise::new(0x43_0002),
            slap_lp: Svf::default(),
            rng: crate::math::Rng::new(0x43_0003),
        }
    }

    fn snap(&mut self, x: &[f32], _p: &RotorParams) {
        self.spin = x[0];
    }

    fn block(&mut self, x: &[f32], p: &RotorParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let spin = slew(&mut self.spin, x[0], p.spin_up, p.spin_down, dt);
        let load = x[1];
        let bpf = p.max_bpf * spin;
        let n_blades = (p.blades.round() as usize).clamp(2, 8);
        self.thump.set(FilterMode::LowPass, bpf * 4.0 + 40.0, 0.2, sr);
        self.wash.set(FilterMode::BandPass, p.wash_hz * (0.7 + 0.6 * spin), 0.3, sr);
        self.slap_lp.set(FilterMode::LowPass, p.slap_hz, 0.1, sr);
        let thump_gain = p.thump_level * (0.5 + 0.5 * load) * 2.0;
        let wash_gain = p.wash_level * 2.5;
        let turbine_gain = p.turbine_level * spin * 0.5;
        // The slap needs airspeed over the blade: it grows with the square of the spin.
        let slap_gain = p.slap_level * (0.6 + 0.8 * load) * spin * spin * 1.5;
        let pulse_len = p.slap_ms * 0.001;
        let burst_k = [(-1.0 / (0.0007 * sr)).exp(), (-1.0 / (pulse_len * 0.7 * sr)).exp()];
        let (inc, t_inc) = (bpf / sr, p.turbine_hz * spin / sr);
        let level = fade_in(spin) * (0.3 + 0.7 * spin) * p.gain * 0.65;
        for s in out.iter_mut() {
            if self.pass.tick(inc) {
                self.blade = (self.blade + 1) % n_blades;
                // One blade tracks slightly off, giving the once-per-revolution lope.
                self.blade_amp = if self.blade == 0 { 0.82 } else { 1.0 };
                if p.slap_level > 0.0 {
                    let j = p.slap_jitter;
                    self.since = 0.0;
                    self.pulse_amp = self.blade_amp * self.rng.range(1.0 - j, 1.0 + j);
                    let b = p.slap_noise * self.blade_amp * self.rng.range(1.0 - 3.0 * j, 1.0 + j);
                    self.burst = [b, b];
                }
            }
            self.turbine[0].tick(t_inc);
            self.turbine[1].tick(t_inc * 1.31);
            let env = (0.5 + 0.5 * (self.pass.phase * TAU).cos()).powf(p.sharp) * self.blade_amp;
            let thump = self.thump.tick(env - 0.3) + 0.3 * self.pass.sin();
            let wash = self.wash.tick(self.noise.pink()) * (1.0 - p.chop + p.chop * env);
            let turbine = self.turbine[0].sin() + 0.5 * self.turbine[1].sin();
            let mut out_s = thump * thump_gain + wash * wash_gain + turbine * turbine_gain;
            if p.slap_level > 0.0 {
                let pulse = slap_pulse(self.since / pulse_len) * self.pulse_amp;
                self.burst[0] *= burst_k[0];
                self.burst[1] *= burst_k[1];
                let crack = self.slap_lp.tick(self.slap_noise.pink()) * (self.burst[1] - self.burst[0]) * 4.0;
                self.since += 1.0 / sr;
                out_s += (pulse + crack) * slap_gain;
            }
            *s = out_s * level;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Scrape (hull grinding along a wall)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Metal on track wall: noise forced through wandering metallic resonances (the screech),
    /// stick-slip chatter, a shower of sparks and low body rumble.
    ScrapeParams / ScrapeParamId {
        metal_hz: "metal/hz" = 1700.0, exp(400.0, 6000.0);
        metal_res: "metal/resonance" = 0.93, lin(0.5, 0.99);
        screech: "metal/wander" = 0.5, UNIT;
        grind_level: "metal/level" = 0.7, UNIT;
        chatter: "metal/chatter" = 0.5, UNIT;
        sparks_rate: "sparks/per_second" = 140.0, exp(5.0, 1500.0);
        sparks_level: "sparks/level" = 0.6, UNIT;
        rumble_level: "rumble/level" = 0.5, UNIT;
        gain: "master/gain" = 0.9, GAIN;
    }
}

pub struct Scrape {
    sr: f32,
    noise: Noise,
    sparks: Dust,
    spark_env: f32,
    metal: [Svf; 3],
    wander: [SlowNoise; 3],
    chatter: SlowNoise,
    spark_hp: Svf,
    rumble: Svf,
    brown: crate::blocks::Brown,
}

impl Generator for Scrape {
    type P = ScrapeParams;
    const NAME: &'static str = "scrape";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Hull grinding along a wall: metallic screech, chatter, sparks and rumble.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "pressure", default: 0.0, doc: "How hard the hull is pressed against the wall; 0 is silent" },
        InputSpec { name: "speed", default: 0.6, doc: "Sliding speed: pitch, chatter rate and spark count" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.02;

    fn presets() -> Vec<(&'static str, ScrapeParams)> {
        vec![
            ("Heavy hull", ScrapeParams { metal_hz: 900.0, rumble_level: 0.8, sparks_rate: 80.0, ..Default::default() }),
            ("Wing tip", ScrapeParams { metal_hz: 3200.0, metal_res: 0.96, rumble_level: 0.2, screech: 0.8, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Scrape {
            sr,
            noise: Noise::new(0x44_0001),
            sparks: Dust::new(0x44_0002),
            spark_env: 0.0,
            metal: [Svf::default(); 3],
            wander: [SlowNoise::new(0x44_0003), SlowNoise::new(0x44_0004), SlowNoise::new(0x44_0005)],
            chatter: SlowNoise::new(0x44_0006),
            spark_hp: Svf::default(),
            rumble: Svf::default(),
            brown: crate::blocks::Brown::default(),
        }
    }

    fn block(&mut self, x: &[f32], p: &ScrapeParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (pressure, speed) = (x[0], x[1]);
        const RATIOS: [f32; 3] = [1.0, 1.47, 2.09];
        for ((metal, wander), ratio) in self.metal.iter_mut().zip(self.wander.iter_mut()).zip(RATIOS) {
            // The contact patch keeps changing, so the resonances never sit still.
            let w = wander.advance(8.0 + 30.0 * speed, dt);
            let hz = p.metal_hz * ratio * (0.7 + 0.6 * speed) * (p.screech * 0.35 * w).exp2();
            metal.set(FilterMode::BandPass, hz.min(sr * 0.4), p.metal_res, sr);
        }
        self.spark_hp.set(FilterMode::HighPass, 3000.0, 0.1, sr);
        self.rumble.set(FilterMode::LowPass, 90.0 + 160.0 * speed, 0.3, sr);
        let stick = 1.0 - p.chatter * 0.7 * (0.5 + 0.5 * self.chatter.advance(25.0 + 70.0 * speed, dt));
        let spark_p = p.sparks_rate * pressure * (0.2 + 0.8 * speed) / sr;
        let spark_decay = (-1.0 / (0.0025 * sr)).exp();
        let grind = p.grind_level * stick * 0.34;
        let (spark_gain, rumble_gain) = (p.sparks_level * 1.2, p.rumble_level * 1.0 * stick);
        let level = pressure.powf(0.7) * (0.4 + 0.6 * speed) * p.gain;
        for s in out.iter_mut() {
            let w = self.noise.white();
            let mut y = 0.0;
            for m in self.metal.iter_mut() {
                y += m.tick(w);
            }
            let d = self.sparks.tick(spark_p);
            if d > 0.0 {
                self.spark_env = self.spark_env.max(d);
            }
            self.spark_env *= spark_decay;
            let sparks = self.spark_hp.tick(self.noise.white() * self.spark_env);
            let rumble = self.rumble.tick(self.brown.tick(w));
            *s = soft_clip(y * grind + sparks * spark_gain + rumble * rumble_gain) * level;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Tyre (rolling and sliding on loose ground)
// ---------------------------------------------------------------------------------------------

/// Surfaces along the `surface` input, at 0, 0.25, 0.5, 0.75 and 1.0.
pub const TYRE_SURFACES: [&str; 5] = ["packed dirt", "gravel", "sand", "mud", "rock / tarmac"];

model_params! {
    /// Tyres on loose ground: road rumble, the tyre's own contact roar and knobbly-tread hum
    /// under everything, then per surface gravel crunch and stones knocking the underside,
    /// sand's soft rush, mud squelch, slap and sucking, and on rock or tarmac a squeal when the
    /// tyre slides.
    ///
    /// Tuned against recordings (`tools/reference/tyres`): a real tyre is a mid-range sound.
    /// On gravel it peaks between 500 Hz and 1 kHz and has only 3 to 5 % of its energy above
    /// 2.5 kHz, so nothing here is a bright hiss.
    TyreParams / TyreParamId {
        rumble_level: "road/rumble" = 0.7, UNIT;
        body_level: "road/roar" = 0.7, UNIT;
        hum_level: "tread/hum" = 0.4, UNIT;
        tread_hz: "tread/hz" = 380.0, exp(60.0, 1500.0);
        crunch_level: "gravel/crunch" = 0.7, UNIT;
        crunch_hz: "gravel/hz" = 800.0, exp(300.0, 6000.0);
        stones_level: "gravel/stones" = 0.5, UNIT;
        hiss_level: "sand/hiss" = 0.6, UNIT;
        squelch_level: "mud/squelch" = 0.8, UNIT;
        squeal_level: "rock/squeal" = 0.6, UNIT;
        squeal_hz: "rock/squeal_hz" = 1050.0, exp(400.0, 3000.0);
        slide_level: "slide/level" = 0.7, UNIT;
        snow_crunch: "snow/crunch" = 0.7, UNIT;
        snow_squeak: "snow/squeak" = 0.5, UNIT;
        powder_hush: "snow/powder_hush" = 0.7, UNIT;
        powder_whump: "snow/powder_whump" = 0.6, UNIT;
        ice_hum: "ice/hum" = 0.6, UNIT;
        ice_scrape: "ice/scrape" = 0.6, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// Per-surface weight of each layer: [dirt, gravel, sand, mud, rock].
const T_RUMBLE: [f32; 5] = [1.0, 0.35, 0.4, 0.6, 0.8];
const T_HUM: [f32; 5] = [0.7, 0.45, 0.3, 0.2, 1.0];
const T_CRUNCH: [f32; 5] = [0.12, 1.0, 0.05, 0.0, 0.08];
const T_STONES: [f32; 5] = [0.08, 1.0, 0.0, 0.0, 0.25];
const T_LOOSE: [f32; 5] = [0.8, 1.0, 0.9, 0.3, 0.15];
/// The tyre's own contact roar: level and centre frequency on each surface.
const T_BODY: [f32; 5] = [0.8, 0.4, 0.4, 0.5, 1.0];
const T_BODY_HZ: [f32; 5] = [420.0, 650.0, 380.0, 600.0, 850.0];

// Layer levels, set by measuring renders against the recordings (see the tests).
const RUMBLE_GAIN: f32 = 3.0;
const BODY_GAIN: f32 = 3.0;
const CRUNCH_GAIN: f32 = 10.0;
const STONES_GAIN: f32 = 1.2;
const SAND_GAIN: f32 = 3.0;
const SPRAY_GAIN: f32 = 1.5;
const SLAP_GAIN: f32 = 12.0;
const SLIDE_GAIN: f32 = 4.0;

/// Snow and ice, along the `snow` input: [bare ground, packed snow, powder, ice]. Tuned against
/// recordings of tyres on snow and ice and of steps in fresh snow (`tools/reference/snow`): all
/// broad up to about 2 kHz and low-heavy, with about 5 % above 2.5 kHz; a studded tyre on ice
/// is a steady band at 250 Hz and 1 kHz.
const SNOW_POINTS: usize = 4;

/// The snow layers, in their own struct and with their own random streams, so that a tyre on
/// bare ground sounds exactly as it did before snow existed.
struct SnowLayers {
    noise: Noise,
    rng: crate::math::Rng,
    body: Svf,
    grain_dust: Dust,
    grain_env: f32,
    grain_soft: f32,
    grain: Svf,
    grain_lp: Svf,
    squeak_dust: Dust,
    /// Phase, increment and level of the current squeak.
    squeak: (f32, f32, f32),
    hush: Svf,
    whump_dust: Dust,
    whump: (f32, f32, f32),
    ice_band: Svf,
    ice_hum: Svf,
    studs: Svf,
    scrape: Svf,
    scrape_lp: Svf,
    chatter: SlowNoise,
}

impl SnowLayers {
    fn new() -> Self {
        SnowLayers {
            noise: Noise::new(crate::blocks::mix_seed(0x45_0101)),
            rng: crate::math::Rng::new(crate::blocks::mix_seed(0x45_0102)),
            body: Svf::default(),
            grain_dust: Dust::new(0x45_0103),
            grain_env: 0.0,
            grain_soft: 0.0,
            grain: Svf::default(),
            grain_lp: Svf::default(),
            squeak_dust: Dust::new(0x45_0104),
            squeak: (0.0, 0.0, 0.0),
            hush: Svf::default(),
            whump_dust: Dust::new(0x45_0105),
            whump: (0.0, 0.0, 0.0),
            ice_band: Svf::default(),
            ice_hum: Svf::default(),
            studs: Svf::default(),
            scrape: Svf::default(),
            scrape_lp: Svf::default(),
            chatter: SlowNoise::new(0x45_0106),
        }
    }

    /// Add the snow and ice sound for one block into `out`, weighted by `w` (packed, powder, ice).
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, sr: f32, dt: f32, speed: f32, slip: f32, load: f32, w: [f32; 3], p: &TyreParams, out: &mut [f32]) {
        let [packed, powder, ice] = w;
        let roll = speed.powf(0.8);
        let heavy = 0.6 + 0.6 * load;
        let moving = (speed + slip).min(1.0);
        let level = p.gain * 0.8;
        // Packed snow: the roar of the tread compacting it, a dense crunch of grains, and now
        // and then a squeak (cold snow squeaks; `snow/squeak` is how cold).
        self.body.set(FilterMode::BandPass, 330.0 * (0.8 + 0.4 * speed), 0.3, sr);
        let body_gain = (packed * 0.9 + powder * 0.35) * roll * heavy * 2.4;
        let grain_p = (200.0 + 900.0 * speed + 600.0 * slip) * moving / sr;
        self.grain_lp.set(FilterMode::LowPass, 3800.0, 0.1, sr);
        let grain_gain = p.snow_crunch * (packed + 0.15 * powder) * (0.4 + 0.6 * moving) * 7.0;
        let (grain_decay, grain_attack) = ((-1.0 / (0.007 * sr)).exp(), 1.0 - (-1.0 / (0.0015 * sr)).exp());
        let squeak_p = (0.5 + 6.0 * speed) * p.snow_squeak * packed * moving / sr;
        let squeak_decay = (-1.0 / (0.06 * sr)).exp();
        // Powder: a soft deep hush, and the whump of the car ploughing through drifts.
        self.hush.set(FilterMode::LowPass, 450.0 + 350.0 * speed, 0.15, sr);
        let hush_gain = p.powder_hush * powder * (0.2 + 0.8 * roll) * heavy * 3.0;
        let whump_p = (0.4 + 2.5 * speed) * p.powder_whump * powder * moving / sr;
        let whump_decay = (-1.0 / (0.09 * sr)).exp();
        // Ice: a smooth glassy band and the hum of the tread; sliding, the studs scrape. All
        // of it kept under 3 kHz.
        self.ice_band.set(FilterMode::BandPass, 900.0 + 300.0 * speed, 0.35, sr);
        self.ice_hum.set(FilterMode::BandPass, 260.0 + 220.0 * speed, 0.7, sr);
        let ice_gain = p.ice_hum * ice * roll * 1.6;
        let judder = 0.7 + 0.3 * self.chatter.advance(25.0, dt);
        self.scrape.set(FilterMode::BandPass, 1500.0 + 500.0 * slip, 0.4, sr);
        self.scrape_lp.set(FilterMode::LowPass, 2600.0, 0.1, sr);
        self.studs.set(FilterMode::BandPass, 1000.0, 0.5, sr);
        let scrape_gain = p.ice_scrape * ice * ((slip - 0.2) / 0.6).clamp(0.0, 1.0) * (0.3 + 0.7 * speed) * judder * 4.0;
        for o in out.iter_mut() {
            let pk = self.noise.pink();
            let mut y = self.body.tick(pk) * body_gain;
            if grain_gain > 0.0 {
                let c = self.grain_dust.tick(grain_p);
                if c > 0.0 {
                    self.grain_env = self.grain_env.max(c);
                    self.grain.set(FilterMode::BandPass, self.rng.range(500.0, 2200.0), 0.4, sr);
                }
                self.grain_soft += (self.grain_env - self.grain_soft) * grain_attack;
                y += self.grain_lp.tick(self.grain.tick(pk * self.grain_soft)) * grain_gain;
                self.grain_env *= grain_decay;
            }
            if packed > 0.0 {
                let q = self.squeak_dust.tick(squeak_p);
                if q > 0.0 && self.squeak.2 < 0.05 {
                    self.squeak = (0.0, self.rng.range(950.0, 1550.0) / sr, 0.6 + 0.4 * q);
                }
                if self.squeak.2 > 1e-4 {
                    let (ph, inc, env) = &mut self.squeak;
                    y += (*ph * TAU).sin() * *env * p.snow_squeak * packed * 1.6;
                    *ph = (*ph + *inc).fract();
                    *inc *= 0.99998;
                    *env *= squeak_decay;
                }
            }
            if hush_gain > 0.0 || powder > 0.0 {
                y += self.hush.tick(pk) * hush_gain;
                let wq = self.whump_dust.tick(whump_p);
                if wq > 0.0 {
                    self.whump = (0.0, self.rng.range(55.0, 110.0) / sr, wq);
                }
                if self.whump.2 > 1e-4 {
                    let (ph, inc, env) = &mut self.whump;
                    y += (*ph * TAU).sin() * *env * p.powder_whump * powder * 0.9;
                    *ph = (*ph + *inc).fract();
                    *env *= whump_decay;
                }
            }
            if ice > 0.0 {
                y += (self.ice_band.tick(pk) * 0.6 + self.ice_hum.tick(pk)) * ice_gain;
                if scrape_gain > 0.0 {
                    y += self.scrape_lp.tick(self.scrape.tick(pk)) * scrape_gain;
                }
                // The studs: a fine, steady texture in the band of the ice hum.
                y += self.studs.tick(pk) * ice_gain * 0.5;
            }
            *o += y * level;
        }
    }
}

pub struct Tyre {
    sr: f32,
    noise: Noise,
    brown: crate::blocks::Brown,
    rumble: Svf,
    body: Svf,
    tread: Phasor,
    tread_bp: Svf,
    wobble: SlowNoise,
    crunch_dust: Dust,
    crunch_env: f32,
    /// The envelope the grain is heard through: `crunch_env` with a soft attack.
    crunch_soft: f32,
    crunch_lp: Svf,
    crunch: [Svf; 3],
    crunch_next: usize,
    stone_dust: Dust,
    stones: [Svf; 3],
    stone_next: usize,
    stone_lp: Svf,
    sand: Svf,
    sand_flow: SlowNoise,
    spray: Svf,
    mud: Svf,
    mud_wander: SlowNoise,
    slap_dust: Dust,
    slap: Svf,
    slap_env: f32,
    slap_soft: f32,
    suck_dust: Dust,
    sucks: [(f32, f32, f32); 3],
    suck_next: usize,
    squeal: [Svf; 2],
    squeal_wander: SlowNoise,
    squeal_tone: Phasor,
    slide: Svf,
    slide_wander: SlowNoise,
    snow: SnowLayers,
}

impl Generator for Tyre {
    type P = TyreParams;
    const NAME: &'static str = "tyre";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Tyres rolling and sliding: dirt, gravel, sand, mud, rock. One per vehicle; sum its wheels into the inputs.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.0, doc: "Rolling speed 0..1" },
        InputSpec { name: "slip", default: 0.0, doc: "Sliding sideways or spinning 0..1" },
        InputSpec { name: "load", default: 0.5, doc: "Weight on the tyres: 0 a bike, 1 a war rig (louder, lower)" },
        InputSpec { name: "surface", default: 0.0, doc: "0 packed dirt, 0.25 gravel, 0.5 sand, 0.75 mud, 1 rock/tarmac; in-between values blend neighbours" },
        InputSpec { name: "snow", default: 0.0, doc: "0 bare ground (as `surface` says), 0.33 packed snow, 0.67 powder, 1 ice; in-between values blend neighbours" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.06;

    fn presets() -> Vec<(&'static str, TyreParams)> {
        vec![
            ("Knobbly", TyreParams { hum_level: 0.65, tread_hz: 300.0, crunch_level: 0.85, stones_level: 0.6, ..Default::default() }),
            ("Road tyre", TyreParams { hum_level: 0.25, tread_hz: 650.0, crunch_level: 0.5, squeal_level: 0.85, rumble_level: 0.5, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Tyre {
            sr,
            noise: Noise::new(0x45_0001),
            brown: crate::blocks::Brown::default(),
            rumble: Svf::default(),
            body: Svf::default(),
            tread: Phasor::default(),
            tread_bp: Svf::default(),
            wobble: SlowNoise::new(0x45_0002),
            crunch_dust: Dust::new(0x45_0003),
            crunch_env: 0.0,
            crunch_soft: 0.0,
            crunch_lp: Svf::default(),
            crunch: [Svf::default(); 3],
            crunch_next: 0,
            stone_dust: Dust::new(0x45_0004),
            stones: [Svf::default(); 3],
            stone_next: 0,
            stone_lp: Svf::default(),
            sand: Svf::default(),
            sand_flow: SlowNoise::new(0x45_0009),
            spray: Svf::default(),
            mud: Svf::default(),
            mud_wander: SlowNoise::new(0x45_0005),
            slap_dust: Dust::new(0x45_000A),
            slap: Svf::default(),
            slap_env: 0.0,
            slap_soft: 0.0,
            suck_dust: Dust::new(0x45_0006),
            sucks: [(0.0, 0.0, 0.0); 3],
            suck_next: 0,
            squeal: [Svf::default(); 2],
            squeal_wander: SlowNoise::new(0x45_0007),
            squeal_tone: Phasor::default(),
            slide: Svf::default(),
            slide_wander: SlowNoise::new(0x45_0008),
            snow: SnowLayers::new(),
        }
    }

    fn block(&mut self, x: &[f32], p: &TyreParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (speed, slip, load) = (x[0], x[1], x[2]);
        // Blend the two surfaces either side of `surface`.
        let pos = x[3].clamp(0.0, 1.0) * 4.0;
        let i0 = (pos.floor() as usize).min(3);
        let f = pos - i0 as f32;
        let mut w = [0.0f32; 5];
        w[i0] = 1.0 - f;
        w[i0 + 1] += f;
        let mix = |table: &[f32; 5]| table.iter().zip(&w).map(|(a, b)| a * b).sum::<f32>();
        let (sand_w, mud_w, rock_w) = (w[2], w[3], w[4]);
        let roll = speed.powf(0.8);
        let heavy = 0.5 + 0.7 * load;

        self.rumble.set(FilterMode::LowPass, 40.0 + 170.0 * speed * (1.0 - 0.35 * load), 0.25, sr);
        let rumble_gain = p.rumble_level * roll * heavy * mix(&T_RUMBLE) * RUMBLE_GAIN;
        // The roar of the tread meeting the ground: the middle of the sound on every surface.
        self.body.set(FilterMode::BandPass, mix(&T_BODY_HZ) * (0.75 + 0.5 * speed) * (1.1 - 0.25 * load), 0.3, sr);
        let body_gain = p.body_level * roll * (0.7 + 0.3 * heavy) * mix(&T_BODY) * BODY_GAIN;
        let wob = self.wobble.advance(3.0, dt);
        let tread_inc = p.tread_hz * speed * (1.0 + 0.04 * wob) / sr;
        self.tread_bp.set(FilterMode::BandPass, (p.tread_hz * speed * 2.0).max(40.0), 0.6, sr);
        let hum_gain = p.hum_level * speed.powf(1.5) * mix(&T_HUM) * 1.2;

        // The grain is dense on every surface; the surface sets how loud it is. Sparse bursts would
        // read as crackle.
        let crunch_p = (40.0 + 400.0 * speed + 500.0 * slip) * (speed + slip).min(1.0) / sr;
        let stone_p = (1.0 + 10.0 * speed + 20.0 * slip) * mix(&T_STONES) * (speed + slip).min(1.0) / sr;
        let crunch_decay = (-1.0 / (0.006 * sr)).exp();
        let crunch_gain = p.crunch_level * CRUNCH_GAIN * (0.7 + 0.5 * load) * mix(&T_CRUNCH);
        let attack = 1.0 - (-1.0 / (0.0015 * sr)).exp();
        self.crunch_lp.set(FilterMode::LowPass, (p.crunch_hz * 7.0).min(sr * 0.4), 0.1, sr);
        let stones_gain = p.stones_level * STONES_GAIN;
        // An impulse into a resonator still clicks; this takes the click off the knock.
        self.stone_lp.set(FilterMode::LowPass, 2200.0, 0.1, sr);

        // Sand gives way under the tyre: a soft rush in the low middle that ebbs and flows,
        // and a duller spray when the tyre slides. No bright hiss: recordings have none.
        let flow = 1.0 + 0.35 * self.sand_flow.advance(5.0, dt);
        self.sand.set(FilterMode::BandPass, 300.0 + 250.0 * speed, 0.2, sr);
        self.spray.set(FilterMode::BandPass, 1100.0 + 400.0 * slip, 0.2, sr);
        let hiss_gain = p.hiss_level * sand_w * (0.15 + 0.85 * speed) * (0.5 + slip) * flow * SAND_GAIN;
        let spray_gain = p.hiss_level * sand_w * slip * SPRAY_GAIN;

        self.mud.set(FilterMode::LowPass, 160.0 + 340.0 * (0.5 + 0.5 * self.mud_wander.advance(6.0, dt)), 0.8, sr);
        let squelch_gain = p.squelch_level * mud_w * (speed * 3.0).min(1.0) * (0.4 + 0.6 * slip.max(speed)) * 0.3;
        let suck_p = (1.5 + 14.0 * speed + 25.0 * slip) * mud_w / sr;
        // Mud thrown against the arch and slapping back down: short wet bursts in the middle.
        let slap_p = (4.0 + 18.0 * speed + 30.0 * slip) * mud_w * (speed + slip).min(1.0) / sr;
        let slap_decay = (-1.0 / (0.035 * sr)).exp();
        let slap_gain = p.squelch_level * mud_w * SLAP_GAIN;
        let suck_decay = (-1.0 / (0.06 * sr)).exp();
        let suck_fall = 0.5f32.powf(1.0 / (0.05 * sr));

        // In recordings the squeal holds its pitch and drifts slowly; it does not warble.
        let sq = self.squeal_wander.advance(2.0 + 3.0 * slip, dt);
        let squeal_hz = p.squeal_hz * (0.08 * sq).exp2() * (0.85 + 0.3 * speed);
        self.squeal[0].set(FilterMode::BandPass, squeal_hz, 0.985, sr);
        self.squeal[1].set(FilterMode::BandPass, squeal_hz * 2.0, 0.98, sr);
        let t = ((slip - 0.25) / 0.45).clamp(0.0, 1.0);
        let squeal_gain = p.squeal_level * rock_w * t * t * (3.0 - 2.0 * t) * (0.3 + 0.7 * speed);
        let squeal_inc = squeal_hz / sr;

        self.slide.set(FilterMode::BandPass, (450.0 + 450.0 * slip) * (0.3 * self.slide_wander.advance(9.0, dt)).exp2(), 0.35, sr);
        let slide_gain = p.slide_level * slip * (0.65 + 0.35 * wob.abs()) * mix(&T_LOOSE) * SLIDE_GAIN;

        // Snow and ice along the `snow` input: [bare ground, packed snow, powder, ice].
        let spos = x[4].clamp(0.0, 1.0) * (SNOW_POINTS - 1) as f32;
        let k0 = (spos.floor() as usize).min(SNOW_POINTS - 2);
        let sf = spos - k0 as f32;
        let mut sw = [0.0f32; SNOW_POINTS];
        sw[k0] = 1.0 - sf;
        sw[k0 + 1] += sf;
        let level = p.gain * 0.8 * sw[0];
        for s in out.iter_mut() {
            let wn = self.noise.white();
            // Pink, not white, feeds every noise layer: the highs then fall away as they do in
            // the recordings.
            let pk = self.noise.pink();
            let mut y = self.rumble.tick(self.brown.tick(wn)) * rumble_gain + self.body.tick(pk) * body_gain;
            if hum_gain > 0.0 {
                self.tread.tick(tread_inc);
                y += self.tread_bp.tick((-self.tread.phase * 10.0).exp() - 0.1) * hum_gain;
            }
            let c = self.crunch_dust.tick(crunch_p);
            if c > 0.0 {
                self.crunch_env = self.crunch_env.max(c);
                self.crunch_next = (self.crunch_next + 1) % 3;
                let hz = p.crunch_hz * (0.7 * self.crunch_dust.rng().next_bipolar()).exp2() * (0.85 + 0.3 * (1.0 - load));
                self.crunch[self.crunch_next].set(FilterMode::BandPass, hz.min(sr * 0.4), 0.45, sr);
            }
            if crunch_gain > 0.0 && (self.crunch_env > 1e-4 || self.crunch_soft > 1e-4) {
                self.crunch_soft += (self.crunch_env - self.crunch_soft) * attack;
                let burst = pk * self.crunch_soft;
                let mut g = 0.0;
                for (k, f) in self.crunch.iter_mut().enumerate() {
                    g += f.tick(if k == self.crunch_next { burst } else { 0.0 });
                }
                y += self.crunch_lp.tick(g) * crunch_gain;
                self.crunch_env *= crunch_decay;
            }
            let st = self.stone_dust.tick(stone_p);
            if st > 0.0 {
                self.stone_next = (self.stone_next + 1) % 3;
                // A stone against the underside is a dull knock, not a ping.
                let hz = 1400.0 * (0.6 * self.stone_dust.rng().next_bipolar()).exp2();
                self.stones[self.stone_next].set(FilterMode::BandPass, hz.min(sr * 0.4), 0.8, sr);
            }
            if stones_gain > 0.0 {
                let mut g = 0.0;
                for (k, f) in self.stones.iter_mut().enumerate() {
                    g += f.tick(if k == self.stone_next { st } else { 0.0 });
                }
                y += self.stone_lp.tick(g) * stones_gain;
            }
            if hiss_gain > 0.0 || spray_gain > 0.0 {
                y += self.sand.tick(pk) * hiss_gain + self.spray.tick(pk) * spray_gain;
            }
            if mud_w > 0.0 {
                y += self.mud.tick(self.brown.tick(wn) * 2.0) * squelch_gain;
                let sl = self.slap_dust.tick(slap_p);
                if sl > 0.0 {
                    self.slap_env = self.slap_env.max(sl);
                    self.slap.set(FilterMode::BandPass, self.slap_dust.rng().range(600.0, 1800.0), 0.4, sr);
                }
                if self.slap_env > 1e-4 || self.slap_soft > 1e-4 {
                    self.slap_soft += (self.slap_env - self.slap_soft) * attack * 0.3;
                    y += self.slap.tick(pk * self.slap_soft) * slap_gain;
                    self.slap_env *= slap_decay;
                }
                let sk = self.suck_dust.tick(suck_p);
                if sk > 0.0 {
                    self.suck_next = (self.suck_next + 1) % 3;
                    let hz = self.suck_dust.rng().range(170.0, 300.0);
                    self.sucks[self.suck_next] = (0.0, hz / sr, sk);
                }
                for (ph, inc, env) in self.sucks.iter_mut() {
                    if *env > 1e-4 {
                        // A falling sine chirp: the suck of a tyre leaving mud.
                        y += (*ph * TAU).sin() * *env * p.squelch_level * mud_w * 0.5;
                        *ph = (*ph + *inc).fract();
                        *inc = (*inc * suck_fall).max(45.0 / sr);
                        *env *= suck_decay;
                    }
                }
            }
            if squeal_gain > 0.0 {
                self.squeal_tone.tick(squeal_inc);
                // A squeal is a tone with a strong octave above it.
                let tone = self.squeal_tone.sin() + 0.35 * (self.squeal_tone.phase * 2.0 * TAU).sin();
                y += (self.squeal[0].tick(wn) + 0.6 * self.squeal[1].tick(wn)) * squeal_gain * 0.3 + tone * squeal_gain * 0.6;
            }
            if slide_gain > 0.0 {
                y += self.slide.tick(pk) * slide_gain;
            }
            *s = y * level;
        }
        if sw[0] < 1.0 {
            self.snow.add(sr, dt, speed, slip, load, [sw[1], sw[2], sw[3]], p, out);
        }
    }
}
