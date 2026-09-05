//! Render every SFX preset for a few seeds to WAV files, so the engine can be auditioned
//! without Godot:  cargo run -p gamesynth-core --example render_sfx --release -- out_dir seeds
use gamesynth_core::render::render_one_shot;
use gamesynth_core::SfxPreset;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "sfx_out".into());
    let seeds: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(3);
    std::fs::create_dir_all(&dir).expect("create output dir");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    for preset in SfxPreset::ALL {
        for seed in 0..seeds {
            let patch = preset.generate(seed);
            let samples = render_one_shot(&patch, 48000.0, 8.0);
            let path = format!("{dir}/{}_{seed}.wav", preset.name().to_lowercase());
            let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
            for s in &samples {
                w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
            }
            w.finalize().unwrap();
            println!("{path}  {:.2}s", samples.len() as f32 / 48000.0);
        }
    }
}
