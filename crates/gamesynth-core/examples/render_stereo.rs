//! Before/after renders for judging the stereo image by ear, plus what it costs:
//!   cargo run -p gamesynth-core --example render_stereo --release -- out_dir
//! For each event: `<name>_before.wav` (space/width 0, the old mono sound) and
//! `<name>_after.wav` (designed width), stereo files with identical triggers. Prints one
//! `# stereo name preset corr_before corr_after cost_mono cost_stereo` line per event, where
//! cost is render time as a fraction of real time while the event is sounding.
use gamesynth_core::{generators, Model};
use std::time::Instant;

const SR: u32 = 48000;
const EVENTS: &[(&str, &str)] = &[
    ("explosion", "Ship destroyed"),
    ("mine_blast", "Default"),
    ("impact", "Heavy slam"),
    ("rocket", "Default"),
    ("shield_up", "Default"),
    ("boost", "Turbo"),
    ("pickup", "Default"),
    ("bell", "Default"),
    ("finish", "Default"),
];

fn make(name: &str, preset: &str, width: Option<f32>) -> Box<dyn Model> {
    let mut m = generators::create(name, SR as f32).unwrap();
    let i = m.desc().preset_index(preset).unwrap_or_else(|| panic!("{name} has no preset {preset}"));
    m.load_preset(i);
    if let Some(w) = width {
        let p = m.desc().param_index("space/width").unwrap();
        m.set_param(p, w);
    }
    m
}

/// Two triggers (they differ, as in the game), each given the event's full length.
fn render(m: &mut dyn Model) -> (Vec<f32>, Vec<f32>, f64) {
    let n = (m.length_secs().unwrap_or(3.0).min(8.0) * SR as f32) as usize;
    let (mut left, mut right) = (vec![0.0f32; 2 * n], vec![0.0f32; 2 * n]);
    let start = Instant::now();
    for k in 0..2 {
        m.trigger();
        m.render_stereo(&mut left[k * n..(k + 1) * n], &mut right[k * n..(k + 1) * n]);
    }
    (left, right, start.elapsed().as_secs_f64() / (2.0 * n as f64 / SR as f64))
}

fn correlation(l: &[f32], r: &[f32]) -> f32 {
    let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    dot(l, r) / (dot(l, l) * dot(r, r)).sqrt().max(1e-12)
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "stereo_out".into());
    std::fs::create_dir_all(&dir).expect("create output dir");
    let spec = hound::WavSpec { channels: 2, sample_rate: SR, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    for (name, preset) in EVENTS {
        let mut stats = Vec::new();
        for (label, width) in [("before", Some(0.0)), ("after", None)] {
            let mut m = make(name, preset, width);
            let (l, r, cost) = render(m.as_mut());
            let mut w = hound::WavWriter::create(format!("{dir}/{name}_{label}.wav"), spec).expect("create wav");
            for (a, b) in l.iter().zip(&r) {
                w.write_sample((a.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
                w.write_sample((b.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
            }
            w.finalize().unwrap();
            stats.push((correlation(&l, &r), cost));
        }
        println!("# stereo {name} {} {:.3} {:.3} {:.5} {:.5}", preset.replace(' ', "_"), stats[0].0, stats[1].0, stats[0].1, stats[1].1);
    }
}
