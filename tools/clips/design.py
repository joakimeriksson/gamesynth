"""Motion-designed version of a game + panel cut (the "design" pass on v8_game). Run from the repo root:
  python3 tools/clips/design.py <cut.json> <wide|square> <out.mp4>

Same cut format as assemble.py (see tools/clips/v8_game_design.json). The sound is assemble.py's audio
graph unchanged, so the mix is identical; only the picture differs:
- the cut between pieces is a "signal sweep": an orange scope trace wipes right to left, starting
  exactly on the sound cut;
- game footage gets a slow push-in (a short settle on the first frames), a grade that sits with the
  dark panel and a vignette; HUD stays clear of anything we draw;
- a Brusverk badge (symbol, wordmark, SOUND ON with a live mini waveform of the game audio) sits
  top-centre on game footage, between the position list and the minimap;
- the lower third has the panel's orange "changed line" marker, slides in and types its sub line;
- the end card pushes the footage back (blur and darken), the logo pops in and emits sound arcs,
  then the claim, the typed URL and the stack line.
Media paths starting with clips_out/ are read from $CLIPS instead when it is set.
"""
import json
import os
import subprocess
import sys

FONTS = "target/clip-fonts"
MONO = f"{FONTS}/IBMPlexMono-Medium.ttf"
DISP = f"{FONTS}/SairaCondensed-Bold.ttf"
SYMBOL = "web/assets/brusverk-symbol.png"          # 198x250, orange, the dot and arcs cut out
WORDMARK = "web/assets/brusverk-wordmark-white.png"  # 703x120
C = dict(ground="0x0F141B", panel="0x161D27", text="0xE8ECF1", muted="0x8593A5", accent="0xFE6A37")
CLIPS = os.environ.get("CLIPS", "clips_out")


def media(p):
    return CLIPS + p[len("clips_out"):] if p.startswith("clips_out/") else p


cut = json.load(open(sys.argv[1]))
mode, out = sys.argv[2], sys.argv[3]
WIDE = mode == "wide"
W, H = (1280, 720) if WIDE else (1080, 1080)
WIPE = cut.get("wipe", 0.2)
work = os.path.join("target", "clips_work", cut["name"] + "_" + mode)
os.makedirs(work, exist_ok=True)
n = 0


def textfile(s):
    global n
    n += 1
    p = os.path.join(work, f"d{n}.txt")
    open(p, "w").write(s)
    return p


def dt(s, x, y, font, size, color, t0=None, t1=None, extra=""):
    en = f":enable='between(t,{t0:.4f},{t1:.4f})'" if t0 is not None else ""
    return (f"drawtext=fontfile={font}:textfile={textfile(s)}:x={x}:y={y}:fontsize={size}"
            f":fontcolor={color}{extra}{en}")


def ease(t0, d, var="t"):
    """Cubic ease-out 0->1 over [t0, t0+d]."""
    return f"(1-pow(1-clip(({var}-{t0:.4f})/{d},0,1),3))"


def typed(s, x, y, font, size, color, t0, t1, spc=1 / 70):
    """Monospaced s typed in, a character every spc s, with a _ cursor; x is the left edge (a number).
    Font-line alignment keeps every prefix on one baseline; y is where y_align=text would put it."""
    yf, al = round(y - 0.33 * size), ":y_align=font"
    out = [dt(s[:k], x, yf, font, size, color, t0 + (k - 1) * spc, t0 + k * spc, extra=al) for k in range(1, len(s))]
    tend = t0 + (len(s) - 1) * spc
    out.append(dt(s, x, yf, font, size, color, tend, t1, extra=al))
    out.append(dt("_", f"'{x}+{size * 0.6}*min({len(s)},floor((t-{t0:.4f})/{spc:.5f})+1)'", yf, font, size,
                  color, t0, tend + 0.35, extra=al))
    return out


def probe(path):
    r = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path],
                       capture_output=True, text=True, check=True)
    return float(r.stdout)


pieces = cut["pieces"]
for p in pieces:
    if "panel" in p:
        p["path"] = media(p["panel"].format(mode="720p" if WIDE else "sq"))
        p["len"] = probe(p["path"])
    else:
        p["path"] = media(p["game"])
        p["len"] = p["dur"]

# ---------------- Audio: assemble.py's graph, unchanged ----------------
# The video side here only reproduces assemble.py's frame timing (concat pads each piece's audio to its
# video), so the concatenated audio is sample-identical; that video is thrown away.
inputs, chains, labels = [], [], []
gain = cut.get("game_gain_db", 0.0)
for i, piece in enumerate(pieces):
    if "panel" in piece:
        inputs += ["-i", piece["path"]]
        chains.append(f"[{i}:v]fps=60,scale=64:36,setsar=1,format=yuv420p[tv{i}];"
                      f"[{i}:a]aresample=48000,aformat=channel_layouts=stereo,afade=t=in:d=0.03[a{i}]")
    else:
        dur = piece["dur"]
        inputs += ["-ss", str(piece["start"]), "-t", str(dur), "-i", piece["path"]]
        if "card" in piece:
            af = f"volume={gain}dB,afade=t=in:d=0.03,afade=t=out:st={dur - 2.0}:d=2.0"
        else:
            af = f"volume={gain}dB,afade=t=in:d=0.03,afade=t=out:st={dur - 0.03}:d=0.03"
        chains.append(f"[{i}:v]scale=64:36,fps=60,setsar=1,format=yuv420p[tv{i}];"
                      f"[{i}:a]aresample=48000,aformat=channel_layouts=stereo,{af}[a{i}]")
    labels.append(f"[tv{i}][a{i}]")
graph = chains + ["".join(labels) + f"concat=n={len(labels)}:v=1:a=1[tv][aout]", "[tv]nullsink"]

# ---------------- Video ----------------
# Contiguous game pieces from one file (a shot and the end card over it) become one segment, so the
# footage runs on under the card. Every segment that is followed by another runs WIPE s longer
# (more footage, or the panel's last frame held) to feed the sweep that starts on the cut.
segs, T = [], 0.0
for p in pieces:
    last = segs[-1] if segs else None
    if (last and "game" in p and last["kind"] == "game" and last["path"] == p["path"]
            and abs(last["start"] + last["len"] - p["start"]) < 1e-6):
        last["parts"].append((last["len"], p))
        last["len"] += p["len"]
    else:
        segs.append(dict(kind="panel" if "panel" in p else "game", path=p["path"], start=p.get("start", 0),
                         len=p["len"], T=T, parts=[(0.0, p)]))
    T += p["len"]
TOTAL = T

nin = sum(1 for a in inputs if a == "-i")


def add_input(args):
    global nin
    inputs.extend(args)
    nin += 1
    return nin - 1


# Badge: [symbol wordmark | ~wave~ SOUND ON], rendered once to a PNG.
bh = 50 if WIDE else 58
k = bh / 50
b_sym_h, b_wm_h = round(34 * k), round(24 * k)
b_sym_w, b_wm_w = round(b_sym_h * 198 / 250), round(b_wm_h * 703 / 120)
chip_w = round(12 * k) + b_sym_w + round(10 * k) + b_wm_w + round(14 * k)
wave_w, wave_h, wave_x = round(50 * k), round(26 * k), round(12 * k)
pill_w = round(205 * k)
badge_w = chip_w + pill_w
badge_png = os.path.join(work, "badge.png")
subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", SYMBOL, "-i", WORDMARK, "-filter_complex",
                f"color=c={C['ground']}@0.84:s={chip_w}x{bh},format=rgba[chip];"
                f"[0:v]scale={b_sym_w}:{b_sym_h},format=rgba[s];[1:v]scale={b_wm_w}:{b_wm_h},format=rgba[w];"
                f"[chip][s]overlay={round(12 * k)}:(H-h)/2[c1];"
                f"[c1][w]overlay={round(12 * k) + b_sym_w + round(10 * k)}:(H-h)/2+1[c2];"
                f"color=c={C['accent']}:s={pill_w}x{bh},format=rgba,"
                f"drawtext=fontfile={DISP}:text='SOUND ON':fontsize={round(32 * k)}:fontcolor={C['ground']}"
                f":x={wave_x + wave_w}+(w-{wave_x + wave_w}-text_w)/2:y=(h-text_h)/2-2[pill];"
                f"[c2][pill]hstack", "-frames:v", "1", badge_png], check=True)
badge_x, badge_y = (W - badge_w) // 2, (22 if WIDE else 36)

vlabels = []
for si, sg in enumerate(segs):
    extra = WIPE if si + 1 < len(segs) else 0.0
    vdur = sg["len"] + extra
    if sg["kind"] == "panel":
        j = add_input(["-i", sg["path"]])
        graph.append(f"[{j}:v]fps=60,scale={W}:{H},setsar=1,tpad=stop_mode=clone:stop_duration={extra},"
                     f"format=yuv420p[sv{si}]")
        vlabels.append(f"[sv{si}]")
        continue
    j = add_input(["-ss", str(sg["start"]), "-t", f"{vdur:.4f}", "-i", sg["path"]])
    first = sg["T"] == 0
    # Push-in: a settle from a slight zoom on the first frames (a punch on the cut), then a slow drift.
    # The first frame is the thumbnail: no punch there, so the HUD is whole.
    punch = 0.0 if first else 0.03
    z = f"1+{punch}*pow(1-min(on/60/0.45,1),3)+0.025*on/60/{vdur:.3f}"
    zp = f"zoompan=z='{z}':x='iw/2-iw/zoom/2':y='ih/2-ih/zoom/2':d=1:fps=60"
    geo = (f"fps=60,{zp}:s=1280x720" if WIDE else
           f"fps=60,crop=1080:1080:420:0,scale=2160:2160:flags=bicubic,{zp}:s=1080x1080")
    grade = "eq=contrast=1.05:saturation=0.9:gamma=0.97,colorbalance=rs=-0.02:bs=0.04:rm=-0.01:bm=0.02,vignette=angle=0.45"
    chain = [f"[{j}:v]{geo},{grade},setsar=1,format=yuv420p[g{si}]"]
    cur = f"g{si}"

    def ov(layer, x, y, en=None):
        global cur
        e = f":enable='{en}'" if en else ""
        chain.append(f"[{cur}][{layer}]overlay=x='{x}':y='{y}'{e}[{cur}o]")
        cur = cur + "o"

    # Badge with a live mini waveform of the game audio (ground-coloured on the orange pill).
    jb = add_input(["-loop", "1", "-framerate", "60", "-t", f"{vdur:.4f}", "-i", badge_png])
    cfade, ct0 = "", 0.0
    for t0, p in sg["parts"]:
        if "card" in p:  # the end card's logo takes over from the badge
            cfade, ct0 = f",fade=t=out:st={t0:.3f}:d=0.3:alpha=1", t0
    chain.append(f"[{jb}:v]format=rgba{cfade}[bd{si}]")
    ov(f"bd{si}", badge_x, badge_y)
    chain.append(f"[{j}:a]showwaves=s={wave_w}x{wave_h}:mode=cline:rate=60:colors={C['ground']}:draw=full"
                 f":scale=sqrt,format=rgba[mw{si}]")
    chain.append(f"[mw{si}]fade=t=out:st={ct0:.3f}:d=0.3:alpha=1[mwf{si}]" if cfade else f"[mw{si}]null[mwf{si}]")
    ov(f"mwf{si}", badge_x + chip_w + wave_x, badge_y + (bh - wave_h) // 2)

    card = None
    for t0, p in sg["parts"]:
        if "card" in p:
            card = (t0, p)
            continue
        # Ends with the segment, or stays on while an end card pushes it back.
        t1 = t0 + p["len"] + (extra if (t0, p) == sg["parts"][-1] else 0.5)
        # Lower third: dark box with the orange "changed line" marker; slides in unless it is the
        # very first frame of the clip (the thumbnail must be complete).
        bx, by, bw, bhh = (24, H - 166, 860, 142) if WIDE else (24, H - 206, W - 48, 170)
        static = first and t0 == 0
        e = "1" if static else ease(t0 + 0.02, 0.38)
        off = f"-({bw}+40)*(1-{e})"
        chain.append(f"color=c={C['ground']}@0.80:s={bw}x{bhh}:r=60:d={vdur:.4f},format=rgba[lb{si}];"
                     f"color=c={C['accent']}:s=6x{bhh}:r=60:d={vdur:.4f},format=rgba[la{si}]")
        en = f"between(t,{t0:.4f},{t1:.4f})"
        ov(f"lb{si}", f"{bx}{off}", by, en)
        ov(f"la{si}", f"{bx}{off}", by, en)
        capx, capy = bx + 18, by + 16
        capfs, subfs = (52, 24) if WIDE else (58, 26)
        suby = capy + (70 if WIDE else 80)
        chain[-1] = chain[-1][:-len(f"[{cur}]")] + "," + dt(p["caption"], f"'{capx}{off}'", capy, DISP, capfs,
                                                            C["text"], t0, t1) + f"[{cur}]"
        if p.get("sub"):
            if static:
                subs = [dt(p["sub"], capx, round(suby - 0.33 * subfs), MONO, subfs, C["accent"], t0, t1,
                           extra=":y_align=font")]
            else:
                subs = typed(p["sub"], capx, suby, MONO, subfs, C["accent"], t0 + 0.38, t1)
            chain[-1] = chain[-1][:-len(f"[{cur}]")] + "," + ",".join(subs) + f"[{cur}]"

    if card:
        c0, p = card
        c = p["card"]
        # Push the footage back: blur and darken fade in over 0.45 s.
        chain.append(f"[{cur}]split[cs{si}][cb{si}];[cb{si}]gblur=sigma=18:enable='gte(t,{c0 - 0.05:.3f})',"
                     f"format=yuva420p,fade=t=in:st={c0:.3f}:d=0.45:alpha=1[cbf{si}];"
                     f"[cs{si}][cbf{si}]overlay[cbo{si}];"
                     f"color=c={C['ground']}@0.80:s={W}x{H}:r=60:d={vdur:.4f},format=rgba,"
                     f"fade=t=in:st={c0:.3f}:d=0.45:alpha=1[dk{si}];[cbo{si}][dk{si}]overlay[cd{si}]")
        cur = f"cd{si}"
        # Layout: symbol, wordmark, claim, URL, stack, as one centred column.
        if WIDE:
            SH, WMW, CL, UF, FF, g = 140, 340, 62, 34, 22, (22, 40, 26, 24)
        else:
            SH, WMW, CL, UF, FF, g = 200, 480, 84, 40, 26, (30, 54, 34, 28)
        SW = round(SH * 198 / 250)
        WMH = round(WMW * 120 / 703)
        col = SH + g[0] + WMH + g[1] + CL + g[2] + UF + g[3] + FF
        top = (H - col) // 2 - (14 if WIDE else 20)
        sy = top
        wy = sy + SH + g[0]
        cy = wy + WMH + g[1]
        uy = cy + CL + g[2]
        fy = uy + UF + g[3]
        # Symbol pops in (back-ease overshoot from 40 %).
        ts = c0 + 0.08
        pp = f"clip((t-{ts:.3f})/0.42,0,1)"
        back = f"(1+2.70158*pow({pp}-1,3)+1.70158*pow({pp}-1,2))"
        js = add_input(["-loop", "1", "-framerate", "60", "-t", f"{vdur:.4f}", "-i", SYMBOL])
        chain.append(f"[{js}:v]format=rgba,scale=w='max(2,{SW}*(0.4+0.6*{back}))':h=-1:eval=frame,"
                     f"fade=t=in:st={ts:.3f}:d=0.14:alpha=1[sym{si}]")
        ov(f"sym{si}", f"{W / 2}-w/2", f"{sy + SH / 2}-h/2", f"gte(t,{c0:.3f})")
        # Sound arcs leave the symbol's dot, like its own arcs carrying on outwards.
        f = SH / 250
        dotx, doty = W / 2 - SW / 2 + 81 * f, sy + 162 * f
        R0, span = 132 * f, 240 * f
        AW, AH = int(R0 + span + 20), int(2 * 0.72 * (R0 + span) + 20)
        sig = 3.4 * f * 1.4
        terms = []
        for a in range(3):
            ta = c0 + 0.40 + 0.16 * a
            R = f"({R0:.1f}+{span / 0.85:.1f}*(T-{ta:.3f}))"
            life = f"gt(T,{ta:.3f})*clip(1-({R}-{R0:.1f})/{span:.1f},0,1)"
            terms.append(f"exp(-pow((hypot(X,Y-{AH / 2})-{R})/{sig:.2f},2))*{life}")
        gate = f"clip((0.72-abs(atan2(Y-{AH / 2},X)))/0.12,0,1)"
        alpha = f"255*min(1,({'+'.join(terms)}))*{gate}"
        chain.append(f"color=c=black@0:s={AW}x{AH}:r=60:d={vdur:.4f},format=rgba,"
                     f"geq=r=254:g=106:b=55:a='{alpha}':enable='between(t,{c0 + 0.38:.3f},{c0 + 1.6:.3f})'[arc{si}]")
        ov(f"arc{si}", int(dotx), int(doty - AH / 2), f"between(t,{c0 + 0.38:.3f},{c0 + 1.6:.3f})")
        # Wordmark rises in, then the claim, the URL types, the stack fades in.
        jw = add_input(["-loop", "1", "-framerate", "60", "-t", f"{vdur:.4f}", "-i", WORDMARK])
        chain.append(f"[{jw}:v]format=rgba,scale={WMW}:-1,fade=t=in:st={c0 + 0.3:.3f}:d=0.25:alpha=1[wm{si}]")
        ov(f"wm{si}", f"{(W - WMW) // 2}", f"{wy}+16*(1-{ease(c0 + 0.3, 0.4)})", f"gte(t,{c0:.3f})")
        e = ease(c0 + 0.5, 0.35)
        texts = [dt(c["caption"], "(w-text_w)/2", f"'{cy}+16*(1-{e})'", DISP, CL, C["text"], c0 + 0.5, vdur,
                    extra=f":alpha='{e}'")]
        ux = round((W - len(c["sub"]) * UF * 0.6) / 2)
        texts += typed(c["sub"], ux, uy, MONO, UF, C["accent"], c0 + 0.8, vdur)
        e = ease(c0 + 1.3, 0.4)
        texts.append(dt(c.get("foot", ""), "(w-text_w)/2", fy, MONO, FF, C["muted"], c0 + 1.3, vdur,
                        extra=f":alpha='{e}'"))
        chain.append(f"[{cur}]{','.join(texts)}[ct{si}]")
        cur = f"ct{si}"
    chain.append(f"[{cur}]format=yuv420p[sv{si}]")
    graph += chain
    vlabels.append(f"[sv{si}]")

# Signal sweep between segments: xfade reveals the next segment right to left behind a scope trace.
PE = "(P*P*(3-2*P))"
cur = vlabels[0]
for si in range(1, len(segs)):
    T = segs[si]["T"]
    graph.append(f"{cur}{vlabels[si]}xfade=transition=custom:duration={WIPE}:offset={T:.4f}"
                 f":expr='if(gt(X/W,{PE}),B,A)'[x{si}]")
    cur = f"[x{si}]"
LW = 44
graph.append(f"color=c=black@0:s={LW}x{H}:r=60:d={TOTAL:.4f},format=rgba,"
             f"geq=r=254:g='106+130*exp(-pow((X-{LW // 2})/1.6,2))':b='55+150*exp(-pow((X-{LW // 2})/1.6,2))'"
             f":a='255*min(1,exp(-pow((X-{LW // 2})/2.4,2))+0.32*exp(-pow((X-{LW // 2})/9,2)))',split={len(segs) - 1}"
             + "".join(f"[ln{si}]" for si in range(1, len(segs))))
for si in range(1, len(segs)):
    T = segs[si]["T"]
    P = f"clip(1-(t-{T:.4f})/{WIPE},0,1)"
    graph.append(f"{cur}[ln{si}]overlay=x='W*{P}*{P}*(3-2*{P})-{LW // 2}':y=0"
                 f":enable='between(t,{T:.4f},{T + WIPE:.4f})'[y{si}]")
    cur = f"[y{si}]"
graph.append(f"{cur}trim=duration={TOTAL:.4f},format=yuv420p[v]")

fcfile = os.path.join(work, "graph.txt")
open(fcfile, "w").write(";\n".join(graph))
cmd = ["ffmpeg", "-v", "error", "-y", *inputs, "-filter_complex_script", fcfile, "-map", "[v]", "-map", "[aout]",
       "-c:v", "libx264", "-preset", "slow", "-crf", "17", "-profile:v", "high", "-pix_fmt", "yuv420p", "-r", "60",
       "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-movflags", "+faststart", out]
subprocess.run(cmd, check=True)
print(out)
