#!/bin/sh
# Render every species (two individuals, 30 s) and their alarm calls into <ours_dir>, then the
# comparison images.   sh tools/birds/render.sh <ours_dir> [species...]
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
O=$1; shift
cd "$ROOT" || exit 1
cargo run -q -p brusverk-core --release --example render_birds -- species "$O" 30 0 "$@" | sed 's/.*\///'
cargo run -q -p brusverk-core --release --example render_birds -- alarm "$O/alarm" 12 "$@" | sed 's/.*\///'
