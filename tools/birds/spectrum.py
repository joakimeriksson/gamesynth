"""Average spectrum of the loud frames (within 20 dB of the loudest), in third-octave bands,
relative to the strongest band, for several sources side by side.

    spectrum.py <fmin> <fmax> <label>=<wav>@<start>:<dur> [...]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np

import birdlib as bl

fmin, fmax = float(sys.argv[1]), float(sys.argv[2])
edges = fmin * 2 ** (np.arange(0, np.log2(fmax / fmin) + 1e-6, 1 / 3))
rows = []
for arg in sys.argv[3:]:
    label, rest = arg.split("=", 1)
    path, span = rest.rsplit("@", 1)
    start, dur = (float(v) for v in span.split(":"))
    sr, x = bl.load(path, start, dur)
    x = bl.highpass(x, sr, fmin * 0.7)
    mag, freqs, _ = bl.stft(x, sr, 1024, 480, 2048)
    p = (mag ** 2).sum(1)
    loud = p > p.max() * 10 ** (-2.0)
    spec = (mag[loud] ** 2).mean(0)
    bands = [spec[(freqs >= a) & (freqs < b)].sum() for a, b in zip(edges, edges[1:])]
    b = 10 * np.log10(np.array(bands) + 1e-20)
    rows.append((label, b - b.max()))
print("band Hz " + " ".join(f"{int(a):>6d}" for a in edges[:-1]))
for label, b in rows:
    print(f"{label[:7]:<7} " + " ".join(f"{v:6.1f}" for v in b))
