"""Side-by-side zoomed spectrograms: real recording(s) above, ours below, same scale.

    compare.py <out.png> <fmax> <dur_s> <title> <label>=<wav>@<start> [<label>=<wav>@<start> ...]

Each panel is `dur_s` long, 0..fmax Hz, ~2 ms hop.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

import numpy as np

import birdlib as bl

out, fmax, dur, title = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
panels = []
for arg in sys.argv[5:]:
    label, rest = arg.split("=", 1)
    path, start = rest.rsplit("@", 1)
    if start.startswith("auto"):
        # The first song after `auto<seconds>`: where the level first comes within 25 dB of
        # the loudest 10 ms, minus 0.25 s.
        skip = float(start[4:] or 0)
        sr, y = bl.load(path, skip)
        n = sr // 100
        e = np.array([np.sqrt(np.mean(y[i:i + n] ** 2)) + 1e-9 for i in range(0, len(y) - n, n)])
        k = int(np.argmax(bl.db(e) > bl.db(e).max() - 25))
        start = max(0.0, skip + k * 0.01 - 0.25)
    panels.append((label, path, float(start)))
fig, axs = plt.subplots(len(panels), 1, figsize=(13, 2.6 * len(panels) + 0.5), squeeze=False)
win = 1024 if fmax <= 2500 else 512 if fmax <= 5000 else 384
for ax, (label, path, start) in zip(axs[:, 0], panels):
    sr, x = bl.load(path, start, dur)
    x = bl.highpass(x, sr, 150 if fmax <= 2500 else 300)
    bl.specgram_ax(ax, x, sr, fmax=fmax, win=win, hop=96, nfft=2048, dyn=60, t0=0.0, title=f"{label}  ({os.path.basename(path)} @ {start:.1f} s)")
    ax.set_xlim(0, dur)
axs[-1, 0].set_xlabel("seconds")
fig.suptitle(title, fontsize=11, x=0.01, ha="left")
plt.tight_layout()
plt.savefig(out, dpi=72)
print(out)
