//! A day in the woods, driven the way a game would drive it: the living-nature generators
//! (`birds`, `insects`, `frogs`, `leaves`, `thunder`) with `wind` and `rain`, their inputs moved
//! by a time-lapse script, mixed to one stereo WAV.
//!   cargo run -p brusverk-core --release --example render_wildlife -- wildlife_scene.wav
//! Prints the scene's chapters (start time and caption) for labelling a video.
use brusverk_core::{generators, Model};

const SR: f32 = 48000.0;
const BLOCK: usize = 256;
const SECS: f32 = 48.0;

/// Chapters: (start s, caption).
const CHAPTERS: &[(f32, &str)] = &[
    (0.0, "05:30 dawn chorus"),
    (8.0, "14:00 hot afternoon, cicadas"),
    (16.0, "20:30 warm evening, crickets speed up"),
    (24.0, "22:30 the pond at night"),
    (32.0, "storm coming: wind, leaves, far thunder"),
    (40.0, "storm overhead"),
];

struct Track {
    m: Box<dyn Model>,
    gain: f32,
    /// Per input: (time, value) keys, linearly interpolated.
    keys: Vec<(&'static str, Vec<(f32, f32)>)>,
}

fn track(name: &str, gain: f32, keys: Vec<(&'static str, Vec<(f32, f32)>)>) -> Track {
    let mut m = generators::create(name, SR).unwrap();
    for (input, k) in &keys {
        assert!(m.set_input_by_name(input, k[0].1), "{name} has no input {input}");
    }
    m.snap();
    Track { m, gain, keys }
}

fn at(keys: &[(f32, f32)], t: f32) -> f32 {
    if t <= keys[0].0 {
        return keys[0].1;
    }
    for w in keys.windows(2) {
        let ((t0, v0), (t1, v1)) = (w[0], w[1]);
        if t <= t1 {
            return v0 + (v1 - v0) * (t - t0) / (t1 - t0).max(1e-6);
        }
    }
    keys[keys.len() - 1].1
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "wildlife_scene.wav".into());
    let mut tracks = [
        // Birds: the dawn chorus, a quieter afternoon, the last songs at dusk, then the owl.
        track("birds", 1.0, vec![
            ("activity", vec![(0.0, 0.9), (8.0, 0.9), (10.0, 0.45), (18.0, 0.5), (24.0, 0.7), (32.0, 0.6), (36.0, 0.0)]),
            ("time_of_day", vec![(0.0, 0.23), (7.5, 0.24), (9.0, 0.58), (15.5, 0.6), (17.0, 0.84), (23.0, 0.86), (25.0, 0.95), (48.0, 0.98)]),
        ]),
        // Insects: cicadas in the afternoon heat, crickets at dusk speeding up as it stays warm.
        track("insects", 1.0, vec![
            ("temperature", vec![(0.0, 0.3), (8.0, 0.4), (10.0, 0.85), (16.0, 0.85), (17.0, 0.35), (23.0, 0.85), (32.0, 0.8), (40.0, 0.5)]),
            ("density", vec![(0.0, 0.0), (8.0, 0.0), (10.0, 0.9), (24.0, 0.9), (33.0, 0.6), (38.0, 0.0)]),
            ("daylight", vec![(0.0, 0.3), (8.0, 0.4), (9.5, 1.0), (15.5, 1.0), (17.0, 0.0)]),
        ]),
        // Frogs: the pond wakes up after dark and falls silent when the storm breaks.
        track("frogs", 1.0, vec![
            ("activity", vec![(0.0, 0.0), (18.0, 0.0), (22.0, 0.5), (25.0, 1.0), (33.0, 0.8), (38.0, 0.0)]),
            ("temperature", vec![(0.0, 0.7)]),
        ]),
        // The storm: wind and leaves rise together (same inputs), rain follows.
        track("wind", 0.6, vec![("strength", vec![(0.0, 0.08), (31.0, 0.12), (36.0, 0.45), (41.0, 0.62), (48.0, 0.68)]), ("gustiness", vec![(0.0, 0.6)])]),
        track("leaves", 0.8, vec![("strength", vec![(0.0, 0.18), (8.0, 0.25), (16.0, 0.15), (31.0, 0.15), (36.0, 0.45), (41.0, 0.62), (48.0, 0.68)]), ("gustiness", vec![(0.0, 0.6)])]),
        track("rain", 0.5, vec![("intensity", vec![(0.0, 0.0), (38.0, 0.0), (41.0, 0.3), (46.0, 0.6)]), ("shelter", vec![(0.0, 0.0)])]),
    ];
    // Thunder strikes: (time, distance, power), each on its own player as a game would.
    let strikes = [(32.5, 1.0, 0.8), (36.5, 0.75, 0.9), (40.2, 0.4, 1.0), (44.0, 0.0, 1.0)];
    let mut thunder: Vec<(f32, Box<dyn Model>, bool)> = strikes
        .iter()
        .map(|(t, d, p)| {
            let mut m = generators::create("thunder", SR).unwrap();
            m.set_input_by_name("distance", *d);
            m.set_input_by_name("power", *p);
            (*t, m, false)
        })
        .collect();
    for (t, c) in CHAPTERS {
        println!("{t:5.1}s  {c}");
    }
    let spec = hound::WavSpec { channels: 2, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(&out, spec).unwrap();
    let n = (SECS * SR) as usize;
    let (mut l, mut r, mut ml, mut mr) = ([0.0f32; BLOCK], [0.0f32; BLOCK], [0.0f32; BLOCK], [0.0f32; BLOCK]);
    let mut peak = 0.0f32;
    let start = std::time::Instant::now();
    let mut i = 0;
    while i < n {
        let k = BLOCK.min(n - i);
        let t = i as f32 / SR;
        ml[..k].iter_mut().chain(mr[..k].iter_mut()).for_each(|s| *s = 0.0);
        for tr in tracks.iter_mut() {
            for (input, keys) in &tr.keys {
                tr.m.set_input_by_name(input, at(keys, t));
            }
            tr.m.render_stereo(&mut l[..k], &mut r[..k]);
            for j in 0..k {
                ml[j] += l[j] * tr.gain;
                mr[j] += r[j] * tr.gain;
            }
        }
        for (when, m, fired) in thunder.iter_mut() {
            if !*fired && t >= *when {
                m.trigger();
                *fired = true;
            }
            if *fired {
                m.render_stereo(&mut l[..k], &mut r[..k]);
                for j in 0..k {
                    ml[j] += l[j];
                    mr[j] += r[j];
                }
            }
        }
        for j in 0..k {
            for s in [ml[j], mr[j]] {
                // A gentle bus limiter, as a game's master bus would have.
                let y = brusverk_core::math::limit(s);
                peak = peak.max(y.abs());
                w.write_sample((y * 32767.0) as i16).unwrap();
            }
        }
        i += k;
    }
    w.finalize().unwrap();
    let el = start.elapsed().as_secs_f32();
    println!("wrote {out}: {SECS} s, peak {peak:.2}, rendered {:.0}x real time (everything at once)", SECS / el);
}
