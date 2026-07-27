# vrules-mcp

`vrules-mcp` is a WASI component that implements the Model Context Protocol (MCP) server interface for `vector-rules`.

## Responsibility

- Accepts MCP JSON-RPC messages from host transports (stdio or WebSocket).
- Evaluates tool exposure rules via the rule harness plugin (`vrules-harness`) to determine which tools are exposed to an AI assistant.
- Evaluates rule-driven routing policies (`shared-rules/proxy/routing.grl`) to dispatch assistant tool invocations to external providers (such as web grounding or search).
- Intercepts assistant activity to append searchable audit logs and governed memory events to storage (`vrules-storage`).

## Interfaces

Exported WASI World: `runtime-component` (`wit/vrules.wit`)
- Exports interface `runtime` (`initialize`, `mcp`).
- Imports interface `host` (`invoke`, `get-embedding-info`, `embed`, `http`, `log`).

## Configuration

Specified in `release/vrules-components.json`:
- `rules_plugin`: ID of the rules harness component (default: `"rules"`).
- `storage_plugin`: ID of the append-only storage component (default: `"storage"`).
- `cache_ttl_secs`: Time-to-live for cached response events (default: `300`).
