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
        howl_hz: "howl/hz" = 340.0, exp(100.0, 4000.0);
        howl_res: "howl/resonance" = 0.78, lin(0.0, 0.95);
        howl_level: "howl/level" = 0.7, UNIT;
        gust_rate: "gusts/rate_hz" = 0.25, exp(0.02, 4.0);
        rumble_level: "rumble/level" = 0.5, UNIT;
        hiss_level: "hiss/level" = 0.04, UNIT;
        whistle_hz: "whistle/hz" = 1900.0, exp(400.0, 6000.0);
        whistle_level: "whistle/level" = 0.1, UNIT;
        gain: "master/gain" = 1.2, GAIN;
        buffet: "buffet/level" = 0.5, UNIT;
        buffet_hz: "buffet/rate_hz" = 2.5, exp(0.5, 8.0);
        // The hiss is a band from hiss/hz up to hiss/top_hz (20000 leaves it open above).
        hiss_hz: "hiss/hz" = 1500.0, exp(200.0, 8000.0);
        hiss_top: "hiss/top_hz" = 6000.0, exp(500.0, 20000.0);
        // Level of the second howl resonance, 2.3 times higher.
        howl_overtone: "howl/overtone" = 0.15, UNIT;
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
    // Buffeting has its own random streams, so wind without it sounds exactly as before.
    buffet: SlowNoise,
    buffet_low: Svf,
    buffet_noise: Noise,
    hiss_top: Svf,
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
            // Tuned against snowstorm recordings (`tools/reference/snow`): loudest at 125 to 500 Hz,
            // with up to a tenth of the energy above 2.5 kHz. The howl sits low, and the storm
            // shoves at whatever is out in it (buffeting) rather than hissing. It keeps the original
            // open hiss above 3.5 kHz and the stronger howl overtone it was fitted with.
            ("Blizzard", WindParams { howl_hz: 330.0, howl_res: 0.7, howl_level: 0.8, rumble_level: 0.8, hiss_level: 0.25, whistle_level: 0.35, gust_rate: 0.5, buffet: 0.7, buffet_hz: 2.5, hiss_hz: 3500.0, hiss_top: 20000.0, howl_overtone: 0.4, gain: 1.0, ..Default::default() }),
            // The rest, and the defaults, are tuned against recordings of strong wind (Freesound CC0:
            // keirofinch 376534, lextrack 344887, FunWithSound 390740): centred at 400 to 600 Hz,
            // with 0 to 2 % of the energy above 2.5 kHz, and uneven (buffeting). The desert is the
            // broad, dry one, its loudness mostly a roar of noise at 300 to 800 Hz; the canyon rings;
            // the corridor is a steadier draught with a whistle. Gains match the old presets'
            // perceived loudness (LUFS), which their bright hiss used to supply.
            ("Desert", WindParams { howl_hz: 300.0, howl_res: 0.4, howl_level: 0.65, hiss_level: 1.0, hiss_hz: 300.0, hiss_top: 800.0, whistle_level: 0.0, rumble_level: 0.42, buffet: 0.45, gain: 1.3, ..Default::default() }),
            ("Canyon", WindParams { howl_hz: 400.0, howl_res: 0.88, howl_level: 0.8, gust_rate: 0.6, rumble_level: 0.45, hiss_level: 0.033, whistle_hz: 1500.0, whistle_level: 0.3, buffet: 0.35, howl_overtone: 0.2, gain: 1.22, ..Default::default() }),
            ("Drafty corridor", WindParams { howl_hz: 380.0, howl_res: 0.85, rumble_level: 0.2, hiss_level: 0.017, whistle_hz: 1150.0, whistle_level: 0.4, gust_rate: 0.12, buffet: 0.25, howl_overtone: 0.2, gain: 1.04, ..Default::default() }),
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
            buffet: SlowNoise::new(0x50_0005),
            buffet_low: Svf::default(),
            buffet_noise: Noise::new(0x50_0006),
            hiss_top: Svf::default(),
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
        self.hiss.set(FilterMode::HighPass, p.hiss_hz, 0.1, sr);
        // Real wind has almost nothing above 6 kHz; hiss/top_hz at its maximum leaves the hiss open.
        let hiss_top = p.hiss_top < 20000.0;
        if hiss_top {
            self.hiss_top.set(FilterMode::LowPass, p.hiss_top, 0.5, sr);
        }
        let wd = self.whistle_drift.advance(0.8, dt);
        self.whistle.set(FilterMode::BandPass, p.whistle_hz * (1.0 + 0.12 * wd) * (0.8 + 0.4 * s), 0.97, sr);
        let t = ((s - 0.45) * 3.0).clamp(0.0, 1.0);
        let whistle_gain = p.whistle_level * t * t * 0.12;
        let (howl, rumble, hiss) = (p.howl_level * 2.3, p.rumble_level * 1.4, p.hiss_level * 0.45 * s);
        // A band of hiss carries far less energy than the open hiss did, so it gets more gain.
        let hiss = if hiss_top { hiss * 3.0 } else { hiss };
        let level = s.powf(1.5) * p.gain;
        // Buffeting: the storm shoving in bursts a few times a second, felt more than heard,
        // as a swell of the whole wind and a low thump.
        let (shove, thump) = if p.buffet > 0.0 {
            let b = self.buffet.advance(p.buffet_hz * (0.7 + 0.6 * s), dt).max(0.0);
            self.buffet_low.set(FilterMode::LowPass, 140.0, 0.4, sr);
            (1.0 + p.buffet * 1.2 * b * b * s, p.buffet * b * b * s * 3.0)
        } else {
            (1.0, 0.0)
        };
        for o in out.iter_mut() {
            let (w, pk) = (self.noise.white(), self.noise.pink());
            let y = (self.body[0].tick(pk) + p.howl_overtone * self.body[1].tick(pk)) * howl
                + self.rumble.tick(self.brown.tick(w)) * rumble
                + (if hiss_top { self.hiss_top.tick(self.hiss.tick(w)) } else { self.hiss.tick(w) }) * hiss
                + self.whistle.tick(w) * whistle_gain;
            *o = y * level;
            if p.buffet > 0.0 {
                *o = *o * shove + self.buffet_low.tick(self.buffet_noise.pink()) * thump * level;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Rain
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Rain: individual drops as randomly excited resonators with a power-law spread of sizes,
    /// a low splat as the bigger drops hit the ground, and a broad wash of rain further off;
    /// `shelter` moves the listener under a roof (muffled air, distinct patter overhead).
    RainParams / RainParamId {
        density: "drops/per_second" = 1500.0, exp(20.0, 6000.0);
        drop_hz: "drops/hz" = 1400.0, exp(500.0, 8000.0);
        drop_res: "drops/resonance" = 0.7, lin(0.5, 0.99);
        drop_spread: "drops/spread_octaves" = 1.5, lin(0.0, 2.0);
        drops_level: "drops/level" = 0.6, UNIT;
        bed_level: "bed/level" = 0.45, UNIT;
        bed_hz: "bed/hz" = 800.0, exp(400.0, 8000.0);
        roof_hz: "roof/hz" = 420.0, exp(100.0, 2500.0);
        roof_level: "roof/level" = 0.6, UNIT;
        gain: "master/gain" = 1.0, GAIN;
        size_spread: "drops/size_spread" = 0.4, UNIT;
        splat_level: "splat/level" = 0.5, UNIT;
        splat_hz: "splat/hz" = 320.0, exp(80.0, 1000.0);
        wander: "drops/wander" = 0.4, UNIT;
        roof_rate: "roof/per_second" = 180.0, exp(10.0, 2000.0);
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
    splat: Svf,
    swell: SlowNoise,
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
            // Tuned against CC0 recordings (Freesound): Default between rain in a garden and heavy
            // rural rain, Tin roof against hard rain on a patio (bright, ringing, rising to
            // 8 kHz), Forest drizzle against rain on grass, leaves and ferns, and Monsoon against
            // the heavy rural rain at full intensity.
            ("Tin roof", RainParams {
                roof_hz: 900.0, roof_level: 0.9, density: 3000.0, drop_hz: 8000.0, drop_spread: 1.4, drop_res: 0.94, drops_level: 0.16,
                bed_level: 0.58, bed_hz: 3000.0, splat_level: 0.225, splat_hz: 180.0, size_spread: 0.45, wander: 0.25, roof_rate: 180.0, ..Default::default()
            }),
            ("Forest drizzle", RainParams {
                drop_hz: 3000.0, drop_spread: 1.8, drop_res: 0.6, drops_level: 0.24, bed_level: 0.195, bed_hz: 2000.0, splat_level: 0.105, splat_hz: 400.0,
                size_spread: 0.4, wander: 0.15, roof_rate: 60.0, ..Default::default()
            }),
            ("Monsoon", RainParams {
                density: 3500.0, drop_hz: 1600.0, drops_level: 0.52, bed_level: 0.73, splat_level: 0.62, splat_hz: 280.0, size_spread: 0.55,
                wander: 0.6, roof_rate: 700.0, ..Default::default()
            }),
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
            splat: Svf::default(),
            swell: SlowNoise::new(0x51_0004),
        }
    }

    fn block(&mut self, x: &[f32], p: &RainParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (i, shelter) = (x[0], x[1]);
        self.roof[0].set(FilterMode::BandPass, p.roof_hz, 0.9, sr);
        self.roof[1].set(FilterMode::BandPass, p.roof_hz * 1.62, 0.9, sr);
        self.bed.set(FilterMode::BandPass, p.bed_hz, 0.15, sr);
        self.splat.set(FilterMode::BandPass, p.splat_hz, 0.6, sr);
        // Real rain comes and goes in slow waves of a few dB.
        let swell = p.wander * self.swell.advance(0.15, dt);
        // Heavier rain is more drops and bigger ones, so the big drops still stand out of a
        // downpour instead of melting into a smooth hiss.
        let drop_p = p.density * i.powf(1.5) * (swell * 1.2).exp2() / sr;
        let swell_gain = (swell * 0.6).exp2() * i.powf(0.25);
        let patter_p = p.roof_rate * i * i * shelter / sr;
        let air_coef = hz_coef(18000.0 + (1200.0 - 18000.0) * shelter, sr);
        // Walls muffle the rain outside and also take some of it away: its body below 1 kHz
        // passes the muffling.
        let walls = 1.0 - 0.4 * shelter;
        // Drops land evenly around the listener, out to 2^(8 * size_spread) times the distance
        // of the nearest, and each is as loud as 1/distance: a power law with many faint ticks
        // and a few close drops far above the rest (real rain has a sample crest of 23-34 dB).
        // The gain keeps the mean drop energy the same whatever the spread.
        let reach2 = (16.0 * p.size_spread).exp2();
        let mean_sq = if reach2 > 1.001 { reach2.ln() / (reach2 - 1.0) } else { 1.0 };
        let size_norm = (0.57 / mean_sq).sqrt();
        let drops_gain = p.drops_level * 4.13 * size_norm;
        let bed_gain = p.bed_level * i.powf(1.5) * 3.32 * (swell * 1.2).exp2();
        let (splat_gain, roof_gain) = (p.splat_level * 8.27 * size_norm, p.roof_level * 5.0);
        for o in out.iter_mut() {
            let mut which = PINGS;
            let (mut d, mut thump) = (0.0, 0.0);
            if self.drops.tick(drop_p) > 0.0 {
                // Each drop rings at its own pitch; fixed pitches would sound like a chime.
                which = self.next_ping;
                self.next_ping = (self.next_ping + 1) % PINGS;
                let rng = self.drops.rng();
                let near = (1.0 + rng.next_f32() * (reach2 - 1.0)).sqrt().recip();
                let size = near * (0.5 + 0.5 * rng.next_f32()) * swell_gain;
                let octave = rng.next_bipolar() * p.drop_spread;
                let hz = p.drop_hz * octave.exp2();
                // A resonator's energy grows with its bandwidth, so high drops are scaled down
                // to give every octave the same share; otherwise the spread tilts the rain bright.
                d = size * (-0.5 * octave).exp2();
                self.ping[which].set(FilterMode::BandPass, hz.min(sr * 0.4), p.drop_res, sr);
                // The same drop hitting the ground: a short, dull thump.
                thump = size;
            }
            let mut pings = 0.0;
            for (k, f) in self.ping.iter_mut().enumerate() {
                pings += f.tick(if k == which { d } else { 0.0 });
            }
            let splat = self.splat.tick(thump) * splat_gain;
            let t = self.patter.tick(patter_p);
            let roof = self.roof[0].tick(t) + 0.6 * self.roof[1].tick(t);
            let open = pings * drops_gain + splat + self.bed.tick(self.noise.pink()) * bed_gain;
            *o = (self.air.lp(open, air_coef) * walls + roof * roof_gain) * p.gain * 1.8;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Fire
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Fire: fluttering low roar, a fizz of tiny ticks, crackles (short broadband noise bursts,
    /// mostly small, a few loud), rare bangs and deeper pops.
    FireParams / FireParamId {
        crackle_rate: "crackle/per_second" = 40.0, exp(1.0, 400.0);
        crackle_level: "crackle/level" = 0.7, UNIT;
        crackle_hz: "crackle/hz" = 2000.0, exp(600.0, 8000.0);
        crackle_ms: "crackle/ms" = 1.5, lin(0.5, 10.0);
        pop_level: "crackle/pops" = 0.5, UNIT;
        roar_level: "roar/level" = 0.6, UNIT;
        roar_hz: "roar/hz" = 220.0, exp(60.0, 1200.0);
        flutter: "roar/flutter" = 0.5, UNIT;
        // The bed between crackles: a fizz of tiny ticks across 1 to 6 kHz (it was a steady hiss).
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
    crack_lp: OnePole,
    pop: Svf,
    roar: Svf,
    bangs: Dust,
    bang_env: f32,
    bang: Svf,
    fizz_dust: Dust,
    fizz_env: f32,
    fizz: Svf,
    fizz_lp: OnePole,
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
            ("Campfire", FireParams { crackle_rate: 30.0, roar_level: 0.35, hiss_level: 0.2, pop_level: 0.7, ..Default::default() }),
            ("Torch", FireParams { crackle_rate: 8.0, crackle_level: 0.3, roar_hz: 380.0, flutter: 0.9, roar_level: 0.7, gain: 1.25, ..Default::default() }),
            ("Inferno", FireParams { crackle_rate: 120.0, crackle_level: 0.8, roar_level: 1.0, roar_hz: 160.0, hiss_level: 0.45, ..Default::default() }),
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
            crack_lp: OnePole::default(),
            pop: Svf::default(),
            roar: Svf::default(),
            bangs: Dust::new(0x52_0006),
            bang_env: 0.0,
            bang: Svf::default(),
            fizz_dust: Dust::new(0x52_0007),
            fizz_env: 0.0,
            fizz: Svf::default(),
            fizz_lp: OnePole::default(),
            flutter: SlowNoise::new(0x52_0004),
            tone: SlowNoise::new(0x52_0005),
        }
    }

    // Tuned against close campfire recordings (`target/refs`, the same changes as
    // models/campfire.toml): real crackles are short (they fall 12 dB in about 2 ms), broadband
    // from 1 to 8 kHz, and mostly small with a long tail of loud ones; between them is a fizz of
    // tiny ticks, and the roar stays under the crackles even when the fire is big.
    fn block(&mut self, x: &[f32], p: &FireParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (i, wind) = (x[0], x[1]);
        let fl = 1.0 + p.flutter * 0.6 * self.flutter.advance(5.0 + 10.0 * wind, dt);
        self.roar.set(FilterMode::LowPass, p.roar_hz * (0.6 + 0.9 * i) * (1.0 + 0.5 * wind), 0.25, sr);
        // Re-tune the crackle filter quickly so every crackle has its own colour. A wide band
        // and a 10 kHz top keep each crackle broadband.
        self.crack.set(FilterMode::BandPass, p.crackle_hz * (0.8 * self.tone.advance(30.0, dt)).exp2(), 0.1, sr);
        self.pop.set(FilterMode::BandPass, 260.0, 0.85, sr);
        self.bang.set(FilterMode::BandPass, 1500.0, 0.05, sr);
        self.fizz.set(FilterMode::HighPass, 1000.0, 0.1, sr);
        let (crack_lp, fizz_lp) = (hz_coef(10000.0, sr), hz_coef(6000.0, sr));
        let crackle_p = p.crackle_rate * (0.15 + 0.85 * i) / sr;
        let (pop_p, bang_p) = (crackle_p * 0.08, crackle_p * 0.05);
        let fizz_p = 250.0 * (0.3 + 0.7 * i) / sr;
        let ms = p.crackle_ms.max(0.1) * 0.001 * sr;
        let (decay, bang_decay, fizz_decay) = ((-1.0 / ms).exp(), (-1.0 / (1.3 * ms)).exp(), (-1.0 / (0.0007 * sr)).exp());
        // The model file's mix, raised to the level the games already mix fire at.
        const LEVEL: f32 = 3.4;
        let (crackle_gain, bang_gain) = (p.crackle_level * 10.0 * LEVEL, p.crackle_level * 9.0 * LEVEL);
        let pop_gain = p.pop_level * 3.0 * LEVEL;
        // The roar grows only a little with intensity: a bigger fire mostly crackles more.
        let roar_gain = p.roar_level * (0.5 + 0.5 * i) * 0.5 * LEVEL * fl;
        let fizz_gain = p.hiss_level * 0.83 * (0.4 + 0.6 * i) * LEVEL;
        for o in out.iter_mut() {
            let w = self.noise.white();
            let d = self.crackles.tick(crackle_p);
            if d > 0.0 {
                // Power-law sizes: most crackles are small, a few are loud.
                let u = self.crackles.rng().next_f32();
                self.env = self.env.max(d * u * u * u);
            }
            self.env *= decay;
            self.bang_env = (self.bang_env * bang_decay).max(self.bangs.tick(bang_p));
            self.fizz_env = (self.fizz_env * fizz_decay).max(self.fizz_dust.tick(fizz_p));
            let crack = self.crack_lp.lp(self.crack.tick(w * self.env), crack_lp);
            let fizz = self.fizz_lp.lp(self.fizz.tick(w * self.fizz_env), fizz_lp);
            let y = crack * crackle_gain
                + self.bang.tick(w * self.bang_env) * bang_gain
                + fizz * fizz_gain
                + self.pop.tick(self.pops.tick(pop_p)) * pop_gain
                + self.roar.tick(self.brown.tick(w)) * roar_gain;
            *o = y * p.gain;
        }
    }
}

#[cfg(test)]
mod fire_tests {
    use crate::generators;

    const SR: f32 = 48000.0;

    /// Default preset at `intensity` with some params overridden, the first half second skipped.
    fn render(intensity: f32, secs: f32, params: &[(&str, f32)]) -> Vec<f32> {
        let mut m = generators::create("fire", SR).unwrap();
        for (name, v) in params {
            assert!(m.set_param_by_name(name, *v), "{name}");
        }
        m.set_input(0, intensity);
        m.snap();
        let mut out = vec![0.0; (secs * SR) as usize];
        m.render_mono(&mut out);
        out.split_off(SR as usize / 2)
    }

    #[test]
    fn fire_bed_is_a_fizz_of_ticks_not_a_steady_hiss() {
        // Between crackles real fire has a floor of tiny ticks. Alone, the bed's 1 ms envelope
        // must be uneven (a steady hiss stays within a few dB of its median).
        let x = render(0.5, 4.0, &[("crackle/level", 0.0), ("crackle/pops", 0.0), ("roar/level", 0.0)]);
        let mut e: Vec<f32> = x.chunks(48).map(crate::render::rms).collect();
        e.sort_by(f32::total_cmp);
        let spread = 20.0 * (e[e.len() * 95 / 100] / e[e.len() / 2]).log10();
        assert!(e[e.len() / 2] > 0.0 && spread > 8.0, "fizz p95 over median {spread:.1} dB");
    }

    #[test]
    fn fire_roar_grows_little_with_intensity() {
        // A bigger fire mostly crackles more; the low roar must not bury the crackles.
        let low = |i: f32| {
            let x = render(i, 6.0, &[]);
            let (mut f, c) = (crate::blocks::OnePole::default(), crate::blocks::hz_coef(200.0, SR));
            let y: Vec<f32> = x.iter().map(|s| f.lp(*s, c)).collect();
            crate::render::rms(&y)
        };
        let ratio = low(1.0) / low(0.2);
        assert!(ratio < 2.0, "roar grows {ratio:.2}x from intensity 0.2 to 1");
    }
}

// ---------------------------------------------------------------------------------------------
// Stream (running water)
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Running water: a dense swarm of bubbles over a churning wash of noise, with sharp
    /// splash ticks on top. Fitted to six close stream recordings (`target/refs/stream`): a
    /// real stream is a granular hiss of countless short pings (median 12 to 15 ms, pitch
    /// nearly steady) spread over 300 Hz to 5 kHz, with bigger bubbles lower, louder
    /// and longer, and 7 to 35 broadband ticks a second that last about a millisecond.
    StreamParams / StreamParamId {
        bubble_rate: "bubbles/per_second" = 900.0, exp(5.0, 1500.0);
        bubble_hz: "bubbles/hz" = 1100.0, exp(200.0, 5000.0);
        spread: "bubbles/spread_octaves" = 2.0, lin(0.0, 2.5);
        rise: "bubbles/rise" = 0.02, UNIT;
        decay_ms: "bubbles/decay_ms" = 4.0, lin(3.0, 120.0);
        bubbles_level: "bubbles/level" = 0.48, UNIT;
        wash_level: "wash/level" = 0.36, UNIT;
        wash_hz: "wash/hz" = 6000.0, exp(300.0, 8000.0);
        gain: "master/gain" = 1.0, GAIN;
        accent: "bubbles/accent" = 0.6, UNIT;
        churn: "wash/churn" = 0.6, UNIT;
        splash_level: "splash/level" = 0.48, UNIT;
        splash_rate: "splash/per_second" = 20.0, exp(1.0, 300.0);
        splash_hz: "splash/hz" = 3000.0, exp(1000.0, 12000.0);
        size_law: "bubbles/size_law" = 1.0, UNIT;
    }
}

#[derive(Clone, Copy, Default)]
struct Bubble {
    phase: f32,
    inc: f32,
    rise: f32,
    env: f32,
    decay: f32,
    /// Onset: the share of the envelope still to come, falling to 0 (0 = an instant start).
    att: f32,
    att_decay: f32,
}

/// Voices for the old fixed-size bubbles (Dripping cave, Bubbling potion), and for the dense
/// swarm of size-law bubbles.
const BUBBLES: usize = 16;
const SWARM: usize = 64;
const SKEW: f32 = 1.6;
const ATTACK_CYCLES: f32 = 1.0;

/// Mean of `d^n` for the 0.3..1 amplitudes [`Dust`] hands out.
fn dust_moment(n: f32) -> f32 {
    (1.0 - 0.3f32.powf(n + 1.0)) / (0.7 * (n + 1.0))
}

pub struct Stream {
    sr: f32,
    noise: Noise,
    dust: Dust,
    bubbles: [Bubble; SWARM],
    next: usize,
    wash: Svf,
    // Churn and splashes have their own random streams, so with them, accent and the size law
    // off, the bubbles and wash are exactly as before (Dripping cave and Bubbling potion render
    // bit-identical).
    churn: SlowNoise,
    swell: SlowNoise,
    tick_noise: Noise,
    tick_dust: Dust,
    tick_env: f32,
    tick_lp: OnePole,
    tick_hp: OnePole,
}

impl Generator for Stream {
    type P = StreamParams;
    const NAME: &'static str = "stream";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Running water: trickle, brook or river, built from a swarm of bubbles, a churning wash and splash ticks.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "flow", default: 0.5, doc: "Trickle to torrent" },
        InputSpec { name: "size", default: 0.3, doc: "Body of water: bigger means deeper, slower bubbles" },
    ];

    fn presets() -> Vec<(&'static str, StreamParams)> {
        let old = StreamParams { accent: 0.0, churn: 0.0, splash_level: 0.0, size_law: 0.0, ..Default::default() };
        vec![
            // Not streams, so not fitted to the stream recordings: they keep their old sound.
            ("Dripping cave", StreamParams { bubble_rate: 9.0, bubble_hz: 1500.0, spread: 1.0, decay_ms: 60.0, wash_level: 0.025, wash_hz: 1800.0, rise: 0.8, bubbles_level: 0.5, ..old }),
            // Bigger and lower than the default, close to a broad, full stream (OneTwo_BER's):
            // loudest at 500 Hz to 2 kHz, a tenth of the energy above 2.5 kHz.
            ("River", StreamParams { bubble_rate: 900.0, bubble_hz: 850.0, spread: 1.7, accent: 0.9, bubbles_level: 0.75, wash_level: 0.57, wash_hz: 3500.0, splash_level: 0.94, splash_rate: 15.0, splash_hz: 3000.0, ..Default::default() }),
            ("Bubbling potion", StreamParams { bubble_rate: 40.0, bubble_hz: 420.0, spread: 0.6, rise: 0.9, decay_ms: 45.0, wash_level: 0.05, wash_hz: 1800.0, bubbles_level: 0.5, ..old }),
        ]
    }

    fn new(sr: f32) -> Self {
        Stream {
            sr,
            noise: Noise::new(0x53_0001),
            dust: Dust::new(0x53_0002),
            bubbles: [Bubble::default(); SWARM],
            next: 0,
            wash: Svf::default(),
            churn: SlowNoise::new(0x53_0003),
            swell: SlowNoise::new(0x53_0004),
            tick_noise: Noise::new(0x53_0005),
            tick_dust: Dust::new(0x53_0006),
            tick_env: 0.0,
            tick_lp: OnePole::default(),
            tick_hp: OnePole::default(),
        }
    }

    fn block(&mut self, x: &[f32], p: &StreamParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, 1.0 / self.sr);
        let (flow, size) = (x[0], x[1]);
        let bubble_p = p.bubble_rate * (0.1 + 0.9 * flow.powf(1.5)) / sr;
        let decay_samples = p.decay_ms * (1.0 + size) * 0.001 * sr;
        let decay = (-1.0 / decay_samples).exp();
        let rise = (p.rise * 1.2 / (decay_samples * 2.0)).exp2();
        let centre = p.bubble_hz * (-1.5 * size).exp2();
        self.wash.set(FilterMode::BandPass, p.wash_hz * (-size).exp2(), 0.2, sr);
        // Accent: a few loud bubbles among many quiet ones (amplitude raised to a power), at the
        // same mean energy.
        let accent_pow = 1.0 + 4.0 * p.accent;
        let accent_norm = (dust_moment(2.0) / dust_moment(2.0 * accent_pow)).sqrt();
        let (bubble_gain, wash_gain) = (p.bubbles_level * 0.9 * accent_norm, p.wash_level * (0.3 + 0.7 * flow) * 4.8);
        // Size law: a bubble's pitch is set by its size (Minnaert), and a big bubble rings
        // longer and louder than a small one, and small bubbles far outnumber big ones.
        // At 0 every bubble has the same decay and loudness, as before.
        let law = p.size_law;
        let voices = if law > 0.0 { SWARM } else { BUBBLES };
        let tick_p = p.splash_rate * (0.15 + 0.85 * flow.powf(1.5)) / sr;
        let tick_gain = p.splash_level * (0.4 + 0.6 * flow) * 20.0;
        let (tick_decay, tick_lp, tick_hp) = ((-1.0 / (0.0005 * sr)).exp(), hz_coef(p.splash_hz, sr), hz_coef(250.0, sr));
        let (churn_hz, swell_hz) = (8.0 + 10.0 * flow, 0.8 + 0.8 * flow);
        for o in out.iter_mut() {
            let d = self.dust.tick(bubble_p);
            if d > 0.0 {
                let r = self.dust.rng().next_bipolar();
                let f = if law > 0.0 {
                    // Many more small bubbles than big ones (a power law in size): per octave the
                    // count grows 2^skew times towards the top.
                    let skew = SKEW * law;
                    let span = (2.0 * skew * p.spread).exp2() - 1.0;
                    centre * ((1.0 + 0.5 * (r + 1.0) * span).log2() / skew - p.spread).exp2()
                } else {
                    centre * (r * p.spread).exp2()
                };
                let mut bubble = Bubble { phase: 0.0, inc: f.min(sr * 0.4) / sr, rise, env: d.powf(accent_pow), decay, att: 0.0, att_decay: 0.0 };
                if law > 0.0 {
                    let big = centre / f;
                    let ds = decay_samples * big.powf(0.6 * law);
                    bubble.decay = (-1.0 / ds).exp();
                    bubble.rise = (p.rise * 1.2 / (ds * 2.0)).exp2();
                    bubble.env *= big.powf(0.3 * law);
                    // A bubble swells into its note over about a cycle rather than clicking on.
                    bubble.att = 1.0;
                    bubble.att_decay = (-bubble.inc / ATTACK_CYCLES).exp();
                }
                self.bubbles[self.next] = bubble;
                self.next = (self.next + 1) % voices;
            }
            let mut y = 0.0;
            for b in self.bubbles[..voices].iter_mut() {
                if b.env > 1e-4 {
                    y += (b.phase * TAU).sin() * b.env * (1.0 - b.att);
                    b.att *= b.att_decay;
                    b.phase += b.inc;
                    if b.phase >= 1.0 {
                        b.phase -= 1.0;
                    }
                    b.inc = (b.inc * b.rise).min(0.45);
                    b.env *= b.decay;
                }
            }
            let mut wash = self.wash.tick(self.noise.pink()) * wash_gain;
            if p.churn > 0.0 || p.splash_level > 0.0 {
                // Churn: the water slops about, so the wash and the splashes come and go.
                let m = 1.3 * self.churn.advance(churn_hz, dt) + 0.25 * self.swell.advance(swell_hz, dt);
                let lump = (p.churn * m).exp2();
                wash *= lump;
                // Splash ticks: a drop or a lip of water slapping, a broadband click about a
                // millisecond long, mostly quiet with the odd loud one.
                let s = self.tick_dust.tick(tick_p * lump);
                if s > 0.0 {
                    self.tick_env += s * s * s;
                }
                if self.tick_env > 1e-5 {
                    let n = self.tick_lp.lp(self.tick_noise.white(), tick_lp);
                    wash += self.tick_hp.hp(n, tick_hp) * self.tick_env * tick_gain;
                    self.tick_env *= tick_decay;
                }
            }
            *o = (y * bubble_gain + wash) * p.gain;
        }
    }
}

#[cfg(test)]
mod stream_tests {
    use crate::filter::{FilterMode, Svf};
    use crate::generators;
    use crate::render::rms;

    /// Share of energy above about 2.5 kHz, and the crest of the 10 ms envelope (dB, p99/median).
    fn measure(preset: &str, flow: f32) -> (f32, f32) {
        let sr = 48000.0;
        let mut m = generators::create("stream", sr).unwrap();
        let i = m.desc().preset_index(preset).unwrap();
        m.load_preset(i);
        m.set_input(0, flow);
        m.snap();
        let mut buf = vec![0.0; 48000 * 6];
        m.render_mono(&mut buf);
        let buf = &buf[48000..];
        let (mut hp, mut hp2) = (Svf::default(), Svf::default());
        hp.set(FilterMode::HighPass, 2500.0, 0.3, sr);
        hp2.set(FilterMode::HighPass, 2500.0, 0.3, sr);
        let high: Vec<f32> = buf.iter().map(|&x| hp2.tick(hp.tick(x))).collect();
        let share = (rms(&high) / rms(buf)).powi(2);
        let mut env: Vec<f32> = buf.chunks(480).map(rms).collect();
        env.sort_by(|a, b| a.total_cmp(b));
        let crest = 20.0 * (env[env.len() * 99 / 100] / env[env.len() / 2]).log10();
        (share, crest)
    }

    #[test]
    fn streams_are_bright_and_lumpy_like_recordings() {
        // Real streams have 9 to 67 % of their energy above 2.5 kHz and a 10 ms crest of 5 to
        // 11 dB; before the retune ours had 1 to 2 % and 4 to 5.5 dB.
        for (preset, min_share, min_crest) in [("Default", 0.10, 5.0), ("River", 0.04, 4.5)] {
            let (share, crest) = measure(preset, 0.5);
            assert!(share > min_share, "{preset}: only {:.1} % above 2.5 kHz", share * 100.0);
            assert!(crest > min_crest, "{preset}: 10 ms crest {crest:.1} dB is too smooth");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Ocean
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Shoreline surf: overlapping, slightly irregular breakers. Each one breaks in about a second,
    /// washes up the beach, then drains back through the sand or pebbles with a fizzing hiss.
    OceanParams / OceanParamId {
        period: "waves/period_s" = 7.0, lin(3.0, 24.0);
        wash: "waves/background_wash" = 0.3, UNIT;
        irregular: "waves/irregularity" = 0.4, UNIT;
        crash_level: "crash/level" = 0.7, UNIT;
        crash_hz: "crash/hz" = 1800.0, exp(300.0, 6000.0);
        foam_level: "foam/level" = 0.25, UNIT;
        rumble_level: "rumble/level" = 0.15, UNIT;
        gain: "master/gain" = 1.0, GAIN;
        crash_low_cut: "crash/low_cut_hz" = 200.0, exp(20.0, 800.0);
        foam_hz: "foam/hz" = 7000.0, exp(2000.0, 16000.0);
        backwash: "backwash/level" = 0.4, UNIT;
        pebbles: "backwash/pebbles" = 0.2, UNIT;
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
    crash_low: Svf,
    foam: Svf,
    foam_top: Svf,
    rumble: Svf,
    far: OnePole,
    drain_noise: Noise,
    drain: Svf,
    grains: Dust,
    grain: f32,
}

/// One breaker `t` seconds after it starts to break; `k` stretches time (bigger waves are slower).
/// Returns the crash, the foam wash and the backwash draining out. Fitted to the average wave of
/// close beach recordings: about a second from the break to the peak, a short plateau, then a
/// slow fall of 6 dB in about 3 s.
fn breaker(t: f32, k: f32, decay: f32) -> (f32, f32, f32) {
    let env = |a: f32, h: f32, d: f32| {
        if t < a {
            // The break hits hard and then eases into the peak.
            let x = 1.0 - t / a;
            1.0 - x * x
        } else if t < a + h {
            1.0
        } else {
            (-(t - a - h) / d).exp()
        }
    };
    let (attack, hold) = (0.7 * k, 0.3 * k);
    let u = (t - attack - hold) / (0.5 * decay);
    let drain = if u > 0.0 { u * (1.0 - u).exp() } else { 0.0 };
    (env(attack, hold, decay), env(attack, hold + 0.4 * k, decay * 1.1), drain)
}

impl Generator for Ocean {
    type P = OceanParams;
    const NAME: &'static str = "ocean";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Waves on a shore: crash, wash, the hiss of the backwash and a low rumble.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "size", default: 0.5, doc: "Ripples to breakers: bigger waves are slower and louder" },
        InputSpec { name: "distance", default: 0.0, doc: "0 = at the waterline, 1 = far off (muffled)" },
    ];

    fn presets() -> Vec<(&'static str, OceanParams)> {
        vec![
            ("Lake shore", OceanParams { period: 4.0, wash: 0.45, crash_level: 0.4, crash_hz: 2600.0, rumble_level: 0.03, foam_level: 0.2, foam_hz: 6000.0, crash_low_cut: 220.0, backwash: 0.5, pebbles: 0.5, ..Default::default() }),
            ("Storm surf", OceanParams { period: 12.0, wash: 0.45, crash_level: 1.0, crash_hz: 550.0, rumble_level: 0.35, irregular: 0.7, foam_level: 0.15, crash_low_cut: 90.0, foam_hz: 4000.0, backwash: 0.2, gain: 1.2, ..Default::default() }),
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
            crash_low: Svf::default(),
            foam: Svf::default(),
            foam_top: Svf::default(),
            rumble: Svf::default(),
            far: OnePole::default(),
            drain_noise: Noise::new(0x54_0003),
            drain: Svf::default(),
            grains: Dust::new(0x54_0004),
            grain: 0.0,
        }
    }

    fn block(&mut self, x: &[f32], p: &OceanParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (size, distance) = (x[0], x[1]);
        let period = p.period * (0.8 + 0.5 * size);
        let (mut swell, mut fizz, mut drain) = (0.0, 0.0, 0.0);
        for (k, w) in self.waves.iter_mut().enumerate() {
            let (scale, weight) = [(1.0, 1.0), (1.37, 0.6), (0.73, 0.45)][k];
            let len = period * scale * w.jitter;
            w.phase += dt / len;
            if w.phase >= 1.0 {
                w.phase -= 1.0;
                w.jitter = 1.0 + p.irregular * 0.5 * self.rng.next_bipolar();
            }
            // This breaker plus the tail of the one before it, so the level never jumps.
            let stretch = (len / 7.0).sqrt().clamp(0.6, 1.6);
            let decay = 0.3 * len;
            let (c, f, d) = breaker(w.phase * len, stretch, decay);
            let (c0, f0, d0) = breaker((w.phase + 1.0) * len, stretch, decay);
            swell += weight * (c + c0);
            fizz += weight * (f + f0);
            drain += weight * (d + d0);
        }
        // The sea never goes quiet between breakers.
        let swell = (p.wash + (1.0 - 0.5 * p.wash) * swell).min(1.3);
        let fizz = (p.wash * 0.6 + fizz).min(1.3);
        let drain = (p.wash * 0.2 + drain).min(1.3);
        self.crash.set(FilterMode::LowPass, 250.0 + p.crash_hz * (0.4 + 0.6 * swell), 0.2, sr);
        self.crash_low.set(FilterMode::HighPass, p.crash_low_cut, 0.1, sr);
        self.foam.set(FilterMode::HighPass, 1800.0, 0.1, sr);
        self.foam_top.set(FilterMode::LowPass, p.foam_hz, 0.1, sr);
        self.rumble.set(FilterMode::LowPass, 90.0, 0.1, sr);
        // The backwash: sand fizzes in a dense spray of tiny grains, pebbles clatter.
        self.drain.set(FilterMode::BandPass, 3200.0 - 1400.0 * p.pebbles, 0.25, sr);
        let grain_rate = (1500.0 - 1380.0 * p.pebbles) * (0.4 + 0.6 * drain.min(1.0));
        let grain_secs = 0.0015 + 0.0035 * p.pebbles;
        let grain_decay = (-1.0 / (grain_secs * sr)).exp();
        let grain_norm = 1.0 / (grain_rate * 0.463 * grain_secs * 0.5).max(1e-3).sqrt();
        let far_coef = hz_coef(16000.0 + (1500.0 - 16000.0) * distance, sr);
        let crash = p.crash_level * swell.powf(1.2) * 2.8;
        let foam = p.foam_level * fizz * 0.35;
        let rumble = p.rumble_level * swell * 2.5;
        let backwash = p.backwash * drain * 0.3 * grain_norm;
        // The games were mixed against the old ocean, whose level was mostly sub-bass, so this
        // matches its loudness as heard (LUFS, within 0.5 dB per preset), not its RMS: the RMS
        // drops about 2 dB because what it gave up was the least audible part.
        let level = 1.35 * (0.3 + 0.7 * size) * (1.0 - 0.5 * distance) * p.gain;
        let grain_p = grain_rate / sr;
        for o in out.iter_mut() {
            let w = self.noise.white();
            let body = self.crash.tick(self.crash_low.tick(self.noise.pink())) * crash;
            let hiss = self.foam_top.tick(self.foam.tick(w)) * foam;
            self.grain = self.grain * grain_decay + self.grains.tick(grain_p);
            let sand = self.drain.tick(self.drain_noise.white()) * self.grain * backwash;
            let y = body + hiss + sand + self.rumble.tick(self.brown.tick(w)) * rumble;
            *o = self.far.lp(y, far_coef) * level;
        }
    }
}
