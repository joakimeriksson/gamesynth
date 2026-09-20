//! The common interface of every continuous sound generator.
//!
//! A [`Model`] is a sound driven by named **inputs** (game state, set every frame: throttle,
//! intensity, tension…) and shaped by named **params** (designer tuning, with ranges and
//! presets). Native generators implement the small [`Generator`] trait and are wrapped by
//! [`Native`]; file-defined models ([`crate::graph::GraphModel`]) implement [`Model`]
//! directly. Engine bindings only ever see `Box<dyn Model>`.

use crate::blocks::{settle_coef, BLOCK};
use crate::math::{limit, LIMIT_KNEE};
use crate::params::{describe, ParamDesc, Params};

#[derive(Clone, Debug, PartialEq)]
pub struct InputDesc {
    pub name: String,
    pub default: f32,
    pub doc: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PresetDesc {
    pub name: String,
    /// One value per parameter, in parameter order.
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelDesc {
    pub name: String,
    pub category: String,
    pub doc: String,
    /// "native" or "graph".
    pub engine: &'static str,
    /// True for event sounds (explosion, pickup…): silent until [`Model::trigger`], then plays
    /// once and reports [`Model::is_finished`].
    pub one_shot: bool,
    pub inputs: Vec<InputDesc>,
    pub params: Vec<ParamDesc>,
    pub presets: Vec<PresetDesc>,
}

impl ModelDesc {
    pub fn input_index(&self, name: &str) -> Option<usize> {
        self.inputs.iter().position(|i| i.name == name)
    }

    pub fn param_index(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.name == name)
    }

    pub fn preset_index(&self, name: &str) -> Option<usize> {
        self.presets.iter().position(|p| p.name.eq_ignore_ascii_case(name))
    }
}

/// A running sound generator. `render_mono` never allocates, locks or blocks.
pub trait Model: Send {
    fn desc(&self) -> &ModelDesc;
    /// Inputs are 0..1 game-state values; out-of-range and non-finite values are clamped.
    fn set_input(&mut self, index: usize, value: f32);
    fn input(&self, index: usize) -> f32;
    /// Values are clamped to the parameter's range.
    fn set_param(&mut self, index: usize, value: f32);
    fn param(&self, index: usize) -> f32;
    /// Jump all smoothing/inertia straight to the current inputs (e.g. spawning mid-flight).
    fn snap(&mut self);
    /// Fire a one-shot using the current inputs (each trigger varies slightly). Continuous
    /// models ignore it.
    fn trigger(&mut self) {}
    /// True once a one-shot has rung out; continuous models never finish.
    fn is_finished(&self) -> bool {
        false
    }
    fn render_mono(&mut self, out: &mut [f32]);
    /// Render stereo (`left.len() == right.len()`). Models without a stereo image render mono
    /// into both channels. For models that have one, [`Model::render_mono`] is the same sound
    /// with its width at zero, not a fold-down.
    fn render_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.render_mono(left);
        right.copy_from_slice(left);
    }
    /// For one-shots: an upper bound, in seconds, on how long a trigger sounds with the current
    /// params, including its tail. `None` for continuous models and when it cannot be known
    /// without rendering (model files).
    fn length_secs(&self) -> Option<f32> {
        None
    }
    /// Playback-rate style pitch control (2.0 = an octave up), e.g. a player's `pitch_scale`.
    /// Event generators transpose; models without a meaningful pitch ignore it.
    fn set_pitch_ratio(&mut self, _ratio: f32) {}
    /// Recent output peak 0..1, decays over ~250 ms.
    fn peak(&self) -> f32;
    fn sample_rate(&self) -> f32;

    fn set_input_by_name(&mut self, name: &str, value: f32) -> bool {
        match self.desc().input_index(name) {
            Some(i) => {
                self.set_input(i, value);
                true
            }
            None => false,
        }
    }

    fn set_param_by_name(&mut self, name: &str, value: f32) -> bool {
        match self.desc().param_index(name) {
            Some(i) => {
                self.set_param(i, value);
                true
            }
            None => false,
        }
    }

    fn load_preset(&mut self, index: usize) -> bool {
        let Some(values) = self.desc().presets.get(index).map(|p| p.values.clone()) else { return false };
        for (i, v) in values.into_iter().enumerate() {
            self.set_param(i, v);
        }
        true
    }

    fn params_vec(&self) -> Vec<f32> {
        (0..self.desc().params.len()).map(|i| self.param(i)).collect()
    }
}

/// Static description of one input of a native generator.
pub struct InputSpec {
    pub name: &'static str,
    pub default: f32,
    pub doc: &'static str,
}

/// What a native generator implements. [`Native`] supplies input smoothing, parameter
/// storage, presets, the output limiter and the peak meter.
pub trait Generator: Send + 'static {
    type P: Params + Send;
    const NAME: &'static str;
    const CATEGORY: &'static str;
    const DOC: &'static str;
    const INPUTS: &'static [InputSpec];
    /// Seconds for inputs to settle; 0 passes them through (the generator has its own inertia).
    const INPUT_SMOOTH_SECS: f32 = 0.05;
    /// Whether [`Native`] applies the master soft limiter.
    const LIMIT: bool = true;
    /// Event sound: silent until triggered, plays once.
    const ONE_SHOT: bool = false;

    /// Named parameter sets. The default parameters are always preset 0 ("Default").
    fn presets() -> Vec<(&'static str, Self::P)> {
        Vec::new()
    }
    /// Allocation is allowed here only.
    fn new(sample_rate: f32) -> Self;
    /// Jump internal inertia to match `x`.
    fn snap(&mut self, _x: &[f32], _p: &Self::P) {}
    /// Start a one-shot with inputs `x`.
    fn trigger(&mut self, _x: &[f32], _p: &Self::P) {}
    /// Whether a one-shot is still sounding (including its tail).
    fn is_active(&self) -> bool {
        true
    }
    /// Upper bound on a one-shot's length with params `p`, tail included.
    fn length(_p: &Self::P) -> Option<f32> {
        None
    }
    fn set_pitch_ratio(&mut self, _ratio: f32) {}
    /// Stereo version of [`Generator::block`]; the default is mono in both channels.
    fn block_stereo(&mut self, x: &[f32], p: &Self::P, left: &mut [f32], right: &mut [f32]) {
        self.block(x, p, left);
        right.copy_from_slice(left);
    }
    /// Render one control block, overwriting `out` (`out.len() <= BLOCK`). `x` holds the
    /// smoothed inputs in `INPUTS` order.
    fn block(&mut self, x: &[f32], p: &Self::P, out: &mut [f32]);
}

/// Adapter turning a [`Generator`] into a [`Model`].
pub struct Native<G: Generator> {
    g: G,
    p: G::P,
    desc: ModelDesc,
    target: Vec<f32>,
    smooth: Vec<f32>,
    sample_rate: f32,
    peak: f32,
    peak_decay: f32,
}

impl<G: Generator> Native<G> {
    /// Idle one-shots cost nothing: true (and the peak meter decayed) when there is nothing to render.
    fn idle(&mut self, frames: usize) -> bool {
        let idle = G::ONE_SHOT && !self.g.is_active();
        if idle {
            self.peak *= self.peak_decay.powi(frames as i32);
        }
        idle
    }

    fn smooth_inputs(&mut self, frames: usize) {
        let coef = settle_coef(G::INPUT_SMOOTH_SECS, frames as f32 / self.sample_rate);
        for (s, t) in self.smooth.iter_mut().zip(&self.target) {
            *s += (*t - *s) * coef;
        }
    }

    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate >= 8000.0 { sample_rate } else { 48000.0 };
        let desc = Self::describe();
        let target: Vec<f32> = desc.inputs.iter().map(|i| i.default).collect();
        Native {
            g: G::new(sample_rate),
            p: G::P::default(),
            smooth: target.clone(),
            target,
            desc,
            sample_rate,
            peak: 0.0,
            peak_decay: (-9.21 / (0.25 * sample_rate)).exp(),
        }
    }

    /// Description without constructing the DSP state.
    pub fn describe() -> ModelDesc {
        let values = |p: &G::P| G::P::ALL.iter().map(|&id| p.get_param(id)).collect::<Vec<_>>();
        let mut presets = vec![PresetDesc { name: "Default".into(), values: values(&G::P::default()) }];
        presets.extend(G::presets().iter().map(|(n, p)| PresetDesc { name: n.to_string(), values: values(p) }));
        ModelDesc {
            name: G::NAME.into(),
            category: G::CATEGORY.into(),
            doc: G::DOC.into(),
            engine: "native",
            one_shot: G::ONE_SHOT,
            inputs: G::INPUTS.iter().map(|i| InputDesc { name: i.name.into(), default: i.default, doc: i.doc.into() }).collect(),
            params: describe::<G::P>(),
            presets,
        }
    }

    pub fn params(&self) -> &G::P {
        &self.p
    }

    pub fn generator(&self) -> &G {
        &self.g
    }
}

impl<G: Generator> Model for Native<G> {
    fn desc(&self) -> &ModelDesc {
        &self.desc
    }

    fn set_input(&mut self, index: usize, value: f32) {
        if let Some(t) = self.target.get_mut(index) {
            *t = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
        }
    }

    fn input(&self, index: usize) -> f32 {
        self.target.get(index).copied().unwrap_or(0.0)
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(&id) = G::P::ALL.get(index) {
            self.p.set_param(id, value);
        }
    }

    fn param(&self, index: usize) -> f32 {
        G::P::ALL.get(index).map(|&id| self.p.get_param(id)).unwrap_or(0.0)
    }

    fn snap(&mut self) {
        self.smooth.copy_from_slice(&self.target);
        self.g.snap(&self.smooth, &self.p);
    }

    fn trigger(&mut self) {
        // An event takes its inputs as they are at that instant.
        self.smooth.copy_from_slice(&self.target);
        self.g.trigger(&self.smooth, &self.p);
    }

    fn is_finished(&self) -> bool {
        G::ONE_SHOT && !self.g.is_active() && self.peak < 1e-4
    }

    fn render_mono(&mut self, out: &mut [f32]) {
        let mut peak = self.peak;
        if self.idle(out.len()) {
            out.iter_mut().for_each(|s| *s = 0.0);
            return;
        }
        for chunk in out.chunks_mut(BLOCK) {
            self.smooth_inputs(chunk.len());
            self.g.block(&self.smooth, &self.p, chunk);
            for s in chunk.iter_mut() {
                let y = if !s.is_finite() {
                    0.0
                } else if G::LIMIT {
                    limit(*s)
                } else {
                    *s
                };
                *s = y;
                peak = (peak * self.peak_decay).max(y.abs());
            }
        }
        self.peak = peak;
    }

    fn render_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut peak = self.peak;
        if self.idle(left.len()) {
            left.iter_mut().chain(right.iter_mut()).for_each(|s| *s = 0.0);
            return;
        }
        for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
            self.smooth_inputs(l.len());
            self.g.block_stereo(&self.smooth, &self.p, l, r);
            for (a, b) in l.iter_mut().zip(r.iter_mut()) {
                if !a.is_finite() || !b.is_finite() {
                    (*a, *b) = (0.0, 0.0);
                }
                // One gain for both channels, so limiting never shifts the stereo image.
                let m = a.abs().max(b.abs());
                if G::LIMIT && m > LIMIT_KNEE {
                    // The louder channel takes the limited value itself (not value * gain), so
                    // equal channels stay sample-identical to the mono render.
                    let (limited, g) = (limit(m), limit(m) / m);
                    *a = if a.abs() == m { limited.copysign(*a) } else { *a * g };
                    *b = if b.abs() == m { limited.copysign(*b) } else { *b * g };
                }
                peak = (peak * self.peak_decay).max(a.abs().max(b.abs()));
            }
        }
        self.peak = peak;
    }

    fn length_secs(&self) -> Option<f32> {
        G::length(&self.p)
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        self.g.set_pitch_ratio(if ratio.is_finite() { ratio.clamp(0.125, 8.0) } else { 1.0 });
    }

    fn peak(&self) -> f32 {
        self.peak
    }

    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}
