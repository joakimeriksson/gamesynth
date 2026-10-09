//! The living-nature family (`birds`, `insects`, `frogs`, `leaves`, `thunder`): each test encodes a
//! trait measured on CC0 field recordings (see `generators/wildlife.rs`).

use brusverk_core::generators;
use brusverk_core::render::rms;
use brusverk_core::Model;

const SR: f32 = 48000.0;

fn create(name: &str, preset: &str, inputs: &[(&str, f32)], params: &[(&str, f32)]) -> Box<dyn Model> {
    let mut m = generators::create(name, SR).unwrap();
    let i = m.desc().preset_index(preset).unwrap_or_else(|| panic!("{name} has no preset {preset}"));
    m.load_preset(i);
    for (k, v) in inputs {
        assert!(m.set_input_by_name(k, *v), "{name} has no input {k}");
    }
    for (k, v) in params {
        assert!(m.set_param_by_name(k, *v), "{name} has no param {k}");
    }
    m.snap();
    m
}

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    for c in out.chunks_mut(512) {
        m.render_mono(c);
    }
    out
}

/// RMS envelope in frames of `ms`, in dB.
fn envelope_db(x: &[f32], ms: f32) -> Vec<f32> {
    let n = (SR * ms / 1000.0) as usize;
    x.chunks_exact(n).map(|c| 20.0 * (rms(c) + 1e-9).log10()).collect()
}

/// Runs of `true` merged across gaps shorter than `bridge` frames: (start, length) in frames.
fn runs(active: &[bool], bridge: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < active.len() {
        if !active[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < active.len() && active[i] {
            i += 1;
        }
        match out.last_mut() {
            Some((s, len)) if start - (*s + *len) < bridge => *len = i - *s,
            _ => out.push((start, i - start)),
        }
    }
    out
}

/// Magnitude of the DFT of `x` (Hann window) at `hz`.
fn dft(x: &[f32], hz: f32) -> f32 {
    let n = x.len() as f32;
    let (mut re, mut im) = (0.0f32, 0.0f32);
    for (i, s) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n).cos();
        let ph = core::f32::consts::TAU * hz * i as f32 / SR;
        re += s * w * ph.cos();
        im -= s * w * ph.sin();
    }
    (re * re + im * im).sqrt()
}

/// Frequency of the strongest DFT bin between `lo` and `hi` (`step` Hz apart).
fn dominant(x: &[f32], lo: f32, hi: f32, step: f32) -> f32 {
    let mut best = (0.0, lo);
    let mut f = lo;
    while f <= hi {
        let m = dft(x, f);
        if m > best.0 {
            best = (m, f);
        }
        f += step;
    }
    best.1
}

/// Energy share above `hz`, through four cascaded one-pole high-passes (24 dB/octave).
fn share_above(x: &[f32], hz: f32) -> f32 {
    let c = 1.0 - (-core::f32::consts::TAU * hz / SR).exp();
    let mut lp = [0.0f32; 4];
    let (mut hi, mut all) = (0.0f64, 0.0f64);
    for s in x {
        let mut v = *s;
        for p in lp.iter_mut() {
            *p += (v - *p) * c;
            v -= *p;
        }
        hi += (v * v) as f64;
        all += (s * s) as f64;
    }
    (hi / all.max(1e-30)) as f32
}

// ---- insects ----

/// Chirps per second of a lone cricket: bursts of pulses with silence between chirps.
fn chirp_rate(x: &[f32]) -> (f32, f32) {
    let env = envelope_db(x, 5.0);
    let top = env.iter().cloned().fold(f32::MIN, f32::max);
    let active: Vec<bool> = env.iter().map(|e| *e > top - 30.0).collect();
    // Pulses inside a chirp are about 20 ms apart; chirps at least 150 ms.
    let chirps = runs(&active, 12);
    let pulses = runs(&active, 1).len() as f32 / chirps.len().max(1) as f32;
    (chirps.len() as f32 / (x.len() as f32 / SR), pulses)
}

#[test]
fn cricket_chirp_rate_follows_temperature_by_dolbears_law() {
    // Dolbear (1897): chirps per minute = 4 (F - 40), in Celsius 7.2 C - 32. The input maps
    // 0..1 to 10..35 C.
    let mut last = 0.0;
    for (t, c) in [(0.3, 17.5f32), (0.5, 22.5), (0.8, 30.0)] {
        let mut m = create("insects", "Lone cricket", &[("temperature", t), ("density", 1.0), ("daylight", 0.0)], &[]);
        render(m.as_mut(), 1.0);
        let x = render(m.as_mut(), 24.0);
        let (rate, pulses) = chirp_rate(&x);
        let dolbear = (7.2 * c - 32.0) / 60.0;
        println!("{c} C: {rate:.2} chirps/s (Dolbear {dolbear:.2}), {pulses:.1} pulses per chirp");
        assert!((rate / dolbear - 1.0).abs() < 0.12, "{c} C: {rate:.2} chirps/s, Dolbear says {dolbear:.2}");
        assert!(rate > last, "warmer must chirp faster: {rate:.2} after {last:.2}");
        assert!((2.5..=6.0).contains(&pulses), "a chirp is a short train of pulses, got {pulses:.1}");
        last = rate;
    }
    assert!(generators::wildlife::dolbear_hz(22.5) > 2.0 && generators::wildlife::dolbear_hz(22.5) < 2.3);
}

#[test]
fn crickets_sing_at_a_4_khz_carrier() {
    let mut m = create("insects", "Lone cricket", &[("temperature", 0.5)], &[]);
    let x = render(m.as_mut(), 6.0);
    // The loudest 50 ms is inside a chirp.
    let n = (0.05 * SR) as usize;
    let loud = x.chunks_exact(n).max_by(|a, b| rms(a).total_cmp(&rms(b))).unwrap();
    let f = dominant(loud, 2000.0, 9000.0, 25.0);
    assert!((3800.0..=4800.0).contains(&f), "carrier {f} Hz");
}

#[test]
fn a_field_of_crickets_drifts_out_of_step() {
    // Several crickets: their chirps do not all land together, so the field's envelope is busier
    // than one cricket's, and in step (sync 1) it is closer to one cricket again.
    let gaps = |sync: f32| {
        let mut m = create("insects", "Default", &[("temperature", 0.5), ("density", 1.0)], &[("chorus/level", 0.0), ("cricket/sync", sync), ("space/distance", 0.0)]);
        render(m.as_mut(), 4.0);
        let x = render(m.as_mut(), 16.0);
        let env = envelope_db(&x, 10.0);
        let top = env.iter().cloned().fold(f32::MIN, f32::max);
        env.iter().filter(|e| **e < top - 30.0).count() as f32 / env.len() as f32
    };
    let (adrift, in_step) = (gaps(0.0), gaps(1.0));
    println!("silent share: adrift {adrift:.2}, in step {in_step:.2}");
    assert!(in_step > adrift + 0.1, "synchronised crickets leave more silence ({in_step:.2}) than a drifting field ({adrift:.2})");
}

// ---- birds ----

#[test]
fn bird_song_is_phrases_of_tonal_syllables_with_gaps() {
    let mut m = create("birds", "Default", &[("activity", 0.35), ("time_of_day", 0.5)], &[]);
    render(m.as_mut(), 4.0);
    let x = render(m.as_mut(), 40.0);
    let env = envelope_db(&x, 10.0);
    let mut sorted = env.clone();
    sorted.sort_by(f32::total_cmp);
    let top = sorted[sorted.len() * 99 / 100];
    let active: Vec<bool> = env.iter().map(|e| *e > top - 25.0).collect();
    // Phrases: syllables less than 0.25 s apart.
    let phrases = runs(&active, 25);
    let in_phrase: usize = phrases.iter().map(|p| p.1).sum();
    let gap_share = 1.0 - in_phrase as f32 / env.len() as f32;
    let mut lens: Vec<f32> = phrases.iter().map(|p| p.1 as f32 * 0.01).collect();
    lens.sort_by(f32::total_cmp);
    let median = lens[lens.len() / 2];
    // Syllables: separate bursts inside the phrases, not one continuous sound.
    let syllables = runs(&active, 1).len();
    let syl_rate = syllables as f32 / (in_phrase as f32 * 0.01);
    println!("{} phrases, median {median:.2} s, gaps {:.0} %, {syl_rate:.1} syllables per second of song", phrases.len(), gap_share * 100.0);
    assert!(phrases.len() >= 6, "only {} phrases in 40 s", phrases.len());
    assert!((0.2..0.85).contains(&gap_share), "gaps take {:.0} % of the time", gap_share * 100.0);
    assert!((0.3..4.0).contains(&median), "median phrase {median:.2} s");
    assert!(syl_rate > 2.0, "phrases are made of syllables: {syl_rate:.1} per second");
    // Tonal, like whistles: in loud frames one frequency stands far above the rest.
    let n = 2048;
    let mut tonal = 0;
    let mut frames = 0;
    for c in x.chunks_exact(n).filter(|c| 20.0 * rms(c).log10() > top - 10.0).take(24) {
        let mags: Vec<f32> = (0..96).map(|k| dft(c, 1000.0 + 75.0 * k as f32)).collect();
        let mean = mags.iter().sum::<f32>() / mags.len() as f32;
        let max = mags.iter().cloned().fold(0.0, f32::max);
        frames += 1;
        if max > 6.0 * mean {
            tonal += 1;
        }
    }
    assert!(frames >= 8 && tonal * 3 >= frames * 2, "{tonal} of {frames} loud frames are tonal");
}

#[test]
fn birds_sing_in_the_dawn_chorus_and_owls_at_night() {
    let level = |t: f32| {
        let mut m = create("birds", "Default", &[("activity", 1.0), ("time_of_day", t)], &[]);
        rms(&render(m.as_mut(), 30.0)[(4.0 * SR) as usize..])
    };
    let (dawn, noon, night) = (level(0.25), level(0.5), level(0.0));
    println!("rms dawn {dawn:.4}, noon {noon:.4}, midnight {night:.4}");
    assert!(dawn > noon * 1.4, "the dawn chorus is the loudest time of day");
    assert!(night > 0.003, "night birds sing at night");
    let mut m = create("birds", "Default", &[("activity", 0.0), ("time_of_day", 0.25)], &[]);
    render(m.as_mut(), 4.0);
    assert!(rms(&render(m.as_mut(), 6.0)) < 1e-3, "activity 0 is a silent forest");
}

// ---- frogs ----

#[test]
fn tree_frogs_call_through_a_resonant_throat_in_bouts() {
    let mut m = create("frogs", "Tree frogs", &[("activity", 0.6)], &[]);
    render(m.as_mut(), 3.0);
    let x = render(m.as_mut(), 30.0);
    // The throat: energy peaks near 2 kHz, little of it above 4 kHz.
    let n = (0.1 * SR) as usize;
    let loud: Vec<&[f32]> = x.chunks_exact(n).filter(|c| rms(c) > rms(&x)).take(10).collect();
    for c in &loud {
        let f = dominant(c, 400.0, 6000.0, 50.0);
        assert!((1700.0..2800.0).contains(&f) || (850.0..1250.0).contains(&f), "formant at {f} Hz");
    }
    assert!(share_above(&x, 4500.0) < 0.1, "a frog is not a hiss: {:.2} above 4.5 kHz", share_above(&x, 4500.0));
    // Calls with gaps between them: the envelope drops well below the calls most seconds.
    let env = envelope_db(&x, 10.0);
    let top = env.iter().cloned().fold(f32::MIN, f32::max);
    let quiet = env.iter().filter(|e| **e < top - 20.0).count() as f32 / env.len() as f32;
    let calls = runs(&env.iter().map(|e| *e > top - 20.0).collect::<Vec<_>>(), 3).len() as f32 / 30.0;
    println!("{calls:.2} calls per second, {:.0} % quiet", quiet * 100.0);
    assert!(quiet > 0.15 && (0.5..6.0).contains(&calls), "calls {calls:.2}/s, quiet {quiet:.2}");
}

// ---- leaves ----

#[test]
fn leaves_rustle_with_the_wind_and_its_gusts() {
    let take = |strength: f32, gustiness: f32| {
        let mut m = create("leaves", "Default", &[("strength", strength), ("gustiness", gustiness)], &[]);
        render(m.as_mut(), 2.0);
        render(m.as_mut(), 30.0)
    };
    let (still, light, strong) = (take(0.05, 0.0), take(0.3, 0.5), take(0.8, 0.5));
    assert!(rms(&still) < 0.01 * rms(&strong), "still air moves no leaves");
    assert!(rms(&strong) > 2.0 * rms(&light), "stronger wind rustles louder");
    // Gusts: the 250 ms level swings by several dB.
    let gusty = take(0.5, 1.0);
    let mut e = envelope_db(&gusty, 250.0);
    e.sort_by(f32::total_cmp);
    let swell = e[e.len() * 95 / 100] - e[e.len() / 20];
    assert!(swell > 6.0, "gusts swell the rustle by only {swell:.1} dB");
    // Recorded wind in trees has 2-29 % of its energy above 2.5 kHz: a rustle, not a hiss.
    let hi = share_above(&strong, 2500.0);
    assert!(hi < 0.3, "{:.0} % above 2.5 kHz", hi * 100.0);
}

// ---- thunder ----

fn strike(distance: f32, secs: f32) -> Vec<f32> {
    let mut m = create("thunder", "Default", &[("power", 1.0), ("distance", distance)], &[]);
    m.trigger();
    render(m.as_mut(), secs)
}

#[test]
fn close_thunder_cracks_and_far_thunder_rolls_in() {
    let (near, far) = (strike(0.0, 4.0), strike(1.0, 4.0));
    // Attack: from the first sound (60 dB under the loudest 5 ms of the first 3 s) to within
    // 12 dB of it. Recorded close strikes go from silence to near full level in a few ms; far
    // ones swell for a second or more.
    let rise = |x: &[f32]| {
        let e = envelope_db(&x[..(3.0 * SR) as usize], 5.0);
        let top = e.iter().cloned().fold(f32::MIN, f32::max);
        let first = e.iter().position(|v| *v > top - 60.0).unwrap();
        (e.iter().position(|v| *v > top - 12.0).unwrap() - first) as f32 * 0.005
    };
    let (rn, rf) = (rise(&near), rise(&far));
    // The crack: energy above 2 kHz in the first 300 ms.
    let first = (0.3 * SR) as usize;
    let (hn, hf) = (share_above(&near[..first], 2000.0), share_above(&far[..first], 2000.0));
    println!("rise to -12 dB: near {:.0} ms, far {:.0} ms; above 2 kHz in the first 300 ms: near {:.1} %, far {:.3} %", rn * 1000.0, rf * 1000.0, hn * 100.0, hf * 100.0);
    assert!(rn <= 0.02, "a close strike hits at once, not after {:.0} ms", rn * 1000.0);
    assert!(rf > 0.4, "far thunder swells in, but it peaked after {:.0} ms", rf * 1000.0);
    assert!(hn > 0.003 && hf < hn * 0.05, "the crack is a close strike's alone: near {hn:.4}, far {hf:.5}");
}

#[test]
fn thunder_rolls_on_and_every_strike_differs() {
    let mut m = create("thunder", "Default", &[("power", 1.0), ("distance", 0.3)], &[]);
    let mut strikes = Vec::new();
    for _ in 0..2 {
        m.trigger();
        strikes.push(envelope_db(&render(m.as_mut(), 12.0), 100.0));
    }
    let (a, b) = (&strikes[0], &strikes[1]);
    // It rolls on for seconds after the first arrival.
    let top = a.iter().cloned().fold(f32::MIN, f32::max);
    assert!(a[80] > top - 40.0, "still rolling 8 s later: {:.1} dB under the peak", top - a[80]);
    // The lumps land at different times on every strike.
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let (ma, mb) = (mean(a), mean(b));
    let cov: f32 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let var = |v: &[f32], m: f32| v.iter().map(|x| (x - m) * (x - m)).sum::<f32>();
    let corr = cov / (var(a, ma) * var(b, mb)).sqrt();
    let diff = a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32;
    println!("two strikes: envelope correlation {corr:.2}, mean difference {diff:.1} dB");
    assert!(diff > 1.5, "two strikes roll the same way (mean difference {diff:.1} dB)");
}

#[test]
fn thunder_peaks_sit_with_explosions() {
    // Event levels: overhead about as loud as a point-blank explosion, far off well under it.
    let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let near = peak(&strike(0.0, 6.0));
    let far = peak(&strike(1.0, 12.0));
    println!("thunder peaks: overhead {near:.2}, 15 km {far:.2}");
    assert!((0.4..=1.0).contains(&near), "overhead peak {near:.2}");
    assert!(far < near * 0.7 && far > 0.05, "far peak {far:.2}");
}

// ---- the family ----

#[test]
fn width_zero_is_the_mono_sound() {
    for name in ["birds", "insects", "frogs", "leaves", "thunder"] {
        let (mut a, mut b) = (generators::create(name, SR).unwrap(), generators::create(name, SR).unwrap());
        let w = a.desc().param_index("space/width").unwrap();
        a.set_param(w, 0.0);
        for m in [&mut a, &mut b] {
            for i in 0..m.desc().inputs.len() {
                m.set_input(i, 0.9);
            }
            m.set_input(1, 0.25);
            m.trigger();
        }
        let n = (3.0 * SR) as usize;
        let (mut l, mut r, mut mono) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        a.render_stereo(&mut l, &mut r);
        b.render_mono(&mut mono);
        assert!(rms(&mono) > 1e-4, "{name} silent");
        assert!(l == r && l == mono, "{name}: width 0 must be the mono render");
    }
}

#[test]
fn ambiences_are_stereo_and_cheap() {
    // `wind` and `rain` are printed for comparison; thunder is timed while it sounds.
    for name in ["birds", "insects", "frogs", "leaves", "thunder", "wind", "rain"] {
        let mut m = create(name, "Default", &[], &[]);
        for i in 0..m.desc().inputs.len() {
            m.set_input(i, 0.8);
        }
        if name == "birds" {
            m.set_input(1, 0.25);
        }
        if name == "insects" {
            m.set_input(2, 0.5);
        }
        m.snap();
        if name == "thunder" {
            m.set_input(1, 0.0);
            m.trigger();
        }
        let n = (10.0 * SR) as usize;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        let t = std::time::Instant::now();
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            m.render_stereo(a, b);
        }
        let speed = 10.0 / t.elapsed().as_secs_f32();
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x * y) as f64).sum::<f64>();
        let corr = dot(&l, &r) / (dot(&l, &l) * dot(&r, &r)).sqrt().max(1e-30);
        println!("{name}: {speed:.0}x real time (stereo, busy), L/R correlation {corr:.2}");
        if !matches!(name, "wind" | "rain") {
            assert!(corr < 0.95, "{name} has no stereo image: correlation {corr:.2}");
            assert!(speed > 10.0, "{name} renders at only {speed:.1}x real time");
        }
    }
}
