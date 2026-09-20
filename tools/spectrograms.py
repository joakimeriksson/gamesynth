#!/usr/bin/env python3
"""Spectrogram contact sheet for rendered generator WAVs.

    cargo run -p gamesynth-core --example render_models --release -- models_out
    cargo run -p gamesynth-core --example render_graphs --release -- models_out
    tools/spectrograms.py models_out sheet.png wind_default rain_default graph_shield

Each row: log-frequency spectrogram (30 Hz - 20 kHz) with the 50 ms RMS level in cyan. The
render examples sweep input 0 from 0 to 1 over 0-4 s, hold it for 4-6 s with input 1 raised
to 1, then fall back over 6-10 s (white lines mark 4 s and 6 s).
"""
import sys
import wave

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from scipy.signal import spectrogram


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    folder, sheet, names = sys.argv[1], sys.argv[2], sys.argv[3:]
    fig, axes = plt.subplots(len(names), 1, figsize=(13, 2.6 * len(names)), facecolor="#0F141B")
    for ax, name in zip(np.atleast_1d(axes), names):
        with wave.open(f"{folder}/{name}.wav") as w:
            sr, channels = w.getframerate(), w.getnchannels()
            x = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(float) / 32767
            x = x.reshape(-1, channels).mean(axis=1)  # renders are stereo; analyse the fold-down
        f, t, power = spectrogram(x, fs=sr, nperseg=2048, noverlap=1536, window="hann")
        band = (f >= 30) & (f <= 20000)
        ax.pcolormesh(t, f[band], 10 * np.log10(power[band] + 1e-12), vmin=-105, vmax=-35, cmap="magma", shading="auto")
        ax.set_yscale("log")
        ax.set_ylim(30, 20000)
        ax.set_yticks([50, 100, 200, 500, 1000, 2000, 5000, 10000])
        ax.set_yticklabels(["50", "100", "200", "500", "1k", "2k", "5k", "10k"])
        window = sr // 20
        rms = np.sqrt(np.convolve(x**2, np.ones(window) / window, "same"))
        level = ax.twinx()
        level.plot(np.arange(len(x)) / sr, rms, color="#7BE0FF", lw=1.2)
        level.set_ylim(0, 0.7)
        level.tick_params(colors="#7BE0FF", labelsize=7)
        ax.set_title(name, color="w", fontsize=9, loc="left")
        ax.tick_params(colors="#8593A5", labelsize=7)
        ax.set_facecolor("black")
        ax.set_xlim(0, len(x) / sr)
    plt.tight_layout()
    plt.savefig(sheet, dpi=70)
    print(sheet)


if __name__ == "__main__":
    main()
