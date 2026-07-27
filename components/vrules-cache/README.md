# vrules-cache

`vrules-cache` is a persistent, content-addressed embedding vector accelerator component.

## Responsibility

- Stores computed f32 embedding vectors indexed by a content digest: `hash(model_revision + canon_namespace + text)`.
- Eliminates redundant embedding inference passes across rule evaluation, memory recall, and MCP routing.
- Supports epoch invalidation and append-only cache persistence on disk (`/cache`).
- Serves as the local storage layer for the `vrules-rest` two-tier embedding cache network.

## Interfaces

Exported WASI World: `plugin-component` (`wit/vrules.wit`)
- Exports interface `plugin` (`initialize`, `invoke`).
- Imports interface `host` (`invoke`, `get-embedding-info`, `embed`, `http`, `log`).

## Supported Operations

Invocations through `plugin.invoke(operation, payload)`:
- `cache_get`: Retrieves a cached vector by model revision, canonical namespace, and text hash.
- `cache_put`: Stores a locally computed vector.
- `cache_stats`: Reports hit/miss counts and cache store generation.
