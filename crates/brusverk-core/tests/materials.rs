//! The material generators: `strike` (a struck object) and `roll` (rolling and sliding).
//! Each test encodes a trait measured on recordings of real hits (tools/impact_measure.py).
use brusverk_core::generators;
use brusverk_core::render::{peak, rms};
use brusverk_core::Model;

const SR: f32 = 48000.0;
const MATERIALS: [&str; 7] = ["Wood", "Metal", "Glass", "Stone", "Plastic", "Ceramic", "Rubber ball"];

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

fn strike(preset: &str) -> Box<dyn Model> {
    let mut m = generators::create("strike", SR).unwrap();
    let i = m.desc().preset_index(preset).unwrap_or_else(|| panic!("no preset {preset}"));
    m.load_preset(i);
    m
}

/// One hit with the strike position fixed, so two settings can be compared like for like.
fn hit(preset: &str, power: f32, size: f32, hardness: f32, secs: f32) -> Vec<f32> {
    let mut m = strike(preset);
    m.set_param_by_name("shape/variation", 0.0);
    m.set_input_by_name("power", power);
    m.set_input_by_name("size", size);
    m.set_input_by_name("hardness", hardness);
    m.trigger();
    render(m.as_mut(), secs)
}

/// Power spectrum of a Hann-windowed excerpt (zero-padded to a power of two), with bin width.
fn spectrum(x: &[f32]) -> (Vec<f32>, f32) {
    let n = x.len().next_power_of_two().max(1024);
    let mut re: Vec<f32> = (0..n)
        .map(|i| if i < x.len() { x[i] * (0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / x.len() as f32).cos()) } else { 0.0 })
        .collect();
    let mut im = vec![0.0f32; n];
    // Iterative radix-2 FFT.
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
        let ang = -core::f32::consts::TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = ((ang * k as f32).cos(), (ang * k as f32).sin());
                let (a, b) = (start + k, start + k + len / 2);
                let (xr, xi) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
            }
        }
        len <<= 1;
    }
    ((0..n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect(), SR / n as f32)
}

fn centroid(x: &[f32]) -> f32 {
    let (p, df) = spectrum(x);
    let total: f32 = p.iter().sum();
    p.iter().enumerate().map(|(k, v)| k as f32 * df * v).sum::<f32>() / total.max(1e-20)
}

/// The strongest spectral peaks between `lo` and `hi` Hz, at least 5 % apart, strongest first.
fn peaks(x: &[f32], lo: f32, hi: f32, count: usize) -> Vec<f32> {
    let (p, df) = spectrum(x);
    let mut cand: Vec<(f32, f32)> = (1..p.len() - 1)
        .filter(|&k| p[k] > p[k - 1] && p[k] >= p[k + 1] && k as f32 * df >= lo && k as f32 * df <= hi)
        .map(|k| (p[k], k as f32 * df))
        .collect();
    cand.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out: Vec<f32> = Vec::new();
    for (_, f) in cand {
        if out.iter().all(|g| (f / g - 1.0).abs() > 0.05) {
            out.push(f);
        }
        if out.len() == count {
            break;
        }
    }
    out
}

/// Milliseconds until the 5 ms RMS envelope stays `db` below its peak.
fn fall_ms(x: &[f32], db: f32) -> f32 {
    let hop = (SR * 0.005) as usize;
    let env: Vec<f32> = x.chunks(hop).map(rms).collect();
    let top = env.iter().cloned().fold(0.0, f32::max);
    let limit = top * 10f32.powf(-db / 20.0);
    let last = env.iter().rposition(|&e| e > limit).unwrap_or(0);
    (last + 1) as f32 * 5.0
}

fn db(a: f32, b: f32) -> f32 {
    20.0 * (a / b.max(1e-12)).log10()
}

#[test]
fn wood_is_short_and_woody() {
    // Recordings: a plank falls 20 dB in 20..60 ms; its attack centres near 1 kHz.
    let x = hit("Wood", 1.0, 0.5, 1.0, 1.0);
    let t20 = fall_ms(&x, 20.0);
    assert!(t20 <= 80.0, "wood should fall 20 dB within 80 ms, took {t20} ms");
    assert!(fall_ms(&x, 40.0) < 250.0, "wood should be 40 dB down within 250 ms");
    let c = centroid(&x[..(0.02 * SR) as usize]);
    assert!((500.0..2500.0).contains(&c), "wood attack centroid {c} Hz, real 800..1300");
}

#[test]
fn glass_rings_with_inharmonic_modes_above_1khz() {
    // A drinking glass: fundamental near 1.35 kHz ringing past half a second, partials at
    // 2.3x and 3.7x (not harmonics).
    // (The recording falls 60 dB in about 0.55 s; a plank in about 0.2 s.)
    let x = hit("Glass", 1.0, 0.5, 1.0, 1.5);
    let ring = fall_ms(&x, 60.0);
    assert!(ring > 500.0, "glass should ring past 0.5 s, fell 60 dB in {ring} ms");
    let wood = fall_ms(&hit("Wood", 1.0, 0.5, 1.0, 1.5), 60.0);
    assert!(ring > 2.0 * wood, "glass ({ring} ms) should ring far longer than wood ({wood} ms)");
    let modes = peaks(&x[(0.005 * SR) as usize..(0.3 * SR) as usize], 200.0, 12000.0, 3);
    assert!(modes.iter().all(|&f| f > 1000.0), "glass modes should all be above 1 kHz: {modes:?}");
    let f0 = modes.iter().cloned().fold(f32::MAX, f32::min);
    let inharmonic = modes.iter().filter(|&&f| f != f0).any(|&f| ((f / f0) - (f / f0).round()).abs() > 0.15);
    assert!(inharmonic, "glass partials should not be harmonics of {f0}: {modes:?}");
}

#[test]
fn metal_rings_for_seconds() {
    // A metal tube: 20 dB down after about a second, still sounding after three.
    let x = hit("Metal", 1.0, 0.5, 1.0, 4.0);
    assert!(fall_ms(&x, 20.0) > 500.0, "metal should ring: 20 dB fall in {} ms", fall_ms(&x, 20.0));
    let late = rms(&x[(3.0 * SR) as usize..(3.2 * SR) as usize]);
    assert!(db(late, peak(&x)) > -70.0, "metal should still sound at 3 s");
}

#[test]
fn stone_is_dull_and_noisy() {
    // Cobblestones: 20 dB down within 50 ms, a duller attack than glass and a dense, noisy
    // spectrum rather than a few clean partials.
    let stone = hit("Stone", 1.0, 0.5, 1.0, 1.0);
    let glass = hit("Glass", 1.0, 0.5, 1.0, 1.0);
    assert!(fall_ms(&stone, 20.0) <= 80.0, "stone 20 dB fall {} ms", fall_ms(&stone, 20.0));
    let a = (0.02 * SR) as usize;
    assert!(centroid(&stone[..a]) < 0.5 * centroid(&glass[..a]), "stone should be duller than glass");
    // Share of the ring's energy in its three strongest peaks (+-1 % each): low = noisy.
    let tonal = |x: &[f32]| {
        let ring = &x[(0.005 * SR) as usize..(0.15 * SR) as usize];
        let (p, df) = spectrum(ring);
        let total: f32 = p.iter().sum();
        peaks(ring, 50.0, 16000.0, 3)
            .iter()
            .map(|f| {
                let (lo, hi) = (((f * 0.99) / df) as usize, ((f * 1.01) / df) as usize + 1);
                p[lo..hi.min(p.len())].iter().sum::<f32>()
            })
            .sum::<f32>()
            / total
    };
    let (ts, tg) = (tonal(&stone), tonal(&glass));
    assert!(ts < 0.6 * tg, "stone should be noisier than glass: tonal share {ts:.2} vs {tg:.2}");
}

#[test]
fn materials_have_their_own_ring_times() {
    let t40 = |m: &str| fall_ms(&hit(m, 1.0, 0.5, 1.0, 6.0), 40.0);
    let (wood, plastic, stone, ceramic, glass, metal) = (t40("Wood"), t40("Plastic"), t40("Stone"), t40("Ceramic"), t40("Glass"), t40("Metal"));
    assert!(wood < ceramic && plastic < ceramic && stone < ceramic, "wood {wood}, plastic {plastic}, stone {stone} < ceramic {ceramic}");
    assert!(ceramic < glass && glass < metal, "ceramic {ceramic} < glass {glass} < metal {metal}");
}

#[test]
fn a_bigger_object_is_lower_and_rings_longer() {
    for mat in MATERIALS {
        let (small, big) = (hit(mat, 1.0, 0.25, 1.0, 4.0), hit(mat, 1.0, 0.75, 1.0, 4.0));
        let ring = |x: &[f32]| centroid(&x[(0.01 * SR) as usize..(0.3 * SR) as usize]);
        let (cs, cb) = (ring(&small), ring(&big));
        assert!(cb < cs * 0.75, "{mat}: bigger should be lower, ring centroid {cs:.0} -> {cb:.0} Hz");
        let (ts, tb) = (fall_ms(&small, 40.0), fall_ms(&big, 40.0));
        assert!(tb > ts, "{mat}: bigger should ring longer, {ts} -> {tb} ms");
    }
}

#[test]
fn harder_hits_are_louder_and_brighter() {
    // Recordings brighten by 0.5..1 octave of attack centroid per 20 dB of hit strength.
    for mat in MATERIALS {
        let (soft, hard) = (hit(mat, 0.15, 0.5, 1.0, 0.5), hit(mat, 1.0, 0.5, 1.0, 0.5));
        let a = (0.02 * SR) as usize;
        assert!(peak(&hard) > 3.0 * peak(&soft), "{mat}: power 1 should be much louder than 0.15");
        let (cs, ch) = (centroid(&soft[..a]), centroid(&hard[..a]));
        assert!(ch > cs * 1.15, "{mat}: a harder hit should be brighter: attack centroid {cs:.0} -> {ch:.0} Hz");
    }
}

#[test]
fn a_soft_floor_dulls_the_attack() {
    for mat in MATERIALS {
        let (carpet, stone) = (hit(mat, 0.8, 0.5, 0.0, 0.5), hit(mat, 0.8, 0.5, 1.0, 0.5));
        let a = (0.02 * SR) as usize;
        let (cc, cs) = (centroid(&carpet[..a]), centroid(&stone[..a]));
        assert!(cc < cs * 0.8, "{mat}: a soft floor should dull the attack: {cs:.0} -> {cc:.0} Hz");
    }
}

#[test]
fn every_hit_differs_unless_variation_is_zero() {
    for mat in MATERIALS {
        let mut m = strike(mat);
        m.trigger();
        let a = render(m.as_mut(), 0.3);
        render(m.as_mut(), 8.0);
        m.trigger();
        let b = render(m.as_mut(), 0.3);
        let diff = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(diff > 0.01 * peak(&a), "{mat}: two hits should differ (strike position)");
    }
    let (a, b) = (hit("Ceramic", 0.7, 0.5, 1.0, 0.3), hit("Ceramic", 0.7, 0.5, 1.0, 0.3));
    assert_eq!(a, b, "variation 0 repeats exactly");
}

#[test]
fn a_new_contact_adds_to_the_ringing_object() {
    // One playback per object: a second hit does not cut the first one's ring.
    let mut m = strike("Metal");
    m.set_param_by_name("shape/variation", 0.0);
    m.trigger();
    render(m.as_mut(), 0.5);
    let before = rms(&render(m.as_mut(), 0.1));
    m.set_input_by_name("power", 0.02);
    m.trigger();
    render(m.as_mut(), 0.01);
    let after = rms(&render(m.as_mut(), 0.1));
    assert!(after > before * 0.7, "a light touch must not silence a ringing pipe: {before} -> {after}");
    assert!(!m.is_finished());
}

#[test]
fn peaks_like_the_other_impacts() {
    // Full-power hits peak about as high as `impact` and `rock_hit` (about 0.65), median of 9.
    let median_peak = |m: &mut dyn Model| {
        let mut p: Vec<f32> = (0..9)
            .map(|_| {
                m.trigger();
                let x = render(m, 0.3);
                render(m, 12.0);
                peak(&x)
            })
            .collect();
        p.sort_by(f32::total_cmp);
        p[4]
    };
    let reference = median_peak(generators::create("rock_hit", SR).unwrap().as_mut());
    for mat in MATERIALS {
        let mut m = strike(mat);
        m.set_input_by_name("hardness", 1.0);
        let p = median_peak(m.as_mut());
        assert!(db(p, reference).abs() < 3.0, "{mat}: peak {p:.2} vs rock_hit {reference:.2}");
    }
}

#[test]
fn a_bouncing_ball_falls_quieter() {
    let mut m = strike("Rubber ball");
    let mut v = 1.0f32;
    let mut peaks_seen = Vec::new();
    for _ in 0..5 {
        m.set_input_by_name("power", v);
        m.trigger();
        let gap = 0.6 * v;
        peaks_seen.push(peak(&render(m.as_mut(), gap)));
        v *= 0.7;
    }
    assert!(peaks_seen.windows(2).all(|w| w[1] < w[0]), "each bounce quieter: {peaks_seen:?}");
}

#[test]
fn hundreds_of_strikes_are_cheap() {
    // A physics game triggers many at once: 64 ringing metal strikes (the most modes, the
    // longest ring) must render well inside real time even in a debug build.
    let mut all: Vec<_> = (0..64).map(|_| strike("Metal")).collect();
    for m in all.iter_mut() {
        m.trigger();
    }
    let mut buf = vec![0.0; 512];
    let start = std::time::Instant::now();
    for _ in 0..(SR as usize / 512) {
        for m in all.iter_mut() {
            m.render_mono(&mut buf);
        }
    }
    let t = start.elapsed().as_secs_f32();
    println!("64 ringing metal strikes: 1 s rendered in {:.1} ms", t * 1000.0);
    assert!(t < 0.5, "64 strikes took {t} s for 1 s of audio");
}

// ---------------------------------------------------------------------------------------------
// roll
// ---------------------------------------------------------------------------------------------

fn roll(preset: &str, speed: f32, surface: f32, size: f32, slide: f32) -> Vec<f32> {
    let mut m = generators::create("roll", SR).unwrap();
    let i = m.desc().preset_index(preset).unwrap();
    m.load_preset(i);
    for (k, v) in [speed, surface, size, slide].into_iter().enumerate() {
        m.set_input(k, v);
    }
    m.snap();
    render(m.as_mut(), 2.0)[(0.5 * SR) as usize..].to_vec()
}

#[test]
fn rolling_is_silent_at_rest_and_grows_with_speed() {
    for preset in ["Wooden ball", "Steel ball", "Glass marble", "Bowling ball", "Barrel"] {
        let still = rms(&roll(preset, 0.0, 0.5, 0.5, 0.0));
        let (slow, fast) = (roll(preset, 0.2, 0.5, 0.5, 0.0), roll(preset, 0.8, 0.5, 0.5, 0.0));
        assert!(still < 1e-4, "{preset}: a still ball is silent ({still})");
        assert!(rms(&fast) > 2.0 * rms(&slow), "{preset}: faster should be louder");
        assert!(centroid(&fast) > centroid(&slow), "{preset}: faster should be brighter");
    }
}

#[test]
fn a_marble_rolls_higher_than_a_bowling_ball() {
    // Recordings: a marble on a wooden floor centres near 1.7 kHz, a bowling ball near 650 Hz,
    // a wooden ball on wood near 350..450 Hz.
    let marble = centroid(&roll("Glass marble", 0.5, 0.4, 0.1, 0.0));
    let bowling = centroid(&roll("Bowling ball", 0.45, 0.4, 0.75, 0.0));
    let wood = centroid(&roll("Wooden ball", 0.5, 0.4, 0.5, 0.0));
    assert!(marble > 1.5 * bowling && bowling > wood, "marble {marble:.0}, bowling {bowling:.0}, wooden ball {wood:.0} Hz");
    assert!((250.0..700.0).contains(&wood), "wooden ball centroid {wood:.0} Hz, real 340..440");
}

#[test]
fn a_rough_floor_clacks() {
    // Peak over RMS: the clacks over the floor's grain stand out on a rough floor.
    let crest = |x: &[f32]| db(peak(x), rms(x));
    let (smooth, rough) = (roll("Wooden ball", 0.5, 0.0, 0.5, 0.0), roll("Wooden ball", 0.5, 1.0, 0.5, 0.0));
    assert!(rms(&rough) > rms(&smooth), "rough floor louder");
    assert!(crest(&rough) > crest(&smooth) + 2.0, "rough floor should clack: crest {:.1} vs {:.1} dB", crest(&rough), crest(&smooth));
}

#[test]
fn sliding_adds_friction() {
    let hi = |x: &[f32]| {
        let (p, df) = spectrum(x);
        p.iter().enumerate().filter(|(k, _)| *k as f32 * df > 1500.0).map(|(_, v)| v).sum::<f32>() / p.iter().sum::<f32>()
    };
    let (rolling, sliding) = (roll("Wooden ball", 0.5, 0.4, 0.5, 0.0), roll("Wooden ball", 0.5, 0.4, 0.5, 1.0));
    assert!(rms(&sliding) > rms(&rolling), "sliding is louder");
    assert!(hi(&sliding) > 2.0 * hi(&rolling), "sliding should add friction noise up high: {:.3} vs {:.3}", hi(&sliding), hi(&rolling));
}

#[test]
fn a_bigger_ball_rolls_lower() {
    let small = centroid(&roll("Plastic ball", 0.5, 0.4, 0.2, 0.0));
    let big = centroid(&roll("Plastic ball", 0.5, 0.4, 0.9, 0.0));
    assert!(big < small * 0.8, "bigger ball should roll lower: {small:.0} -> {big:.0} Hz");
}
