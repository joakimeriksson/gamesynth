"""ASCII level envelope (0.5 ms frames, 45 dB range) of a stretch, to read rhythms by eye.

    envelope.py <wav> <start_s> <dur_s>
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np

import birdlib as bl

sr, x = bl.load(sys.argv[1], float(sys.argv[2]), float(sys.argv[3]))
n = sr // 2000
e = np.array([np.sqrt(np.mean(x[i:i + n] ** 2)) for i in range(0, len(x) - n, n)])
top = e.max()
line = "".join(" .:-=+*#%@"[min(9, int(max(0, 20 * np.log10(v / top + 1e-9) + 45) / 4.5))] for v in e)
for k in range(0, len(line), 100):
    print(f"{float(sys.argv[2]) + k * n / sr:6.3f} {line[k:k + 100]}")
