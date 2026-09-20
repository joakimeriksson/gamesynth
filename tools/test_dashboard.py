#!/usr/bin/env python3
"""Build the gamesynth test dashboard.

    tools/test_dashboard.py            # run everything, write dashboard/index.html
    tools/test_dashboard.py --quick    # skip the release benchmarks
    tools/test_dashboard.py --sounds   # re-render and re-review the sounds, keep the last test results
    tools/test_dashboard.py --restyle  # re-apply the page template to the last results

Runs every test surface (core tests, clippy, the WebAssembly build, the Godot smoke test),
renders every generator and model file with a standard input sweep, scores each sound with
detectors for the faults a spectrogram review looks for, and writes a self-contained page with
a spectrogram and an audio player per sound. Needs numpy, scipy and matplotlib; node, godot,
ffmpeg and the wasm toolchain are used when present and reported as skipped when not.
"""
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import time
import wave

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from scipy.signal import spectrogram, welch

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "dashboard")
WAV = os.path.join(OUT, "wav")

# Sounds whose character legitimately breaks a generic rule.
FLAT_BY_DESIGN = {"drone": "level is constant by design; tension changes the timbre",
                  "mine_armed": "proximity changes the tick rate, not the level"}
SPARSE_BY_DESIGN = {"geiger": "isolated clicks are the sound", "radio": "clicks over quiet hiss"}
# Noise-based sounds that must not ring at fixed pitches (the "glass chime" fault).
BROADBAND = {"wind", "rain", "fire", "stream", "ocean", "crowd", "rain_on_tent", "campfire"}
# `recharge` is gated by its first input and pitched by its second, so the level sweep
# still applies; nothing else needs an exemption so far.


def _native_prefix():
    """On Apple silicon, a Python running under Rosetta would start cargo's linker and Godot as
    x86_64 too (link errors, extension fails to load). Launch tools natively instead."""
    try:
        translated = subprocess.run(["sysctl", "-n", "sysctl.proc_translated"], capture_output=True, text=True).stdout.strip() == "1"
    except FileNotFoundError:
        translated = False
    return ["arch", "-arm64"] if translated else []


NATIVE = _native_prefix()


def run(cmd, timeout=1800, env=None):
    start = time.time()
    cmd = NATIVE + cmd
    try:
        # One merged stream keeps cargo's "Running tests/x.rs" headers in order with the results.
        p = subprocess.run(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=timeout, env=env)
        return p.returncode, p.stdout, time.time() - start
    except (FileNotFoundError, subprocess.TimeoutExpired) as e:
        return -1, str(e), time.time() - start


def suite(name, what, tests, seconds, skipped=None, note=""):
    failed = [t for t in tests if t["status"] == "fail"]
    status = "skip" if skipped else ("fail" if failed else "pass")
    return {"name": name, "what": what, "status": status, "tests": tests, "seconds": round(seconds, 1),
            "passed": sum(t["status"] == "pass" for t in tests), "failed": len(failed), "note": skipped or note}


# ------------------------------------------------------------------------------------------
# Test surfaces
# ------------------------------------------------------------------------------------------

def core_tests():
    code, out, secs = run(["cargo", "test", "-p", "gamesynth-core"])
    groups, current = {}, "unit"
    for line in out.splitlines():
        m = re.search(r"Running (?:unittests )?(\S+)", line)
        if m:
            current = os.path.splitext(os.path.basename(m.group(1)))[0]
            current = "unit (expressions)" if current == "lib" else current
        m = re.match(r"test (\S+) \.\.\. (ok|FAILED|ignored)", line)
        if m and m.group(2) != "ignored" and "Doc-tests" not in current:
            groups.setdefault(current, []).append({"name": m.group(1), "status": "pass" if m.group(2) == "ok" else "fail", "detail": ""})
    for m in re.finditer(r"---- (\S+) stdout ----\n(.*?)\n\n", out, re.S):
        for tests in groups.values():
            for t in tests:
                if t["name"] == m.group(1):
                    t["detail"] = m.group(2).strip()[-400:]
    what = {"unit (expressions)": "Formula parser and evaluator", "generators": "Native generator library", "graph": "Model files: compile, render, reject bad input",
            "jet": "Jet engine model", "synth": "Synth voice, SFX presets, patches"}
    suites = [suite(f"core · {g}", what.get(g, ""), tests, secs / max(len(groups), 1)) for g, tests in groups.items()]
    if code != 0 and not any(s["status"] == "fail" for s in suites):
        suites.append(suite("core · build", "cargo test", [{"name": "cargo test", "status": "fail", "detail": out[-600:]}], secs))
    return suites


def clippy():
    code, out, secs = run(["cargo", "clippy", "--workspace", "--all-targets"])
    warnings = [w for w in re.findall(r"^(?:warning|error)(?:\[\w+\])?: (.+)$", out, re.M) if "generated" not in w and "could not compile" not in w]
    tests = [{"name": "no lints across the workspace", "status": "pass" if code == 0 and not warnings else "fail", "detail": "\n".join(sorted(set(warnings)))[:600]}]
    return suite("clippy", "Lints for core, Godot and wasm crates", tests, secs)


def wasm():
    wasm_file = os.path.join(ROOT, "web/pkg/gamesynth_wasm.wasm")
    if not shutil.which("node"):
        return suite("wasm", "WebAssembly build as the web lab uses it", [], 0, skipped="node is not installed")
    cargo = "/opt/homebrew/opt/rustup/bin/cargo" if os.path.exists("/opt/homebrew/opt/rustup/bin/cargo") else "cargo"
    code, out, build_secs = run(["sh", "web/build.sh"], env={**os.environ, "CARGO": cargo})
    built = code == 0
    if not os.path.exists(wasm_file):
        return suite("wasm", "WebAssembly build as the web lab uses it", [], build_secs, skipped="no wasm build (needs the wasm32-unknown-unknown target)")
    code, out, secs = run(["node", "tools/wasm_check.mjs", wasm_file])
    try:
        checks = json.loads(out.strip().splitlines()[-1])["checks"]
    except (ValueError, IndexError):
        checks = [{"name": "wasm check script", "ok": False, "detail": out[-400:]}]
    tests = [{"name": c["name"], "status": "pass" if c["ok"] else "fail", "detail": c["detail"]} for c in checks]
    note = f"{os.path.getsize(wasm_file) // 1024} KB" + ("" if built else " (rebuild failed; checked the existing file)")
    return suite("wasm", "WebAssembly build as the web lab uses it", tests, build_secs + secs, note=note)


def godot():
    if not shutil.which("godot"):
        return suite("godot", "Headless Godot smoke test of the extension", [], 0, skipped="godot is not installed")
    code, out, build_secs = run(["cargo", "build", "-p", "gamesynth-godot"])
    if code != 0:
        return suite("godot", "Headless Godot smoke test of the extension", [{"name": "extension builds", "status": "fail", "detail": out[-600:]}], build_secs)
    run(["godot", "--headless", "--path", "godot", "--import"], timeout=300)
    code, out, secs = run(["godot", "--headless", "--path", "godot", "-s", "tests/smoke.gd"], timeout=600)
    tests, section = [], ""
    for line in out.splitlines():
        m = re.match(r"\s+(ok|FAIL)\s+(.*)", line)
        if m:
            tests.append({"name": f"{section}: {m.group(2)}"[:140], "status": "pass" if m.group(1) == "ok" else "fail", "detail": ""})
        elif re.match(r"^[A-Z][A-Za-z ]+$", line.strip()) and not line.startswith(" "):
            section = line.strip()
    if not tests:
        tests = [{"name": "smoke test ran", "status": "fail", "detail": out[-600:]}]
    return suite("godot", "Headless Godot smoke test of the extension", tests, build_secs + secs)


def benchmarks():
    rows = []
    code, out, _ = run(["cargo", "test", "-p", "gamesynth-core", "--release", "--test", "graph", "--", "graph_cost", "--nocapture"])
    m = re.search(r"graph jet_lite \((\d+) nodes\): ([\d.]+)x real time, native jet: ([\d.]+)x real time, ratio ([\d.]+)", out)
    if m:
        rows += [{"label": "Native jet", "value": float(m.group(3)), "unit": "× real time"},
                 {"label": f"jet_lite model file ({m.group(1)} nodes)", "value": float(m.group(2)), "unit": "× real time"},
                 {"label": "Model file cost vs native", "value": float(m.group(4)), "unit": "× slower"}]
    code, out, _ = run(["cargo", "test", "-p", "gamesynth-core", "--release", "--test", "generators", "--", "whole_library", "--nocapture"])
    m = re.search(r"library: all (\d+) generators together render at ([\d.]+)x real time", out)
    if m:
        rows.append({"label": f"All {m.group(1)} generators at once", "value": float(m.group(2)), "unit": "× real time"})
    return rows


# ------------------------------------------------------------------------------------------
# Sound review
# ------------------------------------------------------------------------------------------

def load(path):
    with wave.open(path) as w:
        sr = w.getframerate()
        return np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(float) / 32767, sr


def rms(x):
    return float(np.sqrt(np.mean(x**2))) if len(x) else 0.0


def chime_prominence(x, sr):
    """How far narrow spectral peaks stand above the smooth spectrum, 800 Hz - 8 kHz, in dB.

    Calibrated on this engine: white noise scores 0.5 dB, the shipped rain 0.9 dB, and rain with
    its drops forced to one fixed pitch (the "glass chime" fault) 6.6 dB. The limit is 4 dB."""
    f, p = welch(x, fs=sr, nperseg=8192)
    band = (f >= 800) & (f <= 8000)
    db = 10 * np.log10(p[band] + 1e-16)
    logf = np.log2(f[band])
    smooth = np.array([np.median(db[np.abs(logf - c) < 1 / 3]) for c in logf])
    fine = np.array([np.mean(db[np.abs(logf - c) < 1 / 48]) for c in logf])
    return float(np.percentile(fine - smooth, 99))


def treble(x, sr):
    f, p = welch(x, fs=sr, nperseg=4096)
    return float(np.sum(p[f > 2000]) / (np.sum(p) + 1e-20))


def picture(name, x, sr, marks):
    fs, ts, power = spectrogram(x, fs=sr, nperseg=2048, noverlap=1536, window="hann")
    band = (fs >= 30) & (fs <= 20000)
    fig = plt.figure(figsize=(9, 2.3), dpi=100)
    ax = fig.add_axes([0, 0, 1, 1])
    ax.pcolormesh(ts, fs[band], 10 * np.log10(power[band] + 1e-12), vmin=-105, vmax=-35, cmap="magma", shading="auto")
    ax.set_yscale("log")
    ax.set_ylim(30, 20000)
    ax.set_xlim(0, len(x) / sr)
    ax.axis("off")
    for mark in marks:
        ax.axvline(mark, color="w", lw=0.6, alpha=0.45)
    os.makedirs(os.path.join(OUT, "spec"), exist_ok=True)
    fig.savefig(os.path.join(OUT, "spec", f"{name}.png"), facecolor="black")
    plt.close(fig)
    win = sr // 20
    level = np.sqrt(np.convolve(x**2, np.ones(win) / win, "same"))[:: max(1, len(x) // 120)]
    return [round(float(v), 3) for v in level]


def review_event(name, path, engine):
    """One-shots are rendered as four events 3 s apart: full power, full power again, power 0.4,
    and full power at distance 0.8 (for model files: with their second input raised)."""
    x, sr = load(path)
    ev = [x[int(k * 3 * sr):int((k + 1) * 3 * sr)] for k in range(4)]
    peak = float(np.max(np.abs(x)))
    first = float(np.max(np.abs(ev[0])))
    head = slice(0, int(1.5 * sr))
    differ = float(np.max(np.abs(ev[0][head] - ev[1][head])) / (first + 1e-9))
    tail = rms(ev[0][int(2.7 * sr):])
    checks = [
        {"name": "Bounded", "status": "pass" if np.all(np.isfinite(x)) and peak <= 1.0 else "fail", "detail": f"peak {peak:.2f}"},
        {"name": "Fires", "status": "pass" if first > 0.15 else "fail", "detail": f"peak {first:.2f} at full power (want > 0.15)"},
        {"name": "Rings out", "status": "pass" if tail < 0.01 else "fail", "detail": f"rms {tail:.4f} by 2.7 s after the trigger (want < 0.01)"},
        {"name": "Responds to power", "status": "pass" if rms(ev[2]) < rms(ev[0]) * 0.8 else "fail", "detail": f"rms {rms(ev[0]):.3f} at power 1, {rms(ev[2]):.3f} at 0.4"},
    ]
    if name in ("beep", "lock_on"):
        checks.append({"name": "Varies", "status": "exempt", "detail": "UI tones are meant to repeat, so their layers barely vary"})
    else:
        checks.append({"name": "Varies", "status": "pass" if differ > 0.02 else "fail", "detail": f"two identical triggers differ by {differ * 100:.0f}% of peak (want > 2%)"})
    if engine == "native":
        # Share of energy above 2 kHz. (A spectral centroid barely moves for sounds that are
        # mostly low already, like a cannon thump.)
        near, far = treble(ev[0][head], sr), treble(ev[3][head], sr)
        checks.append({"name": "Distance dulls", "status": "pass" if far < near * 0.6 and rms(ev[3]) < rms(ev[0]) else "fail", "detail": f"energy above 2 kHz: {near * 100:.1f}% near, {far * 100:.1f}% far"})
    levels = picture(name, x, sr, (3, 6, 9))
    return {"name": name, "engine": engine, "kind": "event", "checks": checks, "status": "fail" if any(c["status"] == "fail" for c in checks) else "pass",
            "levels": levels, "sub": 0, "spec": f"spec/{name}.png", "audio": encode(name, path)}


def review(name, path, engine):
    x, sr = load(path)
    seg = lambda a, b: x[int(a * sr):int(b * sr)]
    low, high = rms(seg(0.5, 1.5)), rms(seg(3.0, 4.0))
    win = sr // 20
    env = np.sqrt(np.mean(seg(2.0, 8.0)[: (6 * sr // win) * win].reshape(-1, win) ** 2, axis=1))
    gaps = int(np.sum(env < 0.08 * np.median(env))) if np.median(env) > 0 else len(env)
    peak = float(np.max(np.abs(x)))
    f, p = welch(seg(3.0, 6.0), fs=sr, nperseg=8192)
    sub = float(np.sum(p[f < 100]) / (np.sum(p) + 1e-20))

    checks = [{"name": "Bounded", "status": "pass" if np.all(np.isfinite(x)) and peak <= 1.0 else "fail", "detail": f"peak {peak:.2f}"}]
    if name in FLAT_BY_DESIGN:
        checks.append({"name": "Follows input", "status": "exempt", "detail": FLAT_BY_DESIGN[name]})
    else:
        checks.append({"name": "Follows input", "status": "pass" if high > low * 1.2 else "fail", "detail": f"rms {low:.3f} at low input, {high:.3f} at high"})
    if name in SPARSE_BY_DESIGN:
        checks.append({"name": "No dropouts", "status": "exempt", "detail": SPARSE_BY_DESIGN[name]})
    else:
        checks.append({"name": "No dropouts", "status": "pass" if gaps <= 2 else "fail", "detail": f"{gaps} silent 50 ms windows while driven"})
    if name in BROADBAND:
        prom = chime_prominence(seg(2.0, 8.0), sr)
        checks.append({"name": "Not a chime", "status": "pass" if prom < 4.0 else "fail", "detail": f"narrow peaks stand {prom:.1f} dB above the spectrum (limit 4)"})
    audible = rms(seg(3.0, 6.0))
    checks.append({"name": "Audible", "status": "pass" if 0.02 < audible < 0.6 else "fail", "detail": f"rms {audible:.3f} at full input (want 0.02 - 0.6)"})

    levels = picture(name, x, sr, (4, 6))
    return {"name": name, "engine": engine, "kind": "continuous", "checks": checks, "status": "fail" if any(c["status"] == "fail" for c in checks) else "pass",
            "levels": levels, "sub": round(sub, 2), "spec": f"spec/{name}.png", "audio": encode(name, path)}


def encode(name, path):
    """MP3 because it plays everywhere the page might be hosted; WAV when no encoder exists."""
    folder = os.path.join(OUT, "audio")
    os.makedirs(folder, exist_ok=True)
    if shutil.which("ffmpeg"):
        code, _, _ = run(["ffmpeg", "-y", "-loglevel", "error", "-i", path, "-codec:a", "libmp3lame", "-b:a", "128k", os.path.join(folder, f"{name}.mp3")])
        if code == 0:
            return f"audio/{name}.mp3"
    shutil.copy(path, os.path.join(folder, f"{name}.wav"))
    return f"audio/{name}.wav"


def sounds():
    os.makedirs(WAV, exist_ok=True)
    code, out, _ = run(["cargo", "run", "-q", "-p", "gamesynth-core", "--release", "--example", "render_models", "--", WAV])
    presets = {}
    for line in out.splitlines():
        m = re.match(r"(\w+)\s+(.+?)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)$", line)
        if m and m.group(1) != "model":
            presets.setdefault(m.group(1), []).append({"name": m.group(2).strip(), "rms": float(m.group(5)), "peak": float(m.group(6))})
    code, graph_out, _ = run(["cargo", "run", "-q", "-p", "gamesynth-core", "--release", "--example", "render_graphs", "--", WAV])
    events = set(re.findall(r"^# one_shot (\w+)", out + graph_out, re.M))
    result = []
    for file in sorted(os.listdir(WAV)):
        if file.endswith("_default.wav"):
            name = file[: -len("_default.wav")]
            card = (review_event if name in events else review)(name, os.path.join(WAV, file), "native")
            card["presets"] = presets.get(name, [])
            quiet = [p["name"] for p in card["presets"] if not 0.02 < p["rms"] < 0.6 or p["peak"] > 1.0]
            card["checks"].append({"name": "Presets in range", "status": "fail" if quiet else "pass", "detail": ", ".join(quiet) or f"{len(card['presets'])} presets between 0.02 and 0.6 rms"})
            card["status"] = "fail" if quiet else card["status"]
            result.append(card)
    for file in sorted(os.listdir(WAV)):
        if file.startswith("graph_"):
            name = file[len("graph_"):-4]
            result.append((review_event if name in events else review)(name, os.path.join(WAV, file), "model file"))
    order = ["jet", "hover", "combustion", "motor", "rotor", "scrape", "laser", "beam", "plasma", "cannon", "rocket", "mine_drop", "mine_blast", "explosion", "emp", "quake", "impact", "shield_hit", "shield_up", "lock_on", "boost", "airbrake", "pickup", "beep", "wind", "rain", "fire", "stream", "ocean", "electric", "drone", "crowd", "radio", "siren"]
    result.sort(key=lambda c: (c["engine"] != "native", order.index(c["name"]) if c["name"] in order else 99, c["name"]))
    return result


def write_page(data):
    template = open(os.path.join(ROOT, "tools/dashboard_template.html")).read()
    open(os.path.join(OUT, "index.html"), "w").write(template.replace("/*DATA*/null", json.dumps(data)))


def main():
    if "--restyle" in sys.argv:
        # Re-apply the page template to the results of the last run.
        old = open(os.path.join(OUT, "index.html")).read()
        write_page(json.loads(re.search(r"const DATA = (.*);\n", old).group(1)))
        print("dashboard/index.html restyled")
        return
    quick = "--quick" in sys.argv
    previous = None
    if "--sounds" in sys.argv:
        # Re-render and re-review the sounds only; keep the last run's test results.
        old = open(os.path.join(OUT, "index.html")).read()
        previous = json.loads(re.search(r"const DATA = (.*);\n", old).group(1))
    shutil.rmtree(os.path.join(OUT, "audio"), ignore_errors=True)
    os.makedirs(OUT, exist_ok=True)
    code, head, _ = run(["git", "rev-parse", "--short", "HEAD"])
    code, dirty, _ = run(["git", "status", "--porcelain"])
    code, subject, _ = run(["git", "log", "-1", "--format=%s"])
    print("running test suites…", flush=True)
    suites = previous["suites"] if previous else core_tests() + [clippy(), wasm(), godot()]
    print("rendering and reviewing sounds…", flush=True)
    cards = sounds()
    data = {
        "commit": head.strip(), "dirty": bool(dirty.strip()), "subject": subject.strip(),
        "generated": datetime.datetime.now().strftime("%Y-%m-%d %H:%M"),
        "suites": suites, "sounds": cards, "benchmarks": previous["benchmarks"] if previous else [] if quick else benchmarks(),
    }
    write_page(data)
    files = {c[k]: f"dashboard/{c[k]}" for c in cards for k in ("spec", "audio")}
    json.dump(files, open(os.path.join(OUT, "files.json"), "w"), indent=1)
    shutil.rmtree(WAV, ignore_errors=True)
    bad = [s["name"] for s in suites if s["status"] == "fail"] + [c["name"] for c in cards if c["status"] == "fail"]
    print(f"dashboard/index.html: {sum(s['passed'] for s in suites)} tests passed, {len(cards)} sounds reviewed, " + (f"NEEDS ATTENTION: {', '.join(bad)}" if bad else "all clear"))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
