#!/usr/bin/env bash
# Render the window (`--gui`) as the program draws it, with the default look and a made-up song,
# without opening a window: scripts/render.sh [out.png]   (default: target/render/main.png)
set -euo pipefail
cd "$(dirname "$0")/.."
out="${1:-target/render/main.png}"
MDIS_RENDER="$out" cargo test --features gui render_main_view -- --nocapture 2>&1 | grep -E "rendered|panicked|error" || true
[ -f "$out" ] && echo "$out" && echo "${out%.png}-spotify.png"
