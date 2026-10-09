//! The crowd's presets, tuned against recordings (see tools/reference/crowd and hockey).

use brusverk_core::generators;
use brusverk_core::Model;

const SR: f32 = 48000.0;

fn crowd(preset: &str, size: f32, excitement: f32) -> Box<dyn Model> {
    let mut m = generators::create("crowd", SR).unwrap();
    let k = m.desc().preset_index(preset).unwrap_or_else(|| panic!("no preset {preset}"));
    m.load_preset(k);
    m.set_input_by_name("size", size);
    m.set_input_by_name("excitement", excitement);
    m.snap();
    m
}

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

/// FNV-1a over the exact bits of every sample.
fn fingerprint(hash: &mut u64, x: &[f32]) {
    for v in x {
        for b in v.to_bits().to_le_bytes() {
            *hash ^= b as u64;
            *hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    }
}

#[test]
fn arena_is_bit_identical_to_the_fitted_version() {
    // The Arena preset was fitted to hockey recordings and the hockey game ships with it: it must
    // not change by a single bit. This drives it through calm, a groan, booing, the chant and a
    // goal (as the hockey game does), in mono and stereo. The number is from commit c7b396e.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut m = crowd("Arena", 0.9, 0.12);
    let steps: [(&str, f32, f32); 12] = [
        ("excitement", 0.12, 2.0),
        ("groan", 1.0, 0.5),
        ("groan", 0.0, 2.0),
        ("boo", 1.0, 3.0),
        ("boo", 0.0, 1.5),
        ("chant", 1.0, 4.0),
        ("chant", 0.0, 0.5),
        ("excitement", 0.42, 2.0),
        ("goal", 1.0, 3.0),
        ("goal", 0.0, 3.0),
        ("excitement", 1.0, 2.0),
        ("size", 0.3, 2.0),
    ];
    for (input, v, secs) in steps {
        m.set_input_by_name(input, v);
        fingerprint(&mut hash, &render(m.as_mut(), secs));
    }
    let n = (2.0 * SR) as usize;
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    m.set_input_by_name("size", 0.9);
    m.render_stereo(&mut l, &mut r);
    fingerprint(&mut hash, &l);
    fingerprint(&mut hash, &r);
    println!("arena fingerprint {hash:#018x}");
    assert_eq!(hash, ARENA_FINGERPRINT, "the Arena crowd changed: {hash:#018x}");
}

const ARENA_FINGERPRINT: u64 = 0xeeddfaae156012f2;

fn set(m: &mut dyn Model, param: &str, v: f32) {
    assert!(m.set_param_by_name(param, v), "no param {param}");
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

/// Hann-windowed DFT magnitudes of `x` from `lo` to `hi` Hz, one bin per `x.len()` resolution.
fn spectrum(x: &[f32], lo: f32, hi: f32) -> Vec<f32> {
    let n = x.len();
    let step = SR / n as f32;
    let (k0, k1) = ((lo / step) as usize, (hi / step) as usize);
    (k0..=k1)
        .map(|k| {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            let w = core::f32::consts::TAU * k as f32 / n as f32;
            for (i, v) in x.iter().enumerate() {
                let hann = 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n as f32).cos();
                let a = w * i as f32;
                re += v * hann * a.cos();
                im += v * hann * a.sin();
            }
            (re * re + im * im).sqrt()
        })
        .collect()
}

/// Share of 85 ms frames with a partial between `lo` and `hi` Hz that stands `ratio` times over
/// the median of the spectrum within 300 Hz of it: a voice's harmonics, a whistle. Noise through
/// formant filters never gets that peaky.
fn tonal_share(x: &[f32], lo: f32, hi: f32, ratio: f32) -> f32 {
    let frames: Vec<&[f32]> = x.chunks_exact(4096).collect();
    let reach = (300.0 / (SR / 4096.0)) as usize;
    let tonal = frames
        .iter()
        .filter(|f| {
            let s = spectrum(f, lo - 300.0, hi + 300.0);
            (reach..s.len() - reach).any(|k| {
                let mut near = s[k - reach..=k + reach].to_vec();
                near.sort_by(|a, b| a.partial_cmp(b).unwrap());
                s[k] > ratio * near[reach]
            })
        })
        .count();
    tonal as f32 / frames.len() as f32
}

/// p95 / p5 of the 250 ms RMS envelope, in dB: how much the crowd rises and falls.
fn swell_db(x: &[f32]) -> f32 {
    let mut e: Vec<f32> = x.chunks_exact((0.25 * SR) as usize).map(rms).collect();
    e.sort_by(|a, b| a.partial_cmp(b).unwrap());
    20.0 * (e[e.len() * 95 / 100] / e[e.len() * 5 / 100]).log10()
}

/// Sharp clicks per second (claps): on the slope of the signal (which favours the top), the 1 ms
/// RMS envelope jumping `db` over the median of the 50 ms around it, at least 20 ms apart.
fn clicks_per_second(x: &[f32], db: f32) -> f32 {
    let slope: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let e: Vec<f32> = slope.chunks_exact((0.001 * SR) as usize).map(rms).collect();
    let ratio = 10f32.powf(db / 20.0);
    let (mut count, mut last) = (0, 0usize);
    for k in 25..e.len() - 25 {
        let mut near = e[k - 25..=k + 25].to_vec();
        near.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if e[k] > ratio * near[25] && k >= last + 20 {
            count += 1;
            last = k;
        }
    }
    count as f32 * SR / x.len() as f32
}

#[test]
fn tavern_talk_has_pitch_not_hiss() {
    // Walla is speech without words: recordings of pubs show voice harmonics in 43-80 % of
    // 100 ms frames (laughter included). Our talk alone shows them in about 30 %; noise through
    // formant filters (the crowd before its retune) in none.
    let run = |voiced: f32| {
        let mut m = crowd("Tavern", 0.5, 0.1);
        set(m.as_mut(), "murmur/voiced", voiced);
        set(m.as_mut(), "calls/per_second", 0.0);
        render(m.as_mut(), 1.0);
        render(m.as_mut(), 6.0)
    };
    let (talk, hiss) = (tonal_share(&run(0.85), 300.0, 4000.0, 5.6), tonal_share(&run(0.0), 300.0, 4000.0, 5.6));
    println!("voiced frames: talk {talk:.2}, hiss {hiss:.2}");
    assert!(talk > 0.2 && hiss < 0.05, "voiced frames: talk {talk:.2}, hiss {hiss:.2}");
}

#[test]
fn voicing_changes_the_murmur_not_its_level() {
    // `murmur/voiced` blends the noise voices into talkers at about the same loudness.
    let run = |voiced: f32| {
        let mut m = crowd("Tavern", 0.5, 0.1);
        for (param, v) in [("murmur/voiced", voiced), ("calls/per_second", 0.0), ("cheer/level", 0.0), ("cheer/roar", 0.0), ("arena/room", 0.0), ("murmur/swell", 0.0)] {
            set(m.as_mut(), param, v);
        }
        render(m.as_mut(), 1.0);
        rms(&render(m.as_mut(), 8.0))
    };
    let (noise, talk, half) = (run(0.0), run(1.0), run(0.5));
    for (name, v) in [("talk", talk), ("half and half", half)] {
        let db = 20.0 * (v / noise).log10();
        println!("{name}: {db:.1} dB from the noise murmur");
        assert!(db.abs() < 2.0, "{name} is {db:.1} dB from the noise murmur");
    }
}

#[test]
fn stadium_is_a_band_not_a_rumble_and_swells_by_itself() {
    // Football crowd recordings sit between 500 Hz and 1.5 kHz, 20-30 dB down at 63-125 Hz, and
    // rise and fall by 9-20 dB over a minute even with nothing happening. The old Stadium had
    // its loudest band at 63 Hz and stayed within 3 dB.
    let mut m = crowd("Stadium", 1.0, 0.75);
    render(m.as_mut(), 1.0);
    let x = render(m.as_mut(), 60.0);
    let n = 8192;
    let step = SR / n as f32;
    let s = spectrum(&x[..n], 0.0, 1600.0);
    let band = |lo: f32, hi: f32| s.iter().enumerate().filter(|(k, _)| (*k as f32 * step) >= lo && (*k as f32 * step) < hi).map(|(_, v)| v * v).sum::<f32>();
    let low = band(40.0, 160.0) / band(400.0, 1600.0);
    println!("below 160 Hz: {:.1} dB", 10.0 * low.log10());
    assert!(low < 0.03, "energy below 160 Hz is {:.1} dB of the 400-1600 Hz band", 10.0 * low.log10());
    let swell = swell_db(&x);
    let mut still = crowd("Stadium", 1.0, 0.75);
    set(still.as_mut(), "murmur/swell", 0.0);
    render(still.as_mut(), 1.0);
    let flat = swell_db(&render(still.as_mut(), 60.0));
    println!("swell {swell:.1} dB, without it {flat:.1} dB");
    assert!(swell > 6.0 && swell > flat + 2.0, "swell {swell:.1} dB, without it {flat:.1} dB");
}

#[test]
fn festival_has_whistles_and_voices_that_stand_out() {
    // Open-air crowds are full of single people: finger whistles at 2-3.5 kHz, "woo"s, scattered
    // claps. A wash of filtered noise (the old Festival, but for its air horns) has none of them.
    let run = |events: bool| {
        let mut m = crowd("Festival", 0.8, 0.85);
        if !events {
            for param in ["calls/per_second", "whistles/per_minute", "claps/per_second", "horns/per_minute"] {
                set(m.as_mut(), param, 0.0);
            }
        }
        render(m.as_mut(), 1.0);
        render(m.as_mut(), 20.0)
    };
    let (lively, wash) = (run(true), run(false));
    let (whistles, none) = (tonal_share(&lively, 1800.0, 3800.0, 5.6), tonal_share(&wash, 1800.0, 3800.0, 5.6));
    let (claps, no_claps) = (clicks_per_second(&lively, 10.0), clicks_per_second(&wash, 10.0));
    println!("whistle frames {whistles:.3} / {none:.3}, clicks {claps:.2} / {no_claps:.2} per second");
    assert!(whistles > 0.05 && none < 0.02, "whistle frames: {whistles:.3} with events, {none:.3} without");
    assert!(claps > 0.5 && claps > 3.0 * no_claps, "clicks per second: {claps:.2} with events, {no_claps:.2} without");
}
