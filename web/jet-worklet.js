// AudioWorklet processor hosting the Rust jet engine (gamesynth-core compiled to wasm).
// The compiled WebAssembly.Module arrives via processorOptions; instantiation happens here,
// on the audio thread, so rendering never crosses a thread boundary.
const MAX_FRAMES = 4096;

class GsJetProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const { module, preset } = options.processorOptions;
    this.ready = false;
    this.pending = [];
    this.frames = 0;
    this.port.onmessage = (e) => (this.ready ? this.handle(e.data) : this.pending.push(e.data));
    WebAssembly.instantiate(module, {}).then((instance) => {
      this.w = instance.exports;
      this.engine = this.w.jet_new(sampleRate, preset | 0);
      this.buf = this.w.gs_alloc_f32(MAX_FRAMES);
      this.ready = true;
      for (const m of this.pending) this.handle(m);
      this.pending = [];
      this.port.postMessage({ ready: true, sampleRate });
    });
  }

  handle(m) {
    const w = this.w, e = this.engine;
    switch (m.t) {
      case "ctl": w.jet_set_controls(e, m.throttle, m.boost, m.speed, m.damage); break;
      case "preset": w.jet_set_preset(e, m.index | 0); break;
      case "param": w.jet_set_param(e, m.index | 0, m.value); break;
      case "snap": w.jet_snap_rpm(e); break;
      case "gain": w.jet_set_master_gain(e, m.gain); break;
      case "json": {
        const bytes = new TextEncoder().encode(m.json);
        const p = w.gs_alloc_u8(bytes.length);
        new Uint8Array(w.memory.buffer, p, bytes.length).set(bytes);
        w.jet_from_json(e, p, bytes.length);
        w.gs_free_u8(p, bytes.length);
        break;
      }
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    if (!out || !out.length) return true;
    const left = out[0];
    if (!this.ready) return true;
    const n = Math.min(left.length, MAX_FRAMES);
    this.w.jet_render(this.engine, this.buf, n);
    // Memory may have grown; always take a fresh view.
    left.set(new Float32Array(this.w.memory.buffer, this.buf, n));
    for (let c = 1; c < out.length; c++) out[c].set(left);
    this.frames += n;
    if (this.frames >= sampleRate / 30) {
      this.frames = 0;
      this.port.postMessage({ rpm: this.w.jet_rpm(this.engine), peak: this.w.jet_peak(this.engine) });
    }
    return true;
  }
}

registerProcessor("gs-jet", GsJetProcessor);
