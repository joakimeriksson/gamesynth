//! Procedural jet / turbine engine for vehicles.
//!
//! A continuous sound driven by game state each frame: `throttle`, `boost` (afterburner),
//! `speed` (airspeed, for wind) and `damage`. An internal RPM model with separate
//! spool-up / spool-down times gives the engine inertia so it never snaps.
//!
//! Layers (all procedural, no samples):
//! * turbine whine — detuned sine partials + sub-harmonic shaft tone, pitch tracks RPM
//! * core roar — pink noise through RPM-tracking resonant filters, turbulence AM, drive
//! * tube resonance — short RPM-modulated feedback comb (the "jet" timbre)
//! * intake hiss — high-passed white noise, level ∝ RPM²
//! * afterburner — driven noise with a low thrum, gated by `boost`
//! * wind — band-passed noise, level ∝ speed²
//! * damage — random sputter drop-outs

use crate::filter::{FilterMode, Svf};
use crate::math::{limit, soft_clip, Rng, TAU};
use crate::noise::Noise;
use crate::params::ParamKind;

const BLOCK: usize = 32;
/// Longest tube delay supported (sizes the comb buffer at construction).
const MAX_TUBE_MS: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct JetParams {
    /// Seconds for RPM to reach ~95% of a higher throttle.
    pub spool_up: f32,
    /// Seconds for RPM to fall to ~95% of the way to a lower throttle.
    pub spool_down: f32,
    /// RPM fraction at zero throttle.
    pub idle_rpm: f32,
    /// RPM multiplier at full boost (afterburner over-speed).
    pub boost_rpm: f32,

    /// Whine fundamental at RPM 1.0.
    pub whine_hz: f32,
    pub whine_level: f32,
    /// Detune between the two main partials, in cents (creates beating).
    pub whine_detune: f32,
    /// Level of the sub-harmonic shaft tone (one octave below).
    pub whine_shaft: f32,

    pub roar_level: f32,
    /// Roar low-pass cutoff at RPM 1.0 (tracks RPM over ±2 octaves).
    pub roar_hz: f32,
    pub roar_resonance: f32,
    /// Rate of the turbulence amplitude modulation in Hz.
    pub rumble_hz: f32,
    /// Depth of the turbulence modulation 0..1.
    pub rumble: f32,
    pub roar_drive: f32,

    /// Comb delay in milliseconds at idle (shortens as RPM rises).
    pub tube_ms: f32,
    pub tube_feedback: f32,
    pub tube_mix: f32,

    pub hiss_level: f32,
    pub hiss_hz: f32,

    pub burner_level: f32,
    pub burner_drive: f32,
    /// Afterburner low-frequency thrum rate in Hz.
    pub burner_thrum_hz: f32,

    pub wind_level: f32,
    /// Wind band-pass centre at speed 1.0.
    pub wind_hz: f32,

    /// Output gain (linear).
    pub gain: f32,
}

impl Default for JetParams {
    fn default() -> Self {
        JetPreset::Racer.params()
    }
}

param_table! {
    /// Identifier of every tweakable value in [`JetParams`].
    JetParamId for JetParams {
        SpoolUp => "spool/up", ParamKind::Float { min: 0.05, max: 10.0, step: 0.01 }, (spool_up);
        SpoolDown => "spool/down", ParamKind::Float { min: 0.05, max: 10.0, step: 0.01 }, (spool_down);
        IdleRpm => "spool/idle_rpm", ParamKind::Float { min: 0.05, max: 0.9, step: 0.01 }, (idle_rpm);
        BoostRpm => "spool/boost_rpm", ParamKind::Float { min: 1.0, max: 2.0, step: 0.01 }, (boost_rpm);

        WhineHz => "whine/hz", ParamKind::Exp { min: 100.0, max: 8000.0 }, (whine_hz);
        WhineLevel => "whine/level", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (whine_level);
        WhineDetune => "whine/detune_cents", ParamKind::Float { min: 0.0, max: 50.0, step: 0.1 }, (whine_detune);
        WhineShaft => "whine/shaft", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (whine_shaft);

        RoarLevel => "roar/level", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (roar_level);
        RoarHz => "roar/hz", ParamKind::Exp { min: 60.0, max: 6000.0 }, (roar_hz);
        RoarResonance => "roar/resonance", ParamKind::Float { min: 0.0, max: 0.95, step: 0.001 }, (roar_resonance);
        RumbleHz => "roar/rumble_hz", ParamKind::Float { min: 0.5, max: 60.0, step: 0.1 }, (rumble_hz);
        Rumble => "roar/rumble", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (rumble);
        RoarDrive => "roar/drive", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (roar_drive);

        TubeMs => "tube/ms", ParamKind::Float { min: 0.5, max: MAX_TUBE_MS, step: 0.01 }, (tube_ms);
        TubeFeedback => "tube/feedback", ParamKind::Float { min: 0.0, max: 0.9, step: 0.001 }, (tube_feedback);
        TubeMix => "tube/mix", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (tube_mix);

        HissLevel => "hiss/level", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (hiss_level);
        HissHz => "hiss/hz", ParamKind::Exp { min: 500.0, max: 16000.0 }, (hiss_hz);

        BurnerLevel => "burner/level", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (burner_level);
        BurnerDrive => "burner/drive", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (burner_drive);
        BurnerThrumHz => "burner/thrum_hz", ParamKind::Float { min: 5.0, max: 120.0, step: 0.1 }, (burner_thrum_hz);

        WindLevel => "wind/level", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (wind_level);
        WindHz => "wind/hz", ParamKind::Exp { min: 200.0, max: 8000.0 }, (wind_hz);

        Gain => "master/gain", ParamKind::Float { min: 0.0, max: 2.0, step: 0.001 }, (gain);
    }
}

/// Ready-made engine characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum JetPreset {
    /// Light, responsive racing turbine.
    Racer = 0,
    /// Big slow-spooling engine with lots of low rumble.
    Heavy = 1,
    /// Old-school whiny turbojet.
    Turbine = 2,
    /// Exotic, hissy, almost tone-less ramjet.
    Scramjet = 3,
}

impl JetPreset {
    pub const ALL: [JetPreset; 4] = [JetPreset::Racer, JetPreset::Heavy, JetPreset::Turbine, JetPreset::Scramjet];
    pub const NAMES: [&'static str; 4] = ["Racer", "Heavy", "Turbine", "Scramjet"];

    pub fn from_index(i: usize) -> JetPreset {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn from_name(name: &str) -> Option<JetPreset> {
        Self::ALL.iter().copied().find(|p| p.name().eq_ignore_ascii_case(name))
    }

    pub fn params(self) -> JetParams {
        let racer = JetParams {
            spool_up: 1.2,
            spool_down: 2.0,
            idle_rpm: 0.28,
            boost_rpm: 1.25,
            whine_hz: 2600.0,
            whine_level: 0.2,
            whine_detune: 7.0,
            whine_shaft: 0.35,
            roar_level: 0.8,
            roar_hz: 900.0,
            roar_resonance: 0.45,
            rumble_hz: 14.0,
            rumble: 0.5,
            roar_drive: 0.35,
            tube_ms: 3.6,
            tube_feedback: 0.55,
            tube_mix: 0.5,
            hiss_level: 0.25,
            hiss_hz: 5000.0,
            burner_level: 0.8,
            burner_drive: 0.7,
            burner_thrum_hz: 38.0,
            wind_level: 0.4,
            wind_hz: 1500.0,
            gain: 0.7,
        };
        match self {
            JetPreset::Racer => racer,
            JetPreset::Heavy => JetParams {
                spool_up: 2.6,
                spool_down: 3.5,
                idle_rpm: 0.22,
                boost_rpm: 1.15,
                whine_hz: 1400.0,
                whine_level: 0.14,
                whine_detune: 4.0,
                whine_shaft: 0.5,
                roar_level: 0.9,
                roar_hz: 480.0,
                roar_resonance: 0.6,
                rumble_hz: 9.0,
                rumble: 0.7,
                roar_drive: 0.5,
                tube_ms: 7.5,
                tube_feedback: 0.65,
                tube_mix: 0.55,
                hiss_level: 0.15,
                hiss_hz: 4000.0,
                burner_thrum_hz: 24.0,
                ..racer
            },
            JetPreset::Turbine => JetParams {
                spool_up: 1.6,
                spool_down: 2.4,
                idle_rpm: 0.3,
                whine_hz: 3400.0,
                whine_level: 0.34,
                whine_detune: 9.0,
                whine_shaft: 0.3,
                roar_level: 0.65,
                roar_hz: 1400.0,
                roar_resonance: 0.35,
                rumble: 0.35,
                roar_drive: 0.2,
                tube_ms: 2.8,
                tube_feedback: 0.45,
                hiss_level: 0.35,
                hiss_hz: 6500.0,
                ..racer
            },
            JetPreset::Scramjet => JetParams {
                spool_up: 0.7,
                spool_down: 1.2,
                idle_rpm: 0.35,
                boost_rpm: 1.4,
                whine_hz: 5200.0,
                whine_level: 0.06,
                whine_shaft: 0.0,
                roar_level: 0.85,
                roar_hz: 2000.0,
                roar_resonance: 0.3,
                rumble_hz: 22.0,
                rumble: 0.4,
                roar_drive: 0.6,
                tube_ms: 1.8,
                tube_feedback: 0.75,
                tube_mix: 0.6,
                hiss_level: 0.5,
                hiss_hz: 7000.0,
                burner_level: 1.0,
                burner_drive: 0.9,
                burner_thrum_hz: 55.0,
                wind_level: 0.6,
                ..racer
            },
        }
    }
}

/// Game-thread → audio-thread messages for a [`JetEngine`].
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JetCommand {
    /// 0..1
    Throttle(f32),
    /// 0..1 afterburner amount.
    Boost(f32),
    /// 0..1 normalised airspeed (wind noise).
    Speed(f32),
    /// 0..1, adds random sputtering.
    Damage(f32),
    SetParams(JetParams),
    SetParam(JetParamId, f32),
    /// Extra gain multiplier 0..2.
    MasterGain(f32),
    /// Snap RPM to the current throttle target (e.g. spawning an already-running ship).
    SnapRpm,
}

pub struct JetEngine {
    p: JetParams,
    sr: f32,
    // Control targets from the game and their smoothed versions.
    throttle: f32,
    boost: f32,
    speed: f32,
    damage: f32,
    rpm: f32,
    boost_s: f32,
    speed_s: f32,
    master_gain: f32,
    // Oscillators
    whine_phase: [f32; 4],
    jitter: f32,
    thrum_phase: f32,
    // Noise + filters
    noise: Noise,
    rng: Rng,
    roar_lp: Svf,
    roar_bp: Svf,
    hiss_hp: Svf,
    burner_hp: Svf,
    wind_bp: Svf,
    // Turbulence
    rumble: f32,
    rumble_target: f32,
    rumble_phase: f32,
    // Tube comb
    tube: Vec<f32>,
    tube_pos: usize,
    tube_delay: f32,
    // Damage sputter
    sputter_blocks: u32,
    sputter_gain: f32,
    peak: f32,
    peak_decay: f32,
}

impl JetEngine {
    /// Allocates the comb buffer; construct on the game thread.
    pub fn new(sample_rate: f32, params: JetParams) -> Self {
        let tube_len = ((MAX_TUBE_MS * 0.001 * sample_rate) as usize + 4).max(8);
        let mut e = JetEngine {
            p: params,
            sr: sample_rate,
            throttle: 0.0,
            boost: 0.0,
            speed: 0.0,
            damage: 0.0,
            rpm: params.idle_rpm,
            boost_s: 0.0,
            speed_s: 0.0,
            master_gain: 1.0,
            whine_phase: [0.0, 0.31, 0.62, 0.17],
            jitter: 0.0,
            thrum_phase: 0.0,
            noise: Noise::new(0x1E7_0001),
            rng: Rng::new(0x1E7_0002),
            roar_lp: Svf::default(),
            roar_bp: Svf::default(),
            hiss_hp: Svf::default(),
            burner_hp: Svf::default(),
            wind_bp: Svf::default(),
            rumble: 0.0,
            rumble_target: 0.0,
            rumble_phase: 0.0,
            tube: vec![0.0; tube_len],
            tube_pos: 0,
            tube_delay: 1.0,
            sputter_blocks: 0,
            sputter_gain: 1.0,
            peak: 0.0,
            peak_decay: (-9.21 / (0.25 * sample_rate)).exp(),
        };
        e.tube_delay = e.tube_delay_samples();
        e
    }

    pub fn sample_rate(&self) -> f32 {
        self.sr
    }

    pub fn params(&self) -> &JetParams {
        &self.p
    }

    pub fn set_params(&mut self, p: JetParams) {
        self.p = p;
    }

    /// Current RPM fraction (idle_rpm..boost_rpm). Useful for UI gauges and rumble.
    pub fn rpm(&self) -> f32 {
        self.rpm
    }

    pub fn throttle(&self) -> f32 {
        self.throttle
    }

    pub fn set_throttle(&mut self, v: f32) {
        self.throttle = clamp01(v);
    }

    pub fn set_boost(&mut self, v: f32) {
        self.boost = clamp01(v);
    }

    pub fn set_speed(&mut self, v: f32) {
        self.speed = clamp01(v);
    }

    pub fn set_damage(&mut self, v: f32) {
        self.damage = clamp01(v);
    }

    pub fn set_master_gain(&mut self, g: f32) {
        self.master_gain = if g.is_finite() { g.clamp(0.0, 2.0) } else { 1.0 };
    }

    /// Jump RPM straight to the throttle target (no spool).
    pub fn snap_rpm(&mut self) {
        self.rpm = self.rpm_target();
        self.boost_s = self.boost;
        self.speed_s = self.speed;
    }

    /// Recent output peak 0..1, decays over ~250 ms.
    pub fn peak(&self) -> f32 {
        self.peak
    }

    pub fn apply(&mut self, cmd: JetCommand) {
        match cmd {
            JetCommand::Throttle(v) => self.set_throttle(v),
            JetCommand::Boost(v) => self.set_boost(v),
            JetCommand::Speed(v) => self.set_speed(v),
            JetCommand::Damage(v) => self.set_damage(v),
            JetCommand::SetParams(p) => self.set_params(p),
            JetCommand::SetParam(id, v) => self.p.set(id, v),
            JetCommand::MasterGain(g) => self.set_master_gain(g),
            JetCommand::SnapRpm => self.snap_rpm(),
        }
    }

    fn rpm_target(&self) -> f32 {
        let base = self.p.idle_rpm + (1.0 - self.p.idle_rpm) * self.throttle;
        base * (1.0 + (self.p.boost_rpm - 1.0) * self.boost)
    }

    fn tube_delay_samples(&self) -> f32 {
        let ms = self.p.tube_ms / (0.6 + 0.4 * self.rpm.min(1.5));
        (ms * 0.001 * self.sr).clamp(1.0, (self.tube.len() - 3) as f32)
    }

    /// Render mono audio into `out`, overwriting it. Allocation free.
    pub fn render_mono(&mut self, out: &mut [f32]) {
        for chunk in out.chunks_mut(BLOCK) {
            self.render_block(chunk);
        }
    }

    fn render_block(&mut self, out: &mut [f32]) {
        let n = out.len();
        let dt = n as f32 / self.sr;
        let p = self.p;

        // --- control rate --------------------------------------------------------------
        let target = self.rpm_target();
        let tau = if target > self.rpm { p.spool_up } else { p.spool_down } / 3.0;
        self.rpm += (target - self.rpm) * (1.0 - (-dt / tau.max(1e-3)).exp());
        let ctl = 1.0 - (-dt / 0.08).exp();
        self.boost_s += (self.boost - self.boost_s) * ctl;
        self.speed_s += (self.speed - self.speed_s) * ctl;
        let r = self.rpm;
        let r1 = r.min(1.0);

        // Whine: slow random walk on pitch for life.
        self.jitter = (self.jitter + self.rng.next_bipolar() * 0.004).clamp(-0.01, 0.01) * 0.98;
        let f0 = (p.whine_hz * r * (1.0 + self.jitter)).clamp(20.0, self.sr * 0.4);
        let det = (p.whine_detune / 1200.0).exp2();
        let whine_inc = [f0 / self.sr, f0 * det / self.sr, 2.0 * f0 / self.sr, 0.5 * f0 / self.sr];
        let whine_w = [1.0, 0.7, 0.3, p.whine_shaft];
        let whine_gain = p.whine_level * (0.35 + 0.65 * r1) * 0.5;

        // Roar: cutoff tracks RPM over ±2 octaves, turbulence AM.
        let roar_cut = p.roar_hz * (2.0 * (r - 1.0)).exp2();
        self.roar_lp.set(FilterMode::LowPass, roar_cut, p.roar_resonance, self.sr);
        self.roar_bp.set(FilterMode::BandPass, roar_cut * 2.5, 0.3, self.sr);
        self.rumble_phase += p.rumble_hz * dt;
        if self.rumble_phase >= 1.0 {
            self.rumble_phase -= self.rumble_phase.floor();
            self.rumble_target = self.rng.next_bipolar();
        }
        self.rumble += (self.rumble_target - self.rumble) * (dt * p.rumble_hz * 3.0).min(1.0);
        let am = 1.0 + p.rumble * 0.5 * self.rumble;
        let roar_gain = p.roar_level * (0.35 + 0.65 * r1) * am;
        let drive = 1.0 + p.roar_drive * 3.0 + self.boost_s * 2.0;
        let drive_norm = 0.75 / (1.0 + 0.15 * drive);

        // Hiss, burner, wind.
        self.hiss_hp.set(FilterMode::HighPass, p.hiss_hz, 0.1, self.sr);
        let hiss_gain = p.hiss_level * r1 * r1 * 0.5;
        self.burner_hp.set(FilterMode::HighPass, 250.0, 0.2, self.sr);
        let burner_gain = p.burner_level * self.boost_s * 0.5;
        let burner_drive = 1.0 + p.burner_drive * 6.0;
        let thrum_inc = p.burner_thrum_hz / self.sr;
        self.wind_bp.set(FilterMode::BandPass, p.wind_hz * (0.5 + 0.9 * self.speed_s), 0.25, self.sr);
        let wind_gain = p.wind_level * self.speed_s * self.speed_s * 2.5;

        // Tube comb delay glides with RPM (fractional, interpolated per sample).
        let tube_target = self.tube_delay_samples();
        let tube_step = (tube_target - self.tube_delay) / n as f32;
        let tube_len = self.tube.len();

        // Damage sputter: random short drop-outs.
        if self.sputter_blocks > 0 {
            self.sputter_blocks -= 1;
        } else {
            self.sputter_gain = 1.0;
            if self.damage > 0.0 && self.rng.chance(self.damage * self.damage * 0.12) {
                self.sputter_blocks = 2 + (self.rng.next_u32() % 6);
                self.sputter_gain = self.rng.range(0.05, 0.4);
            }
        }
        let out_gain = p.gain * self.master_gain * self.sputter_gain;

        // --- audio rate ----------------------------------------------------------------
        let mut peak = self.peak;
        for s in out.iter_mut() {
            let white = self.noise.white();
            let pink = self.noise.pink();

            let mut whine = 0.0;
            for i in 0..4 {
                whine += (self.whine_phase[i] * TAU).sin() * whine_w[i];
                self.whine_phase[i] += whine_inc[i];
                if self.whine_phase[i] >= 1.0 {
                    self.whine_phase[i] -= 1.0;
                }
            }

            let body = self.roar_lp.tick(pink * 2.5) + 0.5 * self.roar_bp.tick(white);
            let roar = soft_clip(body * drive) * drive_norm * roar_gain;

            let bus = whine * whine_gain + roar;

            // Feedback comb with linear interpolation.
            self.tube_delay += tube_step;
            let d = self.tube_delay;
            let di = d as usize;
            let frac = d - di as f32;
            let i0 = (self.tube_pos + tube_len - di) % tube_len;
            let i1 = (i0 + tube_len - 1) % tube_len;
            let delayed = self.tube[i0] + (self.tube[i1] - self.tube[i0]) * frac;
            let y = bus + delayed * p.tube_feedback;
            self.tube[self.tube_pos] = y;
            self.tube_pos += 1;
            if self.tube_pos == tube_len {
                self.tube_pos = 0;
            }
            let tubed = bus + (y - bus) * p.tube_mix;

            let hiss = self.hiss_hp.tick(white) * hiss_gain;
            let thrum = 0.6 + 0.4 * (self.thrum_phase * TAU).sin();
            self.thrum_phase += thrum_inc;
            if self.thrum_phase >= 1.0 {
                self.thrum_phase -= 1.0;
            }
            let burner = soft_clip(self.burner_hp.tick(white) * burner_drive) * thrum * burner_gain;
            let wind = self.wind_bp.tick(white) * wind_gain;

            let y = limit((tubed + hiss + burner + wind) * out_gain);
            *s = y;
            peak = (peak * self.peak_decay).max(y.abs());
        }
        self.peak = if peak.is_finite() { peak } else { 0.0 };
        if !self.tube[self.tube_pos].is_finite() {
            self.tube.iter_mut().for_each(|x| *x = 0.0);
        }
    }
}

#[inline]
fn clamp01(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}
