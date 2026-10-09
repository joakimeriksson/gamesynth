//! Footsteps: what each surface must keep, measured the way the references were.
//!
//! The thresholds come from CC0 recordings (Freesound) of single walkers, measured per step
//! (onset-aligned, 200 ms spectra): every dry surface is a mid-to-low sound (centroid 300 Hz to
//! 1.1 kHz, under 6 % above 2.5 kHz); stone falls 20 dB in about 13 ms; gravel and snow keep
//! crunching after the heel (snow densely, gravel in distinct grains); metal rings (12 dB of
//! tonal peak in its tail, against 7-8 for the rest); a lawn is mostly below 250 Hz; shallow
//! water and mud are the bright ones.
use brusverk_core::filter::{FilterMode, Svf};
use brusverk_core::generators::{self, foley::SURFACES};
use brusverk_core::render::peak;
use brusverk_core::Model;

const SR: f32 = 48000.0;

fn walker(surface: &str) -> Box<dyn Model> {
    let mut m = generators::create("footstep", SR).unwrap();
    m.set_input_by_name("surface", SURFACES.iter().position(|s| *s == surface).unwrap() as f32 / 10.0);
    m
}

/// Trigger `n` steps `interval` seconds apart with the walker's current inputs; each step's
/// samples from its trigger on.
fn steps(m: &mut dyn Model, n: usize, interval: f32) -> Vec<Vec<f32>> {
    (0..n)
        .map(|_| {
            m.trigger();
            let mut x = vec![0.0; (interval * SR) as usize];
            m.render_mono(&mut x);
            x
        })
        .collect()
}

fn walk(surface: &str, speed: f32, n: usize) -> Vec<Vec<f32>> {
    let mut m = walker(surface);
    m.set_input_by_name("speed", speed);
    steps(m.as_mut(), n, 0.55)
}

fn db(x: f32) -> f32 {
    10.0 * x.max(1e-20).log10()
}

/// A power spectrum: `p[k]` is the power at `k * df` Hz.
struct Spec {
    df: f32,
    p: Vec<f32>,
}

/// In-place radix-2 FFT of a power-of-two length.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
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
    let mut size = 2;
    while size <= n {
        let ang = -core::f32::consts::TAU / size as f32;
        for start in (0..n).step_by(size) {
            for k in 0..size / 2 {
                let (wr, wi) = ((ang * k as f32).cos(), (ang * k as f32).sin());
                let (a, b) = (start + k, start + k + size / 2);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        size *= 2;
    }
}

/// Welch power spectrum (Hann segments of `n` samples, half overlapping), as
/// `scipy.signal.welch` measured the recordings.
fn welch(x: &[f32], n: usize) -> Spec {
    let mut p = vec![0.0f32; n / 2];
    let mut start = 0;
    loop {
        let (mut re, mut im) = (vec![0.0f32; n], vec![0.0f32; n]);
        for (i, r) in re.iter_mut().enumerate() {
            let v = x.get(start + i).copied().unwrap_or(0.0);
            *r = v * (0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n as f32).cos());
        }
        fft(&mut re, &mut im);
        for (k, a) in p.iter_mut().enumerate() {
            *a += re[k] * re[k] + im[k] * im[k];
        }
        start += n / 2;
        if start + n > x.len() {
            break;
        }
    }
    Spec { df: SR / n as f32, p }
}

/// Average Welch spectrum of the steps between `t0` and `t1` seconds after each trigger.
fn spectrum(steps: &[Vec<f32>], t0: f32, t1: f32, n: usize) -> Spec {
    let mut acc = Spec { df: SR / n as f32, p: vec![0.0f32; n / 2] };
    for s in steps {
        for (a, p) in acc.p.iter_mut().zip(welch(&s[(t0 * SR) as usize..(t1 * SR) as usize], n).p) {
            *a += p;
        }
    }
    acc
}

/// The spectrum of a step: its first 200 ms in 2048-sample segments.
fn step_spectrum(steps: &[Vec<f32>]) -> Spec {
    spectrum(steps, 0.0, 0.2, 2048)
}

/// How much the steps ring: the median over steps of the tonality of each step's tail
/// (40-300 ms in 4096-sample segments).
fn ring_db(steps: &[Vec<f32>]) -> f32 {
    median(steps.iter().map(|s| tonality(&welch(&s[(0.04 * SR) as usize..(0.3 * SR) as usize], 4096))).collect())
}

fn share(s: &Spec, lo: f32, hi: f32) -> f32 {
    let hz = |k: usize| k as f32 * s.df;
    let total: f32 = s.p.iter().enumerate().filter(|(k, _)| hz(*k) >= 30.0).map(|(_, v)| v).sum();
    s.p.iter().enumerate().filter(|(k, _)| hz(*k) >= lo.max(30.0) && hz(*k) < hi).map(|(_, v)| v).sum::<f32>() / total
}

fn centroid(s: &Spec) -> f32 {
    let (mut num, mut den) = (0.0, 0.0);
    for (k, v) in s.p.iter().enumerate() {
        let f = k as f32 * s.df;
        if f >= 30.0 {
            num += f * v;
            den += v;
        }
    }
    num / den
}

/// How far the strongest spectral line between 150 Hz and 6 kHz stands above its third-octave
/// neighbourhood (dB): about 7-8 for noisy steps, 12 for a ringing plate.
fn tonality(s: &Spec) -> f32 {
    let hz = |k: usize| k as f32 * s.df;
    let d: Vec<f32> = s.p.iter().map(|v| db(*v)).collect();
    let top = (0..d.len()).filter(|&k| hz(k) > 100.0 && hz(k) < 8000.0).map(|k| d[k]).fold(f32::MIN, f32::max);
    let mut best = 0.0f32;
    for k in (0..d.len()).filter(|&k| hz(k) > 150.0 && hz(k) < 6000.0 && d[k] > top - 25.0) {
        let mut near: Vec<f32> = (0..d.len()).filter(|&i| hz(i) > hz(k) / 1.26 && hz(i) < hz(k) * 1.26).map(|i| d[i]).collect();
        near.sort_by(f32::total_cmp);
        best = best.max(d[k] - near[near.len() / 2]);
    }
    best
}

/// RMS envelope in `ms` frames.
fn envelope(x: &[f32], ms: f32) -> Vec<f32> {
    x.chunks((ms * SR / 1000.0) as usize).map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt()).collect()
}

/// Milliseconds from the loudest 1 ms frame until the envelope is 20 dB below it.
fn fall_20db_ms(x: &[f32]) -> f32 {
    let e = envelope(x, 1.0);
    let k = (0..e.len()).max_by(|&a, &b| e[a].total_cmp(&e[b])).unwrap();
    (k..e.len()).find(|&i| e[i] < e[k] * 0.1).unwrap_or(e.len()) as f32 - k as f32
}

/// Energy 60-250 ms after the heel against the first 60 ms (dB): the crunch or roll-off
/// that follows the impact.
fn tail_db(x: &[f32]) -> f32 {
    let e = |a: f32, b: f32| x[(a * SR) as usize..(b * SR) as usize].iter().map(|v| v * v).sum::<f32>();
    db(e(0.06, 0.25) / e(0.0, 0.06))
}

/// Crest (p95 / p50, dB) of the 0.5 ms envelope of the 1.5-6 kHz band, 20-220 ms after the
/// heel: distinct grains (gravel) score high, a dense even crunch (snow) low.
fn grain_crest(x: &[f32]) -> f32 {
    let (mut hp, mut lp) = (Svf::default(), Svf::default());
    hp.set(FilterMode::HighPass, 1500.0, 0.2, SR);
    lp.set(FilterMode::LowPass, 6000.0, 0.2, SR);
    let band: Vec<f32> = x[..(0.24 * SR) as usize].iter().map(|v| lp.tick(hp.tick(*v))).collect();
    let mut e = envelope(&band[(0.02 * SR) as usize..], 0.5);
    e.sort_by(f32::total_cmp);
    20.0 * (e[e.len() * 95 / 100] / e[e.len() / 2].max(1e-12)).log10()
}

/// When the toe lands (ms after the heel): the strongest rise in the 2 ms envelope after the
/// heel has died down.
fn toe_ms(x: &[f32]) -> f32 {
    let e = envelope(&x[..(0.2 * SR) as usize], 2.0);
    (12..e.len()).max_by(|&a, &b| (e[a] / e[a - 3]).total_cmp(&(e[b] / e[b - 3]))).unwrap() as f32 * 2.0
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

fn peak_db(x: &[f32]) -> f32 {
    20.0 * peak(x).max(1e-9).log10()
}

#[test]
fn silent_until_a_foot_lands_then_rings_out() {
    let mut m = walker("gravel");
    let mut x = vec![0.0; (0.5 * SR) as usize];
    m.render_mono(&mut x);
    assert!(x.iter().all(|v| *v == 0.0), "no step, no sound");
    let s = steps(m.as_mut(), 1, 1.5).remove(0);
    assert!(peak(&s[..(0.1 * SR) as usize]) > 0.05, "a trigger is a step");
    assert!(peak(&s[(1.0 * SR) as usize..]) < 1e-4, "a step rings out within a second");
    assert!(!m.is_finished(), "one walker is one long-lived player: it never finishes");
}

#[test]
fn dry_surfaces_are_mid_range_and_wet_ones_bright() {
    for surface in SURFACES {
        let p = step_spectrum(&walk(surface, 0.3, 10));
        let (c, top) = (centroid(&p), share(&p, 2500.0, 24000.0));
        if surface == "water" || surface == "mud" {
            // Splash and squelch: the recordings have 50-77 % above 2.5 kHz.
            assert!(top > 0.3, "{surface}: only {:.0} % above 2.5 kHz", top * 100.0);
        } else {
            // Recordings: centroid 300 Hz-1.1 kHz, 0.2-6 % above 2.5 kHz.
            assert!(c < 1300.0, "{surface}: centroid {c:.0} Hz is too bright");
            assert!(top < 0.1, "{surface}: {:.1} % above 2.5 kHz", top * 100.0);
        }
    }
}

#[test]
fn stone_is_sharp_and_grass_is_a_dull_thump() {
    let stone = walk("stone", 0.3, 10);
    let fall = median(stone.iter().map(|s| fall_20db_ms(&s[..(0.08 * SR) as usize])).collect());
    // Recordings: 13 ms (hiking boots), 24-31 ms (shoes on concrete and a patio).
    assert!(fall < 20.0, "stone takes {fall} ms to fall 20 dB");
    let tail = median(stone.iter().map(|s| tail_db(s)).collect());
    assert!(tail < -6.0, "stone should not ring on after the heel and toe: tail {tail:.1} dB");

    let grass = walk("grass", 0.3, 10);
    let p = step_spectrum(&grass);
    // Recording: 85 % below 250 Hz, centroid 325 Hz, a short step.
    assert!(share(&p, 0.0, 250.0) > 0.6, "grass: only {:.0} % below 250 Hz", share(&p, 0.0, 250.0) * 100.0);
    assert!(centroid(&p) < 500.0, "grass centroid {:.0} Hz", centroid(&p));
    let tail = median(grass.iter().map(|s| tail_db(s)).collect());
    assert!(tail < -8.0, "grass is a short thump, tail {tail:.1} dB");
}

#[test]
fn gravel_and_snow_keep_crunching_snow_dense_and_soft() {
    let gravel = walk("gravel", 0.3, 12);
    let snow = walk("snow", 0.3, 12);
    let tail = |s: &[Vec<f32>]| median(s.iter().map(|x| tail_db(x)).collect());
    // Recordings: gravel -5 to -1 dB, snow +3 to +7 dB, stone and grass -16 to -19 dB.
    assert!(tail(&gravel) > -6.0, "gravel stops crunching: tail {:.1} dB", tail(&gravel));
    assert!(tail(&snow) > -6.0, "snow stops crunching: tail {:.1} dB", tail(&snow));
    let crest = |s: &[Vec<f32>]| median(s.iter().map(|x| grain_crest(x)).collect());
    // Recordings: gravel 15-21 dB (distinct stones), snow 11-13 dB (dense).
    assert!(crest(&snow) + 3.0 < crest(&gravel), "snow ({:.1} dB) should crunch more densely than gravel ({:.1} dB)", crest(&snow), crest(&gravel));
    // Snow's crunch is soft: it lives below 3 kHz.
    let p = spectrum(&snow, 0.03, 0.2, 2048);
    assert!(share(&p, 3000.0, 24000.0) < 0.06, "snow: {:.1} % above 3 kHz", share(&p, 3000.0, 24000.0) * 100.0);
    assert!(share(&p, 200.0, 3000.0) > 0.5, "snow's crunch should sit in 200 Hz-3 kHz");
}

#[test]
fn metal_rings_and_nothing_else_does() {
    let ring = |surface: &str| ring_db(&walk(surface, 0.3, 12));
    let metal = ring("metal");
    // Recordings: 12.4 dB for a metal floor (11.8 for grating and stairs), 7-9 for the rest.
    assert!(metal > 11.0, "metal does not ring: {metal:.1} dB");
    for surface in SURFACES.iter().filter(|s| **s != "metal") {
        let r = ring(surface);
        assert!(r < metal - 3.0, "{surface} rings like metal: {r:.1} vs {metal:.1} dB");
    }
    // And it rings on: metal takes far longer than stone to fall 20 dB.
    let fall = |surface: &str| median(walk(surface, 0.3, 8).iter().map(|s| fall_20db_ms(&s[..(0.4 * SR) as usize])).collect());
    assert!(fall("metal") > 2.5 * fall("stone"), "metal {} ms vs stone {} ms", fall("metal"), fall("stone"));
}

#[test]
fn wood_has_a_low_boom_under_the_knock() {
    // The boards ring at about 110 Hz long after the knock; stone's thud is over at once.
    let boom = |surface: &str| {
        median(
            walk(surface, 0.3, 12)
                .iter()
                .map(|x| {
                    let mut lp = Svf::default();
                    lp.set(FilterMode::LowPass, 200.0, 0.2, SR);
                    let low: Vec<f32> = x.iter().map(|v| lp.tick(*v)).collect();
                    let e = |a: f32, b: f32| low[(a * SR) as usize..(b * SR) as usize].iter().map(|v| v * v).sum::<f32>();
                    db(e(0.15, 0.35) / e(0.0, 0.06))
                })
                .collect(),
        )
    };
    let (wood, stone) = (boom("wood"), boom("stone"));
    assert!(wood > -32.0, "wood's boom dies too soon: {wood:.1} dB");
    assert!(wood > stone + 10.0, "wood ({wood:.1} dB) should boom on longer than stone ({stone:.1} dB)");
}

#[test]
fn steps_vary_and_the_heel_and_toe_land_apart() {
    let gravel = walk("gravel", 0.3, 24);
    let peaks: Vec<f32> = gravel.iter().map(|s| peak_db(s)).collect();
    let mean = peaks.iter().sum::<f32>() / peaks.len() as f32;
    let sd = (peaks.iter().map(|p| (p - mean) * (p - mean)).sum::<f32>() / peaks.len() as f32).sqrt();
    // Recordings: step peaks spread by 2-5 dB.
    assert!((1.5..6.0).contains(&sd), "step peaks spread by {sd:.1} dB");
    let cents: Vec<f32> = gravel.iter().take(8).map(|s| centroid(&welch(&s[..(0.2 * SR) as usize], 2048))).collect();
    let (lo, hi) = (cents.iter().cloned().fold(f32::MAX, f32::min), cents.iter().cloned().fold(0.0, f32::max));
    assert!(hi > lo * 1.15, "every step should have its own colour: centroids {lo:.0}..{hi:.0} Hz");

    // Walking on wood the toe lands 60-140 ms after the heel: a second knock in the envelope.
    let wood = walk("wood", 0.3, 10);
    let toe_at = median(wood.iter().map(|s| toe_ms(s)).collect());
    assert!((50.0..150.0).contains(&toe_at), "toe lands {toe_at} ms after the heel");
}

#[test]
fn running_hits_harder_and_rolls_quicker() {
    let level = |s: &[Vec<f32>]| median(s.iter().map(|x| peak_db(x)).collect());
    for surface in ["gravel", "wood", "snow"] {
        let mut m = walker(surface);
        let walking = steps(m.as_mut(), 24, 0.55);
        m.set_input_by_name("speed", 0.75);
        let running = steps(m.as_mut(), 24, 0.55);
        // Across walkers: 3.4 to 7.9 dB.
        assert!(level(&running) > level(&walking) + 2.5, "{surface}: running {:.1} dB vs walking {:.1} dB", level(&running), level(&walking));
        if surface == "wood" {
            // Running, the foot lands nearly flat: the toe follows the heel sooner.
            let toe = |s: &[Vec<f32>]| median(s.iter().map(|x| toe_ms(x)).collect());
            assert!(toe(&running) < toe(&walking) - 15.0, "wood: toe at {} ms running, {} ms walking", toe(&running), toe(&walking));
            continue;
        }
        // Loose ground: the foot lifts sooner, so the crunch is over sooner. Time centre of the
        // step's energy:
        let centre = |s: &[Vec<f32>]| {
            median(
                s.iter()
                    .map(|x| {
                        let e = envelope(&x[..(0.3 * SR) as usize], 1.0);
                        let w: f32 = e.iter().map(|v| v * v).sum();
                        e.iter().enumerate().map(|(i, v)| i as f32 * v * v).sum::<f32>() / w
                    })
                    .collect(),
            )
        };
        // Across walkers: 0.73 to 0.91 of the walking step's.
        assert!(centre(&running) < centre(&walking) * 0.95, "{surface}: run centre {:.0} ms, walk {:.0} ms", centre(&running), centre(&walking));
    }
}

#[test]
fn weight_and_shoe_change_the_step() {
    let at = |m: &mut dyn Model, weight: f32, shoe: f32| {
        m.set_input_by_name("weight", weight);
        m.set_input_by_name("shoe", shoe);
        steps(m, 16, 0.55)
    };
    let mut m = walker("dirt");
    let (light, heavy) = (at(m.as_mut(), 0.0, 0.5), at(m.as_mut(), 1.0, 0.5));
    let level = |s: &[Vec<f32>]| median(s.iter().map(|x| peak_db(x)).collect());
    assert!(level(&heavy) > level(&light) + 2.0, "heavy {:.1} dB vs light {:.1} dB", level(&heavy), level(&light));
    let c = |s: &[Vec<f32>]| centroid(&step_spectrum(s));
    assert!(c(&heavy) < c(&light) * 0.85, "heavy should be lower: {:.0} vs {:.0} Hz", c(&heavy), c(&light));

    // Hard soles click on stone; on grass the sole matters far less.
    let top = |s: &[Vec<f32>]| share(&spectrum(s, 0.0, 0.1, 2048), 2000.0, 24000.0);
    let mut m = walker("stone");
    let stone = top(&at(m.as_mut(), 0.5, 1.0)) / top(&at(m.as_mut(), 0.5, 0.0));
    let mut m = walker("grass");
    let grass = top(&at(m.as_mut(), 0.5, 1.0)) / top(&at(m.as_mut(), 0.5, 0.0));
    assert!(stone > 3.0, "hard soles should click on stone: {stone:.1}x the top of soft ones");
    assert!(grass < stone / 2.0, "on grass the shoe should matter less: {grass:.1}x vs {stone:.1}x on stone");
}

#[test]
fn a_landing_is_both_feet_and_heavier() {
    let mut m = walker("dirt");
    let walking = steps(m.as_mut(), 8, 0.55);
    m.set_input_by_name("land", 0.8);
    let landing = steps(m.as_mut(), 8, 0.8);
    let level = |s: &[Vec<f32>]| median(s.iter().map(|x| peak_db(x)).collect());
    assert!(level(&landing) > level(&walking) + 4.0, "landing {:.1} dB vs step {:.1} dB", level(&landing), level(&walking));
    // Both feet within 30 ms: almost all of the energy is in the first 60 ms, no toe roll later.
    let late = median(landing.iter().map(|x| tail_db(x)).collect());
    assert!(late < median(walking.iter().map(|x| tail_db(x)).collect()), "a landing should not roll onto the toe");
    assert!(landing.iter().all(|x| peak(x) <= 1.0));
}

#[test]
fn surface_changes_take_effect_on_the_next_step_only() {
    // A step on metal keeps ringing after the game has moved the walker onto grass.
    let mut m = walker("metal");
    m.trigger();
    let mut head = vec![0.0; (0.01 * SR) as usize];
    m.render_mono(&mut head);
    m.set_input_by_name("surface", 0.1);
    let mut tail = vec![0.0; (0.3 * SR) as usize];
    m.render_mono(&mut tail);
    assert!(tonality(&welch(&tail[(0.03 * SR) as usize..], 4096)) > 10.0, "the metal step was cut short by the surface change");
    // And the next step is grass.
    let grass = steps(m.as_mut(), 6, 0.55);
    assert!(ring_db(&grass) < 10.0, "the step after the change should be grass");
}

#[test]
fn pace_walks_by_itself_at_its_cadence_and_two_walkers_differ() {
    let mut m = walker("wood");
    m.set_input_by_name("pace", 0.45);
    let mut x = vec![0.0; (10.0 * SR) as usize];
    m.render_mono(&mut x);
    // Count steps: 5 ms envelope rising 15 dB, at least 0.3 s apart.
    let e = envelope(&x, 5.0);
    let mut onsets = vec![];
    let mut last = -100i32;
    for i in 6..e.len() {
        let floor = e[i - 6..i].iter().cloned().fold(f32::MAX, f32::min).max(1e-6);
        if e[i] > floor * 5.6 && i as i32 - last > 60 {
            onsets.push(i);
            last = i as i32;
        }
    }
    // 1.8 steps per second.
    assert!((17..=19).contains(&onsets.len()), "{} steps in 10 s at pace 0.45", onsets.len());
    let gaps: Vec<f32> = onsets.windows(2).map(|w| (w[1] - w[0]) as f32 * 0.005).collect();
    let (lo, hi) = (gaps.iter().cloned().fold(f32::MAX, f32::min), gaps.iter().cloned().fold(0.0, f32::max));
    assert!(hi - lo > 0.005 && hi < 0.62 && lo > 0.49, "strides {lo:.3}..{hi:.3} s: even, but not a metronome");

    // Two walkers given the same steps do not sound the same.
    let (mut a, mut b) = (walker("gravel"), walker("gravel"));
    let (sa, sb) = (steps(a.as_mut(), 1, 0.3).remove(0), steps(b.as_mut(), 1, 0.3).remove(0));
    let diff = sa.iter().zip(&sb).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(diff > 0.01, "two walkers stepped identically");
}

#[test]
fn steps_sit_with_the_other_events_and_never_clip() {
    // A walk's median step peaks near -11 dBFS on every surface, its loudest steps near
    // rock_hit (-3.8 dBFS at full power); a heavy runner's landings stay inside the limiter.
    for surface in SURFACES {
        let s = walk(surface, 0.3, 16);
        let level = median(s.iter().map(|x| peak_db(x)).collect());
        assert!((-15.0..-5.0).contains(&level), "{surface}: median step peak {level:.1} dBFS");
        let mut m = walker(surface);
        for (input, v) in [("speed", 1.0), ("weight", 1.0), ("shoe", 1.0), ("land", 1.0)] {
            m.set_input_by_name(input, v);
        }
        for x in steps(m.as_mut(), 6, 0.3) {
            assert!(x.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "{surface}: out of range");
        }
    }
}

#[test]
fn many_walkers_are_cheap() {
    // A crowd: 32 walkers running at 2.8 steps/s on mixed ground.
    let mut crowd: Vec<_> = (0..32)
        .map(|k| {
            let mut m = walker(SURFACES[k % SURFACES.len()]);
            m.set_input_by_name("pace", 0.7);
            m.set_input_by_name("speed", 0.7);
            m
        })
        .collect();
    let secs = 2.0;
    let mut buf = vec![0.0; 512];
    let start = std::time::Instant::now();
    for _ in 0..((secs * SR) as usize / 512) {
        for m in crowd.iter_mut() {
            m.render_mono(&mut buf);
        }
    }
    let elapsed = start.elapsed().as_secs_f32();
    println!("32 running walkers render at {:.1}x real time", secs / elapsed);
    // Generous for debug builds; a release build is far faster.
    assert!(elapsed < secs, "32 walkers took {elapsed:.2} s for {secs} s");
}
