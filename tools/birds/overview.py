"""Overview spectrogram of a recording, in rows, to find clear songs.

    overview.py <wav> <out.png> [fmax_hz=10000] [row_s=10] [max_s=120] [start_s=0]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

import birdlib as bl

path, out = sys.argv[1], sys.argv[2]
fmax = float(sys.argv[3]) if len(sys.argv) > 3 else 10000
row = float(sys.argv[4]) if len(sys.argv) > 4 else 10
max_s = float(sys.argv[5]) if len(sys.argv) > 5 else 120
start = float(sys.argv[6]) if len(sys.argv) > 6 else 0
sr, x = bl.load(path, start, max_s)
n = int(np.ceil(len(x) / (row * sr)))
fig, axs = plt.subplots(n, 1, figsize=(16, 1.6 * n + 0.4), squeeze=False)
for i in range(n):
    seg = x[int(i * row * sr):int((i + 1) * row * sr)]
    if len(seg) < 2048:
        axs[i, 0].axis("off")
        continue
    win = 1024 if fmax <= 3000 else 512
    bl.specgram_ax(axs[i, 0], seg, sr, fmax=fmax, win=win, hop=240, nfft=2048, dyn=60, t0=start + i * row)
    axs[i, 0].set_xlim(start + i * row, start + (i + 1) * row)
axs[0, 0].set_title(os.path.basename(path), fontsize=9, loc="left")
plt.tight_layout()
plt.savefig(out, dpi=60)
print(out)
