"""Look at sounds several ways at once, one row per file, so real recordings and renders line up:

  1. waveform (300 ms)            what one or two events look like
  2. envelope (3 s, 5 ms RMS)     the rhythm you hear: beats, swells, gaps
  3. envelope rhythm spectrum     how solid a beat is: a sharp tall peak = a steady pulse at that rate,
     (1-80 Hz, per band)          a smear = random; per band (low / mid / high) shows where the beat lives
  4. envelope autocorrelation     periodicity: height of the first peak (1 = every beat identical)
  5. period-folded envelope       the shape of one beat (mean) and how much passes differ (band)
  6. spectrogram (2 s, 0-4 kHz)   events in time and frequency
  7. long-term spectrum (log f)   the overall colour

and prints per file: beat rate, periodicity, beat depth, pass-to-pass spread, and per-band beat strength.

  python tools/sound_views.py out.png "label" file.wav[@start] ["label" file.wav[@start] ...] [--rate HZ] [--secs N]

`@start` picks the excerpt (seconds). --rate forces the beat rate for the folding (else found per file
between 6 and 40 Hz). Needs numpy, scipy, matplotlib (e.g. target/refs/.venv).
"""
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from scipy import signal
from scipy.io import wavfile

SR = 48000
BANDS = [("low <300", 20, 300), ("mid 300-2k", 300, 2000), ("high >2k", 2000, 12000)]


def load(spec, secs):
    path, _, start = spec.partition("@")
    sr, x = wavfile.read(path)
    x = x.astype(np.float64)
    if x.ndim > 1:
        x = x.mean(axis=1)
    x /= np.abs(x).max() + 1e-12
    if sr != SR:
        x = signal.resample_poly(x, SR, sr)
    s = int(float(start or 0) * SR)
    x = x[s : s + int(secs * SR)]
    sos = signal.butter(2, 20, "highpass", fs=SR, output="sos")
    return signal.sosfiltfilt(sos, x)


def envelope(x, ms=5.0):
    hop = int(SR * ms / 1000)
    n = len(x) // hop
    return np.sqrt((x[: n * hop].reshape(n, hop) ** 2).mean(axis=1) + 1e-12), 1000.0 / ms


def band(x, lo, hi):
    sos = signal.butter(4, [lo, min(hi, SR / 2 - 100)], "bandpass", fs=SR, output="sos")
    return signal.sosfiltfilt(sos, x)


def rhythm_spectrum(env, fs):
    e = env / env.mean() - 1.0
    f, p = signal.welch(e, fs=fs, nperseg=min(len(e), int(fs * 4)), detrend="linear")
    return f, 10 * np.log10(p + 1e-12)


def beat_rate(env, fs, lo=6.0, hi=40.0):
    f, p = rhythm_spectrum(env, fs)
    m = (f >= lo) & (f <= hi)
    return f[m][np.argmax(p[m])]


def autocorr(env, fs, max_s=0.3):
    e = env - env.mean()
    ac = signal.correlate(e, e, "full")[len(e) - 1 :]
    ac /= ac[0]
    n = int(max_s * fs)
    return np.arange(n) / fs, ac[:n]


def fold(env, fs, rate):
    per = fs / rate
    k = int(len(env) // per)
    grid = np.linspace(0, 1, 64, endpoint=False)
    passes = np.array([np.interp(i * per + grid * per, np.arange(len(env)), env) for i in range(k)])
    db = 20 * np.log10(passes / passes.mean())
    return grid, db


def main():
    args = sys.argv[1:]
    out = args.pop(0)
    rate_forced, secs = None, 3.0
    if "--rate" in args:
        i = args.index("--rate")
        rate_forced = float(args[i + 1])
        del args[i : i + 2]
    if "--secs" in args:
        i = args.index("--secs")
        secs = float(args[i + 1])
        del args[i : i + 2]
    items = list(zip(args[0::2], args[1::2]))
    fig, axes = plt.subplots(len(items), 7, figsize=(30, 2.6 * len(items)), squeeze=False)
    titles = ["waveform 300 ms", "envelope 3 s (dB)", "rhythm spectrum (dB)", "envelope autocorr",
              "one beat (dB, mean ±1σ)", "spectrogram 0-4 kHz", "long-term spectrum"]
    print(f"{'file':<28} {'beat Hz':>7} {'period.':>7} {'depth dB':>8} {'spread dB':>9}   beat strength low/mid/high (dB over floor)")
    for r, (label, spec) in enumerate(items):
        x = load(spec, secs)
        env, fs = envelope(x)
        rate = rate_forced or beat_rate(env, fs)
        lag, ac = autocorr(env, fs)
        per_i = int(round(fs / rate))
        periodicity = ac[max(1, per_i - 3) : per_i + 4].max() if per_i + 4 < len(ac) else float("nan")
        grid, db = fold(env, fs, rate)
        mean = db.mean(axis=0)
        depth = mean.max() - mean.min()
        spread = db.std(axis=0).mean()
        strengths = []
        ax = axes[r]
        t = np.arange(int(0.3 * SR)) / SR
        ax[0].plot(t * 1000, x[: len(t)], lw=0.5)
        ax[0].set_ylabel(label, fontsize=9)
        te = np.arange(len(env)) / fs
        ax[1].plot(te, 20 * np.log10(env / env.max()), lw=0.6)
        ax[1].set_ylim(-40, 1)
        for name, lo, hi in BANDS:
            be, _ = envelope(band(x, lo, hi))
            f, p = rhythm_spectrum(be, fs)
            m = (f > 1) & (f < 80)
            ax[2].plot(f[m], p[m], lw=0.8, label=name)
            near = (f > rate - 1.5) & (f < rate + 1.5)
            floor = np.median(p[(f > 3) & (f < 80)])
            strengths.append(p[near].max() - floor)
        ax[2].axvline(rate, color="k", lw=0.5, ls=":")
        if r == 0:
            ax[2].legend(fontsize=7)
        ax[3].plot(lag * 1000, ac, lw=0.8)
        ax[3].axvline(1000 / rate, color="k", lw=0.5, ls=":")
        ax[3].set_ylim(-0.5, 1)
        ax[4].fill_between(grid, mean - db.std(axis=0), mean + db.std(axis=0), alpha=0.3)
        ax[4].plot(grid, mean, lw=1.2)
        ax[4].set_ylim(-12, 12)
        seg = x[: 2 * SR]
        f, tt, S = signal.spectrogram(seg, fs=SR, nperseg=1024, noverlap=1024 - 96)
        m = f <= 4000
        ax[5].pcolormesh(tt, f[m], 10 * np.log10(S[m] + 1e-14), shading="auto", cmap="magma",
                         vmin=10 * np.log10(S[m].max()) - 70)
        f, p = signal.welch(x, fs=SR, nperseg=8192)
        m = (f > 20) & (f < 12000)
        ax[6].semilogx(f[m], 10 * np.log10(p[m] / p[m].max()), lw=0.6)
        ax[6].set_ylim(-80, 2)
        print(f"{label[:28]:<28} {rate:7.1f} {periodicity:7.2f} {depth:8.1f} {spread:9.1f}   "
              + " / ".join(f"{s:.0f}" for s in strengths))
    for c, t in enumerate(titles):
        axes[0][c].set_title(t, fontsize=10)
    fig.tight_layout()
    fig.savefig(out, dpi=70)
    print(out)


if __name__ == "__main__":
    main()
