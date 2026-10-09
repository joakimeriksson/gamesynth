"""Find clicks: frames (2 ms) whose energy above 14 kHz is a large share of the frame, and
sample-to-sample jumps far above the local slope. Prints the worst few.

    clicks.py <wav> [channel=0]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
import scipy.io.wavfile as wf
import scipy.signal as ss

sr, x = wf.read(sys.argv[1])
x = x.astype(np.float32) / 32768.0
if x.ndim > 1:
    x = x[:, int(sys.argv[2]) if len(sys.argv) > 2 else 0]
hp = ss.sosfilt(ss.butter(6, 14000, "highpass", fs=sr, output="sos"), x)
n = sr // 500
k = len(x) // n
e = np.sqrt((x[:k * n].reshape(k, n) ** 2).mean(1)) + 1e-9
h = np.sqrt((hp[:k * n].reshape(k, n) ** 2).mean(1)) + 1e-12
share = (h / e) ** 2
loud = e > e.max() * 0.01
idx = np.argsort(np.where(loud, share, 0))[::-1][:8]
print("peak", float(np.abs(x).max()))
for i in sorted(idx):
    print(f"t={i * n / sr:7.3f}s  >14k share {share[i] * 100:6.2f} %  level {20 * np.log10(e[i] / e.max()):6.1f} dB")
d2 = np.abs(np.diff(x, 2))
j = int(np.argmax(d2))
print(f"largest 2nd difference {d2[j]:.4f} at {j / sr:.4f}s (local rms {np.sqrt(np.mean(x[max(0, j - 240):j + 240] ** 2)):.4f})")
