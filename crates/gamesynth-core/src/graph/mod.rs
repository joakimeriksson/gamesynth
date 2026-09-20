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

use crate::blocks::{hz_coef, settle_coef, Brown, DelayLine, Dust, OnePole, BLOCK};
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
    pub out: String,
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
    ("limiter", &[], &[], "Soft limiter, never exceeds +-1"),
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
    Limiter,
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
    /// Value table read by programs: `[sr, inputs.., params.., signals..]`.
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
    peak: f32,
    peak_decay: f32,
}

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

    fn render_block(&mut self, out: &mut [f32]) {
        let n = out.len();
        let (sr, dt) = (self.sample_rate, n as f32 / self.sample_rate);
        for (i, target) in self.input_target.iter().enumerate() {
            let v = &mut self.vars[1 + i];
            *v += (*target - *v) * settle_coef(self.input_secs[i], dt);
        }
        for k in 0..self.signals.len() {
            let (prog, slot) = self.signals[k];
            let v = self.programs[prog].eval(&self.vars, &mut self.states, dt);
            self.vars[slot] = v;
        }
        let GraphModel { nodes, bufs, vars, programs, states, .. } = self;
        for (i, node) in nodes.iter_mut().enumerate() {
            let mut pv = [0.0f32; 4];
            for (k, b) in node.params.iter().enumerate() {
                pv[k] = value(b, programs, vars, states, dt);
            }
            let (before, rest) = bufs.split_at_mut(i);
            let (mut inbuf, mut bybuf) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
            gather(&mut node.inputs, before, &mut inbuf[..n], programs, vars, states, dt);
            gather(&mut node.by, before, &mut bybuf[..n], programs, vars, states, dt);
            let dst = &mut rest[0][..n];
            process(&mut node.kind, &pv, &inbuf[..n], &bybuf[..n], dst, sr);
            // A filter that blew up would otherwise stay poisoned forever.
            if !dst[n - 1].is_finite() || dst[n - 1].abs() > 1e6 {
                dst.iter_mut().for_each(|s| *s = 0.0);
                reset(&mut node.kind);
            }
        }
        let mut peak = self.peak;
        for (o, s) in out.iter_mut().zip(&self.bufs[self.out][..n]) {
            let y = if s.is_finite() { limit(*s) } else { 0.0 };
            *o = y;
            peak = (peak * self.peak_decay).max(y.abs());
        }
        self.peak = peak;
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
            self.vars[1 + i] = *t;
        }
        self.states.iter_mut().for_each(|s| s.snap());
        for node in self.nodes.iter_mut() {
            node.inputs.iter_mut().chain(node.by.iter_mut()).for_each(|s| s.last = f32::NAN);
            match &mut node.kind {
                Kind::Osc { last_dt, .. } => *last_dt = -1.0,
                Kind::Gain { last } | Kind::Dc { last } | Kind::Comb { last, .. } => *last = f32::NAN,
                _ => {}
            }
        }
    }

    fn render_mono(&mut self, out: &mut [f32]) {
        for chunk in out.chunks_mut(BLOCK) {
            self.render_block(chunk);
        }
    }

    fn peak(&self) -> f32 {
        self.peak
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
        Kind::Noise { brown, .. } => *brown = Brown::default(),
        _ => {}
    }
}

fn process(kind: &mut Kind, pv: &[f32; 4], x: &[f32], by: &[f32], out: &mut [f32], sr: f32) {
    let n = out.len() as f32;
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
        Kind::Limiter => {
            for (o, s) in out.iter_mut().zip(x) {
                *o = limit(*s);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------------------------

const RESERVED: &[&str] = &[
    "sr", "pi", "tau", "abs", "sqrt", "exp", "exp2", "ln", "sin", "cos", "floor", "db", "midi", "min", "max", "pow", "clamp", "lerp", "select",
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
    let mut names = vec!["sr".to_string()];
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
    let param_off = 1 + spec.inputs.len();
    let signal_off = param_off + spec.params.len();
    let mut vars = vec![0.0f32; names.len()];
    vars[0] = sr;

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
        vars[1 + i] = d;
    }
    let desc = ModelDesc {
        name: if spec.model.name.is_empty() { "custom".into() } else { spec.model.name.clone() },
        category: if spec.model.category.is_empty() { "custom".into() } else { spec.model.category.clone() },
        doc: spec.model.doc.clone(),
        engine: "graph",
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
        peak: 0.0,
        peak_decay: (-9.21 / (0.25 * sr)).exp(),
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
    let is_source = matches!(n.kind.as_str(), "noise" | "sine" | "saw" | "tri" | "pulse" | "dust" | "dc");
    if is_source && !inputs.is_empty() {
        return err(format!("{who}: is a source and takes no `in`"));
    }
    if !is_source && inputs.is_empty() {
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
        _ => Kind::Limiter,
    };
    Ok(Node { kind, params, inputs, by })
}
