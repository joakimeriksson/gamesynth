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

/// Length in bytes of the string buffer (for calls that report errors through it).
#[no_mangle]
pub extern "C" fn gs_str_len() -> u32 {
    OUT.with(|o| o.borrow().len() as u32)
}

/// Pointer to the string produced by the most recent `gs_meta_json` / `jet_to_json` call.
#[no_mangle]
pub extern "C" fn gs_str_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

// --- metadata ---------------------------------------------------------------------------

fn params_json(params: &[gamesynth_core::ParamDesc]) -> Vec<serde_json::Value> {
    params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (kind, min, max, step, names) = match p.kind {
                ParamKind::Float { min, max, step } => ("float", min, max, step, None),
                ParamKind::Exp { min, max } => ("exp", min, max, 0.0, None),
                ParamKind::Int { min, max } => ("int", min as f32, max as f32, 1.0, None),
                ParamKind::Enum(names) => ("enum", 0.0, (names.len() - 1) as f32, 1.0, Some(names)),
            };
            let mut v = json!({ "index": i, "name": p.name, "kind": kind, "min": min, "max": max, "step": step, "default": p.default });
            if let Some(n) = names {
                v["names"] = json!(n);
            }
            v
        })
        .collect()
}

fn describe<P: gamesynth_core::Params>() -> Vec<serde_json::Value> {
    params_json(&gamesynth_core::params::describe::<P>())
}

/// Writes `{"version", "params": [jet params], "presets": [jet presets],
/// "synth_params": [...], "sfx_presets": [...]}` to the string buffer; returns its length.
/// Each param: `{index, name, kind: float|exp|int|enum, min, max, step, names?}`.
#[no_mangle]
pub extern "C" fn gs_meta_json() -> u32 {
    set_out(
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "params": describe::<gamesynth_core::JetParams>(),
            "presets": JetPreset::NAMES,
            "synth_params": describe::<gamesynth_core::Patch>(),
            "sfx_presets": gamesynth_core::SfxPreset::NAMES,
        })
        .to_string(),
    )
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

// =========================================================================================
// Polyphonic synth / sound effects
// =========================================================================================

use gamesynth_core::{ParamId, Patch, Rng, SfxPreset, Synth};

#[no_mangle]
pub extern "C" fn synth_new(sample_rate: f32) -> *mut Synth {
    let sr = if sample_rate.is_finite() && sample_rate > 1000.0 { sample_rate } else { 48000.0 };
    Box::into_raw(Box::new(Synth::new(sr)))
}

#[no_mangle]
pub unsafe extern "C" fn synth_free(s: *mut Synth) {
    if !s.is_null() {
        drop(Box::from_raw(s));
    }
}

/// Load an sfxr-style preset (index into `sfx_presets`) generated from `seed`.
#[no_mangle]
pub unsafe extern "C" fn synth_load_sfx(s: *mut Synth, preset: u32, seed: u32) {
    (*s).set_patch(SfxPreset::from_index(preset as usize).generate(seed));
}

/// Reset to the default instrument patch.
#[no_mangle]
pub unsafe extern "C" fn synth_reset_patch(s: *mut Synth) {
    (*s).set_patch(Patch::default());
}

/// Randomly perturb the patch by `amount` (0..1); deterministic per `seed`.
#[no_mangle]
pub unsafe extern "C" fn synth_mutate(s: *mut Synth, amount: f32, seed: u32) {
    let mut p = *(*s).patch();
    gamesynth_core::sfx::mutate(&mut p, &mut Rng::new(seed), amount);
    (*s).set_patch(p);
}

#[no_mangle]
pub unsafe extern "C" fn synth_trigger(s: *mut Synth) {
    (*s).trigger();
}

#[no_mangle]
pub unsafe extern "C" fn synth_note_on(s: *mut Synth, note: f32, velocity: f32) {
    (*s).note_on(note, velocity);
}

#[no_mangle]
pub unsafe extern "C" fn synth_note_off(s: *mut Synth, note: f32) {
    (*s).note_off(note);
}

/// Note that releases itself after `duration` seconds.
#[no_mangle]
pub unsafe extern "C" fn synth_play(s: *mut Synth, note: f32, velocity: f32, duration: f32) {
    (*s).play(note, velocity, duration);
}

#[no_mangle]
pub unsafe extern "C" fn synth_all_notes_off(s: *mut Synth) {
    (*s).all_notes_off();
}

#[no_mangle]
pub unsafe extern "C" fn synth_panic(s: *mut Synth) {
    (*s).panic();
}

#[no_mangle]
pub unsafe extern "C" fn synth_set_pitch_bend(s: *mut Synth, bend: f32) {
    (*s).set_pitch_bend(bend);
}

#[no_mangle]
pub unsafe extern "C" fn synth_set_master_gain(s: *mut Synth, gain: f32) {
    (*s).set_master_gain(gain);
}

/// Returns 1 if `index` names a synth parameter (see `synth_params` in the metadata).
#[no_mangle]
pub unsafe extern "C" fn synth_set_param(s: *mut Synth, index: u32, value: f32) -> u32 {
    match ParamId::from_index(index as usize) {
        Some(id) => {
            (*s).set_param(id, value);
            1
        }
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn synth_get_param(s: *const Synth, index: u32) -> f32 {
    ParamId::from_index(index as usize).map(|id| (*s).get_param(id)).unwrap_or(0.0)
}

#[no_mangle]
pub unsafe extern "C" fn synth_render(s: *mut Synth, out: *mut f32, frames: u32) {
    if s.is_null() || out.is_null() || frames == 0 {
        return;
    }
    (*s).render_mono(std::slice::from_raw_parts_mut(out, frames as usize));
}

/// Render the current patch as a one-shot into `out` (offline, on a scratch synth) and
/// return how many frames were produced before it went silent. For waveform previews.
#[no_mangle]
pub unsafe extern "C" fn synth_preview(s: *const Synth, out: *mut f32, max_frames: u32) -> u32 {
    if s.is_null() || out.is_null() || max_frames == 0 {
        return 0;
    }
    let sr = (*s).sample_rate();
    let samples = gamesynth_core::render::render_one_shot((*s).patch(), sr, max_frames as f32 / sr);
    let n = samples.len().min(max_frames as usize);
    std::slice::from_raw_parts_mut(out, n).copy_from_slice(&samples[..n]);
    n as u32
}

#[no_mangle]
pub unsafe extern "C" fn synth_peak(s: *const Synth) -> f32 {
    (*s).peak()
}

#[no_mangle]
pub unsafe extern "C" fn synth_active_voices(s: *const Synth) -> u32 {
    (*s).active_voices() as u32
}

/// One-shot length in seconds including tail, or -1 when the patch sustains until note-off.
#[no_mangle]
pub unsafe extern "C" fn synth_one_shot_length(s: *const Synth) -> f32 {
    (*s).patch().one_shot_length().unwrap_or(-1.0)
}

/// Patch as JSON in the exact format `SynthPatch.from_json` accepts; returns the length.
#[no_mangle]
pub unsafe extern "C" fn synth_to_json(s: *const Synth) -> u32 {
    set_out((*s).patch().to_json())
}

/// Load a patch from UTF-8 JSON at (`ptr`, `len`). Returns 1 on success.
#[no_mangle]
pub unsafe extern "C" fn synth_from_json(s: *mut Synth, ptr: *const u8, len: u32) -> u32 {
    let bytes = std::slice::from_raw_parts(ptr, len as usize);
    match std::str::from_utf8(bytes).ok().and_then(|j| Patch::from_json(j).ok()) {
        Some(p) => {
            (*s).set_patch(p);
            1
        }
        None => 0,
    }
}

// =========================================================================================
// Sound models: native generators and file-defined graphs behind one interface
// =========================================================================================

use gamesynth_core::{generators, GraphModel, Model, ModelDesc};

/// Handle type: a thin pointer to a boxed trait object.
pub type ModelHandle = *mut Box<dyn Model>;

const EXAMPLES: &[(&str, &str)] = &[
    ("jet_lite", include_str!("../../../models/jet_lite.toml")),
    ("rain_on_tent", include_str!("../../../models/rain_on_tent.toml")),
    ("campfire", include_str!("../../../models/campfire.toml")),
    ("shield", include_str!("../../../models/shield.toml")),
    ("geiger", include_str!("../../../models/geiger.toml")),
    ("steam_vent", include_str!("../../../models/steam_vent.toml")),
    ("recharge", include_str!("../../../models/recharge.toml")),
    ("checkpoint", include_str!("../../../models/checkpoint.toml")),
    ("mine_armed", include_str!("../../../models/mine_armed.toml")),
    ("rocket_flight", include_str!("../../../models/rocket_flight.toml")),
];

fn desc_json(d: &ModelDesc) -> serde_json::Value {
    json!({
        "name": d.name,
        "category": d.category,
        "doc": d.doc,
        "engine": d.engine,
        "one_shot": d.one_shot,
        "inputs": d.inputs.iter().enumerate().map(|(i, x)| json!({ "index": i, "name": x.name, "default": x.default, "doc": x.doc })).collect::<Vec<_>>(),
        "params": params_json(&d.params),
        "presets": d.presets.iter().map(|p| json!({ "name": p.name, "values": p.values })).collect::<Vec<_>>(),
    })
}

unsafe fn text<'a>(ptr: *const u8, len: u32) -> &'a str {
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len as usize)).unwrap_or("")
}

/// `{"models": [desc..], "examples": [{name, text}..], "nodes": [{type, params, args, doc}..]}`
/// into the string buffer; returns its length.
#[no_mangle]
pub extern "C" fn model_library_json() -> u32 {
    let nodes: Vec<_> = gamesynth_core::graph::node_library()
        .iter()
        .map(|(ty, params, args, doc)| {
            let params: Vec<_> = params.iter().map(|(n, d)| json!({ "name": n, "default": if d.is_nan() { serde_json::Value::Null } else { json!(d) } })).collect();
            json!({ "type": ty, "params": params, "args": args, "doc": doc })
        })
        .collect();
    set_out(
        json!({
            "models": generators::describe_all().iter().map(desc_json).collect::<Vec<_>>(),
            "examples": EXAMPLES.iter().map(|(n, t)| json!({ "name": n, "text": t })).collect::<Vec<_>>(),
            "nodes": nodes,
        })
        .to_string(),
    )
}

/// Create a native generator by name. Returns null for an unknown name.
#[no_mangle]
pub unsafe extern "C" fn model_new(name: *const u8, len: u32, sample_rate: f32) -> ModelHandle {
    match generators::create(text(name, len), sample_rate) {
        Some(m) => Box::into_raw(Box::new(m)),
        None => std::ptr::null_mut(),
    }
}

/// Compile a TOML/JSON model file. Returns null on error and leaves the message in the
/// string buffer (`gs_str_ptr` / `gs_str_len`).
#[no_mangle]
pub unsafe extern "C" fn model_from_config(config: *const u8, len: u32, sample_rate: f32) -> ModelHandle {
    match GraphModel::from_text(text(config, len), sample_rate) {
        Ok(m) => {
            set_out(String::new());
            Box::into_raw(Box::new(Box::new(m) as Box<dyn Model>))
        }
        Err(e) => {
            set_out(e.to_string());
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn model_free(m: ModelHandle) {
    if !m.is_null() {
        drop(Box::from_raw(m));
    }
}

#[no_mangle]
pub unsafe extern "C" fn model_desc_json(m: ModelHandle) -> u32 {
    set_out(desc_json((*m).desc()).to_string())
}

#[no_mangle]
pub unsafe extern "C" fn model_set_input(m: ModelHandle, index: u32, value: f32) {
    (*m).set_input(index as usize, value);
}

#[no_mangle]
pub unsafe extern "C" fn model_set_param(m: ModelHandle, index: u32, value: f32) {
    (*m).set_param(index as usize, value);
}

#[no_mangle]
pub unsafe extern "C" fn model_get_param(m: ModelHandle, index: u32) -> f32 {
    (*m).param(index as usize)
}

/// Returns 1 if the preset exists.
#[no_mangle]
pub unsafe extern "C" fn model_load_preset(m: ModelHandle, index: u32) -> u32 {
    (*m).load_preset(index as usize) as u32
}

#[no_mangle]
pub unsafe extern "C" fn model_snap(m: ModelHandle) {
    (*m).snap();
}

/// Fire a one-shot with the current inputs (no effect on continuous models).
#[no_mangle]
pub unsafe extern "C" fn model_trigger(m: ModelHandle) {
    (*m).trigger();
}

/// 1 once a one-shot has rung out.
#[no_mangle]
pub unsafe extern "C" fn model_is_finished(m: ModelHandle) -> u32 {
    (*m).is_finished() as u32
}

#[no_mangle]
pub unsafe extern "C" fn model_render(m: ModelHandle, out: *mut f32, frames: u32) {
    if m.is_null() || out.is_null() || frames == 0 {
        return;
    }
    (*m).render_mono(std::slice::from_raw_parts_mut(out, frames as usize));
}

#[no_mangle]
pub unsafe extern "C" fn model_peak(m: ModelHandle) -> f32 {
    (*m).peak()
}
