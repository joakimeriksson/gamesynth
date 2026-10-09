"""Cluster transcribed syllables into a repertoire and measure the song grammar.

    grammar.py <song_gap_s> <n_clusters|cut> <a.json> [b.json ...]

Songs are runs of syllables separated by less than song_gap_s. Syllables are clustered
(Ward, on log duration, median pitch, start/end pitch and pitch range in octaves). Prints each
cluster's size, typical numbers and medoid contour, every song as a label string, repeat runs,
song lengths, pauses and intervals, with their spread.
"""
import json
import string
import sys

import numpy as np
from scipy.cluster.hierarchy import fcluster, linkage

gap_s = float(sys.argv[1])
ncl = float(sys.argv[2])
syls = []
for p in sys.argv[3:]:
    d = json.load(open(p))
    for s in d["syllables"]:
        s["src"] = p
        syls.append(s)
syls.sort(key=lambda s: (s["src"], s["t"]))

oct_ = lambda hz: np.log2(max(hz, 1) / 1000)
feat = np.array([[np.log2(s["dur"]) * 0.6, oct_(s["f_med"]) * 2, oct_(s["f_start"]), oct_(s["f_end"]),
                  (oct_(s["f_max"]) - oct_(s["f_min"]))] for s in syls])
if len(syls) > 1:
    Z = linkage(feat, "ward")
    lab = fcluster(Z, int(ncl), "maxclust") if ncl >= 1 else fcluster(Z, -ncl, "distance")
else:
    lab = np.array([1])
names = string.ascii_uppercase + string.ascii_lowercase
for s, l in zip(syls, lab):
    s["lab"] = names[(l - 1) % len(names)]

print("== repertoire")
for l in sorted(set(lab)):
    idx = [i for i in range(len(syls)) if lab[i] == l]
    F = feat[idx]
    med = idx[int(np.argmin(((F - F.mean(0)) ** 2).sum(1)))]
    g = [syls[i] for i in idx]
    q = lambda k: (np.median([s[k] for s in g]), np.std([s[k] for s in g]))
    dur, f0, f1, fl = q("dur"), q("f_start"), q("f_end"), q("flat")
    h = np.median([s["harm"] for s in g], axis=0)
    print(f"{names[(l - 1) % 52]}: n={len(g):3d} dur {dur[0] * 1000:5.0f}±{dur[1] * 1000:3.0f}ms  "
          f"{f0[0]:5.0f}±{f0[1]:4.0f} -> {f1[0]:5.0f}±{f1[1]:4.0f} Hz  med {np.median([s['f_med'] for s in g]):5.0f}  "
          f"h{np.round(h, 3).tolist()} flat {fl[0]:.3f}  db {np.median([s['peak_db'] for s in g]):.1f}")
    print(f"     medoid t={syls[med]['t']} {syls[med]['src'].split('/')[-1]} pitch {syls[med]['pitch']}")
    print(f"     amp {syls[med]['amp']}")

print("== songs")
songs = []
cur = [syls[0]]
for a, b in zip(syls, syls[1:]):
    if b["src"] != a["src"] or b["t"] - (a["t"] + a["dur"]) > gap_s:
        songs.append(cur)
        cur = []
    cur.append(b)
songs.append(cur)
lens, pauses, ns, ioi = [], [], [], []
for i, s in enumerate(songs):
    L = s[-1]["t"] + s[-1]["dur"] - s[0]["t"]
    lens.append(L)
    ns.append(len(s))
    ioi += [b["t"] - a["t"] for a, b in zip(s, s[1:])]
    lab_s = "".join(x["lab"] for x in s)
    nxt = songs[i + 1][0] if i + 1 < len(songs) else None
    p = nxt["t"] - (s[-1]["t"] + s[-1]["dur"]) if nxt and nxt["src"] == s[0]["src"] else None
    if p is not None:
        pauses.append(p)
    print(f"{s[0]['t']:8.2f} {L:5.2f}s n={len(s):3d} pause {'' if p is None else f'{p:5.2f}'}  {lab_s}")
st = lambda v: f"median {np.median(v):.3f} mean {np.mean(v):.3f} sd {np.std(v):.3f} min {np.min(v):.3f} max {np.max(v):.3f}" if len(v) else "-"
print("song length:", st(lens))
print("syllables/song:", st(ns))
print("pause:", st(pauses))
print("onset interval in song:", st(ioi))
