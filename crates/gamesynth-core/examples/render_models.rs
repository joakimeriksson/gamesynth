//! Render every generator (or the named ones) to a stereo WAV with its inputs swept 0 -> 1 -> 0
//! (events: fired four times), and
//! print level statistics used for calibrating default gains:
//!   cargo run -p gamesynth-core --example render_models --release -- out_dir [name...]
use gamesynth_core::render::{peak, rms};
use gamesynth_core::{generators, Model};

const SR: u32 = 48000;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "models_out".into());
    let only: Vec<String> = args.collect();
    std::fs::create_dir_all(&dir).expect("create output dir");
    let spec = hound::WavSpec { channels: 2, sample_rate: SR, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    println!("{:<12} {:<18} {:>9} {:>9} {:>9} {:>7}", "model", "preset", "rms@0.2", "rms@0.6", "rms@1.0", "peak");
    for name in generators::NAMES {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        let presets = generators::create(name, SR as f32).unwrap().desc().presets.len();
        for preset in 0..presets {
            let mut m = generators::create(name, SR as f32).unwrap();
            m.load_preset(preset);
            let label = m.desc().presets[preset].name.clone();
            if preset == 0 && m.desc().one_shot {
                println!("# one_shot {name} {}", slot_secs(m.as_ref()));
            }
            let mut stats = Vec::new();
            for level in [0.2, 0.6, 1.0] {
                if m.desc().one_shot {
                    // Events: one trigger at this power, measured over its first second.
                    let mut s = generators::create(name, SR as f32).unwrap();
                    s.load_preset(preset);
                    s.set_input(0, level);
                    s.trigger();
                    let mut buf = vec![0.0; SR as usize * slot_secs(s.as_ref())];
                    s.render_mono(&mut buf);
                    stats.push((rms(&buf[..SR as usize]), peak(&buf)));
                    continue;
                }
                let mut s = generators::create(name, SR as f32).unwrap();
                s.load_preset(preset);
                steady(s.as_mut(), level);
                let mut buf = vec![0.0; SR as usize * 8];
                s.render_mono(&mut buf);
                stats.push((rms(&buf[SR as usize..]), peak(&buf)));
            }
            println!("{:<12} {:<18} {:>9.3} {:>9.3} {:>9.3} {:>7.2}", name, label, stats[0].0, stats[1].0, stats[2].0, stats[2].1);
            if preset == 0 || !only.is_empty() {
                let (left, right) = sweep(m.as_mut());
                let file = format!("{dir}/{name}_{}.wav", label.to_lowercase().replace(' ', "_"));
                let mut w = hound::WavWriter::create(&file, spec).expect("create wav");
                for (l, r) in left.iter().zip(&right) {
                    w.write_sample((l.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
                    w.write_sample((r.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
                }
                w.finalize().unwrap();
            }
        }
    }
}

/// Primary input at `level`, the rest at their defaults.
fn steady(m: &mut dyn Model, level: f32) {
    m.set_input(0, level);
    m.snap();
}

/// Seconds given to each trigger of an event: its reported length, at least 3 s.
fn slot_secs(m: &dyn Model) -> usize {
    (m.length_secs().unwrap_or(3.0).ceil() as usize).clamp(3, 9)
}

/// One-shots: full power at 0 and again one slot later (they must differ), then power 0.4, then
/// full power at distance 0.8.
fn events(m: &mut dyn Model) -> (Vec<f32>, Vec<f32>) {
    let n = SR as usize * slot_secs(m);
    let (mut left, mut right) = (vec![0.0f32; 4 * n], vec![0.0f32; 4 * n]);
    for (k, (power, distance)) in [(1.0, 0.0), (1.0, 0.0), (0.4, 0.0), (1.0, 0.8)].into_iter().enumerate() {
        m.set_input(0, power);
        m.set_input(1, distance);
        m.trigger();
        m.render_stereo(&mut left[k * n..(k + 1) * n], &mut right[k * n..(k + 1) * n]);
    }
    (left, right)
}

/// 10 s: primary input ramps 0 -> 1 over 4 s, holds 2 s with the second input raised,
/// then falls back over 4 s. Rendered in stereo, as the game hears it.
fn sweep(m: &mut dyn Model) -> (Vec<f32>, Vec<f32>) {
    if m.desc().one_shot {
        return events(m);
    }
    let (mut left, mut right) = (Vec::with_capacity(SR as usize * 10), Vec::with_capacity(SR as usize * 10));
    let (mut l, mut r) = ([0.0f32; 480], [0.0f32; 480]);
    for step in 0..1000 {
        let t = step as f32 / 100.0;
        let primary = if t < 4.0 { t / 4.0 } else if t < 6.0 { 1.0 } else { (10.0 - t) / 4.0 };
        m.set_input(0, primary);
        if m.desc().inputs.len() > 1 {
            let default = m.desc().inputs[1].default;
            m.set_input(1, if (4.0..6.0).contains(&t) { 1.0 } else { default });
        }
        m.render_stereo(&mut l, &mut r);
        left.extend_from_slice(&l);
        right.extend_from_slice(&r);
    }
    (left, right)
}
