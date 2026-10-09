//! Render the material generators (`strike`, `roll`) the way a physics game drives them, to mono
//! WAVs, and print their loudness and cost.
//!
//!   cargo run -p brusverk-core --release --example render_materials -- <out_dir>
//!
//! writes, per material, `strike_<material>_single.wav` (one full-power hit) and
//! `strike_<material>_seq.wav` (hits at power 1, 0.5, 0.25 and 0.8), and the demos:
//! `demo_harder.wav` (one plank dropped harder and harder), `demo_sizes.wav` (glass, small to
//! large), `demo_floor.wav` (a stone on a hard to a soft floor), `demo_bounce.wav` (a rubber
//! ball, then a steel ball, dropped from a metre), `demo_rubble.wav` (60 pebbles, one shared
//! generator per pebble), `demo_roll.wav` (a wooden ball speeding up, rough ground, a skid) and
//! `demo_barrel.wav`. One hit with any settings:
//!
//!   ... --example render_materials -- hit out.wav <preset> <power> <size> <hardness> [group/param=v ...]
use brusverk_core::generators;
use brusverk_core::render::{peak, rms};
use brusverk_core::Model;

const SR: f32 = 48000.0;
const MATERIALS: [&str; 7] = ["Wood", "Metal", "Glass", "Stone", "Plastic", "Ceramic", "Rubber ball"];

fn write(path: &str, x: &[f32]) {
    let spec = hound::WavSpec { channels: 1, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for &s in x {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

fn strike(preset: &str) -> Box<dyn Model> {
    let mut m = generators::create("strike", SR).unwrap();
    let i = m.desc().preset_index(preset).unwrap_or_else(|| panic!("no strike preset {preset}"));
    m.load_preset(i);
    m
}

/// A hit on object `obj` at `t` seconds.
struct Hit {
    t: f32,
    obj: usize,
    power: f32,
    size: f32,
    hardness: f32,
}

fn hit(t: f32, obj: usize, power: f32, size: f32, hardness: f32) -> Hit {
    Hit { t, obj, power, size, hardness }
}

/// Mix a set of objects, each its own strike generator, retriggered on each of its hits.
fn render_hits(objects: &mut [Box<dyn Model>], hits: &[Hit], secs: f32) -> Vec<f32> {
    let n = (secs * SR) as usize;
    let mut out = vec![0.0f32; n];
    let mut buf = vec![0.0f32; 64];
    let mut next = 0;
    let mut pos = 0;
    while pos < n {
        while next < hits.len() && (hits[next].t * SR) as usize <= pos {
            let h = &hits[next];
            let m = &mut objects[h.obj];
            m.set_input_by_name("power", h.power);
            m.set_input_by_name("size", h.size);
            m.set_input_by_name("hardness", h.hardness);
            m.trigger();
            next += 1;
        }
        let len = 64.min(n - pos);
        for m in objects.iter_mut() {
            m.render_mono(&mut buf[..len]);
            for (o, b) in out[pos..pos + len].iter_mut().zip(&buf[..len]) {
                *o += b;
            }
        }
        pos += len;
    }
    out
}

/// A ball dropped from `h` metres: bounces lose `1 - e` of their speed each time.
fn bounces(obj: usize, t0: f32, h: f32, e: f32, size: f32, hardness: f32, until: f32) -> Vec<Hit> {
    let g = 9.81;
    let v_max = 5.0;
    let mut v = (2.0 * g * h).sqrt();
    let mut t = t0;
    let mut out = Vec::new();
    while t < until && v > 0.15 {
        out.push(hit(t, obj, (v / v_max).min(1.0), size, hardness));
        v *= e;
        t += 2.0 * v / g;
    }
    out
}

fn roll(preset: &str, secs: f32, path: impl Fn(f32) -> [f32; 4]) -> Vec<f32> {
    let mut m = generators::create("roll", SR).unwrap();
    let i = m.desc().preset_index(preset).unwrap();
    m.load_preset(i);
    let n = (secs * SR) as usize;
    let mut out = vec![0.0f32; n];
    for (k, chunk) in out.chunks_mut(256).enumerate() {
        let x = path(k as f32 * 256.0 / SR);
        for (i, v) in x.iter().enumerate() {
            m.set_input(i, *v);
        }
        m.render_mono(chunk);
    }
    out
}

fn hit_mode(args: &[String]) {
    let (path, preset) = (&args[1], &args[2]);
    let f = |i: usize| args[i].parse::<f32>().unwrap();
    let mut m = strike(preset);
    for kv in &args[6..] {
        let (k, v) = kv.split_once('=').unwrap();
        assert!(m.set_param_by_name(k, v.parse().unwrap()), "no param {k}");
    }
    let mut objs = vec![m];
    let out = render_hits(&mut objs, &[hit(0.05, 0, f(3), f(4), f(5))], 4.0);
    write(path, &out);
    println!("{path}: peak {:.3}", peak(&out));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("hit") {
        return hit_mode(&args);
    }
    let dir = args.first().cloned().unwrap_or_else(|| "materials_out".into());
    std::fs::create_dir_all(&dir).unwrap();
    let file = |name: &str| format!("{dir}/{name}");

    // Loudness against the existing event sounds, all at full power, point blank.
    println!("loudness at power 1 (median of 9 triggers): peak / RMS of the first 300 ms");
    let level = |m: &mut Box<dyn Model>| {
        let mut pk = Vec::new();
        let mut rm = Vec::new();
        for _ in 0..9 {
            m.set_input(0, 1.0);
            m.trigger();
            let mut out = vec![0.0; (SR * 0.3) as usize];
            m.render_mono(&mut out);
            pk.push(peak(&out));
            rm.push(rms(&out));
            let mut rest = vec![0.0; (SR * 12.0) as usize];
            m.render_mono(&mut rest);
        }
        pk.sort_by(f32::total_cmp);
        rm.sort_by(f32::total_cmp);
        (pk[4], rm[4])
    };
    for name in ["impact", "rock_hit"] {
        let mut m = generators::create(name, SR).unwrap();
        let (p, r) = level(&mut m);
        println!("  {name:<18} peak {p:.2}  rms {r:.3}");
    }
    for mat in MATERIALS {
        let mut m = strike(mat);
        m.set_input_by_name("hardness", 1.0);
        let (p, r) = level(&mut m);
        // The same below the output limiter, to see the level as designed.
        m.set_param_by_name("master/gain", 0.1);
        let (raw, _) = level(&mut m);
        println!("  strike {mat:<11} peak {p:.2}  rms {r:.3}  (unlimited peak {:.2})", raw * 10.0);
    }

    // Per material: one hit, and a sequence at falling and rising strength.
    for mat in MATERIALS {
        let slug = mat.split(' ').next().unwrap().to_lowercase();
        let mut objs = vec![strike(mat)];
        let single = render_hits(&mut objs, &[hit(0.05, 0, 1.0, 0.5, 1.0)], if mat == "Metal" { 8.0 } else { 3.0 });
        write(&file(&format!("strike_{slug}_single.wav")), &single);
        let mut objs = vec![strike(mat)];
        let seq = [hit(0.2, 0, 1.0, 0.5, 1.0), hit(2.2, 0, 0.5, 0.5, 1.0), hit(4.2, 0, 0.25, 0.5, 1.0), hit(6.0, 0, 0.8, 0.5, 1.0)];
        write(&file(&format!("strike_{slug}_seq.wav")), &render_hits(&mut objs, &seq, 8.0));
    }

    // Demos of game control.
    let mut objs = vec![strike("Wood")];
    let hits: Vec<Hit> = [0.06, 0.15, 0.3, 0.5, 0.75, 1.0].iter().enumerate().map(|(i, &p)| hit(0.2 + i as f32 * 1.3, 0, p, 0.55, 0.9)).collect();
    write(&file("demo_harder.wav"), &render_hits(&mut objs, &hits, 8.0));

    let mut objs: Vec<_> = (0..5).map(|_| strike("Glass")).collect();
    let hits: Vec<Hit> = [0.1, 0.3, 0.5, 0.7, 0.9].iter().enumerate().map(|(i, &s)| hit(0.2 + i as f32 * 1.55, i, 0.8, s, 1.0)).collect();
    write(&file("demo_sizes.wav"), &render_hits(&mut objs, &hits, 8.0));

    let mut objs: Vec<_> = (0..4).map(|_| strike("Stone")).collect();
    let hits: Vec<Hit> = [1.0, 0.7, 0.4, 0.0].iter().enumerate().map(|(i, &h)| hit(0.2 + i as f32 * 1.9, i, 0.9, 0.5, h)).collect();
    write(&file("demo_floor.wav"), &render_hits(&mut objs, &hits, 8.0));

    let mut objs = vec![strike("Rubber ball"), strike("Metal")];
    let mut hits = bounces(0, 0.2, 1.0, 0.78, 0.45, 1.0, 4.2);
    hits.extend(bounces(1, 4.3, 0.6, 0.6, 0.05, 1.0, 8.0));
    write(&file("demo_bounce.wav"), &render_hits(&mut objs, &hits, 8.0));

    // Rubble: 60 pebbles, each its own generator, landing over two seconds.
    let mut rng = 0x1234_5678u32;
    let mut rnd = move || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng as f32 / u32::MAX as f32
    };
    let mut objs: Vec<_> = (0..60).map(|_| strike("Stone")).collect();
    let mut hits: Vec<Hit> = (0..60).map(|i| hit(0.3 + 2.0 * rnd() * rnd(), i, 0.2 + 0.6 * rnd(), 0.05 + 0.3 * rnd(), 0.9)).collect();
    hits.push(hit(0.2, 0, 1.0, 0.75, 1.0));
    hits.sort_by(|a, b| a.t.total_cmp(&b.t));
    let start = std::time::Instant::now();
    let rubble = render_hits(&mut objs, &hits, 4.0);
    println!("rubble: 60 strike generators, 4 s rendered in {:.1} ms", start.elapsed().as_secs_f32() * 1000.0);
    write(&file("demo_rubble.wav"), &rubble.iter().map(|x| x * 0.6).collect::<Vec<_>>());

    let ramp = |t: f32, a: f32, b: f32, x0: f32, x1: f32| x0 + (x1 - x0) * ((t - a) / (b - a)).clamp(0.0, 1.0);
    write(
        &file("demo_roll.wav"),
        &roll("Wooden ball", 8.0, |t| {
            let speed = if t < 2.5 { ramp(t, 0.0, 2.5, 0.0, 0.8) } else if t < 6.0 { 0.8 } else { ramp(t, 6.0, 7.6, 0.8, 0.0) };
            let surface = ramp(t, 3.0, 3.6, 0.2, 0.9);
            let slide = if (5.0..5.9).contains(&t) { 1.0 } else { 0.0 };
            [speed, surface, 0.5, slide]
        }),
    );
    write(&file("demo_barrel.wav"), &roll("Barrel", 8.0, |t| [ramp(t, 0.0, 3.0, 0.0, 0.6).min(ramp(t, 5.0, 7.8, 0.6, 0.0)), 0.5, 0.85, 0.0]));

    // Each roll preset at a steady speed, for comparison with recordings.
    for (preset, size, speed) in [("Wooden ball", 0.5, 0.5), ("Glass marble", 0.1, 0.5), ("Bowling ball", 0.75, 0.45), ("Steel ball", 0.3, 0.5), ("Barrel", 0.85, 0.4), ("Boulder", 0.8, 0.4), ("Plastic ball", 0.5, 0.5)] {
        let slug = preset.to_lowercase().replace(' ', "_");
        write(&file(&format!("roll_{slug}.wav")), &roll(preset, 8.0, |t| [ramp(t, 0.0, 0.5, 0.0, speed), 0.4, size, 0.0]));
    }

    // Cost: many sounding strikes at once.
    let mut objs: Vec<_> = (0..100).map(|_| strike("Metal")).collect();
    for m in objs.iter_mut() {
        m.trigger();
    }
    let mut buf = vec![0.0; 512];
    let start = std::time::Instant::now();
    for _ in 0..(SR as usize / 512) {
        for m in objs.iter_mut() {
            m.render_mono(&mut buf);
        }
    }
    let t = start.elapsed().as_secs_f32();
    // t seconds of work for 100 strikes x 1 s of audio: each strike takes t % of a core.
    println!("cost: 100 ringing metal strikes (12 modes each) render 1 s in {:.1} ms, {:.3} % of a core per strike", t * 1000.0, t);
    let mut r = generators::create("roll", SR).unwrap();
    r.set_input(0, 0.7);
    r.set_input(3, 0.5);
    let start = std::time::Instant::now();
    for _ in 0..(10.0 * SR) as usize / 512 {
        r.render_mono(&mut buf);
    }
    println!("cost: one roll renders at {:.0}x real time", 10.0 / start.elapsed().as_secs_f32());
}
