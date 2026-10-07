//! Render a model (model file or native generator name) to a stereo WAV while moving its inputs
//! and params on a timeline, for demo clips (see FILMING.md):
//!   cargo run -p brusverk-core --example render_timeline --release -- <model> <out.wav> <secs> [--trace out.csv] [move ...]
//! A move is `t:name=value[/glide]`: `name` is an input or a param (bare or group/name), moved
//! linearly over `glide` s (default 0.4). `t:preset=Name` switches to a fresh instance with that
//! preset (inputs carried over), crossfaded over 50 ms, so pipe lengths and layouts never jump.
//! `--trace` writes t, engine_rpm (in RPM when the model has engine/idle_rpm and engine/max_rpm, else
//! 0..1) and every input at 60 Hz, for on-screen gauges.
use brusverk_core::{generators, GraphModel, Model};
use std::io::Write;

const SR: f32 = 48000.0;
const BLOCK: usize = 32;
const XFADE: f32 = 0.05;

enum Target {
    Input(usize),
    Param(usize),
    Preset(String),
}

struct Move {
    t: f32,
    target: Target,
    to: f32,
    glide: f32,
    from: Option<f32>,
}

fn create(spec: &str) -> Box<dyn Model> {
    if spec.ends_with(".toml") {
        Box::new(GraphModel::from_text(&std::fs::read_to_string(spec).unwrap(), SR).unwrap_or_else(|e| panic!("{e}")))
    } else {
        generators::create(spec, SR).expect("no such generator")
    }
}

fn with_preset(spec: &str, name: &str, inputs: &[f32]) -> Box<dyn Model> {
    let mut m = create(spec);
    let i = m.desc().preset_index(name).unwrap_or_else(|| panic!("unknown preset {name}"));
    m.load_preset(i);
    for (k, v) in inputs.iter().enumerate() {
        m.set_input(k, *v);
    }
    m.snap();
    m
}

fn main() {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let trace_path = a.iter().position(|s| s == "--trace").map(|i| {
        a.remove(i);
        a.remove(i)
    });
    let spec = a[0].clone();
    let mut m = create(&spec);
    let secs: f32 = a[2].parse().unwrap();
    let mut moves: Vec<Move> = a[3..]
        .iter()
        .map(|s| {
            let (t, rest) = s.split_once(':').unwrap();
            let (name, val) = rest.split_once('=').unwrap();
            if name == "preset" {
                return Move { t: t.parse().unwrap(), target: Target::Preset(val.into()), to: 0.0, glide: 0.0, from: None };
            }
            let (val, glide) = val.split_once('/').map(|(v, g)| (v, g.parse().unwrap())).unwrap_or((val, 0.4));
            let d = m.desc();
            let target = if let Some(i) = d.input_index(name) {
                Target::Input(i)
            } else {
                let i = d
                    .params
                    .iter()
                    .position(|p| p.name == name || p.name.rsplit('/').next() == Some(name))
                    .unwrap_or_else(|| panic!("unknown {name}; params: {:?}", d.params.iter().map(|p| &p.name).collect::<Vec<_>>()));
                Target::Param(i)
            };
            Move { t: t.parse().unwrap(), target, to: val.parse().unwrap(), glide, from: None }
        })
        .collect();
    // Moves at t=0 are the starting state.
    for mv in moves.iter().filter(|mv| mv.t == 0.0) {
        match &mv.target {
            Target::Input(i) => m.set_input(*i, mv.to),
            Target::Param(i) => m.set_param(*i, mv.to),
            Target::Preset(p) => m = with_preset(&spec, p, &[]),
        }
    }
    m.snap();
    if m.desc().one_shot {
        m.trigger();
    }
    let rpm_range = |m: &dyn Model| {
        let d = m.desc();
        Some((m.param(d.param_index("engine/idle_rpm")?), m.param(d.param_index("engine/max_rpm")?)))
    };
    let mut trace = trace_path.map(|p| {
        let mut f = std::io::BufWriter::new(std::fs::File::create(p).unwrap());
        let names: Vec<_> = m.desc().inputs.iter().map(|i| i.name.clone()).collect();
        writeln!(f, "t,engine_rpm,{}", names.join(",")).unwrap();
        f
    });
    let n = (secs * SR) as usize;
    let spec_wav = hound::WavSpec { channels: 2, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(&a[1], spec_wav).unwrap();
    let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    let (mut ol, mut or) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    // The instance being faded out after a preset switch, and when its fade started.
    let mut old: Option<(Box<dyn Model>, f32)> = None;
    let mut peak = 0.0f32;
    let mut next_trace = 0.0f32;
    let mut i = 0;
    while i < n {
        let t = i as f32 / SR;
        for mv in moves.iter_mut().filter(|mv| mv.t > 0.0 && t >= mv.t) {
            if let Target::Preset(p) = &mv.target {
                if mv.from.is_none() {
                    mv.from = Some(0.0);
                    let inputs: Vec<f32> = (0..m.desc().inputs.len()).map(|k| m.input(k)).collect();
                    let fresh = with_preset(&spec, p, &inputs);
                    old = Some((std::mem::replace(&mut m, fresh), t));
                }
                continue;
            }
            let (idx, is_input) = match mv.target {
                Target::Input(i) => (i, true),
                Target::Param(i) => (i, false),
                Target::Preset(_) => unreachable!(),
            };
            if t - mv.t > mv.glide + 0.01 {
                continue;
            }
            let cur = if is_input { m.input(idx) } else { m.param(idx) };
            let from = *mv.from.get_or_insert(cur);
            let k = if mv.glide <= 0.0 { 1.0 } else { ((t - mv.t) / mv.glide).min(1.0) };
            let v = from + (mv.to - from) * k;
            if is_input {
                m.set_input(idx, v);
                if let Some((o, _)) = old.as_mut() {
                    o.set_input(idx, v);
                }
            } else {
                m.set_param(idx, v);
            }
        }
        m.render_stereo(&mut l, &mut r);
        if let Some((o, t0)) = old.as_mut() {
            o.render_stereo(&mut ol, &mut or);
            for k in 0..BLOCK {
                let g = (((i + k) as f32 / SR - *t0) / XFADE).clamp(0.0, 1.0);
                l[k] = l[k] * g + ol[k] * (1.0 - g);
                r[k] = r[k] * g + or[k] * (1.0 - g);
            }
            if t - *t0 > XFADE {
                old = None;
            }
        }
        if let Some(f) = trace.as_mut() {
            if t >= next_trace {
                next_trace += 1.0 / 60.0;
                let rev = m.rpm().unwrap_or(0.0);
                let rpm = rpm_range(m.as_ref()).map_or(rev, |(lo, hi)| lo + (hi - lo) * rev);
                let ins: Vec<String> = (0..m.desc().inputs.len()).map(|k| format!("{:.4}", m.input(k))).collect();
                writeln!(f, "{t:.4},{rpm:.1},{}", ins.join(",")).unwrap();
            }
        }
        for (x, y) in l.iter().zip(&r) {
            peak = peak.max(x.abs()).max(y.abs());
            w.write_sample((x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
            w.write_sample((y.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
        }
        i += BLOCK;
    }
    w.finalize().unwrap();
    eprintln!("{} peak {peak:.3}", a[1]);
}
