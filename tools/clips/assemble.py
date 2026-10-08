"""Join game footage and composed panel clips into one social clip. Run from the repo root:
  python3 tools/clips/assemble.py <cut.json> <wide|square> <out.mp4>

A cut is a list of pieces, played in order:
  {"game": "clips_out/x.avi", "start": 3.0, "dur": 3.4, "caption": "...", "sub": "..."}
  {"panel": "clips_out/v8_mid_{mode}.mp4"}            # {mode} becomes 720p or sq
  {"game": ..., "start": ..., "dur": ..., "card": {"caption": ..., "sub": ..., "foot": ...}}
Game footage is scaled (wide) or centre-cropped (square), gets the SOUND ON pill and a caption box,
and its audio is brought to the panel's loudness (`game_gain_db`); a card piece darkens the footage
under the end card and fades the sound out.
"""
import json
import os
import subprocess
import sys

FONTS = "target/clip-fonts"
MONO = f"{FONTS}/IBMPlexMono-Medium.ttf"
DISP = f"{FONTS}/SairaCondensed-Bold.ttf"
C = dict(ground="0x0F141B", text="0xE8ECF1", muted="0x8593A5", accent="0xFE6A37")

cut = json.load(open(sys.argv[1]))
mode, out = sys.argv[2], sys.argv[3]
W, H = (1280, 720) if mode == "wide" else (1080, 1080)
work = os.path.join("target", "clips_work", cut["name"] + "_" + mode)
os.makedirs(work, exist_ok=True)
n = 0


def textfile(s):
    global n
    n += 1
    p = os.path.join(work, f"a{n}.txt")
    open(p, "w").write(s)
    return p


def dt(s, x, y, font, size, color):
    return f"drawtext=fontfile={font}:textfile={textfile(s)}:x={x}:y={y}:fontsize={size}:fontcolor={color}"


inputs, chains, labels = [], [], []
gain = cut.get("game_gain_db", 0.0)
for i, piece in enumerate(cut["pieces"]):
    if "panel" in piece:
        inputs += ["-i", piece["panel"].format(mode="720p" if mode == "wide" else "sq")]
        chains.append(f"[{i}:v]fps=60,scale={W}:{H},setsar=1,format=yuv420p[v{i}];"
                      f"[{i}:a]aresample=48000,aformat=channel_layouts=stereo,afade=t=in:d=0.03[a{i}]")
    else:
        dur = piece["dur"]
        inputs += ["-ss", str(piece["start"]), "-t", str(dur), "-i", piece["game"]]
        if mode == "wide":
            v = f"scale={W}:{H}"
            pill, cap = (W // 2 - 80, 24), (40, H - 150)
            box = f"drawbox=x=24:y={H - 166}:w=860:h=142:color={C['ground']}@0.78:t=fill"
        else:
            v = f"crop=1080:1080:420:0"
            pill, cap = (W // 2 - 80, 40), (48, H - 190)
            box = f"drawbox=x=24:y={H - 206}:w={W - 48}:h=170:color={C['ground']}@0.78:t=fill"
        vf = [v, "fps=60", "setsar=1"]
        if "card" in piece:
            c = piece["card"]
            big = 84 if mode == "wide" else 96
            vf += [f"drawbox=x=0:y=0:w={W}:h={H}:color={C['ground']}@0.8:t=fill",
                   dt(c["caption"], "(w-text_w)/2", f"h/2-{big // 2}", DISP, big, C["text"]),
                   dt(c["sub"], "(w-text_w)/2", f"h/2+{int(big * 0.7)}", MONO, 30, C["accent"]),
                   dt(c.get("foot", ""), "(w-text_w)/2", f"h/2+{int(big * 0.7) + 56}", MONO, 22, C["muted"])]
            af = f"volume={gain}dB,afade=t=in:d=0.03,afade=t=out:st={dur - 2.0}:d=2.0"
        else:
            vf += [f"drawbox=x={pill[0]}:y={pill[1]}:w=160:h=44:color={C['accent']}:t=fill",
                   dt("SOUND ON", pill[0] + 22, pill[1] + 6, DISP, 30, C["ground"]), box,
                   dt(piece["caption"], cap[0], cap[1], DISP, 52 if mode == "wide" else 58, C["text"])]
            if piece.get("sub"):
                vf.append(dt(piece["sub"], cap[0], cap[1] + (70 if mode == "wide" else 80), MONO, 24, C["accent"]))
            af = f"volume={gain}dB,afade=t=in:d=0.03,afade=t=out:st={dur - 0.03}:d=0.03"
        chains.append(f"[{i}:v]{','.join(vf)},format=yuv420p[v{i}];[{i}:a]aresample=48000,aformat=channel_layouts=stereo,{af}[a{i}]")
    labels.append(f"[v{i}][a{i}]")
fc = ";".join(chains) + ";" + "".join(labels) + f"concat=n={len(labels)}:v=1:a=1[v][a]"
fcfile = os.path.join(work, "graph.txt")
open(fcfile, "w").write(fc)
cmd = ["ffmpeg", "-v", "error", "-y", *inputs, "-filter_complex_script", fcfile, "-map", "[v]", "-map", "[a]",
       "-c:v", "libx264", "-preset", "slow", "-crf", "17", "-profile:v", "high", "-pix_fmt", "yuv420p", "-r", "60",
       "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-movflags", "+faststart", out]
subprocess.run(cmd, check=True)
print(out)
