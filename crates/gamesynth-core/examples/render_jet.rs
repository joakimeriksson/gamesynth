//! Render a jet engine throttle sequence per preset to WAV:
//!   cargo run -p gamesynth-core --example render_jet --release -- out_dir
//! Sequence: 1.5 s idle, 3 s ramp to full, 2 s boost at speed, 3 s back to idle (with a
//! short damaged stretch), 1.5 s idle.
use gamesynth_core::{JetEngine, JetPreset};

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "jet_out".into());
    std::fs::create_dir_all(&dir).expect("create output dir");
    let sr = 48000u32;
    let spec = hound::WavSpec { channels: 1, sample_rate: sr, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    for preset in JetPreset::ALL {
        let mut e = JetEngine::new(sr as f32, preset.params());
        let path = format!("{dir}/jet_{}.wav", preset.name().to_lowercase());
        let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
        let mut buf = [0.0f32; 480]; // 10 ms
        let total = 11.0;
        let mut t = 0.0f32;
        while t < total {
            let (throttle, boost, speed, damage) = match t {
                t if t < 1.5 => (0.0, 0.0, 0.0, 0.0),
                t if t < 4.5 => ((t - 1.5) / 3.0, 0.0, (t - 1.5) / 3.0 * 0.6, 0.0),
                t if t < 6.5 => (1.0, 1.0, 0.6 + (t - 4.5) / 2.0 * 0.4, 0.0),
                t if t < 9.5 => (1.0 - (t - 6.5) / 3.0, 0.0, 1.0 - (t - 6.5) / 3.0, if t > 7.5 && t < 8.7 { 0.8 } else { 0.0 }),
                _ => (0.0, 0.0, 0.0, 0.0),
            };
            e.set_throttle(throttle);
            e.set_boost(boost);
            e.set_speed(speed);
            e.set_damage(damage);
            e.render_mono(&mut buf);
            for s in &buf {
                w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
            }
            t += 0.01;
        }
        w.finalize().unwrap();
        println!("{path}");
    }
}
