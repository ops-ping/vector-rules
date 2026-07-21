# Contributing to vector-rules

The repository is a Cargo workspace with a Svelte/Vite console, Git-governed rule
data, a typed WIT ABI, and a pinned wllama submodule.

## Dependency convention

Third-party versions are pinned with `=` in the root
`[workspace.dependencies]`. Member crates use `{ workspace = true }`.
The root `[patch.crates-io]` resolves `rust-rule-engine` from the public
compatibility fork, and `Cargo.lock` pins the exact commit. A temporary sibling
path override may be used for coordinated local development but is not
committed.

## rust-rule-engine fork consistency

The fork remains compatible with the originating rust-rule-engine project
wherever possible. Treat it as an upstream engine dependency, not as a
vrules-specific engine implementation.

- Preserve upstream public types, GRL syntax, feature boundaries, and evaluator
  semantics.
- Put generally useful parser, error-propagation, streaming, state, and numeric
  fixes in the fork in a form suitable for upstream adoption.
- Add vector, canonicalization, address, and product behavior through the
  engine's existing function and action registration APIs.
- Do not add a vrules-only rule format, expression tree, evaluator, session
  abstraction, or state model.
- Keep an unavoidable deviation narrow, documented, and covered by a focused
  compatibility test.
- Compare fork changes with the originating release before updating the pinned
  dependency so upstream improvements can roll forward without semantic
  conflict.

Initialize the embedding sources after cloning:

```sh
git submodule update --init --recursive
```

## Build

The native workspace and Rust WASI components use Rust 1.94:

```sh
cargo build --workspace
cargo build --target wasm32-wasip2 \
  -p vrules-runtime-component \
  -p vrules-storage-component \
  -p vrules-rules-component \
  -p vrules-admin-component \
  -p vrules-gcp-component
```

The real embedding component requires:

- wasi-sdk 33 at `target/wasi-sdk` or `WASI_SDK_PATH`
- `wit-bindgen` 0.59.0 at `target/wasi-tools/bin/wit-bindgen` or `WIT_BINDGEN`
- `wasm-tools` 1.253.0 at `target/wasi-tools/bin/wasm-tools` or `WASM_TOOLS`
- the Wasmtime Preview 1 reactor adapter at
  `target/wasi-tools/share/wasi_snapshot_preview1.reactor.wasm` or
  `WASI_REACTOR_ADAPTER`

```sh
./release/build-components.sh
```

The script regenerates C bindings from `wit/`, clean-builds the pinned llama.cpp
core with wasi-sdk, emits standard WebAssembly exceptions, componentizes the
reactor, and validates the result.

## Run

Use `release/vrules-components.json` from an extracted release package. For a
development tree, point `--manifest` at a manifest whose component, model, rules,
and data paths are available through its preopens.

```sh
vrules-shim --manifest path/to/vrules-components.json
vrules-shim --manifest path/to/vrules-components.json --daemon
vrules-shim --manifest path/to/vrules-components.json \
  --embedding-model /path/to/embedding-model.gguf
```

The first command is newline-delimited MCP JSON-RPC over stdio. The second serves
`/health`, `/vrules-rest/v1`, `/mcp`, and the embedded PWA. `--embedding-model` mounts a
local GGUF read-only, computes its SHA-256 identity, and overrides the manifest's
default EmbeddingGemma model for that run.

## Embedding integrity

Production vector functions receive real model-produced vectors with validated
model identity and dimensions. Do not substitute hash-derived, random, zero, or
otherwise synthetic embeddings to make a test or host path pass. Tests that do
not exercise semantic behavior use pure GRL; semantic integration checks use
the configured embedding host.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
./scripts/ci-check.sh
```

The console is a non-JVM Vite project:

```sh
cd apps/console
npm ci
npm run build
```

## Documentation

Documentation describes the current state in present tense. Architectural changes
update the WIT contract, component manifest, build scripts, runtime documentation,
and the relevant behavior checks together.
