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
    /// Sine tones only: adds a second partial this many Hz above the first, so the pair beats
    /// (the slow shimmer of a bell). 0 = a single partial.
    pub beat: f32,
    /// Stereo spread of a noise or debris layer, 0..1, scaled by the effect's width. Noise
    /// bodies and tails get an independent side signal; debris clicks land at random positions.
    /// Set 0 for transients, which should stay centred.
    pub wide: f32,
    /// Stereo position of the layer, -1 (left) .. 1 (right), scaled by the effect's width.
    pub pan: f32,
    /// Sine tones only: cents of detune of a side partial, scaled by the effect's width. It
    /// is added to one channel and subtracted from the other, so the tone swirls between the
    /// speakers while the mono fold-down stays exactly the mono tone. 0 = centred.
    pub chorus: f32,
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
    beat: 0.0,
    wide: 1.0,
    pan: 0.0,
    chorus: 0.0,
};

pub trait Recipe: Send + 'static {
    const NAME: &'static str;
    const DOC: &'static str;
    const LAYERS: &'static [Layer];
    /// Reverb send and decay time at the neutral `space/*` settings.
    const REVERB: f32;
    const REVERB_TIME: f32;
    /// Stereo width of the effect as designed (at `space/width` = 0.5): wide for explosions,
    /// narrow for UI tones.
    const WIDTH: f32;
    fn presets() -> Vec<(&'static str, FxParams)> {
        Vec::new()
    }
}

model_params! {
    /// Macro controls shared by every one-shot effect. They bend the recipe rather than
    /// replace it: 0.5 (or 1.0 for size, 0 for pitch) is the effect as designed.
    /// `space/width` 0 is mono (sample-identical to the mono render), 0.5 the designed stereo
    /// image, 1 twice as wide (capped at fully decorrelated).
    FxParams / FxParamId {
        size: "shape/size" = 1.0, exp(0.25, 4.0);
        pitch: "shape/pitch_semitones" = 0.0, lin(-24.0, 24.0);
        brightness: "shape/brightness" = 0.5, UNIT;
        punch: "shape/punch" = 0.5, UNIT;
        variation: "shape/variation" = 0.5, UNIT;
        space: "space/amount" = 0.5, UNIT;
        tail: "space/tail" = 0.5, UNIT;
        width: "space/width" = 0.5, UNIT;
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
    beat_phase: f32,
    side_phase: f32,
    burst: f32,
    next: usize,
    /// Left/right gains of the three debris grains.
    grain_pan: [(f32, f32); 3],
}

const IDLE: LayerState = LayerState {
    on: false,
    t: 0.0,
    pitch: 1.0,
    time: 1.0,
    level: 1.0,
    phase: 0.0,
    fm_phase: 0.0,
    beat_phase: 0.0,
    side_phase: 0.25,
    burst: 0.0,
    next: 0,
    grain_pan: [(1.0, 1.0); 3],
};

/// Balance-law pan: centre is (1, 1), so a width of zero leaves the mono signal untouched.
#[inline]
fn balance(pan: f32) -> (f32, f32) {
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}

/// How a block is placed in space: the recipe's reverb and the effective stereo width.
#[derive(Clone, Copy, Debug)]
pub struct Space {
    /// Reverb send at the neutral `space/amount`.
    pub reverb: f32,
    /// Reverb decay time at the neutral `space/tail`.
    pub reverb_time: f32,
    /// Effective stereo width 0..1; at 0 the right channel is a copy of the left and none of
    /// the stereo work runs.
    pub width: f32,
}

/// Renders any [`Recipe`].
///
/// Stereo is mid/side: everything the mono render produces is the mid signal, and width only
/// adds to it (an independent side noise per wide layer, grain and layer positions, the
/// reverb's side output). Those extras draw from their own random streams, so the mid signal,
/// and with it the mono render, is the same sound at any width.
pub struct FxEngine {
    sr: f32,
    seed: u32,
    state: [LayerState; MAX_LAYERS],
    filter: [Svf; MAX_LAYERS],
    side_filter: [Svf; MAX_LAYERS],
    grains: [[Svf; 3]; MAX_LAYERS],
    osc: [Oscillator; MAX_LAYERS],
    noise: Noise,
    side_noise: Noise,
    rng: Rng,
    pan_rng: Rng,
    reverb: Reverb,
    far: [[OnePole; 2]; 2],
    /// Seconds of reverb tail left after the last layer ends.
    tail_left: f32,
    power: f32,
    distance: f32,
    pitch_ratio: f32,
}

impl FxEngine {
    pub fn new(sr: f32, seed: u32) -> Self {
        FxEngine {
            sr,
            seed,
            state: [IDLE; MAX_LAYERS],
            filter: [Svf::default(); MAX_LAYERS],
            side_filter: [Svf::default(); MAX_LAYERS],
            grains: [[Svf::default(); 3]; MAX_LAYERS],
            osc: core::array::from_fn(|k| Oscillator::new(mix_seed(seed + k as u32))),
            noise: Noise::new(mix_seed(seed ^ 0x5F5F)),
            side_noise: Noise::new(mix_seed(seed ^ 0x51DE)),
            rng: Rng::new(mix_seed(seed ^ 0xA1A1)),
            pan_rng: Rng::new(mix_seed(seed ^ 0x9A17)),
            reverb: Reverb::new(sr),
            far: [[OnePole::default(); 2]; 2],
            tail_left: 0.0,
            power: 1.0,
            distance: 0.0,
            pitch_ratio: 1.0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.tail_left > 0.0 || self.state.iter().any(|s| s.on)
    }

    pub fn set_pitch_ratio(&mut self, ratio: f32) {
        self.pitch_ratio = ratio;
    }

    /// Reverb decay time for these params (also the length of the tail after the last layer).
    fn reverb_time(p: &FxParams, reverb_time: f32, size: f32) -> f32 {
        reverb_time * (0.4 + 1.2 * p.tail) * (0.7 + 0.6 * size.min(2.0))
    }

    /// Upper bound on the length of one trigger: the latest layer end at full power and the
    /// slowest variation roll, the reverb tail, and the 0.25 s the peak meter needs to settle.
    pub fn length(layers: &[Layer], p: &FxParams, reverb_time: f32) -> f32 {
        let slowest = |l: &Layer| 1.0 + p.variation * 2.0 * l.vary * 0.22;
        let end = layers.iter().map(|l| p.size * (l.delay + (l.attack + l.hold + l.decay) * slowest(l))).fold(0.0, f32::max);
        end + Self::reverb_time(p, reverb_time, p.size) + 0.3
    }

    pub fn trigger(&mut self, layers: &[Layer], p: &FxParams, power: f32, distance: f32) {
        self.power = power;
        self.distance = distance;
        if p.variation <= 0.0 {
            // No variation means none at all: restart the noise so UI tones repeat exactly.
            self.noise = Noise::new(mix_seed(self.seed ^ 0x5F5F));
            self.side_noise = Noise::new(mix_seed(self.seed ^ 0x51DE));
            self.rng = Rng::new(mix_seed(self.seed ^ 0xA1A1));
            self.pan_rng = Rng::new(mix_seed(self.seed ^ 0x9A17));
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
                ..IDLE
            };
            self.filter[k].reset();
            self.side_filter[k].reset();
        }
        for s in self.state.iter_mut().skip(layers.len()) {
            s.on = false;
        }
        self.tail_left = 0.0;
    }

    /// Render one block, overwriting `left` and `right`.
    pub fn block(&mut self, layers: &[Layer], p: &FxParams, space: Space, left: &mut [f32], right: &mut [f32]) {
        let Space { reverb, reverb_time, width } = space;
        let sr = self.sr;
        let n = left.len();
        let dt = 1.0 / sr;
        let stereo = width > 0.0;
        left.iter_mut().chain(right.iter_mut()).for_each(|s| *s = 0.0);
        // A bigger event is longer and lower; a weak hit is smaller, duller and quieter.
        let size = p.size * (0.7 + 0.3 * self.power);
        let tune = (p.pitch / 12.0).exp2() * self.pitch_ratio / size.sqrt();
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
            let drive_norm = 1.0 / (1.0 + 0.12 * drive);
            let is_noise = matches!(l.src, Src::White | Src::Pink);
            // Noise bodies: mid is the mono signal, side an independent noise through its own
            // filter. phi = 45 degrees makes the channels fully uncorrelated at equal power.
            let phi = width * l.wide * core::f32::consts::FRAC_PI_4;
            let (mid_gain, side_gain) = if stereo && is_noise { (phi.cos(), phi.sin()) } else { (1.0, 0.0) };
            let wide_noise = side_gain > 0.0;
            if is_noise {
                let cutoff = (f * bright).min(sr * 0.45);
                self.filter[k].set(l.mode, cutoff, l.q, sr);
                if wide_noise {
                    self.side_filter[k].set(l.mode, cutoff, l.q, sr);
                }
            }
            let (pan_l, pan_r) = balance((l.pan * width).clamp(-1.0, 1.0));
            let beat_inc = if l.beat > 0.0 { (f + l.beat * tune) / sr } else { 0.0 };
            let swirl = if stereo && l.chorus > 0.0 { 0.8 * width } else { 0.0 };
            let side_inc = f * (l.chorus / 1200.0).exp2() / sr;
            let burst_decay = (-1.0 / (0.003 * sr)).exp();
            for (ol, or) in left.iter_mut().zip(right.iter_mut()) {
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
                let gain = env * level;
                let (mut xl, mut xr);
                match l.src {
                    Src::White | Src::Pink => {
                        let pink = l.src == Src::Pink;
                        let m = self.filter[k].tick(if pink { self.noise.pink() * 3.0 } else { self.noise.white() });
                        if wide_noise {
                            let side = self.side_filter[k].tick(if pink { self.side_noise.pink() * 3.0 } else { self.side_noise.white() });
                            (xl, xr) = (m * mid_gain + side * side_gain, m * mid_gain - side * side_gain);
                        } else {
                            (xl, xr) = (m, m);
                        }
                    }
                    Src::Tone(Waveform::Sine) => {
                        s.phase = (s.phase + f / sr).fract();
                        s.fm_phase = (s.fm_phase + f * l.fm_ratio / sr).fract();
                        let mut x = (TAU * (s.phase + l.fm * env * (s.fm_phase * TAU).sin() / TAU)).sin();
                        if l.beat > 0.0 {
                            s.beat_phase = (s.beat_phase + beat_inc).fract();
                            // Partial-depth beating: a shimmer, not a tremolo.
                            x = 0.7 * x + 0.3 * (TAU * s.beat_phase).sin();
                        }
                        if swirl > 0.0 {
                            s.side_phase = (s.side_phase + side_inc).fract();
                            let side = swirl * (TAU * s.side_phase).sin();
                            (xl, xr) = (x + side, x - side);
                        } else {
                            (xl, xr) = (x, x);
                        }
                    }
                    Src::Tone(wave) => {
                        let x = self.osc[k].next(wave, f / sr, 0.5);
                        (xl, xr) = (x, x);
                    }
                    Src::Debris => {
                        if self.rng.next_f32() < l.rate * env / (sr * size.max(0.3)) {
                            s.burst = self.rng.range(0.4, 1.0);
                            s.next = (s.next + 1) % 3;
                            let hz = f * (self.rng.next_bipolar() * l.spread).exp2() * bright.sqrt();
                            self.grains[k][s.next].set(FilterMode::BandPass, hz.clamp(40.0, sr * 0.45), l.q, sr);
                            if stereo {
                                // Each piece of debris lands somewhere else.
                                s.grain_pan[s.next] = balance(self.pan_rng.next_bipolar() * width * l.wide);
                            }
                        }
                        s.burst *= burst_decay;
                        let click = self.noise.white() * s.burst;
                        (xl, xr) = (0.0, 0.0);
                        for (g, grain) in self.grains[k].iter_mut().enumerate() {
                            let y = grain.tick(if g == s.next { click } else { 0.0 }) * 3.0;
                            xl += y * s.grain_pan[g].0;
                            xr += y * s.grain_pan[g].1;
                        }
                    }
                }
                if l.drive > 0.0 {
                    xl = soft_clip(xl * drive) * drive_norm;
                    if stereo {
                        xr = soft_clip(xr * drive) * drive_norm;
                    }
                }
                *ol += xl * gain * pan_l;
                if stereo {
                    *or += xr * gain * pan_r;
                }
            }
            if s.t > attack + hold + decay {
                s.on = false;
            }
        }
        let rt = Self::reverb_time(p, reverb_time, size);
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
        let master_norm = 1.0 / (1.0 + 0.2 * master);
        let damping = 0.35 + 0.4 * self.distance;
        if !stereo {
            for (ol, or) in left.iter_mut().zip(right.iter_mut()) {
                let x = self.far[0][0].lp(soft_clip(*ol * master) * master_norm, far);
                let x = self.far[0][1].lp(x, far);
                *ol = (x + self.reverb.tick(x, rt, damping) * send) * dry;
                *or = *ol;
            }
            return;
        }
        for (ol, or) in left.iter_mut().zip(right.iter_mut()) {
            let xl = self.far[0][0].lp(soft_clip(*ol * master) * master_norm, far);
            let xl = self.far[0][1].lp(xl, far);
            let xr = self.far[1][0].lp(soft_clip(*or * master) * master_norm, far);
            let xr = self.far[1][1].lp(xr, far);
            let (mid, side) = self.reverb.tick_stereo(0.5 * (xl + xr), rt, damping);
            *ol = (xl + (mid + side * width) * send) * dry;
            *or = (xr + (mid - side * width) * send) * dry;
        }
    }
}

/// Adapter: any [`Recipe`] as a one-shot [`Generator`].
pub struct OneShot<R: Recipe> {
    engine: FxEngine,
    scratch: [f32; BLOCK],
    recipe: PhantomData<R>,
}

impl<R: Recipe> OneShot<R> {
    /// `space/width` 0.5 is the recipe's designed width; 1.0 doubles it, capped at fully wide.
    fn width(p: &FxParams) -> f32 {
        (R::WIDTH * 2.0 * p.width).clamp(0.0, 1.0)
    }

    fn space(width: f32) -> Space {
        Space { reverb: R::REVERB, reverb_time: R::REVERB_TIME, width }
    }
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
        OneShot { engine: FxEngine::new(sr, seed), scratch: [0.0; BLOCK], recipe: PhantomData }
    }

    fn trigger(&mut self, x: &[f32], p: &FxParams) {
        self.engine.trigger(R::LAYERS, p, x[0], x[1]);
    }

    fn is_active(&self) -> bool {
        self.engine.is_active()
    }

    fn length(p: &FxParams) -> Option<f32> {
        Some(FxEngine::length(R::LAYERS, p, R::REVERB_TIME))
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        self.engine.set_pitch_ratio(ratio);
    }

    /// The mono render is the effect at width zero, not a fold-down of the stereo one.
    fn block(&mut self, _x: &[f32], p: &FxParams, out: &mut [f32]) {
        let n = out.len();
        self.engine.block(R::LAYERS, p, Self::space(0.0), out, &mut self.scratch[..n]);
    }

    fn block_stereo(&mut self, _x: &[f32], p: &FxParams, left: &mut [f32], right: &mut [f32]) {
        self.engine.block(R::LAYERS, p, Self::space(Self::width(p)), left, right);
    }
}

// ---------------------------------------------------------------------------------------------
// Recipes
// ---------------------------------------------------------------------------------------------

use FilterMode::{BandPass, HighPass, LowPass};
use Waveform::{Saw, Sine, Triangle};

macro_rules! recipe {
    ($ty:ident, $name:literal, $doc:literal, reverb $rv:literal / $rt:literal, width $w:literal, presets [$(($pn:literal, $pv:expr)),* $(,)?], [$($layer:expr),* $(,)?]) => {
        pub struct $ty;
        impl Recipe for $ty {
            const NAME: &'static str = $name;
            const DOC: &'static str = $doc;
            const LAYERS: &'static [Layer] = &[$($layer),*];
            const REVERB: f32 = $rv;
            const REVERB_TIME: f32 = $rt;
            const WIDTH: f32 = $w;
            fn presets() -> Vec<(&'static str, FxParams)> {
                vec![$(($pn, $pv)),*]
            }
        }
    };
}

fn fx(size: f32, pitch: f32, brightness: f32) -> FxParams {
    FxParams { size, pitch, brightness, ..FxParams::default() }
}

recipe!(Explosion, "explosion", "Layered blast: crack, swept body, sub drop, rumble and a debris tail.", reverb 0.35 / 1.6, width 0.9,
    presets [("Ship destroyed", fx(1.7, -3.0, 0.45)), ("Mine", fx(0.65, 4.0, 0.6)), ("Depth charge", FxParams { tail: 0.8, ..fx(1.4, -7.0, 0.25) })],
    [
        Layer { src: Src::White, mode: HighPass, f0: 1800.0, f1: 900.0, glide: 0.03, hold: 0.004, decay: 0.07, level: 1.0, drive: 0.5, wide: 0.0, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 3800.0, f1: 140.0, glide: 0.22, q: 0.25, attack: 0.002, hold: 0.03, decay: 1.1, level: 1.6, drive: 0.7, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 95.0, f1: 32.0, glide: 0.25, attack: 0.003, hold: 0.05, decay: 0.8, level: 0.9, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 300.0, f1: 80.0, glide: 0.5, delay: 0.03, attack: 0.05, hold: 0.2, decay: 1.4, level: 1.0, drive: 0.4, wide: 0.4, ..LAYER },
        Layer { src: Src::Debris, f0: 3000.0, f1: 1200.0, glide: 0.6, rate: 90.0, spread: 1.2, q: 0.6, delay: 0.06, attack: 0.02, hold: 0.1, decay: 1.3, level: 0.5, ..LAYER },
    ]);

recipe!(Rocket, "rocket", "Missile launch: ignition snap, a whoosh that falls away, thrust and hiss.", reverb 0.25 / 1.2, width 0.7,
    presets [("Heavy missile", fx(1.5, -5.0, 0.4)), ("Dart", fx(0.6, 5.0, 0.65))],
    [
        Layer { src: Src::White, mode: BandPass, f0: 500.0, f1: 3000.0, glide: 0.05, q: 0.4, hold: 0.01, decay: 0.12, level: 0.8, wide: 0.0, ..LAYER },
        Layer { src: Src::Pink, mode: BandPass, f0: 1600.0, f1: 350.0, glide: 0.5, q: 0.35, attack: 0.03, hold: 0.15, decay: 1.0, level: 1.8, drive: 0.4, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 140.0, f1: 60.0, glide: 0.6, attack: 0.02, hold: 0.1, decay: 0.7, level: 0.35, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 3000.0, attack: 0.01, hold: 0.05, decay: 0.5, level: 0.25, ..LAYER },
    ]);

recipe!(Plasma, "plasma", "Energy bolt: an FM zap that dives in pitch, sizzle and a thump.", reverb 0.2 / 0.9, width 0.45,
    presets [("Heavy cannon", fx(1.6, -9.0, 0.45)), ("Pulse rifle", fx(0.5, 5.0, 0.6))],
    [
        Layer { src: Src::Tone(Sine), f0: 2200.0, f1: 180.0, glide: 0.06, fm_ratio: 2.41, fm: 3.0, hold: 0.02, decay: 0.28, level: 0.8, ..LAYER },
        Layer { src: Src::White, mode: BandPass, f0: 5000.0, f1: 2500.0, glide: 0.1, q: 0.5, hold: 0.01, decay: 0.18, level: 0.4, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 160.0, f1: 50.0, glide: 0.05, hold: 0.01, decay: 0.15, level: 0.8, vary: 0.4, ..LAYER },
    ]);

recipe!(Cannon, "cannon", "One round of an auto-cannon; retrigger it for bursts.", reverb 0.12 / 0.5, width 0.35,
    presets [("Heavy", fx(1.5, -5.0, 0.4)), ("Rattle", fx(0.6, 4.0, 0.65))],
    [
        Layer { src: Src::White, mode: HighPass, f0: 2500.0, hold: 0.002, decay: 0.02, level: 0.8, wide: 0.0, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 2800.0, f1: 300.0, glide: 0.04, hold: 0.008, decay: 0.16, level: 2.8, drive: 0.8, wide: 0.6, ..LAYER },
        Layer { src: Src::Tone(Triangle), f0: 190.0, f1: 60.0, glide: 0.04, hold: 0.008, decay: 0.14, level: 1.0, vary: 0.5, ..LAYER },
    ]);

recipe!(Impact, "impact", "Hull collision: thud, crunch and the ring of struck metal (a different object every time).", reverb 0.3 / 1.0, width 0.85,
    presets [("Heavy slam", fx(1.6, -7.0, 0.4)), ("Glancing", FxParams { punch: 0.3, ..fx(0.6, 5.0, 0.6) })],
    [
        Layer { src: Src::Tone(Sine), f0: 130.0, f1: 45.0, glide: 0.04, hold: 0.01, decay: 0.25, level: 1.0, vary: 0.4, ..LAYER },
        Layer { src: Src::Pink, mode: BandPass, f0: 1400.0, f1: 500.0, glide: 0.05, q: 0.3, hold: 0.01, decay: 0.12, level: 1.6, drive: 0.8, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 410.0, fm_ratio: 1.41, fm: 0.5, hold: 0.0, decay: 0.5, level: 0.35, vary: 1.6, pan: -0.5, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1130.0, fm_ratio: 1.41, fm: 0.4, hold: 0.0, decay: 0.4, level: 0.25, vary: 1.6, pan: 0.5, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 2210.0, hold: 0.0, decay: 0.3, level: 0.18, vary: 1.6, pan: -0.3, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 3660.0, hold: 0.0, decay: 0.2, level: 0.12, vary: 1.6, pan: 0.3, ..LAYER },
    ]);

recipe!(ShieldHit, "shield_hit", "Shield absorbing a hit: a blooming FM tone, glassy shimmer and a soft thump.", reverb 0.4 / 1.4, width 0.8,
    presets [("Overload", fx(1.5, -5.0, 0.7))],
    [
        Layer { src: Src::Tone(Sine), f0: 320.0, f1: 760.0, glide: 0.12, fm_ratio: 1.5, fm: 1.2, attack: 0.005, hold: 0.05, decay: 0.6, level: 0.6, chorus: 7.0, ..LAYER },
        Layer { src: Src::White, mode: BandPass, f0: 4200.0, f1: 6500.0, glide: 0.3, q: 0.8, attack: 0.01, hold: 0.05, decay: 0.5, level: 0.5, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 110.0, f1: 60.0, glide: 0.08, hold: 0.01, decay: 0.25, level: 0.6, vary: 0.4, ..LAYER },
    ]);

recipe!(Pickup, "pickup", "Weapon pad: three rising FM chimes with a sparkle on top.", reverb 0.25 / 0.8, width 0.25,
    presets [("Rare item", fx(1.4, 5.0, 0.65)), ("Energy cell", fx(0.8, -5.0, 0.4))],
    [
        Layer { src: Src::Tone(Sine), f0: 660.0, fm_ratio: 3.0, fm: 0.6, hold: 0.03, decay: 0.12, level: 0.5, vary: 0.15, pan: -0.6, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 990.0, fm_ratio: 3.0, fm: 0.6, delay: 0.07, hold: 0.03, decay: 0.12, level: 0.5, vary: 0.15, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1320.0, fm_ratio: 3.0, fm: 0.6, delay: 0.14, hold: 0.03, decay: 0.3, level: 0.5, vary: 0.15, pan: 0.6, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 6000.0, delay: 0.14, hold: 0.01, decay: 0.2, level: 0.12, ..LAYER },
    ]);

recipe!(Boost, "boost", "Speed pad: a rising whoosh and tone, with a kick as it hits.", reverb 0.3 / 1.2, width 0.85,
    presets [("Turbo", fx(1.5, -3.0, 0.6)), ("Nudge", fx(0.6, 4.0, 0.5))],
    [
        Layer { src: Src::Pink, mode: BandPass, f0: 250.0, f1: 4500.0, glide: 0.35, q: 0.6, attack: 0.25, hold: 0.05, decay: 0.5, level: 1.8, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 70.0, f1: 520.0, glide: 0.4, attack: 0.2, hold: 0.05, decay: 0.35, level: 0.3, vary: 0.4, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 120.0, f1: 40.0, glide: 0.06, hold: 0.01, decay: 0.3, level: 0.8, vary: 0.4, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 5000.0, attack: 0.3, hold: 0.05, decay: 0.6, level: 0.2, ..LAYER },
    ]);

recipe!(Quake, "quake", "Ground-shaking shockwave: a sinking sub, heavy rumble and falling rubble.", reverb 0.3 / 1.5, width 0.8,
    presets [("Tremor", FxParams { punch: 0.3, ..fx(0.6, 3.0, 0.4) })],
    [
        Layer { src: Src::Tone(Sine), f0: 70.0, f1: 24.0, glide: 0.8, attack: 0.05, hold: 0.4, decay: 1.2, level: 0.6, vary: 0.4, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 220.0, f1: 90.0, glide: 0.8, attack: 0.1, hold: 0.5, decay: 1.2, level: 0.9, drive: 0.6, wide: 0.5, ..LAYER },
        Layer { src: Src::Debris, f0: 900.0, f1: 300.0, glide: 1.0, rate: 60.0, spread: 1.5, q: 0.7, attack: 0.1, hold: 0.5, decay: 1.0, level: 0.6, ..LAYER },
    ]);

recipe!(Emp, "emp", "Electro bolt: crackling discharge over a diving FM tone and a mains buzz.", reverb 0.35 / 1.3, width 0.75,
    presets [("Short circuit", fx(0.5, 7.0, 0.7))],
    [
        Layer { src: Src::Debris, f0: 5000.0, f1: 2000.0, glide: 0.4, rate: 320.0, spread: 1.0, q: 0.5, hold: 0.2, decay: 0.8, level: 1.1, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 110.0, hold: 0.06, decay: 0.3, level: 0.16, vary: 0.3, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 900.0, f1: 90.0, glide: 0.15, fm_ratio: 0.5, fm: 2.0, hold: 0.02, decay: 0.45, level: 0.5, ..LAYER },
    ]);

recipe!(Airbrake, "airbrake", "Air brake deploying: a burst of hiss, a servo whine and the flap thunking out.", reverb 0.15 / 0.6, width 0.5,
    presets [],
    [
        Layer { src: Src::White, mode: HighPass, f0: 2200.0, f1: 3800.0, glide: 0.2, attack: 0.015, hold: 0.08, decay: 0.35, level: 0.6, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 95.0, f1: 60.0, glide: 0.04, hold: 0.005, decay: 0.08, level: 0.6, vary: 0.4, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 300.0, f1: 180.0, glide: 0.15, attack: 0.01, hold: 0.05, decay: 0.2, level: 0.1, ..LAYER },
    ]);

recipe!(Beep, "beep", "Race UI tone: countdown, lock-on, warning. Presets change its pitch and length.", reverb 0.08 / 0.4, width 0.12,
    presets [("Go", FxParams { variation: 0.0, ..fx(2.5, 12.0, 0.55) }), ("Lock-on", FxParams { variation: 0.0, ..fx(0.5, 7.0, 0.6) }), ("Warning", FxParams { variation: 0.0, ..fx(1.2, -5.0, 0.6) })],
    [
        Layer { src: Src::Tone(Sine), f0: 880.0, fm_ratio: 2.0, fm: 0.25, attack: 0.002, hold: 0.09, decay: 0.06, level: 0.6, vary: 0.05, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 4000.0, hold: 0.001, decay: 0.01, level: 0.15, wide: 0.0, ..LAYER },
    ]);

recipe!(Laser, "laser", "Laser shot: a bright tone diving several octaves in a few hundredths of a second, with a zing on top.", reverb 0.25 / 0.8, width 0.4,
    presets [("Heavy laser", fx(1.6, -7.0, 0.45)), ("Needle", fx(0.5, 7.0, 0.7)), ("Charged shot", FxParams { punch: 0.8, tail: 0.7, ..fx(2.2, -12.0, 0.55) })],
    [
        Layer { src: Src::Tone(Sine), f0: 4200.0, f1: 350.0, glide: 0.035, fm_ratio: 0.5, fm: 2.0, hold: 0.01, decay: 0.16, level: 0.8, ..LAYER },
        Layer { src: Src::Tone(Saw), f0: 2100.0, f1: 180.0, glide: 0.04, hold: 0.008, decay: 0.12, level: 0.25, ..LAYER },
        Layer { src: Src::White, mode: BandPass, f0: 6500.0, f1: 3000.0, glide: 0.05, q: 0.7, hold: 0.005, decay: 0.08, level: 0.35, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 180.0, f1: 70.0, glide: 0.03, hold: 0.004, decay: 0.06, level: 0.35, vary: 0.4, ..LAYER },
    ]);

recipe!(MineDrop, "mine_drop", "Mine released behind the ship: latch clunk, a short falling whoosh, then three arming beeps.", reverb 0.2 / 0.7, width 0.4,
    presets [("Heavy mine", fx(1.4, -5.0, 0.4)), ("Cluster", fx(0.6, 5.0, 0.6))],
    [
        Layer { src: Src::Tone(Sine), f0: 150.0, f1: 70.0, glide: 0.04, hold: 0.008, decay: 0.12, level: 0.9, vary: 0.4, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 2800.0, hold: 0.002, decay: 0.025, level: 0.5, wide: 0.0, ..LAYER },
        Layer { src: Src::Pink, mode: BandPass, f0: 1100.0, f1: 260.0, glide: 0.12, q: 0.5, attack: 0.02, hold: 0.05, decay: 0.3, level: 1.4, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1500.0, fm_ratio: 2.0, fm: 0.3, delay: 0.38, attack: 0.002, hold: 0.04, decay: 0.03, level: 0.4, vary: 0.05, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1500.0, fm_ratio: 2.0, fm: 0.3, delay: 0.52, attack: 0.002, hold: 0.04, decay: 0.03, level: 0.4, vary: 0.05, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 2000.0, fm_ratio: 2.0, fm: 0.3, delay: 0.66, attack: 0.002, hold: 0.09, decay: 0.08, level: 0.45, vary: 0.05, ..LAYER },
    ]);

recipe!(MineBlast, "mine_blast", "Mine going off: a hard crack, a short fat body and a spray of ringing metal shrapnel.", reverb 0.3 / 1.1, width 0.8,
    presets [("Proximity cluster", fx(0.6, 5.0, 0.65)), ("Magnetic mine", fx(1.5, -5.0, 0.4))],
    [
        Layer { src: Src::White, mode: HighPass, f0: 2500.0, hold: 0.003, decay: 0.05, level: 1.1, drive: 0.6, wide: 0.0, ..LAYER },
        Layer { src: Src::Pink, mode: LowPass, f0: 5000.0, f1: 250.0, glide: 0.09, q: 0.25, hold: 0.015, decay: 0.45, level: 1.6, drive: 0.8, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 140.0, f1: 45.0, glide: 0.1, hold: 0.02, decay: 0.35, level: 0.9, vary: 0.4, ..LAYER },
        Layer { src: Src::Debris, f0: 6000.0, f1: 2500.0, glide: 0.4, rate: 220.0, spread: 1.2, q: 0.85, delay: 0.02, hold: 0.05, decay: 0.7, level: 0.7, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1900.0, fm_ratio: 1.41, fm: 0.8, hold: 0.0, decay: 0.4, level: 0.12, vary: 1.5, ..LAYER },
    ]);

recipe!(LockOn, "lock_on", "Missile lock acquired: three quick pips, then a held tone.", reverb 0.05 / 0.3, width 0.1,
    presets [("Incoming!", FxParams { variation: 0.0, ..fx(0.7, 7.0, 0.65) })],
    [
        Layer { src: Src::Tone(Sine), f0: 1200.0, fm_ratio: 2.0, fm: 0.3, attack: 0.002, hold: 0.04, decay: 0.02, level: 0.45, vary: 0.02, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1200.0, fm_ratio: 2.0, fm: 0.3, delay: 0.09, attack: 0.002, hold: 0.04, decay: 0.02, level: 0.45, vary: 0.02, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1200.0, fm_ratio: 2.0, fm: 0.3, delay: 0.18, attack: 0.002, hold: 0.04, decay: 0.02, level: 0.45, vary: 0.02, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1800.0, fm_ratio: 2.0, fm: 0.3, delay: 0.3, attack: 0.002, hold: 0.28, decay: 0.08, level: 0.45, vary: 0.02, ..LAYER },
    ]);

recipe!(ShieldUp, "shield_up", "Shield powering up: a rising swell with glassy shimmer and sparkle.", reverb 0.4 / 1.4, width 0.9,
    presets [("Autopilot engage", fx(0.7, 7.0, 0.6))],
    [
        Layer { src: Src::Tone(Sine), f0: 180.0, f1: 520.0, glide: 0.35, fm_ratio: 2.0, fm: 1.0, attack: 0.25, hold: 0.1, decay: 0.5, level: 0.5, vary: 0.3, chorus: 9.0, ..LAYER },
        Layer { src: Src::White, mode: BandPass, f0: 3000.0, f1: 7000.0, glide: 0.4, q: 0.85, attack: 0.3, hold: 0.1, decay: 0.6, level: 0.5, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 60.0, f1: 90.0, glide: 0.3, attack: 0.2, hold: 0.1, decay: 0.4, level: 0.5, vary: 0.3, ..LAYER },
        Layer { src: Src::Debris, f0: 7000.0, rate: 120.0, spread: 0.8, q: 0.8, attack: 0.3, hold: 0.1, decay: 0.5, level: 0.3, ..LAYER },
    ]);

// Pure tones show limiter distortion long before noise does, so the partials are kept low enough
// for the in-phase strike to stay under the limiter knee.
recipe!(Bell, "bell", "Struck bell: a strike, then inharmonic partials that ring for seconds, each pair beating slowly.", reverb 0.35 / 1.8, width 0.5,
    presets [("Large bell", fx(1.5, -7.0, 0.45)), ("Glass chime", FxParams { tail: 0.7, ..fx(0.45, 12.0, 0.7) })],
    [
        Layer { src: Src::White, mode: BandPass, f0: 2500.0, q: 0.5, hold: 0.002, decay: 0.03, level: 0.28, wide: 0.0, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 220.0, beat: 0.7, attack: 0.004, hold: 0.0, decay: 6.0, level: 0.28, vary: 0.1, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 440.0, beat: 1.1, attack: 0.002, hold: 0.0, decay: 5.0, level: 0.34, vary: 0.1, pan: -0.2, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 524.0, beat: 0.9, attack: 0.002, hold: 0.0, decay: 3.5, level: 0.19, vary: 0.1, pan: 0.3, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 660.0, attack: 0.002, hold: 0.0, decay: 2.5, level: 0.12, vary: 0.1, pan: -0.3, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 880.0, beat: 1.6, attack: 0.001, hold: 0.0, decay: 3.0, level: 0.25, vary: 0.1, pan: 0.2, chorus: 4.0, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1175.0, attack: 0.001, hold: 0.0, decay: 1.6, level: 0.12, vary: 0.1, pan: -0.4, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1850.0, attack: 0.001, hold: 0.0, decay: 0.9, level: 0.06, vary: 0.1, pan: 0.4, ..LAYER },
    ]);

recipe!(Finish, "finish", "Race finished: four rising chime notes into a held, shimmering chord.", reverb 0.35 / 1.4, width 0.6,
    presets [("New record", FxParams { tail: 0.7, ..fx(1.2, 5.0, 0.6) }), ("Lap", fx(0.6, 0.0, 0.5))],
    [
        Layer { src: Src::Tone(Sine), f0: 523.3, fm_ratio: 2.0, fm: 0.5, attack: 0.002, hold: 0.02, decay: 0.3, level: 0.45, vary: 0.05, pan: -0.5, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 659.3, fm_ratio: 2.0, fm: 0.5, delay: 0.09, attack: 0.002, hold: 0.02, decay: 0.3, level: 0.45, vary: 0.05, pan: -0.2, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 784.0, fm_ratio: 2.0, fm: 0.5, delay: 0.18, attack: 0.002, hold: 0.02, decay: 0.3, level: 0.45, vary: 0.05, pan: 0.2, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 1046.5, fm_ratio: 2.0, fm: 0.6, delay: 0.27, attack: 0.002, hold: 0.05, decay: 1.5, level: 0.5, vary: 0.05, pan: 0.5, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 523.3, beat: 1.5, delay: 0.27, attack: 0.01, hold: 0.1, decay: 1.4, level: 0.3, vary: 0.05, pan: -0.4, chorus: 5.0, ..LAYER },
        Layer { src: Src::Tone(Sine), f0: 784.0, beat: 2.1, delay: 0.27, attack: 0.01, hold: 0.1, decay: 1.4, level: 0.25, vary: 0.05, pan: 0.4, chorus: 6.0, ..LAYER },
        Layer { src: Src::White, mode: HighPass, f0: 6000.0, delay: 0.27, attack: 0.005, hold: 0.02, decay: 0.6, level: 0.12, ..LAYER },
    ]);
