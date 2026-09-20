use gamesynth_core::generators;
use gamesynth_core::render::{peak, rms};
use gamesynth_core::Model;

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
    assert!(generators::NAMES.len() >= 36);
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
    for name in ["hover", "combustion", "motor", "rotor", "scrape", "beam", "wind", "rain", "fire", "stream", "electric", "crowd", "radio"] {
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
