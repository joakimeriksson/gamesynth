//! Godot 4 GDExtension for `gamesynth-core`.
//!
//! Exposes to Godot:
//! * [`SynthPatch`] — a `Resource` describing a synth sound (inspector-editable, `.tres`).
//! * [`SynthStream`] / [`SynthStreamPlayback`] — an `AudioStream` playing a `SynthPatch`
//!   as a one-shot effect or as an instrument driven by `note_on` etc.
//! * [`JetEnginePatch`] — parameters of a procedural jet engine.
//! * [`JetEngineStream`] / [`JetEnginePlayback`] — a continuous vehicle engine driven by
//!   throttle / boost / speed / damage every frame.

use godot::prelude::*;

mod jet;
mod patch;
mod props;
mod shared;
mod stream;

pub use jet::{JetEnginePatch, JetEnginePlayback, JetEngineStream};
pub use patch::SynthPatch;
pub use stream::{SynthStream, SynthStreamPlayback};

struct GameSynthExtension;

#[gdextension]
unsafe impl ExtensionLibrary for GameSynthExtension {}
