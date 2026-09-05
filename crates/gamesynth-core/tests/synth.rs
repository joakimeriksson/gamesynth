use gamesynth_core::render::{peak, render_events, render_one_shot, rms};
use gamesynth_core::*;

const SR: f32 = 48000.0;

fn sine_patch() -> Patch {
    let mut p = Patch::default();
    p.osc[0].wave = Waveform::Sine;
    p.filter.mode = FilterMode::Off;
    p.amp_env = AdsrParams::new(0.0, 0.0, 1.0, 0.01);
    // Stay below the master limiter knee so tests measure the raw signal.
    p.gain = 0.5;
    p
}

fn all_finite(s: &[f32]) -> bool {
    s.iter().all(|x| x.is_finite())
}

#[test]
fn sine_has_expected_amplitude_and_frequency() {
    let p = sine_patch();
    let mut synth = Synth::with_patch(SR, p);
    synth.note_on(69.0, 1.0);
    let mut out = vec![0.0; SR as usize];
    synth.render_mono(&mut out);
    assert!(all_finite(&out));
    let pk = peak(&out[1000..]);
    assert!((pk - 0.5).abs() < 0.01, "peak {pk}");
    // Count zero crossings over one second: 440 Hz -> ~880 crossings.
    let crossings = out.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    assert!((crossings as i32 - 880).abs() <= 2, "crossings {crossings}");
}

#[test]
fn envelope_releases_to_silence() {
    let mut p = sine_patch();
    p.amp_env = AdsrParams::new(0.01, 0.1, 0.5, 0.1);
    let events = [
        (0.0, Command::NoteOn { note: 60.0, velocity: 1.0, duration: 0.0 }),
        (0.5, Command::NoteOff { note: 60.0 }),
    ];
    let out = render_events(&p, SR, &events, 5.0);
    let secs = out.len() as f32 / SR;
    assert!(secs > 0.5 && secs < 1.5, "rendered {secs}s");
    // Sustain portion should sit near 0.5 * gain(0.5) = 0.25 peak.
    let sustain = peak(&out[(0.3 * SR) as usize..(0.45 * SR) as usize]);
    assert!((sustain - 0.25).abs() < 0.02, "sustain {sustain}");
    assert!(peak(&out[out.len() - 100..]) < 1e-3);
}

#[test]
fn lowpass_attenuates_high_frequencies() {
    let mut p = sine_patch();
    p.filter.mode = FilterMode::LowPass;
    p.filter.cutoff_hz = 500.0;
    p.filter.resonance = 0.0;
    let render = |note: f32| {
        let mut synth = Synth::with_patch(SR, p);
        synth.note_on(note, 1.0);
        let mut out = vec![0.0; SR as usize / 2];
        synth.render_mono(&mut out);
        rms(&out[4800..])
    };
    let low = render(45.0); // ~110 Hz
    let high = render(105.0); // ~3520 Hz
    assert!(low > 0.3, "low {low}");
    assert!(high < low * 0.1, "high {high} low {low}");
}

#[test]
fn polyphony_and_voice_stealing() {
    let mut p = sine_patch();
    p.polyphony = 4;
    let mut synth = Synth::with_patch(SR, p);
    for n in 0..10 {
        synth.note_on(50.0 + n as f32, 1.0);
    }
    assert_eq!(synth.active_voices(), 4);
    synth.all_notes_off();
    let mut out = vec![0.0; SR as usize / 2];
    synth.render_mono(&mut out);
    assert!(synth.is_silent());
    assert_eq!(synth.active_voices(), 0);
}

#[test]
fn mono_legato_glides() {
    let mut p = sine_patch();
    p.polyphony = 1;
    p.pitch.glide = 0.1;
    let events = [
        (0.0, Command::NoteOn { note: 60.0, velocity: 1.0, duration: 0.0 }),
        (0.2, Command::NoteOn { note: 72.0, velocity: 1.0, duration: 0.0 }),
        (0.6, Command::AllNotesOff),
    ];
    let out = render_events(&p, SR, &events, 2.0);
    assert!(all_finite(&out));
    // No discontinuity larger than the maximum slope of a 523 Hz sine at 48 kHz (~0.07).
    let max_step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_step < 0.1, "max step {max_step}");
}

#[test]
fn all_sfx_presets_render_clean_one_shots() {
    for preset in SfxPreset::ALL {
        for seed in 0..25u32 {
            let patch = preset.generate(seed);
            let out = render_one_shot(&patch, SR, 6.0);
            assert!(all_finite(&out), "{preset:?} seed {seed} produced NaN/inf");
            assert!(peak(&out) > 0.05, "{preset:?} seed {seed} is silent");
            assert!(peak(&out) <= 1.0, "{preset:?} seed {seed} peak {}", peak(&out));
            let secs = out.len() as f32 / SR;
            assert!(secs < 6.0, "{preset:?} seed {seed} never went silent");
            // Deterministic for a given seed.
            assert_eq!(preset.generate(seed), patch);
        }
    }
}

#[test]
fn param_table_round_trips() {
    let mut p = Patch::default();
    for &id in ParamId::ALL {
        assert_eq!(ParamId::from_name(id.name()), Some(id));
        let v = p.get(id);
        p.set(id, v);
        assert_eq!(p.get(id), v, "{id:?}");
    }
    p.set(ParamId::FilterCutoff, 1e9);
    assert_eq!(p.filter.cutoff_hz, 20000.0);
    p.set(ParamId::Osc1Wave, 5.0);
    assert_eq!(p.osc[0].wave, Waveform::WhiteNoise);
    p.set(ParamId::Polyphony, 99.0);
    assert_eq!(p.polyphony as usize, MAX_VOICES);
    p.set(ParamId::Gain, f32::NAN);
    assert!(p.gain.is_finite());
}

#[test]
fn patch_json_round_trip() {
    let patch = SfxPreset::Laser.generate(7);
    let json = patch.to_json();
    let back = Patch::from_json(&json).unwrap();
    assert_eq!(back, patch);
    // Partial JSON fills in defaults.
    let partial = Patch::from_json(r#"{"gain": 0.25, "filter": {"cutoff_hz": 1234.0}}"#).unwrap();
    assert_eq!(partial.gain, 0.25);
    assert_eq!(partial.filter.cutoff_hz, 1234.0);
    assert_eq!(partial.osc, Patch::default().osc);
}

#[test]
fn effects_and_lfo_stay_bounded() {
    let mut p = Patch::default();
    p.osc[1].wave = Waveform::Pulse;
    p.osc[1].level = 0.7;
    p.osc[2].wave = Waveform::PinkNoise;
    p.osc[2].level = 0.3;
    p.filter.resonance = 1.0;
    p.filter.cutoff_hz = 300.0;
    p.filter.env_amount = 5.0;
    p.lfo.pitch = 2.0;
    p.lfo.cutoff = 3.0;
    p.lfo.amp = 0.5;
    p.lfo.pulse_width = 0.4;
    p.fx.drive = 1.0;
    p.fx.bit_depth = 4.0;
    p.fx.downsample = 6.0;
    p.fx.delay_time = 0.25;
    p.fx.delay_feedback = 0.6;
    p.fx.delay_mix = 0.8;
    p.duration = 1.0;
    let out = render_one_shot(&p, SR, 10.0);
    assert!(all_finite(&out));
    assert!(peak(&out) <= 1.0);
    assert!((out.len() as f32 / SR) < 10.0, "delay tail never decayed");
}

#[test]
fn stereo_render_matches_mono() {
    let p = SfxPreset::Pickup.generate(3);
    let mut a = Synth::with_patch(SR, p);
    let mut b = Synth::with_patch(SR, p);
    a.trigger();
    b.trigger();
    let mut mono = vec![0.0; 4096];
    let mut stereo = vec![StereoFrame::default(); 4096];
    a.render_mono(&mut mono);
    b.render(&mut stereo);
    for (m, s) in mono.iter().zip(&stereo) {
        assert_eq!(*m, s.left);
        assert_eq!(s.left, s.right);
    }
}

#[test]
fn renders_faster_than_real_time() {
    let mut p = Patch::default();
    p.osc[1].level = 0.5;
    p.osc[2].wave = Waveform::WhiteNoise;
    p.osc[2].level = 0.2;
    p.lfo.pitch = 0.1;
    p.polyphony = MAX_VOICES as u8;
    p.fx.delay_time = 0.3;
    p.fx.delay_mix = 0.3;
    let mut synth = Synth::with_patch(SR, p);
    for n in 0..MAX_VOICES {
        synth.note_on(40.0 + n as f32, 1.0);
    }
    let secs = 2.0;
    let mut out = vec![StereoFrame::default(); 512];
    let start = std::time::Instant::now();
    for _ in 0..((secs * SR) as usize / 512) {
        synth.render(&mut out);
    }
    let elapsed = start.elapsed().as_secs_f32();
    // 32 voices should cost far less than one core even in a dev build.
    assert!(elapsed < secs * 0.5, "took {elapsed}s for {secs}s of audio");
}
