use std::sync::Arc;

use gamesynth_core::{generators, GraphModel, Model};
use godot::classes::native::AudioFrame;
use godot::classes::{AudioServer, AudioStream, AudioStreamPlayback, FileAccess, IAudioStream, IAudioStreamPlayback};
use godot::prelude::*;
use godot::register::info::{PropertyHint, PropertyInfo};
use godot_core::meta::RawPtr;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::props;
use crate::shared::{fill_frames, Shared};

const COMMAND_QUEUE_LEN: usize = 1024;
const MAX_PARAMS: usize = 64;
const DEFAULT_GENERATOR: &str = "wind";

const PROP_GENERATOR: &str = "generator";
const PROP_CONFIG_FILE: &str = "config_file";
const PROP_CONFIG: &str = "config";
const PROP_PRESET: &str = "preset";
const PROP_START_SNAPPED: &str = "start_snapped";

/// Parameter values shared from the stream to its running playbacks. `generation` changes
/// whenever the model itself changes, so an old playback never applies values meant for a
/// different parameter layout.
#[derive(Clone, Copy)]
struct ParamBlock {
    generation: u32,
    count: usize,
    values: [f32; MAX_PARAMS],
}

enum Cmd {
    Input(usize, f32),
    Param(usize, f32),
    Preset(usize),
    Snap,
}

/// Any continuous procedural sound: wind, rain, fire, engines, crowds, alarms…
///
/// Pick a built-in `generator` (fast, native Rust), or point `config_file` at a TOML/JSON
/// model file to define your own from graph nodes (about 2.5x the CPU). Both expose the same
/// thing: **inputs** your game sets every frame through the playback, and **params** you
/// tune in the inspector.
///
/// ```gdscript
/// var gen := SoundGenerator.new()
/// gen.generator = "rain"
/// player.stream = gen
/// player.play()
/// var pb := player.get_stream_playback() as SoundGeneratorPlayback
/// pb.set_input("intensity", 0.8)
/// ```
///
/// Changing `generator` / `config*` affects playbacks started afterwards; params and presets
/// reach running playbacks live.
#[derive(GodotClass)]
#[class(base=AudioStream, tool)]
pub struct SoundGenerator {
    generator: GString,
    config_file: GString,
    config: GString,
    preset: GString,
    start_snapped: bool,
    /// Prototype instance: owns the description and the current parameter values.
    proto: Option<Box<dyn Model>>,
    /// Source text when the model comes from a file or inline config.
    source: Option<String>,
    error: String,
    generation: u32,
    shared: Arc<Shared<ParamBlock>>,
    base: Base<AudioStream>,
}

impl SoundGenerator {
    fn build(&self, sample_rate: f32) -> Result<Box<dyn Model>, String> {
        match &self.source {
            Some(text) => GraphModel::from_text(text, sample_rate).map(|m| Box::new(m) as Box<dyn Model>).map_err(|e| e.to_string()),
            None => {
                let name = self.generator.to_string();
                generators::create(&name, sample_rate).ok_or_else(|| format!("unknown generator '{name}'. Available: {}", generators::NAMES.join(", ")))
            }
        }
    }

    /// Re-resolve the model source and rebuild the prototype.
    fn rebuild(&mut self) {
        self.source = None;
        self.error.clear();
        if !self.config_file.is_empty() {
            let path = self.config_file.clone();
            if FileAccess::file_exists(&path) {
                self.source = Some(FileAccess::get_file_as_string(&path).to_string());
            } else {
                self.error = format!("config_file '{path}' does not exist");
            }
        } else if !self.config.to_string().trim().is_empty() {
            self.source = Some(self.config.to_string());
        }
        self.proto = None;
        if self.error.is_empty() {
            match self.build(48000.0) {
                Ok(m) if m.desc().params.len() > MAX_PARAMS => self.error = format!("model has more than {MAX_PARAMS} params"),
                Ok(m) => self.proto = Some(m),
                Err(e) => self.error = e,
            }
        }
        if !self.error.is_empty() {
            godot_error!("SoundGenerator: {}", self.error);
        }
        self.preset = GString::new();
        self.generation = self.generation.wrapping_add(1);
        self.publish();
        self.base_mut().notify_property_list_changed();
        self.base_mut().emit_changed();
    }

    fn publish(&self) {
        let mut block = ParamBlock { generation: self.generation, count: 0, values: [0.0; MAX_PARAMS] };
        if let Some(m) = &self.proto {
            block.count = m.desc().params.len().min(MAX_PARAMS);
            for i in 0..block.count {
                block.values[i] = m.param(i);
            }
        }
        self.shared.publish(block);
    }

    fn set_param_index(&mut self, index: usize, value: f32) {
        if let Some(m) = &mut self.proto {
            m.set_param(index, value);
        }
        self.publish();
    }
}

#[godot_api]
impl SoundGenerator {
    /// Names of the built-in generators.
    #[func]
    fn get_generator_names() -> PackedStringArray {
        generators::NAMES.iter().map(|n| GString::from(*n)).collect()
    }

    /// Convenience: `SoundGenerator.create("fire")`.
    #[func]
    fn create(generator: GString) -> Gd<SoundGenerator> {
        let mut g = SoundGenerator::new_gd();
        g.bind_mut().set_generator(generator);
        g
    }

    /// Convenience: load a TOML/JSON model file, e.g. `res://sounds/shield.toml`.
    #[func]
    fn from_file(path: GString) -> Gd<SoundGenerator> {
        let mut g = SoundGenerator::new_gd();
        g.bind_mut().set_config_file(path);
        g
    }

    #[func]
    fn set_generator(&mut self, name: GString) {
        self.generator = name;
        self.rebuild();
    }

    #[func]
    fn get_generator(&self) -> GString {
        self.generator.clone()
    }

    #[func]
    fn set_config_file(&mut self, path: GString) {
        self.config_file = path;
        self.rebuild();
    }

    #[func]
    fn get_config_file(&self) -> GString {
        self.config_file.clone()
    }

    /// Inline model file text (TOML or JSON). Ignored while `config_file` is set.
    #[func]
    fn set_config(&mut self, text: GString) {
        self.config = text;
        self.rebuild();
    }

    #[func]
    fn get_config(&self) -> GString {
        self.config.clone()
    }

    /// Load a named preset of the current model. Returns false if it does not exist.
    #[func]
    fn set_preset(&mut self, name: GString) -> bool {
        let Some(m) = &mut self.proto else { return false };
        let Some(i) = m.desc().preset_index(&name.to_string()) else {
            godot_warn!("SoundGenerator: no preset '{name}'");
            return false;
        };
        m.load_preset(i);
        self.preset = name;
        self.publish();
        self.base_mut().notify_property_list_changed();
        true
    }

    #[func]
    fn get_preset(&self) -> GString {
        self.preset.clone()
    }

    /// Empty when the model loaded fine; otherwise why it did not (bad file, unknown node…).
    #[func]
    fn get_error(&self) -> GString {
        GString::from(&self.error)
    }

    /// True for built-in generators, false for file-defined (graph) models.
    #[func]
    fn is_native(&self) -> bool {
        self.proto.as_ref().is_some_and(|m| m.desc().engine == "native")
    }

    #[func]
    fn get_description(&self) -> GString {
        self.proto.as_ref().map(|m| GString::from(&m.desc().doc)).unwrap_or_default()
    }

    /// Inputs the game can drive, e.g. `["strength", "gustiness"]`.
    #[func]
    fn get_input_names(&self) -> PackedStringArray {
        self.proto.iter().flat_map(|m| m.desc().inputs.iter()).map(|i| GString::from(&i.name)).collect()
    }

    /// Default value of an input (what the model assumes until the game sets it), or 0.
    #[func]
    fn get_input_default(&self, name: GString) -> f64 {
        let name = name.to_string();
        self.proto.iter().flat_map(|m| m.desc().inputs.iter()).find(|i| i.name == name).map(|i| i.default as f64).unwrap_or(0.0)
    }

    #[func]
    fn get_param_names(&self) -> PackedStringArray {
        self.proto.iter().flat_map(|m| m.desc().params.iter()).map(|p| GString::from(&p.name)).collect()
    }

    #[func]
    fn get_preset_names(&self) -> PackedStringArray {
        self.proto.iter().flat_map(|m| m.desc().presets.iter()).map(|p| GString::from(&p.name)).collect()
    }

    /// Set a parameter by name (clamped). Reaches running playbacks live.
    #[func]
    fn set_param(&mut self, name: GString, value: f64) -> bool {
        match self.proto.as_ref().and_then(|m| m.desc().param_index(&name.to_string())) {
            Some(i) => {
                self.set_param_index(i, value as f32);
                true
            }
            None => {
                godot_warn!("SoundGenerator.set_param: unknown parameter '{name}'");
                false
            }
        }
    }

    #[func]
    fn get_param(&self, name: GString) -> f64 {
        self.proto.as_ref().and_then(|m| m.desc().param_index(&name.to_string()).map(|i| m.param(i) as f64)).unwrap_or(0.0)
    }

    /// All parameters as `{"howl/hz": 520.0, ...}` (the web lab's "Copy params JSON" format).
    #[func]
    fn get_params_json(&self) -> GString {
        let map: serde_json::Map<String, serde_json::Value> =
            self.proto.iter().flat_map(|m| m.desc().params.iter().enumerate().map(move |(i, p)| (p.name.clone(), serde_json::json!(m.param(i))))).collect();
        GString::from(&serde_json::to_string_pretty(&map).unwrap_or_default())
    }

    /// Apply parameters from JSON; unknown names are reported and skipped.
    #[func]
    fn set_params_json(&mut self, json: GString) -> bool {
        let Ok(map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&json.to_string()) else {
            godot_error!("SoundGenerator.set_params_json: not a JSON object");
            return false;
        };
        let Some(m) = &mut self.proto else { return false };
        for (name, v) in &map {
            match (m.desc().param_index(name), v.as_f64()) {
                (Some(i), Some(v)) => m.set_param(i, v as f32),
                _ => godot_warn!("SoundGenerator.set_params_json: skipped '{name}'"),
            }
        }
        self.publish();
        self.base_mut().notify_property_list_changed();
        true
    }
}

#[godot_api]
impl IAudioStream for SoundGenerator {
    fn init(base: Base<AudioStream>) -> Self {
        let mut s = SoundGenerator {
            generator: DEFAULT_GENERATOR.into(),
            config_file: GString::new(),
            config: GString::new(),
            preset: GString::new(),
            start_snapped: true,
            proto: None,
            source: None,
            error: String::new(),
            generation: 0,
            shared: Shared::new(ParamBlock { generation: 0, count: 0, values: [0.0; MAX_PARAMS] }),
            base,
        };
        s.proto = s.build(48000.0).ok();
        s.publish();
        s
    }

    fn on_get_property_list(&mut self) -> Vec<PropertyInfo> {
        let mut list = vec![
            props::string_property(PROP_GENERATOR, PropertyHint::ENUM, &generators::NAMES.join(",")),
            props::string_property(PROP_CONFIG_FILE, PropertyHint::FILE, "*.toml,*.json"),
            props::string_property(PROP_CONFIG, PropertyHint::MULTILINE_TEXT, ""),
            props::bool_property(PROP_START_SNAPPED),
        ];
        if let Some(m) = &self.proto {
            let presets: Vec<&str> = m.desc().presets.iter().map(|p| p.name.as_str()).collect();
            list.push(props::string_property(PROP_PRESET, PropertyHint::ENUM, &presets.join(",")));
            list.extend(m.desc().params.iter().map(|p| props::param_property(&p.name, p.kind)));
        }
        list
    }

    fn on_get(&self, property: StringName) -> Option<Variant> {
        let name = property.to_string();
        match name.as_str() {
            PROP_GENERATOR => Some(self.generator.to_variant()),
            PROP_CONFIG_FILE => Some(self.config_file.to_variant()),
            PROP_CONFIG => Some(self.config.to_variant()),
            PROP_PRESET => Some(self.preset.to_variant()),
            PROP_START_SNAPPED => Some(self.start_snapped.to_variant()),
            _ => {
                let m = self.proto.as_ref()?;
                let i = m.desc().param_index(&name)?;
                Some(props::to_variant(m.desc().params[i].kind, m.param(i)))
            }
        }
    }

    fn on_set(&mut self, property: StringName, value: Variant) -> bool {
        let name = property.to_string();
        let text = || value.try_to::<GString>().unwrap_or_default();
        match name.as_str() {
            PROP_GENERATOR => self.set_generator(text()),
            PROP_CONFIG_FILE => self.set_config_file(text()),
            PROP_CONFIG => self.set_config(text()),
            PROP_PRESET => {
                let preset = text();
                if !preset.is_empty() {
                    self.set_preset(preset);
                }
            }
            PROP_START_SNAPPED => self.start_snapped = value.try_to::<bool>().unwrap_or(true),
            _ => {
                let Some(i) = self.proto.as_ref().and_then(|m| m.desc().param_index(&name)) else { return false };
                if let Some(v) = props::variant_to_f32(&value) {
                    self.set_param_index(i, v);
                }
            }
        }
        true
    }

    fn on_property_get_revert(&self, property: StringName) -> Option<Variant> {
        let name = property.to_string();
        match name.as_str() {
            PROP_GENERATOR => Some(GString::from(DEFAULT_GENERATOR).to_variant()),
            PROP_CONFIG_FILE | PROP_CONFIG | PROP_PRESET => Some(GString::new().to_variant()),
            PROP_START_SNAPPED => Some(true.to_variant()),
            _ => {
                let m = self.proto.as_ref()?;
                let i = m.desc().param_index(&name)?;
                Some(props::to_variant(m.desc().params[i].kind, m.desc().params[i].default))
            }
        }
    }

    fn instantiate_playback(&self) -> Option<Gd<AudioStreamPlayback>> {
        let rate = AudioServer::singleton().get_mix_rate();
        let mut model = match self.build(rate) {
            Ok(m) => m,
            Err(e) => {
                godot_error!("SoundGenerator: cannot start playback: {e}");
                return None;
            }
        };
        let (block, version) = self.shared.snapshot();
        for i in 0..block.count {
            model.set_param(i, block.values[i]);
        }
        let (tx, rx) = RingBuffer::new(COMMAND_QUEUE_LEN);
        let input_names = model.desc().inputs.iter().map(|i| i.name.clone()).collect();
        let pb = Gd::from_init_fn(|base| SoundGeneratorPlayback {
            model,
            input_names,
            tx,
            rx,
            shared: self.shared.clone(),
            version,
            generation: block.generation,
            start_snapped: self.start_snapped,
            playing: false,
            frames_rendered: 0,
            base,
        });
        Some(pb.upcast())
    }

    fn get_stream_name(&self) -> GString {
        self.proto.as_ref().map(|m| GString::from(&m.desc().name)).unwrap_or_else(|| "SoundGenerator".into())
    }

    fn is_monophonic(&self) -> bool {
        true
    }
}

/// Running instance of a `SoundGenerator`. Drive it every frame; all methods are main-thread
/// safe while audio renders.
#[derive(GodotClass)]
#[class(base=AudioStreamPlayback, no_init)]
pub struct SoundGeneratorPlayback {
    model: Box<dyn Model>,
    input_names: Vec<String>,
    tx: Producer<Cmd>,
    rx: Consumer<Cmd>,
    shared: Arc<Shared<ParamBlock>>,
    version: u32,
    generation: u32,
    start_snapped: bool,
    playing: bool,
    frames_rendered: u64,
    base: Base<AudioStreamPlayback>,
}

impl SoundGeneratorPlayback {
    fn send(&mut self, cmd: Cmd) {
        if self.tx.push(cmd).is_err() {
            godot_warn!("SoundGeneratorPlayback: command queue full, dropped an update");
        }
    }

    fn sync_params(&mut self) {
        if let Some(block) = self.shared.poll(&mut self.version) {
            if block.generation == self.generation {
                for i in 0..block.count {
                    self.model.set_param(i, block.values[i]);
                }
            }
        }
    }
}

#[godot_api]
impl SoundGeneratorPlayback {
    /// Set an input (0..1) by name, e.g. `set_input("intensity", 0.7)`. Returns false for an
    /// unknown name. For per-frame use with many instances prefer `set_input_index`.
    #[func]
    fn set_input(&mut self, name: GString, value: f64) -> bool {
        let name = name.to_string();
        match self.input_names.iter().position(|n| *n == name) {
            Some(i) => {
                self.send(Cmd::Input(i, value as f32));
                true
            }
            None => false,
        }
    }

    /// Index of an input for `set_input_index`, or -1.
    #[func]
    fn get_input_index(&self, name: GString) -> i64 {
        let name = name.to_string();
        self.input_names.iter().position(|n| *n == name).map(|i| i as i64).unwrap_or(-1)
    }

    #[func]
    fn set_input_index(&mut self, index: i64, value: f64) {
        if index >= 0 && (index as usize) < self.input_names.len() {
            self.send(Cmd::Input(index as usize, value as f32));
        }
    }

    /// Set several inputs at once: `set_inputs({"throttle": 0.8, "load": 0.4})`.
    #[func]
    fn set_inputs(&mut self, values: VarDictionary) {
        for (k, v) in values.iter_shared() {
            if let (Ok(name), Some(value)) = (k.try_to::<GString>(), props::variant_to_f32(&v)) {
                self.set_input(name, value as f64);
            }
        }
    }

    #[func]
    fn get_input_names(&self) -> PackedStringArray {
        self.input_names.iter().map(GString::from).collect()
    }

    /// Change one parameter on this instance only.
    #[func]
    fn set_param(&mut self, name: GString, value: f64) -> bool {
        match self.model.desc().param_index(&name.to_string()) {
            Some(i) => {
                self.send(Cmd::Param(i, value as f32));
                true
            }
            None => false,
        }
    }

    /// Load a preset on this instance only.
    #[func]
    fn load_preset(&mut self, name: GString) -> bool {
        match self.model.desc().preset_index(&name.to_string()) {
            Some(i) => {
                self.send(Cmd::Preset(i));
                true
            }
            None => false,
        }
    }

    /// Jump all inertia (spool, rev, smoothing) to the current inputs.
    #[func]
    fn snap(&mut self) {
        self.send(Cmd::Snap);
    }

    /// Recent output peak 0..1 (decays over ~250 ms).
    #[func]
    fn get_peak(&self) -> f64 {
        self.model.peak() as f64
    }
}

#[godot_api]
impl IAudioStreamPlayback for SoundGeneratorPlayback {
    fn start(&mut self, _from_pos: f64) {
        self.sync_params();
        if self.start_snapped {
            self.model.snap();
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
        self.frames_rendered as f64 / self.model.sample_rate() as f64
    }

    fn seek(&mut self, _position: f64) {}

    unsafe fn mix_rawptr(&mut self, buffer: RawPtr<*mut AudioFrame>, _rate_scale: f32, frames: i32) -> i32 {
        let ptr = buffer.ptr();
        if ptr.is_null() || frames <= 0 {
            return 0;
        }
        let frames = frames as usize;
        while let Ok(cmd) = self.rx.pop() {
            match cmd {
                Cmd::Input(i, v) => self.model.set_input(i, v),
                Cmd::Param(i, v) => self.model.set_param(i, v),
                Cmd::Preset(i) => {
                    self.model.load_preset(i);
                }
                Cmd::Snap => self.model.snap(),
            }
        }
        self.sync_params();
        let model = &mut self.model;
        // SAFETY: Godot guarantees `buffer` holds at least `frames` AudioFrames.
        unsafe { fill_frames(ptr, frames, |block| model.render_mono(block)) };
        self.frames_rendered += frames as u64;
        frames as i32
    }
}
