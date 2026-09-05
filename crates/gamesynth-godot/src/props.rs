//! Generic dynamic-property support: exposes any `gamesynth_core::Params` table as Godot
//! inspector properties with ranges / enums, and converts values both ways.

use gamesynth_core::{ParamKind, Params};
use godot::builtin::VariantType;
use godot::prelude::*;
use godot::register::info::{PropertyHint, PropertyHintInfo, PropertyInfo, PropertyUsageFlags};

pub fn param_properties<P: Params>() -> impl Iterator<Item = PropertyInfo> {
    P::ALL.iter().map(|&id| {
        let (variant_type, hint, hint_string) = match P::param_kind(id) {
            ParamKind::Float { min, max, step } => (VariantType::FLOAT, PropertyHint::RANGE, format!("{min},{max},{step}")),
            ParamKind::Exp { min, max } => (VariantType::FLOAT, PropertyHint::RANGE, format!("{min},{max},0.01,exp")),
            ParamKind::Int { min, max } => (VariantType::INT, PropertyHint::RANGE, format!("{min},{max},1")),
            ParamKind::Enum(names) => (VariantType::INT, PropertyHint::ENUM, names.join(",")),
        };
        PropertyInfo {
            variant_type,
            class_name: StringName::default(),
            property_name: P::param_name(id).into(),
            hint_info: PropertyHintInfo { hint, hint_string: GString::from(&hint_string) },
            usage: PropertyUsageFlags::DEFAULT,
        }
    })
}

/// Current value of a named parameter as a Variant, or `None` if `name` is not a parameter.
pub fn get_param<P: Params>(p: &P, name: &str) -> Option<Variant> {
    P::param_from_name(name).map(|id| to_variant(P::param_kind(id), p.get_param(id)))
}

/// Current value of a named parameter as f32 (enums/ints included), or `None` if unknown.
pub fn get_param_f32<P: Params>(p: &P, name: &str) -> Option<f32> {
    P::param_from_name(name).map(|id| p.get_param(id))
}

/// Default value of a named parameter (for inspector revert), or `None` if not a parameter.
pub fn default_param<P: Params>(name: &str) -> Option<Variant> {
    get_param(&P::default(), name)
}

/// Apply a Variant to a named parameter. `None` if `name` is not a parameter, otherwise
/// whether the value was usable.
pub fn set_param<P: Params>(p: &mut P, name: &str, value: &Variant) -> Option<bool> {
    let id = P::param_from_name(name)?;
    match variant_to_f32(value) {
        Some(v) => {
            p.set_param(id, v);
            Some(true)
        }
        None => Some(false),
    }
}

pub fn param_names<P: Params>() -> PackedStringArray {
    P::ALL.iter().map(|&id| GString::from(P::param_name(id))).collect()
}

fn to_variant(kind: ParamKind, value: f32) -> Variant {
    match kind {
        ParamKind::Int { .. } | ParamKind::Enum(_) => (value.round() as i64).to_variant(),
        _ => (value as f64).to_variant(),
    }
}

pub fn variant_to_f32(value: &Variant) -> Option<f32> {
    match value.get_type() {
        VariantType::FLOAT => value.try_to::<f64>().ok().map(|v| v as f32),
        VariantType::INT => value.try_to::<i64>().ok().map(|v| v as f32),
        VariantType::BOOL => value.try_to::<bool>().ok().map(|b| if b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

pub fn enum_property(name: &str, names: &[&str]) -> PropertyInfo {
    PropertyInfo {
        variant_type: VariantType::INT,
        class_name: StringName::default(),
        property_name: name.into(),
        hint_info: PropertyHintInfo { hint: PropertyHint::ENUM, hint_string: GString::from(&names.join(",")) },
        usage: PropertyUsageFlags::DEFAULT,
    }
}

pub fn int_property(name: &str, min: i64, max: i64) -> PropertyInfo {
    PropertyInfo {
        variant_type: VariantType::INT,
        class_name: StringName::default(),
        property_name: name.into(),
        hint_info: PropertyHintInfo { hint: PropertyHint::RANGE, hint_string: GString::from(&format!("{min},{max},1")) },
        usage: PropertyUsageFlags::DEFAULT,
    }
}

/// An inspector button (Godot 4.4+) that invokes a method; not persisted.
pub fn tool_button(name: &str, label: &str) -> PropertyInfo {
    PropertyInfo {
        variant_type: VariantType::CALLABLE,
        class_name: StringName::default(),
        property_name: name.into(),
        hint_info: PropertyHintInfo { hint: PropertyHint::TOOL_BUTTON, hint_string: label.into() },
        usage: PropertyUsageFlags::EDITOR,
    }
}
