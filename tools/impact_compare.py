#!/usr/bin/env python3
"""Compare single hits, real against ours, one row per material:

  1. spectrogram of the real hit          2. spectrogram of ours (same scale, log frequency)
  3. decay curves: 5 ms RMS envelope in dB below each hit's peak, real and ours overlaid
  4. ring spectrum (5 ms .. end, high-resolution FFT) in dB, real and ours overlaid,
     with the mode peaks that tools/impact_measure.py picks marked

    python tools/impact_compare.py out.png "label" real.wav@start ours.wav@start [secs] -- ...

Each row's arguments end with `--`. Needs numpy, scipy, matplotlib.
"""
import os
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from scipy import signal

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import impact_measure as im  # noqa: E402

SR = im.SR


def excerpt(spec, secs):
    path, _, start = spec.partition("@")
    _, x = im.load(path)
    s = int((float(start or 0) + im.PAD) * SR)
    x = x[s: s + int(secs * SR)]
    # start at the hit: the first sample within 30 dB of the excerpt's peak
    a = np.abs(x)
    k = int(np.argmax(a > a.max() * 0.03))
    x = x[max(0, k - int(0.003 * SR)):]
    return x / (np.abs(x).max() + 1e-12)


def spec_ax(ax, x, title):
    f, t, S = signal.spectrogram(x, SR, nperseg=1024, noverlap=896)
    db = 10 * np.log10(S + 1e-14)
    ax.pcolormesh(t, f, db, vmin=db.max() - 75, vmax=db.max(), shading="auto", cmap="magma")
    ax.set_yscale("symlog", linthresh=300)
    ax.set_ylim(60, 20000)
    ax.set_title(title, fontsize=9)
    ax.tick_params(labelsize=7)


def main():
    out = sys.argv[1]
    rows, cur = [], []
    for a in sys.argv[2:] + ["--"]:
        if a == "--":
            if cur:
                rows.append(cur)
            cur = []
        else:
            cur.append(a)
    fig, axs = plt.subplots(len(rows), 4, figsize=(20, 2.9 * len(rows)), gridspec_kw=dict(width_ratios=[1, 1, 1, 1.4]))
    axs = np.atleast_2d(axs)
    for ax, row in zip(axs, rows):
        label, real, ours = row[:3]
        secs = float(row[3]) if len(row) > 3 else 1.0
        xr, xo = excerpt(real, secs), excerpt(ours, secs)
        spec_ax(ax[0], xr, f"{label}: real ({os.path.basename(real.split('@')[0])})")
        spec_ax(ax[1], xo, f"{label}: ours")
        for x, name, col in ((xr, "real", "#888888"), (xo, "ours", "#E0662E")):
            e, hop = im.env_db(x, 5.0)
            e -= e.max()
            ax[2].plot(np.arange(len(e)) * hop / SR, e, color=col, lw=1.2, label=name)
            ring = x[int(0.005 * SR):]
            f, db = im.ring_spectrum(ring)
            m = (f > 50) & (f < 16000)
            ax[3].semilogx(f[m][::8], db[m][::8], color=col, lw=0.8, label=name)
            for hz, d in im.mode_peaks(ring):
                ax[3].plot(hz, d, "v", color=col, ms=5)
        ax[2].set_ylim(-70, 3)
        ax[2].set_xlim(0, secs)
        ax[2].axhline(-20, color="#cccccc", lw=0.5)
        ax[2].axhline(-40, color="#cccccc", lw=0.5)
        ax[2].set_title("decay (dB below peak)", fontsize=9)
        ax[2].legend(fontsize=7)
        ax[3].set_ylim(-80, 3)
        ax[3].set_xlim(50, 16000)
        ax[3].set_title("ring spectrum, ▼ = picked modes", fontsize=9)
        ax[3].legend(fontsize=7)
        for a in ax[2:]:
            a.tick_params(labelsize=7)
    plt.tight_layout()
    plt.savefig(out, dpi=70)


if __name__ == "__main__":
    main()
