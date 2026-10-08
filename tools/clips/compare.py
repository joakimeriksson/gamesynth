"""Before/after contact sheet: the same moments from two cuts, wide and square side by side.
  python3 tools/clips/compare.py <old_wide> <new_wide> <old_square> <new_square> <out.png> [t1 t2 ...]
Run from the repo root (fonts in target/clip-fonts, fetched by compose.py).
"""
import os
import subprocess
import sys

MONO = "target/clip-fonts/IBMPlexMono-Medium.ttf"
DISP = "target/clip-fonts/SairaCondensed-Bold.ttf"
ow, nw, osq, nsq, out = sys.argv[1:6]
# Default moments for v8_game: first frame, cut to panel, value change, limiter, cut back, typed
# caption, logo pop, end card.
times = [float(t) for t in sys.argv[6:]] or [0.0, 3.52, 8.33, 12.0, 16.98, 18.6, 22.55, 24.8]
work = os.path.join("target", "clips_work", "compare")
os.makedirs(work, exist_ok=True)
CW, CH, SQ, G, LBL, HEAD = 512, 288, 288, 8, 92, 44
cells = []
for i, t in enumerate(times):
    for j, (v, w, h) in enumerate([(ow, CW, CH), (nw, CW, CH), (osq, SQ, SQ), (nsq, SQ, SQ)]):
        p = os.path.join(work, f"c{i}_{j}.png")
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", f"{t}", "-i", v, "-frames:v", "1",
                        "-vf", f"scale={w}:{h}", p], check=True)
        cells.append(p)
cols = [LBL, LBL + CW + G, LBL + 2 * (CW + G), LBL + 2 * (CW + G) + SQ + G]
Wt = cols[3] + SQ + G
Ht = HEAD + len(times) * (CH + G)
args, f = [], [f"color=c=0x0F141B:s={Wt}x{Ht}:d=1[bg]"]
cur = "bg"
for k, p in enumerate(cells):
    args += ["-i", p]
    i, j = divmod(k, 4)
    f.append(f"[{cur}][{k}:v]overlay={cols[j]}:{HEAD + i * (CH + G)}[o{k}]")
    cur = f"o{k}"
txt = []
for j, name in enumerate(["before, 1280x720", "after, 1280x720", "before, 1080 sq", "after, 1080 sq"]):
    txt.append(f"drawtext=fontfile={DISP}:text='{name}':x={cols[j]}:y=10:fontsize=26:fontcolor=0xE8ECF1")
for i, t in enumerate(times):
    txt.append(f"drawtext=fontfile={MONO}:text='{t:.2f} s':x=12:y={HEAD + i * (CH + G) + CH // 2 - 10}"
               f":fontsize=20:fontcolor=0xFE6A37")
f.append(f"[{cur}]{','.join(txt)}[v]")
subprocess.run(["ffmpeg", "-v", "error", "-y", *args, "-filter_complex", ";".join(f), "-map", "[v]",
                "-frames:v", "1", out], check=True)
print(out)
