//! Water interaction: what each sound must keep from the recordings it was tuned against.
//!
//! References (CC0, Freesound; measured with octave bands, envelopes, ping tracking and
//! resonance tracking before anything was built):
//! * splashes: jc144940 178732/178735/178736 (rock plops), Kraftaggregat 737644 (rocks thrown),
//!   decadylan 825807, Fission9 583348, qubodup 442773 (big splash), kyles 637823 (body dive),
//!   lurpsis 444015, Urkki69 628350, bruno.auzet 529794;
//! * swimming: tbsounddesigns 530158, dawidwmika 372518, craigsmith 438844, tom_woysky 240115;
//! * boats: kyles 637206 and 637980 (hull slaps), craigsmith 438846 (oars), brunoboselli 249707,
//!   Fenodyrie 588308, bruno.auzet 570926 and 4billboards 512126 (wakes), martian 234361;
//! * under water: Fission9 504641, wjoojoo 197751, Kinoton 393819, Tim_Verberne 482167,
//!   felix.blume 384218;
//! * drips: nobarknoonan 465608, CHallSmith 870869, jarfil 151186, thesfxcompany 612273;
//! * pouring: grakshay 614205 (a bottle from a tap), asagatov 323003, ahamirikia 710550,
//!   Alex_hears_things 316686.
use brusverk_core::dsp::{minnaert_hz, Bubbles};
use brusverk_core::generators::{self, water};
use brusverk_core::render::{peak, rms};
use brusverk_core::Model;

const SR: f32 = 48000.0;
const NAMES: [&str; 6] = ["splash", "swim", "boat", "underwater", "drip", "pour"];

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

fn gen(name: &str) -> Box<dyn Model> {
    generators::create(name, SR).unwrap()
}

fn set(m: &mut dyn Model, param: &str, v: f32) {
    assert!(m.set_param_by_name(param, v), "{} has no param {param}", m.desc().name);
}

fn input(m: &mut dyn Model, name: &str, v: f32) {
    assert!(m.set_input_by_name(name, v), "{} has no input {name}", m.desc().name);
}

/// Power spectrum (radix-2 FFT of a Hann-windowed, zero-padded frame) and its bin width.
fn spectrum(x: &[f32]) -> (Vec<f32>, f32) {
    let n = x.len().next_power_of_two().max(1024);
    let mut re: Vec<f32> = (0..n)
        .map(|i| if i < x.len() { x[i] * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / x.len() as f32).cos()) } else { 0.0 })
        .collect();
    let mut im = vec![0.0f32; n];
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
        let w = -std::f32::consts::TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (c, s) = ((w * k as f32).cos(), (w * k as f32).sin());
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
    ((0..n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect(), SR / n as f32)
}

/// The strongest frequency between `lo` and `hi` Hz.
fn dominant(x: &[f32], lo: f32, hi: f32) -> f32 {
    let (p, df) = spectrum(x);
    let (a, b) = ((lo / df) as usize, ((hi / df) as usize).min(p.len() - 1));
    let k = (a..=b).max_by(|&i, &j| p[i].total_cmp(&p[j])).unwrap();
    k as f32 * df
}

fn centroid(x: &[f32]) -> f32 {
    let (p, df) = spectrum(x);
    let lo = (30.0 / df) as usize;
    let (num, den) = p.iter().enumerate().skip(lo).fold((0.0, 0.0), |(n, d), (k, &v)| (n + k as f32 * df * v, d + v));
    num / den.max(1e-30)
}

/// Energy-weighted mean of log2(frequency) above 30 Hz: where a sound sits, in octaves.
fn log_centroid(x: &[f32]) -> f32 {
    let (p, df) = spectrum(x);
    let lo = (30.0 / df) as usize;
    let (num, den) = p.iter().enumerate().skip(lo).fold((0.0, 0.0), |(n, d), (k, &v)| (n + (k as f32 * df).log2() * v, d + v));
    (num / den.max(1e-30)).exp2()
}

/// The first `secs` of a sound, from where it first comes within 26 dB of its peak.
fn attack(x: &[f32], secs: f32) -> &[f32] {
    let start = x.iter().position(|s| s.abs() > 0.05 * peak(x)).unwrap();
    &x[start..(start + (secs * SR) as usize).min(x.len())]
}

/// Where the deep part of a sound sits: the energy-weighted mean of log2(frequency) between
/// 30 Hz and 1 kHz.
fn low_peak(x: &[f32]) -> f32 {
    let (p, df) = spectrum(x);
    let (lo, hi) = ((30.0 / df) as usize, (1000.0 / df) as usize);
    let (num, den) = (lo..hi).fold((0.0, 0.0), |(n, d), k| (n + (k as f32 * df).log2() * p[k], d + p[k]));
    (num / den.max(1e-30)).exp2()
}

/// Onsets per second of the strokes' sharp entries: the signal differentiated twice (+12 dB per
/// octave), so the low pull and kick do not count.
fn entries_per_sec(x: &[f32]) -> f32 {
    let d: Vec<f32> = x.windows(3).map(|w| w[2] - 2.0 * w[1] + w[0]).collect();
    events_per_sec(&d, 0.3)
}

/// 10 ms RMS envelope in dB.
fn envelope_db(x: &[f32]) -> Vec<f32> {
    x.chunks((0.01 * SR) as usize).map(|c| 20.0 * (rms(c) + 1e-9).log10()).collect()
}

/// Seconds from the peak of the 5 ms envelope until it falls `db` below it and stays within
/// 3 dB of that for 50 ms (as the recordings were measured: drops falling back later do not
/// hold the sound open).
fn ring_secs(x: &[f32], db: f32) -> f32 {
    let e: Vec<f32> = x.chunks((0.005 * SR) as usize).map(|c| 20.0 * (rms(c) + 1e-9).log10()).collect();
    let (pk_i, pk) = e.iter().enumerate().fold((0, f32::MIN), |a, (i, &v)| if v > a.1 { (i, v) } else { a });
    let end = (pk_i..e.len()).find(|&i| e[i] < pk - db && e[i..(i + 10).min(e.len())].iter().all(|&v| v < pk - db + 3.0)).unwrap_or(e.len());
    (end - pk_i) as f32 * 0.005
}

fn splash(size: f32, power: f32, flat: f32, tweak: &[(&str, f32)]) -> Vec<f32> {
    let mut m = gen("splash");
    set(m.as_mut(), "shape/variation", 0.0);
    for (k, v) in tweak {
        set(m.as_mut(), k, *v);
    }
    input(m.as_mut(), "size", size);
    input(m.as_mut(), "power", power);
    input(m.as_mut(), "flat", flat);
    m.trigger();
    let len = m.length_secs().unwrap();
    render(m.as_mut(), len)
}

#[test]
fn a_bigger_splash_is_lower_and_longer() {
    // Measured: a stone falls 20 dB in 15-230 ms after its plop and sits low; a body or big
    // object takes 1.4-1.6 s to fall 20 dB.
    let sizes = [0.05, 0.35, 0.6, 1.0];
    for _ in 0..3 {
        // Three rounds: every instance rolls its own random stream.
        bigger_splash_round(&sizes);
    }
}

fn bigger_splash_round(sizes: &[f32]) {
    let runs: Vec<Vec<f32>> = sizes.iter().map(|&s| splash(s, 1.0, 0.2, &[])).collect();
    // Lower: the deep part of the splash (the cavity's bubble, the thump of the water) is lower
    // for a bigger object: 190-280 Hz for thrown stones, about 100-250 Hz for a body or a big
    // rock. Its spray on top is broad (and brighter overall than a stone's plop, as recorded).
    let lows: Vec<f32> = runs.iter().map(|x| low_peak(attack(x, 0.3))).collect();
    let rings: Vec<f32> = runs.iter().map(|x| ring_secs(x, 20.0)).collect();
    for k in 1..sizes.len() {
        assert!(lows[k] < lows[k - 1], "size {} sits at {:.0} Hz, not below size {} at {:.0} Hz", sizes[k], lows[k], sizes[k - 1], lows[k - 1]);
        assert!(rings[k] >= rings[k - 1], "size {} rings {:.2} s, not longer than size {} ({:.2} s)", sizes[k], rings[k], sizes[k - 1], rings[k - 1]);
    }
    assert!(rings[1] < 0.3, "a stone should be short: {:.2} s to fall 20 dB", rings[1]);
    assert!(rings[3] > 1.0, "a big splash should be long: {:.2} s to fall 20 dB", rings[3]);
    // A big splash is broad: the octaves from 125 Hz to 4 kHz within a few dB, like the
    // recordings (centroid 1.7-2.6 kHz), not a dark thud nor a hiss.
    let c = centroid(&runs[3]);
    assert!((1000.0..3000.0).contains(&c), "big splash centroid {c:.0} Hz");
}

#[test]
fn the_plop_rings_at_the_cavity_bubbles_minnaert_pitch() {
    // Only the cavity's bubble: its pitch is the Minnaert pitch of a bubble whose size grows
    // with the object (190-280 Hz measured for thrown stones).
    for size in [0.1, 0.35, 0.6] {
        let x = splash(size, 0.8, 0.0, &[("crack/level", 0.0), ("body/level", 0.0), ("bubbles/level", 0.0), ("droplets/level", 0.0)]);
        let want = water::bloop_hz(size, 0.8);
        let got = dominant(&x[..(0.4 * SR) as usize], 60.0, 4000.0);
        assert!((got / want).log2().abs() < 0.25, "size {size}: bloop at {got:.0} Hz, expected about {want:.0}");
    }
    let stone = water::bloop_hz(0.35, 0.8);
    assert!((180.0..330.0).contains(&stone), "a thrown stone's plop at {stone:.0} Hz");
}

#[test]
fn bubble_pings_sit_in_the_minnaert_range_for_their_size() {
    // The cloud of small bubbles left by a splash: its energy lies between the Minnaert pitches
    // of the largest and smallest radii it releases.
    for size in [0.2, 0.8] {
        let x = splash(size, 1.0, 0.2, &[("crack/level", 0.0), ("bloop/level", 0.0), ("body/level", 0.0), ("droplets/level", 0.0)]);
        let (r_lo, r_hi) = water::splash_bubble_radii(size);
        let (f_lo, f_hi) = (minnaert_hz(r_hi), minnaert_hz(r_lo));
        let c = log_centroid(&x);
        assert!(c > f_lo && c < f_hi, "size {size}: pings centred at {c:.0} Hz, outside {f_lo:.0}..{f_hi:.0} Hz");
    }
    // A drip into water: one ping at the Minnaert pitch of its bubble (measured: 1.1-1.7 kHz,
    // 20-100 ms, rising about a tenth of an octave).
    for (mm, size) in [(1.5, 0.5), (2.4, 0.2), (2.4, 0.5), (2.4, 0.9), (5.0, 0.5)] {
        let mut m = gen("drip");
        set(m.as_mut(), "drops/bubble_mm", mm);
        set(m.as_mut(), "drops/bubble_chance", 1.0);
        set(m.as_mut(), "drops/rebound", 0.0);
        set(m.as_mut(), "drops/click", 0.0);
        input(m.as_mut(), "rate", 0.0);
        input(m.as_mut(), "size", size);
        input(m.as_mut(), "pool", 1.0);
        m.trigger();
        let x = render(m.as_mut(), 0.15);
        let want = minnaert_hz(water::drip_radius(mm, size));
        let got = dominant(&x[(0.003 * SR) as usize..(0.06 * SR) as usize], 100.0, 12000.0);
        assert!((got / want).log2().abs() < 0.2, "drip {mm} mm at size {size}: ping at {got:.0} Hz, Minnaert says {want:.0}");
    }
}

#[test]
fn bubbles_ring_short_and_rise_only_a_little() {
    // The shared block: a 3 mm bubble sings at 1087 Hz, has rung out (60 dB) within about
    // 25 ms, and its pitch rises by `rise` over that time, not a long zip.
    let mut b = Bubbles::new();
    b.spawn(minnaert_hz(3.0), 1.0, 0.03, 1.0, SR);
    let x: Vec<f32> = (0..(0.06 * SR) as usize)
        .map(|i| {
            let y = b.tick();
            if i % 32 == 31 {
                b.end_block(1e-6);
            }
            y
        })
        .collect();
    let crossings = |s: &[f32]| s.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f32 / 2.0 / (s.len() as f32 / SR);
    let early = crossings(&x[..(0.006 * SR) as usize]);
    assert!((early / 1087.0 - 1.0).abs() < 0.05, "3 mm bubble at {early:.0} Hz");
    let late = crossings(&x[(0.012 * SR) as usize..(0.018 * SR) as usize]);
    assert!(late > early && late < early * 1.06, "pitch went {early:.0} -> {late:.0} Hz");
    assert!(peak(&x[(0.03 * SR) as usize..]) < 1e-3, "a 1 kHz bubble should have rung out by 30 ms");
    assert_eq!(b.active(), 0);
}

#[test]
fn a_pour_rises_in_pitch_as_it_fills() {
    // Measured on a bottle filled from a tap: 430 Hz empty, about 590 half full, 800 at three
    // quarters, 1.9 kHz nearly full; the resonance stands 15-30 dB over the splash noise.
    let fills = [0.1, 0.4, 0.7, 0.9];
    let mut last = 0.0;
    for fill in fills {
        let mut m = gen("pour");
        input(m.as_mut(), "flow", 0.7);
        input(m.as_mut(), "fill", fill);
        m.snap();
        let x = render(m.as_mut(), 1.5);
        let got = dominant(&x[(0.5 * SR) as usize..], 200.0, 3000.0);
        let want = water::pour_hz(430.0, 4.5, 1.0, fill);
        assert!((got / want).log2().abs() < 0.12, "fill {fill}: rings at {got:.0} Hz, expected {want:.0}");
        assert!(got > last * 1.1, "fill {fill}: {got:.0} Hz is not higher than {last:.0}");
        last = got;
    }
    assert!((water::pour_hz(430.0, 4.5, 1.0, 0.5) - 590.0).abs() < 30.0);
    assert!((water::pour_hz(430.0, 4.5, 1.0, 0.75) - 800.0).abs() < 60.0);
    // No flow, no sound.
    let mut m = gen("pour");
    input(m.as_mut(), "flow", 0.0);
    m.snap();
    assert!(peak(&render(m.as_mut(), 0.5)) < 1e-3);
}

/// Events per second: onsets that rise 12 dB over the running floor to within 18 dB of the
/// loudest moment, at least `gap` s apart (one wave slaps the hull two or three times within
/// half a second, which counts once; the faint bubbles after it do not count).
fn events_per_sec(x: &[f32], gap: f32) -> f32 {
    let e = envelope_db(x);
    let top = e.iter().cloned().fold(f32::MIN, f32::max);
    let mut count = 0;
    let mut last = -100i64;
    for i in 1..e.len() {
        let floor = e[i.saturating_sub(30)..i].iter().cloned().fold(f32::MAX, f32::min);
        if e[i] > floor + 12.0 && e[i] > top - 18.0 && e[i] - e[i - 1] > 6.0 && i as i64 - last >= (gap * 100.0) as i64 {
            count += 1;
            last = i as i64;
        }
    }
    count as f32 / (x.len() as f32 / SR)
}

#[test]
fn hull_slaps_follow_the_wave_rate() {
    let slaps = |rate: f32, speed: f32| {
        let mut m = gen("boat");
        for (k, v) in [("slap/waves_per_second", rate), ("wash/level", 0.0), ("lap/level", 0.0), ("creak/level", 0.0), ("oars/level", 0.0)] {
            set(m.as_mut(), k, v);
        }
        input(m.as_mut(), "speed", speed);
        input(m.as_mut(), "waves", 1.0);
        input(m.as_mut(), "row", 0.0);
        m.snap();
        events_per_sec(&render(m.as_mut(), 40.0), 0.55)
    };
    let (slow, fast) = (slaps(0.4, 0.0), slaps(1.2, 0.0));
    assert!((slow / 0.4 - 1.0).abs() < 0.35, "0.4 waves/s heard as {slow:.2}/s");
    assert!((fast / 1.2 - 1.0).abs() < 0.35, "1.2 waves/s heard as {fast:.2}/s");
    // Under way, the boat meets the waves more often.
    let moving = slaps(0.4, 1.0);
    assert!(moving > slow * 1.8, "at full speed {moving:.2}/s vs {slow:.2}/s at rest");
    // Calm water: no slaps.
    let mut m = gen("boat");
    set(m.as_mut(), "wash/level", 0.0);
    set(m.as_mut(), "lap/level", 0.0);
    input(m.as_mut(), "waves", 0.0);
    m.snap();
    assert!(peak(&render(m.as_mut(), 6.0)) < 1e-3);
}

#[test]
fn hull_slaps_are_low_and_the_wake_rises_with_speed() {
    // Measured slaps: centroid 450-650 Hz; the wake centred near 1-1.3 kHz.
    let mut m = gen("boat");
    set(m.as_mut(), "wash/level", 0.0);
    set(m.as_mut(), "creak/level", 0.0);
    input(m.as_mut(), "waves", 0.8);
    m.snap();
    let c = centroid(&render(m.as_mut(), 20.0));
    assert!((300.0..900.0).contains(&c), "hull slaps centred at {c:.0} Hz");
    let wake = |speed: f32| {
        let mut m = gen("boat");
        input(m.as_mut(), "waves", 0.0);
        input(m.as_mut(), "speed", speed);
        set(m.as_mut(), "lap/level", 0.0);
        m.snap();
        render(m.as_mut(), 6.0)
    };
    let (slow, fast) = (wake(0.2), wake(0.9));
    assert!(rms(&fast) > rms(&slow) * 3.0, "wake {} at speed 0.9 vs {} at 0.2", rms(&fast), rms(&slow));
    let c = centroid(&fast);
    assert!((700.0..2000.0).contains(&c), "wake centred at {c:.0} Hz");
}

#[test]
fn submerged_is_darker() {
    // Recordings under water: centroid 110-240 Hz and 15-40 dB down at 1 kHz.
    let at = |submerge: f32| {
        let mut m = gen("underwater");
        input(m.as_mut(), "submerge", submerge);
        m.snap();
        render(m.as_mut(), 6.0)
    };
    let (above, under) = (at(0.0), at(1.0));
    let (ca, cu) = (centroid(&above), centroid(&under));
    assert!(cu < 400.0, "under water centred at {cu:.0} Hz");
    assert!(ca > cu * 4.0, "above the surface ({ca:.0} Hz) should be far brighter than under it ({cu:.0} Hz)");
    // Going under plunges with a rush; coming up breaks out: both are heard.
    let mut m = gen("underwater");
    input(m.as_mut(), "submerge", 0.0);
    input(m.as_mut(), "breath", 0.0);
    m.snap();
    let before = rms(&render(m.as_mut(), 1.0));
    input(m.as_mut(), "submerge", 1.0);
    let plunge = rms(&render(m.as_mut(), 0.5));
    assert!(plunge > before * 2.0, "no plunge: {plunge} vs {before}");

    // Dunking any other sound: the host-side Muffle.
    let mut noise = brusverk_core::noise::Noise::new(7);
    let white: Vec<f32> = (0..SR as usize).map(|_| noise.white() * 0.5).collect();
    let dunk = |submerge: f32| {
        let mut f = water::Muffle::new(SR);
        let mut x = white.clone();
        for c in x.chunks_mut(512) {
            f.process(c, submerge);
        }
        x
    };
    let (dry, wet) = (dunk(0.0), dunk(1.0));
    assert!(centroid(&wet) < 600.0 && centroid(&dry) > 5000.0, "muffle: {:.0} Hz under, {:.0} Hz above", centroid(&wet), centroid(&dry));
    assert!(water::muffle_hz(0.5) < water::muffle_hz(0.3) && water::muffle_hz(0.7) < water::muffle_hz(0.5));
}

#[test]
fn swimming_strokes_follow_pace_and_effort() {
    let swim = |pace: f32, effort: f32, tread: f32| {
        let mut m = gen("swim");
        input(m.as_mut(), "pace", pace);
        input(m.as_mut(), "effort", effort);
        input(m.as_mut(), "tread", tread);
        m.snap();
        render(m.as_mut(), 20.0)
    };
    // Still, not triggered: silence.
    assert!(peak(&swim(0.0, 0.5, 0.0)) < 1e-4);
    let (easy, hard) = (swim(0.5, 0.2, 0.0), swim(0.5, 1.0, 0.0));
    assert!(rms(&hard) > rms(&easy) * 1.5, "effort: {} vs {}", rms(&hard), rms(&easy));
    // Each arm goes in twice a second at pace 1, 0.6 times at 0.3 (counted with the kick and the
    // drips off, which follow the pace too).
    let entries = |pace: f32| {
        let mut m = gen("swim");
        set(m.as_mut(), "kick/level", 0.0);
        set(m.as_mut(), "stroke/drips", 0.0);
        input(m.as_mut(), "pace", pace);
        m.snap();
        entries_per_sec(&render(m.as_mut(), 20.0))
    };
    let (fast, slow) = (entries(1.0), entries(0.3));
    assert!((fast / 2.0 - 1.0).abs() < 0.25, "pace 1: {fast:.2} hand entries per second, expected 2");
    assert!((slow / 0.6 - 1.0).abs() < 0.3, "pace 0.3: {slow:.2} hand entries per second, expected 0.6");
    // Measured crawl: most energy 250 Hz - 4 kHz, centroid 1.3-2.1 kHz.
    let c = centroid(&swim(0.6, 0.6, 0.0));
    assert!((900.0..3000.0).contains(&c), "crawl centred at {c:.0} Hz");
    // Treading water is quieter than swimming, but there.
    let tread = swim(0.0, 0.5, 1.0);
    assert!(rms(&tread) > 1e-3 && rms(&tread) < rms(&swim(0.6, 0.5, 0.0)));
    // trigger() makes one stroke.
    let mut m = gen("swim");
    m.trigger();
    assert!(peak(&render(m.as_mut(), 1.0)) > 0.05);
}

#[test]
fn drips_follow_their_rate() {
    let count = |rate: f32| {
        let mut m = gen("drip");
        set(m.as_mut(), "drops/rebound", 0.0);
        set(m.as_mut(), "drops/regularity", 1.0);
        set(m.as_mut(), "drops/bubble_chance", 1.0);
        input(m.as_mut(), "rate", rate);
        m.snap();
        events_per_sec(&render(m.as_mut(), 20.0), 0.1)
    };
    let want = water::drip_rate(0.3);
    let got = count(0.3);
    assert!((got / want - 1.0).abs() < 0.25, "rate 0.3: {got:.2} drips/s, expected {want:.2}");
    let mut m = gen("drip");
    input(m.as_mut(), "rate", 0.0);
    m.snap();
    assert!(peak(&render(m.as_mut(), 2.0)) < 1e-5, "rate 0 must wait for trigger()");
    m.trigger();
    assert!(peak(&render(m.as_mut(), 0.3)) > 0.05);
}

#[test]
fn everything_is_bounded() {
    for name in NAMES {
        let mut m = gen(name);
        let d = m.desc().clone();
        assert_eq!(d.category, "water");
        for preset in 0..d.presets.len() {
            m.load_preset(preset);
            let label = format!("{name}/{}", d.presets[preset].name);
            // Every input at its corners, then thrashed every 20 ms.
            for corner in [0.0, 1.0] {
                for i in 0..d.inputs.len() {
                    m.set_input(i, corner);
                }
                m.snap();
                m.trigger();
                let x = render(m.as_mut(), 2.0);
                assert!(x.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{label} at {corner}: peak {}", peak(&x));
            }
            let mut rng = brusverk_core::math::Rng::new(5);
            for _ in 0..100 {
                for i in 0..d.inputs.len() {
                    m.set_input(i, rng.next_f32());
                }
                if rng.next_f32() < 0.3 {
                    m.trigger();
                }
                let x = render(m.as_mut(), 0.02);
                assert!(x.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{label} thrashed");
            }
            // Many triggers on top of each other stay bounded and the one-shot still ends.
            for _ in 0..20 {
                m.trigger();
                render(m.as_mut(), 0.01);
            }
            if d.one_shot {
                let len = m.length_secs().unwrap();
                let x = render(m.as_mut(), len);
                assert!(x.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
                assert!(m.is_finished(), "{label} still sounding after {len:.1} s");
            }
        }
    }
}

#[test]
fn levels_sit_with_the_other_water_sounds() {
    // Loudness is checked by LUFS outside the tests (see the module docs of `water`); here the
    // peaks: a full-power splash peaks like mud_splash and rock_hit, never pinned on the limiter.
    for size in [0.0, 0.35, 0.6, 1.0] {
        let x = splash(size, 1.0, 0.3, &[]);
        let p = peak(&x);
        assert!((0.4..0.98).contains(&p), "splash size {size} peaks at {p:.2}");
    }
    let mut m = gen("underwater");
    m.snap();
    let r = rms(&render(m.as_mut(), 6.0));
    let mut s = gen("stream");
    s.snap();
    let rs = rms(&render(s.as_mut(), 6.0));
    assert!((r / rs).log10().abs() * 20.0 < 9.0, "underwater rms {r:.3} vs stream {rs:.3}");
}

#[test]
fn model_files_can_make_water_with_the_bubbles_node() {
    let toml = r#"
[model]
name = "brook"
[inputs]
flow = { default = 0.6 }
[graph]
nodes = [
  { id = "drops", type = "powerdust", rate = "lerp(20, 600, flow)", skew = 2 },
  { id = "brook", type = "bubbles", in = ["drops"], radius_mm = 3, spread = 0.3 },
  { id = "out", type = "gain", in = ["brook"], gain = 0.5 },
]
out = "out"
"#;
    let mut m = brusverk_core::graph::GraphModel::from_text(toml, SR).unwrap();
    let x = render(&mut m, 3.0);
    assert!(x.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    assert!(rms(&x) > 1e-3);
    let c = log_centroid(&x);
    let f = minnaert_hz(3.0);
    assert!((c / f).log2().abs() < 0.35, "bubbles of 3 mm centred at {c:.0} Hz, Minnaert {f:.0}");
}
