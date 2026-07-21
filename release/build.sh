#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
export PATH="$HOME/.local/bin:$PATH"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
target_dir="$(
  cargo metadata --format-version 1 --no-deps |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'
)"
components="$root/target/vrules-components"
dist="$root/dist"
rm -rf "$dist"
mkdir -p "$dist"

if [[ "${VRULES_SKIP_COMPONENT_BUILD:-0}" != 1 ]]; then
  "$root/release/build-components.sh" "$components"
fi
[[ -f "$components/vrules-embedding-wllama.wasm" ]] || {
  echo "component artifacts are missing from $components" >&2
  exit 1
}

triples=("$@")
if [[ ${#triples[@]} -eq 0 ]]; then
  triples=("$(rustc -vV | sed -n 's/^host: //p')")
  cargo build --release -p vrules-shim
else
  command -v cross >/dev/null || {
    echo "cross is required when explicit target triples are supplied" >&2
    exit 1
  }
  if [[ -z "${CROSS_CONTAINER_ENGINE:-}" ]] && command -v podman >/dev/null; then
    export CROSS_CONTAINER_ENGINE=podman
  fi
fi

tar_args=(--zstd)
extension=tar.zst
if ! command -v zstd >/dev/null ||
  ! tar --help 2>&1 | grep -q -- '--zstd'; then
  tar_args=(--gzip)
  extension=tar.gz
fi

for triple in "${triples[@]}"; do
  if [[ "$triple" != "$(rustc -vV | sed -n 's/^host: //p')" || ${#@} -gt 0 ]]; then
    cross build --release --target "$triple" -p vrules-shim
    shim="$target_dir/$triple/release/vrules-shim"
  else
    shim="$target_dir/release/vrules-shim"
  fi

  stage="$(mktemp -d)"
  package="vrules-$version-$triple"
  package_dir="$stage/$package"
  mkdir -p \
    "$package_dir/cache" \
    "$package_dir/components" \
    "$package_dir/data" \
    "$package_dir/models" \
    "$package_dir/rules"
  install -m 0755 "$shim" "$package_dir/vrules-shim"
  install -m 0644 "$components"/*.wasm "$package_dir/components/"
  install -m 0644 release/vrules-components.json "$package_dir/"
  install -m 0755 release/fetch-model.sh "$package_dir/"
  install -m 0644 release/pins.env "$package_dir/"
  cp -a shared-rules "$package_dir/rules/"
  (
    cd "$package_dir/rules"
    git init -q -b main
    git config user.name vrules
    git config user.email release@vrules.invalid
    git add shared-rules
    git commit -q -m "rules baseline"
  )
  cp LICENSE-APACHE LICENSE-MIT "$package_dir/" 2>/dev/null || true
  cat > "$package_dir/README" <<EOF
vrules $version ($triple)

1. Fetch and verify EmbeddingGemma:
   VRULES_MODEL_DIR="\$PWD/models" ./fetch-model.sh
2. Run MCP over stdio:
   ./vrules-shim
   To use another GGUF embedding model:
   ./vrules-shim --embedding-model /path/to/model.gguf
3. Run the optional admin HTTP/WebSocket daemon:
   ./vrules-shim --daemon

vrules-components.json selects independently replaceable WASI components.
vrules-gcp.wasm is included but remains disabled until it is added to the
manifest with a Google Cloud project and guest-owned credentials configuration.
EOF
  tar -C "$stage" "${tar_args[@]}" -cf "$dist/$package.$extension" "$package"
  rm -rf "$stage"
done

(
  cd "$dist"
  if command -v sha256sum >/dev/null; then
    sha256sum ./* > SHA256SUMS
  else
    shasum -a 256 ./* > SHA256SUMS
  fi
)
printf 'release artifacts: %s\n' "$dist"
