# vrules-admin

`vrules-admin` is a WASI component that exposes administrative, diagnostic, and governance RPC endpoints for the daemon.

## Responsibility

- Powers the embedded Svelte admin PWA (`apps/console`) and `/vrules-rest/v1` routes.
- Handles rule inspection, what-if evaluation, A/B ruleset comparisons, and promotion workflows.
- Exposes memory inspection, embedding diagnostics, and audit log queries.

## Interfaces

Exported WASI World: `plugin-component` (`wit/vrules.wit`)
- Exports interface `plugin` (`initialize`, `invoke`).
- Imports interface `host` (`invoke`, `get-embedding-info`, `embed`, `http`, `log`).

## Supported Operations

Invocations through `plugin.invoke(operation, payload)`:
- `rules_get` / `rules_list` / `rules_diff`: Rule governance and Git revision inspection.
- `whatif` / `compare`: Runs candidate rule evaluations side-by-side.
- `memory_list` / `memory_inspect`: Inspects governed organizational memory.
- `embedding_diagnostics`: Reports active embedder status, SHA-256 digest, and cache metrics.
