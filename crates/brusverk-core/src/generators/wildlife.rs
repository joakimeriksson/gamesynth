//! Living nature: birds, insects, frogs, leaves in the wind, and thunder.
//!
//! Every sound here was measured against CC0 field recordings (Freesound, kept in
//! `target/refs/wildlife` with their sources) before it was built:
//!
//! * **Birds** are real species: `birds` draws its singers from the `bird` generator's songbook
//!   (`generators/birdsong`), songs transcribed syllable by syllable from CC0 recordings of each
//!   species (blackbird, robin, chaffinch, great tit, cuckoo, doves, crow, sparrow, skylark,
//!   gull, woodpecker; tawny owl and nightingale at night). In a forest every note leaves a
//!   reverberant tail.
//! * **Crickets** chirp at a 4.2-4.3 kHz carrier; each chirp is 3-5 pulses at 20-30 per second,
//!   and the chirp rate follows the temperature (Dolbear's law). A field of them drifts in and
//!   out of step. **Cicadas** are a buzz of tymbal clicks through a 5-6 kHz resonance that swells
//!   for several seconds and dies away with a falling pitch.
//! * **Frogs** are pulse trains (about 100 per second) through a resonant throat: formants near
//!   1 and 2.2 kHz for tree frogs (two-part "rib-bit" calls, about 2 per second, in bouts), 250 Hz
//!   and 1.1 kHz for bullfrogs, 3.2 kHz for the comb-like trill of chorus frogs.
//! * **Leaves** are a dense crackle of tiny contacts over a sheet of foliage noise, centred near
//!   1-2 kHz, rising and falling with the gusts by 9-13 dB.
//! * **Thunder** close by opens with a crack (broadband to 10 kHz, from silence to full level in a
//!   few milliseconds, then 1-1.5 s of tearing), then rolls for 10-30 s below 1 kHz. Far thunder
//!   has no crack: it swells over a second or more and rolls below 500 Hz.
//!
//! The ambiences are stereo: each bird, cricket and frog has its own place and distance; width 0
//! is the mono render, sample for sample.

use crate::blocks::{hz_coef, mix_seed, Brown, Dust, OnePole, Reverb, SlowNoise, BLOCK};
use crate::generators::birdsong::{Control, Singer};
use crate::dsp::balance;
use crate::filter::{FilterMode, Svf};
use crate::math::{soft_clip, Rng, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::params::{exp, int, lin, GAIN, UNIT};

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

#[inline]
fn smooth01(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Piecewise-linear lookup in a table of (x, y) points sorted by x.
fn table(points: &[(f32, f32)], x: f32) -> f32 {
    let (first, last) = (points[0], points[points.len() - 1]);
    if x <= first.0 {
        return first.1;
    }
    for w in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0).max(1e-6);
        }
    }
    last.1
}

/// Where a voice sits: distance 0 (close) .. 1 (far) and pan -1 .. 1.
#[derive(Clone, Copy, Debug, Default)]
struct Spot {
    dist: f32,
    pan: f32,
}

impl Spot {
    fn random(rng: &mut Rng, near: f32, far: f32) -> Self {
        Spot { dist: rng.range(near, far), pan: rng.next_bipolar() }
    }

    /// Distance `dist` scaled into a scene whose overall distance is `scene` (0..1, 0.5 neutral).
    fn effective(&self, scene: f32) -> f32 {
        (self.dist * (0.4 + 1.2 * scene) + 0.6 * (scene - 0.5).max(0.0)).clamp(0.0, 1.0)
    }
}

/// Gain, air/foliage low-pass coefficient and reverb send for a voice at distance `d`.
#[inline]
fn distance_cues(d: f32, top_hz: f32, far_hz: f32, sr: f32) -> (f32, f32, f32) {
    let gain = 1.0 / (1.0 + 4.0 * d * d + 1.5 * d);
    let lp = hz_coef(top_hz * (far_hz / top_hz).powf(d), sr);
    (gain, lp, 0.15 + 0.85 * d)
}

const EAR_LEN: usize = 64;

/// Places a voice between the speakers: balance-law levels plus up to 0.6 ms of delay at the far
/// ear, which is what lets separate voices be heard apart. At width 0 it passes the voice through
/// untouched, so the mono render is unchanged.
#[derive(Clone, Copy)]
struct Ear {
    buf: [f32; EAR_LEN],
    pos: usize,
}

impl Default for Ear {
    fn default() -> Self {
        Ear { buf: [0.0; EAR_LEN], pos: 0 }
    }
}

/// Left gain, right gain, and the far ear's delay in samples (negative: the left ear is far).
#[inline]
fn place(pan: f32, sr: f32) -> (f32, f32, i32) {
    let (gl, gr) = balance(pan);
    (gl, gr, (pan * 0.0006 * sr).round() as i32)
}

impl Ear {
    /// The voice as heard at the far ear, `delay` samples late.
    #[inline]
    fn far(&mut self, y: f32, delay: usize) -> f32 {
        self.buf[self.pos] = y;
        let d = self.buf[(self.pos + EAR_LEN - delay.min(EAR_LEN - 1)) % EAR_LEN];
        self.pos = (self.pos + 1) % EAR_LEN;
        d
    }

    /// Add `y` to the two channels with the gains and delay from [`place`].
    #[inline]
    fn mix(&mut self, y: f32, (gl, gr, delay): (f32, f32, i32), l: &mut f32, r: &mut f32) {
        let far = self.far(y, delay.unsigned_abs() as usize);
        if delay >= 0 {
            *l += far * gl;
            *r += y * gr;
        } else {
            *l += y * gl;
            *r += far * gr;
        }
    }
}

/// Seconds of rest drawn between `lo` and `hi`, skewed short.
#[inline]
fn rest_between(rng: &mut Rng, lo: f32, hi: f32) -> f32 {
    let u = rng.next_f32();
    lo + (hi - lo) * u * u
}

// ---------------------------------------------------------------------------------------------
// Birds
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Birds: a chorus of real species, each singer a [`Singer`] of the `bird` generator (songs
    /// transcribed from recordings), drawn from the pool by the species weights and placed at its
    /// own distance and side. `species/tropical` brings two invented voices (no recordings
    /// behind them); `night/*` are the voices of the night.
    BirdsParams / BirdsParamId {
        voices: "chorus/voices" = 10.0, int(1, 16);
        gaps: "chorus/gaps" = 0.5, UNIT;
        pitch: "song/pitch" = 1.0, exp(0.5, 2.0);
        tempo: "song/tempo" = 1.0, exp(0.5, 2.0);
        variety: "song/variety" = 0.5, UNIT;
        blackbird: "species/blackbird" = 0.8, UNIT;
        robin: "species/robin" = 0.7, UNIT;
        chaffinch: "species/chaffinch" = 0.7, UNIT;
        great_tit: "species/great_tit" = 0.6, UNIT;
        cuckoo: "species/cuckoo" = 0.15, UNIT;
        wood_pigeon: "species/wood_pigeon" = 0.3, UNIT;
        collared_dove: "species/collared_dove" = 0.15, UNIT;
        crow: "species/crow" = 0.15, UNIT;
        sparrow: "species/house_sparrow" = 0.25, UNIT;
        skylark: "species/skylark" = 0.0, UNIT;
        gull: "species/herring_gull" = 0.0, UNIT;
        woodpecker: "species/woodpecker" = 0.1, UNIT;
        tropical: "species/tropical" = 0.0, UNIT;
        owl: "night/owl" = 0.7, UNIT;
        nightingale: "night/nightingale" = 0.6, UNIT;
        distance: "space/distance" = 0.5, UNIT;
        reverb: "space/reverb" = 0.4, UNIT;
        rt60: "space/rt60_s" = 1.3, lin(0.2, 4.0);
        width: "space/width" = 0.7, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// The day species of the pool, in weight order; `species/tropical` is shared by the last two.
const DAY: [usize; 14] = [0, 1, 2, 3, 4, 5, 6, 7, 9, 12, 10, 13, 14, 15];
const OWL: usize = 8;
const NIGHTINGALE: usize = 11;
/// The chorus mixes its singers at this level (a close bird alone peaks near -8 dBFS).
const CHORUS: f32 = 0.55;

impl BirdsParams {
    fn weights(&self) -> [f32; 14] {
        let t = self.tropical * 0.5;
        [self.blackbird, self.robin, self.chaffinch, self.great_tit, self.cuckoo, self.wood_pigeon, self.collared_dove, self.crow, self.sparrow, self.skylark, self.gull, self.woodpecker, t, t]
    }
}

/// One bird of the chorus: its singer, place and air.
struct Perch {
    singer: Singer,
    /// Fixed 0..1 draw that picks the species from the weights.
    pick: f32,
    spot: Spot,
    lp: OnePole,
    ear: Ear,
}

impl Perch {
    fn new(species: usize, seed: u32, pick: f32, spot: Spot, sr: f32) -> Self {
        Perch { singer: Singer::new(species, seed, sr), pick, spot, lp: OnePole::default(), ear: Ear::default() }
    }
}

pub struct Birds {
    sr: f32,
    day: [Perch; 16],
    night: [Perch; 2],
    reverb: Reverb,
}

impl Birds {
    fn species_for(weights: &[f32; 14], pick: f32) -> Option<usize> {
        let total: f32 = weights.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut acc = 0.0;
        for (w, s) in weights.iter().zip(DAY) {
            acc += w / total;
            if pick < acc && *w > 0.0 {
                return Some(s);
            }
        }
        weights.iter().zip(DAY).rev().find(|(w, _)| **w > 0.0).map(|(_, s)| s)
    }

    fn render(&mut self, x: &[f32], p: &BirdsParams, width: f32, left: &mut [f32], right: &mut [f32]) {
        let (sr, n) = (self.sr, left.len());
        left.iter_mut().chain(right.iter_mut()).for_each(|s| *s = 0.0);
        let hour = x[1] * 24.0;
        // The dawn chorus peaks between 5 and 7, the day is quieter, there is a smaller evening
        // chorus, and at night only the night birds sing.
        let day = x[0] * table(&[(3.5, 0.0), (4.5, 0.5), (5.5, 1.0), (7.0, 1.0), (9.5, 0.5), (12.0, 0.35), (16.5, 0.35), (18.5, 0.6), (20.0, 0.5), (21.5, 0.0)], hour);
        let night = x[0] * table(&[(0.0, 1.0), (3.5, 0.8), (4.8, 0.3), (5.5, 0.0), (20.0, 0.0), (21.0, 0.6), (22.0, 1.0), (24.0, 1.0)], hour);
        let weights = p.weights();
        let awake = p.voices.clamp(1.0, 16.0) * day;
        // A busier chorus sings with shorter pauses; `chorus/gaps` scales all pauses.
        // (Real species already rest 3-11 s, so a quiet hour stretches them less than it did.)
        let rest_scale = (0.35 + 1.3 * p.gaps) * (1.8 - 1.25 * day.max(night));
        let control = |awake: bool| Control { awake, rest_scale, excitement: 0.0, pitch: p.pitch, tempo: p.tempo, variety: p.variety };
        let space = p.reverb * 1.2;
        let mut send = [0.0f32; BLOCK];
        let night_on = [night * p.owl > 0.05, night * p.nightingale > 0.05];
        // Night singers carry: an owl's hoot and a nightingale's song fill the dark woods.
        let night_level = [1.6 * p.owl, 2.0 * p.nightingale];
        for (k, perch) in self.day.iter_mut().chain(self.night.iter_mut()).enumerate() {
            let (on, level) = if k < 16 {
                let want = Self::species_for(&weights, perch.pick);
                if let Some(s) = want {
                    if s != perch.singer.species() && !perch.singer.is_busy() {
                        perch.singer.set_species(s);
                    }
                }
                ((k as f32) < awake && want.is_some(), 1.0)
            } else {
                (night_on[k - 16], night_level[k - 16])
            };
            let mut dry = [0.0f32; BLOCK];
            perch.singer.render(&mut dry[..n], &control(on));
            if !perch.singer.is_busy() && dry[..n].iter().all(|s| *s == 0.0) {
                continue;
            }
            let d = perch.spot.effective(p.distance);
            let (g, lp, sendk) = distance_cues(d, 16000.0, 3500.0, sr);
            let at = place(perch.spot.pan * width, sr);
            let (g, send_gain) = (g * level * CHORUS, sendk * space);
            for i in 0..n {
                let y = perch.lp.lp(dry[i] * g, lp);
                perch.ear.mix(y, at, &mut left[i], &mut right[i]);
                send[i] += y * send_gain;
            }
        }
        // The forest answers every note.
        let (gain, rt) = (p.gain, p.rt60);
        for i in 0..n {
            let (m, s) = self.reverb.tick_stereo(send[i], rt, 0.45);
            left[i] = (left[i] + m + s * width) * gain;
            right[i] = (right[i] + m - s * width) * gain;
        }
    }
}

impl Generator for Birds {
    type P = BirdsParams;
    const NAME: &'static str = "birds";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Songbirds: a chorus of real species (blackbird, robin, chaffinch, great tit, cuckoo, doves…) singing transcribed songs with variation and pauses; dawn chorus, day, dusk, and owls and nightingales at night.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "activity", default: 0.5, doc: "How many birds sing and how often: none to a full chorus" },
        InputSpec { name: "time_of_day", default: 0.3, doc: "0 = midnight, 0.25 = 06:00 (dawn chorus), 0.5 = noon, 0.75 = 18:00, 1 = midnight. Night brings owls and nightingales" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.3;

    fn presets() -> Vec<(&'static str, BirdsParams)> {
        let none = BirdsParams { blackbird: 0.0, robin: 0.0, chaffinch: 0.0, great_tit: 0.0, cuckoo: 0.0, wood_pigeon: 0.0, collared_dove: 0.0, crow: 0.0, sparrow: 0.0, skylark: 0.0, gull: 0.0, woodpecker: 0.0, tropical: 0.0, ..Default::default() };
        vec![
            ("Dawn chorus", BirdsParams { voices: 16.0, gaps: 0.3, distance: 0.55, reverb: 0.45, ..Default::default() }),
            ("Garden", BirdsParams { voices: 7.0, blackbird: 1.0, robin: 0.8, great_tit: 0.8, chaffinch: 0.5, collared_dove: 0.5, wood_pigeon: 0.5, sparrow: 0.7, crow: 0.1, distance: 0.3, reverb: 0.15, rt60: 0.8, ..none }),
            ("Meadow", BirdsParams { voices: 8.0, skylark: 1.0, chaffinch: 0.25, cuckoo: 0.3, crow: 0.2, wood_pigeon: 0.15, great_tit: 0.1, distance: 0.6, reverb: 0.06, rt60: 0.5, ..none }),
            // Invented voices: there are no tropical recordings behind this preset.
            ("Tropical", BirdsParams { voices: 12.0, tropical: 1.0, cuckoo: 0.1, wood_pigeon: 0.15, pitch: 0.95, reverb: 0.55, rt60: 2.2, ..none }),
            ("Night woods", BirdsParams { voices: 4.0, owl: 1.0, nightingale: 0.8, gaps: 0.6, reverb: 0.55, rt60: 1.8, ..Default::default() }),
        ]
    }

    fn snap(&mut self, x: &[f32], _p: &BirdsParams) {
        // A scene that starts mid-chorus: the birds that are awake start singing soon.
        if x[0] > 0.2 {
            self.day.iter_mut().chain(self.night.iter_mut()).for_each(|b| b.singer.hurry(2.0));
        }
    }

    fn new(sr: f32) -> Self {
        let mut rng = Rng::new(0x5B_0001);
        let weights = BirdsParams::default().weights();
        let day = core::array::from_fn(|k| {
            // Golden-ratio picks spread the species evenly over the pool.
            let pick = (0.37 + 0.618_034 * k as f32).fract();
            // The first few are the near birds; the rest fill the distance.
            let spot = if k < 3 { Spot::random(&mut rng, 0.0, 0.35) } else { Spot::random(&mut rng, 0.25, 1.0) };
            let species = Birds::species_for(&weights, pick).unwrap_or(0);
            Perch::new(species, 0x5B_0100 + k as u32, pick, spot, sr)
        });
        let night = [
            Perch::new(OWL, 0x5B_0200, 0.0, Spot { dist: 0.55, pan: -0.5 }, sr),
            Perch::new(NIGHTINGALE, 0x5B_0201, 0.0, Spot { dist: 0.25, pan: 0.35 }, sr),
        ];
        Birds { sr, day, night, reverb: Reverb::new(sr) }
    }

    fn block(&mut self, x: &[f32], p: &BirdsParams, out: &mut [f32]) {
        let mut right = [0.0f32; BLOCK];
        let n = out.len();
        self.render(x, p, 0.0, out, &mut right[..n]);
    }

    fn block_stereo(&mut self, x: &[f32], p: &BirdsParams, left: &mut [f32], right: &mut [f32]) {
        self.render(x, p, p.width, left, right);
    }
}

// ---------------------------------------------------------------------------------------------
// Insects
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Insects: a field of crickets, each chirping a train of pulses at its own slightly
    /// different rate (Dolbear's law ties the rate to `temperature`), a far-off chorus of more of
    /// them, and cicadas that swell and fade in the heat of the day.
    InsectsParams / InsectsParamId {
        cricket_hz: "cricket/hz" = 4300.0, exp(1500.0, 12000.0);
        crickets: "cricket/count" = 8.0, int(1, 16);
        pulses: "cricket/pulses" = 4.0, int(1, 12);
        pulse_ms: "cricket/pulse_ms" = 16.0, lin(2.0, 40.0);
        rate: "cricket/rate_scale" = 1.0, exp(0.25, 6.0);
        sync: "cricket/sync" = 0.15, UNIT;
        cricket_level: "cricket/level" = 0.8, UNIT;
        chorus: "chorus/level" = 0.35, UNIT;
        cicada_level: "cicada/level" = 0.6, UNIT;
        cicada_hz: "cicada/hz" = 5600.0, exp(1500.0, 10000.0);
        buzz_hz: "cicada/buzz_hz" = 230.0, exp(40.0, 800.0);
        swell: "cicada/swell_s" = 8.0, lin(1.0, 30.0);
        distance: "space/distance" = 0.5, UNIT;
        width: "space/width" = 0.7, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// Temperature input 0..1 in degrees Celsius.
#[inline]
fn celsius(t: f32) -> f32 {
    10.0 + 25.0 * t
}

/// Dolbear's law: chirps per minute = 7.2 C - 32 (from 4 (F - 40)), in chirps per second.
#[inline]
pub fn dolbear_hz(celsius: f32) -> f32 {
    ((7.2 * celsius - 32.0) / 60.0).max(0.15)
}

#[derive(Clone, Copy)]
struct Cricket {
    spot: Spot,
    rate_k: f32,
    hz_k: f32,
    phase: f32,
    jitter: f32,
    chirping: bool,
    tau: f32,
    pulses: u8,
    osc: f32,
    lp: OnePole,
    ear: Ear,
}

#[derive(Clone, Copy)]
struct Cicada {
    spot: Spot,
    state: u8,
    t: f32,
    len: f32,
    level: f32,
    bend: f32,
    click: f32,
    click_env: f32,
    body: Svf,
    tone: Svf,
    ear: Ear,
}

pub struct Insects {
    sr: f32,
    rng: Rng,
    noise: Noise,
    side_noise: Noise,
    crickets: [Cricket; 16],
    leader: f32,
    cicadas: [Cicada; 3],
    bed: [Svf; 2],
    bed_side: [Svf; 2],
    bed_phase: f32,
    bed_swell: SlowNoise,
}

impl Insects {
    fn render(&mut self, x: &[f32], p: &InsectsParams, width: f32, left: &mut [f32], right: &mut [f32]) {
        let (sr, n) = (self.sr, left.len());
        let dt = n as f32 / sr;
        left.iter_mut().chain(right.iter_mut()).for_each(|s| *s = 0.0);
        let (temp, density, daylight) = (x[0], x[1], x[2]);
        let c = celsius(temp);
        // Crickets slow down and fall quiet in the cold, and give way to the cicadas by day.
        let warm = 0.2 + 0.8 * smooth01((c - 10.0) / 6.0);
        let cricket_gain = p.cricket_level * warm * (1.0 - 0.5 * daylight) * 0.5;
        let chirp_hz = dolbear_hz(c) * p.rate;
        // The pulses inside a chirp speed up with temperature too.
        let pulse_len = p.pulse_ms * 0.001 * (22.5 / c).powf(0.7);
        let pulse_period = pulse_len * 2.6;
        let carrier = p.cricket_hz;
        let singing = (p.crickets * density).ceil() as usize;
        self.leader = (self.leader + chirp_hz * dt).fract();
        let coupling = p.sync * 1.5;
        for (k, cr) in self.crickets.iter_mut().enumerate() {
            let active = k < singing;
            cr.phase += chirp_hz * cr.rate_k * cr.jitter * dt;
            if coupling > 0.0 {
                // Kuramoto pull toward the field's leader: in step at sync 1, adrift at 0.
                let mut diff = self.leader - cr.phase.fract();
                diff -= diff.round();
                cr.phase += coupling * (TAU * diff).sin() / TAU * chirp_hz * dt * 6.0;
            }
            if cr.phase >= 1.0 {
                cr.phase -= cr.phase.floor();
                if active {
                    cr.chirping = true;
                    cr.tau = 0.0;
                    cr.jitter = 1.0 + 0.05 * self.rng.next_bipolar();
                    let extra = if self.rng.chance(0.2) { self.rng.next_bipolar().signum() as i32 } else { 0 };
                    cr.pulses = (p.pulses as i32 + extra).clamp(1, 12) as u8;
                }
            }
            if !cr.chirping {
                continue;
            }
            let d = cr.spot.effective(p.distance);
            let (g, lp, _) = distance_cues(d, 16000.0, 5000.0, sr);
            let at = place(cr.spot.pan * width, sr);
            let amp = cricket_gain * g;
            let hz = carrier * cr.hz_k;
            let chirp_len = cr.pulses as f32 * pulse_period;
            let inv_len = 1.0 / pulse_len;
            for i in 0..n {
                let tau = cr.tau;
                cr.tau += 1.0 / sr;
                let k = (tau / pulse_period) as u32;
                let u = (tau - k as f32 * pulse_period) * inv_len;
                let mut y = 0.0;
                if u < 1.0 && (k as u8) < cr.pulses {
                    // Each pulse is one closing stroke of the wing: a quick rise, a decay and a
                    // slight fall in pitch as the scraper slows along the file.
                    let e = smooth01(u * 8.0) * (1.0 - u);
                    cr.osc += hz * (1.0 - 0.07 * u) / sr;
                    if cr.osc >= 1.0 {
                        cr.osc -= 1.0;
                    }
                    let ph = cr.osc * TAU;
                    y = (ph.sin() + 0.15 * (2.0 * ph).sin()) * e * amp;
                }
                let y = cr.lp.lp(y, lp);
                cr.ear.mix(y, at, &mut left[i], &mut right[i]);
            }
            if cr.tau > chirp_len + 0.01 {
                cr.chirping = false;
            }
        }
        // The far field: more crickets than can be told apart, a shimmering band at the carrier.
        let bed_gain = p.chorus * cricket_gain * density * 0.05 * (1.0 + 0.4 * self.bed_swell.advance(0.2, dt));
        if bed_gain > 1e-5 {
            // Two band-passes in a row: a narrow band with steep skirts, as recorded.
            for f in self.bed.iter_mut().chain(self.bed_side.iter_mut()) {
                f.set(FilterMode::BandPass, carrier, 0.9, sr);
            }
            let pulse_hz = 1.0 / pulse_period;
            let phi = width * core::f32::consts::FRAC_PI_4;
            let (cm, cs) = (phi.cos(), phi.sin());
            for i in 0..n {
                self.bed_phase = (self.bed_phase + pulse_hz / sr).fract();
                let am = 0.55 + 0.45 * (TAU * self.bed_phase).sin();
                let m = self.bed[0].tick(self.noise.white());
                let m = self.bed[1].tick(m) * am * bed_gain;
                let s = if width > 0.0 {
                    let s = self.bed_side[0].tick(self.side_noise.white());
                    self.bed_side[1].tick(s) * am * bed_gain
                } else {
                    0.0
                };
                left[i] += m * cm + s * cs;
                right[i] += m * cm - s * cs;
            }
        }
        // Cicadas: the heat of the day.
        let heat = smooth01((c - 18.0) / 10.0);
        let cicada_gain = p.cicada_level * daylight * (0.3 + 0.7 * heat) * 0.5;
        let decay = (-1.0 / (0.0007 * sr)).exp();
        for (k, ci) in self.cicadas.iter_mut().enumerate() {
            // Swell: rest -> rise -> hold -> fall (with a falling pitch) -> rest.
            ci.t += dt;
            if ci.t >= ci.len {
                ci.t = 0.0;
                ci.state = (ci.state + 1) % 4;
                ci.len = match ci.state {
                    0 => rest_between(&mut self.rng, 1.0, 8.0),
                    1 => self.rng.range(1.5, 4.0),
                    2 => p.swell * self.rng.range(0.6, 1.4),
                    _ => self.rng.range(1.5, 3.0),
                };
                // Fewer cicadas join in on a thin day.
                if ci.state == 1 && (k as f32) >= 0.5 + 2.6 * density {
                    ci.state = 0;
                }
            }
            let u = ci.t / ci.len.max(1e-3);
            let (target, bend) = match ci.state {
                0 => (0.0, 1.0),
                1 => (smooth01(u), 0.92 + 0.08 * u),
                2 => (1.0, 1.0),
                _ => ((1.0 - u) * (1.0 - u), 1.0 - 0.15 * u),
            };
            ci.level += (target - ci.level) * settle(dt, 0.1);
            ci.bend = bend;
            let level = ci.level * cicada_gain;
            if level < 1e-5 {
                continue;
            }
            let d = ci.spot.effective(p.distance);
            let (g, _, _) = distance_cues(d, 16000.0, 5000.0, sr);
            let at = place(ci.spot.pan * width, sr);
            let hz = p.cicada_hz * (1.0 + 0.04 * k as f32) * ci.bend;
            ci.body.set(FilterMode::BandPass, hz, 0.8, sr);
            ci.tone.set(FilterMode::BandPass, hz * 1.02, 0.96, sr);
            let click_hz = p.buzz_hz * (1.0 + 0.06 * k as f32) * ci.bend;
            let amp = level * g;
            for i in 0..n {
                ci.click += click_hz / sr;
                if ci.click >= 1.0 {
                    ci.click -= 1.0;
                    ci.click_env = 1.0;
                }
                ci.click_env *= decay;
                let e = self.noise.white() * ci.click_env;
                let y = (ci.body.tick(e) + 0.5 * ci.tone.tick(e)) * amp;
                ci.ear.mix(y, at, &mut left[i], &mut right[i]);
            }
        }
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            *l *= p.gain;
            *r *= p.gain;
        }
    }
}

/// One-pole settle coefficient for a step of `dt` toward a target with time constant `tau`.
#[inline]
fn settle(dt: f32, tau: f32) -> f32 {
    1.0 - (-dt / tau.max(1e-4)).exp()
}

impl Generator for Insects {
    type P = InsectsParams;
    const NAME: &'static str = "insects";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Crickets at night and cicadas by day; the cricket chirp rate follows the temperature (Dolbear's law).";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "temperature", default: 0.5, doc: "0 = 10 C (slow, faltering) .. 1 = 35 C; Celsius = 10 + 25 x. Chirps per minute = 7.2 C - 32" },
        InputSpec { name: "density", default: 0.6, doc: "How many insects: a few to a whole field" },
        InputSpec { name: "daylight", default: 0.0, doc: "0 = night (crickets), 1 = hot day (cicadas take over)" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.5;

    fn presets() -> Vec<(&'static str, InsectsParams)> {
        vec![
            // Snowy tree crickets, the species Dolbear timed: lower, shorter pulses, in step.
            ("Tree crickets", InsectsParams { cricket_hz: 2900.0, pulses: 7.0, pulse_ms: 7.0, sync: 0.9, chorus: 0.5, ..Default::default() }),
            // Katydid-like: high (9 kHz) and fast, about nine short chirps a second.
            ("Katydids", InsectsParams { cricket_hz: 9300.0, pulses: 8.0, pulse_ms: 3.5, rate: 4.0, crickets: 5.0, sync: 0.3, ..Default::default() }),
            ("Cicada summer", InsectsParams { cicada_level: 1.0, cricket_level: 0.4, chorus: 0.2, swell: 12.0, ..Default::default() }),
            ("Lone cricket", InsectsParams { crickets: 1.0, chorus: 0.0, distance: 0.2, cicada_level: 0.0, cricket_level: 1.0, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        let mut rng = Rng::new(0x5C_0001);
        let crickets = core::array::from_fn(|k| Cricket {
            spot: if k < 2 { Spot::random(&mut rng, 0.0, 0.3) } else { Spot::random(&mut rng, 0.2, 1.0) },
            rate_k: 1.0 + 0.045 * rng.next_bipolar(),
            hz_k: 1.0 + 0.03 * rng.next_bipolar(),
            phase: rng.next_f32(),
            jitter: 1.0,
            chirping: false,
            tau: 0.0,
            pulses: 4,
            osc: 0.0,
            lp: OnePole::default(),
            ear: Ear::default(),
        });
        let cicadas = core::array::from_fn(|k| Cicada {
            spot: Spot::random(&mut rng, 0.2, 0.9),
            state: 0,
            t: 0.0,
            len: 0.5 + 2.0 * k as f32,
            level: 0.0,
            bend: 1.0,
            click: rng.next_f32(),
            click_env: 0.0,
            body: Svf::default(),
            tone: Svf::default(),
            ear: Ear::default(),
        });
        Insects {
            sr,
            rng,
            noise: Noise::new(0x5C_0002),
            side_noise: Noise::new(0x5C_0003),
            crickets,
            leader: 0.0,
            cicadas,
            bed: [Svf::default(); 2],
            bed_side: [Svf::default(); 2],
            bed_phase: 0.0,
            bed_swell: SlowNoise::new(0x5C_0004),
        }
    }

    fn block(&mut self, x: &[f32], p: &InsectsParams, out: &mut [f32]) {
        let mut right = [0.0f32; BLOCK];
        let n = out.len();
        self.render(x, p, 0.0, out, &mut right[..n]);
    }

    fn block_stereo(&mut self, x: &[f32], p: &InsectsParams, left: &mut [f32], right: &mut [f32]) {
        self.render(x, p, p.width, left, right);
    }
}

// ---------------------------------------------------------------------------------------------
// Frogs
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Frogs: each frog is a pulse train (its larynx) through two throat resonances, calling in
    /// bouts with pauses, and waiting for a gap before it calls (frogs take turns).
    FrogsParams / FrogsParamId {
        frogs: "chorus/frogs" = 8.0, int(1, 12);
        treefrog: "species/treefrog" = 1.0, UNIT;
        bullfrog: "species/bullfrog" = 0.2, UNIT;
        chorus_frog: "species/chorus_frog" = 0.3, UNIT;
        toad: "species/toad" = 0.0, UNIT;
        woodfrog: "species/woodfrog" = 0.0, UNIT;
        throat: "throat/scale" = 1.0, exp(0.5, 2.0);
        resonance: "throat/resonance" = 0.93, lin(0.3, 0.97);
        turns: "chorus/turn_taking" = 0.7, UNIT;
        distance: "space/distance" = 0.5, UNIT;
        reverb: "space/reverb" = 0.25, UNIT;
        width: "space/width" = 0.7, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrogKind {
    Tree,
    Bull,
    Chorus,
    Toad,
    Wood,
}

const FROG_KINDS: [FrogKind; 5] = [FrogKind::Tree, FrogKind::Bull, FrogKind::Chorus, FrogKind::Toad, FrogKind::Wood];

/// How one kind of frog calls.
struct Call {
    f1: f32,
    f2: f32,
    g1: f32,
    g2: f32,
    pulse_hz: f32,
    /// Notes of one call: (duration, gap after, formant shift).
    notes: &'static [(f32, f32, f32)],
    /// Pulse groups inside a note (the comb of a chorus frog, the trill of a toad); 0 = none.
    group_hz: f32,
    period: (f32, f32),
    bout: (u8, u8),
    rest: (f32, f32),
    att: f32,
    rel: f32,
    /// Irregularity of the pulses (wood frogs cluck).
    rough: f32,
    level: f32,
}

impl FrogKind {
    fn call(self) -> Call {
        match self {
            // Pacific tree frog: "rib-bit", the second note higher, about two calls a second.
            FrogKind::Tree => Call {
                f1: 1000.0,
                f2: 2150.0,
                g1: 0.3,
                g2: 1.0,
                pulse_hz: 95.0,
                notes: &[(0.1, 0.015, 1.0), (0.13, 0.0, 1.08)],
                group_hz: 0.0,
                period: (0.36, 0.46),
                bout: (6, 22),
                rest: (1.0, 5.0),
                att: 0.15,
                rel: 0.35,
                rough: 0.05,
                level: 1.0,
            },
            // Bullfrog: a deep "rumm", throbbing, a few in a row.
            FrogKind::Bull => Call {
                f1: 230.0,
                f2: 1100.0,
                g1: 1.0,
                g2: 0.3,
                pulse_hz: 105.0,
                notes: &[(0.65, 0.0, 1.0)],
                group_hz: 5.5,
                period: (1.0, 1.4),
                bout: (3, 6),
                rest: (8.0, 22.0),
                att: 0.12,
                rel: 0.2,
                rough: 0.08,
                level: 2.6,
            },
            // Chorus frog: a rising comb-like "crreeek" of pulse groups near 3 kHz.
            FrogKind::Chorus => Call {
                f1: 3200.0,
                f2: 1260.0,
                g1: 1.0,
                g2: 0.25,
                pulse_hz: 114.0,
                notes: &[(0.45, 0.0, 1.0)],
                group_hz: 27.0,
                period: (0.9, 1.3),
                bout: (8, 25),
                rest: (2.0, 7.0),
                att: 0.2,
                rel: 0.1,
                rough: 0.03,
                level: 0.7,
            },
            // Toad: a long, even trill.
            FrogKind::Toad => Call {
                f1: 1700.0,
                f2: 3400.0,
                g1: 1.0,
                g2: 0.2,
                pulse_hz: 120.0,
                notes: &[(5.0, 0.0, 1.0)],
                group_hz: 38.0,
                period: (6.0, 7.0),
                bout: (1, 1),
                rest: (8.0, 25.0),
                att: 0.05,
                rel: 0.05,
                rough: 0.02,
                level: 0.8,
            },
            // Wood frog: short duck-like clucks in little runs.
            FrogKind::Wood => Call {
                f1: 900.0,
                f2: 1900.0,
                g1: 0.8,
                g2: 1.0,
                pulse_hz: 70.0,
                notes: &[(0.07, 0.09, 1.0), (0.06, 0.1, 0.97), (0.07, 0.0, 1.02)],
                group_hz: 0.0,
                period: (0.5, 0.9),
                bout: (2, 8),
                rest: (2.0, 8.0),
                att: 0.2,
                rel: 0.4,
                rough: 0.5,
                level: 1.0,
            },
        }
    }
}

struct Frog {
    kind: FrogKind,
    pick: f32,
    spot: Spot,
    size: f32,
    rng: Rng,
    /// Two throat resonances, each two band-passes in a row (narrow, steep-sided formants).
    f1: [Svf; 2],
    f2: [Svf; 2],
    lp: OnePole,
    ear: Ear,
    pulse: f32,
    group_phase: f32,
    rest: f32,
    calls_left: u8,
    next_call: f32,
    note: usize,
    note_t: f32,
    in_call: bool,
    ring: f32,
    awake: bool,
}

pub struct Frogs {
    sr: f32,
    frogs: [Frog; 12],
    noise: Noise,
    since_onset: f32,
    mood: SlowNoise,
    reverb: Reverb,
}

impl Frogs {
    fn kind_for(weights: &[f32; 5], pick: f32) -> Option<FrogKind> {
        let total: f32 = weights.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut acc = 0.0;
        for (w, k) in weights.iter().zip(FROG_KINDS) {
            acc += w / total;
            if pick < acc {
                return Some(k);
            }
        }
        Some(FROG_KINDS[4])
    }

    fn render(&mut self, x: &[f32], p: &FrogsParams, width: f32, left: &mut [f32], right: &mut [f32]) {
        let (sr, n) = (self.sr, left.len());
        let dt = n as f32 / sr;
        left.iter_mut().chain(right.iter_mut()).for_each(|s| *s = 0.0);
        let (activity, temp) = (x[0], x[1]);
        // Warm frogs call faster and their pulses run faster.
        let warm = 0.75 + 0.5 * temp;
        let weights = [p.treefrog, p.bullfrog, p.chorus_frog, p.toad, p.woodfrog];
        // The chorus comes in waves: one starts, the others join, then it dies down.
        let mood = 0.5 + 0.5 * self.mood.advance(0.05, dt);
        let calling = p.frogs * activity * (0.6 + 0.6 * mood);
        self.since_onset += dt;
        let mut send = [0.0f32; BLOCK];
        let space = p.reverb;
        let res = p.resonance;
        for (k, f) in self.frogs.iter_mut().enumerate() {
            let want = Self::kind_for(&weights, f.pick);
            f.awake = (k as f32) < calling && want.is_some();
            if let Some(kind) = want {
                if kind != f.kind && !f.in_call && f.ring <= 0.0 {
                    f.kind = kind;
                }
            }
            let call = f.kind.call();
            // Bout and call timing.
            if !f.in_call {
                if f.calls_left == 0 {
                    f.rest -= dt;
                    if f.rest <= 0.0 {
                        if f.awake {
                            f.calls_left = call.bout.0 + (f.rng.next_f32() * (call.bout.1 - call.bout.0 + 1) as f32) as u8;
                            f.next_call = 0.0;
                        } else {
                            f.rest = f.rng.range(0.5, 2.0);
                        }
                    }
                } else {
                    f.next_call -= dt;
                    if f.next_call <= 0.0 {
                        // Frogs take turns: one that is about to call waits for a gap.
                        if self.since_onset < 0.08 && f.rng.chance(p.turns) {
                            f.next_call = f.rng.range(0.04, 0.15);
                        } else {
                            f.in_call = true;
                            f.note = 0;
                            f.note_t = 0.0;
                            f.calls_left -= 1;
                            f.next_call = f.rng.range(call.period.0, call.period.1) / warm;
                            self.since_onset = 0.0;
                        }
                    }
                }
            }
            if !f.in_call && f.ring <= 0.0 {
                continue;
            }
            // Where in the call we are: this note's envelope at the block's start and end.
            let (mut e0, mut e1, mut shift) = (0.0, 0.0, 1.0);
            if f.in_call {
                let (dur, gap, s) = call.notes[f.note.min(call.notes.len() - 1)];
                let dur = dur / warm.sqrt();
                shift = s;
                let env = |t: f32| {
                    let u = t / dur;
                    if !(0.0..1.0).contains(&u) {
                        0.0
                    } else {
                        smooth01(u / call.att) * smooth01((1.0 - u) / call.rel)
                    }
                };
                e0 = env(f.note_t);
                f.note_t += dt;
                e1 = env(f.note_t);
                if f.note_t >= dur + gap {
                    f.note += 1;
                    f.note_t = 0.0;
                    if f.note >= call.notes.len() {
                        f.in_call = false;
                        f.ring = 0.05;
                        if f.calls_left == 0 {
                            f.rest = rest_between(&mut f.rng, call.rest.0, call.rest.1) * (1.6 - activity);
                        }
                    }
                }
            } else {
                f.ring -= dt;
            }
            let scale = p.throat * f.size;
            for (a, b) in f.f1.iter_mut().zip(f.f2.iter_mut()) {
                a.set(FilterMode::BandPass, call.f1 / scale * shift, res, sr);
                b.set(FilterMode::BandPass, call.f2 / scale * shift, res, sr);
            }
            let d = f.spot.effective(p.distance);
            let (g, lp, sendk) = distance_cues(d, 14000.0, 3000.0, sr);
            let at = place(f.spot.pan * width, sr);
            // Narrower throats ring longer and louder; keep the level steady across `throat/resonance`.
            let amp = call.level * g * 9.0 * (2.0 - 1.98 * res).powf(1.5);
            let pulse_hz = call.pulse_hz * warm / scale.sqrt();
            let group_hz = call.group_hz * warm;
            let (g1, g2) = (call.g1, call.g2);
            let mut e = e0;
            let de = (e1 - e0) / n as f32;
            for i in 0..n {
                e += de;
                let mut ex = 0.0;
                f.pulse += pulse_hz * (1.0 + call.rough * f.rng.next_bipolar()) / sr;
                if f.pulse >= 1.0 {
                    f.pulse -= 1.0;
                    ex = e;
                    if group_hz > 0.0 {
                        // Pulse groups: the note is chopped into a comb of short bursts.
                        f.group_phase = (f.group_phase + group_hz * (1.0 / pulse_hz)).fract();
                        ex *= smooth01((0.65 - f.group_phase) * 6.0);
                    }
                }
                ex += 0.04 * e * self.noise.white();
                let (a, b) = (f.f1[0].tick(ex), f.f2[0].tick(ex));
                let (a, b) = (f.f1[1].tick(a), f.f2[1].tick(b));
                let y = f.lp.lp(g1 * a + g2 * b, lp) * amp;
                f.ear.mix(y, at, &mut left[i], &mut right[i]);
                send[i] += y * sendk * space;
            }
        }
        let gain = p.gain;
        for i in 0..n {
            let (m, s) = self.reverb.tick_stereo(send[i], 0.9, 0.5);
            left[i] = (left[i] + m + s * width) * gain;
            right[i] = (right[i] + m - s * width) * gain;
        }
    }
}

impl Generator for Frogs {
    type P = FrogsParams;
    const NAME: &'static str = "frogs";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "A pond chorus: tree frogs, bullfrogs, chorus frogs, toads and wood frogs calling in bouts and taking turns.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "activity", default: 0.6, doc: "Silent pond to full chorus" },
        InputSpec { name: "temperature", default: 0.5, doc: "Cold and slow to warm and fast (calls and pulses speed up)" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.3;

    fn presets() -> Vec<(&'static str, FrogsParams)> {
        vec![
            ("Tree frogs", FrogsParams { frogs: 10.0, treefrog: 1.0, bullfrog: 0.0, chorus_frog: 0.0, ..Default::default() }),
            ("Bullfrogs", FrogsParams { frogs: 4.0, treefrog: 0.0, bullfrog: 1.0, chorus_frog: 0.0, reverb: 0.35, ..Default::default() }),
            ("Spring marsh", FrogsParams { frogs: 12.0, treefrog: 0.2, bullfrog: 0.0, chorus_frog: 1.0, toad: 0.2, woodfrog: 0.5, distance: 0.6, ..Default::default() }),
            ("Toads", FrogsParams { frogs: 4.0, treefrog: 0.0, bullfrog: 0.0, chorus_frog: 0.0, toad: 1.0, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        let mut rng = Rng::new(0x5D_0001);
        let weights = {
            let d = FrogsParams::default();
            [d.treefrog, d.bullfrog, d.chorus_frog, d.toad, d.woodfrog]
        };
        let frogs = core::array::from_fn(|k| {
            let pick = (0.21 + 0.618_034 * k as f32).fract();
            let spot = if k < 2 { Spot::random(&mut rng, 0.0, 0.3) } else { Spot::random(&mut rng, 0.2, 1.0) };
            Frog {
                kind: Frogs::kind_for(&weights, pick).unwrap_or(FrogKind::Tree),
                pick,
                spot,
                size: 1.0 + 0.07 * rng.next_bipolar(),
                rng: Rng::new(mix_seed(0x5D_0100 + k as u32)),
                f1: [Svf::default(); 2],
                f2: [Svf::default(); 2],
                lp: OnePole::default(),
                ear: Ear::default(),
                pulse: 0.0,
                group_phase: 0.0,
                rest: rng.range(0.0, 3.0),
                calls_left: 0,
                next_call: 0.0,
                note: 0,
                note_t: 0.0,
                in_call: false,
                ring: 0.0,
                awake: false,
            }
        });
        Frogs { sr, frogs, noise: Noise::new(0x5D_0002), since_onset: 1.0, mood: SlowNoise::new(0x5D_0003), reverb: Reverb::new(sr) }
    }

    fn block(&mut self, x: &[f32], p: &FrogsParams, out: &mut [f32]) {
        let mut right = [0.0f32; BLOCK];
        let n = out.len();
        self.render(x, p, 0.0, out, &mut right[..n]);
    }

    fn block_stereo(&mut self, x: &[f32], p: &FrogsParams, left: &mut [f32], right: &mut [f32]) {
        self.render(x, p, p.width, left, right);
    }
}

// ---------------------------------------------------------------------------------------------
// Leaves
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Leaves and grass in the wind: a crackle of countless small leaf contacts over the sheet
    /// noise of the canopy, a flicker of fluttering leaves and the hush of grass, all moving with
    /// the gusts. Takes the same inputs as `wind`; at the default gust rate the gusts are the
    /// wind's own, so the two move together when started together.
    LeavesParams / LeavesParamId {
        rustle_level: "rustle/level" = 0.7, UNIT;
        rustle_hz: "rustle/hz" = 1300.0, exp(400.0, 8000.0);
        grain_ms: "rustle/grain_ms" = 2.0, lin(0.3, 10.0);
        crisp: "rustle/crispness" = 0.3, UNIT;
        threshold: "rustle/threshold" = 0.12, UNIT;
        canopy_level: "canopy/level" = 0.6, UNIT;
        canopy_hz: "canopy/hz" = 700.0, exp(200.0, 4000.0);
        flutter: "flutter/level" = 0.3, UNIT;
        flutter_hz: "flutter/rate_hz" = 8.0, exp(2.0, 25.0);
        grass: "grass/level" = 0.2, UNIT;
        gust_rate: "gusts/rate_hz" = 0.25, exp(0.02, 4.0);
        width: "space/width" = 0.8, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

/// One channel of rustle (the stereo image is two of these).
struct Rustle {
    noise: Noise,
    dust: Dust,
    env: f32,
    grain: Svf,
    top: OnePole,
    canopy: Svf,
    grass: Svf,
    grass_top: OnePole,
    tone: SlowNoise,
}

impl Rustle {
    fn new(seed: u32) -> Self {
        Rustle {
            noise: Noise::new(mix_seed(seed)),
            dust: Dust::new(seed ^ 0x77),
            env: 0.0,
            grain: Svf::default(),
            top: OnePole::default(),
            canopy: Svf::default(),
            grass: Svf::default(),
            grass_top: OnePole::default(),
            tone: SlowNoise::new(seed ^ 0x99),
        }
    }
}

pub struct Leaves {
    sr: f32,
    gust: SlowNoise,
    motion: f32,
    flutter: SlowNoise,
    a: Rustle,
    b: Rustle,
}

/// Per-block settings shared by both channels of rustle.
struct RustleBlock {
    grain_p: f32,
    decay: f32,
    power: f32,
    rustle_gain: f32,
    canopy_gain: f32,
    grass_gain: f32,
    top: f32,
    grass_top: f32,
    flutter: f32,
    dflutter: f32,
}

impl Leaves {
    /// How much the foliage moves for a wind strength `s` (0 below the threshold).
    fn motion(s: f32, p: &LeavesParams) -> f32 {
        ((s - p.threshold) / (1.0 - p.threshold).max(0.05)).clamp(0.0, 1.4)
    }

    fn channel(r: &mut Rustle, k: &RustleBlock, p: &LeavesParams, sr: f32, dt: f32, out: &mut [f32]) {
        // Each grain gets its own colour: re-tune the grain filter quickly.
        let hz = p.rustle_hz * (0.7 * r.tone.advance(40.0, dt)).exp2();
        r.grain.set(FilterMode::BandPass, hz, 0.15, sr);
        r.canopy.set(FilterMode::BandPass, p.canopy_hz, 0.1, sr);
        r.grass.set(FilterMode::HighPass, 2500.0, 0.1, sr);
        let mut fl = k.flutter;
        for o in out.iter_mut() {
            fl += k.dflutter;
            let (w, pk) = (r.noise.white(), r.noise.pink());
            let d = r.dust.tick(k.grain_p);
            if d > 0.0 {
                // Power-law sizes: most contacts are faint, a few leaves slap.
                let u = r.dust.rng().next_f32();
                r.env = r.env.max(u.powf(k.power));
            }
            r.env *= k.decay;
            let grains = r.top.lp(r.grain.tick(pk * r.env), k.top) * k.rustle_gain;
            let canopy = r.canopy.tick(pk) * k.canopy_gain;
            let grass = r.grass_top.lp(r.grass.tick(w), k.grass_top) * k.grass_gain;
            *o = (grains + canopy) * fl + grass;
        }
    }

    fn render(&mut self, x: &[f32], p: &LeavesParams, width: f32, left: &mut [f32], right: &mut [f32]) {
        let (sr, n) = (self.sr, left.len());
        let dt = n as f32 / sr;
        // The same gust stream and formula as `wind`, so the two move together.
        let gust = self.gust.advance(p.gust_rate, dt);
        let s = (x[0] * (1.0 + x[1] * 0.8 * gust)).clamp(0.0, 1.3);
        // Branches and leaves answer a gust a moment later.
        let target = Self::motion(s, p);
        self.motion += (target - self.motion) * settle(dt, 0.25);
        let m = self.motion;
        // Fluttering leaves flicker the whole sound a few times a second.
        let fl0 = 1.0 - p.flutter * 0.6 * (0.5 + 0.5 * self.flutter.advance(p.flutter_hz * (0.6 + 0.8 * m), dt)) * m.sqrt();
        let k = RustleBlock {
            grain_p: (300.0 + 9000.0 * m) * (1.0 - 0.7 * p.crisp) / sr,
            decay: (-1.0 / (p.grain_ms * (1.0 - 0.6 * p.crisp) * 0.001 * sr)).exp(),
            power: 1.5 + 4.0 * p.crisp,
            rustle_gain: p.rustle_level * m.powf(1.1) * (1.0 + 2.5 * p.crisp) * 6.1,
            canopy_gain: p.canopy_level * m.powf(1.4) * 2.5,
            grass_gain: p.grass * m.powf(1.4) * 0.18,
            top: hz_coef(4500.0 + 6000.0 * p.crisp, sr),
            grass_top: hz_coef(9000.0, sr),
            flutter: fl0,
            dflutter: 0.0,
        };
        Self::channel(&mut self.a, &k, p, sr, dt, left);
        if width > 0.0 {
            Self::channel(&mut self.b, &k, p, sr, dt, right);
            let phi = width * core::f32::consts::FRAC_PI_4;
            let (cm, cs) = (phi.cos(), phi.sin());
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let (mid, side) = (*l, *r);
                *l = (mid * cm + side * cs) * p.gain;
                *r = (mid * cm - side * cs) * p.gain;
            }
        } else {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                *l *= p.gain;
                *r = *l;
            }
        }
    }
}

impl Generator for Leaves {
    type P = LeavesParams;
    const NAME: &'static str = "leaves";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Leaves and grass rustling in the wind, swelling with the gusts. Same inputs as `wind`, to pair with it.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "strength", default: 0.4, doc: "Still air to storm (the wind's strength)" },
        InputSpec { name: "gustiness", default: 0.5, doc: "How much the strength wanders by itself" },
    ];

    fn presets() -> Vec<(&'static str, LeavesParams)> {
        vec![
            ("Aspen", LeavesParams { flutter: 0.85, flutter_hz: 11.0, rustle_hz: 2400.0, canopy_level: 0.45, ..Default::default() }),
            ("Dry autumn", LeavesParams { crisp: 0.85, rustle_hz: 3000.0, grain_ms: 1.2, canopy_level: 0.35, flutter: 0.1, ..Default::default() }),
            ("Tall grass", LeavesParams { rustle_level: 0.25, canopy_level: 0.35, canopy_hz: 1600.0, grass: 0.9, flutter: 0.0, threshold: 0.05, ..Default::default() }),
            ("Pine forest", LeavesParams { rustle_level: 0.2, canopy_level: 0.9, canopy_hz: 700.0, flutter: 0.0, grass: 0.05, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Leaves {
            sr,
            // The seed of `wind`'s gust stream (nature.rs).
            gust: SlowNoise::new(0x50_0002),
            motion: 0.0,
            flutter: SlowNoise::new(0x5E_0001),
            a: Rustle::new(0x5E_0100),
            b: Rustle::new(0x5E_0200),
        }
    }

    fn snap(&mut self, x: &[f32], p: &LeavesParams) {
        let s = x[0].clamp(0.0, 1.3);
        self.motion = Self::motion(s, p);
    }

    fn block(&mut self, x: &[f32], p: &LeavesParams, out: &mut [f32]) {
        let mut right = [0.0f32; BLOCK];
        let n = out.len();
        self.render(x, p, 0.0, out, &mut right[..n]);
    }

    fn block_stereo(&mut self, x: &[f32], p: &LeavesParams, left: &mut [f32], right: &mut [f32]) {
        self.render(x, p, p.width, left, right);
    }
}

// ---------------------------------------------------------------------------------------------
// Thunder
// ---------------------------------------------------------------------------------------------

model_params! {
    /// Thunder: the sound of a lightning channel kilometres long reaching the listener a piece
    /// at a time. The nearest part arrives first as a crack (one per return stroke) and a second
    /// of tearing; the rest rolls in later, lower and duller the farther it came from.
    ThunderParams / ThunderParamId {
        crack: "crack/level" = 0.8, UNIT;
        strokes: "crack/strokes" = 3.0, int(1, 6);
        rumble: "rumble/level" = 0.8, UNIT;
        rumble_hz: "rumble/hz" = 190.0, exp(40.0, 400.0);
        length: "rumble/length_s" = 14.0, lin(2.0, 20.0);
        roll: "rumble/roll" = 0.6, UNIT;
        variation: "shape/variation" = 0.5, UNIT;
        width: "space/width" = 0.7, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

const BOLTS: usize = 64;

/// One piece of the channel arriving: envelope and how much it reaches each band.
#[derive(Clone, Copy, Default)]
struct Bolt {
    t: f32,
    amp: f32,
    att: f32,
    dec: f32,
    hf: f32,
    mid: f32,
    pan: f32,
}

/// A band of noise: a high-pass into a low-pass, for a mid and a side stream.
#[derive(Clone, Copy, Default)]
struct Band {
    m: [Svf; 2],
    s: [Svf; 2],
}

impl Band {
    fn set(&mut self, hp: f32, lp: f32, q: f32, sr: f32) {
        let (hp, lp) = (hp.min(sr * 0.4), lp.min(sr * 0.45));
        for f in [&mut self.m, &mut self.s] {
            f[0].set(FilterMode::HighPass, hp, 0.1, sr);
            f[1].set(FilterMode::LowPass, lp, q, sr);
        }
    }

    #[inline]
    fn tick(&mut self, mid: f32, side: f32, stereo: bool) -> (f32, f32) {
        let m = self.m[0].tick(mid);
        let m = self.m[1].tick(m);
        let s = if stereo {
            let s = self.s[0].tick(side);
            self.s[1].tick(s)
        } else {
            0.0
        };
        (m, s)
    }

    fn reset(&mut self) {
        for f in self.m.iter_mut().chain(self.s.iter_mut()) {
            f.reset();
        }
    }
}

pub struct Thunder {
    sr: f32,
    rng: Rng,
    noise: Noise,
    side: Noise,
    brown: [Brown; 2],
    bolts: [Bolt; BOLTS],
    count: usize,
    /// Seconds since the trigger; negative = idle.
    t: f32,
    end: f32,
    strokes: [f32; 6],
    stroke_count: usize,
    next_stroke: usize,
    snap: f32,
    tear: f32,
    tear_decay: f32,
    tear_env: f32,
    tear_dust: Dust,
    crack_w: f32,
    hf: Band,
    mid: Band,
    low: Band,
    far: [[OnePole; 2]; 2],
    far_hz: f32,
    /// The diffuse rumble: the lumps echoing off the ground and clouds, per channel.
    wash: [f32; 2],
    level: f32,
    post: f32,
    rumble_hz: f32,
    pitch: f32,
}

impl Thunder {
    /// Upper bound of the event length for params `p`, tail included.
    fn max_len(p: &ThunderParams) -> f32 {
        p.length * 1.3 * (1.0 + 0.3 * p.variation) * 1.3 + 7.4
    }

    fn render(&mut self, p: &ThunderParams, width: f32, left: &mut [f32], right: &mut [f32]) {
        let (sr, n) = (self.sr, left.len());
        left.iter_mut().chain(right.iter_mut()).for_each(|s| *s = 0.0);
        if self.t < 0.0 {
            return;
        }
        let dt = n as f32 / sr;
        let (t0, t1) = (self.t, self.t + dt);
        // Band envelopes at the block's start and end, per channel.
        let (mut hf0, mut hf1, mut mid0, mut mid1, mut low0, mut low1) = ([0.0f32; 2], [0.0f32; 2], [0.0f32; 2], [0.0f32; 2], [0.0f32; 2], [0.0f32; 2]);
        for b in &self.bolts[..self.count] {
            let env = |t: f32| {
                let u = t - b.t;
                if u <= 0.0 {
                    0.0
                } else if u < b.att {
                    let a = u / b.att;
                    a * a
                } else {
                    (-(u - b.att) / b.dec).exp()
                }
            };
            if t0 < b.t - dt || t0 > b.t + b.att + 7.0 * b.dec {
                continue;
            }
            let (e0, e1) = (env(t0) * b.amp, env(t1) * b.amp);
            let (gl, gr) = balance(b.pan * width);
            for (c, g) in [gl, gr].into_iter().enumerate() {
                hf0[c] += e0 * b.hf * g;
                hf1[c] += e1 * b.hf * g;
                mid0[c] += e0 * b.mid * g;
                mid1[c] += e1 * b.mid * g;
                low0[c] += e0 * g;
                low1[c] += e1 * g;
            }
        }
        // The echoes of the lumps fill the gaps between them, so the roll never drops out.
        let follow = settle(dt, 2.5);
        let wash0 = self.wash;
        for (c, w) in self.wash.iter_mut().enumerate() {
            *w += (0.5 * (low0[c] + low1[c]) - *w) * follow;
        }
        let wash1 = self.wash;
        // Fade the very end so the event stops cleanly at its reported length.
        let fade = |t: f32| smooth01((self.end - t) / 1.5);
        let (fa, fb) = (fade(t0), fade(t1));
        let tune = self.pitch;
        let hz_low = self.rumble_hz * tune;
        self.hf.set(2200.0 * tune, 7000.0 * tune, 0.1, sr);
        self.mid.set(hz_low * 1.2, hz_low * 8.0, 0.2, sr);
        self.low.set(55.0, hz_low * 2.0, 0.3, sr);
        let far = hz_coef(self.far_hz * tune, sr);
        let phi = width * core::f32::consts::FRAC_PI_4;
        let (cm, cs) = (phi.cos(), phi.sin());
        let stereo = width > 0.0;
        let inv = 1.0 / n as f32;
        let snap_decay = (-1.0 / (0.012 * sr)).exp();
        let click_decay = (-1.0 / (0.0008 * sr)).exp();
        let tear_p = 280.0 / sr;
        let (level, post) = (self.level, self.post);
        let (crack_hf, crack_mid) = (self.crack_w * p.crack * 0.5, self.crack_w * p.crack * 0.7);
        let rumble = p.rumble * 1.4;
        for i in 0..n {
            let k = (i as f32 + 1.0) * inv;
            let t = t0 + k * dt;
            while self.next_stroke < self.stroke_count && t >= self.strokes[self.next_stroke] {
                // A return stroke: an N-wave snap and a fresh burst of tearing.
                let a = [1.0, 0.65, 0.5, 0.4, 0.35, 0.3][self.next_stroke];
                self.snap = self.snap.max(a);
                self.tear_env = self.tear_env.max(a);
                self.next_stroke += 1;
            }
            self.snap *= snap_decay;
            self.tear_env *= self.tear_decay;
            if self.tear_dust.tick(tear_p * (0.3 + self.tear_env)) > 0.0 {
                let u = self.tear_dust.rng().next_f32();
                self.tear = self.tear.max(u * u * self.tear_env);
            }
            self.tear *= click_decay;
            let fade = fa + (fb - fa) * k;
            let (w, ws) = (self.noise.white(), self.side.white());
            let (pk, pks) = (self.noise.pink(), if stereo { self.side.pink() } else { 0.0 });
            let (br, brs) = (self.brown[0].tick(w), if stereo { self.brown[1].tick(ws) } else { 0.0 });
            let crack = (self.snap + self.tear) * crack_hf;
            let (hf_m, hf_s) = self.hf.tick(w, ws, stereo);
            let (mid_m, mid_s) = self.mid.tick(pk, pks, stereo);
            let (low_m, low_s) = self.low.tick(br, brs, stereo);
            let snap_mid = self.snap * crack_mid;
            for c in 0..2 {
                let sign = if c == 0 { 1.0 } else { -1.0 };
                let hf = hf0[c] + (hf1[c] - hf0[c]) * k;
                let wash = wash0[c] + (wash1[c] - wash0[c]) * k;
                let mid = mid0[c] + (mid1[c] - mid0[c]) * k + 0.08 * wash;
                let low = low0[c] + (low1[c] - low0[c]) * k + 0.3 * wash;
                let x_hf = (hf_m * cm + sign * hf_s * cs) * (hf * 0.8 + crack);
                let x_mid = (mid_m * cm + sign * mid_s * cs) * (mid * 3.0 * rumble + snap_mid);
                let x_low = (low_m * cm + sign * low_s * cs) * low * rumble * 2.2;
                // Saturate the sum a little, as air and a recorder do: the roll gets the weight of an
                // explosion without the crack overshooting.
                let y = soft_clip((x_hf + x_mid + x_low) * level * 2.2) * 0.85 * post * fade;
                let y = self.far[c][0].lp(y, far);
                let y = self.far[c][1].lp(y, far);
                if c == 0 {
                    left[i] = y * p.gain;
                } else {
                    right[i] = y * p.gain;
                }
            }
        }
        self.t = t1;
        if self.t > self.end {
            self.t = -1.0;
        }
    }
}

impl Generator for Thunder {
    type P = ThunderParams;
    const NAME: &'static str = "thunder";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "Thunder: a crack and tearing then a long rumble close by, a low rolling swell far away. Every strike differs.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "power", default: 1.0, doc: "A small strike to a huge one: level, length and depth" },
        InputSpec { name: "distance", default: 0.3, doc: "0 = overhead (about 300 m: a crack, then rumble), 0.5 = about 2 km, 1 = about 15 km (a low roll, no crack)" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const ONE_SHOT: bool = true;
    const TRANSPOSES: bool = true;

    fn presets() -> Vec<(&'static str, ThunderParams)> {
        vec![
            ("Overhead", ThunderParams { crack: 1.0, strokes: 4.0, length: 12.0, ..Default::default() }),
            ("Rolling", ThunderParams { roll: 1.0, length: 18.0, rumble_hz: 115.0, crack: 0.5, ..Default::default() }),
            ("Heat lightning", ThunderParams { crack: 0.0, strokes: 1.0, length: 15.0, rumble_hz: 100.0, roll: 0.8, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Thunder {
            sr,
            rng: Rng::new(0x5F_0001),
            noise: Noise::new(0x5F_0002),
            side: Noise::new(0x5F_0003),
            brown: [Brown::default(); 2],
            bolts: [Bolt::default(); BOLTS],
            count: 0,
            t: -1.0,
            end: 0.0,
            strokes: [0.0; 6],
            stroke_count: 0,
            next_stroke: 0,
            snap: 0.0,
            tear: 0.0,
            tear_decay: 1.0,
            tear_env: 0.0,
            tear_dust: Dust::new(0x5F_0004),
            crack_w: 0.0,
            hf: Band::default(),
            mid: Band::default(),
            low: Band::default(),
            far: [[OnePole::default(); 2]; 2],
            far_hz: 16000.0,
            wash: [0.0; 2],
            level: 1.0,
            post: 1.0,
            rumble_hz: 110.0,
            pitch: 1.0,
        }
    }

    fn trigger(&mut self, x: &[f32], p: &ThunderParams) {
        let (power, d) = (x[0], x[1]);
        let v = p.variation * 2.0;
        let rng = &mut self.rng;
        // The nearest point of the channel: 300 m overhead .. 15 km away.
        let r0 = 0.3 * 50f32.powf(d);
        let dur = p.length * (0.7 + 0.6 * power) * (1.0 + 0.15 * v * rng.next_bipolar()) * (1.0 + 0.3 * d);
        // Close by the sound arrives at once; far off it is smeared by the long path.
        let rise = 0.004 + 1.2 * d * d;
        let fall = dur * 0.3;
        let mut end: f32 = 0.0;
        for k in 0..BOLTS {
            let u = rng.next_f32();
            // More of the channel is heard early (it is nearer), the rest trickles in.
            // The nearest point of the channel arrives first, at once.
            let t = if k == 0 { 0.0 } else { dur * u.powf(1.6 - 0.3 * d) };
            let shape = (1.0 - (-t.max(0.03) / rise.max(1e-3)).exp()) * (-t / fall).exp();
            // Lumps: some pieces of the channel point at the listener, some away.
            let g = rng.next_f32() + rng.next_f32() + rng.next_f32() - 1.5;
            let lump = (p.roll * 3.2 * g).exp();
            // Later arrivals came from farther up the channel: duller and more smeared.
            let r = r0 + 0.343 * t;
            // The first arrival of a near strike is the shock front: it hits at once.
            let att = if k == 0 { 0.003 + 0.4 * d } else { (0.01 + 0.03 * r.powf(0.7) + 0.04 * rng.next_f32()).min(0.9) };
            let dec = (0.15 + 0.05 * r + 0.5 * rng.next_f32()).min(1.2);
            self.bolts[k] = Bolt { t, amp: shape * lump, att, dec, hf: (-r / 0.35).exp(), mid: (-r / 12.0).exp(), pan: 0.8 * rng.next_bipolar() };
            end = end.max(t + att + 5.0 * dec);
        }
        // The loudest arrivals set the level, so lumps cannot run away.
        let peak = self.bolts.iter().map(|b| b.amp).fold(0.0f32, f32::max).max(1e-3);
        for b in self.bolts.iter_mut() {
            b.amp = (b.amp / peak).min(1.0);
        }
        // Overhead, the shock front is the loudest moment; far off it is lost in the roll.
        self.bolts[0].amp = self.bolts[0].amp.max(1.0 - 1.5 * d).min(1.0);
        self.count = BOLTS;
        // The first arrival of a near strike: one crack per return stroke, 40-120 ms apart.
        self.crack_w = (-r0 / 1.2).exp();
        self.stroke_count = 1 + (rng.next_f32() * p.strokes) as usize;
        let mut ts = 0.0;
        for k in 0..self.stroke_count.min(6) {
            self.strokes[k] = ts;
            ts += rng.range(0.04, 0.12);
        }
        self.next_stroke = 0;
        self.snap = 0.0;
        self.tear = 0.0;
        self.tear_env = 0.0;
        self.tear_decay = (-1.0 / (rng.range(0.35, 0.7) * self.sr)).exp();
        // Air absorbs the top end with distance: 16 kHz overhead, about 1 kHz at 15 km.
        self.far_hz = 16000.0 * (1500.0f32 / 16000.0).powf(d);
        self.rumble_hz = p.rumble_hz * (1.0 + 0.2 * v * rng.next_bipolar()) * (1.0 - 0.3 * d) * (1.2 - 0.3 * power);
        // Power drives the saturation a little and sets the level after it, so a weak strike is
        // quieter as well as less dense.
        self.level = 0.33 * (0.55 + 0.45 * power) * (1.0 - 0.35 * d * d) / (1.0 + d);
        self.post = 0.4 + 0.6 * power;
        self.end = end.min(Self::max_len(p) - 0.4);
        self.t = 0.0;
        for b in [&mut self.hf, &mut self.mid, &mut self.low] {
            b.reset();
        }
        self.far = [[OnePole::default(); 2]; 2];
        self.wash = [0.0; 2];
    }

    fn is_active(&self) -> bool {
        self.t >= 0.0
    }

    fn length(p: &ThunderParams) -> Option<f32> {
        Some(Self::max_len(p))
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        self.pitch = ratio;
    }

    fn block(&mut self, _x: &[f32], p: &ThunderParams, out: &mut [f32]) {
        let mut right = [0.0f32; BLOCK];
        let n = out.len();
        self.render(p, 0.0, out, &mut right[..n]);
    }

    fn block_stereo(&mut self, _x: &[f32], p: &ThunderParams, left: &mut [f32], right: &mut [f32]) {
        self.render(p, p.width, left, right);
    }
}
