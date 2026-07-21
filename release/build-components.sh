#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
out="${1:-$root/target/vrules-components}"
mkdir -p "$out"

cargo build --release --target wasm32-wasip2 \
  -p vrules-runtime-component \
  -p vrules-storage-component \
  -p vrules-rules-component \
  -p vrules-admin-component \
  -p vrules-gcp-component \
  -p vrules-cache-component

target_dir="$(
  cargo metadata --format-version 1 --no-deps |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'
)"
[[ -n "$target_dir" ]] || { echo "cannot resolve Cargo target directory" >&2; exit 1; }
release="$target_dir/wasm32-wasip2/release"

install -m 0644 "$release/vrules_runtime_component.wasm" "$out/vrules-runtime.wasm"
install -m 0644 "$release/vrules_storage_component.wasm" "$out/vrules-storage.wasm"
install -m 0644 "$release/vrules_rules_component.wasm" "$out/vrules-rules.wasm"
install -m 0644 "$release/vrules_admin_component.wasm" "$out/vrules-admin.wasm"
install -m 0644 "$release/vrules_gcp_component.wasm" "$out/vrules-gcp.wasm"
install -m 0644 "$release/vrules_cache_component.wasm" "$out/vrules-cache.wasm"

"$root/components/vrules-embedding-wllama/build.sh" "$out"
