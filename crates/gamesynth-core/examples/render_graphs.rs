//! Render the model files in `models/` with the same input sweep as `render_models`:
//!   cargo run -p gamesynth-core --example render_graphs --release -- out_dir
use gamesynth_core::{GraphModel, Model};

const SR: u32 = 48000;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "models_out".into());
    std::fs::create_dir_all(&dir).expect("create output dir");
    let spec = hound::WavSpec { channels: 1, sample_rate: SR, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut files: Vec<_> = std::fs::read_dir("models").expect("run from the repo root").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
    files.sort();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let mut m = match GraphModel::from_text(&text, SR as f32) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                continue;
            }
        };
        let file = format!("{dir}/graph_{}.wav", m.desc().name);
        let mut w = hound::WavWriter::create(&file, spec).expect("create wav");
        let mut buf = [0.0f32; 480];
        if m.desc().one_shot {
            println!("# one_shot {}", m.desc().name);
            // Same four events as render_models: power 1, 1, 0.4, then 1 with input 1 raised.
            let mut long = vec![0.0f32; SR as usize * 3];
            for (power, second) in [(1.0, 0.0), (1.0, 0.0), (0.4, 0.0), (1.0, 0.8)] {
                m.set_input(0, power);
                if m.desc().inputs.len() > 1 {
                    m.set_input(1, second);
                }
                m.trigger();
                m.render_mono(&mut long);
                for s in &long {
                    w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
                }
            }
            w.finalize().unwrap();
            println!("{file}");
            continue;
        }
        // 10 s: input 0 ramps 0 -> 1 over 4 s, holds 2 s with input 1 raised, falls over 4 s.
        for step in 0..1000 {
            let t = step as f32 / 100.0;
            m.set_input(0, if t < 4.0 { t / 4.0 } else if t < 6.0 { 1.0 } else { (10.0 - t) / 4.0 });
            if m.desc().inputs.len() > 1 {
                let d = m.desc().inputs[1].default;
                m.set_input(1, if (4.0..6.0).contains(&t) { 1.0 } else { d });
            }
            m.render_mono(&mut buf);
            for s in buf {
                w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
        println!("{file}");
    }
}
