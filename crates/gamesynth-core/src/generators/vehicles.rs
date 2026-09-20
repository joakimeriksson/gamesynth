//! Vehicle and machine generators: jet, anti-grav hover, combustion engine, electric motor,
//! rotor.

use crate::blocks::{hz_coef, settle_coef, DelayLine, Dust, OnePole, Phasor, SlowNoise};
use crate::filter::{FilterMode, Svf};
use crate::jet::{JetEngine, JetParams, JetPreset};
use crate::math::{soft_clip, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::osc::{Oscillator, Waveform};
use crate::params::{exp, int, lin, GAIN, UNIT};

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

model_params! {
    /// Piston engine: per-cylinder firing pulses with combustion noise through exhaust resonances.
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
    }
}

pub struct Combustion {
    sr: f32,
    rev: f32,
    fire: Phasor,
    cyl: usize,
    amp: f32,
    cyl_gain: [f32; 12],
    noise: Noise,
    dust: Dust,
    dc: OnePole,
    exhaust: [Svf; 2],
    body: OnePole,
    intake: Svf,
}

impl Combustion {
    /// Current engine speed in RPM.
    pub fn rpm(&self, p: &CombustionParams) -> f32 {
        p.idle_rpm + (p.max_rpm - p.idle_rpm) * self.rev
    }
}

impl Generator for Combustion {
    type P = CombustionParams;
    const NAME: &'static str = "combustion";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Piston engine: cylinders, rev inertia, exhaust resonance, overrun burble.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "throttle", default: 0.0, doc: "Revs follow with rev_up / rev_down inertia" },
        InputSpec { name: "load", default: 0.3, doc: "Engine load: louder, fuller, more intake" },
    ];

    fn presets() -> Vec<(&'static str, CombustionParams)> {
        vec![
            ("V8 muscle", CombustionParams { cylinders: 8.0, idle_rpm: 700.0, max_rpm: 6200.0, exhaust_hz: 95.0, exhaust_res: 0.8, roughness: 0.45, drive: 0.6, ..Default::default() }),
            ("Motorbike", CombustionParams { cylinders: 2.0, idle_rpm: 1300.0, max_rpm: 11000.0, rev_up: 0.4, rev_down: 0.9, exhaust_hz: 210.0, pulse_sharp: 12.0, ..Default::default() }),
            ("Diesel truck", CombustionParams { cylinders: 6.0, idle_rpm: 600.0, max_rpm: 2800.0, rev_up: 2.2, rev_down: 2.8, exhaust_hz: 70.0, noise_level: 0.7, roughness: 0.5, burble: 0.1, ..Default::default() }),
            ("Lawnmower", CombustionParams { cylinders: 1.0, idle_rpm: 1600.0, max_rpm: 3600.0, exhaust_hz: 260.0, exhaust_res: 0.5, roughness: 0.6, noise_level: 0.6, gain: 1.4, ..Default::default() }),
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
            cyl_gain,
            noise: Noise::new(0x41_0002),
            dust: Dust::new(0x41_0003),
            dc: OnePole::default(),
            exhaust: [Svf::default(); 2],
            body: OnePole::default(),
            intake: Svf::default(),
        }
    }

    fn snap(&mut self, x: &[f32], _p: &CombustionParams) {
        self.rev = x[0];
    }

    fn block(&mut self, x: &[f32], p: &CombustionParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (throttle, load) = (x[0], x[1]);
        let rev = slew(&mut self.rev, throttle, p.rev_up, p.rev_down, dt);
        let n_cyl = (p.cylinders.round() as usize).clamp(1, 12);
        let fire_hz = self.rpm(p) / 60.0 * n_cyl as f32 * 0.5;
        let inc = fire_hz / sr;
        self.exhaust[0].set(FilterMode::BandPass, p.exhaust_hz * (0.7 + 0.9 * rev), p.exhaust_res, sr);
        // The second resonance follows the firing fundamental so the tone stays full at high revs.
        self.exhaust[1].set(FilterMode::BandPass, fire_hz.max(p.exhaust_hz * 1.5), p.exhaust_res * 0.8, sr);
        self.intake.set(FilterMode::BandPass, 300.0 + 1500.0 * rev, 0.4, sr);
        let (dc_coef, body_coef) = (hz_coef(20.0, sr), hz_coef(1200.0, sr));
        // Lifting off at high revs: unburnt fuel pops in the exhaust.
        let overrun = (rev - throttle - 0.15).max(0.0) * p.burble;
        let pop_p = 40.0 * overrun / sr;
        let drive = 1.0 + p.drive * 5.0;
        let intake_gain = p.intake_level * (0.2 + 0.8 * throttle) * 1.5;
        let level = (0.35 + 0.65 * load.max(throttle * 0.5)) * (0.7 + 0.6 * rev) * p.gain * 1.1;
        for s in out.iter_mut() {
            if self.fire.tick(inc) {
                self.cyl = (self.cyl + 1) % n_cyl;
                self.amp = self.cyl_gain[self.cyl] * (1.0 + p.roughness * 0.5 * self.dust.rng().next_bipolar());
            }
            let pulse = self.amp * (-self.fire.phase * p.pulse_sharp).exp();
            let w = self.noise.white();
            let bang = pulse * (0.6 + p.noise_level * 1.5 * w) + self.dust.tick(pop_p) * 1.5;
            let x0 = self.dc.hp(bang, dc_coef);
            let pipe = self.exhaust[0].tick(x0) + 0.8 * self.exhaust[1].tick(x0) + 0.35 * self.body.lp(x0, body_coef);
            let intake = self.intake.tick(self.noise.pink()) * intake_gain * (0.5 + 0.5 * pulse);
            *s = soft_clip((pipe + intake) * drive) / (1.0 + 0.2 * (drive - 1.0)) * level;
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
    /// Rotor: blade-pass thump, chopped downwash noise and a turbine whine.
    RotorParams / RotorParamId {
        blades: "rotor/blades" = 3.0, int(2, 8);
        max_bpf: "rotor/max_blade_hz" = 22.0, exp(4.0, 160.0);
        spin_up: "rotor/spin_up" = 3.0, lin(0.1, 12.0);
        spin_down: "rotor/spin_down" = 5.0, lin(0.1, 12.0);
        sharp: "rotor/sharpness" = 3.0, lin(1.0, 8.0);
        thump_level: "rotor/thump" = 0.7, UNIT;
        chop: "wash/chop" = 0.75, UNIT;
        wash_level: "wash/level" = 0.6, UNIT;
        wash_hz: "wash/hz" = 500.0, exp(100.0, 4000.0);
        turbine_level: "turbine/level" = 0.15, UNIT;
        turbine_hz: "turbine/hz" = 3200.0, exp(500.0, 9000.0);
        gain: "master/gain" = 0.9, GAIN;
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
}

impl Generator for Rotor {
    type P = RotorParams;
    const NAME: &'static str = "rotor";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Helicopter, propeller or fan: blade thump, chopped wash, turbine whine.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "rpm", default: 0.0, doc: "Rotor speed target; follows with spin inertia" },
        InputSpec { name: "load", default: 0.5, doc: "Blade pitch / lift: heavier thump" },
    ];

    fn presets() -> Vec<(&'static str, RotorParams)> {
        vec![
            ("Attack chopper", RotorParams { blades: 4.0, max_bpf: 28.0, sharp: 4.5, thump_level: 0.9, turbine_level: 0.25, ..Default::default() }),
            ("Propeller plane", RotorParams { blades: 3.0, max_bpf: 95.0, spin_up: 1.5, spin_down: 2.5, sharp: 1.6, wash_hz: 900.0, turbine_level: 0.0, ..Default::default() }),
            ("Ceiling fan", RotorParams { blades: 4.0, max_bpf: 9.0, spin_up: 6.0, spin_down: 9.0, thump_level: 0.25, wash_level: 0.35, wash_hz: 300.0, turbine_level: 0.0, ..Default::default() }),
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
        let thump_gain = p.thump_level * (0.5 + 0.5 * load) * 2.0;
        let wash_gain = p.wash_level * 2.5;
        let turbine_gain = p.turbine_level * spin * 0.5;
        let (inc, t_inc) = (bpf / sr, p.turbine_hz * spin / sr);
        let level = fade_in(spin) * (0.3 + 0.7 * spin) * p.gain * 0.65;
        for s in out.iter_mut() {
            if self.pass.tick(inc) {
                self.blade = (self.blade + 1) % n_blades;
                // One blade tracks slightly off, giving the once-per-revolution lope.
                self.blade_amp = if self.blade == 0 { 0.82 } else { 1.0 };
            }
            self.turbine[0].tick(t_inc);
            self.turbine[1].tick(t_inc * 1.31);
            let env = (0.5 + 0.5 * (self.pass.phase * TAU).cos()).powf(p.sharp) * self.blade_amp;
            let thump = self.thump.tick(env - 0.3) + 0.3 * self.pass.sin();
            let wash = self.wash.tick(self.noise.pink()) * (1.0 - p.chop + p.chop * env);
            let turbine = self.turbine[0].sin() + 0.5 * self.turbine[1].sin();
            *s = (thump * thump_gain + wash * wash_gain + turbine * turbine_gain) * level;
        }
    }
}
