#!/usr/bin/env bash
# Build the GDExtension for the web (wasm32-unknown-emscripten, single-threaded).
# Needs: emsdk 4.0.20 (the version Godot 4.7.2's web template was built with) at ~/emsdk,
# and a pinned Rust nightly from before -Zemscripten-wasm-eh was removed (June 2026).
set -euo pipefail
NIGHTLY="${NIGHTLY:-nightly-2026-05-20}"
HOST="$(rustc -vV | sed -n 's/^host: //p')"
rustup toolchain list | grep -q "$NIGHTLY" || { rustup toolchain install "$NIGHTLY" --profile minimal -c rust-src; rustup target add wasm32-unknown-emscripten --toolchain "$NIGHTLY"; }
export EMSDK="${EMSDK:-$HOME/emsdk}"
export PATH="$HOME/.rustup/toolchains/$NIGHTLY-$HOST/bin:$EMSDK:$EMSDK/upstream/emscripten:$PATH"
cd "$(dirname "$0")/.."
cargo build -p gamesynth-godot --no-default-features --features nothreads -Zbuild-std --target wasm32-unknown-emscripten --release
ls -la target/wasm32-unknown-emscripten/release/gamesynth_godot.wasm
