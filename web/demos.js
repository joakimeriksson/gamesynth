// The sounds on the landing page, and the code that computes them.
//
// Each demo is a short script: which generators to run and how their inputs move over time,
// the way a game would drive them. `render` runs the script through the wasm engine faster
// than real time and returns stereo samples; `spectrogram` paints them. No DOM in this file,
// so it also runs under Node (tools/web_audio_check.mjs covers it in a real browser).

export const SR = 48000;
const BLOCK = 480; // inputs are updated every 10 ms

// Continuous tracks: `keys` maps an input name to [seconds, value] points, joined by straight lines.
// Event tracks: `fire` lists [seconds, power, distance] triggers.
export const DEMOS = [
  {
    id: "truck", seconds: 14, title: "Heavy truck",
    lab: "lab.html?gen=piston&preset=Heavy%20truck",
    tracks: [{
      gen: "piston", preset: "Heavy truck", params: { "engine/external_rpm": 1 },
      keys: {
        throttle: [[0, 0], [2, 0], [2.05, 1], [5.2, 1], [5.25, 0], [5.5, 0], [5.55, 1], [8.6, 1], [8.65, 0], [14, 0]],
        rpm: [[0, 0], [2, 0.04], [5.2, 0.95], [5.25, 0.93], [5.5, 0.55], [8.6, 0.95], [8.65, 0.93], [12.4, 0.12], [13, 0], [14, 0]],
        load: [[0, 0.3], [2, 0.3], [2.05, 1], [8.6, 1], [8.65, 0.3], [14, 0.3]],
      },
    }],
  },
  {
    id: "v8", seconds: 12, title: "A V8, fitted to a real one",
    tags: ["piston", "8 cylinders, 2 exhaust banks"],
    text: "Firing order L R R L R L L R into two pipes: the uneven pulses per bank are the rumble. Idle, two blips, a pull, then it burbles on the way down.",
    lab: "lab.html?gen=piston&preset=Muscle%20V8",
    tracks: [{
      gen: "piston", preset: "Muscle V8", params: { "engine/external_rpm": 1 },
      keys: {
        throttle: [[0, 0], [1.5, 0], [1.55, 1], [1.85, 1], [1.9, 0], [2.9, 0], [2.95, 1], [3.3, 1], [3.35, 0], [4.6, 0], [4.65, 1], [8, 1], [8.05, 0], [12, 0]],
        rpm: [[0, 0], [1.5, 0], [1.85, 0.4], [2.7, 0], [2.9, 0], [3.3, 0.55], [4.3, 0], [4.6, 0.02], [8, 0.95], [11, 0.08], [11.6, 0], [12, 0]],
        load: [[0, 0.3], [4.6, 0.3], [4.65, 1], [8, 1], [8.05, 0.3], [12, 0.3]],
      },
    }],
  },
  {
    id: "jet", seconds: 10, title: "Anti-gravity racer",
    tags: ["jet", "throttle, boost, speed, damage"],
    text: "A turbine that spools up with its own inertia, an afterburner on boost, and wind that rises with speed.",
    lab: "lab.html?tab=engines",
    tracks: [{
      gen: "jet",
      keys: {
        throttle: [[0, 0.1], [0.5, 0.1], [3, 1], [7.4, 1], [7.8, 0.1], [10, 0.1]],
        boost: [[0, 0], [4.2, 0], [4.3, 1], [6.4, 1], [6.6, 0], [10, 0]],
        speed: [[0, 0], [1, 0], [7, 1], [10, 0.3]],
      },
    }],
  },
  {
    id: "storm", seconds: 10, title: "Storm on a tin roof",
    tags: ["rain + wind", "two generators mixed"],
    text: "Rain builds from a drizzle to a downpour on a metal roof while the wind gusts over it. Two generators, each following its own input.",
    lab: "lab.html?gen=rain&preset=Tin%20roof",
    tracks: [
      { gen: "rain", preset: "Tin roof", keys: { intensity: [[0, 0.12], [5.5, 1], [10, 0.45]], shelter: [[0, 0.6], [10, 0.6]] } },
      { gen: "wind", gain: 0.6, keys: { strength: [[0, 0.3], [5, 0.9], [10, 0.5]], gustiness: [[0, 0.8], [10, 0.8]] } },
    ],
  },
  {
    id: "weapons", seconds: 8, title: "Weapons",
    tags: ["laser, plasma, rocket, mine, explosion", "events"],
    text: "Layered one-shots. The game passes power and distance when it fires them, and no two triggers are quite the same.",
    lab: "lab.html?gen=explosion&preset=Ship%20destroyed",
    tracks: [
      { gen: "laser", fire: [[0.2, 1, 0], [0.5, 1, 0], [0.8, 1, 0.5]] },
      { gen: "plasma", preset: "Heavy cannon", fire: [[1.6, 1, 0]] },
      { gen: "rocket", preset: "Heavy missile", fire: [[2.6, 1, 0]] },
      { gen: "mine_blast", fire: [[4.6, 0.8, 0.4]] },
      { gen: "explosion", preset: "Ship destroyed", fire: [[5.4, 1, 0]] },
    ],
  },
  {
    id: "tyres", seconds: 10, title: "Tyres on four surfaces",
    tags: ["tyre", "speed, slip, load, surface"],
    text: "One knobbly tyre rolling over packed dirt, gravel, mud and rock, with two slides. The surface is a single input the game sets from what the wheel is on.",
    lab: "lab.html?gen=tyre&preset=Knobbly",
    tracks: [{
      gen: "tyre", preset: "Knobbly",
      keys: {
        speed: [[0, 0], [1, 0.7], [9, 0.7], [10, 0]],
        surface: [[0, 0], [2.4, 0], [2.6, 0.25], [5, 0.25], [5.2, 0.75], [7.4, 0.75], [7.6, 1], [10, 1]],
        slip: [[0, 0.1], [3.8, 0.1], [4, 0.8], [4.4, 0.1], [8.2, 0.1], [8.4, 0.9], [9, 0.1], [10, 0.1]],
        load: [[0, 0.6], [10, 0.6]],
      },
    }],
  },
  {
    id: "crowd", seconds: 10, title: "Festival crowd",
    tags: ["crowd", "size, excitement"],
    text: "A murmur that turns into a roar, with air horns once the excitement is up. No voices were recorded.",
    lab: "lab.html?gen=crowd&preset=Festival",
    tracks: [{ gen: "crowd", preset: "Festival", keys: { size: [[0, 0.6], [10, 0.9]], excitement: [[0, 0.1], [4, 0.3], [6, 1], [10, 0.8]] } }],
  },
  {
    id: "race", seconds: 7.5, title: "Race interface",
    tags: ["beep, pickup, finish", "events"],
    text: "Three beeps and a go, two pickups, then the finish chime.",
    lab: "lab.html?gen=finish&preset=New%20record",
    tracks: [
      { gen: "beep", fire: [[0.3, 1, 0], [1.1, 1, 0], [1.9, 1, 0]] },
      { gen: "beep", preset: "Go", fire: [[2.7, 1, 0]] },
      { gen: "pickup", preset: "Energy cell", fire: [[3.6, 1, 0]] },
      { gen: "pickup", preset: "Rare item", fire: [[4.2, 1, 0]] },
      { gen: "finish", preset: "New record", fire: [[4.9, 1, 0]] },
    ],
  },
  {
    id: "crash", seconds: 5.5, title: "Wreck",
    tags: ["metal crash, debris, rock hit", "events"],
    text: "A hard landing, a rollover, scrap metal coming to rest, then a boulder and breaking glass.",
    lab: "lab.html?gen=metal_crash&preset=Rollover",
    tracks: [
      { gen: "suspension_thud", preset: "War rig", fire: [[0.3, 1, 0]] },
      { gen: "metal_crash", preset: "Rollover", fire: [[1.2, 1, 0]] },
      { gen: "debris", preset: "Scrap metal", fire: [[1.5, 0.9, 0]] },
      { gen: "rock_hit", preset: "Boulder", fire: [[2.7, 1, 0.1]] },
      { gen: "debris", preset: "Glass", fire: [[3.1, 0.8, 0]] },
    ],
  },
  {
    id: "campfire", seconds: 8, title: "Campfire, from a text file",
    tags: ["model file", "campfire.toml"],
    text: "This one is not built into the library. It is the TOML file shown further down, compiled when the page loaded: sparks are random impulses shaped into short bursts of noise.",
    lab: "lab.html?model=campfire",
    tracks: [{ file: "campfire", keys: { intensity: [[0, 0.25], [4, 1], [8, 0.4]] } }],
  },
];

/** Value of a keyframe list at time `t` (linear between points, held outside them). */
export function at(points, t) {
  if (t <= points[0][0]) return points[0][1];
  for (let i = 1; i < points.length; i++) {
    if (t < points[i][0]) {
      const [t0, v0] = points[i - 1], [t1, v1] = points[i];
      return v0 + (v1 - v0) * ((t - t0) / (t1 - t0));
    }
  }
  return points[points.length - 1][1];
}

/** Wrap the wasm exports with the two helpers the engine's C ABI needs. */
export function engine(w) {
  const text = (len) => new TextDecoder().decode(new Uint8Array(w.memory.buffer, w.gs_str_ptr(), len).slice());
  const pass = (string, fn) => {
    const bytes = new TextEncoder().encode(string), p = w.gs_alloc_u8(bytes.length);
    new Uint8Array(w.memory.buffer, p, bytes.length).set(bytes);
    const result = fn(p, bytes.length);
    w.gs_free_u8(p, bytes.length);
    return result;
  };
  const library = JSON.parse(text(w.model_library_json()));
  return { w, text, pass, library };
}

/** Run one demo script through the engine. Returns { left, right, peak, ms }. */
export function render(eng, demo) {
  const { w, text, pass, library } = eng;
  const started = performance.now();
  const frames = Math.round(demo.seconds * SR);
  const left = new Float32Array(frames), right = new Float32Array(frames);
  const pl = w.gs_alloc_f32(BLOCK), pr = w.gs_alloc_f32(BLOCK);

  const tracks = demo.tracks.map((track) => {
    let handle;
    if (track.file) {
      const source = library.examples.find((e) => e.name === track.file);
      if (!source) throw new Error(`no model file ${track.file}`);
      handle = pass(source.text, (p, n) => w.model_from_config(p, n, SR));
    } else {
      handle = pass(track.gen, (p, n) => w.model_new(p, n, SR));
    }
    if (!handle) throw new Error(`could not create ${track.gen || track.file}: ${text(w.gs_str_len())}`);
    const desc = JSON.parse(text(w.model_desc_json(handle)));
    if (track.preset) {
      const index = desc.presets.findIndex((p) => p.name === track.preset);
      if (index < 0) throw new Error(`${desc.name} has no preset ${track.preset}`);
      w.model_load_preset(handle, index);
    }
    for (const [name, value] of Object.entries(track.params || {})) {
      const param = desc.params.find((p) => p.name === name);
      if (!param) throw new Error(`${desc.name} has no param ${name}`);
      w.model_set_param(handle, param.index, value);
    }
    const input = (name) => {
      const found = desc.inputs.find((i) => i.name === name);
      if (!found) throw new Error(`${desc.name} has no input ${name}`);
      return found.index;
    };
    const keys = Object.entries(track.keys || {}).map(([name, points]) => ({ index: input(name), points }));
    keys.forEach((k) => w.model_set_input(handle, k.index, k.points[0][1]));
    w.model_snap(handle); // start at the first values instead of spooling up to them
    const fire = (track.fire || []).map(([t, power, distance]) => ({ t, power, distance }));
    const event = fire.length ? { power: input("power"), distance: input("distance") } : null;
    return { handle, keys, fire, event, gain: track.gain ?? 1 };
  });

  for (let start = 0; start < frames; start += BLOCK) {
    const n = Math.min(BLOCK, frames - start), t = start / SR, next = (start + n) / SR;
    for (const track of tracks) {
      for (const k of track.keys) w.model_set_input(track.handle, k.index, at(k.points, t));
      for (const f of track.fire) {
        if (f.t >= t && f.t < next) {
          w.model_set_input(track.handle, track.event.power, f.power);
          w.model_set_input(track.handle, track.event.distance, f.distance);
          w.model_trigger(track.handle);
        }
      }
      w.model_render_stereo(track.handle, pl, pr, n);
      // The wasm memory can grow while models are created, so take fresh views each block.
      const l = new Float32Array(w.memory.buffer, pl, n), r = new Float32Array(w.memory.buffer, pr, n);
      for (let i = 0; i < n; i++) { left[start + i] += l[i] * track.gain; right[start + i] += r[i] * track.gain; }
    }
  }
  tracks.forEach((track) => w.model_free(track.handle));
  w.gs_free_f32(pl, BLOCK); w.gs_free_f32(pr, BLOCK);

  // A short fade at each end, and headroom if several generators summed past full scale.
  const fade = Math.round(0.02 * SR);
  let peak = 0;
  for (let i = 0; i < frames; i++) {
    const g = Math.min(1, i / fade, (frames - 1 - i) / fade);
    left[i] *= g; right[i] *= g;
    peak = Math.max(peak, Math.abs(left[i]), Math.abs(right[i]));
  }
  if (peak > 0.97) {
    const g = 0.97 / peak;
    for (let i = 0; i < frames; i++) { left[i] *= g; right[i] *= g; }
    peak = 0.97;
  }
  return { left, right, peak, ms: performance.now() - started };
}

// ---- spectrogram ----

const N = 2048;
const HANN = Float32Array.from({ length: N }, (_, i) => 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / N));
const REVERSED = (() => {
  const bits = Math.log2(N), out = new Uint32Array(N);
  for (let i = 0; i < N; i++) { let r = 0; for (let b = 0; b < bits; b++) r |= ((i >> b) & 1) << (bits - 1 - b); out[i] = r; }
  return out;
})();
const COS = Float32Array.from({ length: N / 2 }, (_, i) => Math.cos((2 * Math.PI * i) / N));
const SIN = Float32Array.from({ length: N / 2 }, (_, i) => Math.sin((2 * Math.PI * i) / N));

/** In-place radix-2 FFT of N points. */
function fft(re, im) {
  for (let i = 0; i < N; i++) { const j = REVERSED[i]; if (j > i) { let t = re[i]; re[i] = re[j]; re[j] = t; t = im[i]; im[i] = im[j]; im[j] = t; } }
  for (let size = 2; size <= N; size <<= 1) {
    const half = size >> 1, step = N / size;
    for (let base = 0; base < N; base += size) {
      for (let k = 0; k < half; k++) {
        const c = COS[k * step], s = SIN[k * step], a = base + k, b = a + half;
        const xr = re[b] * c + im[b] * s, xi = im[b] * c - re[b] * s;
        re[b] = re[a] - xr; im[b] = im[a] - xi; re[a] += xr; im[a] += xi;
      }
    }
  }
}

// Black through violet to the page's orange, then to pale yellow for the loudest parts.
const STOPS = [[0, 6, 8, 12], [0.22, 34, 16, 62], [0.48, 132, 38, 86], [0.72, 254, 106, 55], [0.9, 255, 196, 107], [1, 255, 243, 214]];
const COLOURS = (() => {
  const table = new Uint8Array(256 * 3);
  for (let i = 0; i < 256; i++) {
    const x = i / 255;
    let s = 1; while (s < STOPS.length - 1 && x > STOPS[s][0]) s++;
    const a = STOPS[s - 1], b = STOPS[s], f = (x - a[0]) / (b[0] - a[0]);
    for (let c = 0; c < 3; c++) table[i * 3 + c] = Math.round(a[c + 1] + (b[c + 1] - a[c + 1]) * f);
  }
  return table;
})();

const F_MIN = 40, F_MAX = 16000;

/** Paint a log-frequency spectrogram (40 Hz at the bottom, 16 kHz at the top) into RGBA pixels. */
export function spectrogram(left, right, width, height) {
  const pixels = new Uint8ClampedArray(width * height * 4);
  const re = new Float32Array(N), im = new Float32Array(N), mag = new Float32Array(N / 2);
  // Each pixel row covers a band of FFT bins: the loudest bin in the band is drawn.
  const rows = Array.from({ length: height }, (_, y) => {
    const hz = (row) => F_MIN * Math.pow(F_MAX / F_MIN, 1 - row / height);
    const lo = (hz(y + 1) * N) / SR, hi = (hz(y) * N) / SR;
    return [Math.max(1, Math.floor(lo)), Math.max(1, Math.min(N / 2 - 1, Math.ceil(hi)))];
  });
  const span = Math.max(0, left.length - N);
  for (let x = 0; x < width; x++) {
    const start = Math.round((span * x) / Math.max(1, width - 1));
    for (let i = 0; i < N; i++) { re[i] = 0.5 * (left[start + i] + right[start + i] || 0) * HANN[i]; im[i] = 0; }
    fft(re, im);
    for (let k = 0; k < N / 2; k++) mag[k] = re[k] * re[k] + im[k] * im[k];
    for (let y = 0; y < height; y++) {
      let power = 0;
      for (let k = rows[y][0]; k <= rows[y][1]; k++) if (mag[k] > power) power = mag[k];
      // A full-scale sine has power (N/4)^2 after the Hann window: 0 dB here.
      const db = 10 * Math.log10(power / ((N / 4) * (N / 4)) + 1e-12);
      const shade = Math.max(0, Math.min(255, Math.round(((db + 84) / 80) * 255)));   // -84 dB black .. -4 dB palest
      const p = (y * width + x) * 4;
      pixels[p] = COLOURS[shade * 3]; pixels[p + 1] = COLOURS[shade * 3 + 1]; pixels[p + 2] = COLOURS[shade * 3 + 2]; pixels[p + 3] = 255;
    }
  }
  return pixels;
}
