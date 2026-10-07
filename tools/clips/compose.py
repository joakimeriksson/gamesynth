"""Build a Brusverk demo clip: code panel whose values change on a timeline, live scrolling
spectrum + waveform of the engine's own render, captions, SOUND ON cue, end card.
Run from the repo root (see FILMING.md, tools/clips/campfire.sh):
  python3 tools/clips/compose.py <clip.json> <wide|square> <out.mp4>
Needs ffmpeg built with libfreetype (drawtext). Fonts (OFL, as on the site) are fetched once.
"""
import json, os, subprocess, sys, urllib.request

FONTS = "target/clip-fonts"
GF = "https://raw.githubusercontent.com/google/fonts/main/ofl/"
os.makedirs(FONTS, exist_ok=True)
for f in ["ibmplexmono/IBMPlexMono-Medium.ttf", "sairacondensed/SairaCondensed-Bold.ttf"]:
    dst = os.path.join(FONTS, os.path.basename(f))
    if not os.path.exists(dst):
        urllib.request.urlretrieve(GF + f, dst)
MONO = f"{FONTS}/IBMPlexMono-Medium.ttf"
DISP = f"{FONTS}/SairaCondensed-Bold.ttf"
WORDMARK = "web/assets/brusverk-wordmark-white.png"
C = dict(ground="0x0F141B", panel="0x161D27", line="0x2A3542", text="0xE8ECF1", muted="0x8593A5",
         readout="0xBFD8E6", accent="0xFE6A37", gen="0x8FD16A", sfx="0x5BD1C8")

clip = json.load(open(sys.argv[1]))
mode, out = sys.argv[2], sys.argv[3]
work = os.path.join("target", "clips_work", clip["name"] + "_" + mode)
os.makedirs(work, exist_ok=True)
dur = clip["duration"]
hl = C[clip.get("color", "gen")]

if mode == "wide":
    W, H = 1280, 720
    L = dict(logo=(40, 32, 230), pill=(1080, 30),
             code=(40, 100, 560, 440), fs=22, lh=38,
             spec=(640, 100, 600, 300), wave=(640, 410, 600, 130),
             cap=(40, 580), capfs=54, sub=(40, 650), subfs=26)
else:
    W, H = 1080, 1080
    L = dict(logo=(48, 40, 260), pill=(870, 40),
             code=(48, 120, 984, 432), fs=26, lh=40,
             spec=(48, 574, 984, 236), wave=(48, 822, 984, 90),
             cap=(48, 930), capfs=60, sub=(48, 1010), subfs=28)

n = 0
def textfile(s):
    global n
    n += 1
    p = os.path.join(work, f"t{n}.txt")
    open(p, "w").write(s)
    return p

def esc(p):
    return p.replace("\\", "\\\\").replace(":", "\\:").replace("'", "\\'").replace(",", "\\,")

def dt(s, x, y, font, size, color, t0=None, t1=None, extra=""):
    en = f":enable='between(t,{t0},{t1})'" if t0 is not None else ""
    return (f"drawtext=fontfile={esc(font)}:textfile={esc(textfile(s))}:x={x}:y={y}:fontsize={size}"
            f":fontcolor={color}{extra}{en}")

def box(x, y, w, h, color, t0=None, t1=None, fill=True):
    en = f":enable='between(t,{t0},{t1})'" if t0 is not None else ""
    return f"drawbox=x={x}:y={y}:w={w}:h={h}:color={color}:t={'fill' if fill else 2}{en}"

segs = clip["segments"]
# Values carry forward from segment to segment (a "reset" segment goes back to the file's
# values); only a segment's own changes are highlighted.
state = {}
for i, s in enumerate(segs):
    s["t1"] = segs[i + 1]["t"] if i + 1 < len(segs) else dur
    if s.get("reset"):
        state = {}
    state.update(s.get("values", {}))
    s["shown"] = dict(state)
vf = []
# Panels
cx, cy, cw, ch = L["code"]
vf.append(box(cx, cy, cw, ch, C["panel"]))
vf.append(box(cx, cy, cw, ch, C["line"], fill=False))
sx, sy, sw, sh = L["spec"]
wx, wy, ww, wh = L["wave"]
vf.append(box(sx - 2, sy - 2, sw + 4, sh + 4, C["line"], fill=False))
vf.append(box(wx - 2, wy - 2, ww + 4, wh + 4, C["line"], fill=False))
# Code panel: header then key = value lines; values may change per segment.
pad, fs, lh = 22, L["fs"], L["lh"]
y = cy + pad
vf.append(dt(clip["file"], cx + pad, y, MONO, fs - 4, C["muted"]))
y += lh
keyw = max(len(l["key"]) for l in clip["lines"] if "key" in l)
charw = fs * 0.6
live = []
for line in clip["lines"]:
    if "section" in line:
        vf.append(dt(line["section"], cx + pad, y, MONO, fs, C["muted"]))
        y += lh
        continue
    key = line["key"].ljust(keyw) + " = "
    vx = cx + pad + int(len(key) * charw)
    vf.append(dt(key, cx + pad, y, MONO, fs, C["readout"]))
    if "live" in line:
        # Driven every frame from the render's trace (an input the game sets, or the revs).
        col, name = line["live"], f"live{len(live)}"
        live.append((col, name, line))
        if line.get("bar"):
            # A bar, optionally followed by a second live readout ("then"), e.g. the revs.
            bw = cw - (vx - cx) - pad - (int(10 * charw) if "then" in line else 0)
            vf.append(box(vx, y + 2, bw, lh - 18, C["line"]))
            vf.append(f"drawbox@{name}=x={vx}:y={y + 2}:w=1:h={lh - 18}:color={C['accent']}:t=fill")
            line["bw"] = bw
            if "then" in line:
                tn = f"live{len(live)}"
                then = dict(line["then"])
                live.append((then["live"], tn, then))
                vf.append(f"drawtext@{tn}=fontfile={esc(MONO)}:text=0:x={vx + bw + int(charw)}:y={y}:fontsize={fs}:fontcolor={C['accent']}")
        else:
            vf.append(f"drawtext@{name}=fontfile={esc(MONO)}:text=0:x={vx}:y={y}:fontsize={fs}:fontcolor={C['accent']}")
        y += lh
        continue
    for s in segs:
        changed = line["key"] in s.get("values", {})
        val = str(s["shown"].get(line["key"], line["value"]))
        if changed:
            vf.append(box(cx + 8, y - 7, cw - 16, lh - 2, hl + "@0.16", s["t"], s["t1"]))
            vf.append(box(cx + 8, y - 7, 4, lh - 2, hl, s["t"], s["t1"]))
        vf.append(dt(val, vx, y, MONO, fs, hl if changed else C["text"], s["t"], s["t1"]))
    y += lh
# SOUND ON pill
px, py = L["pill"]
vf.append(box(px, py, 160, 44, C["accent"]))
vf.append(dt("SOUND ON", px + 22, py + 6, DISP, 30, C["ground"]))
# Captions
capx, capy = L["cap"]
subx, suby = L["sub"]
for s in segs:
    if s.get("card"):
        continue
    vf.append(dt(s["caption"], capx, capy, DISP, L["capfs"], C["text"], s["t"], s["t1"]))
    if s.get("sub"):
        vf.append(dt(s["sub"], subx, suby, MONO, L["subfs"], hl, s["t"], s["t1"]))
# End card: darken everything, big claim, repo
card = next((s for s in segs if s.get("card")), None)
if card:
    t0, t1 = card["t"], card["t1"]
    vf.append(box(0, 0, W, H, C["ground"] + "@0.82", t0, t1))
    big = 84 if mode == "wide" else 96
    vf.append(dt(card["caption"], "(w-text_w)/2", f"h/2-{big}", DISP, big, C["text"], t0, t1))
    vf.append(dt(card["sub"], "(w-text_w)/2", f"h/2+{int(big*0.35)}", MONO, 30, hl, t0, t1))
    vf.append(dt(card.get("foot", ""), "(w-text_w)/2", f"h/2+{int(big*0.35)+56}", MONO, 24, C["muted"], t0, t1))

# Live lines: one sendcmd entry per trace row (60 Hz).
if live:
    import csv
    cmds = []
    for row in csv.DictReader(open(clip["trace"])):
        parts = []
        for col, name, line in live:
            v = float(row[col])
            if line.get("bar"):
                parts.append(f"drawbox@{name} w {max(1, int(v / line.get('max', 1.0) * line['bw']))}")
            else:
                parts.append(f"drawtext@{name} reinit 'text={line.get("fmt", "{:.0f}").format(v)}'")
        cmds.append(f"{float(row['t']):.4f} {', '.join(parts)};")
    cmdfile = os.path.join(work, "live.cmd")
    open(cmdfile, "w").write("\n".join(cmds) + "\n")
    vf.insert(0, f"sendcmd=f={esc(cmdfile)}")
lx, ly, lw = L["logo"]
fc = (
    f"color=c={C['ground']}:s={W}x{H}:r=60:d={dur}[bg];"
    f"[1:a]asplit=2[a1][a2];"
    f"[a1]showspectrum=s={sw//2}x{sh}:slide=scroll:fscale=log:scale=log:color=magma:"
    f"legend=0:fps=60:drange=80,scale={sw}:{sh},"
    f"format=rgba[spec];"
    f"[a2]showwaves=s={ww}x{wh}:mode=cline:rate=60:colors={hl}:draw=full:scale=sqrt,format=rgba[wav];"
    f"[2:v]scale={lw}:-1[logo];"
    f"[bg][spec]overlay={sx}:{sy}:shortest=1[v1];[v1][wav]overlay={wx}:{wy}[v2];[v2][logo]overlay={lx}:{ly}[v3];"
    f"[v3]{','.join(vf)},format=yuv420p[v]"
)
fcfile = os.path.join(work, "graph.txt")
open(fcfile, "w").write(fc)
cmd = ["ffmpeg", "-v", "error", "-y",
       "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo",  # input 0 unused placeholder
       "-i", clip["audio"], "-loop", "1", "-i", WORDMARK,
       "-filter_complex_script", fcfile, "-map", "[v]", "-map", "1:a", "-t", str(dur),
       "-c:v", "libx264", "-preset", "slow", "-crf", "16", "-profile:v", "high", "-pix_fmt", "yuv420p", "-r", "60",
       "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-movflags", "+faststart", out]
subprocess.run(cmd, check=True)
print(out)
