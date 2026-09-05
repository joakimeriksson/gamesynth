//! sfxr/bfxr-style procedural sound effect generators.
//!
//! Each preset returns a randomised [`Patch`] for a given seed; the same seed always yields
//! the same sound, so games can store just `(preset, seed)`.

use crate::filter::FilterMode;
use crate::math::Rng;
use crate::osc::Waveform;
use crate::patch::{ParamId, ParamKind, Patch};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum SfxPreset {
    Pickup = 0,
    Laser = 1,
    Explosion = 2,
    PowerUp = 3,
    Hit = 4,
    Jump = 5,
    Blip = 6,
    Random = 7,
}

impl SfxPreset {
    pub const ALL: [SfxPreset; 8] = [
        SfxPreset::Pickup,
        SfxPreset::Laser,
        SfxPreset::Explosion,
        SfxPreset::PowerUp,
        SfxPreset::Hit,
        SfxPreset::Jump,
        SfxPreset::Blip,
        SfxPreset::Random,
    ];
    pub const NAMES: [&'static str; 8] =
        ["Pickup", "Laser", "Explosion", "PowerUp", "Hit", "Jump", "Blip", "Random"];

    pub fn from_index(i: usize) -> SfxPreset {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn from_name(name: &str) -> Option<SfxPreset> {
        Self::ALL.iter().copied().find(|p| p.name().eq_ignore_ascii_case(name))
    }

    /// Generate a randomised patch for this preset.
    pub fn generate(self, seed: u32) -> Patch {
        let mut rng = Rng::new(seed ^ (self as u32 + 1).wrapping_mul(0x9E37_79B9));
        match self {
            SfxPreset::Pickup => pickup(&mut rng),
            SfxPreset::Laser => laser(&mut rng),
            SfxPreset::Explosion => explosion(&mut rng),
            SfxPreset::PowerUp => power_up(&mut rng),
            SfxPreset::Hit => hit(&mut rng),
            SfxPreset::Jump => jump(&mut rng),
            SfxPreset::Blip => blip(&mut rng),
            SfxPreset::Random => random(&mut rng),
        }
    }
}

/// Blank one-shot starting point: single oscillator, filter off, mono.
fn base(wave: Waveform, note: f32) -> Patch {
    let mut p = Patch::default();
    p.osc[0].wave = wave;
    p.osc[0].level = 1.0;
    p.filter.mode = FilterMode::Off;
    p.polyphony = 4;
    p.base_note = note;
    p.gain = 0.6;
    p
}

fn tonal_wave(rng: &mut Rng) -> Waveform {
    match rng.next_u32() % 4 {
        0 => Waveform::Square,
        1 => Waveform::Saw,
        2 => Waveform::Sine,
        _ => Waveform::Pulse,
    }
}

fn pickup(rng: &mut Rng) -> Patch {
    let mut p = base(Waveform::Square, rng.range(78.0, 96.0));
    if rng.chance(0.5) {
        p.osc[0].wave = Waveform::Pulse;
        p.osc[0].pulse_width = rng.range(0.2, 0.5);
    }
    p.set_sfx_envelope(0.0, rng.range(0.02, 0.1), rng.range(0.15, 0.45));
    if rng.chance(0.6) {
        p.pitch.arp_time = rng.range(0.04, 0.1);
        p.pitch.arp_semitones = [3.0, 4.0, 5.0, 7.0, 12.0][rng.next_u32() as usize % 5];
    }
    p
}

fn laser(rng: &mut Rng) -> Patch {
    let mut p = base(tonal_wave(rng), rng.range(60.0, 96.0));
    p.osc[0].pulse_width = rng.range(0.1, 0.5);
    p.osc[0].pulse_width_sweep = rng.range(-1.0, 0.0);
    p.pitch.slide = -rng.range(40.0, 160.0);
    if rng.chance(0.3) {
        p.pitch.slide_accel = -rng.range(100.0, 600.0);
    }
    p.set_sfx_envelope(0.0, rng.range(0.03, 0.15), rng.range(0.05, 0.3));
    if rng.chance(0.4) {
        p.filter.mode = FilterMode::HighPass;
        p.filter.cutoff_hz = rng.range(100.0, 1500.0);
        p.filter.resonance = rng.range(0.0, 0.5);
    }
    if rng.chance(0.3) {
        p.osc[1].wave = p.osc[0].wave;
        p.osc[1].level = 0.5;
        p.osc[1].detune_cents = rng.range(5.0, 25.0);
    }
    p
}

fn explosion(rng: &mut Rng) -> Patch {
    let wave = if rng.chance(0.5) { Waveform::WhiteNoise } else { Waveform::PinkNoise };
    let mut p = base(wave, 60.0);
    p.set_sfx_envelope(rng.range(0.0, 0.01), rng.range(0.05, 0.3), rng.range(0.3, 0.9));
    p.filter.mode = FilterMode::LowPass;
    p.filter.cutoff_hz = rng.range(800.0, 5000.0);
    p.filter.resonance = rng.range(0.1, 0.6);
    p.filter.sweep = -rng.range(1.0, 6.0);
    if rng.chance(0.5) {
        p.fx.drive = rng.range(0.2, 0.7);
    }
    if rng.chance(0.3) {
        p.fx.downsample = rng.range(2.0, 8.0);
    }
    if rng.chance(0.3) {
        p.lfo.amp = rng.range(0.3, 0.8);
        p.lfo.rate_hz = rng.range(8.0, 30.0);
    }
    // A low sine thump under the noise adds body.
    if rng.chance(0.5) {
        p.osc[1].wave = Waveform::Sine;
        p.osc[1].level = rng.range(0.3, 0.8);
        p.osc[1].semitones = rng.range(-24.0, -12.0);
        p.pitch.slide = -rng.range(20.0, 80.0);
    }
    p.gain = 0.7;
    p
}

fn power_up(rng: &mut Rng) -> Patch {
    let wave = if rng.chance(0.5) { Waveform::Saw } else { Waveform::Square };
    let mut p = base(wave, rng.range(55.0, 76.0));
    if rng.chance(0.5) {
        p.pitch.slide = rng.range(20.0, 70.0);
        p.pitch.slide_accel = rng.range(0.0, 120.0);
    } else {
        p.pitch.slide = rng.range(10.0, 40.0);
        p.pitch.vibrato_rate = rng.range(6.0, 16.0);
        p.pitch.vibrato_depth = rng.range(0.5, 3.0);
    }
    if rng.chance(0.5) {
        p.pitch.arp_time = rng.range(0.1, 0.25);
        p.pitch.arp_semitones = [5.0, 7.0, 12.0][rng.next_u32() as usize % 3];
    }
    p.set_sfx_envelope(0.0, rng.range(0.2, 0.45), rng.range(0.15, 0.45));
    p
}

fn hit(rng: &mut Rng) -> Patch {
    let wave = match rng.next_u32() % 3 {
        0 => Waveform::WhiteNoise,
        1 => Waveform::Square,
        _ => Waveform::Saw,
    };
    let mut p = base(wave, rng.range(45.0, 70.0));
    p.pitch.slide = -rng.range(120.0, 300.0);
    p.set_sfx_envelope(0.0, rng.range(0.01, 0.08), rng.range(0.08, 0.25));
    if rng.chance(0.5) {
        p.filter.mode = FilterMode::HighPass;
        p.filter.cutoff_hz = rng.range(100.0, 800.0);
    }
    if wave != Waveform::WhiteNoise && rng.chance(0.5) {
        p.osc[1].wave = Waveform::WhiteNoise;
        p.osc[1].level = rng.range(0.3, 0.7);
    }
    p.fx.drive = rng.range(0.0, 0.4);
    p
}

fn jump(rng: &mut Rng) -> Patch {
    let mut p = base(Waveform::Pulse, rng.range(55.0, 72.0));
    p.osc[0].pulse_width = rng.range(0.25, 0.6);
    p.pitch.slide = rng.range(30.0, 80.0);
    p.set_sfx_envelope(0.0, rng.range(0.1, 0.3), rng.range(0.1, 0.25));
    if rng.chance(0.3) {
        p.filter.mode = FilterMode::HighPass;
        p.filter.cutoff_hz = rng.range(100.0, 600.0);
    }
    if rng.chance(0.3) {
        p.filter.mode = FilterMode::LowPass;
        p.filter.cutoff_hz = rng.range(2000.0, 8000.0);
    }
    p
}

fn blip(rng: &mut Rng) -> Patch {
    let wave = if rng.chance(0.5) { Waveform::Square } else { Waveform::Saw };
    let mut p = base(wave, rng.range(72.0, 96.0));
    p.osc[0].pulse_width = rng.range(0.2, 0.5);
    p.set_sfx_envelope(0.0, rng.range(0.01, 0.08), rng.range(0.02, 0.1));
    p.filter.mode = FilterMode::HighPass;
    p.filter.cutoff_hz = rng.range(100.0, 400.0);
    p
}

fn random(rng: &mut Rng) -> Patch {
    let mut p = base(Waveform::Square, 60.0);
    for &id in ParamId::ALL {
        if matches!(id, ParamId::Gain | ParamId::Polyphony | ParamId::VelocitySens) {
            continue;
        }
        p.set(id, random_value(rng, id.kind()));
    }
    // Keep it a one-shot and audible.
    p.osc[0].level = 1.0;
    p.base_note = rng.range(40.0, 96.0);
    p.set_sfx_envelope(rng.range(0.0, 0.05), rng.range(0.02, 0.4), rng.range(0.05, 0.6));
    // Long feedback tails would keep a one-shot alive for many seconds.
    p.fx.delay_mix *= 0.5;
    p.fx.delay_feedback = p.fx.delay_feedback.min(0.5);
    p.fx.delay_time = p.fx.delay_time.min(0.3);
    if p.filter.mode != FilterMode::Off && rng.chance(0.5) {
        p.filter.mode = FilterMode::Off;
    }
    p
}

fn random_value(rng: &mut Rng, kind: ParamKind) -> f32 {
    match kind {
        ParamKind::Float { min, max, .. } => {
            // Bias towards the low end so extreme values are rare.
            let t = rng.next_f32();
            min + (max - min) * t * t
        }
        ParamKind::Exp { min, max } => min * (max / min).powf(rng.next_f32()),
        ParamKind::Int { min, max } => rng.range(min as f32, max as f32 + 0.999).floor(),
        ParamKind::Enum(names) => (rng.next_u32() as usize % names.len()) as f32,
    }
}

/// Randomly perturb every parameter of `patch` by up to `amount` (0..1) of its range.
/// Useful for "mutate" buttons and per-instance variation of a sound effect.
pub fn mutate(patch: &mut Patch, rng: &mut Rng, amount: f32) {
    let amount = amount.clamp(0.0, 1.0);
    for &id in ParamId::ALL {
        if matches!(id, ParamId::Gain | ParamId::Polyphony) || !rng.chance(0.5) {
            continue;
        }
        let v = patch.get(id);
        let nv = match id.kind() {
            ParamKind::Float { min, max, .. } => v + (max - min) * amount * 0.1 * rng.next_bipolar(),
            ParamKind::Exp { .. } => v * (1.0 + amount * 0.5 * rng.next_bipolar()),
            ParamKind::Int { .. } | ParamKind::Enum(_) => {
                if rng.chance(amount * 0.2) {
                    v + rng.range(-1.5, 1.5).round()
                } else {
                    v
                }
            }
        };
        patch.set(id, nv);
    }
}
