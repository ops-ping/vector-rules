# vrules-console

Reference operator console for vector-rules: a Vite + Svelte 5 PWA served by
`vrules-shim` in daemon mode.
It shows how the same rules can be inspected, tested, and partially executed in the
browser while the runtime WASI component remains authoritative.

The console talks to the daemon over `/vrules-rest/v1` (management and embedding
REST) and `/mcp` (MCP over WebSocket). Through `vrules-wasm` it can
validate GRL, run forward evaluation, benchmark isolated sequential event
evaluation, prove goals, and execute browser-side layered vector flows
(`s_cosine`, `s_contrast`, artifact-backed fraud triage) against real
embeddings returned by the configured
embedding component.

## LLMOps fit

The console is the reference surface for human-and-LLM rule operations. An LLM can
help draft rules, explain a PR diff, generate what-if facts, compare candidate
revisions, read traces, and summarize audit evidence while the console calls the
same `/vrules-rest/v1` and `vrules-wasm` paths used by the rest of the stack. The
result is assistance around authoring, testing, verification, debugging, and
management without moving runtime authority out of the reviewed ruleset.

## Layout

- `src/lib/panels/` - reusable operational and capability panels.
- `src/lib/pages/Examples.svelte` and `src/lib/examples/` - hash-routed,
  isolated demonstrations and their fixtures, including address verification,
  semantic rules, streaming, and proof.
- `dist/` - the built app that `vrules-shim` embeds into its binary via `include_dir!`.
- `build:wasm` - builds `../../crates/vrules-wasm` with `wasm-pack` and imports the
  generated package via the `vrules-wasm` alias. No model runs in the browser.

The console is a reference application. Any web app can embed the same
`vrules-wasm` engine directly and use the same `/vrules-rest/v1` surface.

Operational views remain at top-level routes. Demonstrations are intentionally
separate under `#/examples/address`, `#/examples/semantic`,
`#/examples/streaming`, and `#/examples/prove`.

### Address reference implementation

Address verification is an end-to-end reference business workflow, not a
framework feature or admin-console capability. It exists to prove that the same
generic rules can coordinate arbitrary structured and unstructured inputs,
native functions, embeddings, local indexes, reference evidence, and editable
organizational policy.

The PWA executes the workflow through `vrules-wasm`. Its facts and shared GRL
rules remain host-neutral, which makes the workflow a conformance case for
additional native, streaming, and batch adapters. Address-specific fixtures
remain under `src/lib/examples/`; the admin component and generic engine
contracts do not own the business domain.

## Build

```sh
npm ci
npm run build        # prebuild runs build:wasm, then vite build -> dist/
```

`dist/` is committed because `vrules-shim` embeds it at compile time. Refresh it after
changing anything under `src/`, then rebuild the daemon.

## Dev

```sh
npm run dev          # predev runs build:wasm; Vite proxies /vrules-rest and /health
                     # to a daemon on 127.0.0.1:8765
```

## Requirements

- `wasm-pack` (`cargo install wasm-pack`) and the `wasm32-unknown-unknown` target, to
  build `../../crates/vrules-wasm` locally.
