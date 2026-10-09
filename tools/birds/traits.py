"""The four "synthetic tells", measured the same way on recordings and on our renders:

* kink:    how concentrated a syllable's turning is. The pitch track (wobble smoothed out over
           41 ms) is differentiated twice; kink = max |curvature| / mean |curvature| over the
           syllable (syllables >= 80 ms). Straight segments joined at corners score high (all
           turning at the corners); smooth curves score ~1.5-3.
* wobble:  fine pitch modulation: rms (cents) of the 1 ms pitch track around its 41 ms
           Savitzky-Golay smoothing, and its rate (Hz, from zero crossings); flutter is the same
           for the level (dB). Only frames within 15 dB of the syllable's peak count.
* harm:    2nd and 3rd harmonic relative to the fundamental (dB, median of loud frames).
* gaps:    silence between consecutive notes inside a song (ms, median), and the share of
           notes that join the next (gap under 10 ms).

    traits.py <ours_dir> [species...]   (writes compare/traits_<tag>.tsv with tag = dir name)

Our renders are measured with white noise added at each species' median recording SNR, so the
tracker's own jitter and noise floor affect both sides alike (CLEAN=1 measures them dry).
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
import scipy.signal as ss

import birdlib as bl
from measure import R, SPECIES

HOP = 48


def snr_of(path, start, dur, fmin, fmax):
    """Level of the loudest syllables above the recording's floor in the band (dB)."""
    sr, x = bl.load(path, start, dur)
    x = bl.bandpass(x, sr, max(fmin * 0.7, 40), min(fmax * 1.3, sr * 0.45))
    win = 256 if fmin >= 1000 else 1024
    _, _, peak, _, _, _ = bl.track(x, sr, fmin, fmax, win=win, hop=HOP, nfft=4096)
    lvl = bl.db(peak)
    return float(np.percentile(lvl, 99.5) - bl.floor_db(lvl, 30))


def syllables(path, start, dur, fmin, fmax, thresh, snr=None):
    sr, x = bl.load(path, start, dur)
    x = bl.bandpass(x, sr, max(fmin * 0.7, 40), min(fmax * 1.3, sr * 0.45))
    if snr is not None:
        # Match a recording's background: white noise whose tracked peak level sits `snr` dB
        # under the loudest syllables, so the tracker sees our renders as it saw the field.
        win = 256 if fmin >= 1000 else 1024
        _, _, pk, _, _, _ = bl.track(x, sr, fmin, fmax, win=win, hop=HOP, nfft=4096)
        top = np.percentile(bl.db(pk), 99.5)
        rng = np.random.default_rng(1)
        n = rng.standard_normal(len(x)).astype(np.float32)
        n = bl.bandpass(n, sr, max(fmin * 0.7, 40), min(fmax * 1.3, sr * 0.45))
        _, _, npk, _, _, _ = bl.track(n, sr, fmin, fmax, win=win, hop=HOP, nfft=4096)
        n *= 10 ** ((top - snr - bl.floor_db(bl.db(npk), 30)) / 20)
        x = x + n
    win = 256 if fmin >= 1000 else 1024
    times, f, peak, _, mag, freqs = bl.track(x, sr, fmin, fmax, win=win, hop=HOP, nfft=4096)
    lvl = bl.db(peak)
    floor = bl.floor_db(lvl, 30)
    top = np.percentile(lvl, 99.5)
    th = max(floor + thresh, top - 40)
    segs = bl.segment(times, lvl, th, min_dur=0.012, merge_gap=0.003)
    return sr, times, f, lvl, mag, freqs, segs


def analyse(path, start, dur, fmin, fmax, thresh, song_gap, snr=None):
    sr, times, f, lvl, mag, freqs, segs = syllables(path, start, dur, fmin, fmax, thresh, snr)
    dt = HOP / sr
    sg = lambda y, ms, order=2: ss.savgol_filter(y, max(order + 2, int(ms / 1000 / dt) | 1), order)
    kinks, wob, rates, flut, h2, h3 = [], [], [], [], [], []
    for a, b in segs:
        good = lvl[a:b] > lvl[a:b].max() - 15
        # The longest run of good frames.
        idx = np.flatnonzero(good)
        if len(idx) < 2:
            continue
        runs = np.split(idx, np.flatnonzero(np.diff(idx) > 1) + 1)
        r = max(runs, key=len) + a
        if len(r) * dt >= 0.045:
            fo = np.log2(f[r])
            smooth = sg(fo, 41, 3)
            res = (fo - smooth) * 1200
            # Sustained parts only (slope under 4 octaves/s): a fast sweep's misfit is not wobble.
            slope = np.abs(np.gradient(smooth, dt))
            res = res[3:-3][slope[3:-3] < 4]
            if len(res) < 10:
                continue
            wob.append(np.sqrt(np.mean(res ** 2)))
            # Rate: the power-weighted median frequency of the residual (5-250 Hz).
            if len(res) >= 32:
                spec = np.abs(np.fft.rfft(res * np.hanning(len(res)), 256)) ** 2
                fr = np.fft.rfftfreq(256, dt)
                band = (fr >= 5) & (fr <= 250)
                cum = np.cumsum(spec[band])
                rates.append(float(fr[band][np.searchsorted(cum, cum[-1] / 2)]))
            lv = lvl[r]
            flut.append(np.sqrt(np.mean((lv - (sg(lv, 41) if len(r) * dt > 0.045 else lv.mean())) ** 2)))
        if len(r) * dt >= 0.08:
            c = sg(np.log2(f[r]), 41)
            curv = np.abs(np.gradient(np.gradient(c, dt), dt))[3:-3]
            if len(curv) > 4 and np.median(curv) > 0:
                kinks.append(curv.max() / max(np.median(curv), 1e-9))
        hl = bl.harmonic_levels(mag, freqs, f, r, 3)
        if hl[0] > 0:
            h2.append(20 * np.log10(hl[0] + 1e-6))
        if hl[1] > 0:
            h3.append(20 * np.log10(hl[1] + 1e-6))
    gaps = []
    for (a0, b0), (a1, b1) in zip(segs, segs[1:]):
        g = times[a1] - times[b0 - 1]
        if g < song_gap:
            gaps.append(g * 1000)
    return kinks, wob, rates, flut, h2, h3, gaps


def main():
    ours = sys.argv[1]
    want = sys.argv[2:] or list(SPECIES)
    clean = os.environ.get("CLEAN") == "1"
    tag = os.path.basename(os.path.normpath(ours))
    med = lambda v: float(np.median(v)) if len(v) else float("nan")
    keys = ["kink", "wobble_c", "wob_hz", "flutter_db", "h2_db", "h3_db", "gap_ms", "joined"]
    if clean:
        tag += "_clean"
    out = []
    print(f"{'species':<14} {'src':<6} " + " ".join(f"{k:>10}" for k in keys))
    for sp in want:
        fmin, fmax, _, thresh, _, _, song_gap, refs = SPECIES[sp]
        snr = float(np.median([snr_of(os.path.join(R, sp, f), s, d, fmin, fmax) for f, s, d in refs]))
        for label, files in (("real", [(os.path.join(R, sp, f), s, d) for f, s, d in refs]),
                             (tag, [(os.path.join(ours, f"{sp}{x}.wav"), 0, 60) for x in ("", "_2")])):
            acc = [[] for _ in range(7)]
            for path, s, d in files:
                for i, v in enumerate(analyse(path, s, d, fmin, fmax, thresh, song_gap, None if label == "real" or clean else snr)):
                    acc[i] += list(v)
            kinks, wob, rates, flut, h2, h3, gaps = acc
            row = [med(kinks), med(wob), med(rates), med(flut), med(h2), med(h3), med(gaps),
                   float(np.mean(np.array(gaps) < 10)) if gaps else float("nan")]
            out.append((sp, label, row))
            print(f"{sp:<14} {label:<6} " + " ".join(f"{v:10.2f}" for v in row))
    path = f"/Users/joakimeriksson/work/gamesynth/target/refs/birds/compare/traits_{tag}.tsv"
    with open(path, "w") as fh:
        fh.write("species\tsource\t" + "\t".join(keys) + "\n")
        for sp, label, row in out:
            fh.write(f"{sp}\t{label}\t" + "\t".join(f"{v:.3f}" for v in row) + "\n")
    print(path)


if __name__ == "__main__":
    main()
