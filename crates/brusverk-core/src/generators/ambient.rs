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
        horns_rate: "horns/per_minute" = 0.0, lin(0.0, 60.0);
        horns_level: "horns/level" = 0.6, UNIT;
        cheer_dark: "cheer/dark" = 0.0, UNIT;
        room: "arena/room" = 0.0, UNIT;
        react_level: "reactions/level" = 0.8, UNIT;
        chant_bpm: "chant/bpm" = 120.0, lin(60.0, 200.0);
    }
}

/// Which eighth-notes of the bar the crowd claps on: clap, clap, clap-clap-clap.
const CHANT_CLAPS: [bool; 8] = [true, false, true, false, true, true, true, false];

#[derive(Clone, Copy, Default)]
struct Horn {
    /// Seconds left; <= 0 is idle.
    left: f32,
    len: f32,
    hz: f32,
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
    // Air horns have their own random stream, so a crowd without them sounds exactly as before.
    horn_rng: crate::math::Rng,
    horns: [Horn; 2],
    horn_osc: [[Oscillator; 2]; 2],
    horn_tone: Svf,
    // Reactions (groan, boo, chant, goal) have their own noise, for the same reason.
    react_noise: Noise,
    react_rng: crate::math::Rng,
    goal_env: f32,
    groan_was: f32,
    /// Seconds into the current groan; negative when there is none.
    groan_t: f32,
    groan_bp: [Svf; 2],
    boo_env: f32,
    boo_bp: [Svf; 2],
    boo_drift: SlowNoise,
    chant_on: bool,
    /// Position in the chant's bar, 0..1.
    chant_phase: f32,
    chant_slot: usize,
    clap_env: f32,
    clap_soft: f32,
    clap_bp: Svf,
    shout_env: f32,
    shout_soft: f32,
    shout_bp: [Svf; 2],
    room: crate::blocks::Reverb,
}

impl Generator for Crowd {
    type P = CrowdParams;
    const NAME: &'static str = "crowd";
    const CATEGORY: &'static str = "ambient";
    const DOC: &'static str = "Crowd walla from a few people to a stadium; excitement adds cheering and roar. Reactions for sport: groan, boo, a clapping chant, a goal swell.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "size", default: 0.5, doc: "A few voices to a packed arena" },
        InputSpec { name: "excitement", default: 0.1, doc: "Murmur to cheering roar" },
        InputSpec { name: "groan", default: 0.0, doc: "Raise it to make the crowd groan once (a near miss): an 'ooohh' over about a second. Lower it again before the next one" },
        InputSpec { name: "boo", default: 0.0, doc: "Hold it up and the crowd boos" },
        InputSpec { name: "chant", default: 0.0, doc: "Hold it up for a clapping chant: clap, clap, clap-clap-clap, with a shout on the first two" },
        InputSpec { name: "goal", default: 0.0, doc: "Hold it up after a goal: the roar swells for about a second, holds, and dies away after you lower it" },
    ];

    fn presets() -> Vec<(&'static str, CrowdParams)> {
        vec![
            ("Tavern", CrowdParams { formant_hz: 560.0, syllable_hz: 3.2, cheer_level: 0.3, roar_level: 0.1, ..Default::default() }),
            ("Stadium", CrowdParams { formant_hz: 850.0, formant_spread: 1.2, cheer_level: 0.9, roar_level: 0.8, ..Default::default() }),
            // An indoor arena, tuned against recordings of hockey crowds: the cheer sits near
            // 1 kHz, not up where it hisses, and the room slaps back.
            ("Arena", CrowdParams { formant_hz: 620.0, formant_spread: 1.0, syllable_hz: 3.6, cheer_level: 0.75, cheer_hz: 1250.0, cheer_dark: 1.0, roar_level: 0.8, room: 0.6, ..Default::default() }),
            ("Festival", CrowdParams { formant_hz: 760.0, formant_spread: 1.1, syllable_hz: 4.6, murmur_level: 0.8, cheer_level: 0.85, roar_level: 0.6, horns_rate: 14.0, horns_level: 0.6, ..Default::default() }),
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
            horn_rng: crate::math::Rng::new(crate::blocks::mix_seed(0x62_0400)),
            horns: [Horn::default(); 2],
            horn_osc: [[Oscillator::new(0x62_0401), Oscillator::new(0x62_0402)], [Oscillator::new(0x62_0403), Oscillator::new(0x62_0404)]],
            horn_tone: Svf::default(),
            react_noise: Noise::new(0x62_0500),
            react_rng: crate::math::Rng::new(crate::blocks::mix_seed(0x62_0501)),
            goal_env: 0.0,
            groan_was: 0.0,
            groan_t: -1.0,
            groan_bp: [Svf::default(); 2],
            boo_env: 0.0,
            boo_bp: [Svf::default(); 2],
            boo_drift: SlowNoise::new(0x62_0502),
            chant_on: false,
            chant_phase: 0.0,
            chant_slot: 7,
            clap_env: 0.0,
            clap_soft: 0.0,
            clap_bp: Svf::default(),
            shout_env: 0.0,
            shout_soft: 0.0,
            shout_bp: [Svf::default(); 2],
            room: crate::blocks::Reverb::new(sr),
        }
    }

    // Each voice indexes four parallel arrays; a range loop reads clearer than a 4-way zip.
    #[allow(clippy::needless_range_loop)]
    fn block(&mut self, x: &[f32], p: &CrowdParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let size = x[0];
        // A goal: the roar builds for about a second, holds, and takes a few seconds to settle.
        let goal_secs = if x[5] > self.goal_env { 0.4 } else { 2.5 };
        self.goal_env += (x[5] - self.goal_env) * (1.0 - (-dt / goal_secs).exp());
        let ex = x[1].max(self.goal_env);
        let mut amp = [0.0f32; VOICES];
        for k in 0..VOICES {
            let spot = k as f32 / (VOICES - 1) as f32;
            let rate = p.syllable_hz * (0.7 + 0.6 * spot) * (1.0 + ex);
            let t = 0.5 + 0.5 * self.talk[k].advance(rate, dt);
            let talking = 0.12 + 0.88 * t * t;
            // Voices join one by one as the crowd grows.
            let joined = (size * VOICES as f32 - k as f32 + 2.0).clamp(0.0, 1.0);
            amp[k] = talking * joined;
            let f = p.formant_hz * (p.formant_spread * (spot - 0.5) * 2.0 + 0.25 * self.pitch[k].advance(rate * 0.5, dt) + 0.4 * ex).exp2();
            self.voice[k].set(FilterMode::BandPass, f, 0.75, sr);
        }
        self.cheer.set(FilterMode::BandPass, p.cheer_hz, 0.4, sr);
        self.roar.set(FilterMode::LowPass, 400.0, 0.1, sr);
        let cheer_gain = p.cheer_level * ex * ex * (0.7 + 0.3 * self.cheer_swell.advance(0.6, dt)) * 0.8;
        let roar_gain = p.roar_level * ex * 2.0 * (1.0 + 0.8 * self.goal_env);
        let murmur_gain = p.murmur_level * 4.5 / (1.0 + 2.0 * size);
        let level = (0.4 + 0.6 * size) * p.gain;
        let horns = p.horns_rate > 0.0 && p.horns_level > 0.0;
        if horns {
            // A rowdy outdoor crowd: someone always has an air horn. More of them when excited.
            let start_p = p.horns_rate / 60.0 * (0.4 + 0.6 * ex) * (0.5 + 0.5 * size) * dt;
            for h in self.horns.iter_mut() {
                if h.left <= 0.0 && self.horn_rng.chance(start_p) {
                    let len = self.horn_rng.range(0.35, 1.4);
                    *h = Horn { left: len, len, hz: self.horn_rng.range(330.0, 480.0) };
                }
            }
            self.horn_tone.set(FilterMode::LowPass, 2200.0, 0.3, sr);
        }
        // ---- reactions ----
        let react = p.react_level * (0.4 + 0.6 * size) * p.gain;
        // Groan: starts when the input rises, then runs its second or so whatever the input does.
        if x[2] > 0.5 && self.groan_was <= 0.5 {
            self.groan_t = 0.0;
        }
        self.groan_was = x[2];
        let mut groan_gain = 0.0;
        if self.groan_t >= 0.0 {
            let t = self.groan_t;
            let rise = (t / 0.3).min(1.0);
            groan_gain = rise * rise * (3.0 - 2.0 * rise) * (-(t - 0.3).max(0.0) / 0.45).exp() * react * 4.0;
            // "ooOOohh": the vowel closes and sinks as it dies.
            let sink = 1.0 - 0.22 * (t / 1.4).min(1.0);
            self.groan_bp[0].set(FilterMode::BandPass, 480.0 * sink, 0.8, sr);
            self.groan_bp[1].set(FilterMode::BandPass, 880.0 * sink, 0.8, sr);
            self.groan_t = if t > 2.2 { -1.0 } else { t + dt };
        }
        // Boo: a held "oo", many voices near the same low vowel, drifting.
        self.boo_env += (x[3] - self.boo_env) * (1.0 - (-dt / 0.35).exp());
        let boo_gain = if self.boo_env > 1e-3 { self.boo_env * react * 2.8 } else { 0.0 };
        if boo_gain > 0.0 {
            let d = (0.1 * self.boo_drift.advance(0.7, dt)).exp2();
            self.boo_bp[0].set(FilterMode::BandPass, 400.0 * d, 0.85, sr);
            self.boo_bp[1].set(FilterMode::BandPass, 840.0 / d, 0.8, sr);
        }
        // Chant: the bar restarts when the input comes up, so the first clap is on time.
        let chanting = x[4] > 0.05;
        if chanting && !self.chant_on {
            self.chant_phase = 0.0;
            self.chant_slot = 7;
        }
        self.chant_on = chanting;
        if chanting {
            let slot = ((self.chant_phase * 8.0) as usize).min(7);
            if slot != self.chant_slot {
                self.chant_slot = slot;
                if CHANT_CLAPS[slot] {
                    self.clap_env = self.react_rng.range(0.8, 1.0);
                    self.clap_bp.set(FilterMode::BandPass, self.react_rng.range(1000.0, 1300.0), 0.35, sr);
                }
                if slot == 0 || slot == 2 {
                    self.shout_env = self.react_rng.range(0.8, 1.0);
                }
            }
            self.chant_phase = (self.chant_phase + dt * p.chant_bpm / 240.0).fract();
            self.shout_bp[0].set(FilterMode::BandPass, 640.0, 0.8, sr);
            self.shout_bp[1].set(FilterMode::BandPass, 1150.0, 0.75, sr);
        }
        let chant_gain = x[4] * react;
        let (clap_decay, shout_decay) = ((-1.0 / (0.07 * sr)).exp(), (-1.0 / (0.1 * sr)).exp());
        // A thousand hands are never together: the clap swells in over a few milliseconds.
        let (clap_attack, shout_attack) = (1.0 - (-1.0 / (0.006 * sr)).exp(), 1.0 - (-1.0 / (0.02 * sr)).exp());
        let reacting = groan_gain > 0.0 || boo_gain > 0.0 || chanting || self.clap_soft > 1e-4 || self.shout_soft > 1e-4;
        for o in out.iter_mut() {
            let (w, pk) = (self.noise.white(), self.noise.pink());
            let mut y = 0.0;
            for (voice, a) in self.voice.iter_mut().zip(amp) {
                y += voice.tick(pk) * a;
            }
            let mut horn = 0.0;
            if horns {
                for (h, osc) in self.horns.iter_mut().zip(self.horn_osc.iter_mut()) {
                    if h.left > 0.0 {
                        let age = h.len - h.left;
                        // Quick attack, a breathy end that sags in pitch as the can runs out.
                        let env = (age / 0.025).min(1.0) * (h.left / 0.12).min(1.0);
                        let sag = 1.0 - 0.06 * (1.0 - h.left / h.len).powi(3);
                        let f = h.hz * sag / sr;
                        horn += (osc[0].next(Waveform::Saw, f, 0.5) + osc[1].next(Waveform::Pulse, f * 1.005, 0.4)) * env;
                        h.left -= 1.0 / sr;
                    }
                }
                horn = soft_clip(self.horn_tone.tick(horn) * 1.5) * p.horns_level * 0.55;
            }
            // `cheer/dark` swaps the cheer's white noise for pink: less top, as indoors.
            let cheer_in = w + (pk * 2.5 - w) * p.cheer_dark;
            *o = (y * murmur_gain + self.cheer.tick(cheer_in) * cheer_gain + self.roar.tick(self.brown.tick(w)) * roar_gain) * level + horn * p.gain;
            if reacting {
                let rp = self.react_noise.pink();
                let mut r = 0.0;
                if groan_gain > 0.0 {
                    r += (self.groan_bp[0].tick(rp) + 0.6 * self.groan_bp[1].tick(rp)) * groan_gain;
                }
                if boo_gain > 0.0 {
                    r += (self.boo_bp[0].tick(rp) + self.boo_bp[1].tick(rp)) * boo_gain;
                }
                self.clap_soft += (self.clap_env - self.clap_soft) * clap_attack;
                self.shout_soft += (self.shout_env - self.shout_soft) * shout_attack;
                r += self.clap_bp.tick(rp * self.clap_soft) * chant_gain * 20.0;
                r += (self.shout_bp[0].tick(rp * self.shout_soft) + 0.7 * self.shout_bp[1].tick(rp * self.shout_soft)) * chant_gain * 5.0;
                self.clap_env *= clap_decay;
                self.shout_env *= shout_decay;
                *o += r;
            }
            if p.room > 0.0 {
                // An indoor arena: a short, dense slap back off the far side.
                *o += self.room.tick(*o, 0.7, 0.55) * p.room * 0.5;
            }
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
        hum_level: "hum/level" = 0.08, UNIT;
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
        self.tick.set(FilterMode::BandPass, 2500.0, 0.9, sr);
        let hiss_gain = p.hiss_level * (0.15 + 0.85 * inter) * (0.7 + 0.3 * self.swell.advance(1.5, dt)) * 1.6;
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
                + (self.tick.tick(click) * 5.0 + click) * p.click_level
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

// ---------------------------------------------------------------------------------------------
// Beam (held laser / energy weapon)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// A beam weapon held on target: ring-modulated buzz, a sizzling resonance that never sits
    /// still, crackle and a sub. `firing` is the trigger button.
    BeamParams / BeamParamId {
        hz: "beam/hz" = 190.0, exp(60.0, 1200.0);
        edge: "beam/edge" = 0.6, UNIT;
        ring_ratio: "beam/ring_ratio" = 7.3, lin(1.0, 16.0);
        sizzle_hz: "sizzle/hz" = 3800.0, exp(800.0, 9000.0);
        sizzle_level: "sizzle/level" = 0.5, UNIT;
        crackle_rate: "sizzle/crackle_per_second" = 90.0, exp(1.0, 800.0);
        sub_level: "beam/sub" = 0.5, UNIT;
        attack_ms: "beam/attack_ms" = 12.0, lin(1.0, 200.0);
        release_ms: "beam/release_ms" = 90.0, lin(5.0, 800.0);
        gain: "master/gain" = 0.8, GAIN;
    }
}

pub struct Beam {
    sr: f32,
    gate: f32,
    osc: [Oscillator; 2],
    ring: Phasor,
    sub: Phasor,
    noise: Noise,
    crackle: Dust,
    crackle_env: f32,
    sizzle: Svf,
    drift: SlowNoise,
    body: Svf,
}

impl Generator for Beam {
    type P = BeamParams;
    const NAME: &'static str = "beam";
    const CATEGORY: &'static str = "fx";
    const DOC: &'static str = "Held laser or energy beam: hold `firing` while the weapon is on.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "firing", default: 0.0, doc: "1 while the beam is on; it snaps on and releases quickly" },
        InputSpec { name: "intensity", default: 0.6, doc: "Beam power: pitch, edge and sizzle" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.0;

    fn presets() -> Vec<(&'static str, BeamParams)> {
        vec![
            ("Mining laser", BeamParams { hz: 110.0, edge: 0.8, ring_ratio: 3.1, sizzle_hz: 2400.0, crackle_rate: 200.0, ..Default::default() }),
            ("Tractor beam", BeamParams { hz: 320.0, edge: 0.25, ring_ratio: 1.5, sizzle_level: 0.25, crackle_rate: 8.0, attack_ms: 120.0, release_ms: 400.0, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Beam {
            sr,
            gate: 0.0,
            osc: [Oscillator::new(0x65_0001), Oscillator::new(0x65_0002)],
            ring: Phasor::default(),
            sub: Phasor::default(),
            noise: Noise::new(0x65_0003),
            crackle: Dust::new(0x65_0004),
            crackle_env: 0.0,
            sizzle: Svf::default(),
            drift: SlowNoise::new(0x65_0005),
            body: Svf::default(),
        }
    }

    fn snap(&mut self, x: &[f32], _p: &BeamParams) {
        self.gate = x[0];
    }

    fn block(&mut self, x: &[f32], p: &BeamParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (firing, intensity) = (x[0], x[1]);
        let secs = if firing > self.gate { p.attack_ms } else { p.release_ms } * 0.001;
        self.gate += (firing - self.gate) * crate::blocks::settle_coef(secs, dt);
        if self.gate < 1e-4 && firing <= 0.0 {
            out.iter_mut().for_each(|s| *s = 0.0);
            return;
        }
        let f = p.hz * (0.8 + 0.5 * intensity);
        let inc = [f / sr, f * 1.011 / sr];
        self.body.set(FilterMode::LowPass, 600.0 + 5000.0 * p.edge * (0.4 + 0.6 * intensity), 0.35, sr);
        self.sizzle.set(FilterMode::BandPass, p.sizzle_hz * (0.4 * self.drift.advance(11.0, dt)).exp2(), 0.9, sr);
        let crackle_p = p.crackle_rate * intensity / sr;
        let crackle_decay = (-1.0 / (0.004 * sr)).exp();
        let ring_inc = f * p.ring_ratio / sr;
        let sizzle_gain = p.sizzle_level * (0.3 + 0.7 * intensity) * 0.35;
        let level = self.gate * (0.5 + 0.5 * intensity) * p.gain;
        for s in out.iter_mut() {
            self.ring.tick(ring_inc);
            self.sub.tick(inc[0] * 0.5);
            let saws = self.osc[0].next(Waveform::Saw, inc[0], 0.5) + self.osc[1].next(Waveform::Saw, inc[1], 0.5);
            // Ring modulation puts inharmonic sidebands around the buzz: the "energy" colour.
            let buzz = self.body.tick(saws * (1.0 - 0.6 * p.edge + 0.6 * p.edge * self.ring.sin()));
            let d = self.crackle.tick(crackle_p);
            if d > 0.0 {
                self.crackle_env = self.crackle_env.max(d);
            }
            self.crackle_env *= crackle_decay;
            let w = self.noise.white();
            let sizzle = self.sizzle.tick(w) * sizzle_gain + w * self.crackle_env * 0.5 * p.sizzle_level;
            *s = (buzz * 0.4 + sizzle + self.sub.sin() * p.sub_level * 0.4) * level;
        }
    }
}
