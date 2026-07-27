# vrules-embedding-wllama

`vrules-embedding-wllama` is a WASI component providing local GGUF embedding inference powered by llama.cpp / wllama.

## Responsibility

- Loads embedding-capable GGUF models directly (defaulting to EmbeddingGemma 300M).
- Executes single-threaded CPU or WebGPU wllama inference with SIMD128.
- Returns L2-normalized vector embeddings to the host and rule engine.
- Reports model identity, SHA-256 digest, and output dimensions through the typed ABI to prevent vector space contamination.

## Build Requirements

- C++ toolchain with `wasi-sdk` 33.
- `wit-bindgen` 0.58.0 and `wasm-tools` 1.251.0.
- Wasmtime Preview 1 reactor adapter (`wasi_snapshot_preview1.reactor.wasm`).

Build script: `./release/build-components.sh`
