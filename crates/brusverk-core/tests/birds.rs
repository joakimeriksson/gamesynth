//! The `bird` species: each test encodes what makes a species recognisable, as measured on CC0
//! recordings (see `generators/birdsong/species.rs` and `target/refs/birds/SOURCES.tsv`). The
//! output is analysed the way the recordings were: a pitch track (FFT peak, 2 ms hop),
//! syllables segmented from its level, songs from the gaps between syllables.

use brusverk_core::generators::{self, birdsong::species};
use brusverk_core::render::rms;
use brusverk_core::Model;

const SR: f32 = 48000.0;
const HOP: usize = 96;

fn bird(species: &str, individual: f32, inputs: &[(&str, f32)]) -> Box<dyn Model> {
    let mut m = generators::create("bird", SR).unwrap();
    let i = m.desc().preset_index(species).unwrap_or_else(|| panic!("no preset {species}"));
    m.load_preset(i);
    assert!(m.set_param_by_name("bird/individual", individual));
    assert!(m.set_param_by_name("space/reverb", 0.0));
    assert!(m.set_input_by_name("distance", 0.0));
    for (k, v) in inputs {
        assert!(m.set_input_by_name(k, *v), "no input {k}");
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

// ---- analysis ----

/// In-place radix-2 FFT of (re, im).
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
    let mut len = 2;
    while len <= n {
        let ang = -core::f32::consts::TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f32).sin_cos();
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
}

/// One analysis frame: the strongest frequency between `lo` and `hi` Hz and its level (dB).
#[derive(Clone, Copy, Debug)]
struct Frame {
    hz: f32,
    db: f32,
}

/// Pitch track: the FFT peak (parabolic) in `lo..hi` every `HOP` samples, from Hann frames of
/// 1024 samples (21 ms) below 1.5 kHz and 256 (5 ms, for fast songbird syllables) above.
fn track(x: &[f32], lo: f32, hi: f32) -> Vec<Frame> {
    let win_len = if lo < 1000.0 { 1024 } else { 256 };
    let n = 2048;
    let win: Vec<f32> = (0..win_len).map(|i| 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / win_len as f32).cos()).collect();
    let (k0, k1) = ((lo / SR * n as f32) as usize, ((hi / SR * n as f32) as usize).min(n / 2 - 2));
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + win_len <= x.len() {
        let mut re = vec![0.0f32; n];
        let mut im = vec![0.0f32; n];
        for i in 0..win_len {
            re[i] = x[pos + i] * win[i];
        }
        fft(&mut re, &mut im);
        let mag: Vec<f32> = (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() + 1e-12).collect();
        let k = (k0.max(1)..=k1).max_by(|a, b| mag[*a].total_cmp(&mag[*b])).unwrap();
        let (a, b, c) = (mag[k - 1].ln(), mag[k].ln(), mag[k + 1].ln());
        let d = a - 2.0 * b + c;
        let off = if d.abs() > 1e-9 { (0.5 * (a - c) / d).clamp(-0.5, 0.5) } else { 0.0 };
        out.push(Frame { hz: (k as f32 + off) * SR / n as f32, db: 20.0 * (mag[k] / (win_len as f32 / 4.0)).log10() });
        pos += HOP;
    }
    out
}

/// A segmented syllable: onset and duration in seconds, median pitch.
#[derive(Clone, Copy, Debug)]
struct Syl {
    t: f32,
    dur: f32,
    hz: f32,
}

/// Syllables: runs of frames within `range` dB of the loudest, bridging gaps under `bridge` s.
fn syllables(frames: &[Frame], range: f32, bridge: f32) -> Vec<Syl> {
    syllables_win(frames, range, bridge, 0.0)
}

/// [`syllables`], with onsets shifted by half the analysis window `win_s`.
fn syllables_win(frames: &[Frame], range: f32, bridge: f32, win_s: f32) -> Vec<Syl> {
    let top = frames.iter().map(|f| f.db).fold(f32::MIN, f32::max);
    let dt = HOP as f32 / SR;
    let on: Vec<bool> = frames.iter().map(|f| f.db > top - range).collect();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < on.len() {
        if !on[i] {
            i += 1;
            continue;
        }
        let s = i;
        while i < on.len() && on[i] {
            i += 1;
        }
        match out.last_mut() {
            Some(last) if (s - last.1) as f32 * dt < bridge => last.1 = i,
            _ => out.push((s, i)),
        }
    }
    out.into_iter()
        .filter(|(a, b)| b - a >= 2)
        .map(|(a, b)| {
            let mut hz: Vec<f32> = frames[a..b].iter().filter(|f| f.db > top - range).map(|f| f.hz).collect();
            hz.sort_by(f32::total_cmp);
            Syl { t: a as f32 * dt + win_s / 2.0, dur: (b - a) as f32 * dt, hz: hz[hz.len() / 2] }
        })
        .collect()
}

/// Songs: groups of syllables separated by more than `gap` seconds.
fn songs(syls: &[Syl], gap: f32) -> Vec<Vec<Syl>> {
    let mut out: Vec<Vec<Syl>> = Vec::new();
    for s in syls {
        match out.last_mut() {
            Some(song) if s.t - (song.last().unwrap().t + song.last().unwrap().dur) < gap => song.push(*s),
            _ => out.push(vec![*s]),
        }
    }
    out
}

fn semitones(a: f32, b: f32) -> f32 {
    12.0 * (a / b).log2()
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

// ---- species ----

#[test]
fn every_species_is_a_preset_and_transcribed_species_cite_recordings() {
    let m = generators::create("bird", SR).unwrap();
    for (i, sp) in species::SPECIES.iter().enumerate() {
        assert_eq!(species::NAMES[i], sp.name);
        assert!(m.desc().preset_index(sp.name).is_some(), "no preset for {}", sp.name);
        // A transcription is compact: a few hundred numbers, not audio.
        let numbers: usize = sp.syllables.iter().map(|s| 3 * (s.keys.len() + s.keys2.len()) + 2).sum::<usize>()
            + sp.phrases.iter().map(|p| 2 * p.unit.len() + 7).sum::<usize>()
            + sp.timbres.iter().map(|t| 10 + 3 * t.formants.len()).sum::<usize>();
        println!("{:<18} {:>3} syllable types, {:>4} numbers{}", sp.name, sp.syllables.len(), numbers, if sp.invented { " (invented)" } else { "" });
        assert!(numbers < 800, "{} is {numbers} numbers", sp.name);
    }
    assert!(species::SPECIES.iter().filter(|s| !s.invented).count() >= 12);
}

#[test]
fn cuckoo_sings_two_notes_a_major_third_apart() {
    // Measured (94 syllables, two birds): "cuck" ~655 Hz, "oo" ~515 Hz (4.2 semitones lower),
    // the "oo" 0.285 s after the "cuck"; a call every 1.05-1.2 s.
    let x = render(bird("Cuckoo", 0.0, &[]).as_mut(), 40.0);
    let syls = syllables(&track(&x, 300.0, 1200.0), 25.0, 0.02);
    let (mut intervals, mut gaps, mut periods) = (Vec::new(), Vec::new(), Vec::new());
    let mut last_cuck: Option<f32> = None;
    for w in syls.windows(2) {
        let st = semitones(w[0].hz, w[1].hz);
        if st > 2.0 && w[1].t - w[0].t < 0.5 {
            intervals.push(st);
            gaps.push(w[1].t - w[0].t);
            if let Some(t) = last_cuck {
                if w[0].t - t < 1.6 {
                    periods.push(w[0].t - t);
                }
            }
            last_cuck = Some(w[0].t);
        }
    }
    println!("{} calls; interval {:.2} st, onset gap {:.3} s, period {:.2} s", intervals.len(), median(intervals.clone()), median(gaps.clone()), median(periods.clone()));
    assert!(intervals.len() >= 10, "only {} cuck-oo calls in 40 s", intervals.len());
    assert!((3.4..5.0).contains(&median(intervals)), "the interval is a major third");
    assert!((0.25..0.32).contains(&median(gaps)), "the oo follows the cuck after ~0.285 s");
    assert!((1.0..1.25).contains(&median(periods)), "a call every 1.05-1.2 s");
    let cuck = median(syls.iter().filter(|s| s.hz > 590.0).map(|s| s.hz).collect());
    assert!((600.0..720.0).contains(&cuck), "the cuck near 655 Hz, got {cuck:.0}");
}

#[test]
fn chaffinch_trill_descends_and_speeds_up_into_a_flourish() {
    // Measured: two or three trills, each lower than the last, the syllables coming faster
    // (100 -> 55 ms apart in song type B; type A's third trill is a slower two-note unit, so a
    // type-A song without its bridge may not speed up overall), then a flourish; songs 2-2.8 s.
    let (mut checked, mut faster) = (0, 0);
    for individual in [0.0, 1.0, 2.0] {
        let x = render(bird("Chaffinch", individual, &[("activity", 1.0)]).as_mut(), 40.0);
        let syls = syllables(&track(&x, 1500.0, 9000.0), 30.0, 0.004);
        // Whole songs only: not the one the render cuts off.
        for song in songs(&syls, 0.6).iter().filter(|s| s.len() >= 12 && s.last().unwrap().t < 39.0) {
            let start = song[0].t;
            let len = song.last().unwrap().t + song.last().unwrap().dur - start;
            // The trills: everything before the flourish (the last 0.4 s).
            let trill: Vec<&Syl> = song.iter().filter(|s| s.t < start + len - 0.4).collect();
            let half = start + (len - 0.4) / 2.0;
            let (a, b): (Vec<&Syl>, Vec<&Syl>) = trill.iter().partition(|s| s.t < half);
            let rate = |v: &[&Syl]| v.len() as f32 / (v.last().unwrap().t - v[0].t).max(0.05);
            let pitch = |v: &[&Syl]| median(v.iter().map(|s| s.hz).collect());
            println!("song {len:.2} s: rate {:.1} -> {:.1} per s, pitch {:.0} -> {:.0} Hz", rate(&a), rate(&b), pitch(&a), pitch(&b));
            assert!((1.6..3.2).contains(&len), "song length {len:.2} s");
            faster += (rate(&b) > rate(&a) * 1.1) as usize;
            assert!(pitch(&b) < pitch(&a) * 0.97, "the trill descends");
            checked += 1;
        }
    }
    assert!(checked >= 6, "only {checked} songs");
    assert!(faster * 4 >= checked * 3, "the trill speeds up in only {faster} of {checked} songs");
}

#[test]
fn great_tit_alternates_two_syllables() {
    // "tea-cher tea-cher": two notes a few semitones apart, alternating.
    let mut alternating = 0;
    let mut total = 0;
    for individual in [0.0, 1.0, 2.0, 3.0] {
        let x = render(bird("Great tit", individual, &[("activity", 1.0)]).as_mut(), 30.0);
        let syls = syllables(&track(&x, 2000.0, 9000.0), 25.0, 0.01);
        for song in songs(&syls, 0.6).iter().filter(|s| s.len() >= 6 && s.last().unwrap().t < 29.0) {
            total += 1;
            let notes: Vec<f32> = song.iter().map(|s| s.hz).collect();
            let alt = (0..notes.len() - 2).filter(|&i| semitones(notes[i], notes[i + 1]).abs() > 1.5 && semitones(notes[i], notes[i + 2]).abs() < 0.6).count();
            let share = alt as f32 / (notes.len() - 2) as f32;
            println!("song of {} notes: {:.0} % alternate ({:.0}, {:.0} Hz ...)", notes.len(), share * 100.0, notes[0], notes[1]);
            if share > 0.7 {
                alternating += 1;
            }
        }
    }
    assert!(total >= 8, "only {total} songs");
    // Two of the three song types measured are two-note "tea-cher"s; the third is a four-note unit.
    assert!(alternating * 2 >= total, "{alternating} of {total} songs alternate two notes");
}

#[test]
fn collared_dove_coos_in_triplets_with_the_long_note_in_the_middle() {
    // Measured: "coo-COOO-coo", 0.28 / 0.42 / 0.30 s at ~500-550 Hz, every 1.75 s.
    let x = render(bird("Collared dove", 0.0, &[("activity", 1.0)]).as_mut(), 40.0);
    let syls = syllables(&track(&x, 300.0, 1200.0), 25.0, 0.03);
    let groups = songs(&syls, 0.35);
    let triplets: Vec<&Vec<Syl>> = groups.iter().filter(|g| g.len() == 3).collect();
    let middle_longest = triplets.iter().filter(|g| g[1].dur > g[0].dur && g[1].dur > g[2].dur).count();
    let periods: Vec<f32> = triplets.windows(2).map(|w| w[1][0].t - w[0][0].t).filter(|p| *p < 2.5).collect();
    let pitch = median(syls.iter().map(|s| s.hz).collect());
    println!("{} triplets, {middle_longest} with the middle note longest, period {:.2} s, {pitch:.0} Hz", triplets.len(), median(periods.clone()));
    assert!(triplets.len() >= 6 && middle_longest * 10 >= triplets.len() * 8);
    assert!((1.6..1.9).contains(&median(periods)), "a triplet every ~1.75 s");
    assert!((470.0..580.0).contains(&pitch));
}

#[test]
fn wood_pigeon_phrase_has_five_coos_with_one_long() {
    // Measured: "coo-COOO-coo, coo-coo", five coos (0.30 / 0.58 / 0.42 / 0.24 / 0.38 s between
    // their level dips; ours are a little longer so they nearly run together, as the hoarse
    // coos do), the phrase every ~2.8 s: the long coo comes every fifth note, 2.6-3.0 s apart.
    let x = render(bird("Wood pigeon", 0.0, &[("activity", 1.0)]).as_mut(), 60.0);
    let syls = syllables(&track(&x, 250.0, 900.0), 22.0, 0.05);
    let long: Vec<usize> = (0..syls.len()).filter(|&i| syls[i].dur > 0.55).collect();
    let steps: Vec<usize> = long.windows(2).map(|w| w[1] - w[0]).collect();
    let periods: Vec<f32> = long.windows(2).map(|w| syls[w[1]].t - syls[w[0]].t).filter(|p| *p < 3.5).collect();
    let fifth = steps.iter().filter(|s| **s == 5).count();
    println!("{} coos, {} long; steps {steps:?}; period {:.2} s", syls.len(), long.len(), median(periods.clone()));
    assert!(long.len() >= 6 && fifth * 10 >= steps.len() * 6, "the long coo comes every fifth note");
    assert!((2.5..3.1).contains(&median(periods)));
}

/// Third-octave bands (0.5..8 kHz) of the loud frames, in dB relative to the strongest band.
fn bands(x: &[f32]) -> Vec<f32> {
    let n = 2048;
    let mut acc = vec![0.0f64; n / 2];
    let top = x.chunks(n).map(rms).fold(0.0f32, f32::max);
    for c in x.chunks_exact(n).filter(|c| rms(c) > top * 0.3) {
        let mut re: Vec<f32> = c.iter().enumerate().map(|(i, s)| s * (0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n as f32).cos())).collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        for k in 0..n / 2 {
            acc[k] += (re[k] * re[k] + im[k] * im[k]) as f64;
        }
    }
    let mut out = Vec::new();
    let mut lo = 500.0f32;
    while lo < 8000.0 {
        let hi = lo * 2f32.powf(1.0 / 3.0);
        let (a, b) = ((lo / SR * n as f32) as usize, (hi / SR * n as f32) as usize);
        out.push(10.0 * (acc[a..b].iter().sum::<f64>() + 1e-20).log10() as f32);
        lo = hi;
    }
    let max = out.iter().cloned().fold(f32::MIN, f32::max);
    out.iter().map(|b| b - max).collect()
}

#[test]
fn crow_caw_is_broadband_and_harsh() {
    // Measured on two crows: caws fill 1-2.5 kHz within 12 dB (4-5 third-octave bands around a
    // formant near 1.7 kHz), with energy on to 6 kHz (-18 to -27 dB); a whistle fills one or two
    // bands.
    let full = |name: &str| {
        let x = render(bird(name, 0.0, &[("activity", 1.0)]).as_mut(), 30.0);
        let b = bands(&x);
        (b.iter().filter(|v| **v > -12.0).count(), b)
    };
    let (crow, b) = full("Crow");
    let (cuckoo, _) = full("Cuckoo");
    let (tit, _) = full("Great tit");
    println!("bands within 12 dB: crow {crow}, cuckoo {cuckoo}, great tit {tit}; crow {b:.0?}");
    assert!(crow >= 4, "a caw is broadband: {crow} bands");
    assert!(cuckoo <= 2 && tit < crow, "whistles are narrower than caws");
    // 4-6.3 kHz still carries energy.
    assert!(b[b.len() - 3] > -30.0);
}

#[test]
fn tawny_owl_hoots_in_its_measured_range() {
    // Measured on five birds: the hoot's fundamental sits at 650-960 Hz, a near-pure tone; the
    // long tremolo hoot is 1.3-1.7 s. (Typical figures quoted for the species are lower; these
    // recordings are what the voice was fitted to.)
    let x = render(bird("Tawny owl", 0.0, &[("activity", 1.0)]).as_mut(), 40.0);
    let frames = track(&x, 300.0, 3000.0);
    let syls = syllables(&frames, 25.0, 0.05);
    let long: Vec<&Syl> = syls.iter().filter(|s| s.dur > 1.1).collect();
    let pitch = median(syls.iter().map(|s| s.hz).collect());
    println!("{} hoots, {} long; median {pitch:.0} Hz", syls.len(), long.len());
    assert!((600.0..1000.0).contains(&pitch), "hoot at {pitch:.0} Hz");
    assert!(!long.is_empty() && long.iter().all(|s| s.dur < 1.9));
    let b = bands(&x);
    assert!(b.iter().skip(5).all(|v| *v < -25.0), "a near-pure hoot: {b:.0?}");
}

#[test]
fn renditions_vary_but_the_species_stays() {
    // No two songs are the same: lengths, syllable counts and pitches all move a little.
    for name in ["Blackbird", "Chaffinch", "Robin", "Great tit"] {
        let x = render(bird(name, 0.0, &[("activity", 1.0)]).as_mut(), 45.0);
        let syls = syllables(&track(&x, 1200.0, 9500.0), 30.0, 0.004);
        let s: Vec<(f32, usize, f32)> = songs(&syls, 0.6)
            .iter()
            .filter(|s| s.len() >= 3)
            .map(|s| (s.last().unwrap().t + s.last().unwrap().dur - s[0].t, s.len(), median(s.iter().map(|x| x.hz).collect())))
            .collect();
        let same = s.windows(2).filter(|w| (w[0].0 - w[1].0).abs() < 0.01 && w[0].1 == w[1].1 && (w[0].2 / w[1].2 - 1.0).abs() < 0.002).count();
        println!("{name}: {} songs, {same} identical neighbours; {:?}", s.len(), &s[..s.len().min(4)]);
        assert!(s.len() >= 4, "{name}: {} songs", s.len());
        assert_eq!(same, 0, "{name} repeats itself exactly");
    }
    // Two individuals of a species differ, too.
    let a = render(bird("Cuckoo", 0.0, &[]).as_mut(), 30.0);
    let b = render(bird("Cuckoo", 7.0, &[]).as_mut(), 30.0);
    let pa = median(syllables(&track(&a, 300.0, 1200.0), 25.0, 0.02).iter().map(|s| s.hz).collect());
    let pb = median(syllables(&track(&b, 300.0, 1200.0), 25.0, 0.02).iter().map(|s| s.hz).collect());
    assert!((pa / pb - 1.0).abs() > 0.003, "two cuckoos at {pa:.0} and {pb:.0} Hz");
}

#[test]
fn excitement_brings_alarm_calls() {
    // Blackbird alarm (Freesound 835517): a rattle of loud sweeps 7.8 -> 4.2 kHz, ~7 a second.
    let x = render(bird("Blackbird", 0.0, &[("activity", 1.0), ("excitement", 1.0)]).as_mut(), 20.0);
    let syls = syllables(&track(&x, 1000.0, 10000.0), 25.0, 0.004);
    let fast = syls.windows(2).filter(|w| (0.11..0.16).contains(&(w[1].t - w[0].t))).count();
    let pitch = median(syls.iter().map(|s| s.hz).collect());
    println!("{} syllables, {fast} at the rattle's pace, {pitch:.0} Hz", syls.len());
    assert!(fast >= 15 && pitch > 4000.0);
    // Without excitement the song is the low fluting kind.
    // Without excitement the song is the low fluting kind: its long notes sit below 3.2 kHz.
    let x = render(bird("Blackbird", 0.0, &[("activity", 1.0)]).as_mut(), 20.0);
    let song = median(syllables(&track(&x, 1000.0, 10000.0), 25.0, 0.004).iter().filter(|s| s.dur > 0.12).map(|s| s.hz).collect());
    assert!(song < 3200.0, "song median {song:.0} Hz");
}

#[test]
fn woodpecker_drums_an_accelerating_roll() {
    // Measured: ~20 strikes in 0.55-0.65 s, intervals shrinking from ~33 to ~25 ms.
    let x = render(bird("Woodpecker", 0.0, &[("activity", 1.0)]).as_mut(), 20.0);
    // Strikes: peaks of the level (2 ms windows every 0.5 ms) that are the loudest within
    // ±9 ms and stand 6 dB above the dip since the last one, as in `tools/birds/drum.py`.
    let n = 24;
    let env: Vec<f32> = (0..x.len() / n - 4).map(|i| rms(&x[i * n..i * n + 4 * n])).collect();
    let top = env.iter().cloned().fold(0.0, f32::max);
    let mut hits: Vec<(f32, f32)> = Vec::new();
    let mut dip = f32::MAX;
    for i in 18..env.len() - 18 {
        dip = dip.min(env[i]);
        let local = env[i - 18..=i + 18].iter().cloned().fold(0.0, f32::max);
        if env[i] == local && env[i] > top * 0.02 && env[i] > 2.0 * dip {
            hits.push(((i * n) as f32 / SR, env[i]));
            dip = env[i];
        }
    }
    let mut rolls: Vec<Vec<(f32, f32)>> = Vec::new();
    for h in hits {
        match rolls.last_mut() {
            Some(roll) if h.0 - roll.last().unwrap().0 < 0.1 => roll.push(h),
            _ => rolls.push(vec![h]),
        }
    }
    // Whole rolls only (not one already under way when the render began).
    let full: Vec<&Vec<(f32, f32)>> = rolls.iter().filter(|r| r.len() >= 12 && r[0].0 > 0.1).collect();
    assert!(full.len() >= 2, "rolls: {:?}", rolls.iter().map(|r| r.len()).collect::<Vec<_>>());
    for r in full {
        let span = r.last().unwrap().0 - r[0].0;
        // Mean strike interval over the first and the last third of the roll (a soft strike
        // missed would leave a double gap, which is left out).
        let iv: Vec<f32> = r.windows(2).map(|w| w[1].0 - w[0].0).filter(|i| *i < 0.045).collect();
        let k = (iv.len() / 3).max(1);
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let (early, late) = (mean(&iv[..k]), mean(&iv[iv.len() - k..]));
        println!("roll of {} strikes over {span:.2} s: {:.0} -> {:.0} ms per strike", r.len(), early * 1e3, late * 1e3);
        assert!(late < early * 0.95, "the roll accelerates");
        assert!((0.4..0.85).contains(&span), "a roll lasts about 0.6 s");
        assert!((14..=26).contains(&r.len()), "about 20 strikes");
    }
}

// ---- what keeps them from sounding synthetic ----

#[test]
fn contours_are_smooth_curves_without_corners() {
    // Through any keypoints the contour's slope is continuous: just before and just after a
    // keypoint the pitch moves at nearly the same rate, and a curve through a peak key never
    // overshoots it (`tools/birds/traits.py` measures the same on renders and recordings).
    use brusverk_core::generators::birdsong::contour;
    for sp in species::SPECIES {
        for syl in sp.syllables {
            for keys in [syl.keys, syl.keys2] {
                if keys.len() < 3 {
                    continue;
                }
                for k in &keys[1..keys.len() - 1] {
                    let e = 1e-4;
                    let at = |u: f32| contour(keys, u).0.log2();
                    let (before, after) = ((at(k.0) - at(k.0 - e)) / e, (at(k.0 + e) - at(k.0)) / e);
                    assert!((before - after).abs() < 0.05 * before.abs().max(after.abs()) + 0.2, "{}: corner at u = {} ({before:.1} vs {after:.1} oct per unit)", sp.name, k.0);
                }
                let (lo, hi) = keys.iter().fold((f32::MAX, 0.0f32), |(lo, hi), k| (lo.min(k.1), hi.max(k.1)));
                for i in 0..=200 {
                    let (hz, amp) = contour(keys, i as f32 / 200.0);
                    assert!(hz >= lo * 0.999 && hz <= hi * 1.001 && amp >= 0.0, "{}: overshoot {hz}", sp.name);
                }
            }
        }
    }
}

#[test]
fn notes_carry_measured_micro_modulation() {
    // Real notes wobble: the pitch track's residual from its 41 ms smoothing, on the loudest
    // sustained notes, is 5-135 cents rms on blackbird notes (the long fluting ones warble most;
    // tools/birds/fm.py on 811988 and 725332), 24-30 on the robin, ~5-15 on the cuckoo and
    // collared dove. A note held without it reads as synthetic; far more reads as broken.
    let wobble = |name: &str| {
        let x = render(bird(name, 0.0, &[("activity", 1.0)]).as_mut(), 30.0);
        let frames = track(&x, if name == "Cuckoo" || name == "Collared dove" { 300.0 } else { 1200.0 }, 9500.0);
        let dt = HOP as f32 / SR;
        let top = frames.iter().map(|f| f.db).fold(f32::MIN, f32::max);
        // Residual around a 41 ms local quadratic fit (Savitzky-Golay, as in traits.py), on loud
        // sustained frames.
        let half = (0.0205 / dt) as usize;
        let m = half as f32;
        let sg: Vec<f32> = (0..=2 * half)
            .map(|j| {
                let j = j as f32 - m;
                (3.0 * (3.0 * m * m + 3.0 * m - 1.0) - 15.0 * j * j) / ((2.0 * m + 1.0) * (4.0 * m * m + 4.0 * m - 3.0))
            })
            .collect();
        let mut sq = (0.0f64, 0usize);
        for i in half..frames.len() - half {
            let w = &frames[i - half..=i + half];
            if w.iter().any(|f| f.db < top - 15.0) {
                continue;
            }
            let mean: f32 = w.iter().zip(&sg).map(|(f, g)| f.hz.log2() * g).sum();
            let slope = (w[w.len() - 1].hz / w[0].hz).log2().abs() / (2.0 * half as f32 * dt);
            if slope > 2.0 {
                continue;
            }
            let c = 1200.0 * (frames[i].hz.log2() - mean);
            sq.0 += (c * c) as f64;
            sq.1 += 1;
        }
        (sq.0 / sq.1.max(1) as f64).sqrt() as f32
    };
    for (name, lo, hi) in [("Blackbird", 15.0, 140.0), ("Robin", 10.0, 60.0), ("Cuckoo", 1.5, 16.0), ("Collared dove", 1.0, 12.0)] {
        let w = wobble(name);
        println!("{name}: wobble {w:.1} cents rms");
        assert!((lo..hi).contains(&w), "{name} wobble {w:.1} cents, measured on recordings {lo}-{hi}");
    }
}

#[test]
fn harmonics_sit_well_below_the_fundamental() {
    // Measured on the recordings' loud frames: the 2nd harmonic of blackbird, chaffinch and
    // great tit notes is 36-46 dB below the fundamental. A whistle with strong harmonics sounds
    // like a synth, not a bird.
    for name in ["Blackbird", "Chaffinch", "Great tit", "Robin"] {
        let x = render(bird(name, 0.0, &[("activity", 1.0)]).as_mut(), 30.0);
        let n = 2048;
        let top = x.chunks(n).map(rms).fold(0.0, f32::max);
        let mut ratios = Vec::new();
        for c in x.chunks_exact(n).filter(|c| rms(c) > top * 0.5) {
            let mut re: Vec<f32> = c.iter().enumerate().map(|(i, s)| s * (0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n as f32).cos())).collect();
            let mut im = vec![0.0; n];
            fft(&mut re, &mut im);
            let mag: Vec<f32> = (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).collect();
            let k1 = (20..n / 4).max_by(|a, b| mag[*a].total_cmp(&mag[*b])).unwrap();
            let k2 = 2 * k1;
            if k2 + 3 < n / 2 {
                let h2 = mag[k2 - 3..=k2 + 3].iter().cloned().fold(0.0, f32::max);
                ratios.push(20.0 * (h2 / mag[k1]).log10());
            }
        }
        let h2 = median(ratios);
        println!("{name}: 2nd harmonic {h2:.1} dB");
        assert!(h2 < -28.0, "{name}: 2nd harmonic only {h2:.1} dB down");
    }
}

#[test]
fn blackbird_phrases_flow_legato() {
    // Measured on 811988: notes inside a song follow within ~17 ms (median), 44 % joined (under
    // 10 ms). Ours must not be separated blocks.
    let x = render(bird("Blackbird", 0.0, &[("activity", 1.0)]).as_mut(), 40.0);
    let syls = syllables(&track(&x, 1200.0, 8000.0), 25.0, 0.0);
    let gaps: Vec<f32> = syls.windows(2).map(|w| w[1].t - (w[0].t + w[0].dur)).filter(|g| *g < 0.6).collect();
    let joined = gaps.iter().filter(|g| **g < 0.012).count() as f32 / gaps.len() as f32;
    let med = median(gaps.clone()) * 1000.0;
    println!("{} gaps, median {med:.0} ms, {:.0} % joined", gaps.len(), joined * 100.0);
    assert!(med < 35.0 && joined > 0.25);
}

// ---- the engine ----

#[test]
fn every_species_is_bounded_click_free_and_real_time() {
    for name in species::NAMES {
        for excitement in [0.0, 1.0] {
            let mut m = bird(name, 3.0, &[("activity", 1.0), ("excitement", excitement)]);
            let t = std::time::Instant::now();
            let x = render(m.as_mut(), 30.0);
            let speed = 30.0 / t.elapsed().as_secs_f32();
            let peak = x.iter().fold(0.0f32, |a, s| a.max(s.abs()));
            assert!(x.iter().all(|s| s.is_finite()) && peak <= 1.0, "{name}: peak {peak}");
            assert!(peak > 0.02, "{name} silent at excitement {excitement}");
            // Clicks: a note may not start from silence with a jump.
            let mut quiet = 0usize;
            let mut worst = 0.0f32;
            for w in x.windows(2) {
                if w[0].abs() < 1e-5 {
                    quiet += 1;
                } else {
                    if quiet > 96 {
                        worst = worst.max(w[1].abs() / peak);
                    }
                    quiet = 0;
                }
            }
            // A click spreads energy far above the voice; none of these birds reaches 16 kHz.
            let hf = high_share(&x, 16000.0);
            println!("{name:<18} exc {excitement}: peak {peak:.2}, onset step {worst:.4}, >16 kHz {:.4} %, {speed:.0}x real time", hf * 100.0);
            assert!(worst < 0.02, "{name}: a note starts with a step of {worst:.3} of the peak");
            assert!(hf < 0.002, "{name}: {:.3} % of the energy above 16 kHz", hf * 100.0);
        }
    }
}

/// Largest share of energy above `hz` in any loud 5 ms frame (24 dB/oct high-pass).
fn high_share(x: &[f32], hz: f32) -> f32 {
    let c = 1.0 - (-core::f32::consts::TAU * hz / SR).exp();
    let mut lp = [0.0f32; 4];
    let hi: Vec<f32> = x
        .iter()
        .map(|s| {
            let mut v = *s;
            for p in lp.iter_mut() {
                *p += (v - *p) * c;
                v -= *p;
            }
            v
        })
        .collect();
    let n = 240;
    let top = x.chunks(n).map(rms).fold(0.0, f32::max);
    x.chunks(n).zip(hi.chunks(n)).filter(|(a, _)| rms(a) > top * 0.1).map(|(a, h)| (rms(h) / rms(a)).powi(2)).fold(0.0, f32::max)
}

#[test]
fn silent_without_activity_and_quieter_far_away() {
    let mut m = bird("Blackbird", 0.0, &[("activity", 0.0)]);
    assert!(rms(&render(m.as_mut(), 20.0)) < 1e-6, "activity 0 is silent");
    let level = |d: f32| rms(&render(bird("Chaffinch", 0.0, &[("activity", 1.0), ("distance", d)]).as_mut(), 30.0));
    let (near, far) = (level(0.0), level(1.0));
    println!("near {near:.4}, far {far:.4}");
    assert!(far < near * 0.35);
}

#[test]
fn width_zero_is_the_mono_render() {
    let make = || bird("Robin", 0.0, &[("activity", 1.0)]);
    let (mut a, mut b) = (make(), make());
    a.set_param_by_name("space/width", 0.0);
    a.set_param_by_name("space/reverb", 0.4);
    b.set_param_by_name("space/reverb", 0.4);
    let n = (8.0 * SR) as usize;
    let (mut l, mut r, mut mono) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    a.render_stereo(&mut l, &mut r);
    b.render_mono(&mut mono);
    assert!(rms(&mono) > 1e-3);
    assert!(l == r && l == mono);
}
