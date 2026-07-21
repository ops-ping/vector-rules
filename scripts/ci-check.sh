#!/usr/bin/env bash
# Run the same gates as .github/workflows/ci.yml, locally, so a red build is
# caught here instead of on GitHub. Wired to `git push` via the committed
# .githooks/pre-push hook (enable once with: git config core.hooksPath .githooks).
#
# Run manually any time:  ./scripts/ci-check.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

step() { printf '\n\033[1;36m==> %s\033[0m\n' "$1"; }

step "rustfmt (cargo fmt --all -- --check)"
cargo fmt --all -- --check

step "workspace graph resolves (cargo metadata)"
cargo metadata --format-version 1 >/dev/null

step "clippy (cargo clippy --workspace --all-targets -- -D warnings)"
cargo clippy --workspace --all-targets -- -D warnings

step "build native workspace crates"
cargo build --workspace \
  --exclude vrules-runtime-component \
  --exclude vrules-storage-component \
  --exclude vrules-rules-component \
  --exclude vrules-admin-component \
  --exclude vrules-gcp-component

step "WASI components"
cargo build --target wasm32-wasip2 \
  -p vrules-runtime-component \
  -p vrules-storage-component \
  -p vrules-rules-component \
  -p vrules-admin-component \
  -p vrules-gcp-component \
  -p vrules-cache-component

step "wasm packages (vrules-wasm + em-log-n-wasm)"
if command -v wasm-pack >/dev/null 2>&1; then
  wasm-pack build crates/vrules-wasm --target web --out-name vrules_wasm --release
  wasm-pack build crates/em-log-n-wasm --target web --out-name em_log_n_wasm --release
else
  echo "ERROR: wasm-pack not found (install: cargo install wasm-pack --locked)" >&2
  exit 1
fi

step "tests (component host + cached real-embedding conformance)"
cargo test -p vrules-canon -p em-log-n -p em-log-n-wasm -p vrules-address-indexer \
  -p vrules-core -p vrules-wasm -p vrules-shim -p vrules-cache-component

printf '\n\033[1;32m✓ local CI checks passed — safe to push\033[0m\n'
