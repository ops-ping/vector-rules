#!/usr/bin/env bash
# Build the browser-ready vrules-wasm release artifact.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

command -v wasm-pack >/dev/null || { echo "build-wasm: 'wasm-pack' not found (cargo install wasm-pack)"; exit 1; }

version="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"
dist="$root/dist"; mkdir -p "$dist"

tarcomp=(--zstd); ext=tar.zst
command -v zstd >/dev/null || { tarcomp=(--gzip); ext=tar.gz; echo "build-wasm: zstd absent -> .tar.gz"; }

echo "==> vrules-wasm wasm32-unknown-unknown (web package)"
wasm-pack build crates/vrules-wasm --target web --out-name vrules_wasm --release >/dev/null

stage="$(mktemp -d)"; pkg="vrules-wasm-$version-web"
mkdir -p "$stage/$pkg"
cp -r crates/vrules-wasm/pkg/. "$stage/$pkg/"
cp crates/vrules-wasm/README.md LICENSE-APACHE LICENSE-MIT "$stage/$pkg/" 2>/dev/null || true
tar -C "$stage" "${tarcomp[@]}" -cf "$dist/$pkg.$ext" "$pkg"
rm -rf "$stage"

echo "==> em-log-n-wasm wasm32-unknown-unknown (web package)"
wasm-pack build crates/em-log-n-wasm --target web --out-name em_log_n_wasm --release >/dev/null

stage="$(mktemp -d)"; pkg="em-log-n-wasm-$version-web"
mkdir -p "$stage/$pkg/pkg"
cp -r crates/em-log-n-wasm/pkg/. "$stage/$pkg/pkg/"
mkdir -p "$stage/$pkg/web"
cp crates/em-log-n-wasm/web/em_log_n_browser.js "$stage/$pkg/web/"
cp crates/em-log-n-wasm/README.md LICENSE-APACHE LICENSE-MIT "$stage/$pkg/" 2>/dev/null || true
tar -C "$stage" "${tarcomp[@]}" -cf "$dist/$pkg.$ext" "$pkg"
rm -rf "$stage"

echo "build-wasm: artifacts in $dist"
