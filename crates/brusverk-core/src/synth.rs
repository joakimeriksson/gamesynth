//! Polyphonic synthesizer: voice allocation, master effects, command handling.

use crate::effects::Effects;
use crate::math::limit;
use crate::patch::{ParamId, Patch};
use crate::voice::Voice;

pub const MAX_VOICES: usize = 32;
/// Internal processing chunk; larger output buffers are processed in pieces of this size.
pub const MAX_BLOCK: usize = 256;

/// One interleaved stereo sample. Layout-compatible with Godot's `AudioFrame`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct StereoFrame {
    pub left: f32,
    pub right: f32,
}

/// Everything the game thread can ask the synth to do. `Copy` so it can travel through a
/// lock-free queue without allocation.
// `SetPatch` carries the whole patch by value on purpose: no heap, no sharing with the
// audio thread.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Start a note. `duration` > 0 auto-releases after that many seconds.
    NoteOn { note: f32, velocity: f32, duration: f32 },
    NoteOff { note: f32 },
    /// Play the patch's `base_note` for its `duration` (one-shot sound effect).
    Trigger,
    /// Release all held notes.
    AllNotesOff,
    /// Stop everything immediately (no release tails).
    Panic,
    SetPatch(Patch),
    SetParam(ParamId, f32),
    /// -1..1, scaled by `pitch/bend_range`.
    PitchBend(f32),
    /// Extra master gain multiplier 0..2 (independent of the patch).
    MasterGain(f32),
}

pub struct Synth {
    sample_rate: f32,
    patch: Patch,
    voices: [Voice; MAX_VOICES],
    counter: u64,
    pitch_bend: f32,
    transpose: f32,
    master_gain: f32,
    fx: Effects,
    peak: f32,
    peak_decay: f32,
}

impl Synth {
    /// Allocates the effect buffers; do this on the game thread, not in the audio callback.
    pub fn new(sample_rate: f32) -> Self {
        let mut s = Synth {
            sample_rate,
            patch: Patch::default(),
            voices: core::array::from_fn(|i| Voice::new(0x1000_0001u32.wrapping_mul(i as u32 + 1))),
            counter: 0,
            pitch_bend: 0.0,
            transpose: 0.0,
            master_gain: 1.0,
            fx: Effects::new(sample_rate),
            peak: 0.0,
            peak_decay: 1.0,
        };
        s.set_sample_rate(sample_rate);
        s
    }

    pub fn with_patch(sample_rate: f32, patch: Patch) -> Self {
        let mut s = Self::new(sample_rate);
        s.patch = patch;
        s
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Changing the rate reallocates the delay line if needed; call from the game thread.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        if (self.fx.sample_rate() - sample_rate).abs() > 0.5 {
            self.fx = Effects::new(sample_rate);
        }
        // Peak meter falls from 1.0 to silence (-80 dB) in a quarter second.
        self.peak_decay = (-9.21 / (0.25 * sample_rate)).exp();
        self.panic();
    }

    pub fn patch(&self) -> &Patch {
        &self.patch
    }

    pub fn patch_mut(&mut self) -> &mut Patch {
        &mut self.patch
    }

    pub fn set_patch(&mut self, patch: Patch) {
        self.patch = patch;
    }

    pub fn set_param(&mut self, id: ParamId, value: f32) {
        self.patch.set(id, value);
    }

    pub fn get_param(&self, id: ParamId) -> f32 {
        self.patch.get(id)
    }

    pub fn set_pitch_bend(&mut self, bend: f32) {
        self.pitch_bend = bend.clamp(-1.0, 1.0);
    }

    /// Global transposition in semitones applied to every voice (e.g. host pitch scaling).
    pub fn set_transpose(&mut self, semitones: f32) {
        self.transpose = if semitones.is_finite() { semitones.clamp(-48.0, 48.0) } else { 0.0 };
    }

    pub fn set_master_gain(&mut self, gain: f32) {
        self.master_gain = gain.clamp(0.0, 2.0);
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.is_active()).count()
    }

    /// True when no voice is sounding and the effect tail has faded out.
    pub fn is_silent(&self) -> bool {
        self.peak < 1e-4 && self.voices.iter().all(|v| !v.is_active())
    }

    /// Recent output peak level (0..1), decays over ~250 ms. Useful for meters.
    pub fn peak(&self) -> f32 {
        self.peak
    }

    pub fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::NoteOn { note, velocity, duration } => self.play(note, velocity, duration),
            Command::NoteOff { note } => self.note_off(note),
            Command::Trigger => self.trigger(),
            Command::AllNotesOff => self.all_notes_off(),
            Command::Panic => self.panic(),
            Command::SetPatch(p) => self.set_patch(p),
            Command::SetParam(id, v) => self.set_param(id, v),
            Command::PitchBend(b) => self.set_pitch_bend(b),
            Command::MasterGain(g) => self.set_master_gain(g),
        }
    }

    pub fn note_on(&mut self, note: f32, velocity: f32) {
        self.play(note, velocity, 0.0);
    }

    /// Start a note that auto-releases after `duration` seconds (0 = hold until note-off).
    pub fn play(&mut self, note: f32, velocity: f32, duration: f32) {
        self.counter += 1;
        let poly = (self.patch.polyphony.max(1) as usize).min(MAX_VOICES);
        let legato = poly == 1;
        let idx = self.allocate(poly);
        let (patch, sr, age) = (self.patch, self.sample_rate, self.counter);
        self.voices[idx].note_on(&patch, sr, note, velocity, duration, age, legato);
    }

    /// Play the patch's base note for its configured duration.
    pub fn trigger(&mut self) {
        let (note, duration) = (self.patch.base_note, self.patch.duration);
        self.play(note, 1.0, duration);
    }

    pub fn note_off(&mut self, note: f32) {
        let key = note.round() as i32;
        for v in self.voices.iter_mut() {
            if v.is_active() && v.key() == key && !v.is_released() {
                v.release();
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut() {
            if v.is_active() {
                v.release();
            }
        }
    }

    pub fn panic(&mut self) {
        for v in self.voices.iter_mut() {
            v.kill();
        }
        self.fx.reset();
        self.peak = 0.0;
    }

    /// Pick a voice index within the first `poly` slots: free first, then the quietest
    /// released voice, then the oldest.
    fn allocate(&self, poly: usize) -> usize {
        let pool = &self.voices[..poly];
        if let Some(i) = pool.iter().position(|v| !v.is_active()) {
            return i;
        }
        let mut best = 0;
        let mut best_score = f32::INFINITY;
        for (i, v) in pool.iter().enumerate() {
            // Released voices score by level (quiet first); held voices by age (old first).
            let score = if v.is_released() { v.level() } else { 1.0 + v.age() as f32 };
            if score < best_score {
                best_score = score;
                best = i;
            }
        }
        best
    }

    /// Render mono audio into `out`, overwriting it.
    pub fn render_mono(&mut self, out: &mut [f32]) {
        for chunk in out.chunks_mut(MAX_BLOCK) {
            chunk.iter_mut().for_each(|s| *s = 0.0);
            self.render_chunk(chunk);
        }
    }

    /// Render stereo audio into `out`, overwriting it.
    pub fn render(&mut self, out: &mut [StereoFrame]) {
        let mut scratch = [0.0f32; MAX_BLOCK];
        for chunk in out.chunks_mut(MAX_BLOCK) {
            let n = chunk.len();
            scratch[..n].iter_mut().for_each(|s| *s = 0.0);
            self.render_chunk(&mut scratch[..n]);
            for (o, s) in chunk.iter_mut().zip(scratch[..n].iter()) {
                o.left = *s;
                o.right = *s;
            }
        }
    }

    fn render_chunk(&mut self, buf: &mut [f32]) {
        let (patch, sr) = (self.patch, self.sample_rate);
        let bend = self.pitch_bend * patch.pitch.bend_range + self.transpose;
        for v in self.voices.iter_mut() {
            v.render(&patch, sr, bend, buf);
        }
        self.fx.process(&patch.fx, buf);
        let gain = patch.gain * self.master_gain;
        let mut peak = self.peak;
        let decay = self.peak_decay;
        for s in buf.iter_mut() {
            let y = limit(*s * gain);
            *s = y;
            peak = (peak * decay).max(y.abs());
        }
        self.peak = if peak.is_finite() { peak } else { 0.0 };
    }
}
