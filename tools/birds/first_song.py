"""Print the time (s) just before the first song in a WAV: where the 10 ms level first comes
within 25 dB of the loudest, minus 0.3 s (never below 0).

    first_song.py <wav>
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np

import birdlib as bl

sr, y = bl.load(sys.argv[1])
n = sr // 100
e = bl.db(np.array([np.sqrt(np.mean(y[i:i + n] ** 2)) + 1e-9 for i in range(0, len(y) - n, n)]))
print(f"{max(0.0, int(np.argmax(e > e.max() - 25)) * 0.01 - 0.3):.2f}")
