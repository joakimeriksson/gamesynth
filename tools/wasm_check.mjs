// Exercise the WebAssembly build the way the web lab does and print a JSON report:
//   node tools/wasm_check.mjs web/pkg/gamesynth_wasm.wasm
import { readFileSync } from "node:fs";

const { exports: w } = await WebAssembly.instantiate(await WebAssembly.compile(readFileSync(process.argv[2])), {});
const str = (len) => new TextDecoder().decode(new Uint8Array(w.memory.buffer, w.gs_str_ptr(), len).slice());
const put = (text, fn) => {
  const bytes = new TextEncoder().encode(text), p = w.gs_alloc_u8(bytes.length);
  new Uint8Array(w.memory.buffer, p, bytes.length).set(bytes);
  const result = fn(p, bytes.length);
  w.gs_free_u8(p, bytes.length);
  return result;
};
const SR = 48000, buf = w.gs_alloc_f32(SR);
const level = (render) => {
  render(); render();
  let sum = 0, finite = true;
  for (const x of new Float32Array(w.memory.buffer, buf, SR)) { sum += x * x; finite &&= Number.isFinite(x); }
  return { rms: Math.sqrt(sum / SR), finite };
};
const checks = [];
const check = (name, ok, detail = "") => checks.push({ name, ok: !!ok, detail });

const lib = JSON.parse(str(w.model_library_json()));
check("library lists the native generators", lib.models.length >= 28, `${lib.models.length} generators, ${lib.nodes.length} node types`);
for (const d of lib.models) {
  const m = put(d.name, (p, n) => w.model_new(p, n, SR));
  d.inputs.forEach((i) => w.model_set_input(m, i.index, 1));
  if (d.one_shot) {
    // Events: point blank, fired once; must sound, then finish.
    w.model_set_input(m, 1, 0);
    w.model_trigger(m);
    w.model_render(m, buf, SR);
    let peak = 0;
    for (const x of new Float32Array(w.memory.buffer, buf, SR)) peak = Math.max(peak, Math.abs(x));
    for (let i = 0; i < 5; i++) w.model_render(m, buf, SR);
    check(`one-shot ${d.name} fires and finishes`, peak > 0.15 && peak <= 1 && w.model_is_finished(m) === 1, `peak ${peak.toFixed(2)}`);
    w.model_free(m);
    continue;
  }
  w.model_snap(m);
  const l = level(() => w.model_render(m, buf, SR));
  check(`generator ${d.name} renders`, m && l.finite && l.rms > 0.02, `rms ${l.rms.toFixed(3)}`);
  w.model_free(m);
}
for (const e of lib.examples) {
  const m = put(e.text, (p, n) => w.model_from_config(p, n, SR));
  if (!m) { check(`model file ${e.name} compiles`, false, str(w.gs_str_len())); continue; }
  const d = JSON.parse(str(w.model_desc_json(m)));
  d.inputs.forEach((i) => w.model_set_input(m, i.index, 1));
  w.model_snap(m);
  if (d.one_shot) w.model_trigger(m);
  const l = d.one_shot ? (() => { w.model_render(m, buf, SR); let s = 0; for (const x of new Float32Array(w.memory.buffer, buf, SR)) s += x * x; return { rms: Math.sqrt(s / SR), finite: true }; })() : level(() => w.model_render(m, buf, SR));
  check(`model file ${e.name} compiles and renders`, l.finite && l.rms > 0.02, `rms ${l.rms.toFixed(3)}`);
  w.model_free(m);
}
const bad = put('[graph]\nnodes = [{ id = "a", type = "wobble" }]\nout = "a"', (p, n) => w.model_from_config(p, n, SR));
check("a bad model file is rejected with a message", bad === 0 && str(w.gs_str_len()).includes("unknown type"), str(w.gs_str_len()).slice(0, 60));

const meta = JSON.parse(str(w.gs_meta_json()));
const synth = w.synth_new(SR);
for (const [i, name] of meta.sfx_presets.entries()) {
  w.synth_load_sfx(synth, i, 7);
  const n = w.synth_preview(synth, buf, SR);
  let peak = 0;
  for (const x of new Float32Array(w.memory.buffer, buf, n)) peak = Math.max(peak, Math.abs(x));
  check(`sfx ${name} renders a one-shot`, n > 0 && peak > 0.05 && peak <= 1, `${(n / SR).toFixed(2)} s, peak ${peak.toFixed(2)}`);
}
const jet = w.jet_new(SR, 0);
w.jet_set_controls(jet, 1, 1, 1, 0);
const jl = level(() => w.jet_render(jet, buf, SR));
check("jet engine spools and renders", jl.finite && jl.rms > 0.05 && w.jet_rpm(jet) > 0.9, `rpm ${w.jet_rpm(jet).toFixed(2)}, rms ${jl.rms.toFixed(3)}`);
console.log(JSON.stringify({ checks }));
