//! Layered one-shot effects: explosions, weapons, impacts, pickups, boosts.
//!
//! An effect is a [`Recipe`]: up to eight [`Layer`]s (noise bursts with swept filters, tones
//! with pitch glides and FM, debris showers) that start together or staggered, summed into a
//! small reverb. One engine renders every recipe, so a new effect is a table, not new DSP.
//!
//! Every trigger re-rolls per-layer pitch, timing and level (`shape/variation`), because the
//! tenth identical explosion is what makes procedural audio sound cheap. The game supplies
//! `power` (small hit to full blast) and `distance` at the moment of the trigger.

use core::marker::PhantomData;

use crate::blocks::{hz_coef, mix_seed, OnePole, Reverb, BLOCK};
use crate::filter::{FilterMode, Svf};
use crate::math::{soft_clip, Rng, TAU};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::osc::{Oscillator, Waveform};
use crate::params::{exp, lin, GAIN, UNIT};

pub const MAX_LAYERS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Src {
    White,
    Pink,
    Tone(Waveform),
    /// A shower of short filtered clicks at random pitches: sparks, shrapnel, rubble.
    Debris,
}

/// One component of an effect. Times in seconds, frequencies in Hz.
#[derive(Clone, Copy, Debug)]
pub struct Layer {
    pub src: Src,
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    /// Time for the layer to fall 60 dB after the hold.
    pub decay: f32,
    pub level: f32,
    /// Frequency glides from `f0` to `f1` with time constant `glide` (0 = stay at `f0`). It is
    /// the oscillator pitch for tones, the filter cutoff for noise, the centre pitch for debris.
    pub f0: f32,
    pub f1: f32,
    pub glide: f32,
    pub mode: FilterMode,
    pub q: f32,
    /// Sine tones only: FM modulator ratio and index (the index follows the envelope).
    pub fm_ratio: f32,
    pub fm: f32,
    /// Debris only: clicks per second at full level, and their pitch spread in octaves.
    pub rate: f32,
    pub spread: f32,
    pub drive: f32,
    /// How strongly `shape/variation` re-rolls this layer on each trigger.
    pub vary: f32,
}

/// Neutral layer for struct-update syntax in recipe tables.
pub const LAYER: Layer = Layer {
    src: Src::White,
    delay: 0.0,
    attack: 0.0,
    hold: 0.01,
    decay: 0.2,
    level: 1.0,
    f0: 1000.0,
    f1: 1000.0,
    glide: 0.0,
    mode: FilterMode::LowPass,
    q: 0.2,
    fm_ratio: 1.0,
    fm: 0.0,
    rate: 0.0,
    spread: 0.0,
    drive: 0.0,
    vary: 1.0,
};

pub trait Recipe: Send + 'static {
    const NAME: &'static str;
    const DOC: &'static str;
    const LAYERS: &'static [Layer];
    /// Reverb send and decay time at the neutral `space/*` settings.
    const REVERB: f32;
    const REVERB_TIME: f32;
    fn presets() -> Vec<(&'static str, FxParams)> {
        Vec::new()
    }
}

model_params! {
    /// Macro controls shared by every one-shot effect. They bend the recipe rather than
    /// replace it: 0.5 (or 1.0 for size, 0 for pitch) is the effect as designed.
    FxParams / FxParamId {
        size: "shape/size" = 1.0, exp(0.25, 4.0);
        pitch: "shape/pitch_semitones" = 0.0, lin(-24.0, 24.0);
        brightness: "shape/brightness" = 0.5, UNIT;
        punch: "shape/punch" = 0.5, UNIT;
        variation: "shape/variation" = 0.5, UNIT;
        space: "space/amount" = 0.5, UNIT;
        tail: "space/tail" = 0.5, UNIT;
        gain: "master/gain" = 0.9, GAIN;
    }
}

#[derive(Clone, Copy)]
struct LayerState {
    on: bool,
    /// Seconds since this layer's start (negative while delayed).
    t: f32,
    pitch: f32,
    time: f32,
    level: f32,
    phase: f32,
    fm_phase: f32,
    burst: f32,
    next: usize,
}

const IDLE: LayerState = LayerState { on: false, t: 0.0, pitch: 1.0, time: 1.0, level: 1.0, phase: 0.0, fm_phase: 0.0, burst: 0.0, next: 0 };

/// Renders any [`Recipe`].
pub struct FxEngine {
    sr: f32,
    seed: u32,
    state: [LayerState; MAX_LAYERS],
    filter: [Svf; MAX_LAYERS],
    grains: [[Svf; 3]; MAX_LAYERS],
    osc: [Oscillator; MAX_LAYERS],
    noise: Noise,
    rng: Rng,
    reverb: Reverb,
    far: [OnePole; 2],
    /// Seconds of reverb tail left after the last layer ends.
    tail_left: f32,
    power: f32,
    distance: f32,
}

impl FxEngine {
    pub fn new(sr: f32, seed: u32) -> Self {
        FxEngine {
            sr,
            seed,
            state: [IDLE; MAX_LAYERS],
            filter: [Svf::default(); MAX_LAYERS],
            grains: [[Svf::default(); 3]; MAX_LAYERS],
            osc: core::array::from_fn(|k| Oscillator::new(mix_seed(seed + k as u32))),
            noise: Noise::new(mix_seed(seed ^ 0x5F5F)),
            rng: Rng::new(mix_seed(seed ^ 0xA1A1)),
            reverb: Reverb::new(sr),
            far: [OnePole::default(); 2],
            tail_left: 0.0,
            power: 1.0,
            distance: 0.0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.tail_left > 0.0 || self.state.iter().any(|s| s.on)
    }

    pub fn trigger(&mut self, layers: &[Layer], p: &FxParams, power: f32, distance: f32) {
        self.power = power;
        self.distance = distance;
        if p.variation <= 0.0 {
            // No variation means none at all: restart the noise so UI tones repeat exactly.
            self.noise = Noise::new(mix_seed(self.seed ^ 0x5F5F));
            self.rng = Rng::new(mix_seed(self.seed ^ 0xA1A1));
            self.reverb.clear();
        }
        for (k, (s, l)) in self.state.iter_mut().zip(layers).enumerate() {
            let v = p.variation * 2.0 * l.vary;
            *s = LayerState {
                on: true,
                t: -l.delay * p.size,
                pitch: (v * 0.16 * self.rng.next_bipolar()).exp2(),
                time: 1.0 + v * 0.22 * self.rng.next_bipolar(),
                level: 1.0 + v * 0.2 * self.rng.next_bipolar(),
                phase: 0.0,
                fm_phase: 0.0,
                burst: 0.0,
                next: 0,
            };
            self.filter[k].reset();
        }
        for s in self.state.iter_mut().skip(layers.len()) {
            s.on = false;
        }
        self.tail_left = 0.0;
    }

    /// Render one block, overwriting `out`.
    pub fn block(&mut self, layers: &[Layer], p: &FxParams, reverb: f32, reverb_time: f32, out: &mut [f32]) {
        let sr = self.sr;
        let n = out.len();
        let dt = 1.0 / sr;
        out.iter_mut().for_each(|s| *s = 0.0);
        // A bigger event is longer and lower; a weak hit is smaller, duller and quieter.
        let size = p.size * (0.7 + 0.3 * self.power);
        let tune = (p.pitch / 12.0).exp2() / size.sqrt();
        let bright = (2.0 * (p.brightness - 0.5) + 0.8 * (self.power - 1.0)).exp2();
        let mut any = false;
        for (k, l) in layers.iter().enumerate().take(MAX_LAYERS) {
            let s = &mut self.state[k];
            if !s.on {
                continue;
            }
            any = true;
            let scale = size * s.time;
            let (attack, hold, decay) = (l.attack * scale, l.hold * scale, (l.decay * scale).max(1e-3));
            let t_mid = s.t.max(0.0);
            let glide = if l.glide > 0.0 { (-t_mid / (l.glide * scale)).exp() } else { 1.0 };
            let f = ((l.f1 + (l.f0 - l.f1) * glide) * s.pitch * tune).clamp(18.0, sr * 0.45);
            let level = l.level * s.level;
            let drive = 1.0 + l.drive * (0.5 + p.punch) * 8.0;
            match l.src {
                Src::White | Src::Pink => self.filter[k].set(l.mode, (f * bright).min(sr * 0.45), l.q, sr),
                Src::Debris => {}
                Src::Tone(_) => {}
            }
            let burst_decay = (-1.0 / (0.003 * sr)).exp();
            for o in out.iter_mut() {
                s.t += dt;
                if s.t < 0.0 {
                    continue;
                }
                let env = if s.t < attack {
                    s.t / attack
                } else if s.t < attack + hold {
                    1.0
                } else {
                    (-6.9 * (s.t - attack - hold) / decay).exp()
                };
                let x = match l.src {
                    Src::White => self.filter[k].tick(self.noise.white()),
                    Src::Pink => self.filter[k].tick(self.noise.pink() * 3.0),
                    Src::Tone(Waveform::Sine) => {
                        s.phase = (s.phase + f / sr).fract();
                        s.fm_phase = (s.fm_phase + f * l.fm_ratio / sr).fract();
                        (TAU * (s.phase + l.fm * env * (s.fm_phase * TAU).sin() / TAU)).sin()
                    }
                    Src::Tone(wave) => self.osc[k].next(wave, f / sr, 0.5),
                    Src::Debris => {
                        if self.rng.next_f32() < l.rate * env / (sr * size.max(0.3)) {
                            s.burst = self.rng.range(0.4, 1.0);
                            s.next = (s.next + 1) % 3;
                            let hz = f * (self.rng.next_bipolar() * l.spread).exp2() * bright.sqrt();
                            self.grains[k][s.next].set(FilterMode::BandPass, hz.clamp(40.0, sr * 0.45), l.q, sr);
                        }
                        s.burst *= burst_decay;
                        let click = self.noise.white() * s.burst;
                        let mut y = 0.0;
                        for (g, grain) in self.grains[k].iter_mut().enumerate() {
                            y += grain.tick(if g == s.next { click } else { 0.0 });
                        }
                        y * 3.0
                    }
                };
                let x = if l.drive > 0.0 { soft_clip(x * drive) / (1.0 + 0.12 * drive) } else { x };
                *o += x * env * level;
            }
            if s.t > attack + hold + decay {
                s.on = false;
            }
        }
        let rt = reverb_time * (0.4 + 1.2 * p.tail) * (0.7 + 0.6 * size.min(2.0));
        if any {
            self.tail_left = rt;
        } else {
            self.tail_left = (self.tail_left - n as f32 * dt).max(0.0);
        }
        // Distance: duller, quieter, more room than source.
        // Air absorbs treble: the cutoff falls exponentially from 18 kHz to 400 Hz, 12 dB/oct.
        let far = hz_coef(18000.0 * (400.0f32 / 18000.0).powf(self.distance), sr);
        let send = (reverb * 2.0 * p.space * (1.0 + 2.0 * self.distance)).min(1.2);
        let dry = (0.25 + 0.75 * self.power) * (1.0 - 0.75 * self.distance) * p.gain;
        let master = 1.0 + p.punch * 2.0;
        for o in out.iter_mut() {
            let x = self.far[0].lp(soft_clip(*o * master) / (1.0 + 0.2 * master), far);
            let x = self.far[1].lp(x, far);
            *o = (x + self.reverb.tick(x, rt, 0.35 + 0.4 * self.distance) * send) * dry;
        }
    }
}

/// Adapter: any [`Recipe`] as a one-shot [`Generator`].
pub struct OneShot<R: Recipe> {
    engine: FxEngine,
    recipe: PhantomData<R>,
}

impl<R: Recipe> Generator for OneShot<R> {
    type P = FxParams;
    const NAME: &'static str = R::NAME;
    const CATEGORY: &'static str = "fx";
    const DOC: &'static str = R::DOC;
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "power", default: 1.0, doc: "Glancing hit to full blast: level, size and brightness" },
        InputSpec { name: "distance", default: 0.0, doc: "0 = on top of the listener, 1 = far off (duller, roomier)" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const ONE_SHOT: bool = true;

    fn presets() -> Vec<(&'static str, FxParams)> {
        R::presets()
    }

    fn new(sr: f32) -> Self {
        debug_assert!(R::LAYERS.len() <= MAX_LAYERS);
        let seed = R::NAME.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
        OneShot { engine: FxEngine::new(sr, seed), recipe: PhantomData }
    }

    fn trigger(&mut self, x: &[f32], p: &FxParams) {
        self.engine.trigger(R::LAYERS, p, x[0], x[1]);
    }

    fn is_active(&self) -> bool {
        self.engine.is_active()
    }

    fn block(&mut self, _x: &[f32], p: &FxParams, out: &mut [f32]) {
        debug_assert!(out.len() <= BLOCK);
        self.engine.block(R::LAYERS, p, R::REVERB, R::REVERB_TIME, out);
    }
}

// ---------------------------------------------------------------------------------------------
// Recipes
// ---------------------------------------------------------------------------------------------

use FilterMode::{BandPass, HighPass, LowPass};
use Waveform::{Saw, Sine, Triangle};

macro_rules! recipe {
    ($ty:ident, $name:literal, $doc:literal, reverb $rv:literal / $rt:literal, presets [$(($pn:literal, $pv:expr)),* $(,)?], [$($layer:expr),* $(,)?]) => {
        pub struct $ty;
        impl Recipe for $ty {
            const NAME: &'static str = $name;
            const DOC: &'static str = $doc;
            const LAYERS: &'static [Layer] = &[$($layer),*];
            const REVERB: f32 = $rv;
            const REVERB_TIME: f32 = $rt;
            fn presets() -> Vec<(&'static str, FxParams)> {
                vec![$(($pn, $pv)),*]
            }
        }
    };
}

fn fx(size: f32, pitch: f32, brightness: f32) -> FxParams {
    FxParams { size, pitch, brightness, ..FxParams::default() }
}

recipe!(Explosion, "explosion", "Layered blast: crack, swept body, sub drop, rumble and a debris tail.", reverb 0.35 / 1.6,
    presets [("Ship destroyed", fx(1.7, -3.0, 0.45)), ("Mine", fx(0.65, 4.0, 0.6)), ("Depth charge", FxParams { tail: 0.8, ..fx(1.4, -7.0, 0.25) })],
    [
        Layer { src: Src::White, mode: HighPass, f0: 1800.0, f1: 900.0, glide: 0.03, hold: 0.004, decay: 0.07, level: 1.0, drive: 0.5, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 3800.0, f1: 140.0, glide: 0.22, q: 0.25, attack: 0.002, hold: 0.03, decay: 1.1, level: 1.6, drive: 0.7, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 95.0, f1: 32.0, glide: 0.25, attack: 0.003, hold: 0.05, decay: 0.8, level: 0.9, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 300.0, f1: 80.0, glide: 0.5, delay: 0.03, attack: 0.05, hold: 0.2, decay: 1.4, level: 1.0, drive: 0.4, ..LAYER },
        Layer { src: Src::Debris, f0: 3000.0, f1: 1200.0, glide: 0.6, rate: 90.0, spread: 1.2, q: 0.6, delay: 0.06, attack: 0.02, hold: 0.1, decay: 1.3, level: 0.5, ..LAYER },
    ]);

recipe!(Rocket, "rocket", "Missile launch: ignition snap, a whoosh that falls away, thrust and hiss.", reverb 0.25 / 1.2,
    presets [("Heavy missile", fx(1.5, -5.0, 0.4)), ("Dart", fx(0.6, 5.0, 0.65))],
    [
        Layer { src: Src::White, mode: BandPass, f0: 500.0, f1: 3000.0, glide: 0.05, q: 0.4, hold: 0.01, decay: 0.12, level: 0.8, ..LAYER },
        Layer { src: Src::Pink, mode: BandPass, f0: 1600.0, f1: 350.0, glide: 0.5, q: 0.35, attack: 0.03, hold: 0.15, decay: 1.0, level: 1.8, drive: 0.4, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 140.0, f1: 60.0, glide: 0.6, attack: 0.02, hold: 0.1, decay: 0.7, level: 0.35, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 3000.0, attack: 0.01, hold: 0.05, decay: 0.5, level: 0.25, ..LAYER },
    ]);

recipe!(Plasma, "plasma", "Energy bolt: an FM zap that dives in pitch, sizzle and a thump.", reverb 0.2 / 0.9,
    presets [("Heavy cannon", fx(1.6, -9.0, 0.45)), ("Pulse rifle", fx(0.5, 5.0, 0.6))],
    [
        Layer { src: Src::Tone(Sine), f0: 2200.0, f1: 180.0, glide: 0.06, fm_ratio: 2.41, fm: 3.0, hold: 0.02, decay: 0.28, level: 0.8, ..LAYER },
        Layer { src: Src::White, mode: BandPass, f0: 5000.0, f1: 2500.0, glide: 0.1, q: 0.5, hold: 0.01, decay: 0.18, level: 0.4, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 160.0, f1: 50.0, glide: 0.05, hold: 0.01, decay: 0.15, level: 0.8, vary: 0.4, ..LAYER },
    ]);

recipe!(Cannon, "cannon", "One round of an auto-cannon; retrigger it for bursts.", reverb 0.12 / 0.5,
    presets [("Heavy", fx(1.5, -5.0, 0.4)), ("Rattle", fx(0.6, 4.0, 0.65))],
    [
        Layer { src: Src::White, mode: HighPass, f0: 2500.0, hold: 0.002, decay: 0.02, level: 0.8, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 2800.0, f1: 300.0, glide: 0.04, hold: 0.008, decay: 0.16, level: 2.8, drive: 0.8, ..LAYER },
        Layer { src: Src::Tone(Triangle), f0: 190.0, f1: 60.0, glide: 0.04, hold: 0.008, decay: 0.14, level: 1.0, vary: 0.5, ..LAYER },
    ]);

recipe!(Impact, "impact", "Hull collision: thud, crunch and the ring of struck metal (a different object every time).", reverb 0.3 / 1.0,
    presets [("Heavy slam", fx(1.6, -7.0, 0.4)), ("Glancing", FxParams { punch: 0.3, ..fx(0.6, 5.0, 0.6) })],
    [
        Layer { src: Src::Tone(Sine), f0: 130.0, f1: 45.0, glide: 0.04, hold: 0.01, decay: 0.25, level: 1.0, vary: 0.4, ..LAYER },
        Layer { src: Src::Pink, mode: BandPass, f0: 1400.0, f1: 500.0, glide: 0.05, q: 0.3, hold: 0.01, decay: 0.12, level: 1.6, drive: 0.8, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 410.0, fm_ratio: 1.41, fm: 0.5, hold: 0.0, decay: 0.5, level: 0.35, vary: 1.6, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1130.0, fm_ratio: 1.41, fm: 0.4, hold: 0.0, decay: 0.4, level: 0.25, vary: 1.6, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 2210.0, hold: 0.0, decay: 0.3, level: 0.18, vary: 1.6, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 3660.0, hold: 0.0, decay: 0.2, level: 0.12, vary: 1.6, ..LAYER },
    ]);

recipe!(ShieldHit, "shield_hit", "Shield absorbing a hit: a blooming FM tone, glassy shimmer and a soft thump.", reverb 0.4 / 1.4,
    presets [("Overload", fx(1.5, -5.0, 0.7))],
    [
        Layer { src: Src::Tone(Sine), f0: 320.0, f1: 760.0, glide: 0.12, fm_ratio: 1.5, fm: 1.2, attack: 0.005, hold: 0.05, decay: 0.6, level: 0.6, ..LAYER },
        Layer { src: Src::White, mode: BandPass, f0: 4200.0, f1: 6500.0, glide: 0.3, q: 0.8, attack: 0.01, hold: 0.05, decay: 0.5, level: 0.5, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 110.0, f1: 60.0, glide: 0.08, hold: 0.01, decay: 0.25, level: 0.6, vary: 0.4, ..LAYER },
    ]);

recipe!(Pickup, "pickup", "Weapon pad: three rising FM chimes with a sparkle on top.", reverb 0.25 / 0.8,
    presets [("Rare item", fx(1.4, 5.0, 0.65)), ("Energy cell", fx(0.8, -5.0, 0.4))],
    [
        Layer { src: Src::Tone(Sine), f0: 660.0, fm_ratio: 3.0, fm: 0.6, hold: 0.03, decay: 0.12, level: 0.5, vary: 0.15, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 990.0, fm_ratio: 3.0, fm: 0.6, delay: 0.07, hold: 0.03, decay: 0.12, level: 0.5, vary: 0.15, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1320.0, fm_ratio: 3.0, fm: 0.6, delay: 0.14, hold: 0.03, decay: 0.3, level: 0.5, vary: 0.15, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 6000.0, delay: 0.14, hold: 0.01, decay: 0.2, level: 0.12, ..LAYER },
    ]);

recipe!(Boost, "boost", "Speed pad: a rising whoosh and tone, with a kick as it hits.", reverb 0.3 / 1.2,
    presets [("Turbo", fx(1.5, -3.0, 0.6)), ("Nudge", fx(0.6, 4.0, 0.5))],
    [
        Layer { src: Src::Pink, mode: BandPass, f0: 250.0, f1: 4500.0, glide: 0.35, q: 0.6, attack: 0.25, hold: 0.05, decay: 0.5, level: 1.8, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 70.0, f1: 520.0, glide: 0.4, attack: 0.2, hold: 0.05, decay: 0.35, level: 0.3, vary: 0.4, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 120.0, f1: 40.0, glide: 0.06, hold: 0.01, decay: 0.3, level: 0.8, vary: 0.4, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 5000.0, attack: 0.3, hold: 0.05, decay: 0.6, level: 0.2, ..LAYER },
    ]);

recipe!(Quake, "quake", "Ground-shaking shockwave: a sinking sub, heavy rumble and falling rubble.", reverb 0.3 / 1.5,
    presets [("Tremor", FxParams { punch: 0.3, ..fx(0.6, 3.0, 0.4) })],
    [
        Layer { src: Src::Tone(Sine), f0: 70.0, f1: 24.0, glide: 0.8, attack: 0.05, hold: 0.4, decay: 1.2, level: 0.6, vary: 0.4, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 220.0, f1: 90.0, glide: 0.8, attack: 0.1, hold: 0.5, decay: 1.2, level: 0.9, drive: 0.6, ..LAYER },
        Layer { src: Src::Debris, f0: 900.0, f1: 300.0, glide: 1.0, rate: 60.0, spread: 1.5, q: 0.7, attack: 0.1, hold: 0.5, decay: 1.0, level: 0.6, ..LAYER },
    ]);

recipe!(Emp, "emp", "Electro bolt: crackling discharge over a diving FM tone and a mains buzz.", reverb 0.35 / 1.3,
    presets [("Short circuit", fx(0.5, 7.0, 0.7))],
    [
        Layer { src: Src::Debris, f0: 5000.0, f1: 2000.0, glide: 0.4, rate: 320.0, spread: 1.0, q: 0.5, hold: 0.2, decay: 0.8, level: 1.1, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 110.0, hold: 0.06, decay: 0.3, level: 0.16, vary: 0.3, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 900.0, f1: 90.0, glide: 0.15, fm_ratio: 0.5, fm: 2.0, hold: 0.02, decay: 0.45, level: 0.5, ..LAYER },
    ]);

recipe!(Airbrake, "airbrake", "Air brake deploying: a burst of hiss, a servo whine and the flap thunking out.", reverb 0.15 / 0.6,
    presets [],
    [
        Layer { src: Src::White, mode: HighPass, f0: 2200.0, f1: 3800.0, glide: 0.2, attack: 0.015, hold: 0.08, decay: 0.35, level: 0.6, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 95.0, f1: 60.0, glide: 0.04, hold: 0.005, decay: 0.08, level: 0.6, vary: 0.4, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 300.0, f1: 180.0, glide: 0.15, attack: 0.01, hold: 0.05, decay: 0.2, level: 0.1, ..LAYER },
    ]);

recipe!(Beep, "beep", "Race UI tone: countdown, lock-on, warning. Presets change its pitch and length.", reverb 0.08 / 0.4,
    presets [("Go", FxParams { variation: 0.0, ..fx(2.5, 12.0, 0.55) }), ("Lock-on", FxParams { variation: 0.0, ..fx(0.5, 7.0, 0.6) }), ("Warning", FxParams { variation: 0.0, ..fx(1.2, -5.0, 0.6) })],
    [
        Layer { src: Src::Tone(Sine), f0: 880.0, fm_ratio: 2.0, fm: 0.25, attack: 0.002, hold: 0.09, decay: 0.06, level: 0.6, vary: 0.05, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 4000.0, hold: 0.001, decay: 0.01, level: 0.15, ..LAYER },
    ]);

recipe!(Laser, "laser", "Laser shot: a bright tone diving several octaves in a few hundredths of a second, with a zing on top.", reverb 0.25 / 0.8,
    presets [("Heavy laser", fx(1.6, -7.0, 0.45)), ("Needle", fx(0.5, 7.0, 0.7)), ("Charged shot", FxParams { punch: 0.8, tail: 0.7, ..fx(2.2, -12.0, 0.55) })],
    [
        Layer { src: Src::Tone(Sine), f0: 4200.0, f1: 350.0, glide: 0.035, fm_ratio: 0.5, fm: 2.0, hold: 0.01, decay: 0.16, level: 0.8, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 2100.0, f1: 180.0, glide: 0.04, hold: 0.008, decay: 0.12, level: 0.25, ..LAYER },
        Layer { src: Src::White, mode: BandPass, f0: 6500.0, f1: 3000.0, glide: 0.05, q: 0.7, hold: 0.005, decay: 0.08, level: 0.35, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 180.0, f1: 70.0, glide: 0.03, hold: 0.004, decay: 0.06, level: 0.35, vary: 0.4, ..LAYER },
    ]);

recipe!(MineDrop, "mine_drop", "Mine released behind the ship: latch clunk, a short falling whoosh, then three arming beeps.", reverb 0.2 / 0.7,
    presets [("Heavy mine", fx(1.4, -5.0, 0.4)), ("Cluster", fx(0.6, 5.0, 0.6))],
    [
        Layer { src: Src::Tone(Sine), f0: 150.0, f1: 70.0, glide: 0.04, hold: 0.008, decay: 0.12, level: 0.9, vary: 0.4, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 2800.0, hold: 0.002, decay: 0.025, level: 0.5, ..LAYER },
        Layer { src: Src::Pink, mode: BandPass, f0: 1100.0, f1: 260.0, glide: 0.12, q: 0.5, attack: 0.02, hold: 0.05, decay: 0.3, level: 1.4, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1500.0, fm_ratio: 2.0, fm: 0.3, delay: 0.38, attack: 0.002, hold: 0.04, decay: 0.03, level: 0.4, vary: 0.05, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1500.0, fm_ratio: 2.0, fm: 0.3, delay: 0.52, attack: 0.002, hold: 0.04, decay: 0.03, level: 0.4, vary: 0.05, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 2000.0, fm_ratio: 2.0, fm: 0.3, delay: 0.66, attack: 0.002, hold: 0.09, decay: 0.08, level: 0.45, vary: 0.05, ..LAYER },
    ]);

recipe!(MineBlast, "mine_blast", "Mine going off: a hard crack, a short fat body and a spray of ringing metal shrapnel.", reverb 0.3 / 1.1,
    presets [("Proximity cluster", fx(0.6, 5.0, 0.65)), ("Magnetic mine", fx(1.5, -5.0, 0.4))],
    [
        Layer { src: Src::White, mode: HighPass, f0: 2500.0, hold: 0.003, decay: 0.05, level: 1.1, drive: 0.6, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 5000.0, f1: 250.0, glide: 0.09, q: 0.25, hold: 0.015, decay: 0.45, level: 1.6, drive: 0.8, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 140.0, f1: 45.0, glide: 0.1, hold: 0.02, decay: 0.35, level: 0.9, vary: 0.4, ..LAYER },
        Layer { src: Src::Debris, f0: 6000.0, f1: 2500.0, glide: 0.4, rate: 220.0, spread: 1.2, q: 0.85, delay: 0.02, hold: 0.05, decay: 0.7, level: 0.7, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1900.0, fm_ratio: 1.41, fm: 0.8, hold: 0.0, decay: 0.4, level: 0.12, vary: 1.5, ..LAYER },
    ]);

recipe!(LockOn, "lock_on", "Missile lock acquired: three quick pips, then a held tone.", reverb 0.05 / 0.3,
    presets [("Incoming!", FxParams { variation: 0.0, ..fx(0.7, 7.0, 0.65) })],
    [
        Layer { src: Src::Tone(Sine), f0: 1200.0, fm_ratio: 2.0, fm: 0.3, attack: 0.002, hold: 0.04, decay: 0.02, level: 0.45, vary: 0.02, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1200.0, fm_ratio: 2.0, fm: 0.3, delay: 0.09, attack: 0.002, hold: 0.04, decay: 0.02, level: 0.45, vary: 0.02, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1200.0, fm_ratio: 2.0, fm: 0.3, delay: 0.18, attack: 0.002, hold: 0.04, decay: 0.02, level: 0.45, vary: 0.02, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1800.0, fm_ratio: 2.0, fm: 0.3, delay: 0.3, attack: 0.002, hold: 0.28, decay: 0.08, level: 0.45, vary: 0.02, ..LAYER },
    ]);

recipe!(ShieldUp, "shield_up", "Shield powering up: a rising swell with glassy shimmer and sparkle.", reverb 0.4 / 1.4,
    presets [("Autopilot engage", fx(0.7, 7.0, 0.6))],
    [
        Layer { src: Src::Tone(Sine), f0: 180.0, f1: 520.0, glide: 0.35, fm_ratio: 2.0, fm: 1.0, attack: 0.25, hold: 0.1, decay: 0.5, level: 0.5, vary: 0.3, ..LAYER },
        Layer { src: Src::White, mode: BandPass, f0: 3000.0, f1: 7000.0, glide: 0.4, q: 0.85, attack: 0.3, hold: 0.1, decay: 0.6, level: 0.5, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 60.0, f1: 90.0, glide: 0.3, attack: 0.2, hold: 0.1, decay: 0.4, level: 0.5, vary: 0.3, ..LAYER },
        Layer { src: Src::Debris, f0: 7000.0, rate: 120.0, spread: 0.8, q: 0.8, attack: 0.3, hold: 0.1, decay: 0.5, level: 0.3, ..LAYER },
    ]);
