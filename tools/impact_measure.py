#!/usr/bin/env python3
"""Measure single hits (material impacts) in recordings or renders, the same way for both.

    python tools/impact_measure.py [--max N] [--json out.json] file.wav[@start[:end]] ...

Per file it finds hits (the 2 ms envelope jumping 18 dB above its running median, at least
120 ms apart) and measures each one:

  pk      peak of the hit in dB (file normalised to its loudest sample = 0 dB)
  t20/t40 ms for the 5 ms RMS envelope to fall 20 / 40 dB below the hit's peak ('>' = cut short)
  c20     spectral centroid of the first 20 ms (how bright the attack is), Hz
  cring   centroid of the ring, 20..200 ms, Hz
  noise   attack noise length: ms until 8 spectral peaks hold over half the energy (20 ms frames,
          2 ms hop). A clean ring is near 0; a stone thud may never get there (inf)
  tonal   that peak share averaged over 30..300 ms: 1 = a few modes, low = noise
  modes   the strongest peaks of the ring (high-resolution FFT from 5 ms to at most 1 s) as
          Hz(level dB, T60 ms), T60 from a line fit to the band-passed dB envelope

and, with three or more hits, how the attack centroid moves with hit strength.
Needs numpy and scipy. Files are mixed to mono and high-passed at 30 Hz.
"""
import json
import sys

import numpy as np
from scipy import signal
from scipy.io import wavfile

SR = 48000
PAD = 0.4


def load(spec):
    path, _, rng = spec.partition("@")
    sr, x = wavfile.read(path)
    x = x.astype(np.float64)
    if x.ndim > 1:
        x = x.mean(axis=1)
    if sr != SR:
        x = signal.resample_poly(x, SR, sr)
    if rng:
        a, _, b = rng.partition(":")
        x = x[int(float(a) * SR): int(float(b) * SR) if b else None]
    x /= np.abs(x).max() + 1e-12
    sos = signal.butter(2, 30, "highpass", fs=SR, output="sos")
    # Silence around the file, so a hit at the very start or a short file still has a floor.
    pad = np.zeros(int(PAD * SR))
    return path, np.concatenate([pad, signal.sosfilt(sos, x), pad])


def env_db(x, ms):
    hop = int(SR * ms / 1000)
    n = len(x) // hop
    e = np.sqrt((x[: n * hop].reshape(n, hop) ** 2).mean(axis=1) + 1e-14)
    return 20 * np.log10(e), hop


def onsets(x, min_gap_ms=120.0, rel_db=45.0):
    e, hop = env_db(x, 2.0)
    # The floor is the median of the 300 ms before each frame, so a long ring does not hide its own onset.
    floor = np.concatenate([np.full(76, e.min()), signal.medfilt(e, 151)[:-76]])
    found, last = [], -1e9
    peak = e.max()
    for i in range(1, len(e)):
        if e[i] - floor[i] > 18 and e[i] > peak - rel_db and e[i] - e[i - 1] > 6 and (i - last) * 2.0 > min_gap_ms:
            j = i
            while j > 0 and e[j - 1] < e[j] - 3:
                j -= 1
            found.append(j * hop)
            last = i
    return found


def centroid(seg):
    if len(seg) < 64:
        return 0.0
    p = np.abs(np.fft.rfft(seg * np.hanning(len(seg)), 1 << 14)) ** 2
    f = np.fft.rfftfreq(1 << 14, 1 / SR)
    return float((p * f).sum() / (p.sum() + 1e-20))


def ring_spectrum(seg):
    nfft = 1 << 18
    p = np.abs(np.fft.rfft(seg * np.hanning(len(seg)), nfft)) ** 2
    db = 10 * np.log10(p + 1e-20)
    return np.fft.rfftfreq(nfft, 1 / SR), db - db.max()


def mode_peaks(seg, n=6, fmin=60.0):
    f, db = ring_spectrum(seg)
    pk, _ = signal.find_peaks(db, prominence=10, distance=max(1, int(20 / (f[1]))))
    pk = pk[(f[pk] > fmin) & (f[pk] < 16000)]
    pk = pk[np.argsort(db[pk])[::-1]]
    chosen = []
    for k in pk:
        if all(abs(f[k] / f[c] - 1) > 0.03 for c in chosen):
            chosen.append(k)
        if len(chosen) >= n:
            break
    return [(float(f[k]), float(db[k])) for k in chosen]


def mode_t60(seg, hz):
    bw = max(15.0, hz * 0.02)
    sos = signal.butter(2, [max(20.0, hz - bw), min(SR / 2 - 100, hz + bw)], "bandpass", fs=SR, output="sos")
    e, hop = env_db(signal.sosfilt(sos, seg), 5.0)
    i0 = int(np.argmax(e[: max(2, len(e) // 4)]))
    tail = e[i0:] - e[i0]
    end = int(np.argmax(tail < -33)) if np.any(tail < -33) else len(tail)
    idx = np.where(tail[:end] < -3)[0]
    if len(idx) < 3:
        return float("nan")
    slope = np.polyfit(idx * hop / SR, tail[idx], 1)[0]
    return float(-60.0 / slope) if slope < 0 else float("inf")


def tonal_share(seg):
    hop, n = int(0.002 * SR), int(0.02 * SR)
    out = []
    win = np.hanning(n)
    for s in range(0, len(seg) - n, hop):
        p = np.abs(np.fft.rfft(seg[s: s + n] * win, 2048)) ** 2
        pk, _ = signal.find_peaks(p)
        best = pk[np.argsort(p[pk])[::-1][:8]] if len(pk) else []
        share = sum(p[max(0, k - 1): k + 2].sum() for k in best)
        out.append(share / (p.sum() + 1e-20))
    return np.array(out)


def measure_hit(x, start, end):
    seg = x[start:end]
    e, hop = env_db(seg, 5.0)
    i0 = int(np.argmax(e[:10]))
    top = e[i0]

    def fall(db):
        k = np.where(e[i0:] < top - db)[0]
        return (k[0] * hop / SR * 1000, False) if len(k) else (len(e[i0:]) * hop / SR * 1000, True)

    t20, c20cut = fall(20)
    t40, c40cut = fall(40)
    ts = tonal_share(seg[: int(min(len(seg), 0.4 * SR))])
    above = np.where(ts > 0.5)[0]
    ring = seg[int(0.005 * SR): int(min(len(seg), 1.0 * SR))]
    modes = []
    if len(ring) > int(0.03 * SR):
        modes = [(hz, db, mode_t60(seg, hz)) for hz, db in mode_peaks(ring)]
    return dict(
        pk=float(20 * np.log10(np.abs(seg[: int(0.05 * SR)]).max() + 1e-12)),
        t20=t20, t20cut=c20cut, t40=t40, t40cut=c40cut,
        c20=centroid(seg[: int(0.02 * SR)]), cring=centroid(seg[int(0.02 * SR): int(0.2 * SR)]),
        noise=float(above[0] * 2.0) if len(above) else float("inf"),
        tonal=float(ts[15:150].mean()) if len(ts) > 20 else float(ts.mean()) if len(ts) else 0.0,
        modes=modes, start=start / SR - PAD,
    )


def hits(x, mx=12):
    on = onsets(x)[:mx]
    out = []
    for k, s in enumerate(on):
        e = on[k + 1] - int(0.005 * SR) if k + 1 < len(on) else min(len(x), s + int(3 * SR))
        out.append(measure_hit(x, s, e))
    return out


def main():
    args = sys.argv[1:]
    mx, js = 12, None
    if "--max" in args:
        i = args.index("--max")
        mx = int(args[i + 1])
        del args[i: i + 2]
    if "--json" in args:
        i = args.index("--json")
        js = args[i + 1]
        del args[i: i + 2]
    allres = {}
    for spec in args:
        _, x = load(spec)
        res = hits(x, mx)
        allres[spec] = res
        print(f"## {spec.split('/')[-1]}  ({len(res)} hits)")
        for r in res:
            m = "  ".join(f"{hz:.0f}({db:.0f},{t60 * 1000:.0f})" if np.isfinite(t60) else f"{hz:.0f}({db:.0f},-)" for hz, db, t60 in r["modes"])
            print(f"  @{r['start']:6.2f}s pk {r['pk']:5.1f} t20 {'>' if r['t20cut'] else ''}{r['t20']:4.0f} t40 {'>' if r['t40cut'] else ''}{r['t40']:5.0f}"
                  f" c20 {r['c20']:5.0f} cring {r['cring']:5.0f} noise {r['noise']:4.0f} tonal {r['tonal']:.2f} | {m}")
        if len(res) >= 3:
            pk = np.array([r["pk"] for r in res])
            c = np.array([r["c20"] for r in res])
            if pk.std() > 1:
                slope = np.polyfit(pk, np.log2(c + 1), 1)[0]
                print(f"  strength vs brightness: {slope * 20:+.2f} octaves of attack centroid per +20 dB (r={np.corrcoef(pk, c)[0, 1]:.2f})")
    if js:
        with open(js, "w") as f:
            json.dump(allres, f, indent=1, default=float)


if __name__ == "__main__":
    main()
