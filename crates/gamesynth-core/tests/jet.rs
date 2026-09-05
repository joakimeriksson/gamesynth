use gamesynth_core::render::{peak, rms};
use gamesynth_core::*;

const SR: f32 = 48000.0;

fn render(engine: &mut JetEngine, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    engine.render_mono(&mut out);
    out
}

fn zero_crossings(s: &[f32]) -> usize {
    s.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
}

#[test]
fn all_presets_are_bounded_and_audible() {
    for preset in JetPreset::ALL {
        let mut e = JetEngine::new(SR, preset.params());
        e.set_throttle(1.0);
        e.set_boost(1.0);
        e.set_speed(1.0);
        e.set_damage(0.5);
        let out = render(&mut e, 4.0);
        assert!(out.iter().all(|x| x.is_finite()), "{preset:?} produced NaN");
        assert!(peak(&out) <= 1.0, "{preset:?} peak {}", peak(&out));
        assert!(rms(&out[SR as usize..]) > 0.05, "{preset:?} too quiet");
        // Idle is audible too.
        let mut e = JetEngine::new(SR, preset.params());
        let idle = render(&mut e, 2.0);
        assert!(rms(&idle[SR as usize..]) > 0.01, "{preset:?} idle silent");
    }
}

#[test]
fn rpm_spools_with_inertia() {
    let mut p = JetPreset::Racer.params();
    p.spool_up = 1.0;
    p.spool_down = 2.0;
    let mut e = JetEngine::new(SR, p);
    assert!((e.rpm() - p.idle_rpm).abs() < 1e-6);
    e.set_throttle(1.0);
    render(&mut e, 0.1);
    assert!(e.rpm() < 0.5, "rpm jumped: {}", e.rpm());
    render(&mut e, 0.9);
    assert!(e.rpm() > 0.93 && e.rpm() < 1.0, "rpm after 1s: {}", e.rpm());
    e.set_throttle(0.0);
    render(&mut e, 0.5);
    assert!(e.rpm() > 0.5, "spool down too fast: {}", e.rpm());
    render(&mut e, 2.0);
    assert!(e.rpm() < p.idle_rpm + 0.05, "did not return to idle: {}", e.rpm());
    e.set_throttle(1.0);
    e.snap_rpm();
    assert!((e.rpm() - 1.0).abs() < 1e-6);
}

#[test]
fn throttle_raises_pitch_and_level() {
    let mut e = JetEngine::new(SR, JetPreset::Racer.params());
    e.set_throttle(0.0);
    e.snap_rpm();
    let idle = render(&mut e, 2.0);
    e.set_throttle(1.0);
    e.snap_rpm();
    let full = render(&mut e, 2.0);
    let (idle, full) = (&idle[SR as usize..], &full[SR as usize..]);
    assert!(rms(full) > rms(idle) * 1.3, "full {} idle {}", rms(full), rms(idle));
    assert!(zero_crossings(full) > zero_crossings(idle) * 3 / 2, "full {} idle {}", zero_crossings(full), zero_crossings(idle));
}

#[test]
fn boost_and_wind_add_energy() {
    let mut e = JetEngine::new(SR, JetPreset::Racer.params());
    e.set_throttle(1.0);
    e.snap_rpm();
    let base = rms(&render(&mut e, 2.0)[SR as usize..]);
    e.set_boost(1.0);
    e.snap_rpm();
    let boosted = rms(&render(&mut e, 2.0)[SR as usize..]);
    assert!(boosted > base * 1.1, "boost {boosted} base {base}");

    let mut e = JetEngine::new(SR, JetPreset::Racer.params());
    e.set_speed(1.0);
    e.snap_rpm();
    let windy = rms(&render(&mut e, 2.0)[SR as usize..]);
    let mut e = JetEngine::new(SR, JetPreset::Racer.params());
    let still = rms(&render(&mut e, 2.0)[SR as usize..]);
    assert!(windy > still * 1.1, "windy {windy} still {still}");
}

#[test]
fn damage_sputters() {
    let mut e = JetEngine::new(SR, JetPreset::Racer.params());
    e.set_throttle(0.8);
    e.snap_rpm();
    e.set_damage(1.0);
    let out = render(&mut e, 4.0);
    // Envelope in 10 ms windows: sputter must create windows far quieter than the median.
    let win = (SR * 0.01) as usize;
    let mut env: Vec<f32> = out.chunks(win).map(rms).collect();
    env.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = env[env.len() / 2];
    let quiet = env.iter().filter(|&&v| v < median * 0.5).count();
    assert!(quiet > 5, "no drop-outs (quiet windows {quiet}, median {median})");
}

#[test]
fn params_round_trip_and_json() {
    let mut p = JetParams::default();
    for &id in JetParamId::ALL {
        assert_eq!(JetParamId::from_name(id.name()), Some(id));
        let v = p.get(id);
        p.set(id, v);
        assert_eq!(p.get(id), v);
        assert_eq!(<JetParams as Params>::param_name(id), id.name());
    }
    p.set(JetParamId::WhineHz, 1e9);
    assert_eq!(p.whine_hz, 8000.0);
    let json = serde_json::to_string(&p).unwrap();
    let back: JetParams = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
    let partial: JetParams = serde_json::from_str(r#"{"whine_hz": 1000.0}"#).unwrap();
    assert_eq!(partial.whine_hz, 1000.0);
    assert_eq!(partial.roar_hz, JetParams::default().roar_hz);
}

#[test]
fn renders_many_engines_in_real_time() {
    let mut engines: Vec<JetEngine> = (0..8).map(|i| JetEngine::new(SR, JetPreset::from_index(i % 4).params())).collect();
    for (i, e) in engines.iter_mut().enumerate() {
        e.set_throttle(0.5 + 0.05 * i as f32);
        e.set_boost(if i % 2 == 0 { 1.0 } else { 0.0 });
        e.set_speed(0.7);
    }
    let secs = 2.0;
    let mut buf = vec![0.0; 512];
    let start = std::time::Instant::now();
    for _ in 0..((secs * SR) as usize / 512) {
        for e in engines.iter_mut() {
            e.render_mono(&mut buf);
        }
    }
    let elapsed = start.elapsed().as_secs_f32();
    assert!(elapsed < secs * 0.5, "8 engines took {elapsed}s for {secs}s");
}
