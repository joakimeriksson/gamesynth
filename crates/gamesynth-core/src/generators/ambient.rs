//! Ambient, sci-fi and signal generators: electric field, tension drone, crowd, radio static,
//! siren.

use crate::blocks::{hz_coef, Brown, Dust, OnePole, Phasor, SlowNoise};
use crate::filter::{FilterMode, Svf};
use crate::math::{soft_clip, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::osc::{Oscillator, Waveform};
use crate::params::{exp, int, lin, ParamKind, GAIN, UNIT};

// ---------------------------------------------------------------------------------------------
// Electric (hum, force field, arcing)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Electrical hum with beating, buzz from saturation, flicker and arcing crackle.
    ElectricParams / ElectricParamId {
        hum_hz: "hum/hz" = 55.0, exp(30.0, 400.0);
        hum_level: "hum/level" = 0.6, UNIT;
        brightness: "hum/brightness" = 0.5, UNIT;
        buzz: "hum/buzz" = 0.4, UNIT;
        beat_hz: "hum/beat_hz" = 0.7, lin(0.0, 8.0);
        flicker: "hum/flicker" = 0.5, UNIT;
        arc_rate: "arcs/per_second" = 12.0, exp(0.2, 200.0);
        arc_level: "arcs/level" = 0.6, UNIT;
        arc_hz: "arcs/hz" = 3500.0, exp(800.0, 9000.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Electric {
    sr: f32,
    hum: [Phasor; 2],
    noise: Noise,
    arcs: Dust,
    arc_env: f32,
    arc: Svf,
    flick: SlowNoise,
}

impl Generator for Electric {
    type P = ElectricParams;
    const NAME: &'static str = "electric";
    const CATEGORY: &'static str = "ambient";
    const DOC: &'static str = "Transformer hum, force field or faulty wiring: hum, buzz, flicker and arcs.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "power", default: 0.6, doc: "Overall field strength" },
        InputSpec { name: "instability", default: 0.2, doc: "Flicker and arcing" },
    ];

    fn presets() -> Vec<(&'static str, ElectricParams)> {
        vec![
            ("Force field", ElectricParams { hum_hz: 110.0, brightness: 0.8, buzz: 0.2, beat_hz: 3.0, arc_rate: 4.0, arc_hz: 5200.0, ..Default::default() }),
            ("Substation", ElectricParams { hum_hz: 50.0, brightness: 0.35, buzz: 0.7, beat_hz: 0.3, arc_level: 0.3, ..Default::default() }),
            ("Tesla coil", ElectricParams { hum_hz: 160.0, buzz: 0.9, flicker: 0.9, arc_rate: 90.0, arc_level: 0.9, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Electric { sr, hum: [Phasor::default(); 2], noise: Noise::new(0x60_0001), arcs: Dust::new(0x60_0002), arc_env: 0.0, arc: Svf::default(), flick: SlowNoise::new(0x60_0003) }
    }

    fn block(&mut self, x: &[f32], p: &ElectricParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (power, inst) = (x[0], x[1]);
        let slope = 2.2 - 1.6 * p.brightness;
        let mut amps = [0.0f32; 7];
        for (k, a) in amps.iter_mut().enumerate() {
            let n = (k + 1) as f32;
            // Mains hum is dominated by odd harmonics.
            *a = n.powf(-slope) * if k % 2 == 0 { 1.0 } else { 0.5 };
        }
        let flick = 1.0 - p.flicker * inst * (0.5 + 0.5 * self.flick.advance(18.0, dt));
        self.arc.set(FilterMode::BandPass, p.arc_hz, 0.6, sr);
        let arc_p = p.arc_rate * inst * inst / sr;
        let arc_decay = (-1.0 / (0.006 * sr)).exp();
        let drive = 1.0 + p.buzz * 8.0;
        let hum_gain = p.hum_level * flick / (1.0 + p.buzz * 2.0);
        let inc = [p.hum_hz / sr, (p.hum_hz + p.beat_hz) / sr];
        let level = power * p.gain;
        for o in out.iter_mut() {
            self.hum[0].tick(inc[0]);
            self.hum[1].tick(inc[1]);
            let (a, b) = (self.hum[0].phase * TAU, self.hum[1].phase * TAU);
            let mut hum = 0.0;
            for (k, amp) in amps.iter().enumerate() {
                hum += amp * (a * (k + 1) as f32).sin();
            }
            hum += 0.6 * b.sin() + 0.3 * (b * 3.0).sin();
            let d = self.arcs.tick(arc_p);
            if d > 0.0 {
                self.arc_env = self.arc_env.max(d);
            }
            self.arc_env *= arc_decay;
            let burst = self.noise.white() * self.arc_env;
            let arc = self.arc.tick(burst) + 0.3 * burst;
            *o = (soft_clip(hum * 0.6 * drive) * hum_gain + arc * p.arc_level * 2.5) * level;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Drone (tension bed)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Slowly evolving drone whose dissonant partials, tremolo and airy noise rise with tension.
    DroneParams / DroneParamId {
        root_hz: "drone/root_hz" = 55.0, exp(20.0, 220.0);
        detune: "drone/detune_cents" = 8.0, lin(0.0, 40.0);
        dissonance: "drone/dissonance" = 0.5, UNIT;
        motion: "drone/motion_hz" = 0.15, exp(0.01, 2.0);
        sub_level: "drone/sub" = 0.5, UNIT;
        drive: "drone/drive" = 0.3, UNIT;
        air_level: "air/level" = 0.3, UNIT;
        air_hz: "air/hz" = 3000.0, exp(500.0, 9000.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

const PARTIALS: usize = 8;
/// Consonant stack first, then a minor ninth, a tritone and a beating high cluster.
const RATIOS: [f32; PARTIALS] = [1.0, 1.498, 2.0, 2.119, 3.0, 5.657, 8.03, 8.47];
const TENSE: [bool; PARTIALS] = [false, false, false, true, false, true, true, true];
const WEIGHT: [f32; PARTIALS] = [1.0, 0.5, 0.6, 0.7, 0.35, 0.5, 0.3, 0.3];

pub struct Drone {
    sr: f32,
    partial: [Phasor; PARTIALS],
    wander: [SlowNoise; PARTIALS],
    sub: Phasor,
    trem: Phasor,
    noise: Noise,
    air: Svf,
    air_drift: SlowNoise,
    dark: OnePole,
}

impl Generator for Drone {
    type P = DroneParams;
    const NAME: &'static str = "drone";
    const CATEGORY: &'static str = "ambient";
    const DOC: &'static str = "Tension bed for horror and suspense: consonant at rest, dissonant and restless under tension.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "tension", default: 0.2, doc: "Brings in dissonant partials, tremolo and air" },
        InputSpec { name: "darkness", default: 0.5, doc: "Closes the top end and deepens the sub" },
    ];

    fn presets() -> Vec<(&'static str, DroneParams)> {
        vec![
            ("Deep space", DroneParams { root_hz: 36.0, motion: 0.05, dissonance: 0.25, air_level: 0.15, ..Default::default() }),
            ("Dread", DroneParams { root_hz: 49.0, dissonance: 0.9, motion: 0.4, drive: 0.6, air_level: 0.5, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Drone {
            sr,
            partial: [Phasor::default(); PARTIALS],
            wander: core::array::from_fn(|k| SlowNoise::new(0x61_0001 + k as u32 * 97)),
            sub: Phasor::default(),
            trem: Phasor::default(),
            noise: Noise::new(0x61_1001),
            air: Svf::default(),
            air_drift: SlowNoise::new(0x61_1002),
            dark: OnePole::default(),
        }
    }

    fn block(&mut self, x: &[f32], p: &DroneParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (tension, darkness) = (x[0], x[1]);
        let mut amp = [0.0f32; PARTIALS];
        let mut inc = [0.0f32; PARTIALS];
        for k in 0..PARTIALS {
            let wander = self.wander[k].advance(p.motion * (1.0 + 0.13 * k as f32), dt);
            let presence = if TENSE[k] { tension * p.dissonance * 1.4 } else { 1.0 };
            amp[k] = WEIGHT[k] * presence * (0.6 + 0.4 * wander);
            let cents = p.detune * wander * if k % 2 == 0 { 1.0 } else { -1.0 };
            inc[k] = (p.root_hz * RATIOS[k] * (cents / 1200.0).exp2()).min(sr * 0.4) / sr;
        }
        self.air.set(FilterMode::BandPass, p.air_hz * (1.0 + 0.3 * self.air_drift.advance(0.3, dt)), 0.8, sr);
        let dark_coef = hz_coef(6000.0 + (300.0 - 6000.0) * darkness, sr);
        let trem_inc = (0.5 + 6.0 * tension * tension) / sr;
        let trem_depth = 0.3 * tension;
        let sub_gain = p.sub_level * (0.5 + 0.5 * darkness);
        let air_gain = p.air_level * tension * 1.5;
        let drive = 1.0 + p.drive * 4.0;
        for o in out.iter_mut() {
            let mut y = 0.0;
            for k in 0..PARTIALS {
                self.partial[k].tick(inc[k]);
                y += amp[k] * self.partial[k].sin();
            }
            self.sub.tick(inc[0] * 0.5);
            self.trem.tick(trem_inc);
            let body = self.dark.lp(soft_clip(y * 0.3 * drive), dark_coef) + self.sub.sin() * sub_gain * 0.5;
            let air = self.air.tick(self.noise.white()) * air_gain * 0.2;
            *o = (body + air) * (1.0 - trem_depth * (0.5 + 0.5 * self.trem.sin())) * p.gain * 0.6;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Crowd
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Crowd murmur: formant-filtered noise "voices" with independent syllable rhythms, plus
    /// cheering and a low roar when excited.
    CrowdParams / CrowdParamId {
        formant_hz: "murmur/formant_hz" = 700.0, exp(300.0, 2000.0);
        formant_spread: "murmur/spread_octaves" = 0.8, lin(0.0, 2.0);
        syllable_hz: "murmur/syllable_hz" = 4.0, lin(1.0, 10.0);
        murmur_level: "murmur/level" = 0.7, UNIT;
        cheer_level: "cheer/level" = 0.6, UNIT;
        cheer_hz: "cheer/hz" = 2600.0, exp(1000.0, 6000.0);
        roar_level: "cheer/roar" = 0.4, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

const VOICES: usize = 8;

pub struct Crowd {
    sr: f32,
    noise: Noise,
    brown: Brown,
    voice: [Svf; VOICES],
    talk: [SlowNoise; VOICES],
    pitch: [SlowNoise; VOICES],
    cheer: Svf,
    cheer_swell: SlowNoise,
    roar: Svf,
}

impl Generator for Crowd {
    type P = CrowdParams;
    const NAME: &'static str = "crowd";
    const CATEGORY: &'static str = "ambient";
    const DOC: &'static str = "Crowd walla from a few people to a stadium; excitement adds cheering and roar.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "size", default: 0.5, doc: "A few voices to a packed arena" },
        InputSpec { name: "excitement", default: 0.1, doc: "Murmur to cheering roar" },
    ];

    fn presets() -> Vec<(&'static str, CrowdParams)> {
        vec![
            ("Tavern", CrowdParams { formant_hz: 560.0, syllable_hz: 3.2, cheer_level: 0.3, roar_level: 0.1, ..Default::default() }),
            ("Stadium", CrowdParams { formant_hz: 850.0, formant_spread: 1.2, cheer_level: 0.9, roar_level: 0.8, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Crowd {
            sr,
            noise: Noise::new(0x62_0001),
            brown: Brown::default(),
            voice: [Svf::default(); VOICES],
            talk: core::array::from_fn(|k| SlowNoise::new(0x62_0100 + k as u32 * 131)),
            pitch: core::array::from_fn(|k| SlowNoise::new(0x62_0200 + k as u32 * 137)),
            cheer: Svf::default(),
            cheer_swell: SlowNoise::new(0x62_0300),
            roar: Svf::default(),
        }
    }

    // Each voice indexes four parallel arrays; a range loop reads clearer than a 4-way zip.
    #[allow(clippy::needless_range_loop)]
    fn block(&mut self, x: &[f32], p: &CrowdParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (size, ex) = (x[0], x[1]);
        let mut amp = [0.0f32; VOICES];
        for k in 0..VOICES {
            let spot = k as f32 / (VOICES - 1) as f32;
            let rate = p.syllable_hz * (0.7 + 0.6 * spot) * (1.0 + ex);
            let talking = self.talk[k].advance(rate, dt).max(0.0).powf(1.5);
            // Voices join one by one as the crowd grows.
            let joined = (size * VOICES as f32 - k as f32 + 2.0).clamp(0.0, 1.0);
            amp[k] = talking * joined;
            let f = p.formant_hz * (p.formant_spread * (spot - 0.5) * 2.0 + 0.25 * self.pitch[k].advance(rate * 0.5, dt) + 0.4 * ex).exp2();
            self.voice[k].set(FilterMode::BandPass, f, 0.75, sr);
        }
        self.cheer.set(FilterMode::BandPass, p.cheer_hz, 0.4, sr);
        self.roar.set(FilterMode::LowPass, 400.0, 0.1, sr);
        let cheer_gain = p.cheer_level * ex * ex * (0.7 + 0.3 * self.cheer_swell.advance(0.6, dt)) * 0.8;
        let roar_gain = p.roar_level * ex * 2.0;
        let murmur_gain = p.murmur_level * 6.0 / (1.0 + 2.0 * size);
        let level = (0.4 + 0.6 * size) * p.gain;
        for o in out.iter_mut() {
            let (w, pk) = (self.noise.white(), self.noise.pink());
            let mut y = 0.0;
            for (voice, a) in self.voice.iter_mut().zip(amp) {
                y += voice.tick(pk) * a;
            }
            *o = (y * murmur_gain + self.cheer.tick(w) * cheer_gain + self.roar.tick(self.brown.tick(w)) * roar_gain) * level;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Radio (static, interference, geiger clicks)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Radio static: band-limited hiss, clicks, a drifting heterodyne whistle and mains hum.
    RadioParams / RadioParamId {
        hiss_level: "hiss/level" = 0.5, UNIT;
        hiss_hz: "hiss/hz" = 2500.0, exp(500.0, 8000.0);
        click_rate: "clicks/per_second" = 40.0, exp(1.0, 2000.0);
        click_level: "clicks/level" = 0.6, UNIT;
        whistle_hz: "whistle/hz" = 1800.0, exp(200.0, 6000.0);
        whistle_level: "whistle/level" = 0.25, UNIT;
        drift: "whistle/drift_hz" = 0.3, exp(0.02, 5.0);
        hum_level: "hum/level" = 0.15, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Radio {
    sr: f32,
    noise: Noise,
    clicks: Dust,
    hiss: Svf,
    tick: Svf,
    whistle: Phasor,
    hum: Phasor,
    swell: SlowNoise,
    tune: SlowNoise,
    fade: SlowNoise,
}

impl Generator for Radio {
    type P = RadioParams;
    const NAME: &'static str = "radio";
    const CATEGORY: &'static str = "ambient";
    const DOC: &'static str = "Radio static and interference; `activity` alone works as a geiger counter.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "interference", default: 0.5, doc: "Hiss, whistle and hum level" },
        InputSpec { name: "activity", default: 0.2, doc: "Click rate (geiger counter, sparks on the line)" },
    ];

    fn presets() -> Vec<(&'static str, RadioParams)> {
        vec![
            ("Geiger counter", RadioParams { hiss_level: 0.05, whistle_level: 0.0, hum_level: 0.0, click_rate: 300.0, click_level: 0.9, ..Default::default() }),
            ("Shortwave", RadioParams { whistle_level: 0.5, drift: 0.8, hiss_hz: 1800.0, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Radio {
            sr,
            noise: Noise::new(0x63_0001),
            clicks: Dust::new(0x63_0002),
            hiss: Svf::default(),
            tick: Svf::default(),
            whistle: Phasor::default(),
            hum: Phasor::default(),
            swell: SlowNoise::new(0x63_0003),
            tune: SlowNoise::new(0x63_0004),
            fade: SlowNoise::new(0x63_0005),
        }
    }

    fn block(&mut self, x: &[f32], p: &RadioParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (inter, activity) = (x[0], x[1]);
        self.hiss.set(FilterMode::BandPass, p.hiss_hz, 0.2, sr);
        self.tick.set(FilterMode::BandPass, 2500.0, 0.6, sr);
        let hiss_gain = p.hiss_level * (0.15 + 0.85 * inter) * (0.7 + 0.3 * self.swell.advance(1.5, dt)) * 0.8;
        let click_p = p.click_rate * activity * activity / sr;
        let whistle_inc = p.whistle_hz * (0.7 * self.tune.advance(p.drift, dt)).exp2() / sr;
        let whistle_gain = p.whistle_level * inter * self.fade.advance(p.drift * 0.7, dt).max(0.0);
        let hum_gain = p.hum_level * inter;
        for o in out.iter_mut() {
            self.whistle.tick(whistle_inc);
            self.hum.tick(50.0 / sr);
            let h = self.hum.phase * TAU;
            let click = self.clicks.tick(click_p);
            *o = (self.hiss.tick(self.noise.white()) * hiss_gain
                + (self.tick.tick(click) * 2.0 + click * 0.5) * p.click_level
                + self.whistle.sin() * whistle_gain
                + (h.sin() + 0.5 * (h * 2.0).sin()) * hum_gain)
                * p.gain
                * 1.5;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Siren / alarm
// ---------------------------------------------------------------------------------------------

pub const SIREN_MODES: [&str; 4] = ["Wail", "Yelp", "TwoTone", "Pulse"];

model_params! {
    /// Siren or alarm: a swept or stepped tone through a horn resonance.
    SirenParams / SirenParamId {
        mode: "siren/mode" = 0.0, ParamKind::Enum(&SIREN_MODES);
        low_hz: "siren/low_hz" = 650.0, exp(100.0, 3000.0);
        high_hz: "siren/high_hz" = 1350.0, exp(100.0, 4000.0);
        rate: "siren/rate_hz" = 0.35, exp(0.05, 12.0);
        brightness: "siren/brightness" = 0.6, UNIT;
        horn_hz: "horn/hz" = 1600.0, exp(400.0, 5000.0);
        horn_res: "horn/resonance" = 0.5, lin(0.0, 0.9);
        voices: "siren/voices" = 1.0, int(1, 2);
        gain: "master/gain" = 0.8, GAIN;
    }
}

pub struct Siren {
    sr: f32,
    lfo: Phasor,
    osc: [Oscillator; 2],
    tone: [Phasor; 2],
    horn: Svf,
    gate: OnePole,
}

impl Generator for Siren {
    type P = SirenParams;
    const NAME: &'static str = "siren";
    const CATEGORY: &'static str = "ambient";
    const DOC: &'static str = "Emergency siren, klaxon or base alarm; urgency speeds it up.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "active", default: 1.0, doc: "Gate: 1 = sounding" },
        InputSpec { name: "urgency", default: 0.0, doc: "Speeds up the cycle up to 4x" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.03;

    fn presets() -> Vec<(&'static str, SirenParams)> {
        vec![
            ("Police yelp", SirenParams { mode: 1.0, rate: 3.2, low_hz: 700.0, high_hz: 1600.0, ..Default::default() }),
            ("Euro two-tone", SirenParams { mode: 2.0, rate: 0.9, low_hz: 440.0, high_hz: 587.0, brightness: 0.8, ..Default::default() }),
            ("Reactor alarm", SirenParams { mode: 3.0, rate: 1.6, high_hz: 880.0, brightness: 0.9, horn_hz: 1100.0, horn_res: 0.7, voices: 2.0, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Siren { sr, lfo: Phasor::default(), osc: [Oscillator::new(0x64_0001), Oscillator::new(0x64_0002)], tone: [Phasor::default(); 2], horn: Svf::default(), gate: OnePole::default() }
    }

    fn block(&mut self, x: &[f32], p: &SirenParams, out: &mut [f32]) {
        let sr = self.sr;
        let (active, urgency) = (x[0], x[1]);
        let mode = p.mode.round() as usize;
        let lfo_inc = p.rate * (1.0 + 3.0 * urgency) / sr;
        let span = (p.high_hz / p.low_hz).max(1e-3).ln();
        self.horn.set(FilterMode::BandPass, p.horn_hz, p.horn_res, sr);
        let gate_coef = hz_coef(120.0, sr);
        let second = if p.voices >= 1.5 { 1.0 } else { 0.0 };
        for o in out.iter_mut() {
            self.lfo.tick(lfo_inc);
            let ph = self.lfo.phase;
            let (shape, gate) = match mode {
                0 => (1.0 - (2.0 * ph - 1.0).abs(), 1.0),
                1 => (ph, 1.0),
                2 => (if ph < 0.5 { 0.0 } else { 1.0 }, 1.0),
                _ => (1.0, if ph < 0.5 { 1.0 } else { 0.0 }),
            };
            let f = p.low_hz * (span * shape).exp();
            let mut y = 0.0;
            for v in 0..2 {
                let inc = f * if v == 0 { 1.0 } else { 1.26 } / sr;
                self.tone[v].tick(inc);
                let voice = self.osc[v].next(Waveform::Pulse, inc, 0.35) * p.brightness + self.tone[v].sin() * (1.0 - p.brightness);
                y += voice * if v == 0 { 1.0 } else { second * 0.7 };
            }
            let g = self.gate.lp(gate * active, gate_coef);
            *o = (self.horn.tick(y) * 1.5 + y * 0.3) * g * p.gain * 0.36;
        }
    }
}
