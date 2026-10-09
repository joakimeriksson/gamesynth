"""Fine FM of sustained notes: the 0.5 ms pitch track (short windows) minus its 41 ms smoothing,
over frames within 12 dB of the note's peak: rms and 5-95 % swing in cents, dominant rate.
Also the level's flutter (dB rms).

    fm.py <label>=<wav>@<start>:<dur>:<lo_hz>:<hi_hz>:<win> [...]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
import scipy.signal as ss

import birdlib as bl

for arg in sys.argv[1:]:
    label, rest = arg.split("=", 1)
    path, spec = rest.rsplit("@", 1)
    start, dur, lo, hi, win = spec.split(":")
    start, dur, lo, hi, win = float(start), float(dur), float(lo), float(hi), int(win)
    sr, x = bl.load(path, start, dur)
    x = bl.bandpass(x, sr, lo * 0.8, hi * 1.2)
    hop = 24
    t, f, pk, _, _, _ = bl.track(x, sr, lo, hi, win=win, hop=hop, nfft=8192)
    lv = bl.db(pk)
    keep = lv > lv.max() - 12
    c = 1200 * np.log2(f)
    n = int(float(os.environ.get("SMOOTH_MS", "41")) / 1000 * sr / hop) | 1
    sm = ss.savgol_filter(c, n, 3)
    r = (c - sm)[keep]
    sp = np.abs(np.fft.rfft((c - sm) * np.hanning(len(c)), 4096)) ** 2
    fr = np.fft.rfftfreq(4096, hop / sr)
    b = (fr > 15) & (fr < 300)
    fl = (lv - ss.savgol_filter(lv, n, 3))[keep]
    print(f"{label:<14} {np.median(f[keep]):6.0f} Hz  FM rms {np.std(r):5.0f} c  swing {np.percentile(r, 95) - np.percentile(r, 5):5.0f} c  "
          f"rate {fr[b][np.argmax(sp[b])]:4.0f} Hz  flutter {np.std(fl):4.1f} dB")
