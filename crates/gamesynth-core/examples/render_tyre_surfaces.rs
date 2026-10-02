//! The tyre on each surface at speed 0.7: 3 s rolling, then 3 s sliding (slip 0.9):
//!   cargo run -p gamesynth-core --example render_tyre_surfaces --release -- out_dir
use gamesynth_core::generators;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "tyre_out".into());
    std::fs::create_dir_all(&dir).expect("create output dir");
    let spec = hound::WavSpec { channels: 2, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    for (name, surface) in [("dirt", 0.0), ("gravel", 0.25), ("sand", 0.5), ("mud", 0.75), ("rock", 1.0)] {
        let mut m = generators::create("tyre", 48000.0).unwrap();
        m.set_input_by_name("speed", 0.7);
        m.set_input_by_name("surface", surface);
        m.snap();
        let mut w = hound::WavWriter::create(format!("{dir}/tyre_{name}.wav"), spec).expect("create wav");
        let (mut l, mut r) = ([0.0f32; 480], [0.0f32; 480]);
        for step in 0..600 {
            m.set_input_by_name("slip", if step < 300 { 0.05 } else { 0.9 });
            m.render_stereo(&mut l, &mut r);
            for (a, b) in l.iter().zip(&r) {
                w.write_sample((a.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
                w.write_sample((b.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
        println!("{dir}/tyre_{name}.wav");
    }
}
