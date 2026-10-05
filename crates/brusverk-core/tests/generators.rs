use brusverk_core::generators;
use brusverk_core::render::{peak, rms};
use brusverk_core::Model;

const SR: f32 = 48000.0;

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

fn set_all(m: &mut dyn Model, v: f32) {
    for i in 0..m.desc().inputs.len() {
        m.set_input(i, v);
    }
}

#[test]
fn registry_is_consistent() {
    assert!(generators::NAMES.len() >= 43);
    let descs = generators::describe_all();
    assert_eq!(descs.len(), generators::NAMES.len());
    for (name, d) in generators::NAMES.iter().zip(&descs) {
        assert_eq!(*name, d.name);
        assert!(!d.inputs.is_empty(), "{name} has no inputs");
        assert!(d.params.iter().any(|p| p.name == "master/gain"), "{name} lacks master/gain");
        assert_eq!(d.presets[0].name, "Default");
        for p in &d.presets {
            assert_eq!(p.values.len(), d.params.len(), "{name} preset {}", p.name);
        }
        let mut names: Vec<_> = d.params.iter().map(|p| p.name.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), d.params.len(), "{name} has duplicate param names");
        assert!(generators::create(&name.to_uppercase(), SR).is_some());
    }
    assert!(generators::create("nope", SR).is_none());
}

#[test]
fn every_generator_and_preset_is_bounded_and_audible() {
    for name in generators::NAMES {
        let mut m = generators::create(name, SR).unwrap();
        for preset in 0..m.desc().presets.len() {
            assert!(m.load_preset(preset));
            let label = format!("{name}/{}", m.desc().presets[preset].name);
            set_all(m.as_mut(), 1.0);
            if m.desc().one_shot {
                // Events are judged at point-blank range.
                m.set_input(1, 0.0);
                assert!(render(m.as_mut(), 0.2).iter().all(|x| *x == 0.0), "{label} must be silent before its trigger");
                m.trigger();
                // The reported length must be an honest upper bound: finished by then, and not
                // wildly pessimistic (still audible in its second half).
                let length = m.length_secs().unwrap_or_else(|| panic!("{label} reports no length"));
                let out = render(m.as_mut(), length);
                assert!(out.iter().all(|x| x.is_finite()), "{label} produced NaN/inf");
                assert!(peak(&out) > 0.15 && peak(&out) <= 1.0, "{label} peak {}", peak(&out));
                assert!(m.is_finished(), "{label} still sounding after its reported length of {length:.2} s");
                assert!(peak(&out[out.len() / 3..]) > 1e-4, "{label} length {length:.2} s is far too long");
                continue;
            }
            m.snap();
            // Ocean waves take several seconds per cycle.
            let out = render(m.as_mut(), 6.0);
            assert!(out.iter().all(|x| x.is_finite()), "{label} produced NaN/inf");
            assert!(peak(&out) <= 1.0, "{label} peak {}", peak(&out));
            assert!(rms(&out[SR as usize..]) > 0.02, "{label} too quiet at full input: rms {}", rms(&out[SR as usize..]));
            set_all(m.as_mut(), 0.0);
            let out = render(m.as_mut(), 1.0);
            assert!(out.iter().all(|x| x.is_finite()), "{label} NaN at zero input");
        }
    }
}

#[test]
fn inputs_change_the_sound() {
    // Raising the primary input must raise the level for everything driven by intensity.
    for name in ["hover", "combustion", "motor", "piston", "rotor", "scrape", "beam", "tyre", "wind", "rain", "fire", "stream", "electric", "crowd", "radio"] {
        let level = |v: f32| {
            let mut m = generators::create(name, SR).unwrap();
            set_all(m.as_mut(), 0.5);
            m.set_input(0, v);
            m.snap();
            rms(&render(m.as_mut(), 3.0)[SR as usize..])
        };
        let (lo, hi) = (level(0.1), level(1.0));
        assert!(hi > lo * 1.3, "{name}: input 0 at 1.0 ({hi}) is not clearly louder than at 0.1 ({lo})");
    }
}

#[test]
fn params_and_inputs_are_clamped() {
    let mut m = generators::create("wind", SR).unwrap();
    let i = m.desc().param_index("howl/hz").unwrap();
    m.set_param(i, 1e9);
    assert_eq!(m.param(i), 4000.0);
    m.set_param(i, f32::NAN);
    assert_eq!(m.param(i), 4000.0);
    m.set_input(0, 7.0);
    assert_eq!(m.input(0), 1.0);
    m.set_input(0, f32::NAN);
    assert_eq!(m.input(0), 0.0);
    m.set_input(99, 1.0);
    assert!(m.set_input_by_name("strength", 0.3));
    assert!(!m.set_input_by_name("nope", 0.3));
    assert!(m.set_param_by_name("master/gain", 0.5));
}

#[test]
fn inertia_and_snap() {
    let mut m = generators::create("combustion", SR).unwrap();
    m.set_input(0, 1.0);
    let quiet = rms(&render(m.as_mut(), 0.1));
    let mut n = generators::create("combustion", SR).unwrap();
    n.set_input(0, 1.0);
    n.snap();
    let loud = rms(&render(n.as_mut(), 0.1));
    assert!(loud > quiet, "snap should skip the rev-up: {loud} vs {quiet}");
}

#[test]
fn whole_library_renders_in_real_time() {
    let mut all: Vec<_> = generators::NAMES.iter().map(|n| generators::create(n, SR).unwrap()).collect();
    for m in all.iter_mut() {
        set_all(m.as_mut(), 0.8);
    }
    let secs = 2.0;
    let mut buf = vec![0.0; 512];
    let start = std::time::Instant::now();
    for _ in 0..((secs * SR) as usize / 512) {
        for m in all.iter_mut() {
            m.render_mono(&mut buf);
        }
    }
    let elapsed = start.elapsed().as_secs_f32();
    println!("library: all {} generators together render at {:.1}x real time", all.len(), secs / elapsed);
    assert!(elapsed < secs * 0.5, "all {} generators took {elapsed}s for {secs}s", all.len());
}

fn event(name: &str, power: f32, distance: f32, m: &mut dyn Model) -> Vec<f32> {
    let _ = name;
    m.set_input(0, power);
    m.set_input(1, distance);
    m.trigger();
    render(m, 3.0)
}

#[test]
fn one_shots_vary_respond_to_power_and_distance() {
    for name in generators::NAMES {
        let mut m = generators::create(name, SR).unwrap();
        if !m.desc().one_shot {
            continue;
        }
        let a = event(name, 1.0, 0.0, m.as_mut());
        let b = event(name, 1.0, 0.0, m.as_mut());
        let weak = event(name, 0.3, 0.0, m.as_mut());
        let far = event(name, 1.0, 0.9, m.as_mut());
        // Two identical triggers must not be sample-identical (anti-repetition).
        let diff = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(diff > 0.01, "{name}: repeated triggers are identical");
        assert!(rms(&weak) < rms(&a) * 0.8, "{name}: power 0.3 ({}) is not quieter than 1.0 ({})", rms(&weak), rms(&a));
        assert!(rms(&far) < rms(&a) * 0.8, "{name}: distance does not attenuate");
        let crossings = |s: &[f32]| s.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f32 / s.len() as f32;
        assert!(crossings(&far[..SR as usize / 2]) < crossings(&a[..SR as usize / 2]), "{name}: distance does not dull the sound");
    }
}

#[test]
fn variation_zero_is_repeatable() {
    let mut m = generators::create("beep", SR).unwrap();
    let i = m.desc().param_index("shape/variation").unwrap();
    m.set_param(i, 0.0);
    let a = event("beep", 1.0, 0.0, m.as_mut());
    let b = event("beep", 1.0, 0.0, m.as_mut());
    let diff = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(diff < 1e-3, "UI tones must be able to repeat exactly: {diff}");
}

fn stereo(m: &mut dyn Model, secs: f32) -> (Vec<f32>, Vec<f32>) {
    let n = (secs * SR) as usize;
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    m.render_stereo(&mut l, &mut r);
    (l, r)
}

fn correlation(l: &[f32], r: &[f32]) -> f32 {
    let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    dot(l, r) / (dot(l, l) * dot(r, r)).sqrt().max(1e-12)
}

#[test]
fn width_zero_is_the_mono_sound() {
    for name in ["explosion", "impact", "pickup", "bell"] {
        let (mut a, mut b) = (generators::create(name, SR).unwrap(), generators::create(name, SR).unwrap());
        let w = a.desc().param_index("space/width").unwrap();
        a.set_param(w, 0.0);
        a.trigger();
        b.trigger();
        let (l, r) = stereo(a.as_mut(), 1.5);
        let mono = render(b.as_mut(), 1.5);
        assert!(l == r, "{name}: width 0 must be identical in both channels");
        assert!(l == mono, "{name}: width 0 must be sample-identical to the mono render");
    }
}

#[test]
fn stereo_events_are_wide_but_keep_their_level_and_centre() {
    // (generator, highest acceptable L/R correlation at its designed width)
    for (name, max_corr) in [("explosion", 0.6), ("shield_up", 0.6), ("mine_blast", 0.7), ("rocket", 0.8), ("impact", 0.97)] {
        let (mut wide, mut mono) = (generators::create(name, SR).unwrap(), generators::create(name, SR).unwrap());
        wide.trigger();
        mono.trigger();
        let (l, r) = stereo(wide.as_mut(), 2.0);
        let m = render(mono.as_mut(), 2.0);
        let c = correlation(&l, &r);
        assert!(c < max_corr, "{name}: L/R correlation {c:.3} is not wide enough (< {max_corr})");
        assert!(c > -0.2, "{name}: L/R correlation {c:.3} is out of phase");
        // Each channel carries about the level of the mono sound, and neither side dominates.
        let (rl, rr, rm) = (rms(&l), rms(&r), rms(&m));
        assert!((rl / rr).ln().abs() < 0.2, "{name}: unbalanced, left {rl:.3} right {rr:.3}");
        assert!(rl > rm * 0.7 && rl < rm * 1.3, "{name}: stereo level {rl:.3} strays from mono {rm:.3}");
        // Mono compatibility: the fold-down keeps most of the energy (no cancellation).
        let fold: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
        assert!(rms(&fold) > rm * 0.6, "{name}: fold-down {:.3} collapses against mono {rm:.3}", rms(&fold));
    }
    // UI tones stay essentially centred.
    for name in ["beep", "lock_on"] {
        let mut m = generators::create(name, SR).unwrap();
        m.trigger();
        let (l, r) = stereo(m.as_mut(), 1.0);
        assert!(correlation(&l, &r) > 0.97, "{name} should be narrow: {}", correlation(&l, &r));
    }
    // Continuous generators have no stereo image yet: both channels equal the mono render.
    let mut wind = generators::create("wind", SR).unwrap();
    wind.set_input(0, 0.8);
    let (l, r) = stereo(wind.as_mut(), 0.5);
    assert!(l == r && rms(&l) > 0.01);
}

#[test]
fn pitch_ratio_transposes_events() {
    let crossings = |s: &[f32]| s.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f32;
    let tone = |ratio: f32| {
        let mut m = generators::create("beep", SR).unwrap();
        let v = m.desc().param_index("shape/variation").unwrap();
        m.set_param(v, 0.0);
        m.set_pitch_ratio(ratio);
        m.trigger();
        crossings(&render(m.as_mut(), 0.08))
    };
    let (base, up) = (tone(1.0), tone(2.0));
    assert!((up / base - 2.0).abs() < 0.15, "an octave up should double the pitch: {base} -> {up}");
    let mut m = generators::create("beep", SR).unwrap();
    m.set_pitch_ratio(f32::NAN);
    m.trigger();
    assert!(render(m.as_mut(), 0.1).iter().all(|x| x.is_finite()));
}

// ---------------------------------------------------------------------------------------------
// Off-road requests (Dirtrace): game-driven revs, engine character, tyres
// ---------------------------------------------------------------------------------------------

fn engine(preset: &str) -> Box<dyn Model> {
    let mut m = generators::create("combustion", SR).unwrap();
    let k = m.desc().preset_index(preset).unwrap_or_else(|| panic!("no preset {preset}"));
    m.load_preset(k);
    m
}

fn set(m: &mut dyn Model, param: &str, v: f32) {
    assert!(m.set_param_by_name(param, v), "no param {param}");
}

/// Strongest frequency between `lo` and `hi` Hz (zero-padded DFT scan, 2 Hz steps).
fn dominant(x: &[f32], lo: f32, hi: f32) -> f32 {
    let mut best = (0.0, lo);
    let mut f = lo;
    while f <= hi {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (core::f32::consts::TAU * n as f32 / x.len() as f32).cos();
            let a = core::f32::consts::TAU * f * n as f32 / SR;
            re += v * w * a.cos();
            im += v * w * a.sin();
        }
        let m = re * re + im * im;
        if m > best.0 {
            best = (m, f);
        }
        f += 2.0;
    }
    best.1
}

fn band_energy(x: &[f32], lo: f32, hi: f32) -> f32 {
    // Crude band power: difference of two one-pole low-passes, averaged.
    let (a, b) = (1.0 - (-core::f32::consts::TAU * hi / SR).exp(), 1.0 - (-core::f32::consts::TAU * lo / SR).exp());
    let (mut ya, mut yb, mut acc) = (0.0f32, 0.0f32, 0.0f32);
    for v in x {
        ya += (v - ya) * a;
        yb += (v - yb) * b;
        acc += (ya - yb) * (ya - yb);
    }
    acc / x.len() as f32
}

#[test]
fn combustion_follows_a_game_driven_rpm_within_30_ms() {
    let mut m = engine("V8 muscle");
    set(m.as_mut(), "engine/external_rpm", 1.0);
    set(m.as_mut(), "engine/roughness", 0.0);
    m.set_input_by_name("throttle", 0.6);
    m.set_input_by_name("load", 0.6);
    m.set_input_by_name("rpm", 1.0);
    m.snap();
    render(m.as_mut(), 0.3);
    // V8 at 6200 rpm fires 8 / 2 * 6200 / 60 = 413 Hz.
    let high = dominant(&render(m.as_mut(), 0.25), 250.0, 600.0);
    assert!((high - 413.0).abs() < 25.0, "firing at full revs {high} Hz");
    assert!((m.rpm().unwrap() - 1.0).abs() < 0.01, "rpm() reports the revs");
    // Upshift: revs drop to 0.4 (700 + 0.4 * 5500 = 2900 rpm, 193 Hz). After 30 ms it is there.
    m.set_input_by_name("rpm", 0.4);
    render(m.as_mut(), 0.03);
    assert!((m.rpm().unwrap() - 0.4).abs() < 0.04, "after 30 ms revs are {}", m.rpm().unwrap());
    let low = dominant(&render(m.as_mut(), 0.25), 120.0, 300.0);
    assert!((low - 193.0).abs() < 20.0, "firing after the shift {low} Hz");
    // Without external rpm the same input does nothing: throttle drives the revs as before.
    let mut inert = engine("V8 muscle");
    inert.set_input_by_name("rpm", 1.0);
    inert.set_input_by_name("throttle", 0.0);
    render(inert.as_mut(), 0.5);
    assert!(inert.rpm().unwrap() < 0.01);
}

#[test]
fn two_stroke_fires_every_turn() {
    let rate = |cycle: f32| {
        let mut m = engine("Lawnmower");
        set(m.as_mut(), "engine/external_rpm", 1.0);
        set(m.as_mut(), "engine/cycle", cycle);
        set(m.as_mut(), "engine/roughness", 0.0);
        m.set_input_by_name("throttle", 0.9);
        m.set_input_by_name("rpm", 1.0);
        m.snap();
        render(m.as_mut(), 0.2);
        dominant(&render(m.as_mut(), 0.3), 20.0, 90.0)
    };
    // Single cylinder at 3600 rpm: 30 Hz four-stroke, 60 Hz two-stroke.
    let (four, two) = (rate(0.0), rate(1.0));
    assert!((four - 30.0).abs() < 4.0 && (two - 60.0).abs() < 6.0, "four-stroke {four} Hz, two-stroke {two} Hz");
}

#[test]
fn blower_whines_at_its_drive_ratio_and_boost_adds_drive() {
    let mut m = engine("Blown V8");
    set(m.as_mut(), "engine/external_rpm", 1.0);
    m.set_input_by_name("throttle", 0.8);
    m.set_input_by_name("rpm", 0.5);
    m.set_input_by_name("boost", 1.0);
    m.snap();
    render(m.as_mut(), 0.2);
    let x = render(m.as_mut(), 0.3);
    // 750 + 0.5 * 5750 = 3625 rpm -> 60.4 rev/s x 14 = 846 Hz.
    let whine = dominant(&x, 700.0, 1000.0);
    assert!((whine - 846.0).abs() < 20.0, "blower whine at {whine} Hz");
    let level = |boost: f32| {
        let mut e = engine("Buggy flat-four");
        e.set_input_by_name("throttle", 0.8);
        e.set_input_by_name("boost", boost);
        e.snap();
        render(e.as_mut(), 0.2);
        rms(&render(e.as_mut(), 0.5))
    };
    assert!(level(1.0) > level(0.0) * 1.1, "boost should push the engine harder");
}

#[test]
fn lifting_off_sets_off_backfires() {
    let peak_after_lift = |backfire: f32| {
        let mut worst = 0.0f32;
        for trial in 0..6 {
            let mut m = engine("Blown V8");
            set(m.as_mut(), "exhaust/backfire", backfire);
            set(m.as_mut(), "exhaust/overrun_burble", 0.0);
            set(m.as_mut(), "engine/rev_limiter", 0.0);
            m.set_input_by_name("throttle", 1.0);
            m.set_input_by_name("load", 1.0);
            m.snap();
            render(m.as_mut(), 0.5 + 0.03 * trial as f32);
            let before = peak(&render(m.as_mut(), 0.2));
            m.set_input_by_name("throttle", 0.0);
            let after = render(m.as_mut(), 0.5);
            // Revs fall away after a lift, so compare the bang against the engine at full throttle.
            worst = worst.max(peak(&after) / before);
        }
        worst
    };
    let (off, on) = (peak_after_lift(0.0), peak_after_lift(1.0));
    assert!(on > off * 1.25, "backfire bangs: {on:.2} x the running peak, without {off:.2}");
}

#[test]
fn rev_limiter_cuts_the_ignition_at_redline() {
    let dips = |limiter: f32| {
        let mut m = engine("Dirt bike 2-stroke");
        set(m.as_mut(), "engine/external_rpm", 1.0);
        set(m.as_mut(), "engine/rev_limiter", limiter);
        m.set_input_by_name("throttle", 1.0);
        m.set_input_by_name("rpm", 1.0);
        m.snap();
        render(m.as_mut(), 0.2);
        let x = render(m.as_mut(), 1.0);
        let win = (SR * 0.01) as usize;
        let env: Vec<f32> = x.chunks(win).map(rms).collect();
        let mean = env.iter().sum::<f32>() / env.len() as f32;
        env.iter().filter(|v| **v < 0.4 * mean).count()
    };
    let (free, limited) = (dips(0.0), dips(0.8));
    assert!(limited > free + 10, "limiter cuts: {limited} quiet 10 ms windows vs {free} without");
}

#[test]
fn damage_misfires_and_rattles() {
    let run = |damage: f32| {
        let mut m = engine("V8 muscle");
        set(m.as_mut(), "engine/roughness", 0.0);
        m.set_input_by_name("throttle", 0.5);
        m.set_input_by_name("damage", damage);
        m.snap();
        render(m.as_mut(), 0.3);
        render(m.as_mut(), 1.0)
    };
    let (clean, wrecked) = (run(0.0), run(1.0));
    // Misfires make the level lurch from window to window.
    let wobble = |x: &[f32]| {
        let env: Vec<f32> = x.chunks((SR * 0.02) as usize).map(rms).collect();
        let mean = env.iter().sum::<f32>() / env.len() as f32;
        (env.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / env.len() as f32).sqrt() / mean
    };
    assert!(wobble(&wrecked) > wobble(&clean) * 1.5, "misfires: {:.3} vs {:.3}", wobble(&wrecked), wobble(&clean));
    // Rattle and exhaust-leak hiss add top end.
    assert!(band_energy(&wrecked, 2000.0, 8000.0) > band_energy(&clean, 2000.0, 8000.0) * 1.5);
}

#[test]
fn tyre_surfaces_have_their_own_character() {
    let tyre = |surface: f32, speed: f32, slip: f32| {
        let mut m = generators::create("tyre", SR).unwrap();
        m.set_input_by_name("speed", speed);
        m.set_input_by_name("slip", slip);
        m.set_input_by_name("surface", surface);
        m.snap();
        render(m.as_mut(), 0.3);
        render(m.as_mut(), 1.5)
    };
    let (dirt, gravel, sand, mud, rock) = (tyre(0.0, 0.7, 0.1), tyre(0.25, 0.7, 0.1), tyre(0.5, 0.7, 0.1), tyre(0.75, 0.7, 0.1), tyre(1.0, 0.7, 0.1));
    // Measured on recordings (tools/reference/tyres): a tyre is a mid-range sound. On gravel 3
    // to 5 % of the energy is above 2.5 kHz and the loudest octaves are 500 Hz and 1 kHz. A
    // bright hiss up there is what made the old tyre sound like rain.
    let share = |x: &[f32], lo: f32, hi: f32| {
        let w = &x[..(SR * 0.5) as usize];
        spectral_energy(w, lo, hi, 50.0) / spectral_energy(w, 50.0, 9000.0, 50.0)
    };
    for (name, x) in [("dirt", &dirt), ("gravel", &gravel), ("sand", &sand), ("mud", &mud), ("rock", &rock)] {
        let top = share(x, 2500.0, 9000.0);
        assert!(top < 0.09, "{name} should not hiss: {:.3} of its energy is above 2.5 kHz", top);
    }
    let (g_low, g_mid, g_high) = (share(&gravel, 50.0, 300.0), share(&gravel, 350.0, 1400.0), share(&gravel, 1450.0, 9000.0));
    assert!(g_mid > g_high * 1.5 && g_mid > g_low, "gravel lives in the middle: low {g_low:.2}, mid {g_mid:.2}, high {g_high:.2}");
    // Each surface keeps a character of its own: dirt rumbles, gravel is the most mid-heavy,
    // sand is the softest.
    assert!(share(&dirt, 50.0, 300.0) > share(&gravel, 50.0, 300.0) * 1.3, "dirt rumbles more than gravel");
    assert!(share(&gravel, 350.0, 1400.0) > share(&dirt, 350.0, 1400.0), "gravel has more middle than dirt");
    assert!(rms(&sand) < rms(&gravel) * 0.8, "sand is softer than gravel: {:.3} vs {:.3}", rms(&sand), rms(&gravel));
    // Squeal is a rock/tarmac thing, and only when sliding: the squeal band (around
    // rock/squeal_hz) gains a lot on rock, and gravel's slide noise does not match it.
    let squeal = |x: &[f32]| share(x, 900.0, 1300.0);
    let rock_slide = tyre(1.0, 0.7, 0.9);
    assert!(squeal(&rock_slide) > squeal(&rock) * 2.5, "sliding on rock squeals: {:.3} vs rolling {:.3}", squeal(&rock_slide), squeal(&rock));
    // And it is a tone: in the squeal band the strongest bin stands far above the mean, which
    // gravel's broadband slide noise never does.
    let peaky = |x: &[f32]| {
        let w = &x[(SR * 0.5) as usize..(SR * 0.65) as usize];
        let mags: Vec<f32> = (0..=80).map(|k| {
            let f = 850.0 + 8.0 * k as f32;
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (n, v) in w.iter().enumerate() {
                let a = core::f32::consts::TAU * f * n as f32 / SR;
                re += v * a.cos();
                im += v * a.sin();
            }
            (re * re + im * im).sqrt()
        }).collect();
        mags.iter().cloned().fold(0.0, f32::max) / (mags.iter().sum::<f32>() / mags.len() as f32)
    };
    // The tread hum is tonal too (on every surface), so silence it to compare just the slides.
    let slide_only = |surface: f32| {
        let mut m = generators::create("tyre", SR).unwrap();
        assert!(m.set_param_by_name("tread/hum", 0.0));
        m.set_input_by_name("speed", 0.7);
        m.set_input_by_name("slip", 0.9);
        m.set_input_by_name("surface", surface);
        m.snap();
        render(m.as_mut(), 0.3);
        render(m.as_mut(), 1.0)
    };
    let (rock_only, gravel_only) = (slide_only(1.0), slide_only(0.25));
    assert!(peaky(&rock_only) > peaky(&gravel_only) * 1.3, "squeal is a tone: {:.2} vs {:.2}", peaky(&rock_only), peaky(&gravel_only));
    // (Gravel's slide noise sits in the same band, as it does in recordings; the tone is the difference.)
    for (name, x) in [("dirt", &dirt), ("gravel", &gravel), ("sand", &sand), ("mud", &mud), ("rock", &rock)] {
        assert!(x.iter().all(|v| v.is_finite()) && peak(x) <= 1.0 && rms(x) > 0.02, "{name}: rms {} peak {}", rms(x), peak(x));
    }
    // Standing still is silent.
    assert!(rms(&tyre(0.25, 0.0, 0.0)) < 0.01);
}

#[test]
fn festival_crowd_has_air_horns() {
    let run = |preset: &str| {
        let mut m = generators::create("crowd", SR).unwrap();
        let k = m.desc().preset_index(preset).unwrap();
        m.load_preset(k);
        m.set_input(0, 0.8);
        m.set_input(1, 0.8);
        m.snap();
        render(m.as_mut(), 12.0)
    };
    // An air horn is a loud harmonic tone in 330..480 Hz. Scan 0.15 s windows: the most tonal
    // window (strongest bin / mean bin) shows a horn; a crowd of voices never gets that peaky.
    let most_tonal = |x: &[f32]| {
        let n = (SR * 0.15) as usize;
        x.chunks(n).filter(|w| w.len() == n).map(|w| {
            let mags: Vec<f32> = (0..=40).map(|k| {
                let f = 300.0 + 6.0 * k as f32;
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (i, v) in w.iter().enumerate() {
                    let a = core::f32::consts::TAU * f * i as f32 / SR;
                    re += v * a.cos();
                    im += v * a.sin();
                }
                (re * re + im * im).sqrt()
            }).collect();
            mags.iter().cloned().fold(0.0, f32::max) / (mags.iter().sum::<f32>() / mags.len() as f32)
        }).fold(0.0, f32::max)
    };
    let (festival, stadium) = (run("Festival"), run("Stadium"));
    assert!(most_tonal(&festival) > most_tonal(&stadium) * 1.5, "horns: {:.2} vs {:.2}", most_tonal(&festival), most_tonal(&stadium));
    assert!(peak(&festival) <= 1.0);
}

// ---------------------------------------------------------------------------------------------
// Physical piston engine
// ---------------------------------------------------------------------------------------------

/// A piston engine reduced to its exhaust pulses: no noise, no variation, no other sources.
fn bare_piston(layout: f32, rev: f32) -> Box<dyn Model> {
    let mut m = generators::create("piston", SR).unwrap();
    for (name, v) in [
        ("engine/layout", layout), ("engine/external_rpm", 1.0), ("engine/idle_rpm", 900.0), ("engine/max_rpm", 6900.0), ("engine/roughness", 0.0), ("engine/cam_lope", 0.0),
        ("pulse/turbulence", 0.0), ("exhaust/unequal_ms", 0.0), ("exhaust/interference", 0.0), ("exhaust/crossover", 0.0), ("exhaust/overrun_burble", 0.0),
        ("intake/level", 0.0), ("mechanical/valvetrain", 0.0), ("mechanical/block", 0.0), ("mechanical/fan", 0.0), ("stereo/width", 1.0),
    ] {
        set(m.as_mut(), name, v);
    }
    m.set_input_by_name("throttle", 0.5);
    m.set_input_by_name("rpm", rev);
    m.snap();
    m
}

/// Share of the energy on harmonics of the 720 degree cycle that are not multiples of `every`.
fn off_harmonic_share(x: &[f32], cycle_hz: f32, every: usize) -> f32 {
    let (mut on, mut off) = (0.0f32, 0.0f32);
    for k in 1..=24 {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, v) in x.iter().enumerate() {
            let a = core::f32::consts::TAU * cycle_hz * k as f32 * n as f32 / SR;
            re += v * a.cos();
            im += v * a.sin();
        }
        *(if k % every == 0 { &mut on } else { &mut off }) += re * re + im * im;
    }
    off / (on + off)
}

#[test]
fn piston_matches_combustion_as_a_drop_in() {
    let (p, c) = (generators::create("piston", SR).unwrap(), generators::create("combustion", SR).unwrap());
    let names = |m: &dyn Model| m.desc().inputs.iter().map(|i| i.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(p.as_ref()), names(c.as_ref()), "same inputs in the same order");
    for shared in ["engine/idle_rpm", "engine/max_rpm", "engine/external_rpm", "engine/rev_limiter", "exhaust/backfire", "blower/level", "blower/ratio", "damage/misfire", "damage/wear", "master/gain"] {
        assert!(p.desc().param_index(shared).is_some(), "piston lacks {shared}");
    }
}

#[test]
fn piston_fires_at_the_crank_speed_it_is_given() {
    // 900 + 0.35 * 6000 = 3000 rpm: 25 cycles a second, a V8 fires 200 times a second.
    let mut m = bare_piston(0.0, 0.35);
    set(m.as_mut(), "exhaust/crossover", 1.0);
    render(m.as_mut(), 0.3);
    let hz = dominant(&render(m.as_mut(), 0.4), 150.0, 250.0);
    assert!((hz - 200.0).abs() < 6.0, "V8 firing at 3000 rpm: {hz} Hz");
    assert!((m.rpm().unwrap() - 0.35).abs() < 0.01);
    // An inline four at the same speed fires half as often.
    let mut four = bare_piston(2.0, 0.35);
    render(four.as_mut(), 0.3);
    let hz = dominant(&render(four.as_mut(), 0.4), 60.0, 140.0);
    assert!((hz - 100.0).abs() < 4.0, "inline four firing at 3000 rpm: {hz} Hz");
}

#[test]
fn crossplane_rumbles_and_flatplane_does_not() {
    // One bank of a cross-plane V8 gets pulses 270, 180, 90 and 180 degrees apart: every
    // harmonic of the cycle. One bank of a flat-plane V8 fires evenly: only every fourth.
    let bank = |layout: f32| {
        let mut m = bare_piston(layout, 0.35);
        let n = (SR * 1.5) as usize;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        m.render_stereo(&mut l, &mut r);
        off_harmonic_share(&l[n / 3..], 25.0, 4)
    };
    let (cross, flat) = (bank(0.0), bank(1.0));
    assert!(cross > 0.5, "one bank of a cross-plane V8 should be mostly half-order rumble: {cross:.2}");
    assert!(flat < 0.2, "one bank of a flat-plane V8 fires evenly: {flat:.2}");
    // And as shipped, the rumble survives a mono mix at speed (measured on a real V8: 0.92).
    let mut stock = generators::create("piston", SR).unwrap();
    set(stock.as_mut(), "engine/external_rpm", 1.0);
    stock.set_input_by_name("throttle", 0.25);
    stock.set_input_by_name("rpm", (3000.0 - 750.0) / (6500.0 - 750.0));
    stock.snap();
    render(stock.as_mut(), 0.5);
    let share = off_harmonic_share(&render(stock.as_mut(), 1.5), 25.0, 8);
    assert!(share > 0.6, "default V8 at 3000 rpm, mono: rumble share {share:.2}");
}

#[test]
fn piston_stereo_puts_a_bank_on_each_side() {
    let run = |layout: f32, width: f32| {
        let mut m = generators::create("piston", SR).unwrap();
        set(m.as_mut(), "engine/layout", layout);
        set(m.as_mut(), "stereo/width", width);
        m.set_input_by_name("throttle", 0.6);
        m.snap();
        render(m.as_mut(), 0.3);
        stereo(m.as_mut(), 1.0)
    };
    let (l, r) = run(0.0, 0.6);
    let c = correlation(&l, &r);
    assert!(c < 0.9 && c > 0.0, "a V8's banks should be heard apart: correlation {c:.2}");
    assert!((rms(&l) / rms(&r)).ln().abs() < 0.25, "balanced: {:.3} vs {:.3}", rms(&l), rms(&r));
    let (l, r) = run(0.0, 0.0);
    assert!(l == r, "width 0 is mono");
    let mut mono = generators::create("piston", SR).unwrap();
    set(mono.as_mut(), "stereo/width", 0.0);
    mono.set_input_by_name("throttle", 0.6);
    mono.snap();
    render(mono.as_mut(), 0.3);
    assert!(render(mono.as_mut(), 1.0) == l, "the mono render is the stereo one at width 0");
    // One exhaust (inline four): both sides carry the same pulses.
    let (l, r) = run(2.0, 0.6);
    assert!(correlation(&l, &r) > 0.85, "an inline four has one pipe: {:.2}", correlation(&l, &r));
}

#[test]
fn piston_pipe_length_moves_the_formants() {
    // Flow noise through the pipe shows its resonances: a quarter-wave pipe of 1 m rings at
    // 130 Hz and 390 Hz, one of 2 m at 65, 195 and 325 Hz.
    let shape = |metres: f32| {
        let mut m = bare_piston(6.0, 0.1);
        set(m.as_mut(), "pulse/turbulence", 1.0);
        set(m.as_mut(), "exhaust/length_m", metres);
        set(m.as_mut(), "exhaust/header_m", 0.2);
        set(m.as_mut(), "exhaust/resonance", 0.9);
        set(m.as_mut(), "exhaust/muffling", 0.0);
        render(m.as_mut(), 0.3);
        let x = render(m.as_mut(), 1.0);
        // Spectral energy in a band, by direct transform every 3 Hz (`band_energy` is too leaky here).
        let energy = |lo: f32, hi: f32| {
            let mut sum = 0.0f32;
            let mut f = lo;
            while f <= hi {
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (n, v) in x.iter().enumerate() {
                    let a = core::f32::consts::TAU * f * n as f32 / SR;
                    re += v * a.cos();
                    im += v * a.sin();
                }
                sum += re * re + im * im;
                f += 3.0;
            }
            sum
        };
        energy(180.0, 210.0) / energy(115.0, 145.0)
    };
    let (short, long) = (shape(1.0), shape(2.0));
    assert!(long > short * 1.5, "195 Hz against 130 Hz: 2 m pipe {long:.2}, 1 m pipe {short:.2}");
}

/// Spectral energy between `lo` and `hi` Hz by direct transform every `step` Hz.
fn spectral_energy(x: &[f32], lo: f32, hi: f32, step: f32) -> f32 {
    let mut sum = 0.0f32;
    let mut f = lo;
    while f <= hi {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, v) in x.iter().enumerate() {
            let a = core::f32::consts::TAU * f * n as f32 / SR;
            re += v * a.cos();
            im += v * a.sin();
        }
        sum += re * re + im * im;
        f += step;
    }
    sum
}

#[test]
fn piston_v12_and_two_stroke_fire_at_their_own_rate() {
    // 3000 rpm is 25 cycles a second: a V12 fires 300 times a second.
    let mut v12 = bare_piston(7.0, 0.35);
    set(v12.as_mut(), "exhaust/crossover", 1.0);
    render(v12.as_mut(), 0.3);
    let hz = dominant(&render(v12.as_mut(), 0.4), 240.0, 360.0);
    assert!((hz - 300.0).abs() < 8.0, "V12 firing at 3000 rpm: {hz} Hz");
    // A two-stroke V8 fires all eight every turn: 400 a second, not 200.
    let mut two = bare_piston(0.0, 0.35);
    set(two.as_mut(), "exhaust/crossover", 1.0);
    set(two.as_mut(), "engine/cycle", 1.0);
    render(two.as_mut(), 0.3);
    let hz = dominant(&render(two.as_mut(), 0.4), 150.0, 450.0);
    assert!((hz - 400.0).abs() < 10.0, "two-stroke V8 firing at 3000 rpm: {hz} Hz");
}

#[test]
fn engine_brake_barks_only_on_a_closed_throttle() {
    let run = |throttle: f32, jake: f32| {
        let mut m = bare_piston(4.0, 0.6);
        set(m.as_mut(), "engine/jake_brake", jake);
        m.set_input_by_name("throttle", throttle);
        m.snap();
        render(m.as_mut(), 0.5);
        render(m.as_mut(), 0.5)
    };
    let (coast, brake) = (rms(&run(0.0, 0.0)), rms(&run(0.0, 1.0)));
    assert!(brake > coast * 2.0, "engine brake {brake:.3} against coasting {coast:.3}");
    assert_eq!(run(0.6, 0.0), run(0.6, 1.0), "the brake must not touch an engine under power");
}

#[test]
fn turbo_whistles_lags_and_blows_off() {
    let turbo = |throttle: f32, blowoff: f32| {
        let mut m = bare_piston(4.0, 1.0);
        for (name, v) in [("turbo/level", 1.0), ("turbo/hz", 5000.0), ("turbo/lag_s", 0.5), ("turbo/blowoff", blowoff)] {
            set(m.as_mut(), name, v);
        }
        m.set_input_by_name("throttle", throttle);
        m.snap();
        m
    };
    // Snapped at full throttle and full revs the turbine is at speed.
    let mut m = turbo(1.0, 0.0);
    let hz = dominant(&render(m.as_mut(), 0.2)[4800..], 4800.0, 5200.0);
    assert!((hz - 5000.0).abs() < 30.0, "whistle at full spool: {hz} Hz");
    // From a closed throttle it has to spool up first.
    let mut m = turbo(0.0, 0.0);
    m.set_input_by_name("throttle", 1.0);
    let early = spectral_energy(&render(m.as_mut(), 0.1), 4700.0, 5100.0, 20.0);
    render(m.as_mut(), 2.5);
    let late = spectral_energy(&render(m.as_mut(), 0.1), 4700.0, 5100.0, 20.0);
    assert!(late > early * 20.0, "whistle should arrive with the boost: early {early:.3}, late {late:.3}");
    // Lifting off a spooled turbo dumps the boost: a burst of air around 2.6 kHz.
    let lift = |blowoff: f32| {
        let mut m = turbo(1.0, blowoff);
        render(m.as_mut(), 0.2);
        m.set_input_by_name("throttle", 0.0);
        spectral_energy(&render(m.as_mut(), 0.25)[2400..], 2000.0, 3200.0, 20.0)
    };
    let (with, without) = (lift(1.0), lift(0.0));
    // (The engine's own harmonics share this band, so the bar is a doubling, not silence.)
    assert!(with > without * 2.5, "blow-off: {with:.3} against {without:.3}");
}

// ---- hockey ----

fn crowd(preset: &str) -> Box<dyn Model> {
    let mut m = generators::create("crowd", SR).unwrap();
    let k = m.desc().preset_index(preset).unwrap_or_else(|| panic!("no preset {preset}"));
    m.load_preset(k);
    m.set_input_by_name("size", 0.9);
    m.set_input_by_name("excitement", 0.12);
    m.snap();
    m
}

#[test]
fn arena_crowd_is_not_bright() {
    // Recordings of hockey crowds have under 2 % of their energy above 2.5 kHz, cheering or
    // booing. The outdoor Stadium preset has several times that; indoors it read as hiss.
    let top = |preset: &str| {
        let mut m = crowd(preset);
        m.set_input_by_name("excitement", 1.0);
        m.snap();
        render(m.as_mut(), 0.5);
        let x = render(m.as_mut(), 0.5);
        spectral_energy(&x, 2500.0, 9000.0, 50.0) / spectral_energy(&x, 50.0, 9000.0, 50.0)
    };
    let (arena, stadium) = (top("Arena"), top("Stadium"));
    assert!(arena < 0.06 && arena < stadium * 0.6, "Arena {arena:.3}, Stadium {stadium:.3}");
}

#[test]
fn crowd_groans_once_boos_while_held_and_swells_for_a_goal() {
    // Each step: (input value, seconds). Returns the level of each step. The crowd underneath
    // is the same from run to run, so a run without the reaction is an exact baseline.
    let run = |input: &str, steps: &[(f32, f32)]| {
        let mut m = crowd("Arena");
        steps.iter().map(|(v, secs)| { m.set_input_by_name(input, *v); rms(&render(m.as_mut(), *secs)) }).collect::<Vec<f32>>()
    };
    // Groan: starts when the input rises, lasts about a second, ends by itself though the input stays up.
    let steps = [(0.0, 1.0), (1.0, 0.15), (1.0, 0.6), (1.0, 2.5), (1.0, 1.0)];
    let (base, groan) = (run("groan", &steps.map(|(_, s)| (0.0, s))), run("groan", &steps));
    assert!(groan[2] > base[2] * 1.3, "groan {:.3} against {:.3} without", groan[2], base[2]);
    assert!(groan[4] < base[4] * 1.03, "the groan should be over: {:.3} against {:.3}", groan[4], base[4]);
    // Boo: there while held, gone a couple of seconds after.
    let steps = [(0.0, 1.0), (1.0, 1.0), (1.0, 1.0), (0.0, 2.5), (0.0, 1.0)];
    let (base, boo) = (run("boo", &steps.map(|(_, s)| (0.0, s))), run("boo", &steps));
    assert!(boo[2] > base[2] * 1.3 && boo[4] < base[4] * 1.03, "boo {:.3} / {:.3}, afterwards {:.3} / {:.3}", boo[2], base[2], boo[4], base[4]);
    // Goal: still building after a quarter of a second, full a second later, and it takes seconds to settle.
    let steps = [(0.0, 1.0), (1.0, 0.25), (1.0, 1.0), (1.0, 1.0), (0.0, 1.0), (0.0, 9.0), (0.0, 1.0)];
    let (base, goal) = (run("goal", &steps.map(|(_, s)| (0.0, s))), run("goal", &steps));
    assert!(goal[3] > goal[1] * 1.25 && goal[3] > base[3] * 1.6, "swell: early {:.3}, full {:.3}, without {:.3}", goal[1], goal[3], base[3]);
    assert!(goal[4] > base[4] * 1.3 && goal[6] < base[6] * 1.05, "a second after {:.3} / {:.3}, settled {:.3} / {:.3}", goal[4], base[4], goal[6], base[6]);
}

#[test]
fn crowd_chant_claps_on_the_beat() {
    // With the murmur off, the chant is claps on eighths 0, 2, 4, 5, 6 of a two second bar.
    let mut m = crowd("Arena");
    for (param, v) in [("murmur/level", 0.0), ("cheer/level", 0.0), ("cheer/roar", 0.0), ("arena/room", 0.0)] {
        set(m.as_mut(), param, v);
    }
    render(m.as_mut(), 0.5);
    m.set_input_by_name("chant", 1.0);
    let x = render(m.as_mut(), 4.0);
    let slot = |bar: usize, eighth: usize| {
        // The input is smoothed, so the bar starts a few tens of milliseconds late: look late in each slot.
        let start = ((bar as f32 * 2.0 + eighth as f32 * 0.25 + 0.06) * SR) as usize;
        rms(&x[start..start + (0.12 * SR) as usize])
    };
    for bar in 0..2 {
        let claps = [0, 2, 4, 5, 6].map(|e| slot(bar, e));
        let rests = [1, 3, 7].map(|e| slot(bar, e));
        let (quietest_clap, loudest_rest) = (claps.iter().cloned().fold(f32::MAX, f32::min), rests.iter().cloned().fold(0.0, f32::max));
        assert!(quietest_clap > loudest_rest * 2.0, "bar {bar}: claps {claps:?}, rests {rests:?}");
    }
}

#[test]
fn skate_strides_glides_and_stops() {
    let skate = |preset: Option<&str>, push: f32, edge: f32| {
        let mut m = generators::create("skate", SR).unwrap();
        if let Some(p) = preset {
            let k = m.desc().preset_index(p).unwrap();
            m.load_preset(k);
        }
        m.set_input_by_name("speed", 0.7);
        m.set_input_by_name("push", push);
        m.set_input_by_name("edge", edge);
        m.snap();
        render(m.as_mut(), 3.0)
    };
    let (striding, gliding, stopping) = (skate(None, 1.0, 0.0), skate(None, 0.0, 0.0), skate(None, 0.0, 1.0));
    // Strides are separate cuts: the level in 50 ms windows swings from loud to nearly nothing.
    let windows: Vec<f32> = striding.chunks((0.05 * SR) as usize).map(rms).collect();
    let (loud, soft) = (windows.iter().cloned().fold(0.0, f32::max), windows.iter().cloned().fold(f32::MAX, f32::min));
    assert!(loud > soft * 6.0, "strides should come and go: {loud:.3} to {soft:.3}");
    assert!(rms(&gliding) < rms(&striding) * 0.3, "gliding is almost silent: {:.3} against {:.3}", rms(&gliding), rms(&striding));
    assert!(rms(&stopping) > rms(&gliding) * 4.0, "a stop sprays: {:.3} against {:.3}", rms(&stopping), rms(&gliding));
    // The default keeps its top down for a whole match of four skaters; "Close up" is the
    // recording, which has about a fifth of its energy above 2.5 kHz.
    let top = |x: &[f32]| spectral_energy(&x[..(SR * 1.5) as usize], 2500.0, 9000.0, 100.0) / spectral_energy(&x[..(SR * 1.5) as usize], 100.0, 9000.0, 100.0);
    let (default, close) = (top(&striding), top(&skate(Some("Close up"), 1.0, 0.0)));
    assert!(default < 0.16 && close > default * 1.4, "default {default:.3}, close up {close:.3}");
    // The spray of a stop is dull, like snow.
    assert!(top(&stopping) < 0.06, "stop spray {:.3}", top(&stopping));
}

#[test]
fn hockey_events_are_dull_and_the_right_length() {
    // A puck on the boards, measured: almost nothing above 2 kHz. None of these may hiss or tick bright.
    for name in ["puck_stick", "puck_boards", "puck_glass", "puck_post", "puck_pad", "buzzer"] {
        let mut m = generators::create(name, SR).unwrap();
        let x = event(name, 1.0, 0.0, m.as_mut());
        let top = spectral_energy(&x, 2500.0, 9000.0, 100.0) / spectral_energy(&x, 100.0, 9000.0, 100.0);
        // The post is a ping of steel, so it is allowed a little more top than the knocks and thuds.
        let limit = if name == "puck_post" { 0.2 } else { 0.1 };
        assert!(top < limit, "{name}: {top:.3} of its energy is above 2.5 kHz");
    }
    // Horn: swells (it is not at full level in its first tenth of a second) and lasts over two seconds.
    let mut horn = generators::create("goal_horn", SR).unwrap();
    horn.trigger();
    let x = render(horn.as_mut(), 4.0);
    let at = |t: f32| rms(&x[(t * SR) as usize..((t + 0.1) * SR) as usize]);
    assert!(at(0.0) < at(1.0) * 0.5, "the horn should swell: {:.3} then {:.3}", at(0.0), at(1.0));
    assert!(at(2.0) > at(1.0) * 0.7 && at(3.6) < at(1.0) * 0.2, "about 2.5 s: at 2 s {:.3}, at 3.6 s {:.3}", at(2.0), at(3.6));
    // Buzzer: flat for about 1.2 s.
    let mut buzzer = generators::create("buzzer", SR).unwrap();
    buzzer.trigger();
    let x = render(buzzer.as_mut(), 3.0);
    let at = |t: f32| rms(&x[(t * SR) as usize..((t + 0.1) * SR) as usize]);
    assert!((at(0.2) / at(0.9) - 1.0).abs() < 0.25, "flat: {:.3} and {:.3}", at(0.2), at(0.9));
    assert!(at(1.0) > at(0.2) * 0.7 && at(1.8) < at(0.2) * 0.25, "about 1.2 s: at 1 s {:.3}, at 1.8 s {:.3}", at(1.0), at(1.8));
}

// ---- Doppler: continuous generators played faster or slower ----

/// Render `secs` of `m` through a resampler at `ratio`, in game-sized blocks.
fn resampled(m: &mut dyn Model, ratio: f32, secs: f32) -> Vec<f32> {
    use brusverk_core::resample::Resampler;
    let mut rs = Resampler::new();
    let n = (secs * SR) as usize;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    for (cl, cr) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
        rs.process(ratio, cl, cr, |a, b| m.render_stereo(a, b));
    }
    l
}

#[test]
fn resampling_shifts_every_frequency_and_costs_nothing_at_one() {
    // An engine at fixed revs, as Dirtrace measures it: the firing frequency follows the ratio.
    let engine = || {
        let mut m = bare_piston(0.0, 0.35);
        set(m.as_mut(), "exhaust/crossover", 1.0);
        render(m.as_mut(), 0.3);
        m
    };
    let base = dominant(&resampled(engine().as_mut(), 1.0, 0.4), 150.0, 400.0);
    let up = dominant(&resampled(engine().as_mut(), 1.5, 0.4), 150.0, 400.0);
    let down = dominant(&resampled(engine().as_mut(), 0.85, 0.4), 150.0, 400.0);
    assert!((up / base - 1.5).abs() < 0.03, "x1.5 should be +7.02 semitones: {base} -> {up} Hz");
    assert!((down / base - 0.85).abs() < 0.03, "x0.85: {base} -> {down} Hz");
    // A ratio of 1 is the generator itself, sample for sample.
    let (mut a, mut b) = (engine(), engine());
    let (direct, _) = stereo(a.as_mut(), 0.5);
    assert!(resampled(b.as_mut(), 1.0, 0.5) == direct, "ratio 1 must be a pass-through");
}

#[test]
fn resampling_follows_a_moving_ratio_without_clicks() {
    // A sine source: a pass-by that sweeps the ratio 1.2 -> 0.85 over a second, block by block.
    use brusverk_core::resample::Resampler;
    let mut rs = Resampler::new();
    let mut phase = 0.0f32;
    let (mut out, mut l, mut r) = (Vec::new(), vec![0.0f32; 480], vec![0.0f32; 480]);
    for k in 0..100 {
        let ratio = 1.2 - 0.35 * (k as f32 / 99.0);
        rs.process(ratio, &mut l, &mut r, |a, b| {
            for (x, y) in a.iter_mut().zip(b.iter_mut()) {
                *x = phase.sin();
                *y = *x;
                phase = (phase + core::f32::consts::TAU * 440.0 / SR) % core::f32::consts::TAU;
            }
        });
        out.extend_from_slice(&l);
    }
    // The biggest step between samples of a 440 Hz sine at x1.2 is 2 pi 528 / 48000, about 0.069.
    let jump = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(jump < 0.075, "a click: a step of {jump:.3}");
    let start = dominant(&out[..4800], 400.0, 600.0);
    let end = dominant(&out[out.len() - 4800..], 300.0, 500.0);
    assert!((start - 523.0).abs() < 10.0 && (end - 377.0).abs() < 10.0, "sweep {start} -> {end} Hz");
}

#[test]
fn only_events_transpose_by_themselves() {
    for name in generators::NAMES {
        let m = generators::create(name, SR).unwrap();
        assert_eq!(m.transposes(), m.desc().one_shot, "{name}: events transpose, continuous generators are resampled");
    }
}

// ---- snow and ice (Dirtrace's snow track) ----

#[test]
fn tyre_snow_and_ice_have_their_own_character() {
    let tyre = |snow: f32, slip: f32, squeak: f32| {
        let mut m = generators::create("tyre", SR).unwrap();
        set(m.as_mut(), "snow/squeak", squeak);
        m.set_input_by_name("speed", 0.6);
        m.set_input_by_name("slip", slip);
        m.set_input_by_name("snow", snow);
        m.snap();
        render(m.as_mut(), 0.3);
        render(m.as_mut(), 1.5)
    };
    let share = |x: &[f32], lo: f32, hi: f32| {
        let w = &x[..(SR * 0.5) as usize];
        spectral_energy(w, lo, hi, 50.0) / spectral_energy(w, 50.0, 9000.0, 50.0)
    };
    let (packed, powder, ice, ice_slide) = (tyre(1.0 / 3.0, 0.05, 0.5), tyre(2.0 / 3.0, 0.05, 0.5), tyre(1.0, 0.05, 0.5), tyre(1.0, 0.85, 0.5));
    // Recordings of tyres on snow and ice: about 5 % of the energy above 2.5 kHz. Nothing here may hiss.
    for (name, x) in [("packed", &packed), ("powder", &powder), ("ice", &ice), ("ice sliding", &ice_slide)] {
        assert!(share(x, 2500.0, 9000.0) < 0.09, "{name}: {:.3} above 2.5 kHz", share(x, 2500.0, 9000.0));
        assert!(rms(x) > 0.02 && peak(x) <= 1.0, "{name}: rms {} peak {}", rms(x), peak(x));
    }
    // Powder is a soft, deep hush: far darker than packed snow.
    assert!(share(&powder, 50.0, 300.0) > share(&packed, 50.0, 300.0) * 1.5, "powder should be the low one");
    // On ice, sliding brings the studs' scrape in, between 1 and 3 kHz.
    assert!(share(&ice_slide, 1000.0, 3000.0) > share(&ice, 1000.0, 3000.0) * 1.3, "ice sliding should scrape");
    // Cold snow squeaks: with snow/squeak up, packed snow carries short tones.
    // (About two squeaks a second at this speed, so listen for a few seconds.)
    let long = |squeak: f32| {
        let mut m = generators::create("tyre", SR).unwrap();
        set(m.as_mut(), "snow/squeak", squeak);
        m.set_input_by_name("speed", 0.6);
        m.set_input_by_name("snow", 1.0 / 3.0);
        m.snap();
        render(m.as_mut(), 5.0)
    };
    let (cold, mild) = (long(1.0), long(0.0));
    let tone = spectral_energy(&cold, 950.0, 1550.0, 10.0) / spectral_energy(&mild, 950.0, 1550.0, 10.0);
    assert!(tone > 1.1, "squeaks should add tone near 1.2 kHz: {tone:.2}x");
    // Snow 0 is bare ground, unchanged.
    let mut bare = generators::create("tyre", SR).unwrap();
    bare.set_input_by_name("speed", 0.6);
    bare.snap();
    let mut zero = generators::create("tyre", SR).unwrap();
    zero.set_input_by_name("speed", 0.6);
    zero.set_input_by_name("snow", 0.0);
    zero.snap();
    assert!(render(bare.as_mut(), 0.5) == render(zero.as_mut(), 0.5));
}

#[test]
fn blizzard_is_a_low_buffeting_storm() {
    // As the game plays it: no hiss, a little whistle. Snowstorm recordings are loudest at
    // 125 to 500 Hz, with up to a tenth of the energy above 2.5 kHz.
    let storm = |buffet: f32| {
        let mut m = generators::create("wind", SR).unwrap();
        let k = m.desc().preset_index("Blizzard").unwrap();
        m.load_preset(k);
        set(m.as_mut(), "hiss/level", 0.0);
        set(m.as_mut(), "whistle/level", 0.25);
        set(m.as_mut(), "buffet/level", buffet);
        m.set_input_by_name("strength", 0.8);
        m.set_input_by_name("gustiness", 0.7);
        m.snap();
        render(m.as_mut(), 6.0)
    };
    let x = storm(0.7);
    let w = &x[..(SR * 1.0) as usize];
    let total = spectral_energy(w, 50.0, 9000.0, 25.0);
    let low = spectral_energy(w, 100.0, 600.0, 25.0) / total;
    let high = spectral_energy(w, 2500.0, 9000.0, 25.0) / total;
    assert!(low > 0.4 && high < 0.08, "low share {low:.2}, above 2.5 kHz {high:.3}");
    // Buffeting shoves the level about: the 50 ms level swings more with it than without.
    let swing = |x: &[f32]| {
        let env: Vec<f32> = x.chunks((0.05 * SR) as usize).map(rms).collect();
        let mean = env.iter().sum::<f32>() / env.len() as f32;
        (env.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / env.len() as f32).sqrt() / mean
    };
    assert!(swing(&x) > swing(&storm(0.0)) * 1.3, "buffeting {:.3} against {:.3}", swing(&x), swing(&storm(0.0)));
}
