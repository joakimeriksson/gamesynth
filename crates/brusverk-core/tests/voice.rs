//! Voices (`babble`, `creature_*`): text in, the same text out; voices that differ; mood in the
//! melody; size in the pitch; rough growls; bounded and click-free.
use brusverk_core::generators::{self, voice};
use brusverk_core::render::{peak, rms};
use brusverk_core::Model;

const SR: f32 = 48000.0;
const CALLS: [&str; 6] = ["creature_growl", "creature_roar", "creature_hiss", "creature_squeak", "creature_chirp", "creature_chatter"];

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

fn babble(preset: &str) -> Box<dyn Model> {
    let mut m = generators::create("babble", SR).unwrap();
    if !preset.is_empty() {
        let i = m.desc().preset_index(preset).unwrap_or_else(|| panic!("no preset {preset}"));
        m.load_preset(i);
    }
    m
}

fn set(m: &mut dyn Model, param: &str, v: f32) {
    assert!(m.set_param_by_name(param, v), "no param {param}");
}

/// Type `text` into `m` at `cps` characters a second; returns the audio and each character's
/// start (sample index).
fn say(m: &mut dyn Model, text: &str, cps: f32) -> (Vec<f32>, Vec<usize>) {
    let step = (SR / cps) as usize;
    let mut out = Vec::new();
    let mut starts = Vec::new();
    for c in text.chars() {
        m.set_input_by_name("letter", voice::letter_value(c));
        m.trigger();
        starts.push(out.len());
        let mut buf = vec![0.0; step];
        m.render_mono(&mut buf);
        out.extend_from_slice(&buf);
    }
    out.extend(render(m, 1.0));
    (out, starts)
}

/// Pitch (Hz) of `x` between `lo` and `hi` Hz by YIN (cumulative-mean-normalised difference),
/// and how periodic it is (0..1, 1 = every period identical).
fn pitch(x: &[f32], lo: f32, hi: f32) -> (f32, f32) {
    let (a, b) = ((SR / hi) as usize, (SR / lo) as usize);
    let n = x.len() - b;
    let d: Vec<f64> = (0..=b)
        .map(|lag| (0..n).map(|i| (x[i] as f64 - x[i + lag] as f64).powi(2)).sum::<f64>())
        .collect();
    let mut cm = vec![1.0f64; b + 1];
    let mut sum = 0.0;
    for t in 1..=b {
        sum += d[t];
        cm[t] = d[t] * t as f64 / sum.max(1e-20);
    }
    let range = &cm[a..=b];
    let k = match range.iter().position(|v| *v < 0.3) {
        Some(mut k) => {
            while k + 1 < range.len() && range[k + 1] < range[k] {
                k += 1;
            }
            k
        }
        None => (0..range.len()).min_by(|&i, &j| range[i].total_cmp(&range[j])).unwrap(),
    };
    (SR / (a + k) as f32, (1.0 - range[k]).clamp(0.0, 1.0) as f32)
}

/// Spectral centroid (Hz) by a crude DFT scan of 64 bands.
fn centroid(x: &[f32]) -> f32 {
    let (mut num, mut den) = (0.0f32, 0.0f32);
    let n = x.len().min(8192);
    for k in 1..64 {
        let f = k as f32 * 250.0;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, v) in x[..n].iter().enumerate() {
            let a = core::f32::consts::TAU * f * i as f32 / SR;
            re += v * a.cos();
            im += v * a.sin();
        }
        let p = re * re + im * im;
        num += f * p;
        den += p;
    }
    num / den.max(1e-20)
}

/// Window of `len` seconds starting `at` seconds into `x`.
fn win(x: &[f32], at: f32, len: f32) -> &[f32] {
    let a = (at * SR) as usize;
    &x[a..(a + (len * SR) as usize).min(x.len())]
}

fn semitones(a: f32, b: f32) -> f32 {
    12.0 * (b / a).log2()
}

// ---------------------------------------------------------------------------------------------
// Text in
// ---------------------------------------------------------------------------------------------

#[test]
fn letters_travel_exactly_through_the_input() {
    for c in (32u8..127).map(char::from) {
        let v = voice::letter_value(c);
        assert!((0.0..=1.0).contains(&v));
        assert_eq!(voice::letter_code(v), c as u32, "{c:?}");
        // The value survives the f32 input (k / 128 is exact).
        let mut m = babble("");
        let i = m.desc().input_index("letter").unwrap();
        m.set_input(i, v);
        assert_eq!(voice::letter_code(m.input(i)), c as u32);
    }
    use voice::{glyph, Glyph, Onset};
    assert!(matches!(glyph('a' as u32), Glyph::Syllable { onset: Onset::Open, vowel: Some(_) }));
    assert_eq!(glyph('A' as u32), glyph('a' as u32));
    assert!(matches!(glyph('s' as u32), Glyph::Syllable { onset: Onset::Fricative { .. }, vowel: None }));
    assert!(matches!(glyph('m' as u32), Glyph::Syllable { onset: Onset::Nasal, .. }));
    assert_eq!(glyph(' ' as u32), Glyph::Space);
    assert_eq!(glyph('?' as u32), Glyph::Question);
    assert_eq!(glyph('"' as u32), Glyph::Silent);
    // Every other value (digits, hashes, the top of the range) still speaks.
    for code in [0, 7, '5' as u32, 127, 128] {
        assert!(matches!(glyph(code), Glyph::Syllable { .. }), "code {code}");
    }
}

#[test]
fn spaces_and_punctuation_are_silent() {
    let mut m = babble("");
    for c in [' ', ',', '.', '"', '-'] {
        m.set_input_by_name("letter", voice::letter_value(c));
        m.trigger();
        assert!(render(m.as_mut(), 0.2).iter().all(|x| *x == 0.0), "{c:?} made a sound");
        assert!(m.is_finished());
    }
}

#[test]
fn the_same_text_gives_the_same_babble() {
    let text = "Hello, friend! Nice day.";
    // Without variation, exactly the same samples, every time (fresh or reused voice).
    let mut a = babble("Critter");
    set(a.as_mut(), "shape/variation", 0.0);
    let (x, _) = say(a.as_mut(), text, 14.0);
    let (y, _) = say(a.as_mut(), text, 14.0);
    let mut b = babble("Critter");
    set(b.as_mut(), "shape/variation", 0.0);
    let (z, _) = say(b.as_mut(), text, 14.0);
    assert!(x == y && x == z, "the same text must render the same samples");
    assert!(rms(&x) > 0.01);
    // With variation each syllable still sits on the same pitch, within half a semitone.
    let mut m = babble("");
    set(m.as_mut(), "voice/speed", 0.5);
    let (p, s) = say(m.as_mut(), "aeiou", 6.0);
    let (q, t) = say(m.as_mut(), "aeiou", 6.0);
    for k in 0..5 {
        let at = |x: &[f32], s: usize| pitch(&x[s + 2400..s + 7200], 70.0, 600.0).0;
        let d = semitones(at(&p, s[k]), at(&q, t[k]));
        assert!(d.abs() < 0.6, "syllable {k} moved {d:.2} semitones between two readings");
    }
}

/// The pitch of each vowel in "aeiou" spoken slowly by `m`.
fn melody(m: &mut dyn Model) -> Vec<f32> {
    set(m, "shape/variation", 0.0);
    set(m, "voice/speed", 0.5);
    let (x, s) = say(m, "aoeiu", 6.0);
    s.iter().map(|&i| pitch(&x[i + 2400..i + 7200], 60.0, 900.0).0).collect()
}

#[test]
fn every_character_has_its_own_voice() {
    // Same settings, another seed: another melody for the same text.
    let a = melody(babble("").as_mut());
    let mut m = babble("");
    set(m.as_mut(), "voice/seed", 17.0);
    let b = melody(m.as_mut());
    let moved = a.iter().zip(&b).filter(|(x, y)| semitones(**x, **y).abs() > 0.7).count();
    assert!(moved >= 2, "seed 17 should change the melody: {a:?} vs {b:?}");
    // The presets are clearly different voices: register and timbre.
    let mut seen: Vec<(String, f32, f32)> = Vec::new();
    for preset in ["Critter", "Kid", "Robot", "Gruff", "Elder", "Ghost", "Giant", "Default"] {
        let mut m = babble(if preset == "Default" { "" } else { preset });
        set(m.as_mut(), "shape/variation", 0.0);
        set(m.as_mut(), "voice/speed", 0.5);
        let (x, s) = say(m.as_mut(), "aoaeo", 5.0);
        let mut fs: Vec<f32> = s.iter().map(|&i| pitch(&x[i + 2400..i + 7200], 40.0, 900.0).0).collect();
        fs.sort_by(f32::total_cmp);
        let f = fs[2];
        let (x, _) = say(m.as_mut(), "We went on a walk.", 12.0);
        let c = centroid(win(&x, 0.0, 1.2));
        for (other, g, d) in &seen {
            let apart = semitones(*g, f).abs() > 2.0 || (c / d).ln().abs() > 0.2;
            assert!(apart, "{preset} ({f:.0} Hz, centroid {c:.0}) sounds like {other} ({g:.0} Hz, {d:.0})");
        }
        seen.push((preset.into(), f, c));
    }
    let f = |name: &str| seen.iter().find(|s| s.0 == name).unwrap().1;
    assert!(f("Critter") > 1.8 * f("Gruff") && f("Giant") < f("Gruff") * 1.05, "critters are high, giants low");
}

#[test]
fn mood_shifts_the_melody() {
    // One long vowel in each mood: (mean pitch, rise over the syllable in semitones).
    let read = |mood: f32, anger: f32| {
        let mut m = babble("");
        set(m.as_mut(), "shape/variation", 0.0);
        set(m.as_mut(), "space/room", 0.0);
        set(m.as_mut(), "voice/speed", 0.5);
        m.set_input_by_name("mood", mood);
        m.set_input_by_name("anger", anger);
        let (x, _) = say(m.as_mut(), "a", 4.0);
        let start = pitch(win(&x, 0.035, 0.05), 70.0, 500.0).0;
        let end = pitch(win(&x, 0.17, 0.05), 70.0, 500.0).0;
        let mid = pitch(win(&x, 0.09, 0.05), 70.0, 500.0).0;
        (mid, semitones(start, end), rms(&x))
    };
    let (happy, neutral, sad) = (read(1.0, 0.0), read(0.5, 0.0), read(0.0, 0.0));
    assert!(happy.1 > 1.0, "happy rises: {:.2} st", happy.1);
    assert!(sad.1 < -1.5, "sad falls: {:.2} st", sad.1);
    assert!(happy.1 > neutral.1 && neutral.1 > sad.1, "contours {:.2} > {:.2} > {:.2}", happy.1, neutral.1, sad.1);
    assert!(semitones(neutral.0, happy.0) > 2.0 && semitones(neutral.0, sad.0) < -2.0, "register {:.0} / {:.0} / {:.0} Hz", happy.0, neutral.0, sad.0);
    // Anger: louder and rough (less periodic).
    let angry = read(0.5, 1.0);
    assert!(angry.2 > neutral.2 * 1.2, "anger is louder: {:.3} vs {:.3}", angry.2, neutral.2);
    // A question lifts the last syllable; a full stop drops it.
    let end_pitch = |text: &str| {
        let mut m = babble("");
        set(m.as_mut(), "shape/variation", 0.0);
        set(m.as_mut(), "voice/speed", 0.5);
        let (x, s) = say(m.as_mut(), text, 8.0);
        pitch(&x[s[1] + 4800..s[1] + 7200], 60.0, 900.0).0
    };
    let (q, stop) = (end_pitch("a?"), end_pitch("a."));
    assert!(semitones(stop, q) > 2.0, "'?' should rise above '.': {q:.0} vs {stop:.0} Hz");
}

#[test]
fn babble_follows_the_typing_and_dips_between_letters() {
    // At 14 letters a second each letter is its own beat: the level dips between them.
    let mut m = babble("");
    set(m.as_mut(), "shape/variation", 0.0);
    let (x, s) = say(m.as_mut(), "aaaaaaaa", 14.0);
    let level = |a: usize, b: usize| rms(&x[a..b]);
    for k in 1..7 {
        let mid = level(s[k] + 600, s[k] + 1800);
        let edge = level(s[k + 1] - 500, s[k + 1]);
        assert!(edge < mid * 0.8, "letter {k}: no dip before the next ({edge:.4} vs {mid:.4})");
    }
}

// ---------------------------------------------------------------------------------------------
// Creatures
// ---------------------------------------------------------------------------------------------

fn creature(name: &str, size: f32, aggression: f32, excitement: f32) -> Vec<f32> {
    let mut m = generators::create(name, SR).unwrap();
    set(m.as_mut(), "shape/variation", 0.0);
    set(m.as_mut(), "space/room", 0.0);
    m.set_input_by_name("size", size);
    m.set_input_by_name("aggression", aggression);
    m.set_input_by_name("excitement", excitement);
    m.trigger();
    let secs = m.length_secs().unwrap();
    render(m.as_mut(), secs)
}

#[test]
fn a_bigger_creature_is_lower() {
    for name in CALLS {
        let small = creature(name, 0.2, 0.5, 0.5);
        let big = creature(name, 0.8, 0.5, 0.5);
        let (cs, cb) = (centroid(&small), centroid(&big));
        assert!(cb < cs * 0.7, "{name}: centroid big {cb:.0} Hz vs small {cs:.0} Hz");
        assert!(big.len() >= small.len(), "{name}: a bigger creature calls at least as long");
    }
    // The voiced calls: the pitch itself drops (about an octave from 0.2 to 0.8).
    for (name, lo, hi) in [("creature_growl", 25.0, 400.0), ("creature_roar", 50.0, 800.0)] {
        let f = |size: f32| {
            let x = creature(name, size, 0.0, 0.5);
            // A third of the way through the sound itself (the reported length is a bound).
            let top = peak(&x);
            let n = x.iter().rposition(|v| v.abs() > 0.01 * top).unwrap();
            pitch(&x[n / 3..n / 3 + 9600], lo, hi).0
        };
        let (s, b) = (f(0.2), f(0.8));
        assert!(semitones(s, b) < -8.0, "{name}: pitch {s:.0} Hz -> {b:.0} Hz");
    }
    // Tones: a mouse squeaks at 4..5 kHz, a big one far lower.
    let zc = |x: &[f32]| x.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f32 / (2.0 * x.len() as f32 / SR);
    let mouse = creature("creature_squeak", 0.0, 0.5, 0.5);
    let core = &mouse[(0.03 * SR) as usize..(0.08 * SR) as usize];
    let f = zc(core);
    assert!((3500.0..7500.0).contains(&f), "mouse squeak at {f:.0} Hz");
}

#[test]
fn growls_are_rough() {
    // A furious, rough growl is far less periodic than a calm, smooth one (subharmonics and
    // jitter), and both less than a clean babble vowel.
    let periodicity = |x: &[f32]| {
        let n = x.len();
        let mut acc = 0.0;
        for k in 0..4 {
            let a = n / 5 + k * 2400;
            acc += pitch(&x[a..a + 4800], 40.0, 400.0).1;
        }
        acc / 4.0
    };
    let growl = |rough: f32, aggression: f32| {
        let mut m = generators::create("creature_growl", SR).unwrap();
        set(m.as_mut(), "shape/variation", 0.0);
        set(m.as_mut(), "space/room", 0.0);
        set(m.as_mut(), "voice/roughness", rough);
        m.set_input_by_name("aggression", aggression);
        m.set_input_by_name("excitement", 0.0);
        m.trigger();
        let s = m.length_secs().unwrap();
        render(m.as_mut(), s)
    };
    let (smooth, rough) = (periodicity(&growl(0.0, 0.0)), periodicity(&growl(1.0, 1.0)));
    assert!(rough < smooth - 0.1, "rough growl periodicity {rough:.2} vs smooth {smooth:.2}");
    // The subharmonic: a rough growl's envelope beats at f0 / 3 (25..40 Hz at wolf size), as
    // the recorded growls do (28 Hz).
    let x = growl(1.0, 1.0);
    let env: Vec<f32> = x.chunks(48).map(rms).collect();
    let n = env.len();
    let body = &env[n / 5..4 * n / 5];
    let mean = body.iter().sum::<f32>() / body.len() as f32;
    let e: Vec<f32> = body.iter().map(|v| v - mean).collect();
    let beat = |hz: f32| {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in e.iter().enumerate() {
            let a = core::f32::consts::TAU * hz * i as f32 / 1000.0;
            re += v * a.cos();
            im += v * a.sin();
        }
        re * re + im * im
    };
    let best = (10..80).map(|h| (beat(h as f32), h)).fold((0.0, 0), |a, b| if b.0 > a.0 { b } else { a }).1;
    assert!((22..=45).contains(&best), "growl rattle at {best} Hz");
}

#[test]
fn excitement_and_aggression_change_the_call() {
    // More excitement: more chirps and squeaks in the series.
    let count = |x: &[f32]| {
        let env: Vec<f32> = x.chunks(240).map(rms).collect();
        let top = env.iter().cloned().fold(0.0, f32::max);
        env.windows(2).filter(|w| w[0] < 0.2 * top && w[1] >= 0.2 * top).count()
    };
    for name in ["creature_chirp", "creature_squeak", "creature_chatter"] {
        let (calm, wild) = (count(&creature(name, 0.3, 0.5, 0.0)), count(&creature(name, 0.3, 0.5, 1.0)));
        assert!(wild > calm, "{name}: {calm} syllables calm, {wild} excited");
    }
    // Aggression: louder and brighter.
    for name in ["creature_growl", "creature_roar", "creature_hiss"] {
        let (calm, mad) = (creature(name, 0.5, 0.0, 0.5), creature(name, 0.5, 1.0, 0.5));
        assert!(rms(&mad) > rms(&calm) * 1.2, "{name}: aggression should be louder");
        assert!(centroid(&mad) > centroid(&calm), "{name}: aggression should be brighter");
    }
}

#[test]
fn idle_breathes_and_purrs() {
    let idle = |exc: f32, purr: f32| {
        let mut m = generators::create("creature_idle", SR).unwrap();
        m.set_input_by_name("excitement", exc);
        m.set_input_by_name("purr", purr);
        m.set_input_by_name("size", 0.25);
        m.snap();
        render(m.as_mut(), 6.0)
    };
    // The purr is a pulse train at 25..30 Hz (recorded cats: 25..29 Hz).
    let x = idle(0.1, 1.0);
    let seg = &x[(1.0 * SR) as usize..(1.6 * SR) as usize];
    let env: Vec<f32> = seg.chunks(48).map(rms).collect();
    let mean = env.iter().sum::<f32>() / env.len() as f32;
    let e: Vec<f32> = env.iter().map(|v| v - mean).collect();
    let beat = |hz: f32| {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in e.iter().enumerate() {
            let a = core::f32::consts::TAU * hz * i as f32 / 1000.0;
            re += v * a.cos();
            im += v * a.sin();
        }
        re * re + im * im
    };
    let best = (10..60).map(|h| (beat(h as f32), h)).fold((0.0, 0), |a, b| if b.0 > a.0 { b } else { a }).1;
    assert!((24..=30).contains(&best), "purr pulses at {best} Hz");
    // Panting is louder and quicker than resting breath.
    let breaths = |x: &[f32]| {
        let env: Vec<f32> = x.chunks(4800).map(rms).collect();
        let top = env.iter().cloned().fold(0.0, f32::max);
        env.windows(2).filter(|w| w[0] < 0.4 * top && w[1] >= 0.4 * top).count()
    };
    let (rest, pant) = (idle(0.0, 0.0), idle(1.0, 0.0));
    assert!(rms(&pant) > 2.0 * rms(&rest), "panting {:.4} vs resting {:.4}", rms(&pant), rms(&rest));
    assert!(breaths(&pant) > breaths(&rest) + 3, "panting {} vs resting {} breaths in 6 s", breaths(&pant), breaths(&rest));
    assert!(peak(&x) <= 1.0 && x.iter().all(|v| v.is_finite()));
}

// ---------------------------------------------------------------------------------------------
// Safety
// ---------------------------------------------------------------------------------------------

/// Largest second difference of `x`: a click (a step) shows up as its full size, a tone of
/// amplitude A at f Hz only as A (2 pi f / SR)^2.
fn roughest(x: &[f32]) -> f32 {
    x.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).abs()).fold(0.0, f32::max)
}

#[test]
fn everything_is_bounded_and_click_free() {
    let mut rng = brusverk_core::Rng::new(7);
    // Every preset of every voice, random inputs, overlapping retriggers.
    for name in generators::NAMES.iter().copied().filter(|n| *n == "babble" || n.starts_with("creature_")) {
        let mut m = generators::create(name, SR).unwrap();
        for preset in 0..m.desc().presets.len() {
            m.load_preset(preset);
            let label = format!("{name}/{}", m.desc().presets[preset].name);
            for _ in 0..6 {
                for i in 0..m.desc().inputs.len() {
                    m.set_input(i, rng.next_f32());
                }
                m.trigger();
                let x = render(m.as_mut(), rng.range(0.03, 0.4));
                assert!(x.iter().all(|v| v.is_finite()), "{label}: NaN");
                assert!(peak(&x) <= 1.0, "{label}: peak {}", peak(&x));
            }
            let tail = render(m.as_mut(), 6.0);
            assert!(tail.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "{label}: tail");
            if m.desc().one_shot {
                assert!(m.is_finished(), "{label}: still sounding 6 s after its last trigger");
                assert!(tail[tail.len() - 4800..].iter().all(|v| *v == 0.0), "{label}: does not end in silence");
            }
        }
    }
    // Onsets start from silence, and legato retriggers (vowels typed fast) do not click.
    for preset in ["", "Critter", "Robot", "Gruff", "Elder", "Ghost", "Giant"] {
        let mut m = babble(preset);
        let (x, s) = say(m.as_mut(), "aoaeiuoa", 20.0);
        let first = peak(&x[..48]);
        assert!(first < 0.05 * peak(&x), "{preset}: the first millisecond jumps to {first:.3}");
        let r = roughest(&x);
        assert!(r < 0.12, "{preset}: a click (second difference {r:.3}), retriggers at {s:?}");
    }
    for name in CALLS {
        for (size, aggr) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)] {
            let x = creature(name, size, aggr, 0.5);
            // The call ramps in (no instant onset): its first millisecond stays under a quarter
            // of its peak.
            let early = peak(&x[..48]) / peak(&x).max(1e-6);
            assert!(early < 0.25, "{name} size {size} aggression {aggr}: {early:.2} of the peak within 1 ms");
            if name == "creature_growl" || name == "creature_roar" {
                // Low voices: no step anywhere (a step shows in the second difference).
                let r = roughest(&x) / peak(&x).max(1e-3);
                assert!(r < 0.35, "{name} size {size} aggression {aggr}: a click (second difference {r:.3} of peak)");
            }
        }
    }
}

#[test]
fn pitch_ratio_transposes_the_voice() {
    let f = |ratio: f32| {
        let mut m = babble("");
        set(m.as_mut(), "shape/variation", 0.0);
        set(m.as_mut(), "voice/speed", 0.5);
        m.set_pitch_ratio(ratio);
        let (x, _) = say(m.as_mut(), "a", 4.0);
        pitch(win(&x, 0.06, 0.08), 60.0, 900.0).0
    };
    let d = semitones(f(1.0), f(2.0));
    assert!((d - 12.0).abs() < 1.0, "an octave up moved {d:.2} semitones");
}
