//! The machinery family: trains, conveyor, press, clock, gears, music box, unlock, pneumatic,
//! hydraulic, elevator. Each test checks a defining trait measured on recordings
//! (`target/refs/machines`): the rhythm a mechanism's geometry makes, and where its sound sits.
use brusverk_core::generators::{self, machines};
use brusverk_core::render::{peak, rms};
use brusverk_core::Model;

const SR: f32 = 48000.0;
const MACHINES: [&str; 10] = ["train", "conveyor", "press", "clock", "gears", "music_box", "unlock", "pneumatic", "hydraulic", "elevator"];

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

fn make(name: &str, preset: Option<&str>, params: &[(&str, f32)]) -> Box<dyn Model> {
    let mut m = generators::create(name, SR).unwrap();
    if let Some(p) = preset {
        let i = m.desc().preset_index(p).unwrap_or_else(|| panic!("{name} has no preset {p}"));
        m.load_preset(i);
    }
    for (k, v) in params {
        assert!(m.set_param_by_name(k, *v), "{name} has no param {k}");
    }
    m
}

fn inputs(m: &mut dyn Model, inputs: &[(&str, f32)]) {
    for (k, v) in inputs {
        assert!(m.set_input_by_name(k, *v), "no input {k}");
    }
}

/// Onset times (s): where the 1 ms peak envelope of the high-passed signal rises through
/// `rel` of its maximum, at least `gap` seconds after the previous onset.
fn onsets(x: &[f32], hp_hz: f32, rel: f32, gap: f32) -> Vec<f32> {
    let coef = 1.0 - (-core::f32::consts::TAU * hp_hz / SR).exp();
    let mut lp = 0.0;
    let hop = (SR / 1000.0) as usize;
    let env: Vec<f32> = x
        .chunks(hop)
        .map(|c| {
            c.iter()
                .map(|s| {
                    lp += (s - lp) * coef;
                    (s - lp).abs()
                })
                .fold(0.0, f32::max)
        })
        .collect();
    let thr = rel * env.iter().fold(0.0f32, |a, b| a.max(*b));
    let mut out: Vec<f32> = Vec::new();
    let mut below = true;
    for (i, e) in env.iter().enumerate() {
        let t = i as f32 / 1000.0;
        if *e >= thr && below && out.last().is_none_or(|l| t - l >= gap) {
            out.push(t);
            below = false;
        } else if *e < thr * 0.5 {
            below = true;
        }
    }
    out
}

fn iois(t: &[f32]) -> Vec<f32> {
    t.windows(2).map(|w| w[1] - w[0]).collect()
}

/// Goertzel power of `x` at `hz`.
fn power_at(x: &[f32], hz: f32) -> f32 {
    let w = core::f32::consts::TAU * hz / SR;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        // Hann window, so neighbouring lines do not leak into each other.
        let win = 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / x.len() as f32).cos();
        let s = (*v * win) as f64 + c as f64 * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - c as f64 * s1 * s2) as f32 / (x.len() as f32).powi(2)
}

/// The frequency with most power between `lo` and `hi`, in `step` Hz steps.
fn peak_hz(x: &[f32], lo: f32, hi: f32, step: f32) -> f32 {
    let mut best = (lo, 0.0);
    let mut f = lo;
    while f <= hi {
        let p = power_at(x, f);
        if p > best.1 {
            best = (f, p);
        }
        f += step;
    }
    best.0
}

/// Power-weighted mean frequency (Hz), from the slope's level against the signal's: a sine of
/// frequency f has `rms(diff) / rms = 2 sin(pi f / sr)`.
fn centroid(x: &[f32]) -> f32 {
    let d: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let r = (rms(&d) / rms(x).max(1e-12) / 2.0).min(1.0);
    r.asin() * SR / core::f32::consts::PI
}

fn zero_crossing_rate(x: &[f32]) -> f32 {
    x.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f32 / x.len() as f32
}

#[test]
fn the_family_is_registered_as_machines() {
    for name in MACHINES {
        let d = generators::describe_all().into_iter().find(|d| d.name == name).unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(d.category, "machines", "{name}");
        assert!(d.presets.len() >= 4, "{name} needs presets");
    }
}

// ---- train ----

/// A train with only its clack (no roar, drive, squeal or thump), one car, onboard.
fn clack_only(preset: Option<&str>, extra: &[(&str, f32)]) -> Box<dyn Model> {
    let mut params = vec![("rolling/level", 0.0), ("drive/level", 0.0), ("squeal/level", 0.0), ("brake/level", 0.0), ("brake/air", 0.0), ("clack/thump", 0.0), ("clack/ring", 0.0)];
    params.extend_from_slice(extra);
    make("train", preset, &params)
}

#[test]
fn clack_geometry_helpers() {
    // Two bogies 3.5 m in from each end of a 24 m car, axles 2.5 m apart.
    assert_eq!(machines::car_axles(24.0, 3.5, 2.5), [2.25, 4.75, 19.25, 21.75]);
    let t = machines::clack_times(2, 24.0, 3.5, 2.5, 20.0);
    assert_eq!(t.len(), 8);
    assert!((t[1] - 0.125).abs() < 1e-5 && (t[4] - 1.2).abs() < 1e-5, "{t:?}");
}

#[test]
fn onboard_clack_is_ta_dum_and_its_tempo_follows_speed() {
    // One 24 m car on 25 m rails at 20 m/s: each joint is crossed by the leading bogie's two
    // axles 125 ms apart (ta-dum), 725 ms later by the trailing bogie's (ta-dum), and the next
    // joint comes 275 ms after that: a 1.25 s cycle.
    let geometry = [("train/cars", 1.0), ("train/car_m", 24.0), ("train/bogie_m", 3.5), ("train/wheelbase_m", 2.5), ("track/joint_m", 25.0), ("train/max_kmh", 100.0)];
    let run = |mps: f32| {
        let mut m = clack_only(None, &geometry);
        inputs(m.as_mut(), &[("speed", mps * 3.6 / 100.0), ("throttle", 0.0)]);
        m.snap();
        let x = render(m.as_mut(), 8.0);
        onsets(&x, 1000.0, 0.15, 0.03)
    };
    let fast = run(20.0);
    let gaps = iois(&fast);
    assert!(gaps.len() >= 20, "too few clacks: {fast:?}");
    for g in &gaps {
        let near = [0.125, 0.725, 0.275].iter().map(|e| (g - e).abs()).fold(f32::MAX, f32::min);
        assert!(near < 0.006, "clack gap {g:.3} s is not part of the bogie pattern: {gaps:?}");
    }
    let count = |e: f32| gaps.iter().filter(|g| (*g - e).abs() < 0.006).count();
    // Two short gaps (one per bogie) for each long gap.
    assert!(count(0.125) >= 2 * count(0.725) - 1 && count(0.725) >= 4 && count(0.275) >= 4, "{gaps:?}");
    // Half the speed, twice the time between clacks.
    let slow = run(10.0);
    let cycle = |t: &[f32]| (t[t.len() - 1] - t[0]) / (t.len() - 1) as f32;
    let ratio = cycle(&slow) / cycle(&fast);
    assert!((ratio - 2.0).abs() < 0.12, "tempo does not follow speed: ratio {ratio:.3}");
    assert!(iois(&slow).iter().any(|g| (g - 0.25).abs() < 0.008), "the ta-dum stretches with speed: {:?}", iois(&slow));
}

#[test]
fn more_cars_add_clacks_and_your_own_car_is_loudest() {
    let geometry = [("train/car_m", 24.0), ("track/joint_m", 25.0), ("train/max_kmh", 100.0), ("listen/position", 0.0)];
    let count = |cars: f32| {
        let mut extra = geometry.to_vec();
        extra.push(("train/cars", cars));
        let mut m = clack_only(None, &extra);
        inputs(m.as_mut(), &[("speed", 0.72)]);
        m.snap();
        let x = render(m.as_mut(), 6.0);
        // Faint, far clacks count too.
        (onsets(&x, 1000.0, 0.04, 0.015).len(), x)
    };
    let (one, x1) = count(1.0);
    let (three, x3) = count(3.0);
    assert!(three as f32 > one as f32 * 1.4, "3 cars ({three} clacks) should clack more than 1 ({one})");
    // The other cars are further away: they add less than your own does.
    assert!(rms(&x3) < rms(&x1) * 1.6, "far cars are as loud as your own: {} vs {}", rms(&x3), rms(&x1));
}

#[test]
fn trackside_hears_every_axle_of_every_car_cross_its_joint() {
    // Freight cars 17 m long, bogies 2 m in, 1.8 m wheelbase, at 42 % of 100 km/h: the joint by
    // the listener is crossed by 1.8 m pairs (154 ms apart), the pairs either side of each
    // coupling 2.2 m apart, and a new car every 17 m (1.46 s).
    let mut m = clack_only(Some("Freight trackside"), &[]);
    inputs(m.as_mut(), &[("speed", 0.42)]);
    m.snap();
    let v = 0.42 * 100.0 / 3.6;
    let x = render(m.as_mut(), 9.0);
    let t = onsets(&x, 1000.0, 0.45, 0.06);
    let gaps = iois(&t);
    let expected = [1.8 / v, 2.2 / v, 11.2 / v];
    for g in &gaps {
        let near = expected.iter().map(|e| (g - e).abs()).fold(f32::MAX, f32::min);
        assert!(near < 0.008, "gap {g:.3} s not in the freight pattern {expected:?}: {gaps:?}");
    }
    let car = t.windows(5).map(|w| w[4] - w[0]).sum::<f32>() / (t.len() - 4) as f32;
    assert!((car - 17.0 / v).abs() < 0.03, "a car passes every {car:.3} s, expected {:.3}", 17.0 / v);
}

#[test]
fn horn_chords_are_the_measured_ones() {
    for (kind, chord) in [(0.0, &[322.0, 390.0, 431.0][..]), (1.0, &[366.0, 460.0][..])] {
        let mut m = make("train", None, &[("horn/kind", kind), ("rolling/level", 0.0), ("drive/level", 0.0), ("clack/level", 0.0)]);
        inputs(m.as_mut(), &[("horn", 1.0)]);
        render(m.as_mut(), 0.5);
        let x = render(m.as_mut(), 1.0);
        for hz in chord {
            let p = power_at(&x, *hz);
            let off = power_at(&x, hz * 1.06);
            assert!(p > off * 20.0, "horn kind {kind}: no chime at {hz} Hz");
        }
    }
    // Silent without the horn.
    let mut m = make("train", None, &[("rolling/level", 0.0), ("drive/level", 0.0), ("clack/level", 0.0)]);
    assert!(peak(&render(m.as_mut(), 0.5)) < 1e-3);
}

#[test]
fn curves_squeal_high_and_brakes_squeal_near_a_stop() {
    let quiet = [("rolling/level", 0.0), ("drive/level", 0.0), ("clack/level", 0.0), ("brake/air", 0.0)];
    let mut m = make("train", None, &quiet);
    inputs(m.as_mut(), &[("speed", 0.25), ("curve", 1.0)]);
    m.snap();
    let x = render(m.as_mut(), 4.0);
    assert!(rms(&x) > 0.01, "no curve squeal");
    let f = peak_hz(&x, 1000.0, 8000.0, 10.0);
    assert!((f - 3900.0).abs() < 60.0, "squeal at {f} Hz, measured 3.8-3.9 kHz");
    let straight = {
        let mut m = make("train", None, &quiet);
        inputs(m.as_mut(), &[("speed", 0.25)]);
        m.snap();
        rms(&render(m.as_mut(), 2.0))
    };
    assert!(straight < rms(&x) * 0.1, "squeals on the straight: {straight}");
    // Braking: a grind at speed, then the squeal as it comes to a stop.
    let brake_at = |speed: f32| {
        let mut m = make("train", None, &quiet);
        inputs(m.as_mut(), &[("speed", speed), ("braking", 1.0)]);
        m.snap();
        let x = render(m.as_mut(), 2.0);
        power_at(&x, 3900.0 * 0.55)
    };
    assert!(brake_at(0.03) > brake_at(0.6) * 10.0, "no squeal as the train stops");
}

#[test]
fn diesel_revs_follow_the_throttle() {
    // 12 cylinders, two-stroke, 900 rpm max: 63 Hz firing at idle (35 %), 180 Hz flat out.
    let firing = |throttle: f32| {
        let mut m = make("train", None, &[("rolling/level", 0.0), ("clack/level", 0.0)]);
        inputs(m.as_mut(), &[("throttle", throttle)]);
        m.snap();
        render(m.as_mut(), 0.5);
        peak_hz(&render(m.as_mut(), 1.0), 40.0, 260.0, 1.0)
    };
    let (idle, full) = (firing(0.0), firing(1.0));
    assert!((idle - 63.0).abs() < 3.0 && (full - 180.0).abs() < 4.0, "firing {idle} / {full} Hz");
}

#[test]
fn electric_inverter_sings_only_under_power() {
    let tone = |throttle: f32| {
        let mut m = make("train", Some("Electric commuter"), &[("rolling/level", 0.0), ("clack/level", 0.0)]);
        inputs(m.as_mut(), &[("speed", 0.05), ("throttle", throttle)]);
        m.snap();
        render(m.as_mut(), 0.3);
        power_at(&render(m.as_mut(), 1.0), 620.0)
    };
    assert!(tone(1.0) > tone(0.0) * 100.0, "the inverter's 620 Hz carrier should only sound while it drives");
}

// ---- clock ----

#[test]
fn clocks_tick_steadily_at_their_rate_with_tick_and_tock() {
    for (preset, beats, error) in [("Default", 1.0, 0.03), ("Alarm clock", 5.0, 0.06), ("Mantel clock", 2.6, 0.04)] {
        let mut m = make("clock", Some(preset), &[]);
        let x = render(m.as_mut(), 12.0);
        let t = onsets(&x, 1500.0, 0.3, 0.6 / beats);
        let gaps = iois(&t);
        assert!(gaps.len() as f32 >= 10.0 * beats, "{preset}: {} beats in 12 s", gaps.len());
        let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
        assert!((mean * beats - 1.0).abs() < 0.01, "{preset}: {:.4} s a beat, expected {:.4}", mean, 1.0 / beats);
        // Tick and tock alternate long and short by the beat error, and every pair is the same.
        for pair in gaps.chunks(2).filter(|c| c.len() == 2) {
            assert!(((pair[0] + pair[1]) * beats / 2.0 - 1.0).abs() < 0.01, "{preset}: uneven pair {pair:?}");
            let asym = (pair[0] - pair[1]).abs() * beats / 2.0;
            assert!((asym - error).abs() < 0.008, "{preset}: tick-tock asymmetry {asym:.3}, expected {error}");
        }
    }
}

#[test]
fn clock_speed_and_spring() {
    let beats = |speed: f32, tension: f32| {
        let mut m = make("clock", None, &[]);
        inputs(m.as_mut(), &[("speed", speed), ("tension", tension)]);
        m.snap();
        let x = render(m.as_mut(), 8.0);
        if peak(&x) < 1e-3 {
            return 0.0;
        }
        let t = onsets(&x, 1500.0, 0.3, 0.2);
        // Beats per second, over whole tick-tock pairs.
        let pairs = (t.len() - 1) / 2 * 2;
        pairs as f32 / (t[pairs] - t[0])
    };
    let (normal, double) = (beats(0.5, 1.0), beats(1.0, 1.0));
    assert!((double / normal - 2.0).abs() < 0.02, "speed 1 doubles the tempo: {normal} vs {double} beats/s");
    assert!((normal - 1.0).abs() < 0.01, "the wall clock beats once a second: {normal}");
    assert_eq!(beats(0.5, 0.0), 0.0, "a run-down clock stops");
}

// ---- gears, ratchet, wind-up toy ----

#[test]
fn slow_gears_click_tooth_by_tooth() {
    // 12 teeth at 0.3 rev/s: 3.6 tooth contacts a second.
    let mut m = make("gears", None, &[("gears/stages", 1.0), ("whir/level", 0.0), ("gears/wear", 0.0)]);
    inputs(m.as_mut(), &[("speed", 0.2)]);
    m.snap();
    let x = render(m.as_mut(), 6.0);
    let t = onsets(&x, 1000.0, 0.3, 0.05);
    let mean = (t[t.len() - 1] - t[0]) / (t.len() - 1) as f32;
    assert!((mean * 3.6 - 1.0).abs() < 0.02, "tooth rate {:.3}/s, expected 3.6", 1.0 / mean);
}

#[test]
fn wind_up_toy_runs_in_a_lopsided_three_click_pattern() {
    // Measured: gaps of 30, 30 and 60 ms. Three teeth bunched to one side at 8.3 rev/s give
    // 27, 27 and 67 ms.
    let mut m = make("gears", Some("Wind-up toy"), &[("whir/level", 0.0), ("gears/wear", 0.0)]);
    inputs(m.as_mut(), &[("speed", 1.0)]);
    m.snap();
    let x = render(m.as_mut(), 2.0);
    let gaps = iois(&onsets(&x, 1000.0, 0.3, 0.015));
    let rev = 1.0 / 8.3;
    for g in &gaps {
        let near = [2.0 / 9.0 * rev, 5.0 / 9.0 * rev].iter().map(|e| (g - e).abs()).fold(f32::MAX, f32::min);
        assert!(near < 0.003, "gap {g:.4} not in the toy's pattern: {gaps:?}");
    }
}

#[test]
fn ratchet_pawl_clicks_bright_and_bounces() {
    let mut m = make("gears", Some("Ratchet winch"), &[("gears/level", 0.0)]);
    inputs(m.as_mut(), &[("speed", 0.6)]);
    m.snap();
    let x = render(m.as_mut(), 4.0);
    // 24 teeth at 0.3 rev/s: 7.2 clicks a second, each with a faint bounce 25 ms later.
    let t = onsets(&x, 2000.0, 0.3, 0.06);
    let mean = (t[t.len() - 1] - t[0]) / (t.len() - 1) as f32;
    assert!((mean * 7.2 - 1.0).abs() < 0.02, "ratchet rate {:.2}/s", 1.0 / mean);
    let faint = onsets(&x, 2000.0, 0.04, 0.012);
    assert!(faint.len() as f32 > t.len() as f32 * 1.7, "no bounces: {} faint vs {} clicks", faint.len(), t.len());
    assert!(centroid(&x) > 3500.0, "a pawl click is bright (measured centroid 6.7 kHz): {}", centroid(&x));
}

// ---- conveyor, press ----

#[test]
fn conveyor_hum_is_a_comb_of_shaft_harmonics_that_strains_with_load() {
    let make_c = |load: f32| {
        let mut m = make("conveyor", None, &[("rollers/level", 0.0), ("belt/splice", 0.0), ("items/level", 0.0), ("belt/rumble", 0.0), ("motor/gear_whine", 0.0)]);
        inputs(m.as_mut(), &[("speed", 1.0), ("load", load)]);
        m.snap();
        render(m.as_mut(), 0.5);
        render(m.as_mut(), 2.0)
    };
    let x = make_c(0.5);
    // 32 Hz shaft: lines at 32 k Hz, nothing between.
    for k in [6.0, 10.0, 12.0, 14.0] {
        let (on, off) = (power_at(&x, 32.0 * k), power_at(&x, 32.0 * (k + 0.5)));
        assert!(on > off * 8.0, "no hum line at {} Hz", 32.0 * k);
    }
    assert!(rms(&make_c(1.0)) > rms(&make_c(0.0)) * 1.5, "load should strain the motor");
}

#[test]
fn conveyor_splice_thumps_once_a_loop() {
    // A 6.5 m belt at 1.2 m/s comes round every 5.4 s, as the measured one did.
    let mut m = make("conveyor", None, &[("rollers/level", 0.0), ("items/level", 0.0), ("belt/rumble", 0.0), ("motor/level", 0.0), ("motor/gear_whine", 0.0)]);
    inputs(m.as_mut(), &[("speed", 1.0)]);
    m.snap();
    let x = render(m.as_mut(), 17.0);
    let t = onsets(&x, 300.0, 0.5, 1.0);
    let gaps = iois(&t);
    assert!(gaps.len() >= 2 && gaps.iter().all(|g| (g - 6.5 / 1.2).abs() < 0.02), "splice gaps {gaps:?}");
}

#[test]
fn press_strokes_at_its_rate() {
    for (rate, spm) in [(0.0001, 0.0), (0.5, 12.0 * 5f32.sqrt()), (1.0, 60.0)] {
        let mut m = make("press", None, &[("flywheel/level", 0.0), ("slide/level", 0.0), ("air/level", 0.0), ("clutch/level", 0.0)]);
        inputs(m.as_mut(), &[("rate", rate), ("force", 1.0)]);
        m.snap();
        let x = render(m.as_mut(), 12.0);
        let t = onsets(&x, 30.0, 0.4, 0.5);
        if spm == 0.0 {
            assert!(peak(&x) < 1e-3, "a stopped press does not stroke");
            continue;
        }
        let mean = (t[t.len() - 1] - t[0]) / (t.len() - 1) as f32;
        assert!((60.0 / mean - spm).abs() < spm * 0.02, "rate {rate}: {:.1} strokes/min, expected {spm:.1}", 60.0 / mean);
    }
}

#[test]
fn hydraulic_press_pump_labours_while_pressing() {
    let mut m = make("press", Some("Hydraulic press"), &[("impact/level", 0.0), ("slide/level", 0.0), ("air/level", 0.0)]);
    inputs(m.as_mut(), &[("rate", 1.0), ("force", 1.0)]);
    m.snap();
    let x = render(m.as_mut(), 12.0);
    let f = peak_hz(&x[..(SR * 2.0) as usize], 380.0, 480.0, 1.0);
    assert!((f - 442.0 * 0.985).abs() < 8.0, "pump at {f} Hz, measured 442 Hz");
}

// ---- music box ----

#[test]
fn music_box_plays_its_comb_and_winds_down() {
    let mut m = make("music_box", Some("Carillon"), &[]);
    let x = render(m.as_mut(), 6.0);
    // The carillon starts on the octave of F5 (698.5 Hz): 1397 Hz.
    let f = peak_hz(&x[..(SR * 0.5) as usize], 1300.0, 1500.0, 1.0);
    assert!((f - 1397.0).abs() < 4.0, "first note at {f} Hz");
    let notes = |wind: f32| {
        let mut m = make("music_box", Some("Carillon"), &[("comb/ring", 0.2), ("comb/overtone", 0.0)]);
        inputs(m.as_mut(), &[("wind", wind)]);
        m.snap();
        onsets(&render(m.as_mut(), 6.0), 2500.0, 0.2, 0.05).len() as f32
    };
    let (full, low) = (notes(1.0), notes(0.2));
    // Carillon: a note on every step at 6 a second; at wind 0.2 it drags to about 56 %.
    assert!((full / 36.0 - 1.0).abs() < 0.1, "{full} notes in 6 s at full wind");
    assert!((low / full - 0.56).abs() < 0.12, "running down: {low} vs {full} notes");
    let mut m = make("music_box", None, &[]);
    inputs(m.as_mut(), &[("wind", 0.0)]);
    m.snap();
    render(m.as_mut(), 4.0);
    assert!(peak(&render(m.as_mut(), 1.0)) < 1e-3, "a run-down music box is silent");
}

#[test]
fn music_box_lid_muffles_it() {
    let bright = |lid: f32| {
        let mut m = make("music_box", None, &[]);
        inputs(m.as_mut(), &[("lid", lid)]);
        m.snap();
        let x = render(m.as_mut(), 4.0);
        (centroid(&x), rms(&x))
    };
    let (open, shut) = (bright(1.0), bright(0.0));
    assert!(shut.0 < open.0 * 0.8 && shut.1 < open.1 * 0.7, "lid: open {open:?}, shut {shut:?}");
}

// ---- unlock, pneumatic ----

#[test]
fn unlock_clicks_each_tumbler_then_clunks() {
    for n in [2.0, 5.0] {
        let mut m = make("unlock", Some("Door lock"), &[("lock/tumblers", n), ("shape/variation", 0.0), ("bolt/slide", 0.0)]);
        m.trigger();
        let x = render(m.as_mut(), 2.0);
        let clunk_at = 0.01 + n * 0.12 + 0.05 + 0.18;
        let clicks = onsets(&x[..((clunk_at - 0.02) * SR) as usize], 2000.0, 0.2, 0.04);
        assert_eq!(clicks.len(), n as usize, "{n} tumblers: {clicks:?}");
        // The clunk is the heavy part: far more low end after it than before.
        let low = |a: f32, b: f32| {
            let s = &x[(a * SR) as usize..(b * SR) as usize];
            let mut lp = 0.0;
            rms(&s.iter().map(|v| {
                lp += (v - lp) * 0.02;
                lp
            }).collect::<Vec<_>>())
        };
        assert!(low(clunk_at, clunk_at + 0.2) > low(0.0, clunk_at - 0.05) * 4.0, "no clunk at {clunk_at:.2} s");
    }
}

#[test]
fn air_release_is_bright_and_falls_away() {
    let mut m = make("pneumatic", Some("Pressure release"), &[]);
    m.trigger();
    let x = render(m.as_mut(), 4.0);
    let early = &x[(0.1 * SR) as usize..(0.6 * SR) as usize];
    // Measured: centroid near 12 kHz, 98 % above 2.5 kHz.
    assert!(zero_crossing_rate(early) > 0.25, "air should hiss bright: zcr {}", zero_crossing_rate(early));
    let late = &x[(2.0 * SR) as usize..(2.5 * SR) as usize];
    assert!(rms(late) < rms(early) * 0.25, "the hiss should fall away: {} vs {}", rms(late), rms(early));
    // A piston: the clack lands at piston/ms.
    let mut m = make("pneumatic", None, &[("exhaust/level", 0.0), ("valve/click", 0.0), ("shape/variation", 0.0)]);
    m.trigger();
    let t = onsets(&render(m.as_mut(), 0.5), 300.0, 0.3, 0.02);
    assert!((t[0] - 0.045).abs() < 0.003, "piston clack at {t:?}");
}

// ---- hydraulic, elevator ----

#[test]
fn hydraulic_pump_sags_under_load_and_relief_whistles_against_a_stop() {
    let quiet = [("flow/level", 0.0), ("relief/level", 0.0), ("creak/level", 0.0), ("valve/level", 0.0)];
    let pump = |motion: f32, load: f32| {
        let mut m = make("hydraulic", None, &quiet);
        inputs(m.as_mut(), &[("motion", motion), ("load", load)]);
        m.snap();
        render(m.as_mut(), 0.3);
        let x = render(m.as_mut(), 1.0);
        (peak_hz(&x, 380.0, 480.0, 0.5), rms(&x))
    };
    let (idle, labour) = (pump(0.0, 0.0), pump(1.0, 1.0));
    assert!((idle.0 - 442.0).abs() < 1.5 && (labour.0 - 442.0 * 0.94).abs() < 2.0, "pump {} -> {} Hz", idle.0, labour.0);
    assert!(labour.1 > idle.1 * 2.0, "the pump labours louder under load");
    let relief = |motion: f32| {
        let mut m = make("hydraulic", None, &[("pump/level", 0.0), ("flow/level", 0.0), ("creak/level", 0.0), ("valve/level", 0.0)]);
        inputs(m.as_mut(), &[("motion", motion), ("load", 1.0)]);
        m.snap();
        render(m.as_mut(), 0.5);
        rms(&render(m.as_mut(), 1.0))
    };
    assert!(relief(0.0) > relief(0.8) * 10.0, "relief valve whistles only against a stop");
}

#[test]
fn elevator_passes_a_landing_every_floor_and_runs_its_door() {
    let floors = |speed: f32| {
        let only = [("car/rumble", 0.0), ("motor/level", 0.0), ("motor/inverter", 0.0), ("car/rail_knock", 0.0), ("brake/level", 0.0), ("chime/level", 0.0)];
        let mut m = make("elevator", None, &only);
        inputs(m.as_mut(), &[("speed", speed)]);
        m.snap();
        let x = render(m.as_mut(), 14.0);
        // Whooshes: peaks of the 20 ms envelope.
        let env: Vec<f32> = x.chunks((SR * 0.02) as usize).map(rms).collect();
        let top = env.iter().fold(0.0f32, |a, b| a.max(*b));
        let mut t = Vec::new();
        for i in 1..env.len() - 1 {
            if env[i] > 0.5 * top && env[i] >= env[i - 1] && env[i] > env[i + 1] && t.last().is_none_or(|l: &f32| i as f32 * 0.02 - l > 0.8) {
                t.push(i as f32 * 0.02);
            }
        }
        (t[t.len() - 1] - t[0]) / (t.len() - 1) as f32
    };
    // 3.2 m floors at 1.6 m/s: one every 2 s; at half speed every 4 s.
    let (fast, slow) = (floors(1.0), floors(0.5));
    assert!((fast - 2.0).abs() < 0.08 && (slow - 4.0).abs() < 0.16, "floors every {fast:.2} / {slow:.2} s");
    // The door motor runs while the door moves, not while it stands open.
    let mut m = make("elevator", None, &[]);
    let mut moving = Vec::new();
    for i in 0..60 {
        m.set_input_by_name("door", i as f32 / 60.0);
        moving.extend(render(m.as_mut(), 0.025));
    }
    m.set_input_by_name("door", 1.0);
    render(m.as_mut(), 0.5);
    let open = render(m.as_mut(), 1.0);
    assert!(rms(&moving) > rms(&open) * 5.0, "door motor: moving {} vs open {}", rms(&moving), rms(&open));
}

#[test]
fn elevator_chimes_on_arrival() {
    let mut m = make("elevator", None, &[("car/rumble", 0.0), ("motor/level", 0.0), ("motor/inverter", 0.0), ("car/rail_knock", 0.0), ("brake/level", 0.0), ("car/floor_whoosh", 0.0)]);
    m.set_input_by_name("speed", 1.0);
    render(m.as_mut(), 4.0);
    m.set_input_by_name("speed", 0.0);
    let x = render(m.as_mut(), 4.0);
    assert!(power_at(&x, 988.0) > power_at(&x, 900.0) * 30.0, "no 988 Hz chime on arrival");
}

// ---- everything ----

#[test]
fn every_machine_stays_bounded_under_wild_inputs() {
    let mut rng = brusverk_core::Rng::new(7);
    for name in MACHINES {
        let mut m = generators::create(name, SR).unwrap();
        for preset in 0..m.desc().presets.len() {
            m.load_preset(preset);
            let n_in = m.desc().inputs.len();
            let mut out = Vec::new();
            for _ in 0..40 {
                for i in 0..n_in {
                    let v = if rng.chance(0.3) { rng.next_f32().round() } else { rng.next_f32() };
                    m.set_input(i, v);
                }
                if m.desc().one_shot && rng.chance(0.4) {
                    m.trigger();
                }
                out.extend(render(m.as_mut(), 0.1));
            }
            let label = format!("{name}/{}", m.desc().presets[preset].name);
            assert!(out.iter().all(|x| x.is_finite()), "{label} produced NaN/inf");
            assert!(peak(&out) <= 1.0, "{label} peak {}", peak(&out));
        }
    }
}

#[test]
fn continuous_machines_sit_near_the_motor_and_electric_level() {
    // Typical use, measured as RMS dB: the family sits together, between a quiet clock and a
    // full-throttle train, all inside 20 dB.
    let level = |name: &str, preset: Option<&str>, ins: &[(&str, f32)]| {
        let mut m = make(name, preset, &[]);
        inputs(m.as_mut(), ins);
        m.snap();
        render(m.as_mut(), 1.0);
        20.0 * rms(&render(m.as_mut(), 6.0)).log10()
    };
    let electric = level("electric", None, &[]);
    for (name, preset, ins) in [
        ("train", None, &[("speed", 0.5), ("throttle", 0.5)][..]),
        ("train", Some("Freight trackside"), &[("speed", 0.42)][..]),
        ("conveyor", None, &[("speed", 0.7), ("load", 0.5)][..]),
        ("hydraulic", None, &[("motion", 0.6), ("load", 0.6)][..]),
        ("elevator", None, &[("speed", 1.0)][..]),
        ("music_box", None, &[][..]),
    ] {
        let l = level(name, preset, ins);
        assert!((l - electric).abs() < 7.0, "{name} {preset:?}: {l:.1} dB against electric's {electric:.1} dB");
    }
}
