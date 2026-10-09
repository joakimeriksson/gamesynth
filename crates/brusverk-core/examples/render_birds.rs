//! Render the `bird` species (dry and close, for comparison with recordings), or a scene.
//!   cargo run -p brusverk-core --release --example render_birds -- species out_dir secs [individual] [name...]
//!   cargo run -p brusverk-core --release --example render_birds -- alarm out_dir secs [name...]
//!   cargo run -p brusverk-core --release --example render_birds -- probe [input_value]
//!   cargo run -p brusverk-core --release --example render_birds -- demo|ab out_dir
//! `species` writes one mono WAV per species: `<out_dir>/<name>.wav`.
use brusverk_core::{generators, Model};

const SR: f32 = 48000.0;

fn write_wav(path: &str, l: &[f32], r: Option<&[f32]>) {
    let ch = if r.is_some() { 2 } else { 1 };
    let spec = hound::WavSpec { channels: ch, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for i in 0..l.len() {
        w.write_sample((l[i].clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
        if let Some(r) = r {
            w.write_sample((r[i].clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
        }
    }
    w.finalize().unwrap();
}

fn bird(species: &str, individual: f32, inputs: &[(&str, f32)]) -> Box<dyn Model> {
    let mut m = generators::create("bird", SR).unwrap();
    let i = m.desc().preset_index(species).unwrap_or_else(|| panic!("no species {species}"));
    m.load_preset(i);
    m.set_param_by_name("bird/individual", individual);
    m.set_param_by_name("space/reverb", 0.0);
    for (k, v) in inputs {
        m.set_input_by_name(k, *v);
    }
    m.snap();
    m
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("species");
    match mode {
        "species" | "alarm" => {
            let dir = &args[1];
            let secs: f32 = args[2].parse().unwrap();
            let (individual, names) = if mode == "species" { (args.get(3).map_or(0.0, |s| s.parse().unwrap()), &args[4.min(args.len())..]) } else { (0.0, &args[3.min(args.len())..]) };
            std::fs::create_dir_all(dir).unwrap();
            let all: Vec<String> = generators::create("bird", SR).unwrap().desc().presets.iter().skip(1).map(|p| p.name.clone()).collect();
            for name in all.iter().filter(|n| names.is_empty() || names.iter().any(|x| x.eq_ignore_ascii_case(n))) {
                let excitement = if mode == "alarm" { 1.0 } else { 0.0 };
                // Two individuals for songs: `<name>.wav` and `<name>_2.wav`.
                let who: &[(f32, &str)] = if mode == "alarm" { &[(individual, "")] } else { &[(individual, ""), (individual + 1.0, "_2")] };
                for &(ind, suffix) in who {
                    let mut m = bird(name, ind, &[("activity", 0.6), ("distance", 0.0), ("excitement", excitement)]);
                    let mut out = vec![0.0; (secs * SR) as usize];
                    for c in out.chunks_mut(512) {
                        m.render_mono(c);
                    }
                    let file = format!("{dir}/{}{suffix}.wav", name.to_lowercase().replace(' ', "_"));
                    write_wav(&file, &out, None);
                    let peak = out.iter().fold(0.0f32, |a, s| a.max(s.abs()));
                    println!("{file} peak {peak:.2}");
                }
            }
        }
        "probe" => {
            // Level of every species with all inputs at `v` (default 1), over seconds 1..6 and 1..31.
            let v: f32 = args.get(1).map_or(1.0, |s| s.parse().unwrap());
            let presets = generators::create("bird", SR).unwrap().desc().presets.len();
            for p in 0..presets {
                let mut m = generators::create("bird", SR).unwrap();
                m.load_preset(p);
                for i in 0..m.desc().inputs.len() {
                    m.set_input(i, v);
                }
                m.snap();
                let mut out = vec![0.0; (31.0 * SR) as usize];
                for c in out.chunks_mut(512) {
                    m.render_mono(c);
                }
                let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
                let peak = out.iter().fold(0.0f32, |a, s| a.max(s.abs()));
                println!("{:<18} rms 1-6 s {:.4}  1-31 s {:.4}  peak {peak:.2}", m.desc().presets[p].name, rms(&out[SR as usize..(6.0 * SR) as usize]), rms(&out[SR as usize..]));
            }
        }
        "demo" | "ab" => {
            // Stereo clips for the videos: every species alone, close, a little reverb (`demo`:
            // 8 s, singing at once; `ab`: 20 s at the recorded pace), plus (`demo`) a garden
            // morning chorus and a dawn chorus from `birds`.
            let dir = &args[1];
            std::fs::create_dir_all(dir).unwrap();
            let secs = if mode == "demo" { 8.0 } else { 20.0 };
            let names: Vec<String> = generators::create("bird", SR).unwrap().desc().presets.iter().skip(1).map(|p| p.name.clone()).collect();
            for name in names.iter().filter(|n| !n.starts_with("Tropical")) {
                let mut m = bird(name, 0.0, &[("activity", if mode == "demo" { 0.9 } else { 0.6 }), ("distance", 0.12)]);
                m.set_param_by_name("space/reverb", 0.25);
                m.snap();
                let n = (secs * SR) as usize;
                let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
                for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
                    m.render_stereo(a, b);
                }
                let file = format!("{dir}/{}.wav", name.to_lowercase().replace(' ', "_"));
                write_wav(&file, &l, Some(&r));
                println!("{file}");
            }
            if mode == "demo" {
                for (preset, file, tod) in [("Garden", "chorus_garden", 0.28), ("Dawn chorus", "chorus_dawn", 0.24), ("Night woods", "chorus_night", 0.95)] {
                    let mut m = generators::create("birds", SR).unwrap();
                    m.load_preset(m.desc().preset_index(preset).unwrap());
                    m.set_input_by_name("activity", 0.9);
                    m.set_input_by_name("time_of_day", tod);
                    m.snap();
                    let n = (16.0 * SR) as usize;
                    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
                    for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
                        m.render_stereo(a, b);
                    }
                    let file = format!("{dir}/{file}.wav");
                    write_wav(&file, &l, Some(&r));
                    println!("{file}");
                }
            }
        }
        _ => panic!("unknown mode {mode}"),
    }
}
