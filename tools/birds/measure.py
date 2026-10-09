"""Syllable-level match, real against ours, per species: the same segmentation on both, then
pitch range (10-90 % of syllable median pitch), the median start->end glide (contour shape, in
semitones), syllable duration, onset interval inside songs, syllables per second, song length
and the pause between songs.

    measure.py <ours_dir> [species...]

Prints a table and writes target/refs/birds/compare/measurements.tsv.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
import scipy.ndimage as nd

import birdlib as bl

R = "/Users/joakimeriksson/work/gamesynth/target/refs/decoded/birds"
OUT = "/Users/joakimeriksson/work/gamesynth/target/refs/birds/compare/measurements.tsv"

# species: (fmin, fmax, win, thresh_db, merge_gap_s, min_dur_s, song_gap_s, [(file, start, dur)])
SPECIES = {
    "blackbird": (1200, 8000, 384, 18, 0.008, 0.02, 0.8, [("811988_richwise.wav", 0, 59), ("725332_Sacha.Julien.wav", 0, 60)]),
    "robin": (1800, 9500, 256, 18, 0.006, 0.015, 0.6, [("671058_Andrew.soundscape.wav", 0, 46), ("725331_Sacha.Julien.wav", 0, 60)]),
    "chaffinch": (1800, 9000, 256, 18, 0.004, 0.006, 0.6, [("332851_Travesia_Sonora.wav", 0, 61), ("725328_Sacha.Julien.wav", 0, 78)]),
    "great_tit": (2500, 9000, 384, 18, 0.01, 0.025, 0.6, [("418102_straget.wav", 0, 12), ("389404_BeeProductive.wav", 0, 91), ("346066_pulswelle.wav", 0, 90)]),
    "cuckoo": (350, 1000, 2048, 14, 0.03, 0.04, 0.6, [("688844_Elanor1995.wav", 0, 77), ("377213_jaromarsalek.wav", 0, 10.7)]),
    "wood_pigeon": (250, 800, 2048, 12, 0.04, 0.06, 0.35, [("814890_Sadiquecat.wav", 0, 64)]),
    "collared_dove": (350, 900, 2048, 14, 0.03, 0.05, 0.6, [("610563_randomthoughts7.wav", 0, 6.4), ("516091_Karola3206.wav", 0, 13)]),
    "crow": (250, 3000, 1024, 14, 0.03, 0.08, 1.0, [("181088_cfrooos.wav", 0, 22), ("741366_Mish7913.wav", 0, 4.8), ("476139_cupido-1.wav", 0, 58)]),
    "tawny_owl": (500, 1300, 2048, 14, 0.06, 0.04, 2.5, [("745208_Patrick_Corra.wav", 0, 38), ("450214_Marcuspepsi.wav", 0, 8.8), ("688869_Elanor1995.wav", 0, 22)]),
    "house_sparrow": (2000, 9000, 256, 16, 0.01, 0.03, 0.4, [("404663_straget.wav", 0, 43), ("383160_BeeProductive.wav", 0, 62)]),
    "herring_gull": (600, 2500, 1024, 14, 0.02, 0.05, 0.6, [("538016_Canardo55.wav", 0, 33), ("538015_Canardo55.wav", 0, 21)]),
    "nightingale": (1000, 9500, 256, 18, 0.006, 0.008, 0.8, [("521035_smand.wav", 0, 122), ("395101_Uocca.wav", 0, 121)]),
    "skylark": (1800, 9000, 256, 16, 0.006, 0.006, 0.6, [("387426_Kinoton.wav", 0.6, 74)]),
}


def analyse(path, start, dur, fmin, fmax, win, thresh, merge, mindur):
    sr, x = bl.load(path, start, dur)
    x = bl.bandpass(x, sr, max(fmin * 0.7, 40), min(fmax * 1.3, sr * 0.45))
    times, f, peak, _, _, _ = bl.track(x, sr, fmin, fmax, win=win, hop=96, nfft=4096)
    lvl = bl.db(peak)
    floor = bl.floor_db(lvl, 30)
    top = np.percentile(lvl, 99.5)
    th = max(floor + thresh, top - 40)
    syls = []
    for a, b in bl.segment(times, lvl, th, min_dur=mindur, merge_gap=merge):
        good = lvl[a:b] > lvl[a:b].max() - 20
        fo = nd.median_filter(np.log2(f[a:b][good]), size=min(5, int(good.sum()) | 1), mode="nearest") if good.sum() else np.array([0.0])
        syls.append((times[a], times[b - 1] - times[a] + 0.002, 2 ** np.median(fo), 12 * (fo[-1] - fo[0])))
    return syls


def stats(groups, song_gap):
    """groups: per file, the syllable list."""
    pitch, durs, glide, ioi, lens, pauses, rates = [], [], [], [], [], [], []
    for syls in groups:
        if not syls:
            continue
        pitch += [s[2] for s in syls]
        durs += [s[1] for s in syls]
        glide += [s[3] for s in syls]
        songs, cur = [], [syls[0]]
        for a, b in zip(syls, syls[1:]):
            if b[0] - (a[0] + a[1]) > song_gap:
                songs.append(cur)
                cur = []
            cur.append(b)
        songs.append(cur)
        for i, s in enumerate(songs):
            L = s[-1][0] + s[-1][1] - s[0][0]
            if len(s) >= 2:
                lens.append(L)
                ioi += [b[0] - a[0] for a, b in zip(s, s[1:])]
                rates.append(len(s) / max(L, 0.05))
            if i + 1 < len(songs):
                pauses.append(songs[i + 1][0][0] - (s[-1][0] + s[-1][1]))
    med = lambda v: float(np.median(v)) if v else float("nan")
    return {
        "n": len(pitch),
        "pitch_lo": float(np.percentile(pitch, 10)) if pitch else 0,
        "pitch_hi": float(np.percentile(pitch, 90)) if pitch else 0,
        "glide_st": med(glide),
        "dur_ms": med(durs) * 1000,
        "ioi_ms": med(ioi) * 1000,
        "rate": med(rates),
        "song_s": med(lens),
        "pause_s": med(pauses),
    }


def main():
    ours = sys.argv[1]
    want = sys.argv[2:] or list(SPECIES)
    keys = ["n", "pitch_lo", "pitch_hi", "glide_st", "dur_ms", "ioi_ms", "rate", "song_s", "pause_s"]
    rows = []
    print(f"{'species':<14} {'src':<5} " + " ".join(f"{k:>8}" for k in keys))
    for sp in want:
        fmin, fmax, win, thresh, merge, mindur, song_gap, refs = SPECIES[sp]
        real = [analyse(os.path.join(R, sp, f), s, d, fmin, fmax, win, thresh, merge, mindur) for f, s, d in refs]
        mine = [analyse(os.path.join(ours, f"{sp}{suffix}.wav"), 0, 60, fmin, fmax, win, thresh, merge, mindur) for suffix in ("", "_2")]
        for label, g in (("real", real), ("ours", mine)):
            st = stats(g, song_gap)
            rows.append((sp, label, st))
            print(f"{sp:<14} {label:<5} " + " ".join(f"{st[k]:8.0f}" if k in ("n", "pitch_lo", "pitch_hi", "dur_ms", "ioi_ms") else f"{st[k]:8.2f}" for k in keys))
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as fh:
        fh.write("species\tsource\t" + "\t".join(keys) + "\n")
        for sp, label, st in rows:
            fh.write(f"{sp}\t{label}\t" + "\t".join(f"{st[k]:.3f}" for k in keys) + "\n")
    print(OUT)


if __name__ == "__main__":
    main()
