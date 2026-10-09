//! Bit-identity check: hashes 3 s of stereo output of every native generator preset (inputs at
//! 0.7, events triggered) and of every model file, one line each.
//!   cargo run -p brusverk-core --release --example hash_all -- models_dir [skip_name...]
use brusverk_core::{generators, GraphModel, Model};

const SR: f32 = 48000.0;

fn hash(m: &mut dyn Model) -> u64 {
    for i in 0..m.desc().inputs.len() {
        m.set_input(i, 0.7);
    }
    m.snap();
    m.trigger();
    let n = (3.0 * SR) as usize;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
        m.render_stereo(a, b);
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in l.iter().chain(&r) {
        for byte in s.to_bits().to_le_bytes() {
            h = (h ^ byte as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "models".into());
    let skip: Vec<String> = args.collect();
    for name in generators::NAMES {
        if skip.iter().any(|s| s == name) {
            continue;
        }
        let presets = generators::create(name, SR).unwrap().desc().presets.len();
        for p in 0..presets {
            let mut m = generators::create(name, SR).unwrap();
            m.load_preset(p);
            let label = m.desc().presets[p].name.clone();
            println!("{name}/{label} {:016x}", hash(m.as_mut()));
        }
    }
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
    files.sort();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let mut m = GraphModel::from_text(&text, SR).unwrap();
        println!("{} {:016x}", f.file_name().unwrap().to_string_lossy(), hash(&mut m));
    }
}
