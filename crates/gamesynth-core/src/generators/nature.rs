//! Nature and weather generators: wind, rain, fire, stream, ocean.

use crate::blocks::{hz_coef, Brown, Dust, OnePole, SlowNoise};
use crate::filter::{FilterMode, Svf};
use crate::math::{Rng, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::params::{exp, lin, GAIN, UNIT};

// ---------------------------------------------------------------------------------------------
// Wind
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Wind: resonant howl that tracks strength, low rumble, hiss and a gap whistle.
    WindParams / WindParamId {
        howl_hz: "howl/hz" = 520.0, exp(100.0, 4000.0);
        howl_res: "howl/resonance" = 0.78, lin(0.0, 0.95);
        howl_level: "howl/level" = 0.7, UNIT;
        gust_rate: "gusts/rate_hz" = 0.25, exp(0.02, 4.0);
        rumble_level: "rumble/level" = 0.5, UNIT;
        hiss_level: "hiss/level" = 0.4, UNIT;
        whistle_hz: "whistle/hz" = 1900.0, exp(400.0, 6000.0);
        whistle_level: "whistle/level" = 0.3, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Wind {
    sr: f32,
    noise: Noise,
    brown: Brown,
    body: [Svf; 2],
    rumble: Svf,
    hiss: Svf,
    whistle: Svf,
    gust: SlowNoise,
    drift: SlowNoise,
    whistle_drift: SlowNoise,
}

impl Generator for Wind {
    type P = WindParams;
    const NAME: &'static str = "wind";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Wind with gusts: howl, rumble, hiss and a whistle through gaps at high strength.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "strength", default: 0.4, doc: "Breeze to storm" },
        InputSpec { name: "gustiness", default: 0.5, doc: "How much the strength wanders by itself" },
    ];

    fn presets() -> Vec<(&'static str, WindParams)> {
        vec![
            ("Blizzard", WindParams { howl_hz: 800.0, howl_res: 0.75, hiss_level: 0.7, whistle_level: 0.5, gust_rate: 0.5, ..Default::default() }),
            ("Desert", WindParams { howl_hz: 320.0, howl_res: 0.4, hiss_level: 0.6, whistle_level: 0.0, rumble_level: 0.7, ..Default::default() }),
            ("Drafty corridor", WindParams { howl_hz: 420.0, howl_res: 0.85, rumble_level: 0.2, hiss_level: 0.1, whistle_hz: 1300.0, whistle_level: 0.6, gust_rate: 0.12, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Wind {
            sr,
            noise: Noise::new(0x50_0001),
            brown: Brown::default(),
            body: [Svf::default(); 2],
            rumble: Svf::default(),
            hiss: Svf::default(),
            whistle: Svf::default(),
            gust: SlowNoise::new(0x50_0002),
            drift: SlowNoise::new(0x50_0003),
            whistle_drift: SlowNoise::new(0x50_0004),
        }
    }

    fn block(&mut self, x: &[f32], p: &WindParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let gust = self.gust.advance(p.gust_rate, dt);
        let s = (x[0] * (1.0 + x[1] * 0.8 * gust)).clamp(0.0, 1.3);
        let drift = self.drift.advance(p.gust_rate * 1.7 + 0.05, dt);
        let fc = p.howl_hz * (1.6 * (s - 0.5) + 0.4 * drift).exp2();
        self.body[0].set(FilterMode::BandPass, fc, p.howl_res, sr);
        self.body[1].set(FilterMode::BandPass, fc * 2.3, p.howl_res * 0.8, sr);
        self.rumble.set(FilterMode::LowPass, 60.0 + 140.0 * s, 0.1, sr);
        self.hiss.set(FilterMode::HighPass, 3500.0, 0.1, sr);
        let wd = self.whistle_drift.advance(0.8, dt);
        self.whistle.set(FilterMode::BandPass, p.whistle_hz * (1.0 + 0.12 * wd) * (0.8 + 0.4 * s), 0.97, sr);
        let t = ((s - 0.45) * 3.0).clamp(0.0, 1.0);
        let whistle_gain = p.whistle_level * t * t * 0.12;
        let (howl, rumble, hiss) = (p.howl_level * 2.3, p.rumble_level * 1.4, p.hiss_level * 0.45 * s);
        let level = s.powf(1.5) * p.gain;
        for o in out.iter_mut() {
            let (w, pk) = (self.noise.white(), self.noise.pink());
            let y = (self.body[0].tick(pk) + 0.4 * self.body[1].tick(pk)) * howl
                + self.rumble.tick(self.brown.tick(w)) * rumble
                + self.hiss.tick(w) * hiss
                + self.whistle.tick(w) * whistle_gain;
            *o = y * level;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Rain
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Rain: a noise bed plus individual drops as randomly excited resonators; `shelter`
    /// moves the listener under a roof (muffled air, distinct patter overhead).
    RainParams / RainParamId {
        density: "drops/per_second" = 900.0, exp(20.0, 6000.0);
        drop_hz: "drops/hz" = 2600.0, exp(500.0, 8000.0);
        drop_res: "drops/resonance" = 0.88, lin(0.5, 0.99);
        drop_spread: "drops/spread_octaves" = 0.7, lin(0.0, 2.0);
        drops_level: "drops/level" = 0.6, UNIT;
        bed_level: "bed/level" = 0.5, UNIT;
        bed_hz: "bed/hz" = 2200.0, exp(400.0, 8000.0);
        roof_hz: "roof/hz" = 420.0, exp(100.0, 2500.0);
        roof_level: "roof/level" = 0.6, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

const PINGS: usize = 8;

pub struct Rain {
    sr: f32,
    noise: Noise,
    drops: Dust,
    patter: Dust,
    ping: [Svf; PINGS],
    next_ping: usize,
    roof: [Svf; 2],
    bed: Svf,
    air: OnePole,
}

impl Generator for Rain {
    type P = RainParams;
    const NAME: &'static str = "rain";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Rain from drizzle to downpour, in the open or under a roof.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "intensity", default: 0.5, doc: "Drizzle to downpour" },
        InputSpec { name: "shelter", default: 0.0, doc: "0 = open air, 1 = under a roof" },
    ];

    fn presets() -> Vec<(&'static str, RainParams)> {
        vec![
            ("Tin roof", RainParams { roof_hz: 900.0, roof_level: 0.9, drop_res: 0.96, ..Default::default() }),
            ("Forest drizzle", RainParams { density: 300.0, drop_hz: 1700.0, bed_level: 0.3, bed_hz: 1500.0, ..Default::default() }),
            ("Monsoon", RainParams { density: 3500.0, bed_level: 0.8, drops_level: 0.4, bed_hz: 3000.0, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Rain {
            sr,
            noise: Noise::new(0x51_0001),
            drops: Dust::new(0x51_0002),
            patter: Dust::new(0x51_0003),
            ping: [Svf::default(); PINGS],
            next_ping: 0,
            roof: [Svf::default(); 2],
            bed: Svf::default(),
            air: OnePole::default(),
        }
    }

    fn block(&mut self, x: &[f32], p: &RainParams, out: &mut [f32]) {
        let sr = self.sr;
        let (i, shelter) = (x[0], x[1]);
        self.roof[0].set(FilterMode::BandPass, p.roof_hz, 0.9, sr);
        self.roof[1].set(FilterMode::BandPass, p.roof_hz * 1.62, 0.9, sr);
        self.bed.set(FilterMode::BandPass, p.bed_hz, 0.15, sr);
        let drop_p = p.density * i * i / sr;
        let patter_p = p.density * 0.2 * i * i * shelter / sr;
        let air_coef = hz_coef(18000.0 + (1200.0 - 18000.0) * shelter, sr);
        let (drops_gain, bed_gain, roof_gain) = (p.drops_level * 2.0, p.bed_level * i.powf(1.5) * 1.6, p.roof_level * 5.0);
        for o in out.iter_mut() {
            let d = self.drops.tick(drop_p);
            let mut which = PINGS;
            if d > 0.0 {
                // Each drop rings at its own pitch; fixed pitches would sound like a chime.
                which = self.next_ping;
                self.next_ping = (self.next_ping + 1) % PINGS;
                let hz = p.drop_hz * (self.drops.rng().next_bipolar() * p.drop_spread).exp2();
                self.ping[which].set(FilterMode::BandPass, hz.min(sr * 0.4), p.drop_res, sr);
            }
            let mut pings = 0.0;
            for (k, f) in self.ping.iter_mut().enumerate() {
                pings += f.tick(if k == which { d } else { 0.0 });
            }
            let t = self.patter.tick(patter_p);
            let roof = self.roof[0].tick(t) + 0.6 * self.roof[1].tick(t);
            let open = pings * drops_gain + self.bed.tick(self.noise.pink()) * bed_gain;
            *o = (self.air.lp(open, air_coef) + roof * roof_gain) * p.gain * 1.8;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Fire
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Fire: fluttering low roar, hiss, crackles (short filtered noise bursts) and deeper pops.
    FireParams / FireParamId {
        crackle_rate: "crackle/per_second" = 35.0, exp(1.0, 400.0);
        crackle_level: "crackle/level" = 0.7, UNIT;
        crackle_hz: "crackle/hz" = 2800.0, exp(600.0, 8000.0);
        pop_level: "crackle/pops" = 0.5, UNIT;
        roar_level: "roar/level" = 0.6, UNIT;
        roar_hz: "roar/hz" = 220.0, exp(60.0, 1200.0);
        flutter: "roar/flutter" = 0.5, UNIT;
        hiss_level: "hiss/level" = 0.3, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Fire {
    sr: f32,
    noise: Noise,
    brown: Brown,
    crackles: Dust,
    pops: Dust,
    env: f32,
    crack: Svf,
    pop: Svf,
    roar: Svf,
    hiss: Svf,
    flutter: SlowNoise,
    tone: SlowNoise,
}

impl Generator for Fire {
    type P = FireParams;
    const NAME: &'static str = "fire";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Campfire to inferno: roar, hiss, crackle and pops; wind fans the flames.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "intensity", default: 0.5, doc: "Embers to blaze" },
        InputSpec { name: "wind", default: 0.0, doc: "Fans the fire: brighter, faster flutter" },
    ];

    fn presets() -> Vec<(&'static str, FireParams)> {
        vec![
            ("Campfire", FireParams { crackle_rate: 22.0, roar_level: 0.35, hiss_level: 0.2, pop_level: 0.7, ..Default::default() }),
            ("Torch", FireParams { crackle_rate: 8.0, crackle_level: 0.3, roar_hz: 380.0, flutter: 0.9, roar_level: 0.7, ..Default::default() }),
            ("Inferno", FireParams { crackle_rate: 120.0, roar_level: 1.0, roar_hz: 160.0, hiss_level: 0.6, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Fire {
            sr,
            noise: Noise::new(0x52_0001),
            brown: Brown::default(),
            crackles: Dust::new(0x52_0002),
            pops: Dust::new(0x52_0003),
            env: 0.0,
            crack: Svf::default(),
            pop: Svf::default(),
            roar: Svf::default(),
            hiss: Svf::default(),
            flutter: SlowNoise::new(0x52_0004),
            tone: SlowNoise::new(0x52_0005),
        }
    }

    fn block(&mut self, x: &[f32], p: &FireParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (i, wind) = (x[0], x[1]);
        let fl = 1.0 + p.flutter * 0.6 * self.flutter.advance(5.0 + 10.0 * wind, dt);
        self.roar.set(FilterMode::LowPass, p.roar_hz * (0.6 + 0.9 * i) * (1.0 + 0.5 * wind), 0.25, sr);
        self.hiss.set(FilterMode::HighPass, 4000.0, 0.1, sr);
        // Re-tune the crackle filter quickly so every crackle has its own colour.
        self.crack.set(FilterMode::BandPass, p.crackle_hz * (0.8 * self.tone.advance(30.0, dt)).exp2(), 0.5, sr);
        self.pop.set(FilterMode::BandPass, 260.0, 0.85, sr);
        let crackle_p = p.crackle_rate * (0.15 + 0.85 * i) / sr;
        let pop_p = crackle_p * 0.08;
        let decay = (-1.0 / (0.004 * sr)).exp();
        let (crackle_gain, pop_gain) = (p.crackle_level * 4.0, p.pop_level * 3.0);
        let (roar_gain, hiss_gain) = (p.roar_level * i * 2.5 * fl, p.hiss_level * 0.2 * i * (0.5 + 0.5 * fl));
        for o in out.iter_mut() {
            let w = self.noise.white();
            let d = self.crackles.tick(crackle_p);
            if d > 0.0 {
                self.env = self.env.max(d);
            }
            self.env *= decay;
            let y = self.crack.tick(w * self.env) * crackle_gain
                + self.pop.tick(self.pops.tick(pop_p)) * pop_gain
                + self.roar.tick(self.brown.tick(w)) * roar_gain
                + self.hiss.tick(w) * hiss_gain;
            *o = y * p.gain;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Stream (running water)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Running water as a swarm of bubbles: each is a short sine chirp that rises in pitch.
    StreamParams / StreamParamId {
        bubble_rate: "bubbles/per_second" = 340.0, exp(5.0, 1500.0);
        bubble_hz: "bubbles/hz" = 1100.0, exp(200.0, 5000.0);
        spread: "bubbles/spread_octaves" = 1.0, lin(0.0, 2.5);
        rise: "bubbles/rise" = 0.35, UNIT;
        decay_ms: "bubbles/decay_ms" = 9.0, lin(3.0, 120.0);
        bubbles_level: "bubbles/level" = 0.7, UNIT;
        wash_level: "wash/level" = 0.55, UNIT;
        wash_hz: "wash/hz" = 1800.0, exp(300.0, 8000.0);
        gain: "master/gain" = 1.0, GAIN;
    }
}

#[derive(Clone, Copy, Default)]
struct Bubble {
    phase: f32,
    inc: f32,
    rise: f32,
    env: f32,
    decay: f32,
}

const BUBBLES: usize = 16;

pub struct Stream {
    sr: f32,
    noise: Noise,
    dust: Dust,
    bubbles: [Bubble; BUBBLES],
    next: usize,
    wash: Svf,
}

impl Generator for Stream {
    type P = StreamParams;
    const NAME: &'static str = "stream";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Running water: trickle, brook or river, built from bubble chirps.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "flow", default: 0.5, doc: "Trickle to torrent" },
        InputSpec { name: "size", default: 0.3, doc: "Body of water: bigger means deeper, slower bubbles" },
    ];

    fn presets() -> Vec<(&'static str, StreamParams)> {
        vec![
            ("Dripping cave", StreamParams { bubble_rate: 9.0, bubble_hz: 1500.0, decay_ms: 60.0, wash_level: 0.05, rise: 0.8, bubbles_level: 1.0, ..Default::default() }),
            ("River", StreamParams { bubble_rate: 900.0, bubble_hz: 700.0, wash_level: 0.9, wash_hz: 1200.0, ..Default::default() }),
            ("Bubbling potion", StreamParams { bubble_rate: 40.0, bubble_hz: 420.0, spread: 0.6, rise: 0.9, decay_ms: 45.0, wash_level: 0.1, bubbles_level: 1.0, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Stream { sr, noise: Noise::new(0x53_0001), dust: Dust::new(0x53_0002), bubbles: [Bubble::default(); BUBBLES], next: 0, wash: Svf::default() }
    }

    fn block(&mut self, x: &[f32], p: &StreamParams, out: &mut [f32]) {
        let sr = self.sr;
        let (flow, size) = (x[0], x[1]);
        let bubble_p = p.bubble_rate * (0.1 + 0.9 * flow.powf(1.5)) / sr;
        let decay_samples = p.decay_ms * (1.0 + size) * 0.001 * sr;
        let decay = (-1.0 / decay_samples).exp();
        let rise = (p.rise * 1.2 / (decay_samples * 2.0)).exp2();
        let centre = p.bubble_hz * (-1.5 * size).exp2();
        self.wash.set(FilterMode::BandPass, p.wash_hz * (-size).exp2(), 0.2, sr);
        let (bubble_gain, wash_gain) = (p.bubbles_level * 0.45, p.wash_level * (0.3 + 0.7 * flow) * 2.4);
        for o in out.iter_mut() {
            let d = self.dust.tick(bubble_p);
            if d > 0.0 {
                let f = centre * (self.dust.rng().next_bipolar() * p.spread).exp2();
                self.bubbles[self.next] = Bubble { phase: 0.0, inc: f.min(sr * 0.4) / sr, rise, env: d, decay };
                self.next = (self.next + 1) % BUBBLES;
            }
            let mut y = 0.0;
            for b in self.bubbles.iter_mut() {
                if b.env > 1e-4 {
                    y += (b.phase * TAU).sin() * b.env;
                    b.phase += b.inc;
                    if b.phase >= 1.0 {
                        b.phase -= 1.0;
                    }
                    b.inc = (b.inc * b.rise).min(0.45);
                    b.env *= b.decay;
                }
            }
            *o = (y * bubble_gain + self.wash.tick(self.noise.pink()) * wash_gain) * p.gain;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Ocean
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Shoreline surf: two overlapping, slightly irregular wave cycles of swell, crash and foam.
    OceanParams / OceanParamId {
        period: "waves/period_s" = 7.0, lin(3.0, 24.0);
        wash: "waves/background_wash" = 0.3, UNIT;
        irregular: "waves/irregularity" = 0.4, UNIT;
        crash_level: "crash/level" = 0.7, UNIT;
        crash_hz: "crash/hz" = 1800.0, exp(300.0, 6000.0);
        foam_level: "foam/level" = 0.4, UNIT;
        rumble_level: "rumble/level" = 0.5, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

#[derive(Clone, Copy)]
struct Wave {
    phase: f32,
    jitter: f32,
}

pub struct Ocean {
    sr: f32,
    noise: Noise,
    brown: Brown,
    rng: Rng,
    waves: [Wave; 3],
    crash: Svf,
    foam: Svf,
    rumble: Svf,
    far: OnePole,
}

/// Swell builds slowly, breaks, then drains away.
fn wave_env(phase: f32) -> f32 {
    if phase < 0.35 {
        (phase / 0.35).powf(2.5)
    } else {
        (-(phase - 0.35) * 3.2).exp()
    }
}

impl Generator for Ocean {
    type P = OceanParams;
    const NAME: &'static str = "ocean";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Waves on a shore: swell, crash, hissing foam and low rumble.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "size", default: 0.5, doc: "Ripples to breakers: bigger waves are slower and louder" },
        InputSpec { name: "distance", default: 0.0, doc: "0 = at the waterline, 1 = far off (muffled)" },
    ];

    fn presets() -> Vec<(&'static str, OceanParams)> {
        vec![
            ("Lake shore", OceanParams { period: 4.0, wash: 0.45, crash_level: 0.4, crash_hz: 2600.0, rumble_level: 0.1, foam_level: 0.5, ..Default::default() }),
            ("Storm surf", OceanParams { period: 12.0, crash_level: 1.0, crash_hz: 1200.0, rumble_level: 0.9, irregular: 0.7, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Ocean {
            sr,
            noise: Noise::new(0x54_0001),
            brown: Brown::default(),
            rng: Rng::new(0x54_0002),
            waves: [Wave { phase: 0.15, jitter: 1.0 }, Wave { phase: 0.6, jitter: 1.0 }, Wave { phase: 0.85, jitter: 1.0 }],
            crash: Svf::default(),
            foam: Svf::default(),
            rumble: Svf::default(),
            far: OnePole::default(),
        }
    }

    fn block(&mut self, x: &[f32], p: &OceanParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (size, distance) = (x[0], x[1]);
        let period = p.period * (0.8 + 0.5 * size);
        let (mut swell, mut fizz) = (0.0, 0.0);
        for (k, w) in self.waves.iter_mut().enumerate() {
            let (scale, weight) = [(1.0, 1.0), (1.37, 0.6), (0.73, 0.45)][k];
            w.phase += dt / (period * scale * w.jitter);
            if w.phase >= 1.0 {
                w.phase -= 1.0;
                w.jitter = 1.0 + p.irregular * 0.5 * self.rng.next_bipolar();
            }
            swell += weight * wave_env(w.phase);
            fizz += weight * wave_env((w.phase - 0.08).rem_euclid(1.0)).powf(0.7);
        }
        // The sea never goes quiet between breakers.
        let swell = (p.wash + (1.0 - 0.5 * p.wash) * swell).min(1.3);
        let fizz = p.wash * 0.6 + fizz;
        self.crash.set(FilterMode::LowPass, 250.0 + p.crash_hz * swell, 0.2, sr);
        self.foam.set(FilterMode::HighPass, 2500.0, 0.1, sr);
        self.rumble.set(FilterMode::LowPass, 90.0, 0.1, sr);
        let far_coef = hz_coef(16000.0 + (1500.0 - 16000.0) * distance, sr);
        let (crash, foam, rumble) = (p.crash_level * swell * 2.8, p.foam_level * fizz.min(1.3) * 0.35, p.rumble_level * swell * 2.5);
        let level = (0.3 + 0.7 * size) * (1.0 - 0.5 * distance) * p.gain;
        for o in out.iter_mut() {
            let w = self.noise.white();
            let y = self.crash.tick(self.noise.pink()) * crash + self.foam.tick(w) * foam + self.rumble.tick(self.brown.tick(w)) * rumble;
            *o = self.far.lp(y, far_coef) * level;
        }
    }
}
