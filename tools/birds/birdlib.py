"""Bird-song analysis: fine-grained pitch tracking, syllable segmentation, features, plots.

Used to transcribe CC0 field recordings into the contour tables of
`crates/brusverk-core/src/generators/birdsong/species.rs`. Recordings are untrusted data: they
are only read as WAV samples (scipy.io.wavfile). Run with the refs venv and `-I`.
"""
import numpy as np
import scipy.io.wavfile as wf
import scipy.signal as ss


def load(path, start=0.0, dur=None):
    sr, x = wf.read(path)
    x = x.astype(np.float32)
    if x.ndim > 1:
        x = x.mean(axis=1)
    if np.abs(x).max() > 2.0:
        x /= 32768.0
    a = int(start * sr)
    b = len(x) if dur is None else min(len(x), a + int(dur * sr))
    return sr, x[a:b]


def highpass(x, sr, hz, order=4):
    if hz <= 0:
        return x
    sos = ss.butter(order, hz, "highpass", fs=sr, output="sos")
    return ss.sosfiltfilt(sos, x).astype(np.float32)


def bandpass(x, sr, lo, hi, order=4):
    sos = ss.butter(order, [lo, hi], "bandpass", fs=sr, output="sos")
    return ss.sosfiltfilt(sos, x).astype(np.float32)


def stft(x, sr, win=512, hop=48, nfft=2048):
    """Magnitude STFT: (mag [frames, bins], freqs, times of frame centres)."""
    w = np.hanning(win).astype(np.float32)
    n = 1 + max(0, (len(x) - win) // hop)
    idx = np.arange(win)[None, :] + hop * np.arange(n)[:, None]
    frames = x[idx] * w
    mag = np.abs(np.fft.rfft(frames, nfft, axis=1)) / (w.sum() / 2)
    freqs = np.fft.rfftfreq(nfft, 1 / sr)
    times = (np.arange(n) * hop + win / 2) / sr
    return mag.astype(np.float32), freqs, times


def track(x, sr, fmin, fmax, win=512, hop=48, nfft=4096):
    """Per-frame dominant peak in fmin..fmax with parabolic interpolation.
    Returns times, f (Hz), peak amp (linear), band rms (linear), mag, freqs."""
    mag, freqs, times = stft(x, sr, win, hop, nfft)
    lo, hi = np.searchsorted(freqs, fmin), np.searchsorted(freqs, fmax)
    band = mag[:, lo:hi]
    k = band.argmax(axis=1)
    kk = np.clip(k, 1, band.shape[1] - 2)
    rows = np.arange(len(k))
    a = np.log(band[rows, kk - 1] + 1e-12)
    b = np.log(band[rows, kk] + 1e-12)
    c = np.log(band[rows, kk + 1] + 1e-12)
    denom = a - 2 * b + c
    off = np.where(np.abs(denom) > 1e-9, 0.5 * (a - c) / np.where(np.abs(denom) > 1e-9, denom, 1), 0.0)
    off = np.clip(off, -0.5, 0.5)
    df = freqs[1] - freqs[0]
    f = freqs[lo] + (kk + off) * df
    peak = band[rows, k]
    rms = np.sqrt((band ** 2).sum(axis=1))
    return times, f, peak, rms, mag, freqs


def db(x):
    return 20 * np.log10(np.maximum(x, 1e-9))


def segment(times, level_db, thresh_db, min_dur=0.008, merge_gap=0.006):
    """Runs where level_db > thresh_db (a scalar or per-frame array): (i0, i1) frame ranges."""
    on = level_db > thresh_db
    segs = []
    i = 0
    n = len(on)
    while i < n:
        if not on[i]:
            i += 1
            continue
        j = i
        while j < n and on[j]:
            j += 1
        if segs and times[i] - times[segs[-1][1] - 1] < merge_gap:
            segs[-1] = (segs[-1][0], j)
        else:
            segs.append((i, j))
        i = j
    return [(a, b) for a, b in segs if times[b - 1] - times[a] >= min_dur]


def floor_db(level_db, q=20):
    return float(np.percentile(level_db, q))


def harmonic_levels(mag, freqs, f, rows, nh=4):
    """Mean amplitude ratio of harmonics 2..nh to the fundamental along a track."""
    df = freqs[1] - freqs[0]
    out = []
    for h in range(2, nh + 1):
        vals = []
        for r in rows:
            fh = f[r] * h
            if fh >= freqs[-1] - 200:
                continue
            k = int(round(fh / df))
            k0 = int(round(f[r] / df))
            w = max(2, int(0.02 * fh / df))
            vh = mag[r, max(0, k - w):k + w + 1].max()
            v1 = mag[r, max(0, k0 - 2):k0 + 3].max()
            vals.append(vh / max(v1, 1e-9))
        out.append(float(np.median(vals)) if vals else 0.0)
    return out


def flatness(mag_rows, lo=0, hi=None):
    """Spectral flatness (0 tonal .. 1 noise) of the mean power spectrum of some frames."""
    p = (mag_rows[:, lo:hi] ** 2).mean(axis=0) + 1e-14
    return float(np.exp(np.log(p).mean()) / p.mean())


def rdp(points, eps):
    """Ramer-Douglas-Peucker on (t, y) points, returns the indices kept."""
    pts = np.asarray(points, dtype=float)
    keep = np.zeros(len(pts), bool)
    keep[0] = keep[-1] = True
    stack = [(0, len(pts) - 1)]
    while stack:
        a, b = stack.pop()
        if b <= a + 1:
            continue
        p0, p1 = pts[a], pts[b]
        d = p1 - p0
        L = np.hypot(*d) + 1e-12
        seg = pts[a + 1:b]
        dist = np.abs(d[0] * (seg[:, 1] - p0[1]) - d[1] * (seg[:, 0] - p0[0])) / L
        i = int(dist.argmax())
        if dist[i] > eps:
            m = a + 1 + i
            keep[m] = True
            stack += [(a, m), (m, b)]
    return np.nonzero(keep)[0]


def specgram_ax(ax, x, sr, fmax=10000, win=512, hop=96, nfft=1024, dyn=70, title=None, t0=0.0, fmin=0):
    mag, freqs, times = stft(x, sr, win, hop, nfft)
    k0, k = np.searchsorted(freqs, fmin), np.searchsorted(freqs, fmax)
    S = db(mag[:, k0:k]).T
    top = np.percentile(S, 99.7)
    ax.imshow(S, origin="lower", aspect="auto", cmap="magma", vmin=top - dyn, vmax=top,
              extent=[t0 + times[0], t0 + times[-1], freqs[k0] / 1000, freqs[k - 1] / 1000],
              interpolation="nearest")
    ax.set_ylabel("kHz")
    if title:
        ax.set_title(title, fontsize=9, loc="left")
