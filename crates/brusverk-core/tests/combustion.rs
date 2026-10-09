//! The `combustion` engine after its retune against recordings (October 2026; the recordings
//! and what they show are in `tools/reference/README.md`, "Combustion"). Each test pins down
//! one thing the real engines all do and the first version did not, measured the same way as
//! the recordings were.

use brusverk_core::render::peak;
use brusverk_core::*;

const SR: f32 = 48000.0;
const CYCLE_ORDERS_MIN: f32 = -10.0;

const PRESETS: [&str; 9] = [
    "Default",
    "V8 muscle",
    "Motorbike",
    "Diesel truck",
    "Lawnmower",
    "Blown V8",
    "Buggy flat-four",
    "Dirt bike 2-stroke",
    "Rattletrap V8",
];

fn engine(preset: &str) -> Box<dyn Model> {
    let mut m = generators::create("combustion", SR).unwrap();
    let k = m.desc().preset_index(preset).unwrap_or_else(|| panic!("no preset {preset}"));
    m.load_preset(k);
    m
}

fn param(m: &dyn Model, name: &str) -> f32 {
    m.param(m.desc().param_index(name).unwrap_or_else(|| panic!("no param {name}")))
}

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

/// Steady state at `throttle` (revs follow it), the last `secs` of it.
fn steady(preset: &str, throttle: f32, load: f32, secs: f32) -> (Box<dyn Model>, Vec<f32>) {
    let mut m = engine(preset);
    m.set_input_by_name("throttle", throttle);
    m.set_input_by_name("load", load);
    m.snap();
    render(m.as_mut(), 0.5);
    let x = render(m.as_mut(), secs);
    (m, x)
}

/// Firing frequency at revs `rev` (0..1 between idle and max).
fn fire_hz(m: &dyn Model, rev: f32) -> f32 {
    let (idle, max) = (param(m, "engine/idle_rpm"), param(m, "engine/max_rpm"));
    let cyl = param(m, "engine/cylinders").round();
    let per_turn = if param(m, "engine/cycle") >= 0.5 { 1.0 } else { 0.5 };
    (idle + (max - idle) * rev) / 60.0 * cyl * per_turn
}

/// Power spectrum (Hann window, zero-padded radix-2 FFT): (bin width in Hz, power per bin).
fn spectrum(x: &[f32]) -> (f32, Vec<f32>) {
    let n = x.len().next_power_of_two();
    let mut re = vec![0.0f64; n];
    let mut im = vec![0.0f64; n];
    for (i, v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / x.len() as f64).cos();
        re[i] = *v as f64 * w;
    }
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (c, s) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
    (SR / n as f32, (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]) as f32).collect())
}

fn band(df: f32, p: &[f32], lo: f32, hi: f32) -> f32 {
    p.iter().enumerate().filter(|(k, _)| (*k as f32 * df) >= lo && (*k as f32 * df) < hi).map(|(_, v)| v).sum()
}

/// Share of the 30 Hz - 2 kHz energy that sits on the firing harmonics (+-4 %, at most
/// a fifth of the firing frequency): 1 = a clean buzz, real engines 0.15-0.7.
fn harmonic_share(x: &[f32], fire: f32) -> f32 {
    let (df, p) = spectrum(x);
    let total = band(df, &p, 30.0, 2000.0);
    let mut on = 0.0;
    let mut k = 1.0;
    while k * fire < 2000.0 {
        let w = (0.04 * k * fire).min(0.2 * fire);
        on += band(df, &p, (k * fire - w).max(30.0), k * fire + w);
        k += 1.0;
    }
    on / total
}

/// Energy on the cycle's own harmonics between the firing harmonics, over the energy on the
/// firing harmonics, dB. A four-stroke repeats every two turns; when its cylinders differ
/// from each other (as every recorded engine's do) that cycle shows up between the firing
/// harmonics.
fn cycle_orders_db(x: &[f32], fire: f32, cylinders: usize) -> f32 {
    let (df, p) = spectrum(x);
    let cycle = fire / cylinders as f32;
    let (mut on, mut between) = (0.0, 0.0);
    let mut k = 1;
    while k as f32 * cycle < 2000.0 {
        let f = k as f32 * cycle;
        let w = (0.03 * f).min(0.3 * cycle);
        let e = band(df, &p, f - w, f + w);
        if k % cylinders == 0 {
            on += e;
        } else {
            between += e;
        }
        k += 1;
    }
    10.0 * (between / on).log10()
}

/// How far the 1 ms envelope swings over one firing period (folded, p90 / p10 of 24 phase
/// bins, dB). Recordings: 0.3-2 dB. The first version clicked and fell silent: 4-30 dB at idle.
fn firing_swing_db(x: &[f32], fire: f32) -> f32 {
    let hop = ((SR * 0.001).min(SR / (fire * 16.0)) as usize).max(1);
    let env: Vec<f32> = x.chunks_exact(hop).map(|c| (c.iter().map(|v| v * v).sum::<f32>() / hop as f32).sqrt()).collect();
    let mut sum = [0.0f32; 24];
    let mut count = [0usize; 24];
    for (i, e) in env.iter().enumerate() {
        let b = (((i as f32 * hop as f32 / SR * fire) % 1.0) * 24.0) as usize % 24;
        sum[b] += e;
        count[b] += 1;
    }
    let mut fold: Vec<f32> = sum.iter().zip(count).map(|(s, c)| s / c.max(1) as f32).collect();
    fold.sort_by(|a, b| a.partial_cmp(b).unwrap());
    20.0 * (fold[21] / fold[2].max(1e-9)).log10()
}

/// Integrated loudness (ITU-R BS.1770: K-weighting, 400 ms blocks, absolute and relative
/// gates) of a stereo signal, LUFS: what `ffmpeg -af ebur128` reports.
fn lufs(left: &[f32], right: &[f32]) -> f32 {
    let k_weight = |x: &[f32]| -> Vec<f64> {
        let stages: [([f64; 3], [f64; 3]); 2] = [
            ([1.535_124_859_586_97, -2.691_696_189_406_38, 1.198_392_810_852_85], [1.0, -1.690_659_293_182_41, 0.732_480_774_215_85]),
            ([1.0, -2.0, 1.0], [1.0, -1.990_047_454_833_98, 0.990_072_250_366_21]),
        ];
        let mut y: Vec<f64> = x.iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0).trunc() as f64 / 32767.0).collect();
        for (b, a) in stages {
            let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
            for v in y.iter_mut() {
                let out = b[0] * *v + b[1] * x1 + b[2] * x2 - a[1] * y1 - a[2] * y2;
                (x2, x1, y2, y1) = (x1, *v, y1, out);
                *v = out;
            }
        }
        y
    };
    let (l, r) = (k_weight(left), k_weight(right));
    let (block, hop) = ((0.4 * SR) as usize, (0.1 * SR) as usize);
    let mut powers = Vec::new();
    let mut i = 0;
    while i + block <= l.len() {
        let z = (l[i..i + block].iter().map(|v| v * v).sum::<f64>() + r[i..i + block].iter().map(|v| v * v).sum::<f64>()) / block as f64;
        powers.push(z);
        i += hop;
    }
    let loud = |z: f64| -0.691 + 10.0 * z.log10();
    let gated: Vec<f64> = powers.iter().copied().filter(|z| loud(*z) > -70.0).collect();
    let rel = loud(gated.iter().sum::<f64>() / gated.len() as f64) - 10.0;
    let kept: Vec<f64> = gated.into_iter().filter(|z| loud(*z) > rel).collect();
    loud(kept.iter().sum::<f64>() / kept.len() as f64) as f32
}

/// The `render_models` sweep: throttle 0 -> 1 over 4 s, held 2 s with load raised to 1, back
/// to 0 over 4 s.
fn sweep(m: &mut dyn Model) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let (mut l, mut r) = ([0.0f32; 480], [0.0f32; 480]);
    let load_default = m.desc().inputs[1].default;
    for step in 0..1000 {
        let t = step as f32 / 100.0;
        m.set_input(0, if t < 4.0 { t / 4.0 } else if t < 6.0 { 1.0 } else { (10.0 - t) / 4.0 });
        m.set_input(1, if (4.0..6.0).contains(&t) { 1.0 } else { load_default });
        m.render_stereo(&mut l, &mut r);
        left.extend_from_slice(&l);
        right.extend_from_slice(&r);
    }
    (left, right)
}

#[test]
fn presets_keep_their_names() {
    // Dirtrace's docs/gamesynth_requests.md names the last four; games load presets by name.
    let m = generators::create("combustion", SR).unwrap();
    let names: Vec<&str> = m.desc().presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, PRESETS);
}

#[test]
fn loudness_is_unchanged_by_the_retune() {
    // Integrated loudness of each preset's render_models sweep before the retune (c7b396e,
    // measured with ffmpeg's ebur128). The retune keeps every preset within 1 LU of it.
    let before = [-9.2, -11.1, -11.8, -10.0, -14.5, -10.8, -12.3, -9.6, -9.2];
    for (preset, was) in PRESETS.iter().zip(before) {
        let (l, r) = sweep(engine(preset).as_mut());
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{preset}: not finite");
        assert!(peak(&l) <= 1.0, "{preset}: peak {}", peak(&l));
        let now = lufs(&l, &r);
        assert!((now - was).abs() <= 1.0, "{preset}: {now:.1} LUFS, was {was:.1}");
    }
}

#[test]
fn idle_does_not_fall_silent_between_firings() {
    for preset in PRESETS {
        let (m, x) = steady(preset, 0.0, 0.3, 2.0);
        let swing = firing_swing_db(&x, fire_hz(m.as_ref(), 0.0));
        assert!(swing < 6.0, "{preset}: the idle swings {swing:.1} dB over a firing (recordings 0.3-2)");
    }
}

#[test]
fn the_exhaust_is_not_a_clean_buzz() {
    // At full revs the first version put 86-95 % of its energy on the firing harmonics.
    for preset in PRESETS {
        let (m, x) = steady(preset, 1.0, 1.0, 1.0);
        let share = harmonic_share(&x, fire_hz(m.as_ref(), 1.0));
        assert!(share < 0.75, "{preset}: {:.0} % on the firing harmonics (recordings 15-70 %)", share * 100.0);
    }
}

/// Cycle orders (see `cycle_orders_db`) averaged as energy over three steady speeds, so a pipe
/// resonance that happens to sit on a firing harmonic at one speed does not decide it.
fn cycle_orders_over_revs(preset: &str) -> f32 {
    let mut sum = 0.0;
    for throttle in [0.1, 0.3, 0.5] {
        let (m, x) = steady(preset, throttle, 0.5, 1.5);
        let n = param(m.as_ref(), "engine/cylinders").round() as usize;
        sum += 10f32.powf(cycle_orders_db(&x, fire_hz(m.as_ref(), m.rpm().unwrap()), n) / 10.0);
    }
    10.0 * (sum / 3.0).log10()
}

#[test]
fn cylinders_differ_from_each_other() {
    // CYCLE_ORDERS_MIN sits between the first version and the retune.
    for preset in ["Default", "V8 muscle", "Motorbike", "Diesel truck", "Blown V8", "Buggy flat-four", "Rattletrap V8"] {
        let between = cycle_orders_over_revs(preset);
        assert!(between > CYCLE_ORDERS_MIN, "{preset}: the cycle between the firing harmonics is {between:.1} dB down");
    }
}

#[test]
fn diesel_clatters_and_the_muscle_v8_stays_dark() {
    let hi_share = |x: &[f32]| {
        let (df, p) = spectrum(x);
        band(df, &p, 2500.0, 20000.0) / band(df, &p, 30.0, 20000.0)
    };
    // Recorded diesels idle with 6-29 % of their energy above 2.5 kHz; the first version 0.3 %.
    let (_, diesel) = steady("Diesel truck", 0.0, 0.3, 1.5);
    assert!(hi_share(&diesel) > 0.04, "diesel idle: {:.1} % above 2.5 kHz", hi_share(&diesel) * 100.0);
    // A Mustang or Challenger roar has almost nothing up there.
    let (_, v8) = steady("V8 muscle", 1.0, 1.0, 1.5);
    assert!(hi_share(&v8) < 0.03, "V8 flat out: {:.1} % above 2.5 kHz", hi_share(&v8) * 100.0);
}

#[test]
fn lawnmower_runs_near_3000_rpm_in_the_low_mids() {
    // Recorded mowers run at 2600-3500 rpm (a single cylinder: 21-29 firings a second), loudest
    // between 100 and 500 Hz, and fall away by 20 dB towards 8 kHz.
    let (m, x) = steady("Lawnmower", 0.6, 0.6, 1.5);
    let fire = fire_hz(m.as_ref(), m.rpm().unwrap());
    assert!((21.0..=29.0).contains(&fire), "mower firing at {fire:.1} Hz");
    let (df, p) = spectrum(&x);
    let octave = |c: f32| band(df, &p, c / 2f32.sqrt(), c * 2f32.sqrt());
    let body = octave(125.0).max(octave(250.0)).max(octave(500.0));
    for c in [63.0, 1000.0, 2000.0, 4000.0, 8000.0] {
        assert!(octave(c) < body, "mower octave {c} Hz is louder than its body");
    }
    assert!(10.0 * (octave(8000.0) / body).log10() < -15.0, "mower top end too loud");
}

#[test]
fn revs_rise_and_fall_like_a_free_revving_engine() {
    // Real blips in neutral: up in 0.15-0.5 s, back to idle in 0.3-0.8 s. A heavy diesel is
    // slower but still settles within two seconds.
    for (preset, up, down) in [("Motorbike", 0.5, 1.0), ("Dirt bike 2-stroke", 0.4, 0.8), ("V8 muscle", 0.6, 1.2), ("Buggy flat-four", 0.7, 1.2), ("Diesel truck", 1.2, 2.0)] {
        let mut m = engine(preset);
        m.set_input_by_name("throttle", 1.0);
        render(m.as_mut(), up);
        assert!(m.rpm().unwrap() > 0.8, "{preset}: revs only {:.2} after {up} s flat out", m.rpm().unwrap());
        render(m.as_mut(), 1.0);
        m.set_input_by_name("throttle", 0.0);
        render(m.as_mut(), down);
        assert!(m.rpm().unwrap() < 0.1, "{preset}: revs still {:.2} {down} s after lifting off", m.rpm().unwrap());
    }
}

/// `cargo test --release -p brusverk-core --test combustion -- --ignored --nocapture report`:
/// the measurements above for every preset, for retuning.
#[test]
#[ignore]
fn report() {
    println!("{:<20} {:>9} {:>9} {:>9} {:>9} {:>9} {:>7}", "preset", "idle sw", "cyc revs", "harm idle", "harm full", "hi% idle", "LUFS");
    for preset in PRESETS {
        let (m, idle) = steady(preset, 0.0, 0.3, 2.0);
        let (m3, full) = steady(preset, 1.0, 1.0, 1.0);
        let (df, p) = spectrum(&idle);
        let hi = band(df, &p, 2500.0, 20000.0) / band(df, &p, 30.0, 20000.0);
        let (l, r) = sweep(engine(preset).as_mut());
        println!(
            "{:<20} {:>9.1} {:>9.1} {:>9.2} {:>9.2} {:>9.1} {:>7.1}",
            preset,
            firing_swing_db(&idle, fire_hz(m.as_ref(), 0.0)),
            if param(m.as_ref(), "engine/cylinders") > 1.5 { cycle_orders_over_revs(preset) } else { f32::NAN },
            harmonic_share(&idle, fire_hz(m.as_ref(), 0.0)),
            harmonic_share(&full, fire_hz(m3.as_ref(), 1.0)),
            hi * 100.0,
            lufs(&l, &r)
        );
    }
}
