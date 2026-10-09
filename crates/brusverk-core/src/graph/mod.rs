//! File-defined sound models: a TOML or JSON description compiled into a DSP graph.
//!
//! This is the slower, flexible tier next to the native [`crate::generators`]: a model file
//! declares **inputs** (game state), **params** (designer tuning), control-rate **signals**
//! (formulas, see [`expr`]) and a **graph** of nodes from a fixed library. [`GraphModel`]
//! implements [`Model`], so bindings treat it exactly like a native generator.
//!
//! All validation, ordering and allocation happens in [`GraphModel::from_text`]; rendering
//! is allocation-free. Audio feedback between nodes is not allowed (use `comb` / `delay`,
//! which contain their own feedback path).
//!
//! A model is mono unless `[graph]` names `out_right` as well as `out`: then it renders stereo
//! (left = `out`), built with `pan` nodes. Its mono render centres every `pan` and averages
//! the two outputs. The building-block nodes (`powerdust`, `modal`, `chirp`, `fm`, `ad`, `adsr`,
//! `formant`, `pattern`, `impulse`, `pan`, `distance`) wrap [`crate::dsp`].
//!
//! ```toml
//! [model]
//! name = "rain_on_tent"
//!
//! [inputs]
//! intensity = { default = 0.5 }
//!
//! [params]
//! drop_hz = { default = 2600, min = 500, max = 8000, scale = "exp" }
//!
//! [signals]
//! density = "lerp(5, 900, pow(intensity, 2))"
//!
//! [graph]
//! nodes = [
//!   { id = "drops", type = "dust", rate = "density" },
//!   { id = "ping", type = "resonators", in = ["drops"], freqs = [0.6, 1.0, 1.4], scale = "drop_hz", route = "random" },
//! ]
//! out = "ping"
//! ```

mod expr;

use indexmap::IndexMap;
use serde::Deserialize;

use crate::blocks::{hz_coef, settle_coef, Brown, DelayLine, Dust, OnePole, Reverb, BLOCK};
use crate::dsp::{self, Air, Chirp, Envelope, FmOp, Formant, Modal, Onset, Pattern, PowerDust, Rhythm, MAX_FORMANTS};
use crate::filter::{FilterMode, Svf};
use crate::math::{limit, soft_clip, Rng, TAU};
use crate::model::{InputDesc, Model, ModelDesc, PresetDesc};
use crate::noise::Noise;
use crate::osc::{Oscillator, Waveform};
use crate::params::{ParamDesc, ParamKind};
use expr::{ExprState, Program};

pub const MAX_NODES: usize = 256;
pub const MAX_SIGNALS: usize = 128;
pub const MAX_PARAMS: usize = 64;
pub const MAX_INPUTS: usize = 16;
pub const MAX_RESONATORS: usize = 32;
/// Most arguments a node type takes as numbers or formulas.
const MAX_NODE_PARAMS: usize = 8;
const MAX_DELAY_MS: f32 = 2000.0;

/// Why a model file was rejected. The message names the offending section and entry.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelError(pub String);

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ModelError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ModelError> {
    Err(ModelError(msg.into()))
}

// ---------------------------------------------------------------------------------------------
// File schema
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    #[serde(default)]
    pub model: MetaSpec,
    #[serde(default)]
    pub inputs: IndexMap<String, InputSpecFile>,
    #[serde(default)]
    pub params: IndexMap<String, ParamSpecFile>,
    #[serde(default)]
    pub signals: IndexMap<String, Arg>,
    pub graph: GraphSpec,
    #[serde(default)]
    pub presets: IndexMap<String, IndexMap<String, f32>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetaSpec {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub doc: String,
    #[serde(default)]
    pub version: u32,
    /// Event sound: silent until triggered. Formulas use `t` (seconds since the trigger) and
    /// `rnd` (0..1, re-rolled per trigger) to shape envelopes and variation.
    #[serde(default)]
    pub one_shot: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSpecFile {
    #[serde(default)]
    pub default: f32,
    #[serde(default)]
    pub doc: String,
    /// Seconds for the input to settle (default 0.05).
    pub smooth: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamSpecFile {
    pub default: f32,
    pub min: f32,
    pub max: f32,
    /// "lin" (default) or "exp".
    #[serde(default)]
    pub scale: String,
    /// Inspector group; the public parameter name is `group/key`.
    #[serde(default)]
    pub group: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSpec {
    pub nodes: Vec<NodeSpec>,
    /// The output node; the left channel when `out_right` is set.
    pub out: String,
    /// Makes the model stereo: the node heard on the right. See [`GraphModel`].
    #[serde(default)]
    pub out_right: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NodeSpec {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, rename = "in")]
    pub inputs: Vec<InputRef>,
    /// Second input group, used by `mul`.
    #[serde(default)]
    pub by: Vec<InputRef>,
    #[serde(flatten)]
    pub args: IndexMap<String, Arg>,
}

/// A number, a formula, or a list of numbers.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Arg {
    Num(f64),
    Text(String),
    List(Vec<f64>),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum InputRef {
    Id(String),
    Full {
        from: String,
        #[serde(default)]
        gain: Option<Arg>,
    },
}

/// Parse a model file. JSON if it starts with `{`, TOML otherwise.
pub fn parse_spec(text: &str) -> Result<ModelSpec, ModelError> {
    if text.trim_start().starts_with('{') {
        serde_json::from_str(text).map_err(|e| ModelError(format!("JSON: {e}")))
    } else {
        toml::from_str(text).map_err(|e| ModelError(format!("TOML: {}", e.to_string().trim_end())))
    }
}

// ---------------------------------------------------------------------------------------------
// Node library
// ---------------------------------------------------------------------------------------------

/// (type name, [(param name, default; NaN = required)], static argument names, doc)
type NodeInfo = (&'static str, &'static [(&'static str, f32)], &'static [&'static str], &'static str);

const REQ: f32 = f32::NAN;

/// The node library, also exposed to tooling through [`node_library`].
const NODE_TYPES: &[NodeInfo] = &[
    ("noise", &[], &["color"], "Noise source. color = white | pink | brown"),
    ("sine", &[("freq", REQ)], &[], "Sine oscillator"),
    ("saw", &[("freq", REQ)], &[], "Band-limited sawtooth"),
    ("tri", &[("freq", REQ)], &[], "Triangle oscillator"),
    ("pulse", &[("freq", REQ), ("width", 0.5)], &[], "Band-limited pulse"),
    ("dust", &[("rate", REQ)], &[], "Random impulses, `rate` per second (rain drops, crackles, clicks)"),
    ("dc", &[("value", REQ)], &[], "Constant (smoothed) value, e.g. the offset in `1 + depth * lfo` tremolo via mul"),
    ("svf", &[("cutoff", REQ), ("resonance", 0.2)], &["mode"], "State variable filter. mode = lowpass | highpass | bandpass | notch"),
    ("lowpass", &[("cutoff", REQ)], &[], "Gentle 6 dB/oct low-pass"),
    ("highpass", &[("cutoff", REQ)], &[], "Gentle 6 dB/oct high-pass"),
    ("comb", &[("ms", REQ), ("feedback", 0.5), ("mix", 1.0)], &["max_ms"], "Feedback comb: tube and pipe resonance, flanging"),
    ("delay", &[("ms", REQ), ("feedback", 0.3), ("mix", 0.3)], &["max_ms"], "Echo"),
    ("resonators", &[("resonance", 0.95), ("scale", 1.0)], &["freqs", "route", "spread"], "Bank of ringing band-passes at freqs * scale. route = all | random (each impulse excites one, detuned by up to +-spread octaves so drops do not sound like a chime)"),
    ("decay", &[("ms", 5.0)], &[], "Turns impulses into decaying envelopes; multiply with noise for bursts"),
    ("drive", &[("amount", 0.5)], &[], "Soft-clip saturation"),
    ("crush", &[("bits", 8.0), ("downsample", 1.0)], &[], "Bit depth and sample-rate reduction"),
    ("gain", &[("gain", REQ)], &[], "Smoothed gain"),
    ("mix", &[], &[], "Sum of inputs (each input may carry its own gain)"),
    ("mul", &[], &[], "sum(in) * sum(by): ring modulation, envelopes, gating"),
    ("reverb", &[("time", 1.2), ("mix", 0.3), ("damping", 0.4)], &[], "Small room reverb; gives one-shots a tail"),
    ("limiter", &[], &[], "Soft limiter, never exceeds +-1"),
    ("powerdust", &[("rate", REQ), ("skew", 3.0)], &[], "Random impulses whose sizes follow a power law: size = u^skew, so at skew 3 most are tiny and a few are big (crackle, gravel, drops, leaf contacts). skew 1 = uniform, 0 = all equal"),
    ("impulse", &[("amp", 1.0), ("rate", 0.0)], &[], "One impulse of size `amp` when a one-shot is triggered, plus `rate` per second (0 = none) as a steady pulse train"),
    ("pattern", &[("rate", REQ), ("count", 4.0), ("gap", 1.0), ("jitter", 0.0), ("variation", 0.0), ("run", 1.0)], &["euclid"], "Impulses in phrases: `count` at `rate` per second, then `gap` extra seconds of rest. jitter 0..1 of a step; variation 0..1 rolls each phrase's count and gap (+-50%) and each impulse's size. Plays while run > 0.5 and restarts a phrase on each one-shot trigger. euclid = [hits, steps] or [hits, steps, rotation]: a phrase is one cycle of the steps"),
    ("ad", &[("attack", 0.005), ("hold", 0.0), ("decay", 0.3)], &[], "Click-free envelope fired by impulses or gate edges in `in`: S-curve attack (at least 0.5 ms) to the impulse's size, hold, then a decay (60 dB time) that lands exactly on zero. Retriggers from where it is"),
    ("adsr", &[("gate", REQ), ("attack", 0.01), ("decay", 0.2), ("sustain", 0.7), ("release", 0.3)], &[], "Click-free gated envelope 0..1: attacks while gate > 0.5, releases to exactly zero when it falls. Re-attacks on each one-shot trigger"),
    ("chirp", &[("from", REQ), ("to", REQ), ("time", 0.1), ("curve", 1.0), ("ratio", 1.0), ("index", 0.0)], &[], "Pitch glide fired by impulses in `in`: from -> to Hz in `time` s (even in octaves at curve 1; >1 late, <1 early), then holds `to`. A sine, or FM with index > 0 (modulator at ratio * pitch). Multiply by an `ad` on the same trigger"),
    ("fm", &[("freq", REQ), ("ratio", 1.0), ("index", 1.0), ("index_decay", 0.0)], &[], "Two-operator FM: carrier at freq, modulator at ratio * freq, index in radians. Optional `in` = triggers that restart the index envelope (index_decay = its 60 dB time; 0 = steady). No DC at ratios 1 and 0.5"),
    ("modal", &[("freq", REQ), ("size", 0.5), ("decay", 1.0), ("position", 0.23), ("variation", 0.3)], &["ratios", "t60", "levels"], "Modal resonator bank (struck objects): modes at freq * ratios, each with its own ring time (t60, seconds) and level; `in` excites them and adds to what still rings. size 0..1 (0.5 as written) lowers pitch and lengthens rings as it grows; each impulse strikes at `position` (0..0.5) moved by `variation`, which also detunes and re-times the modes a little"),
    ("formant", &[("vowel", 0.0), ("shift", 1.0), ("q", 1.0)], &["freqs", "qs", "gains", "routing"], "Formant filter: vowel 0 a, 1 e, 2 i, 3 o, 4 u (fractions morph), or your own freqs = [..] (2 to 4) with qs and gains. shift scales the frequencies (small creature > 1, big < 1), q scales the Qs. routing = parallel (band-passes, default) | series (Klatt cascade)"),
    ("pan", &[("pan", 0.0)], &["channel"], "Equal-power pan, -1 left .. 1 right; channel = left | right picks which side this node outputs. Feed one pan per channel into the outputs named by `out` and `out_right`"),
    ("distance", &[("distance", REQ), ("rolloff", 24.0), ("far_hz", 1000.0), ("delay_ms", 0.0)], &["max_ms"], "Distance 0..1: level falls `rolloff` dB by distance 1, a 12 dB/oct air low-pass falls from 18 kHz to far_hz, and the sound arrives up to delay_ms later (max_ms, default 200)"),
];

/// Node types with their parameters, for editors and documentation:
/// `(type, [(param, default or NaN when required)], static args, doc)`.
pub fn node_library() -> &'static [NodeInfo] {
    NODE_TYPES
}

enum Kind {
    Noise { color: u8, noise: Noise, brown: Brown },
    Osc { wave: Waveform, osc: Oscillator, phase: f32, last_dt: f32 },
    Dust(Dust),
    Dc { last: f32 },
    Svf { mode: FilterMode, f: Svf },
    OnePole { high: bool, f: OnePole },
    Comb { line: DelayLine, last: f32, echo: bool },
    Resonators { f: Vec<Svf>, freqs: Vec<f32>, random: bool, spread: f32, rng: Rng },
    Decay { env: f32 },
    Drive,
    Crush { hold: f32, count: f32 },
    Gain { last: f32 },
    Mix,
    Mul,
    Reverb(Box<Reverb>),
    Limiter,
    PowerDust(PowerDust),
    Impulse { phase: f32 },
    Pattern(Pattern),
    Ad { env: Envelope, onset: Onset },
    Adsr { env: Envelope, gate: bool },
    Chirp { c: Chirp, onset: Onset, started: bool },
    Fm { op: FmOp, onset: Onset, has_in: bool, env: f32, last_inc: f32 },
    Modal(Box<Modal>),
    Formant { f: Formant, custom: Option<Box<[(f32, f32, f32); MAX_FORMANTS]>>, n: usize },
    Pan { right: bool, last: f32 },
    Distance { air: Air, line: DelayLine, last_gain: f32, last_delay: f32 },
}

/// Per-block context for [`process`].
struct Cx {
    sr: f32,
    /// 1 when rendering stereo, 0 for the mono render (pans centred).
    width: f32,
    /// A one-shot was triggered just before this block.
    triggered: bool,
}

enum Binding {
    Const(f32),
    Prog(usize),
}

struct Source {
    node: usize,
    gain: Binding,
    last: f32,
}

struct Node {
    kind: Kind,
    params: Vec<Binding>,
    inputs: Vec<Source>,
    by: Vec<Source>,
}

// ---------------------------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------------------------

/// A compiled, running model. Build with [`GraphModel::from_text`].
pub struct GraphModel {
    desc: ModelDesc,
    sample_rate: f32,
    /// Value table read by programs: `[sr, t, rnd, inputs.., params.., signals..]`.
    vars: Vec<f32>,
    input_target: Vec<f32>,
    input_secs: Vec<f32>,
    param_off: usize,
    signal_off: usize,
    signal_names: Vec<String>,
    /// (program, var slot) per signal, in dependency order.
    signals: Vec<(usize, usize)>,
    programs: Vec<Program>,
    states: Vec<ExprState>,
    nodes: Vec<Node>,
    bufs: Vec<[f32; BLOCK]>,
    out: usize,
    out_right: Option<usize>,
    /// A one-shot trigger the nodes have not seen yet.
    triggered: bool,
    peak: f32,
    peak_decay: f32,
    trigger_rng: Rng,
}

/// Slots of the built-in variables in the value table.
const VAR_T: usize = 1;
const VAR_RND: usize = 2;
const INPUT_OFF: usize = 3;
/// `t` before the first trigger of a one-shot: long enough for any envelope to be silent.
const NEVER: f32 = 1.0e6;

impl GraphModel {
    /// Parse and compile a TOML or JSON model file.
    pub fn from_text(text: &str, sample_rate: f32) -> Result<GraphModel, ModelError> {
        GraphModel::from_spec(&parse_spec(text)?, sample_rate)
    }

    pub fn from_spec(spec: &ModelSpec, sample_rate: f32) -> Result<GraphModel, ModelError> {
        compile(spec, if sample_rate.is_finite() && sample_rate >= 8000.0 { sample_rate } else { 48000.0 })
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Current value of a named signal (for gauges and debugging).
    pub fn signal(&self, name: &str) -> Option<f32> {
        let i = self.signal_names.iter().position(|n| n == name)?;
        self.vars.get(self.signal_off + i).copied()
    }

    /// One block into `out`; with `right`, a stereo model's right channel too. A stereo model
    /// rendered without `right` is the mono render: pans centred, the channels averaged.
    fn render_block(&mut self, out: &mut [f32], right: Option<&mut [f32]>) {
        let n = out.len();
        let (sr, dt) = (self.sample_rate, n as f32 / self.sample_rate);
        let cx = Cx { sr, width: if right.is_some() { 1.0 } else { 0.0 }, triggered: core::mem::take(&mut self.triggered) };
        self.vars[VAR_T] += dt;
        for (i, target) in self.input_target.iter().enumerate() {
            let v = &mut self.vars[INPUT_OFF + i];
            *v += (*target - *v) * settle_coef(self.input_secs[i], dt);
        }
        for k in 0..self.signals.len() {
            let (prog, slot) = self.signals[k];
            let v = self.programs[prog].eval(&self.vars, &mut self.states, dt);
            self.vars[slot] = v;
        }
        let GraphModel { nodes, bufs, vars, programs, states, .. } = self;
        for (i, node) in nodes.iter_mut().enumerate() {
            let mut pv = [0.0f32; MAX_NODE_PARAMS];
            for (k, b) in node.params.iter().enumerate() {
                pv[k] = value(b, programs, vars, states, dt);
            }
            let (before, rest) = bufs.split_at_mut(i);
            let (mut inbuf, mut bybuf) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
            gather(&mut node.inputs, before, &mut inbuf[..n], programs, vars, states, dt);
            gather(&mut node.by, before, &mut bybuf[..n], programs, vars, states, dt);
            let dst = &mut rest[0][..n];
            process(&mut node.kind, &pv, &inbuf[..n], &bybuf[..n], dst, &cx);
            // A filter that blew up would otherwise stay poisoned forever.
            if !dst[n - 1].is_finite() || dst[n - 1].abs() > 1e6 {
                dst.iter_mut().for_each(|s| *s = 0.0);
                reset(&mut node.kind);
            }
        }
        let mut peak = self.peak;
        let clean = |s: f32| if s.is_finite() { limit(s) } else { 0.0 };
        match (self.out_right, right) {
            (None, _) => {
                for (o, s) in out.iter_mut().zip(&self.bufs[self.out][..n]) {
                    let y = clean(*s);
                    *o = y;
                    peak = (peak * self.peak_decay).max(y.abs());
                }
            }
            (Some(r), None) => {
                for ((o, a), b) in out.iter_mut().zip(&self.bufs[self.out][..n]).zip(&self.bufs[r][..n]) {
                    let y = clean(0.5 * (a + b));
                    *o = y;
                    peak = (peak * self.peak_decay).max(y.abs());
                }
            }
            (Some(r), Some(right)) => {
                for (((o, p), a), b) in out.iter_mut().zip(right.iter_mut()).zip(&self.bufs[self.out][..n]).zip(&self.bufs[r][..n]) {
                    (*o, *p) = (clean(*a), clean(*b));
                    peak = (peak * self.peak_decay).max(o.abs().max(p.abs()));
                }
            }
        }
        self.peak = peak;
    }

    /// Whether the model file has a stereo output (`out_right`).
    pub fn is_stereo(&self) -> bool {
        self.out_right.is_some()
    }
}

impl std::fmt::Debug for GraphModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GraphModel({:?}, {} nodes, {} signals)", self.desc.name, self.nodes.len(), self.signals.len())
    }
}

impl Model for GraphModel {
    fn desc(&self) -> &ModelDesc {
        &self.desc
    }

    fn set_input(&mut self, index: usize, v: f32) {
        if let Some(t) = self.input_target.get_mut(index) {
            *t = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        }
    }

    fn input(&self, index: usize) -> f32 {
        self.input_target.get(index).copied().unwrap_or(0.0)
    }

    fn set_param(&mut self, index: usize, v: f32) {
        if let Some(p) = self.desc.params.get(index) {
            if v.is_finite() {
                self.vars[self.param_off + index] = p.kind.clamp(v);
            }
        }
    }

    fn param(&self, index: usize) -> f32 {
        if index < self.desc.params.len() {
            self.vars[self.param_off + index]
        } else {
            0.0
        }
    }

    fn snap(&mut self) {
        for (i, t) in self.input_target.iter().enumerate() {
            self.vars[INPUT_OFF + i] = *t;
        }
        self.states.iter_mut().for_each(|s| s.snap());
        for node in self.nodes.iter_mut() {
            node.inputs.iter_mut().chain(node.by.iter_mut()).for_each(|s| s.last = f32::NAN);
            match &mut node.kind {
                Kind::Osc { last_dt, .. } | Kind::Fm { last_inc: last_dt, .. } => *last_dt = -1.0,
                Kind::Gain { last } | Kind::Dc { last } | Kind::Comb { last, .. } | Kind::Pan { last, .. } => *last = f32::NAN,
                Kind::Distance { last_gain, last_delay, .. } => (*last_gain, *last_delay) = (f32::NAN, f32::NAN),
                _ => {}
            }
        }
    }

    fn trigger(&mut self) {
        self.vars[VAR_T] = 0.0;
        self.vars[VAR_RND] = self.trigger_rng.next_f32();
        self.triggered = self.desc.one_shot;
        // An event takes its inputs as they are at that instant.
        for (i, t) in self.input_target.iter().enumerate() {
            self.vars[INPUT_OFF + i] = *t;
        }
    }

    fn is_finished(&self) -> bool {
        // -60 dB: exponential tails take forever to reach true silence.
        self.desc.one_shot && self.vars[VAR_T] > 0.1 && self.peak < 1e-3
    }

    fn render_mono(&mut self, out: &mut [f32]) {
        if self.desc.one_shot && self.vars[VAR_T] >= NEVER {
            out.iter_mut().for_each(|s| *s = 0.0);
            return;
        }
        for chunk in out.chunks_mut(BLOCK) {
            self.render_block(chunk, None);
        }
        if self.is_finished() {
            // Park it: the next render is free until the next trigger.
            self.vars[VAR_T] = NEVER;
        }
    }

    /// Mono models (no `out_right`) play the same in both channels.
    fn render_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.out_right.is_none() {
            self.render_mono(left);
            right.copy_from_slice(left);
            return;
        }
        if self.desc.one_shot && self.vars[VAR_T] >= NEVER {
            left.iter_mut().chain(right.iter_mut()).for_each(|s| *s = 0.0);
            return;
        }
        for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
            self.render_block(l, Some(r));
        }
        if self.is_finished() {
            self.vars[VAR_T] = NEVER;
        }
    }

    fn peak(&self) -> f32 {
        self.peak
    }

    /// A model file that defines a signal named `rpm` reports it.
    fn rpm(&self) -> Option<f32> {
        self.signal("rpm")
    }

    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}

#[inline]
fn value(b: &Binding, programs: &[Program], vars: &[f32], states: &mut [ExprState], dt: f32) -> f32 {
    match *b {
        Binding::Const(c) => c,
        Binding::Prog(i) => programs[i].eval(vars, states, dt),
    }
}

/// Sum `sources` into `dst`, ramping each gain across the block to avoid zipper noise.
fn gather(sources: &mut [Source], bufs: &[[f32; BLOCK]], dst: &mut [f32], programs: &[Program], vars: &[f32], states: &mut [ExprState], dt: f32) {
    let n = dst.len() as f32;
    for src in sources.iter_mut() {
        let target = value(&src.gain, programs, vars, states, dt);
        if !src.last.is_finite() {
            src.last = target;
        }
        let step = (target - src.last) / n;
        let mut g = src.last;
        for (d, s) in dst.iter_mut().zip(&bufs[src.node]) {
            g += step;
            *d += s * g;
        }
        src.last = target;
    }
}

fn reset(kind: &mut Kind) {
    match kind {
        Kind::Svf { f, .. } => f.reset(),
        Kind::OnePole { f, .. } => *f = OnePole::default(),
        Kind::Comb { line, .. } => line.clear(),
        Kind::Resonators { f, .. } => f.iter_mut().for_each(|r| r.reset()),
        Kind::Decay { env } => *env = 0.0,
        Kind::Reverb(r) => r.clear(),
        Kind::Noise { brown, .. } => *brown = Brown::default(),
        Kind::Modal(m) => m.clear(),
        Kind::Formant { f, .. } => f.reset(),
        Kind::Distance { air, line, .. } => {
            air.reset();
            line.clear();
        }
        Kind::Fm { op, .. } => op.reset(),
        Kind::Ad { env, .. } | Kind::Adsr { env, .. } => env.reset(),
        _ => {}
    }
}

fn process(kind: &mut Kind, pv: &[f32; MAX_NODE_PARAMS], x: &[f32], by: &[f32], out: &mut [f32], cx: &Cx) {
    let n = out.len() as f32;
    let sr = cx.sr;
    match kind {
        Kind::Noise { color, noise, brown } => {
            for o in out.iter_mut() {
                *o = match *color {
                    0 => noise.white(),
                    1 => noise.pink() * 3.0,
                    _ => brown.tick(noise.white()),
                };
            }
        }
        Kind::Osc { wave, osc, phase, last_dt } => {
            let target = (pv[0] / sr).clamp(0.0, 0.45);
            if *last_dt < 0.0 {
                *last_dt = target;
            }
            let step = (target - *last_dt) / n;
            let mut dt = *last_dt;
            for o in out.iter_mut() {
                dt += step;
                *o = if *wave == Waveform::Sine {
                    *phase = (*phase + dt).fract();
                    (*phase * TAU).sin()
                } else {
                    osc.next(*wave, dt, pv[1])
                };
            }
            *last_dt = target;
        }
        Kind::Dust(d) => {
            let p = pv[0].max(0.0) / sr;
            out.iter_mut().for_each(|o| *o = d.tick(p));
        }
        Kind::Svf { mode, f } => {
            f.set(*mode, pv[0], pv[1], sr);
            for (o, s) in out.iter_mut().zip(x) {
                *o = f.tick(*s);
            }
        }
        Kind::OnePole { high, f } => {
            let c = hz_coef(pv[0].clamp(1.0, sr * 0.49), sr);
            for (o, s) in out.iter_mut().zip(x) {
                *o = if *high { f.hp(*s, c) } else { f.lp(*s, c) };
            }
        }
        Kind::Comb { line, last, echo } => {
            let target = (pv[0] * 0.001 * sr).clamp(1.0, line.max_delay());
            if !last.is_finite() {
                *last = target;
            }
            let step = (target - *last) / n;
            let (fb, mix) = (pv[1].clamp(0.0, 0.97), pv[2].clamp(0.0, 1.0));
            let mut d = *last;
            for (o, s) in out.iter_mut().zip(x) {
                d += step;
                let delayed = line.read(d);
                let y = *s + delayed * fb;
                line.write(y);
                *o = if *echo { *s * (1.0 - mix) + delayed * mix } else { *s + (y - *s) * mix };
            }
            *last = target;
        }
        Kind::Resonators { f, freqs, random, spread, rng } => {
            // With a spread, resonators are tuned when struck instead of every block.
            if !*random || *spread <= 0.0 {
                for (r, hz) in f.iter_mut().zip(freqs.iter()) {
                    r.set(FilterMode::BandPass, (hz * pv[1]).clamp(20.0, sr * 0.45), pv[0], sr);
                }
            }
            let norm = 1.0 / (f.len() as f32).sqrt();
            for (o, s) in out.iter_mut().zip(x) {
                let pick = if *random && *s != 0.0 { rng.next_u32() as usize % f.len() } else { usize::MAX };
                if pick != usize::MAX && *spread > 0.0 {
                    let hz = freqs[pick] * pv[1] * (rng.next_bipolar() * *spread).exp2();
                    f[pick].set(FilterMode::BandPass, hz.clamp(20.0, sr * 0.45), pv[0], sr);
                }
                let mut y = 0.0;
                for (k, r) in f.iter_mut().enumerate() {
                    y += r.tick(if !*random || k == pick { *s } else { 0.0 });
                }
                *o = if *random { y } else { y * norm };
            }
        }
        Kind::Decay { env } => {
            let c = (-1.0 / (pv[0].max(0.05) * 0.001 * sr)).exp();
            for (o, s) in out.iter_mut().zip(x) {
                *env = (*env * c).max(s.abs());
                *o = *env;
            }
        }
        Kind::Drive => {
            let a = pv[0].clamp(0.0, 1.0);
            let (pre, post) = (1.0 + a * 15.0, 1.0 - 0.5 * a);
            for (o, s) in out.iter_mut().zip(x) {
                *o = soft_clip(*s * pre) * post;
            }
        }
        Kind::Crush { hold, count } => {
            let levels = (pv[0].clamp(1.0, 16.0) - 1.0).exp2();
            let down = pv[1].max(1.0);
            for (o, s) in out.iter_mut().zip(x) {
                *count += 1.0;
                if *count >= down {
                    *count -= down;
                    *hold = *s;
                }
                *o = (*hold * levels).round() / levels;
            }
        }
        Kind::Dc { last } => {
            if !last.is_finite() {
                *last = pv[0];
            }
            let step = (pv[0] - *last) / n;
            for o in out.iter_mut() {
                *last += step;
                *o = *last;
            }
            *last = pv[0];
        }
        Kind::Gain { last } => {
            if !last.is_finite() {
                *last = pv[0];
            }
            let step = (pv[0] - *last) / n;
            let mut g = *last;
            for (o, s) in out.iter_mut().zip(x) {
                g += step;
                *o = *s * g;
            }
            *last = pv[0];
        }
        Kind::Mix => out.copy_from_slice(x),
        Kind::Mul => {
            for ((o, a), b) in out.iter_mut().zip(x).zip(by) {
                *o = a * b;
            }
        }
        Kind::Reverb(r) => {
            let mix = pv[1].clamp(0.0, 1.0);
            for (o, s) in out.iter_mut().zip(x) {
                *o = *s + r.tick(*s, pv[0].clamp(0.05, 10.0), pv[2]) * mix;
            }
        }
        Kind::Limiter => {
            for (o, s) in out.iter_mut().zip(x) {
                *o = limit(*s);
            }
        }
        Kind::PowerDust(d) => {
            let (p, skew) = (pv[0].max(0.0) / sr, pv[1].max(0.0));
            out.iter_mut().for_each(|o| *o = d.tick(p, skew));
        }
        Kind::Impulse { phase } => {
            let inc = pv[1].max(0.0) / sr;
            if cx.triggered {
                *phase = 0.0;
            }
            for (i, o) in out.iter_mut().enumerate() {
                *o = 0.0;
                if i == 0 && cx.triggered {
                    *o = pv[0];
                } else if inc > 0.0 {
                    *phase += inc;
                    if *phase >= 1.0 {
                        *phase -= phase.floor();
                        *o = pv[0];
                    }
                }
            }
        }
        Kind::Pattern(p) => {
            if cx.triggered {
                p.restart();
            }
            let r = Rhythm { rate: pv[0], count: pv[1], gap: pv[2], jitter: pv[3], variation: pv[4] };
            let run = pv[5] > 0.5;
            out.iter_mut().for_each(|o| *o = p.tick(run, &r, sr));
        }
        Kind::Ad { env, onset } => {
            let dt = 1.0 / sr;
            for (o, s) in out.iter_mut().zip(x) {
                if let Some(v) = onset.tick(*s) {
                    env.trigger(v);
                }
                *o = env.tick(dt, pv[0], pv[1].max(0.0), pv[2], 0.0, pv[2]);
            }
        }
        Kind::Adsr { env, gate } => {
            let g = pv[0] > 0.5;
            if g && (!*gate || cx.triggered) {
                env.trigger(1.0);
            } else if !g && *gate {
                env.release();
            }
            *gate = g;
            let dt = 1.0 / sr;
            for o in out.iter_mut() {
                *o = env.tick(dt, pv[1], 0.0, pv[2], pv[3], pv[4]);
            }
        }
        Kind::Chirp { c, onset, started } => {
            if !*started {
                // Rests at its end pitch until the first trigger.
                c.start(pv[1], pv[1], pv[2], pv[3]);
                *started = true;
            }
            for (o, s) in out.iter_mut().zip(x) {
                if onset.tick(*s).is_some() {
                    c.start(pv[0], pv[1], pv[2], pv[3]);
                }
                *o = c.tick(pv[4], pv[5], sr);
            }
        }
        Kind::Fm { op, onset, has_in, env, last_inc } => {
            let target = (pv[0] / sr).clamp(0.0, 0.45);
            if *last_inc < 0.0 {
                *last_inc = target;
            }
            let step = (target - *last_inc) / n;
            let mut inc = *last_inc;
            let k = if *has_in && pv[3] > 0.0 { (-6.91 / (pv[3] * sr)).exp() } else { 1.0 };
            for (o, s) in out.iter_mut().zip(x) {
                inc += step;
                if onset.tick(*s).is_some() {
                    *env = 1.0;
                }
                let index = pv[2] * *env;
                *env *= k;
                *o = op.tick(inc, pv[1], index);
            }
            *last_inc = target;
        }
        Kind::Modal(m) => {
            m.set(pv[0], pv[1], pv[2], sr);
            for (o, s) in out.iter_mut().zip(x) {
                *o = m.tick(*s, pv[3], pv[4], sr);
            }
            m.end_block();
        }
        Kind::Formant { f, custom, n: count } => {
            let (shift, qm) = (pv[1].max(0.01), pv[2].max(0.01));
            let mut set = [(0.0f32, 1.0f32); MAX_FORMANTS];
            let mut levels = [0.0f32; MAX_FORMANTS];
            let used = match custom {
                Some(c) => {
                    for k in 0..*count {
                        set[k] = (c[k].0 * shift, c[k].1 * qm);
                        levels[k] = c[k].2;
                    }
                    *count
                }
                None => {
                    for (k, (hz, q, level)) in dsp::vowel(pv[0]).into_iter().enumerate() {
                        set[k] = (hz * shift, q * qm);
                        levels[k] = level;
                    }
                    3
                }
            };
            f.set(&set[..used], sr);
            for (o, s) in out.iter_mut().zip(x) {
                *o = f.tick(*s, &levels);
            }
        }
        Kind::Pan { right, last } => {
            let (l, r) = dsp::equal_power(pv[0] * cx.width);
            let target = if *right { r } else { l };
            if !last.is_finite() {
                *last = target;
            }
            let step = (target - *last) / n;
            let mut g = *last;
            for (o, s) in out.iter_mut().zip(x) {
                g += step;
                *o = *s * g;
            }
            *last = target;
        }
        Kind::Distance { air, line, last_gain, last_delay } => {
            let d = pv[0].clamp(0.0, 1.0);
            air.set(d, 18000.0, pv[2].clamp(20.0, 18000.0), 1.0, sr);
            let gain = crate::math::db_to_gain(-pv[1].max(0.0) * d);
            let delay = (pv[3].max(0.0) * d * 0.001 * sr).min(line.max_delay());
            if !last_gain.is_finite() {
                *last_gain = gain;
            }
            if !last_delay.is_finite() {
                *last_delay = delay;
            }
            let (gs, ds) = ((gain - *last_gain) / n, (delay - *last_delay) / n);
            let (mut g, mut dl) = (*last_gain, *last_delay);
            for (o, s) in out.iter_mut().zip(x) {
                g += gs;
                dl += ds;
                let y = if dl < 1.0 { *s } else { line.read(dl) };
                line.write(*s);
                *o = air.tick(y) * g;
            }
            (*last_gain, *last_delay) = (gain, delay);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------------------------

const RESERVED: &[&str] = &[
    "sr", "t", "rnd", "pi", "tau", "abs", "sqrt", "exp", "exp2", "ln", "sin", "cos", "floor", "db", "midi", "min", "max", "pow", "clamp", "lerp", "select",
    "smoothstep", "slew", "lag", "noise", "sh", "lfo", "ramp",
];

fn check_name(section: &str, name: &str) -> Result<(), ModelError> {
    let mut chars = name.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return err(format!("[{section}] '{name}': names must be identifiers (letters, digits, _) so formulas can use them"));
    }
    if RESERVED.contains(&name) {
        return err(format!("[{section}] '{name}' is a reserved word"));
    }
    Ok(())
}

struct Ctx<'a> {
    names: &'a [String],
    programs: Vec<Program>,
    states: Vec<ExprState>,
}

impl Ctx<'_> {
    fn program(&mut self, src: &str, what: &str) -> Result<Program, ModelError> {
        let names = self.names;
        expr::compile(src, &mut |n| names.iter().position(|x| x == n).map(|i| i as u16), &mut self.states)
            .map_err(|e| ModelError(format!("{what}: {e} in \"{src}\"")))
    }

    fn bind(&mut self, arg: &Arg, what: &str) -> Result<Binding, ModelError> {
        match arg {
            Arg::Num(v) => Ok(Binding::Const(*v as f32)),
            Arg::List(_) => err(format!("{what}: expected a number or a formula, got a list")),
            Arg::Text(src) => {
                let p = self.program(src, what)?;
                Ok(match p.as_const() {
                    Some(c) => Binding::Const(c),
                    None => {
                        self.programs.push(p);
                        Binding::Prog(self.programs.len() - 1)
                    }
                })
            }
        }
    }
}

/// Order `count` items so each comes after its dependencies; `deps(i)` lists them.
fn topo_order(count: usize, deps: impl Fn(usize) -> Vec<usize>, label: impl Fn(usize) -> String, what: &str) -> Result<Vec<usize>, ModelError> {
    let all: Vec<Vec<usize>> = (0..count).map(&deps).collect();
    let mut done = vec![false; count];
    let mut order = Vec::with_capacity(count);
    while order.len() < count {
        let before = order.len();
        for i in 0..count {
            if !done[i] && all[i].iter().all(|d| done[*d]) {
                done[i] = true;
                order.push(i);
            }
        }
        if order.len() == before {
            let stuck: Vec<String> = (0..count).filter(|i| !done[*i]).map(&label).collect();
            return err(format!("{what} form a loop: {}. {}", stuck.join(", "), if what == "nodes" { "Feedback between nodes is not supported; use a comb or delay node." } else { "" }));
        }
    }
    Ok(order)
}

fn compile(spec: &ModelSpec, sr: f32) -> Result<GraphModel, ModelError> {
    if spec.inputs.len() > MAX_INPUTS || spec.params.len() > MAX_PARAMS || spec.signals.len() > MAX_SIGNALS {
        return err(format!("too large: at most {MAX_INPUTS} inputs, {MAX_PARAMS} params and {MAX_SIGNALS} signals"));
    }
    if spec.graph.nodes.is_empty() || spec.graph.nodes.len() > MAX_NODES {
        return err(format!("[graph] needs between 1 and {MAX_NODES} nodes"));
    }

    // ---- names and the value table: [sr, inputs.., params.., signals..] ----
    let mut names = vec!["sr".to_string(), "t".to_string(), "rnd".to_string()];
    let sections: [(&str, Vec<&String>); 3] =
        [("inputs", spec.inputs.keys().collect()), ("params", spec.params.keys().collect()), ("signals", spec.signals.keys().collect())];
    for (section, keys) in sections {
        for key in keys {
            check_name(section, key)?;
            if names.contains(key) {
                return err(format!("[{section}] '{key}' is already defined; inputs, params and signals share one namespace"));
            }
            names.push(key.clone());
        }
    }
    let param_off = INPUT_OFF + spec.inputs.len();
    let signal_off = param_off + spec.params.len();
    let mut vars = vec![0.0f32; names.len()];
    vars[0] = sr;
    vars[VAR_T] = if spec.model.one_shot { NEVER } else { 0.0 };
    vars[VAR_RND] = 0.5;

    // ---- description ----
    let mut params = Vec::new();
    for (i, (key, p)) in spec.params.iter().enumerate() {
        // partial_cmp so that NaN bounds are rejected too.
        if p.min.partial_cmp(&p.max) != Some(std::cmp::Ordering::Less) || !(p.min..=p.max).contains(&p.default) {
            return err(format!("[params] '{key}': need min < max and default within the range"));
        }
        let kind = match p.scale.as_str() {
            "" | "lin" | "linear" => ParamKind::Float { min: p.min, max: p.max, step: 0.001 },
            "exp" | "log" if p.min > 0.0 => ParamKind::Exp { min: p.min, max: p.max },
            "exp" | "log" => return err(format!("[params] '{key}': an exp scale needs min > 0")),
            other => return err(format!("[params] '{key}': unknown scale '{other}' (use \"lin\" or \"exp\")")),
        };
        let group = if p.group.is_empty() { "tuning" } else { p.group.as_str() };
        params.push(ParamDesc { name: format!("{group}/{key}"), kind, default: p.default });
        vars[param_off + i] = p.default;
    }
    let defaults: Vec<f32> = params.iter().map(|p| p.default).collect();
    let mut presets = vec![PresetDesc { name: "Default".into(), values: defaults.clone() }];
    for (name, overrides) in &spec.presets {
        let mut values = defaults.clone();
        for (key, v) in overrides {
            let Some(i) = spec.params.get_index_of(key) else { return err(format!("[presets.{name}] unknown param '{key}'")) };
            values[i] = params[i].kind.clamp(*v);
        }
        presets.push(PresetDesc { name: name.clone(), values });
    }
    let mut input_target = Vec::new();
    let mut input_secs = Vec::new();
    let mut inputs = Vec::new();
    for (i, (key, inp)) in spec.inputs.iter().enumerate() {
        let d = inp.default.clamp(0.0, 1.0);
        inputs.push(InputDesc { name: key.clone(), default: d, doc: inp.doc.clone() });
        input_target.push(d);
        input_secs.push(inp.smooth.unwrap_or(0.05).clamp(0.0, 10.0));
        vars[INPUT_OFF + i] = d;
    }
    let desc = ModelDesc {
        name: if spec.model.name.is_empty() { "custom".into() } else { spec.model.name.clone() },
        category: if spec.model.category.is_empty() { "custom".into() } else { spec.model.category.clone() },
        doc: spec.model.doc.clone(),
        engine: "graph",
        one_shot: spec.model.one_shot,
        inputs,
        params,
        presets,
    };

    // ---- signals, in dependency order ----
    let mut ctx = Ctx { names: &names, programs: Vec::new(), states: Vec::new() };
    let signal_src: Vec<String> = spec
        .signals
        .iter()
        .map(|(k, a)| match a {
            Arg::Num(v) => Ok(format!("{v}")),
            Arg::Text(t) => Ok(t.clone()),
            Arg::List(_) => err(format!("[signals] '{k}': expected a formula")),
        })
        .collect::<Result<_, _>>()?;
    let signal_keys: Vec<&String> = spec.signals.keys().collect();
    let mut signal_deps = Vec::new();
    for (k, src) in signal_keys.iter().zip(&signal_src) {
        let used = expr::variables(src).map_err(|e| ModelError(format!("[signals] '{k}': {e} in \"{src}\"")))?;
        signal_deps.push(used.iter().filter_map(|n| spec.signals.get_index_of(n)).collect::<Vec<_>>());
    }
    let order = topo_order(signal_src.len(), |i| signal_deps[i].clone(), |i| signal_keys[i].clone(), "signals")?;
    let mut signals = Vec::new();
    for i in order {
        let p = ctx.program(&signal_src[i], &format!("[signals] '{}'", signal_keys[i]))?;
        ctx.programs.push(p);
        signals.push((ctx.programs.len() - 1, signal_off + i));
    }

    // ---- nodes, in dependency order ----
    let specs = &spec.graph.nodes;
    let find = |id: &str, user: &str| -> Result<usize, ModelError> {
        specs.iter().position(|n| n.id == id).ok_or_else(|| ModelError(format!("node '{user}': input '{id}' is not a node id")))
    };
    for (i, n) in specs.iter().enumerate() {
        if specs[..i].iter().any(|m| m.id == n.id) {
            return err(format!("[graph] duplicate node id '{}'", n.id));
        }
    }
    let ref_id = |r: &InputRef| match r {
        InputRef::Id(id) | InputRef::Full { from: id, .. } => id.clone(),
    };
    let mut node_deps = Vec::new();
    for n in specs {
        let mut d = Vec::new();
        for r in n.inputs.iter().chain(&n.by) {
            d.push(find(&ref_id(r), &n.id)?);
        }
        node_deps.push(d);
    }
    let order = topo_order(specs.len(), |i| node_deps[i].clone(), |i| specs[i].id.clone(), "nodes")?;
    let mut position = vec![0usize; specs.len()];
    for (pos, &i) in order.iter().enumerate() {
        position[i] = pos;
    }
    let mut nodes = Vec::with_capacity(specs.len());
    for (pos, &i) in order.iter().enumerate() {
        let n = &specs[i];
        let sources = |refs: &[InputRef], ctx: &mut Ctx| -> Result<Vec<Source>, ModelError> {
            refs.iter()
                .map(|r| {
                    let gain = match r {
                        InputRef::Full { gain: Some(g), .. } => ctx.bind(g, &format!("node '{}' input gain", n.id))?,
                        _ => Binding::Const(1.0),
                    };
                    Ok(Source { node: position[find(&ref_id(r), &n.id)?], gain, last: f32::NAN })
                })
                .collect()
        };
        let (inputs, by) = (sources(&n.inputs, &mut ctx)?, sources(&n.by, &mut ctx)?);
        nodes.push(build_node(n, inputs, by, &mut ctx, sr, pos as u32)?);
    }
    let out = position[specs.iter().position(|n| n.id == spec.graph.out).ok_or_else(|| ModelError(format!("[graph] out = '{}' is not a node id", spec.graph.out)))?];
    let out_right = match &spec.graph.out_right {
        None => None,
        Some(id) => Some(position[specs.iter().position(|n| &n.id == id).ok_or_else(|| ModelError(format!("[graph] out_right = '{id}' is not a node id")))?]),
    };

    let Ctx { programs, states, .. } = ctx;
    Ok(GraphModel {
        desc,
        sample_rate: sr,
        vars,
        input_target,
        input_secs,
        param_off,
        signal_off,
        signal_names: spec.signals.keys().cloned().collect(),
        signals,
        programs,
        states,
        bufs: vec![[0.0; BLOCK]; nodes.len()],
        nodes,
        out,
        out_right,
        triggered: false,
        peak: 0.0,
        peak_decay: (-9.21 / (0.25 * sr)).exp(),
        trigger_rng: Rng::new(crate::blocks::mix_seed(spec.graph.nodes.len() as u32 ^ 0x7E57)),
    })
}

fn build_node(n: &NodeSpec, inputs: Vec<Source>, by: Vec<Source>, ctx: &mut Ctx, sr: f32, seed: u32) -> Result<Node, ModelError> {
    let who = format!("node '{}' ({})", n.id, n.kind);
    let Some(&(_, param_info, statics, _)) = NODE_TYPES.iter().find(|t| t.0 == n.kind) else {
        let known: Vec<&str> = NODE_TYPES.iter().map(|t| t.0).collect();
        return err(format!("node '{}': unknown type '{}'. Available: {}", n.id, n.kind, known.join(", ")));
    };
    for key in n.args.keys() {
        if !param_info.iter().any(|p| p.0 == key) && !statics.contains(&key.as_str()) {
            let valid: Vec<&str> = param_info.iter().map(|p| p.0).chain(statics.iter().copied()).collect();
            return err(format!("{who}: unknown argument '{key}'. Valid: {}", if valid.is_empty() { "(none)".into() } else { valid.join(", ") }));
        }
    }
    debug_assert!(param_info.len() <= MAX_NODE_PARAMS);
    let is_source = matches!(n.kind.as_str(), "noise" | "sine" | "saw" | "tri" | "pulse" | "dust" | "dc" | "powerdust" | "impulse" | "pattern" | "adsr");
    // `in` is optional: fm takes triggers there if it has any.
    let optional_in = n.kind == "fm";
    if is_source && !inputs.is_empty() {
        return err(format!("{who}: is a source and takes no `in`"));
    }
    if !is_source && !optional_in && inputs.is_empty() {
        return err(format!("{who}: needs at least one input (`in = [...]`)"));
    }
    if (n.kind == "mul") == by.is_empty() {
        return err(format!("{who}: {}", if n.kind == "mul" { "needs `by = [...]` as its second factor" } else { "`by` is only used by mul" }));
    }
    let mut params = Vec::new();
    for (name, default) in param_info {
        params.push(match n.args.get(*name) {
            Some(arg) => ctx.bind(arg, &format!("{who} {name}"))?,
            None if default.is_nan() => return err(format!("{who}: missing required '{name}'")),
            None => Binding::Const(*default),
        });
    }
    let text = |key: &str, default: &str| -> Result<String, ModelError> {
        match n.args.get(key) {
            None => Ok(default.to_string()),
            Some(Arg::Text(t)) => Ok(t.to_lowercase()),
            Some(_) => err(format!("{who}: '{key}' must be a string")),
        }
    };
    let seed = 0x6A00_0000u32.wrapping_add(seed.wrapping_mul(2_654_435_761));
    // A static list argument: None when absent, an error when not a list of numbers in range.
    let list = |key: &str, max_len: usize, ok: &dyn Fn(f64) -> bool, what: &str| -> Result<Option<Vec<f32>>, ModelError> {
        match n.args.get(key) {
            None => Ok(None),
            Some(Arg::List(l)) if !l.is_empty() && l.len() <= max_len && l.iter().all(|v| ok(*v)) => Ok(Some(l.iter().map(|v| *v as f32).collect())),
            Some(Arg::Num(v)) if max_len >= 1 && ok(*v) => Ok(Some(vec![*v as f32])),
            Some(_) => err(format!("{who}: {key} must be {what}")),
        }
    };
    let has_in = !inputs.is_empty();
    let kind = match n.kind.as_str() {
        "noise" => {
            let color = match text("color", "white")?.as_str() {
                "white" => 0,
                "pink" => 1,
                "brown" | "red" => 2,
                other => return err(format!("{who}: unknown color '{other}' (white, pink, brown)")),
            };
            Kind::Noise { color, noise: Noise::new(seed), brown: Brown::default() }
        }
        "sine" | "saw" | "tri" | "pulse" => {
            let wave = match n.kind.as_str() {
                "sine" => Waveform::Sine,
                "saw" => Waveform::Saw,
                "tri" => Waveform::Triangle,
                _ => Waveform::Pulse,
            };
            if wave != Waveform::Pulse {
                params.push(Binding::Const(0.5));
            }
            Kind::Osc { wave, osc: Oscillator::new(seed), phase: 0.0, last_dt: -1.0 }
        }
        "dust" => Kind::Dust(Dust::new(seed)),
        "dc" => Kind::Dc { last: f32::NAN },
        "svf" => {
            let mode = match text("mode", "lowpass")?.as_str() {
                "lowpass" | "lp" => FilterMode::LowPass,
                "highpass" | "hp" => FilterMode::HighPass,
                "bandpass" | "bp" => FilterMode::BandPass,
                "notch" => FilterMode::Notch,
                other => return err(format!("{who}: unknown mode '{other}' (lowpass, highpass, bandpass, notch)")),
            };
            Kind::Svf { mode, f: Svf::default() }
        }
        "lowpass" | "highpass" => Kind::OnePole { high: n.kind == "highpass", f: OnePole::default() },
        "comb" | "delay" => {
            let echo = n.kind == "delay";
            let max_ms = match n.args.get("max_ms") {
                None => if echo { 1000.0 } else { 50.0 },
                Some(Arg::Num(v)) if *v > 0.0 && *v <= MAX_DELAY_MS as f64 => *v as f32,
                Some(_) => return err(format!("{who}: max_ms must be a number up to {MAX_DELAY_MS}")),
            };
            Kind::Comb { line: DelayLine::new((max_ms * 0.001 * sr) as usize), last: f32::NAN, echo }
        }
        "resonators" => {
            let freqs: Vec<f32> = match n.args.get("freqs") {
                Some(Arg::List(l)) if !l.is_empty() && l.len() <= MAX_RESONATORS && l.iter().all(|f| *f > 0.0) => l.iter().map(|f| *f as f32).collect(),
                _ => return err(format!("{who}: needs freqs = [..] with 1 to {MAX_RESONATORS} positive numbers")),
            };
            let random = match text("route", "all")?.as_str() {
                "all" => false,
                "random" => true,
                other => return err(format!("{who}: unknown route '{other}' (all, random)")),
            };
            let spread = match n.args.get("spread") {
                None => 0.0,
                Some(Arg::Num(v)) if (0.0..=4.0).contains(v) => *v as f32,
                Some(_) => return err(format!("{who}: spread must be a number of octaves from 0 to 4")),
            };
            Kind::Resonators { f: vec![Svf::default(); freqs.len()], freqs, random, spread, rng: Rng::new(seed) }
        }
        "decay" => Kind::Decay { env: 0.0 },
        "drive" => Kind::Drive,
        "crush" => Kind::Crush { hold: 0.0, count: 0.0 },
        "gain" => Kind::Gain { last: f32::NAN },
        "mix" => Kind::Mix,
        "mul" => Kind::Mul,
        "reverb" => Kind::Reverb(Box::new(Reverb::new(sr))),
        "powerdust" => Kind::PowerDust(PowerDust::new(seed)),
        "impulse" => Kind::Impulse { phase: 0.0 },
        "pattern" => {
            let euclid = match list("euclid", 3, &|v| (0.0..=64.0).contains(&v) && v.fract() == 0.0, "[hits, steps] or [hits, steps, rotation], whole numbers up to 64")? {
                None => None,
                Some(e) if e.len() >= 2 && e[1] >= 1.0 && e[0] <= e[1] => Some((e[0] as u32, e[1] as u32, e.get(2).copied().unwrap_or(0.0) as u32)),
                Some(_) => return err(format!("{who}: euclid = [hits, steps] needs 1 <= steps and hits <= steps")),
            };
            Kind::Pattern(Pattern::new(seed, euclid))
        }
        "ad" => Kind::Ad { env: Envelope::default(), onset: Onset::default() },
        "adsr" => Kind::Adsr { env: Envelope::default(), gate: false },
        "chirp" => Kind::Chirp { c: Chirp::default(), onset: Onset::default(), started: false },
        "fm" => Kind::Fm { op: FmOp::default(), onset: Onset::default(), has_in, env: 1.0, last_inc: -1.0 },
        "modal" => {
            let max = dsp::MAX_MODES;
            let Some(ratios) = list("ratios", max, &|v| v > 0.0, &format!("a list of 1 to {max} positive frequency ratios"))? else {
                return err(format!("{who}: needs ratios = [..] with 1 to {max} positive frequency ratios"));
            };
            let t60 = list("t60", max, &|v| v > 0.0 && v <= 30.0, &format!("a ring time in seconds (0..30], or a list of up to {max}"))?.unwrap_or_else(|| vec![1.0]);
            let levels = list("levels", max, &|v| (0.0..=100.0).contains(&v), &format!("a list of up to {max} levels from 0 to 100"))?.unwrap_or_else(|| vec![1.0]);
            Kind::Modal(Box::new(Modal::new(&ratios, &t60, &levels, seed)))
        }
        "formant" => {
            let series = match text("routing", "parallel")?.as_str() {
                "parallel" => false,
                "series" | "cascade" => true,
                other => return err(format!("{who}: unknown routing '{other}' (parallel, series)")),
            };
            let max = MAX_FORMANTS;
            let freqs = list("freqs", max, &|v| v > 0.0 && v < 20000.0, &format!("a list of 2 to {max} frequencies in Hz"))?;
            let qs = list("qs", max, &|v| (0.5..=40.0).contains(&v), &format!("a list of up to {max} Qs from 0.5 to 40"))?;
            let gains = list("gains", max, &|v| (0.0..=100.0).contains(&v), &format!("a list of up to {max} levels from 0 to 100"))?;
            let (custom, count) = match freqs {
                Some(f) if f.len() >= 2 => {
                    let pick = |v: &Option<Vec<f32>>, k: usize, d: f32| v.as_ref().map_or(d, |v| v[k.min(v.len() - 1)]);
                    let table: [(f32, f32, f32); MAX_FORMANTS] = core::array::from_fn(|k| if k < f.len() { (f[k], pick(&qs, k, 10.0), pick(&gains, k, 1.0)) } else { (1000.0, 1.0, 0.0) });
                    (Some(Box::new(table)), f.len())
                }
                Some(_) => return err(format!("{who}: freqs needs 2 to {max} formant frequencies")),
                None if qs.is_some() || gains.is_some() => return err(format!("{who}: qs and gains go with freqs = [..]; vowels have their own")),
                None => (None, 3),
            };
            Kind::Formant { f: Formant::new(series), custom, n: count }
        }
        "pan" => {
            let right = match n.args.get("channel") {
                Some(Arg::Text(t)) if t.eq_ignore_ascii_case("left") => false,
                Some(Arg::Text(t)) if t.eq_ignore_ascii_case("right") => true,
                _ => return err(format!("{who}: needs channel = \"left\" or \"right\"")),
            };
            Kind::Pan { right, last: f32::NAN }
        }
        "distance" => {
            let max_ms = match n.args.get("max_ms") {
                None => 200.0,
                Some(Arg::Num(v)) if *v > 0.0 && *v <= MAX_DELAY_MS as f64 => *v as f32,
                Some(_) => return err(format!("{who}: max_ms must be a number up to {MAX_DELAY_MS}")),
            };
            Kind::Distance { air: Air::default(), line: DelayLine::new((max_ms * 0.001 * sr) as usize), last_gain: f32::NAN, last_delay: f32::NAN }
        }
        _ => Kind::Limiter,
    };
    Ok(Node { kind, params, inputs, by })
}
