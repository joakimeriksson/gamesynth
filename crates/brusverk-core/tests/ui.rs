//! The UI kit (`ui_*`): one shared table, short and click-free, in tune, game state on the scale.
use brusverk_core::generators::{self, ui};
use brusverk_core::render::{peak, rms};
use brusverk_core::Model;

const SR: f32 = 48000.0;
const STYLES: [&str; 5] = ["Soft", "Wooden", "Glass", "Retro", "Sci-fi"];

fn ui_names() -> Vec<&'static str> {
    generators::NAMES.iter().copied().filter(|n| n.starts_with("ui_")).collect()
}

/// A UI event in `style`, in `key` (0 = C) and `scale` (index into `ui::SCALE_NAMES`).
fn kit(event: &str, style: &str, key: usize, scale: usize, variation: f32) -> Box<dyn Model> {
    let mut m = generators::create(event, SR).unwrap_or_else(|| panic!("no {event}"));
    let i = m.desc().preset_index(style).unwrap_or_else(|| panic!("{event} has no preset {style}"));
    m.load_preset(i);
    assert!(m.set_param_by_name("music/key", key as f32));
    assert!(m.set_param_by_name("music/scale", scale as f32));
    assert!(m.set_param_by_name("shape/variation", variation));
    m
}

/// The same, dry: pitch is measured on the notes, not on the room's modes.
fn dry(event: &str, style: &str, key: usize, scale: usize) -> Box<dyn Model> {
    let mut m = kit(event, style, key, scale, 0.0);
    assert!(m.set_param_by_name("space/amount", 0.0));
    m
}

/// When a note's pitch has settled: after Sci-fi's glide into it, else just after the strike.
fn settled(style: &str) -> f32 {
    if style == "Sci-fi" {
        0.02
    } else {
        0.006
    }
}

/// How exactly a short window can place a note: Sci-fi's chorus (a quieter voice 10 cents up)
/// beats, and the reading wanders by a few cents with it.
fn tolerance(style: &str) -> f32 {
    if style == "Sci-fi" {
        0.005
    } else {
        0.002
    }
}

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

/// Fire once and render the reported length.
fn fire(m: &mut dyn Model) -> Vec<f32> {
    m.trigger();
    let secs = m.length_secs().unwrap();
    render(m, secs)
}

fn params_of(m: &dyn Model) -> ui::UiParams {
    let mut p = ui::UiParams::default();
    for (i, d) in m.desc().params.iter().enumerate() {
        let id = ui::UiParamId::from_name(&d.name).unwrap();
        p.set(id, m.param(i));
    }
    p
}

/// YIN pitch of `x` (`x.len() >= FRAME`): (Hz, aperiodicity 0 = perfectly periodic). It only
/// has to find the period (the pitch is then measured on the spectrum), so it runs on the frame
/// averaged down to 24 kHz.
fn yin(x: &[f32]) -> (f32, f32) {
    const W: usize = 384;
    const TAU_MIN: usize = 5; // 4.8 kHz
    const TAU_MAX: usize = 170; // 141 Hz
    let half = SR / 2.0;
    let y: Vec<f32> = x[..FRAME].chunks(2).map(|p| 0.5 * (p[0] + p[1])).collect();
    let mut d = [0.0f32; TAU_MAX + 2];
    for (tau, dt) in d.iter_mut().enumerate().skip(1) {
        let mut acc = 0.0;
        for i in 0..W {
            let e = y[i] - y[i + tau];
            acc += e * e;
        }
        *dt = acc;
    }
    // Cumulative mean normalised difference.
    let mut cmnd = [1.0f32; TAU_MAX + 2];
    let mut sum = 0.0;
    for tau in 1..TAU_MAX + 2 {
        sum += d[tau];
        cmnd[tau] = d[tau] * tau as f32 / sum.max(1e-20);
    }
    let mut best = (TAU_MIN..=TAU_MAX).min_by(|&a, &b| cmnd[a].total_cmp(&cmnd[b])).unwrap();
    // The first dip under the threshold is the period (later dips are its multiples).
    if let Some(t) = (TAU_MIN..=TAU_MAX).find(|&t| cmnd[t] < 0.1) {
        let mut t = t;
        while t < TAU_MAX && cmnd[t + 1] < cmnd[t] {
            t += 1;
        }
        best = t;
    }
    let (a, b, c) = (cmnd[best - 1], cmnd[best], cmnd[best + 1]);
    let shift = if a + c - 2.0 * b > 0.0 { 0.5 * (a - c) / (a + c - 2.0 * b) } else { 0.0 };
    (half / (best as f32 + shift), b)
}

const FRAME: usize = 768 + 342;

/// Exact frequency of the strongest partial within 60 cents of `guess` in the `secs` from `t`:
/// a Hann-windowed DTFT scanned in 2-cent steps, refined by a parabola.
fn precise_pitch_over(x: &[f32], t: f32, secs: f32, guess: f32) -> f32 {
    nearest_partial(x, t, secs, guess).unwrap_or_else(|| panic!("no partial within 60 cents of {guess:.1} Hz")).0
}

/// The partial `precise_pitch_over` measures and its share of the window: its amplitude over
/// the amplitude a lone sine with the window's rms would have (1 for a lone note, about 0.5 for
/// each note of a four-note chord). None when the strongest level in the search is at its edge.
fn nearest_partial(x: &[f32], t: f32, secs: f32, guess: f32) -> Option<(f32, f32)> {
    let s = (t * SR) as usize;
    let w = &x[s..(s + (secs * SR) as usize).min(x.len())];
    let n = w.len() as f32;
    let windowed: Vec<f32> = w.iter().enumerate().map(|(i, v)| v * (0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n).cos())).collect();
    let mag = |hz: f32| {
        // Rotating phasor: one complex multiply per sample.
        let a = core::f32::consts::TAU * hz / SR;
        let (c, sn) = (a.cos() as f64, a.sin() as f64);
        let (mut zr, mut zi, mut re, mut im) = (1.0f64, 0.0f64, 0.0f64, 0.0f64);
        for v in &windowed {
            re += *v as f64 * zr;
            im += *v as f64 * zi;
            (zr, zi) = (zr * c - zi * sn, zr * sn + zi * c);
        }
        (re * re + im * im).sqrt() as f32
    };
    let at = |c: i32| guess * (c as f32 / 1200.0).exp2();
    // Coarse (10 cents) then fine (2 cents) around the coarse maximum; m holds the fine scan,
    // index k = 2-cent steps from -60.
    let coarse = (-6..=6).max_by(|&a, &b| mag(at(10 * a)).total_cmp(&mag(at(10 * b)))).unwrap();
    let mut m = vec![0.0f32; 61];
    for j in (5 * coarse + 30 - 6).max(0)..=(5 * coarse + 30 + 6).min(60) {
        m[j as usize] = mag(at(2 * (j - 30)));
    }
    let k = (0..m.len()).max_by(|&a, &b| m[a].total_cmp(&m[b])).unwrap();
    // A sine of amplitude A gives |X| = A n / 4 and a window-weighted rms of A / sqrt 2.
    let amplitude = 4.0 * m[k] / n;
    let hann_sq: f32 = (0..w.len()).map(|i| (0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n).cos()).powi(2)).sum();
    let weighted_rms = (windowed.iter().map(|v| v * v).sum::<f32>() / hann_sq).sqrt();
    if k == 0 || k + 1 == m.len() {
        return None;
    }
    let (a, b, c) = (m[k - 1], m[k], m[k + 1]);
    let shift = 0.5 * (a - c) / (a + c - 2.0 * b);
    Some((guess * ((2.0 * (k as f32 - 30.0 + shift)) / 1200.0).exp2(), amplitude / (std::f32::consts::SQRT_2 * weighted_rms).max(1e-12)))
}

/// Over 40 ms, or for Sci-fi over a whole beat of its chorus.
fn precise_pitch(x: &[f32], t: f32, guess: f32, style: &str) -> f32 {
    precise_pitch_over(x, t, if style == "Sci-fi" { 0.1 } else { 0.04 }, guess)
}

/// Level of `hz` in the 15 ms from `t` (Hann-windowed).
fn level_at(x: &[f32], t: f32, hz: f32) -> f32 {
    let s = (t * SR) as usize;
    let w = &x[s..(s + (0.015 * SR) as usize).min(x.len())];
    let n = w.len() as f32;
    let a = core::f32::consts::TAU * hz / SR;
    let (mut re, mut im) = (0.0f32, 0.0f32);
    for (i, v) in w.iter().enumerate() {
        let h = 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / n).cos();
        re += v * h * (a * i as f32).cos();
        im += v * h * (a * i as f32).sin();
    }
    (re * re + im * im).sqrt()
}

/// When each of `notes` (Hz) comes in: the first time, scanning the first 0.6 s in 2 ms steps,
/// that its level reaches half its maximum.
fn note_times(x: &[f32], notes: &[f32]) -> Vec<f32> {
    notes
        .iter()
        .map(|&hz| {
            let levels: Vec<f32> = (0..300).map(|k| level_at(x, k as f32 * 0.002, hz)).collect();
            let top = levels.iter().copied().fold(0.0, f32::max);
            levels.iter().position(|l| *l >= 0.5 * top).unwrap() as f32 * 0.002
        })
        .collect()
}

/// Pitches of every clearly pitched, audible frame (hop 5 ms), as (time, Hz). YIN finds the
/// period; the pitch reported is the strongest real partial at it or its first harmonics,
/// measured on the frame (a quarter share at least), because for a chord YIN finds the notes'
/// common fundamental, or settles between them. A note holds its pitch from one
/// frame to the next; a frame that does not agree with both neighbours is noise (a strike, a
/// swish) that YIN happened to find a period in, or straddles two notes, and is dropped.
fn pitched_frames(x: &[f32]) -> Vec<(f32, f32)> {
    let strongest = |t: f32, f0: f32| {
        let partial = |k: usize| nearest_partial(x, t, FRAME as f32 / SR, f0 * k as f32).filter(|&(_, share)| share >= 0.25);
        // Usually the period is a note; only for a chord look among its harmonics.
        partial(1).or_else(|| (2..=4).filter_map(partial).max_by(|a, b| a.1.total_cmp(&b.1))).map(|(hz, _)| (t, hz))
    };
    let raw: Vec<(f32, f32)> = raw_pitched_frames(x).into_iter().filter_map(|(t, hz)| strongest(t, hz)).collect();
    let agree = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0 - 0.005).abs() < 1e-3 && (b.1 / a.1 - 1.0).abs() < 0.03;
    (0..raw.len())
        .filter(|&i| i > 0 && i + 1 < raw.len() && agree(raw[i - 1], raw[i]) && agree(raw[i], raw[i + 1]))
        .map(|i| raw[i])
        .collect()
}

fn raw_pitched_frames(x: &[f32]) -> Vec<(f32, f32)> {
    let hop = (0.005 * SR) as usize;
    let loud = peak(x) * 0.01;
    let mut out = Vec::new();
    let mut s = 0;
    while s + FRAME <= x.len() {
        let w = &x[s..s + FRAME];
        if peak(w) > loud {
            let (hz, ap) = yin(w);
            if ap < 0.1 {
                out.push((s as f32 / SR, hz));
            }
        }
        s += hop;
    }
    out
}

/// Cents from `hz` to the nearest note of `key`/`scale`; the pitch class must be in the scale.
fn cents_off_scale(hz: f32, key: usize, scale: usize) -> f32 {
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let nearest = midi.round();
    let class = (nearest as i32 - key as i32).rem_euclid(12);
    if !ui::SCALES[scale].contains(&class) {
        return f32::INFINITY;
    }
    100.0 * (midi - nearest)
}

/// RMS frequency: rms of the derivative over rms of the signal (a brightness that rises with
/// both pitch and overtones).
fn rms_hz(x: &[f32]) -> f32 {
    let d: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
    SR / core::f32::consts::TAU * rms(&d) / rms(x).max(1e-12)
}

#[test]
fn the_kit_shares_one_table_and_the_style_presets() {
    let names = ui_names();
    assert_eq!(names.len(), 19, "{names:?}");
    let first = generators::create(names[0], SR).unwrap();
    let params: Vec<_> = first.desc().params.iter().map(|p| p.name.clone()).collect();
    let presets: Vec<_> = first.desc().presets.iter().map(|p| p.name.clone()).collect();
    assert_eq!(presets, ["Default", "Soft", "Wooden", "Glass", "Retro", "Sci-fi"]);
    for name in &names {
        let m = generators::create(name, SR).unwrap();
        let d = m.desc();
        assert_eq!(d.category, "ui");
        assert!(d.one_shot && m.transposes(), "{name}");
        let inputs: Vec<_> = d.inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(inputs[..3], ["power", "distance", "combo"], "{name}");
        assert_eq!(d.params.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), params, "{name}");
        assert_eq!(d.presets.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), presets, "{name}");
        // A style preset only picks the style: everything else is the default.
        let k = d.preset_index("Glass").unwrap();
        let style = d.param_index("kit/style").unwrap();
        for (i, (v, def)) in d.presets[k].values.iter().zip(&d.presets[0].values).enumerate() {
            assert!(i == style || v == def, "{name}: Glass changes {}", d.params[i].name);
        }
        assert_eq!(d.presets[k].values[style], 2.0);
    }
    assert!(generators::create("ui_slider", SR).unwrap().desc().input_index("value").is_some());
    assert!(generators::create("ui_type", SR).unwrap().desc().input_index("value").is_some());
    assert!(generators::create("ui_coin", SR).unwrap().desc().input_index("count").is_some());
}

#[test]
fn every_ui_sound_is_short_and_click_free() {
    for style in STYLES {
        for name in ui_names() {
            let mut m = kit(name, style, 0, 0, 0.5);
            let label = format!("{name}/{style}");
            m.trigger();
            let length = m.length_secs().unwrap();
            let x = render(m.as_mut(), length + 0.2);
            assert!(x.iter().all(|v| v.is_finite()), "{label}");
            assert!(m.is_finished(), "{label} still sounding after {length:.2} s");
            let pk = peak(&x);
            // Under the master limiter's knee, so pure tones never get its distortion.
            assert!(pk > 0.15 && pk < 0.66, "{label} peak {pk}");
            // Short: the audible part (above -40 dB) of the event.
            let audible = x.iter().rposition(|v| v.abs() > pk * 0.01).unwrap() as f32 / SR;
            let limit = match name {
                "ui_level_up" | "ui_fanfare" => 2.0,
                "ui_hover" | "ui_click" | "ui_type" | "ui_slider" | "ui_toggle_on" | "ui_toggle_off" | "ui_countdown" => 0.45,
                _ => 0.8,
            };
            assert!(audible < limit, "{label} sounds for {audible:.2} s (limit {limit})");
            // No DC over the audible part (a decaying tone's own offset is far below this).
            let body = &x[..(audible * SR) as usize];
            let mean = body.iter().sum::<f32>() / body.len() as f32;
            assert!(mean.abs() < 0.02 * rms(body), "{label}: DC {mean} against rms {}", rms(body));
            // Starts from silence and ramps up: no step at the onset.
            let onset = peak(&x[..(0.002 * SR) as usize]);
            assert!(x[0].abs() < 0.05 * pk && x[0].abs() < 0.25 * onset, "{label} starts with a step: {} (peak {pk})", x[0]);
            // Ends by fading out: the last 10 ms before silence are 60 dB down, then exact zeros.
            let last = x.iter().rposition(|v| *v != 0.0).unwrap();
            let tail = &x[last.saturating_sub((0.01 * SR) as usize)..=last];
            assert!(peak(tail) < 1e-3 * pk, "{label} ends with a cut: {} against peak {pk}", peak(tail));
            assert!(last + 1 < x.len(), "{label} never reaches silence");
        }
    }
}

#[test]
fn combo_steps_up_the_scale_exactly_one_step_at_a_time() {
    // (key, scale): C major, A minor pentatonic, F# Dorian, Eb pentatonic, B minor, G Lydian.
    for (key, scale) in [(0, 0), (9, 3), (6, 4), (3, 2), (11, 1), (7, 5)] {
        for style in STYLES {
            let p = params_of(kit("ui_combo", style, key, scale, 0.0).as_ref());
            let mut hz = Vec::new();
            for n in 0..9 {
                let mut m = dry("ui_combo", style, key, scale);
                m.set_input_by_name("combo", n as f32 / ui::STEPS_PER_UNIT);
                m.trigger();
                let x = render(m.as_mut(), 0.15);
                // The exact frequency of the partial nearest the expected note; a wrong note
                // (100 cents or more away) leaves no peak inside the search and fails there.
                hz.push(precise_pitch(&x, settled(style), ui::note_hz(&p, 0, n), style));
            }
            let label = format!("{} {} {style}", ui::KEY_NAMES[key], ui::SCALE_NAMES[scale]);
            for (n, f) in hz.iter().enumerate() {
                let want = ui::note_hz(&p, 0, n as i32);
                assert!((f / want - 1.0).abs() < tolerance(style), "{label}: combo {n} plays {f:.2} Hz, want {want:.2}");
            }
            for n in 0..8 {
                let step = ui::scale_semitones(scale, n as i32 + 1) - ui::scale_semitones(scale, n as i32);
                let ratio = hz[n + 1] / hz[n];
                let want = (step as f32 / 12.0).exp2();
                assert!((ratio / want - 1.0).abs() < tolerance(style), "{label}: combo {n} -> {} rises {ratio:.4}, want one step ({step} semitones)", n + 1);
            }
        }
    }
}

#[test]
fn combo_is_capped_and_counts_in_sixteenths() {
    let mut p = ui::UiParams::default();
    assert_eq!(ui::combo_steps(0.0, &p), 0);
    assert_eq!(ui::combo_steps(3.0 / 16.0, &p), 3);
    assert_eq!(ui::combo_steps(1.0, &p), 14);
    p.combo_max = 5.0;
    assert_eq!(ui::combo_steps(9.0 / 16.0, &p), 5);
}

#[test]
fn all_sounds_in_a_style_share_the_key() {
    for (key, scale) in [(0, 0), (9, 3)] {
        for style in STYLES {
            let mut classes = [0usize; 12];
            // As written, then with game state moving it along the scale (never off it).
            for (name, state) in ui_names().into_iter().flat_map(|n| [(n, false), (n, true)]) {
                let label = format!("{name}/{style} in {} {}", ui::KEY_NAMES[key], ui::SCALE_NAMES[scale]);
                let mut m = dry(name, style, key, scale);
                if state {
                    m.set_input_by_name("combo", 2.0 / 16.0);
                    m.set_input_by_name("value", 0.6);
                    m.set_input_by_name("count", 5.0 / 16.0);
                }
                let x = fire(m.as_mut());
                // Judge settled notes: frames clear of each onset's strike, and for Sci-fi of
                // its deliberate glide into the note.
                let starts = onsets(&x);
                let frame_secs = FRAME as f32 / SR;
                let settled_frame = |t: f32| starts.iter().all(|&o| t + frame_secs <= o || t >= o + settled(style));
                let frames: Vec<(f32, f32)> = pitched_frames(&x).into_iter().filter(|&(t, _)| settled_frame(t)).collect();
                assert!(!frames.is_empty(), "{label}: no pitched frame");
                for (t, hz) in frames {
                    let off = cents_off_scale(hz, key, scale);
                    // A note off the scale is at least 100 cents from the nearest note on it. A
                    // reading can lean up to about 25: chorus, and two notes a semitone apart low
                    // down (error), which a 23 ms frame cannot separate.
                    assert!(off.abs() < 30.0, "{label}: {hz:.1} Hz at {t:.2} s is {off:.0} cents off the scale");
                    if !state {
                        let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32;
                        classes[midi.rem_euclid(12) as usize] += 1;
                    }
                }
            }
            // As written, the kit is anchored on the tonic: it is the commonest pitch class.
            let commonest = (0..12).max_by_key(|&c| classes[c]).unwrap();
            assert_eq!(commonest, key, "{style}: pitch classes {classes:?}");
        }
    }
}

#[test]
fn error_is_lower_and_darker_than_confirm() {
    for style in STYLES {
        for (key, scale) in [(0, 0), (7, 1)] {
            let confirm = fire(kit("ui_confirm", style, key, scale, 0.0).as_mut());
            let error = fire(kit("ui_error", style, key, scale, 0.0).as_mut());
            let median = |x: &[f32]| {
                let mut f: Vec<f32> = pitched_frames(x).into_iter().map(|(_, hz)| hz).collect();
                f.sort_by(f32::total_cmp);
                f[f.len() / 2]
            };
            let (pc, pe) = (median(&confirm), median(&error));
            assert!(pe < pc * 0.6, "{style}: error at {pe:.0} Hz is not well below confirm at {pc:.0} Hz");
            let (bc, be) = (rms_hz(&confirm), rms_hz(&error));
            assert!(be < bc * 0.6, "{style}: error (rms frequency {be:.0} Hz) is not darker than confirm ({bc:.0} Hz)");
        }
    }
}

#[test]
fn slider_tick_follows_its_value_on_the_scale() {
    for style in STYLES {
        let p = params_of(kit("ui_slider", style, 2, 0, 0.0).as_ref());
        let mut last = 0.0;
        for k in 0..=7 {
            let mut m = dry("ui_slider", style, 2, 0);
            m.set_input_by_name("value", k as f32 / 7.0);
            m.trigger();
            let x = render(m.as_mut(), 0.15);
            let want = ui::note_hz(&p, 0, k);
            let hz = precise_pitch_over(&x, settled(style), 0.04, want);
            assert!((hz / want - 1.0).abs() < tolerance(style), "{style}: value {k}/7 plays {hz:.1} Hz, want {want:.1}");
            assert!(hz > last, "{style}: the tick does not rise with the value");
            last = hz;
        }
        // Between detents the tick snaps to the nearest step.
        let mut m = dry("ui_slider", style, 2, 0);
        m.set_input_by_name("value", 0.05);
        m.trigger();
        let x = render(m.as_mut(), 0.15);
        let hz = precise_pitch_over(&x, settled(style), 0.04, ui::note_hz(&p, 0, 0));
        assert!((hz / ui::note_hz(&p, 0, 0) - 1.0).abs() < tolerance(style), "{style}: value 0.05 should snap to the bottom step");
    }
}

/// Onset times: where the 2 ms envelope jumps 6 dB above its level 6 ms earlier.
fn onsets(x: &[f32]) -> Vec<f32> {
    let n = (0.002 * SR) as usize;
    // Silence in front, so an onset at t = 0 counts too.
    let env: Vec<f32> = [0.0; 3].into_iter().chain(x.chunks(n).map(rms)).collect();
    let floor = env.iter().copied().fold(0.0, f32::max) * 0.05;
    let mut out: Vec<usize> = Vec::new();
    for i in 3..env.len() {
        if env[i] > floor && env[i] > 2.0 * env[i - 3] && out.last().is_none_or(|&j| i - j > 10) {
            out.push(i);
        }
    }
    out.into_iter().map(|i| ((i - 3) * n) as f32 / SR).collect()
}

#[test]
fn a_coin_burst_scatters_into_a_rising_arpeggio() {
    for style in STYLES {
        // Six coins: the chord tones C6 E6 G6 C7 E7 G7 (in C major), each in turn, in tune.
        let mut d = dry("ui_coin", style, 0, 0);
        d.set_input_by_name("count", 6.0 / 16.0);
        let x = fire(d.as_mut());
        let p = params_of(d.as_ref());
        let notes: Vec<f32> = (0..6).map(|k| ui::note_hz(&p, 0, ui::chord_tone(0, 3 + k))).collect();
        // The first three (no octaves between them, so no note hides in another's overtones)
        // come in one after another, evenly; the rest follow on the same beat.
        let first = note_times(&x, &notes[..3]);
        let gap = 0.5 * (first[2] - first[0]);
        assert!(gap > 0.02 && (first[1] - first[0] - gap).abs() < 0.006, "{style}: the coins do not come one after another up the chord: {first:?}");
        for (k, &want) in notes.iter().enumerate() {
            let t = first[0] + gap * k as f32;
            // Past the strike; for Sci-fi past most of its chirp into the note.
            let after = if style == "Sci-fi" { 0.024 } else { 0.004 };
            let hz = precise_pitch_over(&x, t + after, 0.015, want);
            assert!((hz / want - 1.0).abs() < 0.01, "{style}: coin {k} at {t:.3} s plays {hz:.1} Hz, want {want:.1}");
        }
        // Where the gated 8-bit notes do not run together, each coin is its own strike.
        if style != "Retro" {
            let mut m = kit("ui_coin", style, 0, 0, 0.0);
            m.set_input_by_name("count", 6.0 / 16.0);
            let burst = fire(m.as_mut());
            assert_eq!(onsets(&burst).len(), 6, "{style}: 6 coins should strike 6 times");
        }
        // A single coin is the classic two notes, up a fourth from the fifth to the tonic.
        let one = fire(dry("ui_coin", style, 0, 0).as_mut());
        let pair = note_times(&one, &[ui::note_hz(&p, 0, ui::chord_tone(0, 2)), ui::note_hz(&p, 0, ui::chord_tone(0, 3))]);
        assert!(pair[1] - pair[0] > 0.04, "{style}: one coin should be two notes rising: {pair:?}");
    }
}

#[test]
fn one_style_parameter_reskins_the_kit_but_keeps_the_notes() {
    for name in ["ui_click", "ui_confirm", "ui_coin"] {
        let renders: Vec<Vec<f32>> = STYLES.iter().map(|s| fire(kit(name, s, 0, 0, 0.0).as_mut())).collect();
        // Each starts on its written note: click the tonic an octave up, the others the fifth.
        let written = |m: &dyn Model| {
            let p = params_of(m);
            if name == "ui_click" {
                ui::note_hz(&p, 1, 0)
            } else {
                ui::note_hz(&p, 0, ui::chord_tone(0, 2))
            }
        };
        let first_pitch = |style: &str| {
            let mut m = dry(name, style, 0, 0);
            let want = written(m.as_ref());
            let x = fire(m.as_mut());
            precise_pitch_over(&x, settled(style), 0.04, want)
        };
        let base = first_pitch("Soft");
        for style in STYLES {
            let hz = first_pitch(style);
            assert!((hz / base - 1.0).abs() < tolerance(style), "{name}: {style} starts on {hz:.1} Hz, Soft on {base:.1}");
        }
        // Different timbres: brightness and length differ between the styles.
        let bright: Vec<f32> = renders.iter().map(|x| rms_hz(x)).collect();
        let spread = bright.iter().copied().fold(0.0, f32::max) / bright.iter().copied().fold(f32::MAX, f32::min);
        assert!(spread > 1.3, "{name}: the styles sound alike (rms frequencies {bright:?})");
    }
    // Setting the parameter alone does the same as loading the preset.
    let mut a = kit("ui_confirm", "Retro", 0, 0, 0.0);
    let mut b = kit("ui_confirm", "Soft", 0, 0, 0.0);
    assert!(b.set_param_by_name("kit/style", 3.0));
    assert_eq!(fire(a.as_mut()), fire(b.as_mut()));
}

#[test]
fn retriggering_lets_ringing_notes_finish() {
    // A combo chain on one player: the second hit must not cut the first.
    let mut chain = kit("ui_combo", "Glass", 0, 0, 0.0);
    chain.trigger();
    render(chain.as_mut(), 0.1);
    chain.set_input_by_name("combo", 1.0 / 16.0);
    chain.trigger();
    let both = render(chain.as_mut(), 0.2);
    let mut alone = kit("ui_combo", "Glass", 0, 0, 0.0);
    alone.set_input_by_name("combo", 1.0 / 16.0);
    alone.trigger();
    let second = render(alone.as_mut(), 0.2);
    let rest: Vec<f32> = both.iter().zip(&second).map(|(a, b)| a - b).collect();
    assert!(rms(&rest) > 0.2 * rms(&second), "the first hit was cut: {} vs {}", rms(&rest), rms(&second));
}

#[test]
fn variation_zero_repeats_and_the_default_varies() {
    for name in ["ui_click", "ui_coin", "ui_level_up"] {
        let a = fire(kit(name, "Wooden", 0, 0, 0.0).as_mut());
        let b = fire(kit(name, "Wooden", 0, 0, 0.0).as_mut());
        assert_eq!(a, b, "{name}: variation 0 must repeat exactly");
        let mut m = kit(name, "Wooden", 0, 0, 0.5);
        let c = fire(m.as_mut());
        let d = fire(m.as_mut());
        let diff = c.iter().zip(&d).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(diff > 0.01, "{name}: two triggers at variation 0.5 are identical");
    }
}

#[test]
fn width_zero_is_the_mono_render() {
    for name in ["ui_level_up", "ui_coin"] {
        let mut a = kit(name, "Sci-fi", 0, 0, 0.0);
        let mut b = kit(name, "Sci-fi", 0, 0, 0.0);
        a.set_param_by_name("space/width", 0.0);
        a.trigger();
        b.trigger();
        let n = (1.0 * SR) as usize;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        a.render_stereo(&mut l, &mut r);
        assert!(l == r && l == render(b.as_mut(), 1.0), "{name}: width 0 must be the mono sound");
        // At the designed width the arpeggio spreads, but stays centred and mono-compatible.
        let mut w = kit(name, "Sci-fi", 0, 0, 0.0);
        w.trigger();
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        w.render_stereo(&mut l, &mut r);
        assert!(l != r && (rms(&l) / rms(&r)).ln().abs() < 0.3, "{name}: left {} right {}", rms(&l), rms(&r));
    }
}

#[test]
fn a_busy_ui_is_cheap() {
    // Every event of the kit fired at once, three times over, in stereo.
    let mut all: Vec<Box<dyn Model>> = ui_names().iter().map(|n| kit(n, "Glass", 0, 0, 0.5)).collect();
    let secs = 2.0;
    let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
    let blocks = (secs * SR) as usize / 256;
    let start = std::time::Instant::now();
    for b in 0..blocks {
        if b % (blocks / 3) == 0 {
            for m in all.iter_mut() {
                m.set_input_by_name("count", 1.0);
                m.trigger();
            }
        }
        for m in all.iter_mut() {
            m.render_stereo(&mut l, &mut r);
        }
    }
    let elapsed = start.elapsed().as_secs_f32();
    println!("ui kit: 19 events retriggered 3x render at {:.0}x real time", secs / elapsed);
    assert!(elapsed < secs * 0.25, "19 UI events took {elapsed:.3} s for {secs} s");
}
