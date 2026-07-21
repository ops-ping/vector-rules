# vrules-shim

`vrules-shim` is the sole native vrules executable. It owns transports and a
Wasmtime engine; rules, runtime behavior, storage, administration, providers, and
embedding inference remain independently replaceable WASI components.

## Modes

The default mode reads newline-delimited MCP JSON-RPC from stdin and writes
responses to stdout:

```json
{ "command": "vrules-shim" }
```

Daemon mode adds the admin HTTP/WebSocket surfaces:

```sh
vrules-shim --daemon
vrules-shim --daemon --bind 127.0.0.1:8765
```

| Route | Purpose |
|---|---|
| `GET /health` | Liveness |
| `/vrules-rest/v1/...` | REST surface: embedding cache tier, admin, governance, memory |
| `GET /mcp` | MCP WebSocket transport |
| all other paths | Embedded console PWA |

## vrules-rest

`/vrules-rest/v1` is the daemon's REST surface. The embedding tier serves the
content-addressed cache owned by the `cache` plugin component; vector bodies are
lossless little-endian f32 with strong ETags (`"{key}-{generation}"`) and
`If-None-Match` revalidation.

| Route | Meaning |
|---|---|
| `GET /vrules-rest/v1/embeddings/{model}/{canon}/{hash}` | Cacheable immutable lookup by content hash |
| `GET /vrules-rest/v1/embeddings/{model}/{canon}?text=...` | Compute-on-miss for browser/remote readers |
| `POST /vrules-rest/v1/embeddings/{model}/{canon}` | Compute-on-miss with text in the body |
| `PUT /vrules-rest/v1/embeddings/{model}/{canon}/{hash}` | Write-up receiver for a downstream node's vector |
| `POST /vrules-rest/v1/expire` | Bump the cache epoch for rule-driven mass invalidation |
| `GET /vrules-rest/v1/cache/stats` | Cache entry/epoch/segment statistics |

`{model}` is the active model's revision (its GGUF SHA-256); compute-on-miss
under any other model segment answers 409. Every embedding path — internal `v:`
rule probes, memory operations, and these routes — resolves through the same
cache: lookup first, wllama inference on miss, append-only write-back.

Admin, governance, and memory routes dispatch to the admin plugin component:
`/storage/stats`, `/tools/stats`, `/log[?limit&session]`, `/log/search?query&k`,
`/sessions`, `/test/run`, `/rules[?ruleset]`, `/rules/branches`,
`/rules/diff?a&b`, `/rules/compare?a&b`, `/rules/validate`, `/rules/promote`,
`/ab`, `/embedding/info`, `/embedding`, `/whatif/assert`, `/whatif/prove`,
`/memory/search`, `/memory/stats[?memory_id]`, `/memory/{id}/history`,
`/memory` (POST write), `/memory/{id}` (PUT update, DELETE tombstone) — all
under `/vrules-rest/v1`, GET with query parameters or POST/PUT/DELETE with JSON
bodies. Results are returned unwrapped; errors are `{"error"}` with 4xx/5xx.

With `VRULES_REST_UPSTREAM=host:port` the shim joins a cache tier: local misses
pull through the parent's `/vrules-rest/v1` and locally computed vectors are
written back up, with short timeouts and silent fall-back to local inference.

## Manifest

The host reads `vrules-components.json` beside the executable unless
`--manifest PATH` or `VRULES_COMPONENT_MANIFEST` selects another file. Each entry
declares:

- a WebAssembly component path;
- typed initialization configuration;
- explicit filesystem preopens with read/write scope; and
- an HTTP hostname allowlist.

The host validates component IDs against their descriptors and initializes the
embedding component before plugins and the MCP runtime.

## Bring your own embeddings

EmbeddingGemma is selected by the release manifest, not compiled into the host.
Override it with any embedding-capable GGUF supported by the pinned llama.cpp:

```sh
vrules-shim --embedding-model /path/to/model.gguf
vrules-shim --embedding-model /path/to/model.gguf \
  --embedding-model-name "Organization Embeddings"
```

The host computes the model SHA-256, mounts the containing directory read-only at
`/models`, and passes the guest path, model name, and digest to the embedding
component. The model digest and output dimension partition vector caches and
searches. Persistent deployments can express the same configuration directly in
the component manifest; `pooling` may be `mean`, `cls`, or `last` when the GGUF
does not declare pooling metadata.

## Isolation

Components do not open network connections directly. Provider HTTP requests cross
the typed host interface and are rejected unless the destination hostname appears
in that component's allowlist. Filesystem access is limited to manifest preopens.
The GCP guest owns ADC parsing, token exchange, and provider configuration.

## Environment

| Variable | Meaning |
|---|---|
| `VRULES_COMPONENT_MANIFEST` | Component manifest path |
| `VRULES_REST_UPSTREAM` | Optional `host:port` parent cache tier |
| `VRULES_SESSION_ID` / `CLAUDE_CODE_SESSION_ID` | Parent session identity |
| `VRULES_CHILD_SESSION` / `CLAUDE_CODE_CHILD_SESSION` | Optional child identity |
| `VRULES_CONTEXT` / `VRULES_PROFILE` | Optional rule context |

## Build

```sh
cargo build -p vrules-shim
cargo test -p vrules-shim
```

MIT OR Apache-2.0.
