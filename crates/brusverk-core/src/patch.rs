//! The `Patch`: a plain-data, `Copy` description of a sound, plus a flat, string-addressable
//! parameter table (`ParamId`) used by engine bindings for inspectors and live tweaking.

use crate::env::AdsrParams;
use crate::filter::FilterMode;
use crate::lfo::{LfoParams, LfoWave};
use crate::osc::Waveform;

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct OscParams {
    pub wave: Waveform,
    /// Mix level 0..1. Zero disables the oscillator entirely (no CPU cost).
    pub level: f32,
    /// Coarse tune in semitones.
    pub semitones: f32,
    /// Fine tune in cents.
    pub detune_cents: f32,
    /// Duty cycle for `Waveform::Pulse`.
    pub pulse_width: f32,
    /// Pulse width change per second.
    pub pulse_width_sweep: f32,
}

impl Default for OscParams {
    fn default() -> Self {
        OscParams {
            wave: Waveform::Saw,
            level: 0.0,
            semitones: 0.0,
            detune_cents: 0.0,
            pulse_width: 0.5,
            pulse_width_sweep: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct PitchParams {
    /// Portamento time in seconds between successive notes.
    pub glide: f32,
    /// Constant pitch slide in semitones per second (sfxr "slide").
    pub slide: f32,
    /// Change of the slide rate in semitones per second² (sfxr "delta slide").
    pub slide_accel: f32,
    /// Pitch jump applied once after `arp_time` seconds (sfxr "change amount").
    pub arp_semitones: f32,
    /// Seconds after note-on when the jump happens. 0 disables it.
    pub arp_time: f32,
    pub vibrato_rate: f32,
    /// Vibrato depth in semitones.
    pub vibrato_depth: f32,
    /// Semitones of pitch bend for a full-scale bend (`Synth::set_pitch_bend`).
    pub bend_range: f32,
}

impl Default for PitchParams {
    fn default() -> Self {
        PitchParams {
            glide: 0.0,
            slide: 0.0,
            slide_accel: 0.0,
            arp_semitones: 0.0,
            arp_time: 0.0,
            vibrato_rate: 0.0,
            vibrato_depth: 0.0,
            bend_range: 2.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FilterParams {
    pub mode: FilterMode,
    pub cutoff_hz: f32,
    /// 0..1, 1 is at the edge of self oscillation.
    pub resonance: f32,
    /// Filter envelope depth in octaves.
    pub env_amount: f32,
    /// 0..1: how much the cutoff follows the played note (1 = one octave per octave).
    pub key_track: f32,
    /// Cutoff sweep in octaves per second after note-on.
    pub sweep: f32,
    /// Octaves the cutoff closes at velocity 0 (no effect at velocity 1).
    pub velocity: f32,
}

impl Default for FilterParams {
    fn default() -> Self {
        FilterParams {
            mode: FilterMode::LowPass,
            cutoff_hz: 8000.0,
            resonance: 0.2,
            env_amount: 0.0,
            key_track: 0.0,
            sweep: 0.0,
            velocity: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FxParams {
    /// Soft-clip drive 0..1.
    pub drive: f32,
    /// Bit depth for the bit crusher, 0 = off, otherwise 1..16.
    pub bit_depth: f32,
    /// Sample-and-hold factor. 1 = off, N holds each sample N times.
    pub downsample: f32,
    /// Delay time in seconds. 0 disables the delay.
    pub delay_time: f32,
    pub delay_feedback: f32,
    pub delay_mix: f32,
}

impl Default for FxParams {
    fn default() -> Self {
        FxParams {
            drive: 0.0,
            bit_depth: 0.0,
            downsample: 1.0,
            delay_time: 0.0,
            delay_feedback: 0.3,
            delay_mix: 0.3,
        }
    }
}

/// Complete description of an instrument or sound effect.
///
/// It is `Copy` and contains no heap data so it can be sent to the audio thread by value
/// through a lock-free queue.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Patch {
    pub osc: [OscParams; 3],
    pub amp_env: AdsrParams,
    pub filter: FilterParams,
    pub filter_env: AdsrParams,
    pub lfo: LfoParams,
    pub pitch: PitchParams,
    pub fx: FxParams,
    /// Output gain (linear).
    pub gain: f32,
    /// Maximum simultaneous voices, 1..=MAX_VOICES. 1 gives a legato mono synth.
    pub polyphony: u8,
    /// MIDI note used by `Synth::trigger` (one-shot sound effects).
    pub base_note: f32,
    /// Seconds after which a triggered note auto-releases. 0 = hold until note-off.
    pub duration: f32,
    /// 0..1: how strongly velocity scales the amplitude.
    pub velocity_sens: f32,
}

impl Default for Patch {
    fn default() -> Self {
        let mut osc = [OscParams::default(); 3];
        osc[0].level = 1.0;
        Patch {
            osc,
            amp_env: AdsrParams::default(),
            filter: FilterParams::default(),
            filter_env: AdsrParams::new(0.0, 0.2, 0.0, 0.2),
            lfo: LfoParams::default(),
            pitch: PitchParams::default(),
            fx: FxParams::default(),
            gain: 0.5,
            polyphony: 8,
            base_note: 60.0,
            duration: 0.0,
            velocity_sens: 1.0,
        }
    }
}

impl Patch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Envelope shape used by sfxr-style one-shots: instant full level, `sustain` seconds
    /// of hold (via `duration`), then a `decay` second fade.
    pub fn set_sfx_envelope(&mut self, attack: f32, sustain: f32, decay: f32) {
        self.amp_env = AdsrParams::new(attack, 0.0, 1.0, decay);
        self.duration = attack + sustain;
    }

    /// Total length of a triggered one-shot including release tail, or `None` when the
    /// patch holds until note-off.
    pub fn one_shot_length(&self) -> Option<f32> {
        if self.duration > 0.0 {
            Some(self.duration + self.amp_env.release + self.fx.delay_time * 4.0)
        } else if self.amp_env.sustain <= 0.0 {
            Some(self.amp_env.attack + self.amp_env.decay * 2.0 + self.fx.delay_time * 4.0)
        } else {
            None
        }
    }

    #[cfg(feature = "serde")]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("Patch is always serialisable")
    }

    #[cfg(feature = "serde")]
    pub fn from_json(json: &str) -> Result<Patch, serde_json::Error> {
        serde_json::from_str(json)
    }
}

// ---------------------------------------------------------------------------------------------
// Flat parameter table
// ---------------------------------------------------------------------------------------------

pub use crate::params::{ParamKind, ParamValue, Params};

impl_param_value_enum!(Waveform);
impl_param_value_enum!(FilterMode);
impl_param_value_enum!(LfoWave);

const F01: ParamKind = ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 };
const SECS: ParamKind = ParamKind::Float { min: 0.0, max: 5.0, step: 0.001 };

param_table! {
    /// Identifier of every tweakable value in a [`Patch`].
    ParamId for Patch {
        Osc1Wave => "osc1/wave", ParamKind::Enum(&Waveform::NAMES), (osc[0].wave);
        Osc1Level => "osc1/level", F01, (osc[0].level);
        Osc1Semitones => "osc1/semitones", ParamKind::Float { min: -48.0, max: 48.0, step: 1.0 }, (osc[0].semitones);
        Osc1Detune => "osc1/detune_cents", ParamKind::Float { min: -100.0, max: 100.0, step: 0.1 }, (osc[0].detune_cents);
        Osc1PulseWidth => "osc1/pulse_width", ParamKind::Float { min: 0.02, max: 0.98, step: 0.001 }, (osc[0].pulse_width);
        Osc1PulseWidthSweep => "osc1/pulse_width_sweep", ParamKind::Float { min: -4.0, max: 4.0, step: 0.001 }, (osc[0].pulse_width_sweep);

        Osc2Wave => "osc2/wave", ParamKind::Enum(&Waveform::NAMES), (osc[1].wave);
        Osc2Level => "osc2/level", F01, (osc[1].level);
        Osc2Semitones => "osc2/semitones", ParamKind::Float { min: -48.0, max: 48.0, step: 1.0 }, (osc[1].semitones);
        Osc2Detune => "osc2/detune_cents", ParamKind::Float { min: -100.0, max: 100.0, step: 0.1 }, (osc[1].detune_cents);
        Osc2PulseWidth => "osc2/pulse_width", ParamKind::Float { min: 0.02, max: 0.98, step: 0.001 }, (osc[1].pulse_width);
        Osc2PulseWidthSweep => "osc2/pulse_width_sweep", ParamKind::Float { min: -4.0, max: 4.0, step: 0.001 }, (osc[1].pulse_width_sweep);

        Osc3Wave => "osc3/wave", ParamKind::Enum(&Waveform::NAMES), (osc[2].wave);
        Osc3Level => "osc3/level", F01, (osc[2].level);
        Osc3Semitones => "osc3/semitones", ParamKind::Float { min: -48.0, max: 48.0, step: 1.0 }, (osc[2].semitones);
        Osc3Detune => "osc3/detune_cents", ParamKind::Float { min: -100.0, max: 100.0, step: 0.1 }, (osc[2].detune_cents);
        Osc3PulseWidth => "osc3/pulse_width", ParamKind::Float { min: 0.02, max: 0.98, step: 0.001 }, (osc[2].pulse_width);
        Osc3PulseWidthSweep => "osc3/pulse_width_sweep", ParamKind::Float { min: -4.0, max: 4.0, step: 0.001 }, (osc[2].pulse_width_sweep);

        AmpAttack => "amp_env/attack", SECS, (amp_env.attack);
        AmpDecay => "amp_env/decay", SECS, (amp_env.decay);
        AmpSustain => "amp_env/sustain", F01, (amp_env.sustain);
        AmpRelease => "amp_env/release", SECS, (amp_env.release);

        FilterMode => "filter/mode", ParamKind::Enum(&FilterMode::NAMES), (filter.mode);
        FilterCutoff => "filter/cutoff_hz", ParamKind::Exp { min: 20.0, max: 20000.0 }, (filter.cutoff_hz);
        FilterResonance => "filter/resonance", F01, (filter.resonance);
        FilterEnvAmount => "filter/env_amount", ParamKind::Float { min: -8.0, max: 8.0, step: 0.01 }, (filter.env_amount);
        FilterKeyTrack => "filter/key_track", F01, (filter.key_track);
        FilterSweep => "filter/sweep", ParamKind::Float { min: -16.0, max: 16.0, step: 0.01 }, (filter.sweep);
        FilterVelocity => "filter/velocity", ParamKind::Float { min: 0.0, max: 8.0, step: 0.01 }, (filter.velocity);

        FilterAttack => "filter_env/attack", SECS, (filter_env.attack);
        FilterDecay => "filter_env/decay", SECS, (filter_env.decay);
        FilterSustain => "filter_env/sustain", F01, (filter_env.sustain);
        FilterRelease => "filter_env/release", SECS, (filter_env.release);

        LfoWave => "lfo/wave", ParamKind::Enum(&LfoWave::NAMES), (lfo.wave);
        LfoRate => "lfo/rate_hz", ParamKind::Exp { min: 0.01, max: 50.0 }, (lfo.rate_hz);
        LfoPitch => "lfo/pitch", ParamKind::Float { min: 0.0, max: 24.0, step: 0.01 }, (lfo.pitch);
        LfoCutoff => "lfo/cutoff", ParamKind::Float { min: 0.0, max: 8.0, step: 0.01 }, (lfo.cutoff);
        LfoAmp => "lfo/amp", F01, (lfo.amp);
        LfoPulseWidth => "lfo/pulse_width", ParamKind::Float { min: 0.0, max: 0.5, step: 0.001 }, (lfo.pulse_width);
        LfoFadeIn => "lfo/fade_in", SECS, (lfo.fade_in);

        PitchGlide => "pitch/glide", ParamKind::Float { min: 0.0, max: 2.0, step: 0.001 }, (pitch.glide);
        PitchSlide => "pitch/slide", ParamKind::Float { min: -400.0, max: 400.0, step: 0.1 }, (pitch.slide);
        PitchSlideAccel => "pitch/slide_accel", ParamKind::Float { min: -2000.0, max: 2000.0, step: 0.1 }, (pitch.slide_accel);
        PitchArpSemitones => "pitch/arp_semitones", ParamKind::Float { min: -36.0, max: 36.0, step: 0.1 }, (pitch.arp_semitones);
        PitchArpTime => "pitch/arp_time", ParamKind::Float { min: 0.0, max: 2.0, step: 0.001 }, (pitch.arp_time);
        PitchVibratoRate => "pitch/vibrato_rate", ParamKind::Float { min: 0.0, max: 40.0, step: 0.01 }, (pitch.vibrato_rate);
        PitchVibratoDepth => "pitch/vibrato_depth", ParamKind::Float { min: 0.0, max: 12.0, step: 0.01 }, (pitch.vibrato_depth);
        PitchBendRange => "pitch/bend_range", ParamKind::Float { min: 0.0, max: 24.0, step: 1.0 }, (pitch.bend_range);

        FxDrive => "fx/drive", F01, (fx.drive);
        FxBitDepth => "fx/bit_depth", ParamKind::Float { min: 0.0, max: 16.0, step: 1.0 }, (fx.bit_depth);
        FxDownsample => "fx/downsample", ParamKind::Float { min: 1.0, max: 64.0, step: 0.1 }, (fx.downsample);
        FxDelayTime => "fx/delay_time", ParamKind::Float { min: 0.0, max: 1.0, step: 0.001 }, (fx.delay_time);
        FxDelayFeedback => "fx/delay_feedback", ParamKind::Float { min: 0.0, max: 0.95, step: 0.001 }, (fx.delay_feedback);
        FxDelayMix => "fx/delay_mix", F01, (fx.delay_mix);

        Gain => "master/gain", ParamKind::Float { min: 0.0, max: 2.0, step: 0.001 }, (gain);
        Polyphony => "master/polyphony", ParamKind::Int { min: 1, max: crate::synth::MAX_VOICES as i32 }, (polyphony);
        BaseNote => "master/base_note", ParamKind::Float { min: 0.0, max: 127.0, step: 0.01 }, (base_note);
        Duration => "master/duration", ParamKind::Float { min: 0.0, max: 10.0, step: 0.001 }, (duration);
        VelocitySens => "master/velocity_sens", F01, (velocity_sens);
    }
}
