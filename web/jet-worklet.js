// AudioWorklet processor hosting the Rust gamesynth engine (gamesynth-core compiled to wasm).
// One processor class serves every tab: `kind` selects a jet engine, a polyphonic synth, or a
// sound model (native generator by name, or a TOML/JSON model file compiled in place).
// The compiled WebAssembly.Module arrives via processorOptions and is instantiated here, on
// the audio thread, so rendering never crosses a thread boundary.
const MAX_FRAMES = 4096;

class GsEngineProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const { module, kind, preset, seed, model } = options.processorOptions;
    this.kind = kind || "jet";
    this.ready = false;
    this.pending = [];
    this.frames = 0;
    this.port.onmessage = (e) => (this.ready ? this.handle(e.data) : this.pending.push(e.data));
    WebAssembly.instantiate(module, {}).then((instance) => {
      const w = (this.w = instance.exports);
      if (this.kind === "jet") this.engine = w.jet_new(sampleRate, preset | 0);
      else if (this.kind === "model") this.engine = this.makeModel(model || { name: "wind" }) || this.makeModel({ name: "wind" });
      else {
        this.engine = w.synth_new(sampleRate);
        if (preset != null) w.synth_load_sfx(this.engine, preset | 0, seed >>> 0);
      }
      this.buf = w.gs_alloc_f32(MAX_FRAMES);
      this.ready = true;
      for (const m of this.pending) this.handle(m);
      this.pending = [];
      this.port.postMessage({ ready: true, sampleRate });
    });
  }

  // Build a model from { name } (native) or { config } (file text). Returns 0 on failure and
  // reports the compiler's message to the page.
  makeModel(spec) {
    const w = this.w;
    let handle = 0;
    this.writeJson(spec.config != null ? spec.config : spec.name, (p, n) => {
      handle = spec.config != null ? w.model_from_config(p, n, sampleRate) : w.model_new(p, n, sampleRate);
    });
    if (!handle) {
      const len = w.gs_str_len();
      const error = len ? new TextDecoder().decode(new Uint8Array(w.memory.buffer, w.gs_str_ptr(), len).slice()) : "unknown model";
      this.port.postMessage({ modelError: error });
    }
    return handle;
  }

  writeJson(json, fn) {
    const w = this.w, bytes = new TextEncoder().encode(json);
    const p = w.gs_alloc_u8(bytes.length);
    new Uint8Array(w.memory.buffer, p, bytes.length).set(bytes);
    fn(p, bytes.length);
    w.gs_free_u8(p, bytes.length);
  }

  handle(m) {
    const w = this.w, e = this.engine;
    if (this.kind === "model") {
      switch (m.t) {
        case "input": w.model_set_input(e, m.index | 0, m.value); break;
        case "param": w.model_set_param(e, m.index | 0, m.value); break;
        case "preset": w.model_load_preset(e, m.index | 0); break;
        case "snap": w.model_snap(e); break;
        case "model": {
          // Hot swap. Allocating on the audio thread is fine for a tool page; a game would
          // build the new model elsewhere and hand over a pointer.
          const next = this.makeModel(m.model);
          if (next) {
            w.model_free(e);
            this.engine = next;
            for (const [i, v] of (m.inputs || []).entries()) w.model_set_input(next, i, v);
            for (const [i, v] of (m.params || []).entries()) w.model_set_param(next, i, v);
            w.model_snap(next);
            this.port.postMessage({ modelReady: true });
          }
          break;
        }
      }
    } else if (this.kind === "jet") {
      switch (m.t) {
        case "ctl": w.jet_set_controls(e, m.throttle, m.boost, m.speed, m.damage); break;
        case "preset": w.jet_set_preset(e, m.index | 0); break;
        case "param": w.jet_set_param(e, m.index | 0, m.value); break;
        case "snap": w.jet_snap_rpm(e); break;
        case "gain": w.jet_set_master_gain(e, m.gain); break;
        case "json": this.writeJson(m.json, (p, n) => w.jet_from_json(e, p, n)); break;
      }
    } else {
      switch (m.t) {
        case "trigger": w.synth_trigger(e); break;
        case "noteon": w.synth_note_on(e, m.note, m.velocity == null ? 1 : m.velocity); break;
        case "noteoff": w.synth_note_off(e, m.note); break;
        case "play": w.synth_play(e, m.note, m.velocity == null ? 1 : m.velocity, m.duration || 0); break;
        case "alloff": w.synth_all_notes_off(e); break;
        case "panic": w.synth_panic(e); break;
        case "sfx": w.synth_load_sfx(e, m.preset | 0, m.seed >>> 0); break;
        case "mutate": w.synth_mutate(e, m.amount, m.seed >>> 0); break;
        case "reset": w.synth_reset_patch(e); break;
        case "param": w.synth_set_param(e, m.index | 0, m.value); break;
        case "bend": w.synth_set_pitch_bend(e, m.bend); break;
        case "gain": w.synth_set_master_gain(e, m.gain); break;
        case "json": this.writeJson(m.json, (p, n) => w.synth_from_json(e, p, n)); break;
      }
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    if (!out || !out.length) return true;
    const left = out[0];
    if (!this.ready) return true;
    const n = Math.min(left.length, MAX_FRAMES), w = this.w;
    if (this.kind === "jet") w.jet_render(this.engine, this.buf, n);
    else if (this.kind === "model") w.model_render(this.engine, this.buf, n);
    else w.synth_render(this.engine, this.buf, n);
    // Memory may have grown; always take a fresh view.
    left.set(new Float32Array(w.memory.buffer, this.buf, n));
    for (let c = 1; c < out.length; c++) out[c].set(left);
    this.frames += n;
    if (this.frames >= sampleRate / 30) {
      this.frames = 0;
      this.port.postMessage(this.kind === "jet"
        ? { rpm: w.jet_rpm(this.engine), peak: w.jet_peak(this.engine) }
        : this.kind === "model"
          ? { peak: w.model_peak(this.engine) }
          : { peak: w.synth_peak(this.engine), voices: w.synth_active_voices(this.engine) });
    }
    return true;
  }
}

registerProcessor("gs-engine", GsEngineProcessor);
