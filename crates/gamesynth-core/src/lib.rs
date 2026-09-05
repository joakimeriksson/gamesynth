//! # gamesynth-core
//!
//! A real-time safe software synthesizer for game audio: a polyphonic subtractive synth
//! voice (3 oscillators, state-variable filter, two ADSRs, LFO) with sfxr-style pitch and
//! envelope modulators so the same engine covers procedural sound effects and simple music.
//!
//! The crate has no engine dependencies. Integrations (e.g. the Godot GDExtension in
//! `gamesynth-godot`) own a [`Synth`] on the audio thread and feed it [`Command`]s.
//!
//! ## Real-time guarantees
//! * [`Synth::render`] / [`Synth::render_mono`] never allocate, lock or block.
//! * [`Patch`] and [`Command`] are `Copy` plain data, safe to pass through lock-free queues.
//! * Allocation happens only in [`Synth::new`] / [`Synth::set_sample_rate`].

#![forbid(unsafe_code)]

#[macro_use]
pub mod params;

pub mod effects;
pub mod jet;
pub mod env;
pub mod filter;
pub mod lfo;
pub mod math;
pub mod noise;
pub mod osc;
pub mod patch;
pub mod render;
pub mod sfx;
pub mod synth;
pub mod voice;

pub use env::AdsrParams;
pub use filter::FilterMode;
pub use lfo::{LfoParams, LfoWave};
pub use math::Rng;
pub use osc::Waveform;
pub use jet::{JetCommand, JetEngine, JetParamId, JetParams, JetPreset};
pub use params::{ParamKind, ParamValue, Params};
pub use patch::{FilterParams, FxParams, OscParams, ParamId, Patch, PitchParams};
pub use sfx::SfxPreset;
pub use synth::{Command, StereoFrame, Synth, MAX_BLOCK, MAX_VOICES};
