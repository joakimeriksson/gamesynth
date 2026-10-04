// Real-browser audio check of the web site: starts headless Chrome, clicks through every tab
// of the Sound Lab (lab.html) and MEASURES what reaches the speakers (an analyser tap per tab),
// then opens the landing page (index.html), checks that every demo on it was computed and that
// one plays. Anything a page or the AudioWorklet threw is reported too. Prints a JSON report
// and exits non-zero if something is silent.
//   node tools/web_audio_check.mjs [url]     (default: serves ./web on a local port; `url` is the site root)
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";

const CHROME = process.env.CHROME || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const PORT = 9300 + Math.floor(Math.random() * 500);
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".png": "image/png" };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let url = process.argv[2], server = null;
if (!url) {
  const root = resolve("web");
  server = createServer(async (req, res) => {
    const path = join(root, decodeURIComponent(new URL(req.url, "http://x").pathname).replace(/\/$/, "/index.html"));
    let body = null;
    try { body = await readFile(path); } catch {}
    res.writeHead(body ? 200 : 404, { "content-type": TYPES[extname(path)] || "application/octet-stream" });
    res.end(body ?? "");
  }).listen(PORT + 1, "127.0.0.1");
  url = `http://127.0.0.1:${PORT + 1}/`;
}

const launch = process.arch === "arm64" || process.platform !== "darwin" ? [CHROME] : ["arch", "-arm64", CHROME];
const chrome = spawn(launch[0], [...launch.slice(1), "--headless=new", "--disable-gpu", "--no-first-run", "--mute-audio",
  "--autoplay-policy=no-user-gesture-required", `--remote-debugging-port=${PORT}`, `--user-data-dir=${mkdtempSync(join(tmpdir(), "gs-chrome-"))}`, "about:blank"], { stdio: "ignore" });
const finish = (code) => { chrome.kill(); server?.close(); process.exit(code); };

let target;
for (let i = 0; i < 50 && !target; i++) {
  await sleep(200);
  try { target = (await (await fetch(`http://127.0.0.1:${PORT}/json`)).json()).find((t) => t.type === "page"); } catch {}
}
if (!target) { console.log(JSON.stringify({ error: "could not start Chrome" })); finish(2); }

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let next = 1; const waiting = new Map(), problems = [];
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  if (m.id && waiting.has(m.id)) { waiting.get(m.id)(m.result || m); waiting.delete(m.id); }
  if (m.method === "Runtime.exceptionThrown") problems.push(m.params.exceptionDetails.exception?.description?.split("\n")[0] || m.params.exceptionDetails.text);
  if (m.method === "Runtime.consoleAPICalled" && ["error", "warning"].includes(m.params.type)) problems.push(m.params.args.map((a) => a.value ?? a.description).join(" ").slice(0, 200));
  if (m.method === "Target.attachedToTarget") cdp("Runtime.enable", {}, m.params.sessionId);   // the AudioWorklet's own scope
};
const cdp = (method, params = {}, sessionId) => new Promise((r) => { const id = next++; waiting.set(id, r); ws.send(JSON.stringify({ id, method, params, sessionId })); });

await cdp("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true });
await cdp("Runtime.enable"); await cdp("Page.enable");
// Tap every analyser the page creates (one per tab, in tab order) so output can be measured.
await cdp("Page.addScriptToEvaluateOnNewDocument", { source: `(() => { const make = AudioContext.prototype.createAnalyser; window.__taps = [];
  AudioContext.prototype.createAnalyser = function () { const a = make.call(this); window.__taps.push(a); return a; }; })()` });
const base = url.replace(/[^/]*\.html.*$/, "").replace(/\/?$/, "/");
await cdp("Page.navigate", { url: base + "lab.html?tab=engines" });

const script = async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms)), $ = (s) => document.querySelector(s);
  for (let i = 0; i < 100 && $("#audio").disabled; i++) await wait(100);
  if ($("#audio").disabled) return { error: "engine never loaded: " + $("#foot").textContent };
  const level = async (tap, ms = 900) => { await wait(ms); const a = window.__taps[tap]; if (!a) return -1; const d = new Float32Array(a.fftSize); let peak = 0;
    for (let k = 0; k < 6; k++) { a.getFloatTimeDomainData(d); for (const x of d) peak = Math.max(peak, Math.abs(x)); await wait(60); } return +peak.toFixed(4); };
  const set = (sel, v) => { const e = $(sel); e.value = v; e.dispatchEvent(new Event("input", { bubbles: true })); };
  const out = {};
  $("#audio").click(); await wait(1500);
  out.context = window.__taps.length ? window.__taps[0].context.state : "no audio graph";
  set("#throttle", 0.8); out.engines = await level(0, 1500);
  $('.tab-btn[data-k="models"]').click(); $('[data-key="native:wind"]').click(); await wait(300);
  set("#gen-inputs input", 0.9); out.generator_wind = await level(1, 1200);
  $('[data-key="native:explosion"]').click(); await wait(400); $("#gen-trigger").click(); out.event_explosion = await level(1, 150);
  $('[data-key="file:campfire"]').click(); await wait(400); set("#gen-inputs input", 1); out.model_file_campfire = await level(1, 1200);
  $('.tab-btn[data-k="sfx"]').click(); await wait(200); document.querySelector("#sfx-presets .pbtn").click(); out.sfx = await level(2, 60);
  $('.tab-btn[data-k="inst"]').click(); await wait(200);
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" })); out.instrument = await level(3, 300); window.dispatchEvent(new KeyboardEvent("keyup", { key: "a" }));
  return out;
};
await sleep(1500);
const result = await cdp("Runtime.evaluate", { expression: `(${script})()`, awaitPromise: true, returnByValue: true });
const levels = result.result?.value || { error: JSON.stringify(result).slice(0, 300) };

// A link from the landing page opens the lab on that sound and preset.
await cdp("Page.navigate", { url: base + "lab.html?gen=piston&preset=Heavy%20truck" });
await sleep(2500);
const linked = await cdp("Runtime.evaluate", { returnByValue: true, expression: `(() => { const s = document.querySelector("#gen-preset");
  return [document.querySelector('.tab-btn[aria-selected="true"]')?.dataset.k, document.querySelector("#gen-title")?.textContent, s?.options[s.selectedIndex]?.textContent].join(" / "); })()` });
if (linked.result?.value !== "models / piston / Heavy truck") problems.push("error: lab.html?gen=piston&preset=Heavy%20truck opened " + linked.result?.value);

// The landing page: every demo computed, the lists filled from the library, and playback audible.
await cdp("Page.navigate", { url: base });
const landing = async () => {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 200 && !(window.__brusverk?.ready || window.__brusverk?.error); i++) await wait(100);
  const b = window.__brusverk;
  if (!b?.ready) return { error: "landing page: " + (b?.error || "demos never finished computing") };
  const out = {};
  for (const [id, d] of Object.entries(b.demos)) out["landing_" + id] = d.peak;
  const cards = document.querySelectorAll("#cards .card").length, listed = document.querySelectorAll("#all li a").length;
  if (cards !== Object.keys(b.demos).length - 1 || listed < 50 || document.querySelector(".wait")) return { error: `landing page: ${cards} cards, ${listed} listed sounds, placeholders left: ${!!document.querySelector(".wait")}` };
  document.querySelector("#demo-truck .play").click(); await wait(3500);   // into the first pull
  const a = window.__taps[0], d = new Float32Array(a ? a.fftSize : 0); let peak = 0;
  for (let k = 0; k < 6 && a; k++) { a.getFloatTimeDomainData(d); for (const x of d) peak = Math.max(peak, Math.abs(x)); await wait(60); }
  out.landing_playback = +peak.toFixed(4);
  out.landing_playing = document.querySelector("#demo-truck").classList.contains("playing") ? 1 : 0;
  return out;
};
const page = await cdp("Runtime.evaluate", { expression: `(${landing})()`, awaitPromise: true, returnByValue: true });
Object.assign(levels, page.result?.value || { error: JSON.stringify(page).slice(0, 300) });
const silent = Object.entries(levels).filter(([k, v]) => typeof v === "number" && v < 0.01).map(([k]) => k);
const errors = [...new Set(problems)];
console.log(JSON.stringify({ url, levels, silent, problems: errors }, null, 1));
// Any error counts: an exception in the worklet can leave audio playing but a feature dead.
finish(silent.length || levels.error || errors.some((p) => /error/i.test(p)) ? 1 : 0);
