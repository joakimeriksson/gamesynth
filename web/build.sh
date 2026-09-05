#!/usr/bin/env sh
# Build the engine to WebAssembly and place it where web/index.html loads it.
# Requires the wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown).
set -eu
cd "$(dirname "$0")/.."
CARGO="${CARGO:-cargo}"
# With Homebrew's keg-only rustup next to a Homebrew rust, cargo can pick up the wrong
# rustc from PATH; pin it to the toolchain that owns the wasm target.
if [ -z "${RUSTC:-}" ] && [ -x "$(dirname "$CARGO")/rustup" ]; then
  RUSTC="$("$(dirname "$CARGO")/rustup" which rustc)"; export RUSTC
fi
"$CARGO" build -p gamesynth-wasm --profile wasm --target wasm32-unknown-unknown
mkdir -p web/pkg
cp target/wasm32-unknown-unknown/wasm/gamesynth_wasm.wasm web/pkg/gamesynth_wasm.wasm
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Os -o web/pkg/gamesynth_wasm.wasm web/pkg/gamesynth_wasm.wasm
fi
ls -la web/pkg/gamesynth_wasm.wasm
