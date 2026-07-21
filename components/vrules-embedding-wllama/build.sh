#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${1:-$root/target/vrules-components}"
wasi_sdk="${WASI_SDK_PATH:-$root/target/wasi-sdk}"
wit_bindgen="${WIT_BINDGEN:-$root/target/wasi-tools/bin/wit-bindgen}"
wasm_tools="${WASM_TOOLS:-$root/target/wasi-tools/bin/wasm-tools}"
adapter="${WASI_REACTOR_ADAPTER:-$root/target/wasi-tools/share/wasi_snapshot_preview1.reactor.wasm}"
bindings="$root/target/wllama-wit-bindings"
build="$root/target/wllama-wasi-build"
core="$build/vrules-embedding-wllama.wasm"
component="$out_dir/vrules-embedding-wllama.wasm"

for executable in "$wit_bindgen" "$wasm_tools" "$wasi_sdk/bin/clang++"; do
  [[ -x "$executable" ]] || {
    echo "embedding build tool is missing or not executable: $executable" >&2
    exit 1
  }
done
[[ -f "$adapter" ]] || { echo "WASI reactor adapter is missing: $adapter" >&2; exit 1; }
[[ -f "$root/vendor/wllama/llama.cpp/CMakeLists.txt" ]] || {
  echo "initialize the pinned submodules with: git submodule update --init --recursive" >&2
  exit 1
}

mkdir -p "$out_dir" "$bindings"
"$wit_bindgen" c "$root/wit" --world embedding-component --out-dir "$bindings"
cmake -E remove_directory "$build"
cmake \
  -S "$root/components/vrules-embedding-wllama" \
  -B "$build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_TOOLCHAIN_FILE="$wasi_sdk/share/cmake/wasi-sdk-p1.cmake" \
  -DWLLAMA_LLAMA_CPP_DIR="$root/vendor/wllama/llama.cpp" \
  -DVRULES_WIT_BINDINGS_DIR="$bindings"
cmake --build "$build" --parallel "${VRULES_BUILD_JOBS:-4}"
"$wasm_tools" component new "$core" --adapt "$adapter" -o "$component"
"$wasm_tools" validate "$component"
"$wasm_tools" print "$core" > "$build/vrules-embedding-wllama.wat"
grep -q "try_table" "$build/vrules-embedding-wllama.wat" || {
  echo "embedding core does not use standard WebAssembly exceptions" >&2
  exit 1
}
printf '%s\n' "$component"
