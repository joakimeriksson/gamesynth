//! Godot 4 GDExtension for `gamesynth-core`.
//!
//! Exposes to Godot:
//! * [`SynthPatch`] — a `Resource` describing a synth sound (inspector-editable, `.tres`).
//! * [`SynthStream`] / [`SynthStreamPlayback`] — an `AudioStream` playing a `SynthPatch`
//!   as a one-shot effect or as an instrument driven by `note_on` etc.
//! * [`JetEnginePatch`] — parameters of a procedural jet engine.
//! * [`JetEngineStream`] / [`JetEnginePlayback`] — a continuous vehicle engine driven by
//!   throttle / boost / speed / damage every frame.
//! * [`SoundGenerator`] / [`SoundGeneratorPlayback`] — any continuous procedural sound: the
//!   built-in generator library (wind, rain, fire, engines, crowds…) or a TOML/JSON model file.

use godot::prelude::*;

mod generator;
mod jet;
mod patch;
mod props;
mod shared;
mod stream;

pub use generator::{SoundGenerator, SoundGeneratorPlayback};
pub use jet::{JetEnginePatch, JetEnginePlayback, JetEngineStream};
pub use patch::SynthPatch;
pub use stream::{SynthStream, SynthStreamPlayback};

struct GameSynthExtension;

#[gdextension]
unsafe impl ExtensionLibrary for GameSynthExtension {}
