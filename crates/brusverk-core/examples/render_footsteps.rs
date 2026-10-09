//! Footsteps, rendered the way a game drives them:
//!   cargo run -p brusverk-core --example render_footsteps --release -- out_dir
//!
//! Writes, as 48 kHz mono WAVs:
//! * `walk_<surface>.wav` and `run_<surface>.wav`: 10 s at 1.8 and 2.8 steps/s, one trigger() per
//!   footfall from a stride timer (as an animation event would);
//! * `demo.wav`: a walker on grass speeds up to a run, crosses onto wood, then snow, jumps and lands;
//! * `variety.wav`: a light walker in hard heels on stone, then a heavy walker in boots on metal;
//! * `rock_hit.wav` and `suspension_thud.wav`: existing events at full power, for comparing loudness.
use brusverk_core::generators::{self, foley::SURFACES};
use brusverk_core::Model;

const SR: f32 = 48000.0;

fn write(path: &str, x: &[f32]) {
    let spec = hound::WavSpec { channels: 1, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for s in x {
        w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
    }
    w.finalize().unwrap();
    println!("{path}");
}

fn surface(name: &str) -> f32 {
    SURFACES.iter().position(|s| *s == name).expect("surface") as f32 / 10.0
}

/// A game loop: 10 ms frames, trigger() whenever the stride timer comes round. `plan(t)` gives
/// (speed, surface, steps per second, land) at time t.
fn walk(m: &mut dyn Model, secs: f32, plan: impl Fn(f32) -> (f32, f32, f32, f32)) -> Vec<f32> {
    let frame = (SR * 0.01) as usize;
    let mut out = vec![0.0; (secs * SR) as usize];
    let mut next = 0.05;
    let mut jumped = false;
    for (i, chunk) in out.chunks_mut(frame).enumerate() {
        let t = i as f32 * 0.01;
        let (speed, surface, rate, land) = plan(t);
        m.set_input_by_name("speed", speed);
        m.set_input_by_name("surface", surface);
        if land > 0.0 && !jumped {
            // The walker was in the air: one landing, then walking resumes 0.6 s later.
            jumped = true;
            m.set_input_by_name("land", land);
            m.trigger();
            next = t + 0.6;
        } else if rate > 0.0 && t >= next {
            m.set_input_by_name("land", 0.0);
            m.trigger();
            next = t + 1.0 / rate;
        }
        m.render_mono(chunk);
    }
    out
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "footsteps_out".into());
    std::fs::create_dir_all(&dir).expect("create output dir");
    for name in SURFACES {
        for (gait, speed, rate) in [("walk", 0.3, 1.8), ("run", 0.7, 2.8)] {
            let mut m = generators::create("footstep", SR).unwrap();
            let x = walk(m.as_mut(), 10.0, |_| (speed, surface(name), rate, 0.0));
            write(&format!("{dir}/{gait}_{name}.wav"), &x);
        }
    }
    // Game control: grass walk, speed up to a run, onto wood, then snow, a jump, walk away.
    let mut m = generators::create("footstep", SR).unwrap();
    let demo = walk(m.as_mut(), 16.0, |t| {
        let speed = if t < 3.0 { 0.3 } else { (0.3 + (t - 3.0) * 0.25).min(0.75) };
        let rate = 1.8 + (speed - 0.3) / 0.45 * 1.0;
        let ground = if t < 6.5 { "grass" } else if t < 10.5 { "wood" } else { "snow" };
        let (rate, land) = if (12.6..13.4).contains(&t) { (0.0, if t > 13.1 { 0.8 } else { 0.0 }) } else { (rate, 0.0) };
        let speed = if t > 13.0 { 0.3 } else { speed };
        let rate = if t > 13.0 && rate > 0.0 { 1.8 } else { rate };
        (speed, surface(ground), rate, land)
    });
    write(&format!("{dir}/demo.wav"), &demo);
    // Walkers: a light walker in hard heels on stone, then a heavy one in boots on a metal floor.
    let mut m = generators::create("footstep", SR).unwrap();
    m.set_input_by_name("weight", 0.2);
    m.set_input_by_name("shoe", 1.0);
    let mut variety = walk(m.as_mut(), 4.0, |_| (0.3, surface("stone"), 2.0, 0.0));
    let mut m = generators::create("footstep", SR).unwrap();
    m.set_input_by_name("weight", 1.0);
    m.set_input_by_name("shoe", 0.5);
    variety.extend(walk(m.as_mut(), 4.0, |_| (0.25, surface("metal"), 1.6, 0.0)));
    write(&format!("{dir}/variety.wav"), &variety);
    for name in ["rock_hit", "suspension_thud"] {
        let mut m = generators::create(name, SR).unwrap();
        let mut out = vec![0.0; (6.0 * SR) as usize];
        for chunk in out.chunks_mut(SR as usize) {
            m.set_input_by_name("power", 1.0);
            m.trigger();
            m.render_mono(chunk);
        }
        write(&format!("{dir}/{name}.wav"), &out);
    }
}
