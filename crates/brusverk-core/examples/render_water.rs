//! Water sounds, rendered the way a game drives them:
//!   cargo run -p brusverk-core --example render_water --release -- out_dir [demo|clips|all]
//!
//! Writes 48 kHz mono WAVs:
//! * `clips`: one file per sound for comparing with recordings: `splash_pebble`, `splash_stone`,
//!   `splash_dive`, `splash_bellyflop`, `splash_big` (a few triggers each), `swim_crawl`,
//!   `swim_breast`, `swim_tread`, `boat_moored`, `boat_rowing`, `boat_wake`, `underwater`,
//!   `drip_tap`, `drip_cave`, `pour_bottle`, `pour_glass`;
//! * `demo`: `water_demo.wav`, one scene: a pebble, a dive and a swim; a rowing boat speeding up
//!   into waves; going under water and back up; filling a bottle.
use brusverk_core::generators;
use brusverk_core::Model;

const SR: f32 = 48000.0;
const FRAME: f32 = 0.01;

fn write(path: &str, x: &[f32]) {
    let spec = hound::WavSpec { channels: 1, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for s in x {
        w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
    }
    w.finalize().unwrap();
    println!("{path}");
}

fn gen(name: &str, preset: &str) -> Box<dyn Model> {
    let mut m = generators::create(name, SR).expect("generator");
    if !preset.is_empty() {
        let i = m.desc().preset_index(preset).unwrap_or_else(|| panic!("{name}: no preset {preset}"));
        m.load_preset(i);
    }
    m
}

/// A game loop in 10 ms frames: `plan(t, m)` sets inputs (and may trigger) before each frame.
fn run(m: &mut dyn Model, secs: f32, mut plan: impl FnMut(f32, &mut dyn Model)) -> Vec<f32> {
    let frame = (SR * FRAME) as usize;
    let mut out = vec![0.0; (secs * SR) as usize];
    for (i, chunk) in out.chunks_mut(frame).enumerate() {
        plan(i as f32 * FRAME, m);
        m.render_mono(chunk);
    }
    out
}

/// Inputs by name; the first frame also snaps them.
fn set(m: &mut dyn Model, inputs: &[(&str, f32)]) {
    for (k, v) in inputs {
        assert!(m.set_input_by_name(k, *v), "{} has no input {k}", m.desc().name);
    }
}

/// Splashes of one kind at `times`, (power, size, flat) each.
fn splashes(preset: &str, secs: f32, hits: &[(f32, f32, f32, f32)]) -> Vec<f32> {
    let mut m = gen("splash", preset);
    run(m.as_mut(), secs, |t, m| {
        for &(at, power, size, flat) in hits {
            if (t - at).abs() < FRAME * 0.5 {
                set(m, &[("power", power), ("size", size), ("flat", flat), ("distance", 0.0)]);
                m.trigger();
            }
        }
    })
}

fn clips(dir: &str) {
    let w = |name: &str, x: Vec<f32>| write(&format!("{dir}/{name}.wav"), &x);
    w("splash_pebble", splashes("", 8.0, &[(0.3, 0.6, 0.0, 0.1), (1.6, 0.8, 0.05, 0.2), (2.9, 0.5, 0.02, 0.0), (4.1, 0.9, 0.1, 0.3), (5.4, 0.7, 0.0, 0.1), (6.6, 0.6, 0.08, 0.2)]));
    w("splash_stone", splashes("", 8.0, &[(0.3, 0.8, 0.35, 0.2), (2.3, 1.0, 0.4, 0.3), (4.4, 0.7, 0.3, 0.1), (6.3, 0.9, 0.38, 0.2)]));
    w("splash_dive", splashes("", 8.0, &[(0.4, 0.9, 0.6, 0.05), (4.4, 1.0, 0.62, 0.15)]));
    w("splash_bellyflop", splashes("", 8.0, &[(0.4, 0.9, 0.6, 1.0), (4.4, 1.0, 0.65, 0.9)]));
    w("splash_big", splashes("", 8.0, &[(0.4, 1.0, 0.9, 0.5), (4.4, 0.9, 1.0, 0.6)]));

    let swim = |preset: &str, pace: f32, effort: f32, tread: f32| {
        let mut m = gen("swim", preset);
        run(m.as_mut(), 10.0, |t, m| {
            if t == 0.0 {
                set(m, &[("pace", pace), ("effort", effort), ("tread", tread)]);
                m.snap();
            }
        })
    };
    w("swim_crawl", swim("", 0.6, 0.6, 0.0));
    w("swim_breast", swim("Breaststroke", 0.6, 0.6, 0.0));
    w("swim_tread", swim("", 0.0, 0.5, 1.0));

    let boat = |preset: &str, speed: f32, waves: f32, row: f32| {
        let mut m = gen("boat", preset);
        run(m.as_mut(), 12.0, |t, m| {
            if t == 0.0 {
                set(m, &[("speed", speed), ("waves", waves), ("row", row)]);
                m.snap();
            }
        })
    };
    w("boat_moored", boat("Moored", 0.0, 0.6, 0.0));
    w("boat_rowing", boat("", 0.3, 0.2, 0.6));
    w("boat_wake", boat("Sailboat", 0.8, 0.4, 0.0));

    let mut m = gen("underwater", "");
    w("underwater", run(m.as_mut(), 12.0, |t, m| {
        if t == 0.0 {
            set(m, &[("submerge", 1.0), ("breath", 0.5), ("motion", 0.3)]);
            m.snap();
        }
    }));

    let drip = |preset: &str, rate: f32, size: f32| {
        let mut m = gen("drip", preset);
        run(m.as_mut(), 10.0, |t, m| {
            if t == 0.0 {
                set(m, &[("rate", rate), ("size", size), ("pool", 1.0)]);
                m.snap();
            }
        })
    };
    w("drip_tap", drip("Leaky tap", 0.42, 0.5));
    w("drip_cave", drip("Cave", 0.35, 0.6));

    let pour = |preset: &str, secs: f32| {
        let mut m = gen("pour", preset);
        run(m.as_mut(), secs + 1.0, |t, m| {
            let flow = if t < secs { 0.7 } else { 0.0 };
            set(m, &[("flow", flow), ("fill", (t / secs).min(0.95))]);
        })
    };
    w("pour_bottle", pour("", 16.0));
    w("pour_glass", pour("Glass", 6.0));
}

/// The demo scene, about 60 s: every generator runs on its own track and they are summed.
fn demo(dir: &str) {
    let secs = 60.0;
    // 0-13 s: a pebble, a stone, then a dive and swimming away (crawl), slowing to tread water.
    let shore = splashes("", secs, &[(0.5, 0.6, 0.02, 0.1), (2.0, 0.8, 0.35, 0.2), (4.0, 1.0, 0.6, 0.1)]);
    let mut swimmer = gen("swim", "");
    let swim = run(swimmer.as_mut(), secs, |t, m| {
        let (pace, effort, tread) = match t {
            t if t < 5.0 => (0.0, 0.0, 0.0),
            t if t < 10.5 => (0.6, 0.6, 0.0),
            t if t < 13.5 => (0.0, 0.5, 1.0),
            _ => (0.0, 0.0, 0.0),
        };
        set(m, &[("pace", pace), ("effort", effort), ("tread", tread)]);
    });
    let mut swimmer_gain = vec![0.0; swim.len()];
    for (i, g) in swimmer_gain.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *g = if t < 13.5 { 1.0 } else { (1.0 - (t - 13.5) / 0.5).max(0.0) };
    }
    // 14-30 s: a rowing boat, the rower pulling harder, the boat speeding up into choppier water.
    let mut boat = gen("boat", "");
    let row = run(boat.as_mut(), secs, |t, m| {
        let u = ((t - 14.0) / 14.0).clamp(0.0, 1.0);
        let on = (14.0..31.0).contains(&t);
        set(m, &[("speed", if on { 0.1 + 0.8 * u } else { 0.0 }), ("waves", if on { 0.15 + 0.75 * u } else { 0.0 }), ("row", if on { 0.3 + 0.7 * u } else { 0.0 })]);
        if t == 14.0 {
            m.snap();
        }
    });
    // 30-45 s: under water (a splash going in), swim a little, and break the surface again.
    let dive = splashes("", secs, &[(30.5, 0.9, 0.55, 0.1)]);
    let mut under = gen("underwater", "");
    let uw = run(under.as_mut(), secs, |t, m| {
        let submerge = if !(30.0..44.5).contains(&t) {
            0.0
        } else if t < 31.0 {
            (t - 30.0).clamp(0.0, 1.0)
        } else if t < 42.0 {
            1.0
        } else {
            (1.0 - (t - 42.0) / 1.2).max(0.0)
        };
        set(m, &[("submerge", submerge), ("breath", 0.6), ("motion", if (33.0..39.0).contains(&t) { 0.7 } else { 0.2 })]);
        if t == 0.0 {
            m.snap();
        }
    });
    let mut uw_gain = vec![0.0; uw.len()];
    for (i, g) in uw_gain.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *g = if (30.0..44.5).contains(&t) { 1.0 } else if (44.5..45.0).contains(&t) { 1.0 - (t - 44.5) / 0.5 } else { 0.0 };
    }
    // 46-60 s: filling a bottle from a tap; a drip from the tap afterwards.
    let mut pour = gen("pour", "");
    let fill = run(pour.as_mut(), secs, |t, m| {
        let flow = if (46.0..57.0).contains(&t) { 0.7 } else { 0.0 };
        set(m, &[("flow", flow), ("fill", ((t - 46.0) / 11.5).clamp(0.0, 0.95))]);
    });
    let mut tap = gen("drip", "Leaky tap");
    let drip = run(tap.as_mut(), secs, |t, m| {
        set(m, &[("rate", if t > 57.3 { 0.4 } else { 0.0 }), ("size", 0.5), ("pool", 1.0)]);
    });
    let mix: Vec<f32> = (0..shore.len())
        .map(|i| {
            let t = i as f32 / SR;
            // The shore splashes are the swimmer's; the dive into the underwater scene is heard
            // from above and dunked as it goes under.
            let dive_gain = if t < 31.0 { 1.0 } else { 0.25 };
            // The boat rows out of the scene; the underwater bed sits under the rest.
            let boat_gain = ((30.5 - t) / 1.5).clamp(0.0, 1.0);
            let mix = shore[i] + swim[i] * swimmer_gain[i] + row[i] * boat_gain + dive[i] * dive_gain + uw[i] * uw_gain[i] * 0.5 + fill[i] + drip[i];
            mix * 0.8
        })
        .collect();
    write(&format!("{dir}/water_demo.wav"), &mix);
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "water_out".into());
    let what = std::env::args().nth(2).unwrap_or_else(|| "all".into());
    std::fs::create_dir_all(&dir).expect("create output dir");
    if what == "clips" || what == "all" {
        clips(&dir);
    }
    if what == "demo" || what == "all" {
        demo(&dir);
    }
}
