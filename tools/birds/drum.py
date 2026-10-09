"""Woodpecker drumming: strike times (onsets of the band envelope), strike intervals across
the roll, and the knock's spectrum peaks.

    drum.py <wav> <start_s> <dur_s> [lo_hz=300] [hi_hz=4000]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
import scipy.signal as ss

import birdlib as bl

path, start, dur = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
lo = float(sys.argv[4]) if len(sys.argv) > 4 else 300
hi = float(sys.argv[5]) if len(sys.argv) > 5 else 4000
sr, x = bl.load(path, start, dur)
y = bl.bandpass(x, sr, lo, hi)
hop = sr // 2000
env = np.sqrt(np.convolve(y ** 2, np.ones(hop * 2) / (hop * 2), "same"))[::hop]
e = bl.db(env)
d = np.diff(e, prepend=e[0])
peaks, _ = ss.find_peaks(e, prominence=6, distance=int(0.02 * 2000))
top = e[peaks].max() if len(peaks) else 0
peaks = [p for p in peaks if e[p] > top - 20]
t = np.array(peaks) / 2000 + start
print("strikes:", len(t))
print("times:", " ".join(f"{v:.3f}" for v in t))
iv = np.diff(t) * 1000
print("intervals ms:", " ".join(f"{v:.0f}" for v in iv))
print("levels dB:", " ".join(f"{e[p] - top:.0f}" for p in peaks))
if len(t):
    seg = bl.highpass(x[int((t[0] - start) * sr):int((t[-1] - start + 0.03) * sr)], sr, 150)
    mag, freqs, _ = bl.stft(seg, sr, 2048, 256, 8192)
    spec = (mag ** 2).mean(0)
    k = (freqs > 150) & (freqs < 8000)
    pk, _ = ss.find_peaks(10 * np.log10(spec[k] + 1e-20), prominence=4, distance=40)
    order = np.argsort(spec[k][pk])[::-1][:8]
    ref = spec[k][pk][order[0]]
    print("spectral peaks:", " ".join(f"{freqs[k][pk][i]:.0f}Hz({10 * np.log10(spec[k][pk][i] / ref):.0f})" for i in sorted(order, key=lambda i: freqs[k][pk][i])))
