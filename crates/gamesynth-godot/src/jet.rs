use std::sync::Arc;

use gamesynth_core::{JetCommand, JetEngine, JetParamId, JetParams, JetPreset};
use godot::classes::native::AudioFrame;
use godot::classes::{AudioServer, AudioStream, AudioStreamPlayback, IAudioStream, IAudioStreamPlayback, IResource, Resource};
use godot::prelude::*;
use godot::register::info::PropertyInfo;
use godot_core::meta::RawPtr;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::props;
use crate::shared::{fill_frames, Shared};

const COMMAND_QUEUE_LEN: usize = 1024;
const PROP_PRESET: &str = "preset/name";
const PROP_APPLY: &str = "preset/apply";

/// Parameters of a procedural jet / turbine engine (see `JetEngineStream`).
///
/// Pick `preset/name` and press **Apply** for a starting point, then tune. Parameters are
/// addressed by names such as `"whine/hz"`; see `get_param_names()`.
#[derive(GodotClass)]
#[class(base=Resource, tool)]
pub struct JetEnginePatch {
    params: JetParams,
    preset: i64,
    base: Base<Resource>,
}

impl JetEnginePatch {
    pub fn params(&self) -> JetParams {
        self.params
    }

    fn changed(&mut self) {
        self.base_mut().emit_changed();
    }

    fn replace(&mut self, params: JetParams) {
        self.params = params;
        self.changed();
        self.base_mut().notify_property_list_changed();
    }
}

#[godot_api]
impl JetEnginePatch {
    /// Create a patch from a preset name: `Racer`, `Heavy`, `Turbine`, `Scramjet`.
    #[func]
    pub fn from_preset(preset: GString) -> Gd<JetEnginePatch> {
        let preset = JetPreset::from_name(&preset.to_string()).unwrap_or_else(|| {
            godot_warn!("JetEnginePatch.from_preset: unknown preset '{preset}', using Racer");
            JetPreset::Racer
        });
        Gd::from_init_fn(|base| JetEnginePatch { params: preset.params(), preset: preset as i64, base })
    }

    #[func]
    fn get_preset_names() -> PackedStringArray {
        JetPreset::NAMES.iter().map(|n| GString::from(*n)).collect()
    }

    #[func]
    fn get_param_names() -> PackedStringArray {
        props::param_names::<JetParams>()
    }

    /// Set a parameter by name (clamped to its range). Returns false for an unknown name.
    #[func]
    fn set_param(&mut self, name: GString, value: f64) -> bool {
        match props::set_param(&mut self.params, &name.to_string(), &value.to_variant()) {
            Some(true) => {
                self.changed();
                true
            }
            _ => {
                godot_warn!("JetEnginePatch.set_param: unknown parameter '{name}'");
                false
            }
        }
    }

    #[func]
    fn get_param(&self, name: GString) -> f64 {
        props::get_param_f32(&self.params, &name.to_string()).map(|v| v as f64).unwrap_or(0.0)
    }

    #[func]
    fn to_json(&self) -> GString {
        GString::from(&serde_json::to_string_pretty(&self.params).unwrap_or_default())
    }

    /// Load parameters from JSON (missing fields keep defaults). Returns false on error.
    #[func]
    #[allow(clippy::wrong_self_convention)]
    fn from_json(&mut self, json: GString) -> bool {
        match serde_json::from_str::<JetParams>(&json.to_string()) {
            Ok(p) => {
                self.replace(p);
                true
            }
            Err(e) => {
                godot_error!("JetEnginePatch.from_json: {e}");
                false
            }
        }
    }

    /// Load the parameters of `preset/name`.
    #[func]
    fn apply_preset(&mut self) {
        let preset = JetPreset::from_index(self.preset.max(0) as usize);
        self.replace(preset.params());
    }
}

#[godot_api]
impl IResource for JetEnginePatch {
    fn init(base: Base<Resource>) -> Self {
        JetEnginePatch { params: JetParams::default(), preset: JetPreset::Racer as i64, base }
    }

    fn on_get_property_list(&mut self) -> Vec<PropertyInfo> {
        let mut list = vec![
            props::enum_property(PROP_PRESET, &JetPreset::NAMES),
            props::tool_button(PROP_APPLY, "Apply preset"),
        ];
        list.extend(props::param_properties::<JetParams>());
        list
    }

    fn on_get(&self, property: StringName) -> Option<Variant> {
        let name = property.to_string();
        match name.as_str() {
            PROP_PRESET => Some(self.preset.to_variant()),
            PROP_APPLY => Some(self.base().callable("apply_preset").to_variant()),
            _ => props::get_param(&self.params, &name),
        }
    }

    fn on_set(&mut self, property: StringName, value: Variant) -> bool {
        let name = property.to_string();
        match name.as_str() {
            PROP_PRESET => {
                self.preset = value.try_to::<i64>().unwrap_or(0).clamp(0, JetPreset::NAMES.len() as i64 - 1);
                true
            }
            PROP_APPLY => true,
            _ => match props::set_param(&mut self.params, &name, &value) {
                Some(applied) => {
                    if applied {
                        self.changed();
                    }
                    true
                }
                None => false,
            },
        }
    }

    fn on_property_get_revert(&self, property: StringName) -> Option<Variant> {
        let name = property.to_string();
        match name.as_str() {
            PROP_PRESET => Some((JetPreset::Racer as i64).to_variant()),
            _ => props::default_param::<JetParams>(&name),
        }
    }
}

/// A continuous, procedurally generated jet engine. Attach to an `AudioStreamPlayer3D`
/// on each vehicle, call `play()` once, then drive it every frame through
/// `player.get_stream_playback()` (`set_throttle`, `set_boost`, `set_speed`, `set_damage`).
///
/// Godot handles distance attenuation and doppler; the engine handles RPM inertia
/// (spool-up/down), pitch and timbre.
#[derive(GodotClass)]
#[class(base=AudioStream, tool)]
pub struct JetEngineStream {
    /// Engine character. Edits are picked up by running instances live.
    #[export]
    #[var(get = get_patch, set = set_patch)]
    patch: Option<Gd<JetEnginePatch>>,
    /// Throttle applied when playback starts (before the game sends its first update).
    #[export(range = (0.0, 1.0))]
    initial_throttle: f32,
    /// If true, playback starts at the initial throttle's RPM instead of spooling up from idle.
    #[export]
    start_spooled: bool,
    shared: Arc<Shared<JetParams>>,
    base: Base<AudioStream>,
}

#[godot_api]
impl JetEngineStream {
    #[func]
    fn get_patch(&self) -> Option<Gd<JetEnginePatch>> {
        self.patch.clone()
    }

    #[func]
    fn set_patch(&mut self, patch: Option<Gd<JetEnginePatch>>) {
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
        self.publish();
    }

    #[func]
    fn _on_patch_changed(&mut self) {
        self.publish();
    }

    /// Convenience: create a stream for a preset (`Racer`, `Heavy`, `Turbine`, `Scramjet`).
    #[func]
    fn from_preset(preset: GString) -> Gd<JetEngineStream> {
        let patch = JetEnginePatch::from_preset(preset);
        let mut stream = JetEngineStream::new_gd();
        stream.bind_mut().set_patch(Some(patch));
        stream
    }

    fn publish(&self) {
        let params = self.patch.as_ref().map(|p| p.bind().params()).unwrap_or_default();
        self.shared.publish(params);
    }
}

#[godot_api]
impl IAudioStream for JetEngineStream {
    fn init(base: Base<AudioStream>) -> Self {
        JetEngineStream {
            patch: None,
            initial_throttle: 0.0,
            start_spooled: false,
            shared: Shared::new(JetParams::default()),
            base,
        }
    }

    fn instantiate_playback(&self) -> Option<Gd<AudioStreamPlayback>> {
        let rate = AudioServer::singleton().get_mix_rate();
        let pb = JetEnginePlayback::create(rate, self.shared.clone(), self.initial_throttle, self.start_spooled);
        Some(pb.upcast())
    }

    fn get_stream_name(&self) -> GString {
        "JetEngineStream".into()
    }

    fn is_monophonic(&self) -> bool {
        true
    }
}

/// Live jet engine instance; drive it every frame. All methods are main-thread safe while
/// audio is rendering.
#[derive(GodotClass)]
#[class(base=AudioStreamPlayback, no_init)]
pub struct JetEnginePlayback {
    engine: JetEngine,
    tx: Producer<JetCommand>,
    rx: Consumer<JetCommand>,
    shared: Arc<Shared<JetParams>>,
    params_version: u32,
    initial_throttle: f32,
    start_spooled: bool,
    playing: bool,
    frames_rendered: u64,
    base: Base<AudioStreamPlayback>,
}

impl JetEnginePlayback {
    fn create(sample_rate: f32, shared: Arc<Shared<JetParams>>, initial_throttle: f32, start_spooled: bool) -> Gd<Self> {
        let (tx, rx) = RingBuffer::new(COMMAND_QUEUE_LEN);
        let (params, params_version) = shared.snapshot();
        Gd::from_init_fn(|base| JetEnginePlayback {
            engine: JetEngine::new(sample_rate, params),
            tx,
            rx,
            shared,
            params_version,
            initial_throttle,
            start_spooled,
            playing: false,
            frames_rendered: 0,
            base,
        })
    }

    fn send(&mut self, cmd: JetCommand) {
        if self.tx.push(cmd).is_err() {
            godot_warn!("JetEnginePlayback: command queue full, dropped {cmd:?}");
        }
    }
}

#[godot_api]
impl JetEnginePlayback {
    /// Throttle 0..1. RPM follows with the patch's spool times.
    #[func]
    fn set_throttle(&mut self, throttle: f64) {
        self.send(JetCommand::Throttle(throttle as f32));
    }

    /// Afterburner 0..1: adds the burner layer, more drive and RPM over-speed.
    #[func]
    fn set_boost(&mut self, boost: f64) {
        self.send(JetCommand::Boost(boost as f32));
    }

    /// Normalised airspeed 0..1, drives the wind layer.
    #[func]
    fn set_speed(&mut self, speed: f64) {
        self.send(JetCommand::Speed(speed as f32));
    }

    /// Damage 0..1: random sputtering drop-outs.
    #[func]
    fn set_damage(&mut self, damage: f64) {
        self.send(JetCommand::Damage(damage as f32));
    }

    /// Set throttle, boost, speed and damage in one call (one queue entry per frame).
    #[func]
    fn set_state(&mut self, throttle: f64, boost: f64, speed: f64, damage: f64) {
        self.send(JetCommand::Throttle(throttle as f32));
        self.send(JetCommand::Boost(boost as f32));
        self.send(JetCommand::Speed(speed as f32));
        self.send(JetCommand::Damage(damage as f32));
    }

    /// Jump RPM straight to the current throttle (e.g. spawning a ship already at speed).
    #[func]
    fn snap_rpm(&mut self) {
        self.send(JetCommand::SnapRpm);
    }

    /// Extra gain multiplier 0..2 on top of the patch's `master/gain`.
    #[func]
    fn set_master_gain(&mut self, gain: f64) {
        self.send(JetCommand::MasterGain(gain as f32));
    }

    /// Change one parameter on this instance only.
    #[func]
    fn set_param(&mut self, name: GString, value: f64) -> bool {
        match JetParamId::from_name(&name.to_string()) {
            Some(id) => {
                self.send(JetCommand::SetParam(id, value as f32));
                true
            }
            None => {
                godot_warn!("JetEnginePlayback.set_param: unknown parameter '{name}'");
                false
            }
        }
    }

    /// Replace all parameters on this instance only.
    #[func]
    fn set_patch(&mut self, patch: Gd<JetEnginePatch>) {
        let p = patch.bind().params();
        self.params_version = self.shared.version();
        self.send(JetCommand::SetParams(p));
    }

    /// Current RPM fraction (idle_rpm .. boost_rpm) — drive gauges, controller rumble, VFX.
    #[func]
    fn get_rpm(&self) -> f64 {
        self.engine.rpm() as f64
    }

    /// Recent output peak 0..1 (decays over ~250 ms).
    #[func]
    fn get_peak(&self) -> f64 {
        self.engine.peak() as f64
    }
}

#[godot_api]
impl IAudioStreamPlayback for JetEnginePlayback {
    fn start(&mut self, _from_pos: f64) {
        if let Some(p) = self.shared.poll(&mut self.params_version) {
            self.engine.set_params(p);
        }
        self.engine.set_throttle(self.initial_throttle);
        if self.start_spooled {
            self.engine.snap_rpm();
        }
        self.frames_rendered = 0;
        self.playing = true;
    }

    fn stop(&mut self) {
        self.playing = false;
    }

    fn is_playing(&self) -> bool {
        self.playing
    }

    fn get_loop_count(&self) -> i32 {
        0
    }

    fn get_playback_position(&self) -> f64 {
        self.frames_rendered as f64 / self.engine.sample_rate() as f64
    }

    fn seek(&mut self, _position: f64) {}

    unsafe fn mix_rawptr(&mut self, buffer: RawPtr<*mut AudioFrame>, _rate_scale: f32, frames: i32) -> i32 {
        let ptr = buffer.ptr();
        if ptr.is_null() || frames <= 0 {
            return 0;
        }
        let frames = frames as usize;
        while let Ok(cmd) = self.rx.pop() {
            self.engine.apply(cmd);
        }
        if let Some(p) = self.shared.poll(&mut self.params_version) {
            self.engine.set_params(p);
        }
        let engine = &mut self.engine;
        // SAFETY: Godot guarantees `buffer` holds at least `frames` AudioFrames.
        unsafe { fill_frames(ptr, frames, |block| engine.render_mono(block)) };
        self.frames_rendered += frames as u64;
        frames as i32
    }
}
