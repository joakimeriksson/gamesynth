#!/usr/bin/env python3
"""Fit the `piston` engine to a recording of a real one.

    tools/fit_engine.py recording.wav 0.3:1.9:835:0 10:13.5:2992:0.25

Each condition is `t0:t1:rpm:throttle`: a steady stretch of the recording, the engine speed
there (measure it with tools/engine_analysis.py) and a guess of the throttle. The first
condition should be the idle. Coordinate descent over the combustion, exhaust and mechanical
parameters minimises the third-octave level error plus half the error of the level pattern
over the first 16 harmonics of the 720 degree cycle. Prints the fitted parameters in the form
`render_params` and `SoundGenerator.set_param` accept. Needs numpy and a release build.
"""
import contextlib, io, os, subprocess, sys
import numpy as np
import engine_analysis as ea

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RENDER = os.path.join(ROOT, "target/release/examples/render_params")
MAX_RPM = 6500
SPACE = {  # name: (start, low, high, first step)
    "exhaust/muffling": (0.45, 0.0, 1.0, 0.15), "exhaust/resonance": (0.6, 0.0, 0.9, 0.15), "exhaust/length_m": (2.8, 0.8, 5.5, 0.6),
    "exhaust/header_m": (0.8, 0.25, 1.8, 0.2), "exhaust/unequal_ms": (1.5, 0.0, 5.0, 0.6), "exhaust/interference": (0.5, 0.0, 1.0, 0.2),
    "pulse/degrees": (60, 20, 160, 20), "pulse/turbulence": (0.35, 0.0, 1.0, 0.2), "pulse/steepening": (0.5, 0.0, 1.0, 0.2),
    "intake/level": (0.4, 0.0, 1.0, 0.2), "intake/hz": (420, 150, 1800, 150), "mechanical/valvetrain": (0.3, 0.0, 1.0, 0.2),
    "mechanical/block": (0.3, 0.0, 1.0, 0.2), "mechanical/fan": (0.25, 0.0, 1.0, 0.25), "engine/roughness": (0.3, 0.0, 1.0, 0.2),
    "engine/cam_lope": (0.35, 0.0, 1.0, 0.2),
}


def quiet(*args, **kwargs):
    with contextlib.redirect_stdout(io.StringIO()):
        return ea.analyse(*args, **kwargs)


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    recording = sys.argv[1]
    conds = [tuple(float(v) for v in c.split(":")) for c in sys.argv[2:]]
    idle = conds[0][2]
    subprocess.run(["cargo", "build", "-q", "-p", "brusverk-core", "--release", "--example", "render_params"], cwd=ROOT, check=True)
    ref = [quiet(recording, t0, t1, 0, 0, fc=rpm / 120) for t0, t1, rpm, _ in conds]
    tmp = os.path.join(ROOT, "target", "fit_engine.wav")

    def cost(params, detail=False):
        total, parts = 0.0, []
        for (t0, t1, rpm, throttle), (_, bands_ref, harm_ref) in zip(conds, ref):
            rev = (rpm - idle) / (MAX_RPM - idle)
            args = [RENDER, "piston", tmp, "4", f"engine/idle_rpm={idle}", f"engine/max_rpm={MAX_RPM}", "engine/external_rpm=1", f"@rpm={rev}", f"@throttle={throttle}"]
            subprocess.run(args + [f"{k}={v:.4f}" for k, v in params.items()], check=True)
            _, bands, harm = quiet(tmp, 0.8, 3.8, 0, 0, fc=rpm / 120)
            keys = [k for k in bands if 40 <= k <= 8000]
            band = np.mean(np.abs([bands[k] - bands_ref[k] for k in keys]))
            pattern = np.mean(np.abs(np.clip(harm[:16], -30, 0) - np.clip(harm_ref[:16], -30, 0)))
            total += band + 0.5 * pattern
            parts.append((rpm, band, pattern))
        return (total, parts) if detail else total

    cur = {k: v[0] for k, v in SPACE.items()}
    best = cost(cur)
    print(f"start: {best:.2f}", flush=True)
    for rnd in range(3):
        for k, (_, lo, hi, step) in SPACE.items():
            for cand in (cur[k] - step / 2**rnd, cur[k] + step / 2**rnd):
                cand = float(np.clip(cand, lo, hi))
                if cand != cur[k]:
                    c = cost({**cur, k: cand})
                    if c < best - 0.02:
                        best, cur[k] = c, cand
        print(f"round {rnd + 1}: {best:.2f}", flush=True)
    _, parts = cost(cur, True)
    print(f"engine/idle_rpm={idle:.0f} " + " ".join(f"{k}={v:.3f}" for k, v in cur.items()))
    for rpm, band, pattern in parts:
        print(f"  at {rpm:.0f} rpm: third-octave error {band:.1f} dB, harmonic pattern error {pattern:.1f} dB")


if __name__ == "__main__":
    main()
