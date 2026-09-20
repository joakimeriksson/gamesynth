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
    assert!(generators::NAMES.len() >= 15);
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
    for name in ["hover", "combustion", "motor", "rotor", "wind", "rain", "fire", "stream", "electric", "crowd", "radio"] {
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
    assert!(elapsed < secs * 0.5, "all {} generators took {elapsed}s for {secs}s", all.len());
}
