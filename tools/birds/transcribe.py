"""Transcribe a stretch of a bird recording: track the fundamental with a 1 ms hop, segment
syllables, measure each one (duration, pitch contour, amplitude contour, harmonics, noisiness),
and print/save them, with an overlay plot to check the tracking by eye.

    transcribe.py <wav> <start_s> <dur_s> <fmin> <fmax> <out_prefix> [thresh_db=14] [win=512]
                  [min_gap_ms=6] [min_dur_ms=8]

Writes <out_prefix>.json (syllables) and <out_prefix>.png (overlay).
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
import scipy.ndimage as nd

import birdlib as bl

path, start, dur, fmin, fmax, out = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5]), sys.argv[6]
thresh = float(sys.argv[7]) if len(sys.argv) > 7 else 14.0
win = int(sys.argv[8]) if len(sys.argv) > 8 else 512
min_gap = float(sys.argv[9]) / 1000 if len(sys.argv) > 9 else 0.006
min_dur = float(sys.argv[10]) / 1000 if len(sys.argv) > 10 else 0.008

sr, x = bl.load(path, start, dur)
x = bl.bandpass(x, sr, max(fmin * 0.7, 40), min(fmax * 1.3, sr * 0.45))
hop = 48
times, f, peak, rms, mag, freqs = bl.track(x, sr, fmin, fmax, win=win, hop=hop, nfft=4096)
lvl = bl.db(peak)
floor = bl.floor_db(lvl, 30)
top = np.percentile(lvl, 99.5)
th = max(floor + thresh, top - 45)
segs = bl.segment(times, lvl, th, min_dur=min_dur, merge_gap=min_gap)
syls = []
for a, b in segs:
    rows = np.arange(a, b)
    fo = np.log2(f[a:b] / 1000.0)
    fo = nd.median_filter(fo, size=min(5, len(fo) | 1), mode="nearest")
    amp = peak[a:b] / peak[a:b].max()
    t = times[a:b] - times[a]
    d = float(times[b - 1] - times[a] + hop / sr)
    u = t / max(d, 1e-6)
    # Only frames within 20 dB of the syllable's peak carry a reliable pitch.
    good = lvl[a:b] > lvl[a:b].max() - 20
    pk = bl.rdp(np.c_[u[good], fo[good]], 0.02) if good.sum() >= 2 else np.array([0])
    pitch = [(round(float(u[good][i]), 3), round(float(1000 * 2 ** fo[good][i]))) for i in pk]
    ak = bl.rdp(np.c_[u, amp], 0.08)
    ampk = [(round(float(u[i]), 3), round(float(amp[i]), 2)) for i in ak]
    lo_bin, hi_bin = np.searchsorted(freqs, fmin * 0.5), np.searchsorted(freqs, min(12000, sr / 2 - 100))
    loud = rows[good]
    syls.append({
        "t": round(float(times[a] + start), 4),
        "dur": round(d, 4),
        "peak_db": round(float(lvl[a:b].max()), 1),
        "f_start": round(float(1000 * 2 ** fo[good][0])) if good.any() else 0,
        "f_end": round(float(1000 * 2 ** fo[good][-1])) if good.any() else 0,
        "f_min": round(float(1000 * 2 ** fo[good].min())) if good.any() else 0,
        "f_max": round(float(1000 * 2 ** fo[good].max())) if good.any() else 0,
        "f_med": round(float(1000 * 2 ** np.median(fo[good]))) if good.any() else 0,
        "harm": [round(h, 3) for h in bl.harmonic_levels(mag, freqs, f, loud, 4)],
        "flat": round(bl.flatness(mag[loud], lo_bin, hi_bin), 4),
        "pitch": pitch,
        "amp": ampk,
    })

with open(out + ".json", "w") as fh:
    json.dump({"file": os.path.basename(path), "start": start, "dur": dur, "syllables": syls}, fh, indent=0)

# Print a compact table.
prev_end = None
for s in syls:
    gap = "" if prev_end is None else f"{(s['t'] - prev_end) * 1000:6.0f}"
    prev_end = s["t"] + s["dur"]
    print(f"{s['t']:8.3f} {s['dur'] * 1000:6.0f}ms gap{gap:>6} {s['f_start']:5d}->{s['f_end']:5d} [{s['f_min']:5d}-{s['f_max']:5d}] "
          f"{s['peak_db']:6.1f}dB h{s['harm']} fl{s['flat']:.3f} | {s['pitch']}")
print(f"{len(syls)} syllables, floor {floor:.1f} dB, thresh {th:.1f} dB")

# Overlay: spectrogram + tracked pitch of each syllable.
fig, ax = plt.subplots(1, 1, figsize=(max(10, dur * 4), 4.5))
bl.specgram_ax(ax, x, sr, fmax=min(fmax * 2.2, 12000), win=win, hop=96, nfft=2048, dyn=65, t0=start)
for a, b in segs:
    ax.plot(times[a:b] + start, f[a:b] / 1000, ".", ms=1.2, color="cyan")
    ax.axvspan(times[a] + start, times[b - 1] + start, ymin=0, ymax=0.02, color="lime")
ax.set_xlim(start, start + dur)
plt.tight_layout()
plt.savefig(out + ".png", dpi=80)
