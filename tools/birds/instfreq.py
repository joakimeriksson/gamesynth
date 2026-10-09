"""Instantaneous frequency and envelope of a band-passed stretch (Hilbert), to see fast FM/AM
that a spectrogram smears: prints their rms swing and dominant rates.

    instfreq.py <wav> <start_s> <dur_s> <lo_hz> <hi_hz>
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
import scipy.signal as ss

import birdlib as bl

path, start, dur, lo, hi = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5])
sr, x = bl.load(path, start, dur)
y = bl.bandpass(x, sr, lo, hi, order=6)
a = ss.hilbert(y)
env = np.abs(a)
ph = np.unwrap(np.angle(a))
f = np.diff(ph) * sr / (2 * np.pi)
# Smooth to 1 kHz bandwidth and keep loud samples.
b, aa = ss.butter(2, 1000, fs=sr)
f = ss.filtfilt(b, aa, f)
e = env[1:]
keep = e > np.percentile(e, 30)
cents = 1200 * np.log2(np.maximum(f, 50) / np.median(f[keep]))
slow = ss.savgol_filter(cents, int(0.041 * sr) | 1, 3)
res = (cents - slow)[keep]
envdb = bl.db(e)
eres = (envdb - ss.savgol_filter(envdb, int(0.041 * sr) | 1, 3))[keep]


def rate(v):
    spec = np.abs(np.fft.rfft((v - v.mean()) * np.hanning(len(v)))) ** 2
    fr = np.fft.rfftfreq(len(v), 1 / sr)
    band = (fr > 10) & (fr < 400)
    return fr[band][np.argmax(spec[band])]


print(f"median {np.median(f[keep]):.0f} Hz; FM rms {np.std(res):.0f} cents at {rate(cents - slow):.0f} Hz; AM rms {np.std(eres):.1f} dB at {rate(envdb - ss.savgol_filter(envdb, int(0.041 * sr) | 1, 3)):.0f} Hz")
