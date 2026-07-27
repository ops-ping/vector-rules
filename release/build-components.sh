#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
out="${1:-$root/target/vrules-components}"
mkdir -p "$out"

cargo build --release --target wasm32-wasip2 \
  -p vrules-mcp \
  -p vrules-storage \
  -p vrules-harness \
  -p vrules-admin \
  -p vrules-gcp \
  -p vrules-cache

target_dir="$(
  cargo metadata --format-version 1 --no-deps |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'
)"
[[ -n "$target_dir" ]] || { echo "cannot resolve Cargo target directory" >&2; exit 1; }
release="$target_dir/wasm32-wasip2/release"

install -m 0644 "$release/vrules_mcp.wasm" "$out/vrules-mcp.wasm"
install -m 0644 "$release/vrules_storage.wasm" "$out/vrules-storage.wasm"
install -m 0644 "$release/vrules_harness.wasm" "$out/vrules-harness.wasm"
install -m 0644 "$release/vrules_admin.wasm" "$out/vrules-admin.wasm"
install -m 0644 "$release/vrules_gcp.wasm" "$out/vrules-gcp.wasm"
install -m 0644 "$release/vrules_cache.wasm" "$out/vrules-cache.wasm"

"$root/components/vrules-embedding-wllama/build.sh" "$out"
