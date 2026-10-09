#![cfg(feature = "graph")]
//! The shared DSP building blocks (`brusverk_core::dsp`) and their model-file nodes.
use brusverk_core::dsp::{self, Air, Chirp, Envelope, FmOp, Formant, Modal, Pattern, PowerDust, Rhythm, MAX_FORMANTS};
use brusverk_core::render::{peak, rms};
use brusverk_core::{GraphModel, Model};

const SR: f32 = 48000.0;

/// Magnitude of `x` at `hz` (Hann-windowed single-bin DFT), normalised so a sine of amplitude
/// 1 reads about 0.5.
fn mag_at(x: &[f32], hz: f32) -> f32 {
    let n = x.len() as f32;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, s) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n).cos();
        let ph = std::f64::consts::TAU * hz as f64 * i as f64 / SR as f64;
        re += (s * w) as f64 * ph.cos();
        im += (s * w) as f64 * ph.sin();
    }
    ((re * re + im * im).sqrt() / n as f64) as f32
}

/// Frequency from zero crossings of `x` (upward crossings over the span between first and last).
fn zc_freq(x: &[f32]) -> f32 {
    let ups: Vec<f32> = x.windows(2).enumerate().filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0).map(|(i, w)| i as f32 + w[0] / (w[0] - w[1])).collect();
    assert!(ups.len() >= 3, "too few crossings");
    (ups.len() - 1) as f32 / ((ups[ups.len() - 1] - ups[0]) / SR)
}

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

// ---------------------------------------------------------------------------------------------
// Power-law dust
// ---------------------------------------------------------------------------------------------

#[test]
fn power_dust_sizes_follow_the_power_law() {
    for skew in [1.0f32, 2.0, 3.0, 5.0] {
        let mut d = PowerDust::new(7);
        let n = 400_000;
        let sizes: Vec<f32> = (0..n).map(|_| d.tick(0.25, skew)).filter(|a| *a > 0.0).collect();
        // Poisson rate: a quarter of the samples.
        let rate = sizes.len() as f32 / n as f32;
        assert!((rate - 0.25).abs() < 0.005, "event rate {rate}");
        // P(size <= x) = x^(1/skew), so log P against log x is a line of slope 1/skew.
        let cdf = |x: f32| sizes.iter().filter(|a| **a <= x).count() as f32 / sizes.len() as f32;
        for x in [0.001f32, 0.01, 0.1, 0.3, 0.7] {
            let (got, want) = (cdf(x), x.powf(1.0 / skew));
            assert!((got - want).abs() < 0.01, "skew {skew}: P(size <= {x}) = {got}, power law says {want}");
        }
        let slope = (cdf(0.5).ln() - cdf(0.005).ln()) / (0.5f32.ln() - 0.005f32.ln());
        assert!((slope * skew - 1.0).abs() < 0.03, "skew {skew}: log-log slope {slope}, want {}", 1.0 / skew);
        assert!(sizes.iter().all(|a| *a <= 1.0));
    }
    // Skew 0: every event the same size.
    let mut d = PowerDust::new(3);
    assert!((0..10_000).map(|_| d.tick(0.3, 0.0)).all(|a| a == 0.0 || a == 1.0));
}

// ---------------------------------------------------------------------------------------------
// Modal bank
// ---------------------------------------------------------------------------------------------

/// Ring time (60 dB) of the component at `hz`, from its level in successive windows.
fn measured_t60(x: &[f32], hz: f32, from_s: f32, to_s: f32) -> f32 {
    let win = 4096;
    let (a, b) = ((from_s * SR) as usize, (to_s * SR) as usize);
    let db = |i: usize| 20.0 * mag_at(&x[i..i + win], hz).log10();
    let slope = (db(b) - db(a)) / ((b - a) as f32 / SR);
    -60.0 / slope
}

#[test]
fn modal_bank_rings_each_mode_for_its_own_t60() {
    let (ratios, t60) = ([1.0, 2.76, 5.4], [2.0, 0.8, 0.3]);
    let mut m = Modal::new(&ratios, &t60, &[1.0, 1.0, 1.0], 1);
    m.set(400.0, 0.5, 1.0, SR);
    let mut out = vec![0.0f32; (2.5 * SR) as usize];
    for (i, o) in out.iter_mut().enumerate() {
        *o = m.tick(if i == 0 { 1.0 } else { 0.0 }, 0.23, 0.0, SR);
        if i % 32 == 31 {
            m.end_block();
        }
    }
    for (r, want) in ratios.iter().zip(t60) {
        let got = measured_t60(&out, 400.0 * r, 0.05, (want * 0.5).min(1.5));
        assert!((got / want - 1.0).abs() < 0.08, "mode {r}: t60 {got:.3} s, set {want} s");
    }
    // Silent once rung out: nothing left ringing.
    assert!(out[out.len() - 100..].iter().all(|s| s.abs() < 1e-3));
}

#[test]
fn modal_bank_bigger_is_lower_and_longer() {
    let ring = |size: f32| {
        let mut m = Modal::new(&[1.0], &[0.5], &[1.0], 2);
        m.set(440.0, size, 1.0, SR);
        let x: Vec<f32> = (0..SR as usize).map(|i| m.tick(if i == 0 { 1.0 } else { 0.0 }, 0.23, 0.0, SR)).collect();
        (zc_freq(&x[100..20000]), measured_t60(&x, zc_freq(&x[100..20000]), 0.02, 0.15))
    };
    let (f_mid, t_mid) = ring(0.5);
    let (f_big, t_big) = ring(0.75);
    let (f_small, t_small) = ring(0.25);
    assert!((f_mid - 440.0).abs() < 2.0, "size 0.5 plays as written: {f_mid}");
    assert!((f_big - 220.0).abs() < 2.0, "size 0.75 is an octave down: {f_big}");
    assert!((f_small - 880.0).abs() < 4.0, "size 0.25 is an octave up: {f_small}");
    assert!(t_big > t_mid * 1.4 && t_mid > t_small * 1.4, "rings longer as it grows: {t_small} {t_mid} {t_big}");
}

#[test]
fn modal_bank_adds_new_hits_to_the_ringing_modes() {
    // With no variation the bank is linear: two hits are the sum of each alone, not a restart.
    let run = |hits: &[(usize, f32)]| {
        let mut m = Modal::new(&[1.0, 2.3, 4.1], &[1.5, 0.7, 0.3], &[1.0, 0.6, 0.4], 9);
        m.set(300.0, 0.5, 1.0, SR);
        (0..SR as usize)
            .map(|i| {
                let x = hits.iter().find(|h| h.0 == i).map_or(0.0, |h| h.1);
                let y = m.tick(x, 0.23, 0.0, SR);
                if i % 32 == 31 {
                    m.end_block();
                }
                y
            })
            .collect::<Vec<f32>>()
    };
    let (a, b, both) = (run(&[(0, 1.0)]), run(&[(7000, 0.6)]), run(&[(0, 1.0), (7000, 0.6)]));
    let err = a.iter().zip(&b).zip(&both).map(|((a, b), c)| (a + b - c).abs()).fold(0.0, f32::max);
    assert!(err < 1e-4, "second hit must add to the ring: max error {err}");
    // And the first hit is still ringing after the second one.
    assert!(rms(&a[7000..9000]) > 0.1);
}

#[test]
fn modal_strikes_vary_with_variation() {
    let hit = |variation: f32| {
        let mut m = Modal::new(&[1.0, 2.0, 3.0, 4.0], &[0.5], &[1.0], 4);
        m.set(500.0, 0.5, 1.0, SR);
        let mut runs = Vec::new();
        for _ in 0..2 {
            m.clear();
            runs.push((0..4800).map(|i| m.tick(if i == 0 { 1.0 } else { 0.0 }, 0.23, variation, SR)).collect::<Vec<f32>>());
        }
        runs[0].iter().zip(&runs[1]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
    };
    assert!(hit(0.0) < 1e-6, "no variation: every strike the same");
    assert!(hit(1.0) > 1e-2, "variation: strikes differ");
}

// ---------------------------------------------------------------------------------------------
// Chirp and FM
// ---------------------------------------------------------------------------------------------

#[test]
fn chirp_reaches_its_end_frequency_in_its_time() {
    let glide = |curve: f32| {
        let mut c = Chirp::default();
        c.start(2000.0, 500.0, 0.2, curve);
        (0..(0.4 * SR) as usize).map(|_| c.tick(1.0, 0.0, SR)).collect::<Vec<f32>>()
    };
    let at = |x: &[f32], t: f32| zc_freq(&x[(t * SR) as usize - 240..(t * SR) as usize + 240]);
    let x = glide(1.0);
    let law = |t: f32| 2000.0 * 0.25f32.powf(t / 0.2);
    assert!((at(&x, 0.01) / law(0.01) - 1.0).abs() < 0.02, "starts at from: {}", at(&x, 0.01));
    // Even in octaves: halfway in time is halfway in pitch (1 kHz).
    assert!((at(&x, 0.1) / 1000.0 - 1.0).abs() < 0.03, "midpoint {}", at(&x, 0.1));
    assert!(at(&x, 0.18) > 540.0, "still gliding before its time: {}", at(&x, 0.18));
    let end = zc_freq(&x[(0.2 * SR) as usize..(0.4 * SR) as usize]);
    assert!((end - 500.0).abs() < 2.0, "holds `to` from its time on: {end}");
    // Curve 2 lingers high and falls late.
    assert!(at(&glide(2.0), 0.1) > 1400.0);
    assert!(at(&glide(0.5), 0.1) < 760.0);
    // The phase never jumps: no sample-to-sample step bigger than the steepest sine's.
    let mut c = Chirp::default();
    c.start(800.0, 800.0, 0.1, 1.0);
    let mut prev = 0.0;
    for i in 0..9600 {
        if i == 4000 {
            c.start(3000.0, 600.0, 0.05, 1.0);
        }
        let y = c.tick(1.0, 0.0, SR);
        assert!((y - prev).abs() < std::f32::consts::TAU * 3000.0 / SR * 1.05, "jump at {i}");
        prev = y;
    }
}

#[test]
fn fm_operator_has_sidebands_and_no_dc() {
    for ratio in [1.0f32, 0.5, 2.0, 3.5] {
        let mut op = FmOp::default();
        let x: Vec<f32> = (0..48000).map(|_| op.tick(440.0 / SR, ratio, 2.0)).collect();
        let dc = x.iter().sum::<f32>() / x.len() as f32;
        assert!(dc.abs() < 2e-3, "ratio {ratio}: DC {dc}");
        // Index 2 puts real energy into the first sideband pair.
        assert!(mag_at(&x, 440.0 * (1.0 + ratio)) > 0.05, "ratio {ratio}: no upper sideband");
    }
    let mut op = FmOp::default();
    let pure: Vec<f32> = (0..48000).map(|_| op.tick(440.0 / SR, 2.0, 0.0)).collect();
    assert!(mag_at(&pure, 1320.0) < 1e-3, "index 0 is a pure sine");
}

// ---------------------------------------------------------------------------------------------
// Envelopes
// ---------------------------------------------------------------------------------------------

/// Largest sample-to-sample change.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
}

#[test]
fn ad_envelope_is_click_free_and_ends_at_exactly_zero() {
    let (attack, decay) = (0.01, 0.25);
    let mut e = Envelope::default();
    let dt = 1.0 / SR;
    let mut x = vec![0.0f32];
    for i in 0..(0.6 * SR) as usize {
        if i == 0 {
            e.trigger(1.0);
        }
        // Retrigger halfway down the decay: it must not jump.
        if i == (0.1 * SR) as usize {
            e.trigger(0.8);
        }
        x.push(e.tick(dt, attack, 0.0, decay, 0.0, decay));
    }
    assert!(x[1] < 1e-4, "onset starts from zero: {}", x[1]);
    // An S-curve attack's steepest step is 1.5 / attack samples; decays are gentler still.
    let bound = 1.6 / (attack * SR);
    assert!(max_step(&x) < bound, "step {} > {bound}", max_step(&x));
    assert!((peak(&x) - 1.0).abs() < 1e-6);
    // Lands on exactly zero about `decay` after its peak, and stays there: no lingering DC.
    let end = x.iter().rposition(|v| *v != 0.0).unwrap() as f32 / SR;
    let expect = 0.1 + attack + decay;
    assert!((end - expect).abs() < 0.03, "ended at {end}, expected about {expect}");
    assert!(x[x.len() - 2000..].iter().all(|v| *v == 0.0));
    assert!(e.is_idle());
    let last = x[x.iter().rposition(|v| *v != 0.0).unwrap()];
    assert!(last < 2e-4, "no step at the end: last value {last}");
}

#[test]
fn adsr_sustains_and_releases_without_steps() {
    let mut e = Envelope::default();
    let dt = 1.0 / SR;
    let gate_off = (0.4 * SR) as usize;
    let mut x = Vec::new();
    e.trigger(1.0);
    for i in 0..SR as usize {
        if i == gate_off {
            e.release();
        }
        x.push(e.tick(dt, 0.02, 0.0, 0.1, 0.6, 0.2));
    }
    assert!((x[gate_off - 1] - 0.6).abs() < 1e-6, "holds its sustain level: {}", x[gate_off - 1]);
    assert!(max_step(&x) < 1.6 / (0.02 * SR));
    assert!(x[gate_off + (0.25 * SR) as usize..].iter().all(|v| *v == 0.0), "released to exactly zero");
    // Released during its attack: falls from where it is.
    let mut e = Envelope::default();
    e.trigger(1.0);
    let mut y = Vec::new();
    for i in 0..9600 {
        if i == 200 {
            e.release();
        }
        y.push(e.tick(dt, 0.05, 0.0, 0.1, 0.6, 0.05));
    }
    assert!(max_step(&y) < 1.6 / (0.05 * SR));
    assert!(*y.last().unwrap() == 0.0);
}

// ---------------------------------------------------------------------------------------------
// Formants
// ---------------------------------------------------------------------------------------------

/// Local maxima of the magnitude response of `f` (impulse response, 10 Hz grid).
fn formant_peaks(f: &mut Formant, levels: &[f32; MAX_FORMANTS]) -> Vec<f32> {
    let ir: Vec<f32> = (0..16384).map(|i| f.tick(if i == 0 { 1.0 } else { 0.0 }, levels)).collect();
    let grid: Vec<f32> = (10..500).map(|k| k as f32 * 10.0).collect();
    let mags: Vec<f32> = grid.iter().map(|hz| mag_at(&ir, *hz)).collect();
    (1..grid.len() - 1).filter(|&i| mags[i] > mags[i - 1] && mags[i] >= mags[i + 1]).map(|i| grid[i]).collect()
}

#[test]
fn formant_peaks_land_where_they_are_set() {
    let set = [(500.0f32, 8.0f32), (1500.0, 10.0), (2600.0, 12.0)];
    for series in [false, true] {
        let mut f = Formant::new(series);
        f.set(&set, SR);
        let peaks = formant_peaks(&mut f, &[1.0, 0.7, 0.5, 0.0]);
        for (hz, _) in set {
            assert!(peaks.iter().any(|p| (p / hz - 1.0).abs() < 0.03), "series {series}: no peak near {hz} in {peaks:?}");
        }
    }
    // Vowel presets: 'a' (0) and 'i' (2).
    for (v, f1, f2) in [(0.0, 730.0, 1090.0), (2.0, 270.0, 2290.0)] {
        let fs = dsp::vowel(v);
        let mut f = Formant::new(false);
        f.set(&fs.map(|(hz, q, _)| (hz, q)), SR);
        let levels = [fs[0].2, fs[1].2, fs[2].2, 0.0];
        let peaks = formant_peaks(&mut f, &levels);
        for hz in [f1, f2] {
            assert!(peaks.iter().any(|p| (p / hz - 1.0).abs() < 0.04), "vowel {v}: no peak near {hz} in {peaks:?}");
        }
    }
    // A parallel formant peaks at its level (each band-pass is normalised to unity).
    let mut f = Formant::new(false);
    f.set(&[(1000.0, 10.0), (3000.0, 10.0)], SR);
    let mut x = vec![0.0f32; 48000];
    let mut ph = 0.0f32;
    for s in x.iter_mut() {
        ph += 1000.0 / SR;
        *s = f.tick((ph * std::f32::consts::TAU).sin(), &[0.5, 1.0, 0.0, 0.0]);
    }
    assert!((peak(&x[24000..]) - 0.5).abs() < 0.05, "gain at F1 {}", peak(&x[24000..]));
}

// ---------------------------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------------------------

/// Sample positions of the impulses of `secs` of a pattern.
fn events(p: &mut Pattern, r: &Rhythm, secs: f32) -> Vec<(usize, f32)> {
    (0..(secs * SR) as usize).filter_map(|i| Some((i, p.tick(true, r, SR))).filter(|e| e.1 > 0.0)).collect()
}

/// Split events into phrases wherever a pause is longer than 1.5 steps.
fn phrases(ev: &[(usize, f32)], step: f32) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (i, e) in ev.iter().enumerate() {
        if i == 0 || (e.0 - ev[i - 1].0) as f32 > 1.5 * step * SR {
            out.push(Vec::new());
        }
        out.last_mut().unwrap().push(e.0);
    }
    out
}

#[test]
fn pattern_counts_and_gaps_per_phrase() {
    let r = Rhythm { rate: 10.0, count: 4.0, gap: 0.5, jitter: 0.0, variation: 0.0 };
    let mut p = Pattern::new(1, None);
    let ev = events(&mut p, &r, 6.0);
    let ph = phrases(&ev, 0.1);
    assert!(ph.len() >= 6, "{} phrases", ph.len());
    for w in ph.windows(2) {
        assert_eq!(w[0].len(), 4, "events per phrase");
        for k in w[0].windows(2) {
            assert!(((k[1] - k[0]) as f32 / SR - 0.1).abs() < 1.5 / SR, "step {}", (k[1] - k[0]) as f32 / SR);
        }
        // From the last event to the next phrase: one step plus the gap.
        let rest = (w[1][0] - w[0][3]) as f32 / SR;
        assert!((rest - 0.6).abs() < 1.5 / SR, "rest {rest}");
    }
    assert_eq!(ev[0].0, 0, "the first phrase starts at once");

    // Jitter moves events but keeps the counts; variation rolls the counts within +-50%.
    let r = Rhythm { jitter: 0.5, variation: 1.0, ..r };
    let mut p = Pattern::new(2, None);
    let ph = phrases(&events(&mut p, &r, 30.0), 0.1);
    let counts: Vec<usize> = ph.iter().map(|q| q.len()).collect();
    assert!(counts.iter().all(|c| (2..=6).contains(c)), "{counts:?}");
    assert!(counts.iter().any(|c| *c != 4), "variation should vary the count");
}

#[test]
fn pattern_rests_follow_the_game() {
    // A 10 s rest starts; 1 s in, the game shortens the gap to 0.2 s: the next phrase starts at
    // once instead of 9 s later.
    let mut p = Pattern::new(4, None);
    let long = Rhythm { rate: 10.0, count: 2.0, gap: 10.0, jitter: 0.0, variation: 0.0 };
    let short = Rhythm { gap: 0.2, ..long };
    let ev: Vec<usize> = (0..(3.0 * SR) as usize).filter(|&i| p.tick(true, if i < SR as usize { &long } else { &short }, SR) > 0.0).collect();
    assert_eq!(&ev[..2], &[0, 4800]);
    assert_eq!(ev[2], SR as usize, "the shortened rest is already over");
    // A gap that grows mid-rest does not stretch it.
    let mut p = Pattern::new(5, None);
    let grow = Rhythm { gap: 30.0, ..short };
    let ev: Vec<usize> = (0..SR as usize).filter(|&i| p.tick(true, if i < 5000 { &short } else { &grow }, SR) > 0.0).collect();
    assert_eq!(ev[2], 4800 + 4800 + (0.2 * SR) as usize);
}

#[test]
fn pattern_euclidean_rhythms() {
    // Tresillo: 3 of 8 at 8 steps a second, a continuous loop.
    let r = Rhythm { rate: 8.0, count: 99.0, gap: 0.0, jitter: 0.0, variation: 0.0 };
    let mut p = Pattern::new(3, Some((3, 8, 0)));
    let steps: Vec<usize> = events(&mut p, &r, 3.0).iter().map(|e| (e.0 as f32 / (SR / 8.0)).round() as usize).collect();
    assert_eq!(steps, [0, 3, 6, 8, 11, 14, 16, 19, 22]);
    let hits: Vec<bool> = (0..16).map(|i| dsp::euclid(i, 5, 16, 0)).collect();
    assert_eq!(hits.iter().filter(|h| **h).count(), 5);
}

// ---------------------------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------------------------

#[test]
fn pan_is_equal_power_and_distance_dulls_and_quietens() {
    for i in 0..=40 {
        let pan = i as f32 / 20.0 - 1.0;
        let (l, r) = dsp::equal_power(pan);
        assert!((l * l + r * r - 1.0).abs() < 1e-5, "power at pan {pan}");
        assert!(l >= 0.0 && r >= 0.0);
    }
    let (l, r) = dsp::equal_power(0.0);
    assert!((l - r).abs() < 1e-6 && (l - 0.5f32.sqrt()).abs() < 1e-6, "centre is -3 dB each");
    assert!(dsp::equal_power(-1.0).1.abs() < 1e-6);
    assert!(dsp::equal_power(1.0).0.abs() < 1e-6);

    // Distance: white noise through the air at 0 and 1.
    let through = |d: f32| {
        let mut air = Air::default();
        air.set(d, 18000.0, 1000.0, brusverk_core::math::db_to_gain(-24.0 * d), SR);
        let mut rng = brusverk_core::Rng::new(5);
        let x: Vec<f32> = (0..48000).map(|_| air.tick(rng.next_bipolar())).collect();
        let hf = mag_at(&x, 8000.0) / mag_at(&x, 300.0).max(1e-9);
        (rms(&x), hf, zc_freq(&x))
    };
    let (near, far) = (through(0.0), through(1.0));
    assert!(far.0 < near.0 * 0.2, "farther is quieter: {} vs {}", far.0, near.0);
    assert!(far.1 < near.1 * 0.2, "farther is duller: hf ratio {} vs {}", far.1, near.1);
    assert!(far.2 < near.2 * 0.5, "farther is duller: zero-crossing rate {} vs {}", far.2, near.2);
}

// ---------------------------------------------------------------------------------------------
// Graph nodes
// ---------------------------------------------------------------------------------------------

fn model(extra: &str, nodes: &str, out: &str) -> GraphModel {
    let text = format!("{extra}\n[graph]\nnodes = [\n{nodes}\n]\nout = \"{out}\"\n");
    GraphModel::from_text(&text, SR).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

/// Renders 3 s mono and stereo and checks they are finite, bounded and not silent.
fn sounds(m: &mut GraphModel, what: &str) -> Vec<f32> {
    let out = render(m, 3.0);
    assert!(out.iter().all(|x| x.is_finite()), "{what}: NaN");
    assert!(peak(&out) <= 1.0, "{what}: peak {}", peak(&out));
    assert!(rms(&out) > 1e-3, "{what}: silent ({})", rms(&out));
    let (mut l, mut r) = (vec![0.0; 48000], vec![0.0; 48000]);
    m.render_stereo(&mut l, &mut r);
    assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() <= 1.0), "{what}: stereo out of bounds");
    out
}

#[test]
fn every_new_node_parses_runs_and_is_bounded() {
    let cases: &[(&str, &str)] = &[
        ("powerdust", r#"{ id = "d", type = "powerdust", rate = 300, skew = "2 + sh(3)" }, { id = "o", type = "gain", in = ["d"], gain = 1 }"#),
        ("impulse", r#"{ id = "i", type = "impulse", rate = 20, amp = 0.9 }, { id = "o", type = "ad", in = ["i"], decay = 0.02 }"#),
        ("pattern", r#"{ id = "p", type = "pattern", rate = 12, count = 3, gap = 0.3, jitter = 0.2, variation = 0.5 }, { id = "o", type = "decay", in = ["p"], ms = 20 }"#),
        ("pattern euclid", r#"{ id = "p", type = "pattern", rate = 8, gap = 0, euclid = [3, 8, 1] }, { id = "o", type = "decay", in = ["p"], ms = 20 }"#),
        ("ad", r#"{ id = "p", type = "dust", rate = 5 }, { id = "e", type = "ad", in = ["p"], attack = 0.002, hold = 0.01, decay = "0.2 + 0.1 * lfo(1)" }, { id = "s", type = "sine", freq = 440 }, { id = "o", type = "mul", in = ["s"], by = ["e"] }"#),
        ("adsr", r#"{ id = "e", type = "adsr", gate = "lfo(0.7) > 0", attack = 0.05, sustain = 0.5 }, { id = "s", type = "saw", freq = 110 }, { id = "o", type = "mul", in = ["s"], by = ["e"] }"#),
        ("chirp", r#"{ id = "p", type = "pattern", rate = 6, count = 3, gap = 0.4 }, { id = "c", type = "chirp", in = ["p"], from = 4000, to = 2500, time = 0.08, curve = 0.7, ratio = 0.02, index = 1.5 }, { id = "e", type = "ad", in = ["p"], decay = 0.1 }, { id = "o", type = "mul", in = ["c"], by = ["e"] }"#),
        ("fm steady", r#"{ id = "o", type = "fm", freq = "220 * (1 + 0.1 * lfo(2))", ratio = 2, index = 3 }"#),
        ("fm triggered", r#"{ id = "p", type = "dust", rate = 3 }, { id = "f", type = "fm", in = ["p"], freq = 330, ratio = 3.5, index = 5, index_decay = 0.3 }, { id = "o", type = "gain", in = ["f"], gain = 0.5 }"#),
        ("modal", r#"{ id = "p", type = "powerdust", rate = 8, skew = 2 }, { id = "m", type = "modal", in = ["p"], freq = 520, ratios = [1, 2.76, 5.4, 8.93], t60 = [1.5, 0.8, 0.4, 0.2], levels = [1, 0.6, 0.4, 0.2], size = "0.5 + 0.3 * lfo(0.5)" }, { id = "o", type = "gain", in = ["m"], gain = 0.3 }"#),
        ("modal one t60", r#"{ id = "n", type = "noise" }, { id = "l", type = "lowpass", in = ["n"], cutoff = 300 }, { id = "o", type = "modal", in = [{ from = "l", gain = 0.05 }], freq = 200, ratios = [1, 1.5], t60 = 0.5 }"#),
        ("formant vowel", r#"{ id = "s", type = "saw", freq = 120 }, { id = "o", type = "formant", in = ["s"], vowel = "2 + 2 * lfo(0.5)", shift = 1.2 }"#),
        ("formant custom", r#"{ id = "s", type = "pulse", freq = 90, width = 0.2 }, { id = "o", type = "formant", in = ["s"], freqs = [400, 1100], qs = [6, 9], gains = [1, 0.4], routing = "series" }"#),
        ("pan", r#"{ id = "s", type = "sine", freq = 440 }, { id = "o", type = "pan", in = ["s"], pan = "lfo(1)", channel = "left" }"#),
        ("distance", r#"{ id = "n", type = "noise" }, { id = "o", type = "distance", in = ["n"], distance = "0.5 + 0.5 * lfo(0.3)", delay_ms = 30, far_hz = 800 }"#),
    ];
    for (what, nodes) in cases {
        let mut m = model("", nodes, "o");
        sounds(&mut m, what);
    }
}

#[test]
fn stereo_model_files() {
    let nodes = r#"{ id = "s", type = "sine", freq = 440 },
        { id = "l", type = "pan", in = [{ from = "s", gain = 0.5 }], pan = "pan", channel = "left" },
        { id = "r", type = "pan", in = [{ from = "s", gain = 0.5 }], pan = "pan", channel = "right" }"#;
    let text = |pan: f32| format!("[params]\npan = {{ default = {pan}, min = -1, max = 1 }}\n[graph]\nnodes = [{nodes}]\nout = \"l\"\nout_right = \"r\"\n");
    let mut m = GraphModel::from_text(&text(-0.6), SR).unwrap();
    assert!(m.is_stereo());
    let (mut l, mut r) = (vec![0.0; 24000], vec![0.0; 24000]);
    m.render_stereo(&mut l, &mut r);
    let (gl, gr) = dsp::equal_power(-0.6);
    assert!((rms(&l[4800..]) / rms(&r[4800..]) - gl / gr).abs() < 0.01, "left louder by the pan law: {} vs {}", rms(&l[4800..]) / rms(&r[4800..]), gl / gr);
    // Under the limiter's knee the channels keep exactly the pan law.
    assert!(peak(&l) < 0.7);
    let mut mono = GraphModel::from_text(&text(-0.6), SR).unwrap();
    let x = render(&mut mono, 0.5);
    // The mono render is the same sound with its pans centred, not a fold-down.
    assert!((rms(&x[4800..]) - 0.25).abs() < 0.005, "mono rms {}", rms(&x[4800..]));
    // A model without out_right plays the same in both channels.
    let mut plain = model("", r#"{ id = "s", type = "sine", freq = 440 }, { id = "o", type = "pan", in = ["s"], pan = -1, channel = "left" }"#, "o");
    assert!(!plain.is_stereo());
    plain.render_stereo(&mut l, &mut r);
    assert_eq!(l, r);
}

#[test]
fn one_shot_triggers_reach_the_nodes() {
    let text = r#"[model]
name = "ping"
one_shot = true
[graph]
nodes = [
  { id = "hit", type = "impulse" },
  { id = "env", type = "ad", in = ["hit"], attack = 0.002, decay = 0.4 },
  { id = "pitch", type = "chirp", in = ["hit"], from = 1800, to = 900, time = 0.1 },
  { id = "o", type = "mul", in = ["pitch"], by = [{ from = "env", gain = 0.5 }] },
]
out = "o"
"#;
    let mut m = GraphModel::from_text(text, SR).unwrap();
    assert!(render(&mut m, 0.2).iter().all(|x| *x == 0.0), "silent before the trigger");
    m.trigger();
    let a = render(&mut m, 1.0);
    assert!(peak(&a) > 0.4 && peak(&a) <= 0.5, "peak {}", peak(&a));
    assert!(a[a.len() - 4800..].iter().all(|x| *x == 0.0), "ends in true silence");
    assert!(m.is_finished());
    m.trigger();
    let b = render(&mut m, 1.0);
    assert!(peak(&b) > 0.4, "a second trigger plays again");

    // A pattern restarts its phrase and an adsr re-attacks on every trigger.
    let text = r#"[model]
one_shot = true
[graph]
nodes = [
  { id = "p", type = "pattern", rate = 20, count = 3, gap = 10, run = "t < 1" },
  { id = "e", type = "adsr", gate = "t < 0.3", attack = 0.005, release = 0.1 },
  { id = "s", type = "sine", freq = 600 },
  { id = "o", type = "mul", in = ["s"], by = ["e"] },
  { id = "c", type = "decay", in = ["p"], ms = 3 },
]
out = "c"
"#;
    let mut m = GraphModel::from_text(text, SR).unwrap();
    for _ in 0..2 {
        m.trigger();
        let mut x = vec![0.0];
        x.extend(render(&mut m, 0.5));
        let clicks = x.windows(2).filter(|w| w[1] > 0.5 && w[0] < 0.5).count();
        assert_eq!(clicks, 3, "a phrase of three per trigger");
    }
}

#[test]
fn bad_new_node_args_are_rejected() {
    let cases: &[(&str, &str)] = &[
        ("needs ratios", r#"{ id = "n", type = "noise" }, { id = "a", type = "modal", in = ["n"], freq = 100 }"#),
        ("t60 must be", r#"{ id = "n", type = "noise" }, { id = "a", type = "modal", in = ["n"], freq = 100, ratios = [1], t60 = [-1] }"#),
        ("needs channel", r#"{ id = "n", type = "noise" }, { id = "a", type = "pan", in = ["n"] }"#),
        ("euclid", r#"{ id = "a", type = "pattern", rate = 4, euclid = [9, 8] }"#),
        ("qs and gains go with freqs", r#"{ id = "n", type = "noise" }, { id = "a", type = "formant", in = ["n"], qs = [3] }"#),
        ("unknown routing", r#"{ id = "n", type = "noise" }, { id = "a", type = "formant", in = ["n"], routing = "diagonal" }"#),
        ("needs at least one input", r#"{ id = "a", type = "chirp", from = 1, to = 2 }"#),
        ("takes no `in`", r#"{ id = "n", type = "noise" }, { id = "a", type = "adsr", in = ["n"], gate = 1 }"#),
        ("missing required 'from'", r#"{ id = "n", type = "noise" }, { id = "a", type = "chirp", in = ["n"], to = 2 }"#),
    ];
    for (expect, nodes) in cases {
        let text = format!("[graph]\nnodes = [{nodes}]\nout = \"a\"\n");
        match GraphModel::from_text(&text, SR) {
            Ok(_) => panic!("accepted a bad file; expected \"{expect}\""),
            Err(e) => assert!(e.to_string().contains(expect), "expected \"{expect}\" in: {e}"),
        }
    }
    let text = "[graph]\nnodes = [{ id = \"a\", type = \"noise\" }]\nout = \"a\"\nout_right = \"b\"\n";
    assert!(GraphModel::from_text(text, SR).unwrap_err().to_string().contains("out_right = 'b'"));
}
