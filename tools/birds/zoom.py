"""Zoomed spectrogram of a stretch, fine time resolution, with a grid to read contours off.

    zoom.py <wav> <start_s> <dur_s> <out.png> [fmax=10000] [win=256] [fmin=0] [hop=48]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

import birdlib as bl

path, start, dur, out = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
fmax = float(sys.argv[5]) if len(sys.argv) > 5 else 10000
win = int(sys.argv[6]) if len(sys.argv) > 6 else 256
fmin = float(sys.argv[7]) if len(sys.argv) > 7 else 0
hop = int(sys.argv[8]) if len(sys.argv) > 8 else 48
sr, x = bl.load(path, start, dur)
x = bl.highpass(x, sr, max(150, fmin * 0.6))
fig, ax = plt.subplots(1, 1, figsize=(max(8, dur * 5), 5))
bl.specgram_ax(ax, x, sr, fmax=fmax, win=win, hop=hop, nfft=max(1024, win * 4), dyn=60, t0=start, fmin=fmin)
ax.set_xlim(start, start + dur)
step = 0.1 if dur <= 4 else 0.5
ax.set_xticks(np.arange(np.ceil(start / step) * step, start + dur + 1e-6, step))
ax.tick_params(axis="x", labelsize=7, rotation=90)
ax.grid(True, color="white", alpha=0.18, lw=0.5)
ax.set_title(os.path.basename(path), fontsize=9, loc="left")
plt.tight_layout()
plt.savefig(out, dpi=80)
print(out)
