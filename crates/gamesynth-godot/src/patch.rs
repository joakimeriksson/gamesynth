use gamesynth_core::{Patch, Rng, SfxPreset};
use godot::classes::{IResource, Resource};
use godot::prelude::*;
use godot::register::info::PropertyInfo;

use crate::props;

const PROP_SFX_PRESET: &str = "sfx/preset";
const PROP_SFX_SEED: &str = "sfx/seed";
const PROP_SFX_GENERATE: &str = "sfx/generate";
const PROP_SFX_RANDOMIZE: &str = "sfx/randomize";

/// A synthesizer patch: every parameter of a sound, editable in the inspector and
/// saveable as a `.tres` resource.
///
/// Parameters are addressed by slash-separated names such as `"filter/cutoff_hz"`; see
/// `get_param_names()`. The `sfx/*` section generates sfxr-style sound effects.
#[derive(GodotClass)]
#[class(base=Resource, tool)]
pub struct SynthPatch {
    patch: Patch,
    sfx_preset: i64,
    sfx_seed: i64,
    base: Base<Resource>,
}

impl SynthPatch {
    /// Copy of the underlying patch (plain data, safe to hand to the audio thread).
    pub fn patch(&self) -> Patch {
        self.patch
    }

    fn changed(&mut self) {
        self.base_mut().emit_changed();
    }

    fn replace(&mut self, patch: Patch) {
        self.patch = patch;
        self.changed();
        self.base_mut().notify_property_list_changed();
    }
}

#[godot_api]
impl SynthPatch {
    /// Create a patch from a sound effect preset name (`Pickup`, `Laser`, `Explosion`,
    /// `PowerUp`, `Hit`, `Jump`, `Blip`, `Random`) and a seed. Same inputs give the same sound.
    #[func]
    pub fn from_preset(preset: GString, seed: i64) -> Gd<SynthPatch> {
        let preset = SfxPreset::from_name(&preset.to_string()).unwrap_or_else(|| {
            godot_warn!("SynthPatch.from_preset: unknown preset '{preset}', using Random");
            SfxPreset::Random
        });
        Gd::from_init_fn(|base| SynthPatch {
            patch: preset.generate(seed as u32),
            sfx_preset: preset as i64,
            sfx_seed: seed,
            base,
        })
    }

    /// Create a patch from a JSON string produced by `to_json()`.
    #[func]
    fn from_json_string(json: GString) -> Option<Gd<SynthPatch>> {
        let mut gd = Gd::from_init_fn(SynthPatch::init);
        if gd.bind_mut().from_json(json) {
            Some(gd)
        } else {
            None
        }
    }

    /// Names of the available sound effect presets.
    #[func]
    fn get_preset_names() -> PackedStringArray {
        SfxPreset::NAMES.iter().map(|n| GString::from(*n)).collect()
    }

    /// Names of all parameters, e.g. `"osc1/wave"`, `"filter/cutoff_hz"`.
    #[func]
    fn get_param_names() -> PackedStringArray {
        props::param_names::<Patch>()
    }

    /// Set a parameter by name. Values are clamped to the valid range. Returns false for
    /// an unknown name.
    #[func]
    fn set_param(&mut self, name: GString, value: f64) -> bool {
        match props::set_param(&mut self.patch, &name.to_string(), &value.to_variant()) {
            Some(true) => {
                self.changed();
                true
            }
            _ => {
                godot_warn!("SynthPatch.set_param: unknown parameter '{name}'");
                false
            }
        }
    }

    /// Get a parameter by name (0.0 for unknown names).
    #[func]
    fn get_param(&self, name: GString) -> f64 {
        props::get_param_f32(&self.patch, &name.to_string()).map(|v| v as f64).unwrap_or(0.0)
    }

    /// Serialise the patch to JSON (for copy/paste, network sync or your own file format).
    #[func]
    fn to_json(&self) -> GString {
        GString::from(&self.patch.to_json())
    }

    /// Load parameters from JSON. Missing fields keep their defaults. Returns false on error.
    #[func]
    #[allow(clippy::wrong_self_convention)]
    fn from_json(&mut self, json: GString) -> bool {
        match Patch::from_json(&json.to_string()) {
            Ok(p) => {
                self.replace(p);
                true
            }
            Err(e) => {
                godot_error!("SynthPatch.from_json: {e}");
                false
            }
        }
    }

    /// Regenerate the patch from `sfx/preset` and `sfx/seed`.
    #[func]
    fn generate_sfx(&mut self) {
        let preset = SfxPreset::from_index(self.sfx_preset.max(0) as usize);
        self.replace(preset.generate(self.sfx_seed as u32));
    }

    /// Pick a new random seed and regenerate the patch from `sfx/preset`.
    #[func]
    fn randomize_sfx(&mut self) {
        let mut rng = Rng::new(godot::classes::Time::singleton().get_ticks_usec() as u32);
        self.sfx_seed = (rng.next_u32() % 100_000) as i64;
        self.generate_sfx();
    }

    /// Randomly perturb every parameter by up to `amount` (0..1) of its range.
    /// Deterministic for a given `seed`.
    #[func]
    fn mutate(&mut self, amount: f64, seed: i64) {
        let mut rng = Rng::new(seed as u32);
        let mut p = self.patch;
        gamesynth_core::sfx::mutate(&mut p, &mut rng, amount as f32);
        self.replace(p);
    }

    /// Length in seconds of a triggered one-shot including its tail, or -1 when the patch
    /// sustains until note-off.
    #[func]
    fn get_length(&self) -> f64 {
        self.patch.one_shot_length().map(|l| l as f64).unwrap_or(-1.0)
    }

    /// Reset every parameter to the default instrument patch.
    #[func]
    fn reset(&mut self) {
        self.replace(Patch::default());
    }
}

#[godot_api]
impl IResource for SynthPatch {
    fn init(base: Base<Resource>) -> Self {
        SynthPatch { patch: Patch::default(), sfx_preset: SfxPreset::Pickup as i64, sfx_seed: 0, base }
    }

    fn on_get_property_list(&mut self) -> Vec<PropertyInfo> {
        let mut list = vec![
            props::enum_property(PROP_SFX_PRESET, &SfxPreset::NAMES),
            props::int_property(PROP_SFX_SEED, 0, 99_999),
            props::tool_button(PROP_SFX_GENERATE, "Generate from preset + seed"),
            props::tool_button(PROP_SFX_RANDOMIZE, "Randomize seed and generate"),
        ];
        list.extend(props::param_properties::<Patch>());
        list
    }

    fn on_get(&self, property: StringName) -> Option<Variant> {
        let name = property.to_string();
        match name.as_str() {
            PROP_SFX_PRESET => Some(self.sfx_preset.to_variant()),
            PROP_SFX_SEED => Some(self.sfx_seed.to_variant()),
            PROP_SFX_GENERATE => Some(self.base().callable("generate_sfx").to_variant()),
            PROP_SFX_RANDOMIZE => Some(self.base().callable("randomize_sfx").to_variant()),
            _ => props::get_param(&self.patch, &name),
        }
    }

    fn on_set(&mut self, property: StringName, value: Variant) -> bool {
        let name = property.to_string();
        match name.as_str() {
            PROP_SFX_PRESET => {
                self.sfx_preset = value.try_to::<i64>().unwrap_or(0).clamp(0, SfxPreset::NAMES.len() as i64 - 1);
                true
            }
            PROP_SFX_SEED => {
                self.sfx_seed = value.try_to::<i64>().unwrap_or(0);
                true
            }
            PROP_SFX_GENERATE | PROP_SFX_RANDOMIZE => true,
            _ => match props::set_param(&mut self.patch, &name, &value) {
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
            PROP_SFX_PRESET => Some((SfxPreset::Pickup as i64).to_variant()),
            PROP_SFX_SEED => Some(0i64.to_variant()),
            _ => props::default_param::<Patch>(&name),
        }
    }
}
