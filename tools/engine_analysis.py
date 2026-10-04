#!/usr/bin/env python3
"""Order analysis of an engine recording segment.

    tools/engine_analysis.py recording.wav <t0> <t1> <lowest cycle Hz> <highest cycle Hz> [cylinders]

A 4-stroke repeats every 720 deg = one 'cycle'. Find the cycle frequency fc (the comb of
harmonics k*fc that best explains the spectrum), then report level per harmonic. For a V8:
firing = 8*fc, crank rotation = 2*fc. Harmonics that are not multiples of 8 are the 'rumble'.
Also prints third-octave band levels (dB re the loudest band)."""
import sys, wave, numpy as np

def load(path):
    w = wave.open(path); sr = w.getframerate(); ch = w.getnchannels()
    x = np.frombuffer(w.readframes(w.getnframes()), dtype='<i2').astype(float) / 32767
    return x.reshape(-1, ch).mean(axis=1), sr

def spectrum(x, sr, nfft=1 << 17):
    x = x - x.mean(); w = np.hanning(len(x))
    X = np.abs(np.fft.rfft(x * w, nfft)) / (w.sum() / 2)
    return np.fft.rfftfreq(nfft, 1 / sr), X

def cycle_freq(f, X, lo, hi, cyl=8):
    """Best fc in [lo, hi] Hz: maximise energy on the comb k*fc (k=1..6*cyl)."""
    best = (0, lo)
    df = f[1] - f[0]
    for fc in np.arange(lo, hi, 0.01):
        idx = np.round(np.arange(1, 6 * cyl + 1) * fc / df).astype(int)
        e = np.sum(X[idx] ** 2)
        if e > best[0]: best = (e, fc)
    return best[1]

def peak_at(f, X, hz, tol):
    m = (f > hz - tol) & (f < hz + tol)
    return X[m].max() if m.any() else 0.0

def analyse(path, t0, t1, lo, hi, cyl=8, label=None, fc=None):
    x, sr = load(path); seg = x[int(t0 * sr):int(t1 * sr)]
    f, X = spectrum(seg, sr)
    fc = fc or cycle_freq(f, X, lo, hi, cyl)
    amps = np.array([peak_at(f, X, k * fc, fc * 0.2) for k in range(1, 6 * cyl + 1)])  # 48 harmonics for a V8
    db = 20 * np.log10(amps / amps.max() + 1e-9)
    fire = np.arange(1, 6 * cyl + 1) % cyl == 0
    e_fire, e_rumble = np.sum(amps[fire] ** 2), np.sum(amps[~fire] ** 2)
    print(f"{label or path} [{t0}-{t1}s]: cycle {fc:.2f} Hz = {fc*120:.0f} rpm, firing {fc*cyl:.1f} Hz, rms {np.sqrt(np.mean(seg**2)):.3f}, crest {np.abs(seg).max()/np.sqrt(np.mean(seg**2)):.1f}")
    print("  harmonics of the cycle 1..%d (dB re strongest; * = firing order):" % (3 * cyl))
    print("   " + " ".join(f"{d:4.0f}{'*' if (k+1)%cyl==0 else ' '}" for k, d in enumerate(db[:3 * cyl])))
    print(f"  rumble share (non-firing harmonics / all): {e_rumble/(e_fire+e_rumble):.2f}")
    bands = 1000 * 2.0 ** (np.arange(-15, 13) / 3)
    lv = []
    for c in bands:
        m = (f >= c / 2 ** (1 / 6)) & (f < c * 2 ** (1 / 6)); lv.append(10 * np.log10(np.sum(X[m] ** 2) + 1e-20))
    lv = np.array(lv) - max(lv)
    print("  third-octave dB: " + " ".join(f"{int(c)}:{l:.0f}" for c, l in zip(bands, lv) if c <= 12000))
    return fc, dict(zip(bands.round().astype(int), lv)), db

if __name__ == "__main__":
    a = sys.argv
    analyse(a[1], float(a[2]), float(a[3]), float(a[4]), float(a[5]), int(a[6]) if len(a) > 6 else 8)
