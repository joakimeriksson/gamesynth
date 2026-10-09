"""Gather the per-species SOURCES.tsv files (written by fs_get.py) into one
target/refs/birds/SOURCES.tsv, with the species and how each recording was used.

    sources.py
"""
import os

ROOT = "/Users/joakimeriksson/work/gamesynth/target/refs/birds"
# Recordings that were downloaded but not transcribed, and why.
REJECTED = {
    "517779": "rejected: hard-edged synthetic-sounding notes at 0.75/0.65 kHz with broadband onsets; not used",
    "517780": "rejected: hard-edged synthetic-sounding notes at 0.75/0.65 kHz with broadband onsets; not used",
    "517781": "rejected: hard-edged synthetic-sounding notes at 0.75/0.65 kHz with broadband onsets; not used",
    "274771": "rejected: noisy; the calls in it are not a clear cuckoo",
    "517793": "rejected: hard onsets and a steady 0.65 kHz tone, not the pigeon's five-note coo",
    "517794": "rejected: hard onsets and a steady 0.65 kHz tone, not the pigeon's five-note coo",
    "517795": "rejected: hard onsets and a steady 0.65 kHz tone, not the pigeon's five-note coo",
    "122618": "rejected: repeated 3.5-4 kHz calls, not the chaffinch's song",
    "867131": "not used: tawny-owl ke-wick calls in noise (735744 was clearer)",
    "620236": "rejected: drumming too faint and distorted",
}
# Recordings used only for structure (song order, pauses) or a call, not contours.
SUPPORT = {"547901", "813087", "195908", "276560", "402381", "725331", "790795", "849589", "607224", "607243",
           "476139", "577265", "688869", "670176", "737211", "818829", "73497", "395101", "456208", "516160",
           "399221", "277149", "244357", "867542"}
rows = []
for sp in sorted(os.listdir(ROOT)):
    f = os.path.join(ROOT, sp, "SOURCES.tsv")
    if not os.path.isfile(f):
        continue
    for line in open(f):
        cols = line.rstrip("\n").split("\t")
        if len(cols) < 6:
            continue
        use = REJECTED.get(cols[0]) or ("supporting: structure, timing, timbre" if cols[0] in SUPPORT else "transcribed: contours and grammar")
        rows.append([sp] + cols + [use])
with open(os.path.join(ROOT, "SOURCES.tsv"), "w") as fh:
    fh.write("species\tfreesound_id\tuser\ttitle\tlicence\tduration_s\turl\tuse\n")
    for r in rows:
        fh.write("\t".join(r) + "\n")
print(len(rows), "recordings,", sum(1 for r in rows if r[-1].startswith("transcribed")), "transcribed,",
      sum(1 for r in rows if r[-1].startswith("rejected")), "rejected;", "licences:", sorted({r[4] for r in rows}))
