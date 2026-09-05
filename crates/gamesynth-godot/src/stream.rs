use std::sync::Arc;

use gamesynth_core::{Command, ParamId, Patch, Synth};
use godot::classes::native::AudioFrame;
use godot::classes::{AudioServer, AudioStream, AudioStreamPlayback, IAudioStream, IAudioStreamPlayback};
use godot::prelude::*;
use godot_core::meta::RawPtr;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::patch::SynthPatch;
use crate::shared::{fill_frames_stereo, Shared};

/// Commands buffered per playback between the game thread and the audio thread.
const COMMAND_QUEUE_LEN: usize = 512;

/// An `AudioStream` that synthesizes sound from a `SynthPatch` in real time.
///
/// Assign it to any `AudioStreamPlayer` (2D/3D included). With `one_shot` enabled,
/// `player.play()` fires the patch like a sample. With it disabled the stream stays
/// alive and you drive notes through `player.get_stream_playback()`.
#[derive(GodotClass)]
#[class(base=AudioStream, tool)]
pub struct SynthStream {
    /// The sound to play. Edits to the resource are picked up by playing instances live.
    #[export]
    #[var(get = get_patch, set = set_patch)]
    patch: Option<Gd<SynthPatch>>,
    /// If true, `play()` triggers the patch's base note for its duration and the player
    /// finishes when the sound has died out. If false, the stream plays silence until
    /// notes are sent to the playback and keeps running until `stop()`.
    #[export]
    one_shot: bool,
    shared: Arc<Shared<Patch>>,
    base: Base<AudioStream>,
}

#[godot_api]
impl SynthStream {
    #[func]
    fn get_patch(&self) -> Option<Gd<SynthPatch>> {
        self.patch.clone()
    }

    #[func]
    fn set_patch(&mut self, patch: Option<Gd<SynthPatch>>) {
        let callable = self.base().callable("_on_patch_changed");
        if let Some(old) = &mut self.patch {
            if old.is_connected("changed", &callable) {
                old.disconnect("changed", &callable);
            }
        }
        if let Some(new) = &mut patch.clone() {
            new.connect("changed", &callable);
        }
        self.patch = patch;
        self.publish_patch();
    }

    #[func]
    fn _on_patch_changed(&mut self) {
        self.publish_patch();
    }

    /// Convenience: create a one-shot stream for a sound effect preset.
    #[func]
    fn from_preset(preset: GString, seed: i64) -> Gd<SynthStream> {
        let patch = SynthPatch::from_preset(preset, seed);
        let mut stream = SynthStream::new_gd();
        stream.bind_mut().set_patch(Some(patch));
        stream
    }

    fn publish_patch(&self) {
        let patch = self.patch.as_ref().map(|p| p.bind().patch()).unwrap_or_default();
        self.shared.publish(patch);
    }
}

#[godot_api]
impl IAudioStream for SynthStream {
    fn init(base: Base<AudioStream>) -> Self {
        SynthStream { patch: None, one_shot: true, shared: Shared::new(Patch::default()), base }
    }

    fn instantiate_playback(&self) -> Option<Gd<AudioStreamPlayback>> {
        let rate = AudioServer::singleton().get_mix_rate();
        Some(SynthStreamPlayback::create(rate, self.shared.clone(), self.one_shot).upcast())
    }

    fn get_stream_name(&self) -> GString {
        "SynthStream".into()
    }

    fn get_length(&self) -> f64 {
        if !self.one_shot {
            return 0.0;
        }
        self.shared.snapshot().0.one_shot_length().map(|l| l as f64).unwrap_or(0.0)
    }

    fn is_monophonic(&self) -> bool {
        true
    }
}

/// Live playback of a `SynthStream`. Obtain it with `player.get_stream_playback()` after
/// `player.play()`, then send notes. All methods are safe to call from the main thread
/// while audio is rendering.
#[derive(GodotClass)]
#[class(base=AudioStreamPlayback, no_init)]
pub struct SynthStreamPlayback {
    synth: Synth,
    tx: Producer<Command>,
    rx: Consumer<Command>,
    shared: Arc<Shared<Patch>>,
    patch_version: u32,
    one_shot: bool,
    playing: bool,
    frames_rendered: u64,
    base: Base<AudioStreamPlayback>,
}

impl SynthStreamPlayback {
    fn create(sample_rate: f32, shared: Arc<Shared<Patch>>, one_shot: bool) -> Gd<Self> {
        let (tx, rx) = RingBuffer::new(COMMAND_QUEUE_LEN);
        let (patch, patch_version) = shared.snapshot();
        Gd::from_init_fn(|base| SynthStreamPlayback {
            synth: Synth::with_patch(sample_rate, patch),
            tx,
            rx,
            shared,
            patch_version,
            one_shot,
            playing: false,
            frames_rendered: 0,
            base,
        })
    }

    fn send(&mut self, cmd: Command) {
        if self.tx.push(cmd).is_err() {
            godot_warn!("SynthStreamPlayback: command queue full, dropped {cmd:?}");
        }
    }

    fn sync_patch(&mut self) {
        if let Some(p) = self.shared.poll(&mut self.patch_version) {
            self.synth.set_patch(p);
        }
    }
}

#[godot_api]
impl SynthStreamPlayback {
    /// Start a note. `note` is a MIDI note number (60 = middle C, fractions allowed),
    /// `velocity` is 0..1.
    #[func]
    fn note_on(&mut self, note: f64, velocity: f64) {
        self.send(Command::NoteOn { note: note as f32, velocity: velocity as f32, duration: 0.0 });
    }

    /// Release a note previously started with `note_on`.
    #[func]
    fn note_off(&mut self, note: f64) {
        self.send(Command::NoteOff { note: note as f32 });
    }

    /// Start a note that releases itself after `duration` seconds.
    #[func]
    fn play_note(&mut self, note: f64, velocity: f64, duration: f64) {
        self.send(Command::NoteOn { note: note as f32, velocity: velocity as f32, duration: duration as f32 });
    }

    /// Fire the patch as a one-shot (its `master/base_note` for `master/duration` seconds).
    #[func]
    fn trigger(&mut self) {
        self.send(Command::Trigger);
    }

    /// Release every held note.
    #[func]
    fn all_notes_off(&mut self) {
        self.send(Command::AllNotesOff);
    }

    /// Silence immediately, cutting release tails.
    #[func]
    fn panic(&mut self) {
        self.send(Command::Panic);
    }

    /// Pitch bend -1..1, scaled by the patch's `pitch/bend_range`.
    #[func]
    fn set_pitch_bend(&mut self, bend: f64) {
        self.send(Command::PitchBend(bend as f32));
    }

    /// Extra gain multiplier (0..2) on top of the patch's `master/gain`.
    #[func]
    fn set_master_gain(&mut self, gain: f64) {
        self.send(Command::MasterGain(gain as f32));
    }

    /// Change one parameter on this playback only (the `SynthPatch` resource is untouched).
    #[func]
    fn set_param(&mut self, name: GString, value: f64) -> bool {
        match ParamId::from_name(&name.to_string()) {
            Some(id) => {
                self.send(Command::SetParam(id, value as f32));
                true
            }
            None => {
                godot_warn!("SynthStreamPlayback.set_param: unknown parameter '{name}'");
                false
            }
        }
    }

    /// Replace the whole patch on this playback only.
    #[func]
    fn set_patch(&mut self, patch: Gd<SynthPatch>) {
        let p = patch.bind().patch();
        // Stop following the shared resource; this playback now owns its patch.
        self.patch_version = self.shared.version();
        self.send(Command::SetPatch(p));
    }

    /// Number of voices currently sounding.
    #[func]
    fn get_active_voices(&self) -> i64 {
        self.synth.active_voices() as i64
    }

    /// Recent output peak 0..1 (decays over ~250 ms), handy for meters.
    #[func]
    fn get_peak(&self) -> f64 {
        self.synth.peak() as f64
    }
}

#[godot_api]
impl IAudioStreamPlayback for SynthStreamPlayback {
    fn start(&mut self, _from_pos: f64) {
        self.sync_patch();
        self.synth.panic();
        self.frames_rendered = 0;
        self.playing = true;
        if self.one_shot {
            self.synth.trigger();
        }
    }

    fn stop(&mut self) {
        self.playing = false;
        self.synth.panic();
    }

    fn is_playing(&self) -> bool {
        self.playing
    }

    fn get_loop_count(&self) -> i32 {
        0
    }

    fn get_playback_position(&self) -> f64 {
        self.frames_rendered as f64 / self.synth.sample_rate() as f64
    }

    fn seek(&mut self, _position: f64) {}

    unsafe fn mix_rawptr(&mut self, buffer: RawPtr<*mut AudioFrame>, rate_scale: f32, frames: i32) -> i32 {
        let ptr = buffer.ptr();
        if ptr.is_null() || frames <= 0 {
            return 0;
        }
        let frames = frames as usize;

        while let Ok(cmd) = self.rx.pop() {
            self.synth.apply(cmd);
        }
        self.sync_patch();
        // The player's pitch_scale arrives as a resampling ratio; apply it as transposition.
        let transpose = if rate_scale > 0.0 && (rate_scale - 1.0).abs() > 1e-4 { 12.0 * rate_scale.log2() } else { 0.0 };
        self.synth.set_transpose(transpose);

        let synth = &mut self.synth;
        // SAFETY: Godot guarantees `buffer` holds at least `frames` AudioFrames.
        unsafe { fill_frames_stereo(ptr, frames, |block| synth.render(block)) };
        self.frames_rendered += frames as u64;

        if self.one_shot && self.synth.is_silent() && self.frames_rendered as f32 > self.synth.sample_rate() * 0.05 {
            self.playing = false;
        }
        frames as i32
    }
}
