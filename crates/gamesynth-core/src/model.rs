//! The common interface of every continuous sound generator.
//!
//! A [`Model`] is a sound driven by named **inputs** (game state, set every frame: throttle,
//! intensity, tension…) and shaped by named **params** (designer tuning, with ranges and
//! presets). Native generators implement the small [`Generator`] trait and are wrapped by
//! [`Native`]; file-defined models ([`crate::graph::GraphModel`]) implement [`Model`]
//! directly. Engine bindings only ever see `Box<dyn Model>`.

use crate::blocks::{settle_coef, BLOCK};
use crate::math::limit;
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
    fn render_mono(&mut self, out: &mut [f32]);
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

    /// Named parameter sets. The default parameters are always preset 0 ("Default").
    fn presets() -> Vec<(&'static str, Self::P)> {
        Vec::new()
    }
    /// Allocation is allowed here only.
    fn new(sample_rate: f32) -> Self;
    /// Jump internal inertia to match `x`.
    fn snap(&mut self, _x: &[f32], _p: &Self::P) {}
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

    fn render_mono(&mut self, out: &mut [f32]) {
        let mut peak = self.peak;
        for chunk in out.chunks_mut(BLOCK) {
            let coef = settle_coef(G::INPUT_SMOOTH_SECS, chunk.len() as f32 / self.sample_rate);
            for (s, t) in self.smooth.iter_mut().zip(&self.target) {
                *s += (*t - *s) * coef;
            }
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

    fn peak(&self) -> f32 {
        self.peak
    }

    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}
