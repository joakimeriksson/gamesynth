//! Render the machinery family (`train`, `conveyor`, `press`, `clock`, `gears`, `music_box`,
//! `unlock`, `pneumatic`, `hydraulic`, `elevator`) the way games drive them, to mono WAVs, and
//! print their loudness.
//!
//!   cargo run -p brusverk-core --release --example render_machines -- <out_dir>
//!
//! writes the scenes `demo_train.wav` (pulling away over the joints, a horn, a curve, braking
//! to a stop with squeal and air), `demo_freight.wav` (a freight train rolling past a level
//! crossing), `demo_metro.wav`, `demo_factory.wav` (a conveyor line with a press, a hydraulic
//! arm and pneumatic pistons), `demo_clockwork.wav` (a clockwork puzzle: crank, ratchet, gears,
//! the lock giving way, a music box starting) and `demo_elevator.wav` (doors, a ride of three
//! floors, the chime, doors), plus one steady `ab_*.wav` per sound to compare with the
//! recordings in `target/refs/machines`.
use brusverk_core::generators;
use brusverk_core::render::{peak, rms};
use brusverk_core::Model;

const SR: f32 = 48000.0;
const CHUNK: usize = 64;

/// A steady sound to compare with a recording: (file, generator, preset, input keys, triggers).
type Steady = (&'static str, &'static str, Option<&'static str>, &'static [(f32, &'static str, f32)], &'static [f32]);

fn write(path: &str, x: &[f32]) {
    let spec = hound::WavSpec { channels: 1, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for &s in x {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

/// One generator in a scene: input keyframes (seconds, input, value; linear between keys of
/// the same input), trigger times, and its level in the mix.
struct Track {
    model: Box<dyn Model>,
    keys: Vec<(f32, &'static str, f32)>,
    triggers: Vec<f32>,
    gain: f32,
}

fn track(name: &str, preset: Option<&str>, params: &[(&str, f32)], keys: &[(f32, &'static str, f32)], triggers: &[f32], gain: f32) -> Track {
    let mut model = generators::create(name, SR).unwrap_or_else(|| panic!("no generator {name}"));
    if let Some(p) = preset {
        let i = model.desc().preset_index(p).unwrap_or_else(|| panic!("{name} has no preset {p}"));
        model.load_preset(i);
    }
    for (k, v) in params {
        assert!(model.set_param_by_name(k, *v), "{name} has no param {k}");
    }
    Track { model, keys: keys.to_vec(), triggers: triggers.to_vec(), gain }
}

/// Value of `input` at `t` from the keyframes (held before the first and after the last).
fn value_at(keys: &[(f32, &'static str, f32)], input: &str, t: f32) -> Option<f32> {
    let k: Vec<_> = keys.iter().filter(|k| k.1 == input).collect();
    let first = k.first()?;
    if t <= first.0 {
        return Some(first.2);
    }
    for w in k.windows(2) {
        if t <= w[1].0 {
            let u = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
            return Some(w[0].2 + (w[1].2 - w[0].2) * u);
        }
    }
    Some(k.last().unwrap().2)
}

fn scene(tracks: &mut [Track], secs: f32) -> Vec<f32> {
    let n = (secs * SR) as usize;
    let mut out = vec![0.0f32; n];
    let mut buf = vec![0.0f32; CHUNK];
    for tr in tracks.iter_mut() {
        let names: Vec<String> = tr.model.desc().inputs.iter().map(|i| i.name.clone()).collect();
        for name in &names {
            if let Some(v) = value_at(&tr.keys, name, 0.0) {
                tr.model.set_input_by_name(name, v);
            }
        }
        tr.model.snap();
        let mut next = 0;
        let mut pos = 0;
        while pos < n {
            let t = pos as f32 / SR;
            for name in &names {
                if let Some(v) = value_at(&tr.keys, name, t) {
                    tr.model.set_input_by_name(name, v);
                }
            }
            while next < tr.triggers.len() && tr.triggers[next] <= t {
                tr.model.trigger();
                next += 1;
            }
            let len = CHUNK.min(n - pos);
            tr.model.render_mono(&mut buf[..len]);
            for (o, b) in out[pos..pos + len].iter_mut().zip(&buf[..len]) {
                *o += b * tr.gain;
            }
            pos += len;
        }
    }
    out
}

fn report(dir: &str, name: &str, x: &[f32]) {
    write(&format!("{dir}/{name}.wav"), x);
    let db = |v: f32| 20.0 * v.max(1e-9).log10();
    println!("{name:<22} {:5.1} s  rms {:6.1} dB  peak {:6.1} dB", x.len() as f32 / SR, db(rms(x)), db(peak(x)));
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "target/machines".into());
    std::fs::create_dir_all(&dir).expect("out dir");

    // ---- scenes ----

    // A passenger train pulls away (the clack speeds up joint by joint), sounds its horn, takes a
    // curve, and brakes to a stop: shoe grind, the squeal as it stops, the air letting go.
    let train = scene(
        &mut [track(
            "train",
            None,
            &[],
            &[
                (0.0, "speed", 0.0),
                (0.0, "throttle", 0.0),
                (0.8, "throttle", 0.9),
                (1.5, "speed", 0.02),
                (9.0, "speed", 0.45),
                (13.0, "speed", 0.6),
                (13.0, "throttle", 0.9),
                (14.0, "throttle", 0.45),
                (2.5, "horn", 0.0),
                (2.6, "horn", 1.0),
                (4.0, "horn", 1.0),
                (4.1, "horn", 0.0),
                (4.5, "horn", 0.0),
                (4.6, "horn", 1.0),
                (5.0, "horn", 1.0),
                (5.1, "horn", 0.0),
                (9.5, "curve", 0.0),
                (10.5, "curve", 0.8),
                (14.5, "curve", 0.8),
                (15.5, "curve", 0.0),
                (16.0, "braking", 0.0),
                (16.3, "braking", 0.6),
                (16.0, "speed", 0.6),
                (16.5, "throttle", 0.0),
                (22.0, "speed", 0.08),
                (23.2, "speed", 0.0),
                (23.6, "braking", 0.6),
                (23.8, "braking", 0.0),
            ],
            &[],
            1.0,
        )],
        24.0,
    );
    report(&dir, "demo_train", &train);

    let freight = scene(&mut [track("train", Some("Freight trackside"), &[], &[(0.0, "speed", 0.3), (8.0, "speed", 0.45), (0.0, "throttle", 0.5)], &[], 1.0)], 8.0);
    report(&dir, "demo_freight", &freight);
    let metro = scene(
        &mut [track("train", Some("Metro"), &[], &[(0.0, "speed", 0.0), (0.0, "throttle", 1.0), (6.0, "speed", 0.7), (6.0, "throttle", 0.6), (3.5, "curve", 0.0), (5.0, "curve", 0.9), (8.0, "curve", 0.9)], &[], 1.0)],
        8.0,
    );
    report(&dir, "demo_metro", &metro);

    // A factory line: belt and rollers, a stamping press, a hydraulic arm moving parts, and a
    // pneumatic pusher kicking boxes off the line.
    let factory = scene(
        &mut [
            track("conveyor", None, &[], &[(0.0, "speed", 0.0), (1.5, "speed", 0.8), (0.0, "load", 0.6)], &[], 0.8),
            track("press", None, &[], &[(0.0, "rate", 0.6), (0.0, "force", 0.8)], &[], 0.8),
            track("hydraulic", Some("Robot arm"), &[], &[(0.0, "motion", 0.0), (2.0, "motion", 0.0), (2.2, "motion", 0.7), (3.3, "motion", 0.7), (3.4, "motion", 0.0), (5.5, "motion", 0.0), (5.7, "motion", 0.7), (6.8, "motion", 0.7), (6.9, "motion", 0.0), (0.0, "load", 0.5)], &[], 0.6),
            track("pneumatic", None, &[], &[(0.0, "power", 0.8), (0.0, "distance", 0.3)], &[3.9, 7.2], 0.6),
        ],
        8.0,
    );
    report(&dir, "demo_factory", &factory);

    // A clockwork puzzle: a clock ticks; the player cranks a ratchet, the gears turn faster and
    // faster, the lock gives way, and a music box starts to play.
    let clockwork = scene(
        &mut [
            track("clock", Some("Mantel clock"), &[], &[], &[], 0.8),
            track("gears", Some("Ratchet winch"), &[], &[(0.0, "speed", 0.0), (0.5, "speed", 0.0), (0.6, "speed", 0.7), (3.0, "speed", 0.7), (3.1, "speed", 0.0)], &[], 0.7),
            track("gears", None, &[], &[(0.0, "speed", 0.0), (0.6, "speed", 0.0), (3.0, "speed", 0.6), (4.2, "speed", 0.8), (4.6, "speed", 0.0)], &[], 0.7),
            track("unlock", None, &[], &[(0.0, "power", 1.0), (0.0, "size", 0.5)], &[3.4], 1.0),
            track("music_box", None, &[], &[(0.0, "wind", 0.0), (5.4, "wind", 0.0), (5.5, "wind", 1.0)], &[], 0.8),
        ],
        8.0,
    );
    report(&dir, "demo_clockwork", &clockwork);
    let rundown = scene(&mut [track("music_box", Some("Endless"), &[], &[(0.0, "wind", 1.0), (2.0, "wind", 1.0), (7.5, "wind", 0.0)], &[], 1.0)], 8.0);
    report(&dir, "demo_musicbox_rundown", &rundown);

    // A lift: doors close, the brake lifts, a ride past three floors, the chime, doors open.
    let elevator = scene(
        &mut [track(
            "elevator",
            None,
            &[],
            &[(0.0, "door", 1.0), (0.3, "door", 1.0), (1.8, "door", 0.0), (0.0, "speed", 0.0), (2.2, "speed", 0.0), (2.3, "speed", 1.0), (5.6, "speed", 1.0), (5.7, "speed", 0.0), (7.4, "door", 0.0), (8.9, "door", 1.0)],
            &[],
            1.0,
        )],
        9.0,
    );
    report(&dir, "demo_elevator", &elevator);

    // ---- steady sounds against the recordings ----
    let steady: [Steady; 16] = [
        ("ab_train_freight", "train", Some("Freight trackside"), &[(0.0, "speed", 0.42), (0.0, "throttle", 0.5)], &[]),
        ("ab_train_onboard", "train", None, &[(0.0, "speed", 0.5), (0.0, "throttle", 0.5)], &[]),
        ("ab_train_horn", "train", None, &[(0.0, "horn", 0.0), (0.3, "horn", 0.0), (0.35, "horn", 1.0), (2.0, "horn", 1.0), (2.05, "horn", 0.0), (3.5, "horn", 0.0), (3.55, "horn", 1.0), (5.5, "horn", 1.0), (5.55, "horn", 0.0)], &[]),
        ("ab_train_squeal", "train", None, &[(0.0, "speed", 0.25), (0.0, "curve", 1.0)], &[]),
        ("ab_clock_wall", "clock", None, &[], &[]),
        ("ab_clock_alarm", "clock", Some("Alarm clock"), &[], &[]),
        ("ab_clock_mantel", "clock", Some("Mantel clock"), &[], &[]),
        ("ab_conveyor", "conveyor", None, &[(0.0, "speed", 0.7), (0.0, "load", 0.5)], &[]),
        ("ab_press_hydraulic", "press", Some("Hydraulic press"), &[(0.0, "rate", 0.5), (0.0, "force", 0.7)], &[]),
        ("ab_windup_toy", "gears", Some("Wind-up toy"), &[(0.0, "speed", 1.0)], &[]),
        ("ab_ratchet", "gears", Some("Ratchet winch"), &[(0.0, "speed", 0.6)], &[]),
        ("ab_music_box", "music_box", None, &[], &[]),
        ("ab_air_release", "pneumatic", Some("Pressure release"), &[], &[0.2]),
        ("ab_scissor_lift", "hydraulic", Some("Scissor lift"), &[(0.0, "motion", 0.6), (0.0, "load", 0.6)], &[]),
        ("ab_elevator", "elevator", None, &[(0.0, "speed", 1.0)], &[]),
        ("ab_vault", "unlock", Some("Vault door"), &[], &[0.2]),
    ];
    for (name, generator, preset, keys, triggers) in steady {
        let x = scene(&mut [track(generator, preset, &[], keys, triggers, 1.0)], 8.0);
        report(&dir, name, &x);
    }
}
