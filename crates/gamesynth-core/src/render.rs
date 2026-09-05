//! Offline rendering helpers for tests, tooling and baking sounds to samples.

use crate::patch::Patch;
use crate::synth::{Command, Synth};

/// Render a one-shot of `patch` (its base note / duration) to mono samples.
/// Rendering stops once the synth is silent or `max_secs` is reached.
pub fn render_one_shot(patch: &Patch, sample_rate: f32, max_secs: f32) -> Vec<f32> {
    let mut synth = Synth::with_patch(sample_rate, *patch);
    synth.trigger();
    render_until_silent(&mut synth, max_secs)
}

/// Render `events` (time in seconds, command) and keep going until silence or `max_secs`.
pub fn render_events(patch: &Patch, sample_rate: f32, events: &[(f32, Command)], max_secs: f32) -> Vec<f32> {
    let mut synth = Synth::with_patch(sample_rate, *patch);
    let mut out = Vec::new();
    let mut block = [0.0f32; 256];
    let mut next = 0;
    let max_samples = (max_secs * sample_rate) as usize;
    let last_event = events.last().map(|e| (e.0 * sample_rate) as usize).unwrap_or(0);
    while out.len() < max_samples {
        let now = out.len();
        while next < events.len() && (events[next].0 * sample_rate) as usize <= now {
            synth.apply(events[next].1);
            next += 1;
        }
        let n = if next < events.len() {
            ((events[next].0 * sample_rate) as usize - now).clamp(1, block.len())
        } else {
            block.len()
        };
        synth.render_mono(&mut block[..n]);
        out.extend_from_slice(&block[..n]);
        if next >= events.len() && now > last_event && synth.is_silent() {
            break;
        }
    }
    out
}

fn render_until_silent(synth: &mut Synth, max_secs: f32) -> Vec<f32> {
    let max_samples = (max_secs * synth.sample_rate()) as usize;
    let mut out = Vec::with_capacity(max_samples.min(1 << 20));
    let mut block = [0.0f32; 256];
    while out.len() < max_samples {
        synth.render_mono(&mut block);
        out.extend_from_slice(&block);
        if synth.is_silent() {
            break;
        }
    }
    out
}

/// Peak absolute sample value.
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Root mean square level.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}
