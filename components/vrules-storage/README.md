# vrules-storage

`vrules-storage` is a WASI component providing latency-first append-only storage and vector index capabilities.

## Responsibility

- Stores append-only audit events, governed memory records, and response cache records on disk (`/data`).
- Maintains model-revision-aware vector search indexes over memory items using `em-log-n`.
- Enforces memory write, update, supersession, and tombstone lifecycle semantics so history remains fully reconstructable.
- Ensures vector searches never cross incompatible embedding model revisions.

## Interfaces

Exported WASI World: `plugin-component` (`wit/vrules.wit`)
- Exports interface `plugin` (`initialize`, `invoke`).
- Imports interface `host` (`invoke`, `get-embedding-info`, `embed`, `http`, `log`).

## Supported Operations

Invocations through `plugin.invoke(operation, payload)`:
- `audit_write` / `audit_read`: Records and queries execution audit logs.
- `memory_write` / `memory_update` / `memory_delete`: Governed memory mutations.
- `memory_search`: Vector similarity search over active memory items with model identity validation.
