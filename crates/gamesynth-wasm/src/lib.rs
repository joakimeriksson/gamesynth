//! Plain C-ABI exports of the jet engine for `wasm32-unknown-unknown`.
//!
//! No wasm-bindgen: the module has no imports, so any host (an AudioWorklet, a plain page,
//! Node) can `WebAssembly.instantiate(module, {})` it and call the functions below. Strings
//! cross the boundary as (pointer, length) pairs into the module's linear memory.
//!
//! Build: `cargo build -p gamesynth-wasm --profile wasm --target wasm32-unknown-unknown`

#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;

use gamesynth_core::{JetEngine, JetParamId, JetPreset, ParamKind};
use serde_json::json;

thread_local! {
    /// Scratch string returned by the `*_json` getters; valid until the next such call.
    static OUT: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_out(s: String) -> u32 {
    OUT.with(|o| {
        *o.borrow_mut() = s;
        o.borrow().len() as u32
    })
}

// --- memory helpers ---------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn gs_alloc_f32(n: u32) -> *mut f32 {
    let mut v = vec![0.0f32; n as usize];
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

#[no_mangle]
pub unsafe extern "C" fn gs_free_f32(p: *mut f32, n: u32) {
    if !p.is_null() {
        drop(Vec::from_raw_parts(p, n as usize, n as usize));
    }
}

#[no_mangle]
pub extern "C" fn gs_alloc_u8(n: u32) -> *mut u8 {
    let mut v = vec![0u8; n as usize];
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

#[no_mangle]
pub unsafe extern "C" fn gs_free_u8(p: *mut u8, n: u32) {
    if !p.is_null() {
        drop(Vec::from_raw_parts(p, n as usize, n as usize));
    }
}

/// Pointer to the string produced by the most recent `gs_meta_json` / `jet_to_json` call.
#[no_mangle]
pub extern "C" fn gs_str_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

// --- metadata ---------------------------------------------------------------------------

/// Writes `{"params":[{index,name,kind,min,max,step}...],"presets":[names]}` to the string
/// buffer and returns its length.
#[no_mangle]
pub extern "C" fn gs_meta_json() -> u32 {
    let params: Vec<_> = JetParamId::ALL
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let (kind, min, max, step) = match id.kind() {
                ParamKind::Float { min, max, step } => ("float", min, max, step),
                ParamKind::Exp { min, max } => ("exp", min, max, 0.0),
                ParamKind::Int { min, max } => ("int", min as f32, max as f32, 1.0),
                ParamKind::Enum(names) => ("enum", 0.0, (names.len() - 1) as f32, 1.0),
            };
            json!({ "index": i, "name": id.name(), "kind": kind, "min": min, "max": max, "step": step })
        })
        .collect();
    set_out(json!({ "params": params, "presets": JetPreset::NAMES, "version": env!("CARGO_PKG_VERSION") }).to_string())
}

// --- engine -----------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn jet_new(sample_rate: f32, preset: u32) -> *mut JetEngine {
    let sr = if sample_rate.is_finite() && sample_rate > 1000.0 { sample_rate } else { 48000.0 };
    Box::into_raw(Box::new(JetEngine::new(sr, JetPreset::from_index(preset as usize).params())))
}

#[no_mangle]
pub unsafe extern "C" fn jet_free(e: *mut JetEngine) {
    if !e.is_null() {
        drop(Box::from_raw(e));
    }
}

#[no_mangle]
pub unsafe extern "C" fn jet_set_controls(e: *mut JetEngine, throttle: f32, boost: f32, speed: f32, damage: f32) {
    let e = &mut *e;
    e.set_throttle(throttle);
    e.set_boost(boost);
    e.set_speed(speed);
    e.set_damage(damage);
}

#[no_mangle]
pub unsafe extern "C" fn jet_set_preset(e: *mut JetEngine, preset: u32) {
    (*e).set_params(JetPreset::from_index(preset as usize).params());
}

/// Returns 1 if `index` names a parameter (value is clamped to its range), else 0.
#[no_mangle]
pub unsafe extern "C" fn jet_set_param(e: *mut JetEngine, index: u32, value: f32) -> u32 {
    match JetParamId::from_index(index as usize) {
        Some(id) => {
            let mut p = *(*e).params();
            p.set(id, value);
            (*e).set_params(p);
            1
        }
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn jet_get_param(e: *const JetEngine, index: u32) -> f32 {
    JetParamId::from_index(index as usize).map(|id| (*e).params().get(id)).unwrap_or(0.0)
}

#[no_mangle]
pub unsafe extern "C" fn jet_snap_rpm(e: *mut JetEngine) {
    (*e).snap_rpm();
}

#[no_mangle]
pub unsafe extern "C" fn jet_set_master_gain(e: *mut JetEngine, gain: f32) {
    (*e).set_master_gain(gain);
}

/// Render `frames` mono samples into `out` (must hold at least `frames` f32s).
#[no_mangle]
pub unsafe extern "C" fn jet_render(e: *mut JetEngine, out: *mut f32, frames: u32) {
    if e.is_null() || out.is_null() || frames == 0 {
        return;
    }
    let buf = std::slice::from_raw_parts_mut(out, frames as usize);
    (*e).render_mono(buf);
}

#[no_mangle]
pub unsafe extern "C" fn jet_rpm(e: *const JetEngine) -> f32 {
    (*e).rpm()
}

#[no_mangle]
pub unsafe extern "C" fn jet_peak(e: *const JetEngine) -> f32 {
    (*e).peak()
}

/// Serialise the engine's parameters (the exact format `JetEnginePatch.from_json` accepts)
/// into the string buffer; returns the length.
#[no_mangle]
pub unsafe extern "C" fn jet_to_json(e: *const JetEngine) -> u32 {
    set_out(serde_json::to_string_pretty((*e).params()).unwrap_or_default())
}

/// Load parameters from UTF-8 JSON at (`ptr`, `len`). Returns 1 on success.
#[no_mangle]
pub unsafe extern "C" fn jet_from_json(e: *mut JetEngine, ptr: *const u8, len: u32) -> u32 {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    match std::str::from_utf8(bytes).ok().and_then(|s| serde_json::from_str(s).ok()) {
        Some(p) => {
            (*e).set_params(p);
            1
        }
        None => 0,
    }
}
