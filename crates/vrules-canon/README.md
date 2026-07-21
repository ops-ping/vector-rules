# vrules-canon

Deterministic, pluggable **canonicalization** for vector-rules's rules-first runtime. It sits
in front of embeddings, collapses recurring variants, and gives the platform stable
namespaces for cache keys, audits, and class-of-input reasoning.

Canonicalizing the *variable* parts of a line away (`User 42 login` ->
`User <*> login`) collapses every recurring variant onto one cache key, so a
content-addressed embedding cache pays the embed once instead of per-variant. That is
what lets vector-rules reuse semantic work safely across sessions, machines, and higher cache
tiers without turning policy into heuristics.

## Layers

Two layers, feature-gated so the canon core stays pure (zero-dependency):

| Layer | Feature | Pulls | Provides |
|-------|---------|-------|----------|
| canon core | *(default)* | — | `Canonicalizer` strategies, `SimHash64`, `fnv1a_64` |
| structured | `json` | `serde_json` | `JsonHybrid`, `SchemaFingerprint` |
| audit | `audit` | `serde` | `ExecutionRecord`, `ExecCanonicalizer` |

## Canon core

A `Canonicalizer` is a **pure, deterministic, stateless** function — a hard
requirement, because a content-addressed cache is silently poisoned the moment
canonicalization depends on process state or ingest order. Strategy is selected
per type rather than by one global auto-detect:

- **`LogMask`** — stateless masking of ints, floats, hex blobs, UUIDs, IPv4/IPv6,
  ISO-8601 timestamps, and paths; the surrounding words (the meaning) are kept.
- **`JsonHybrid`** (`json`) — keep string values (the signal), mask numbers/ids,
  sort keys.
- **`SchemaFingerprint`** (`json`) — strip all values to type tokens; for dedup
  *detection* only, never as the thing you embed.
- **`Identity`** — no-op passthrough (A/B baseline, or opaque types).

Each strategy reports a stable `id` and `version` so cache keys namespace by both
and never collide across strategies or a rule change.

`SimHash64` + `NearDupChecker` give cheap Hamming-distance near-duplicate
detection — the pre-filter that reuses an embedding for text canonicalization
didn't make byte-identical.

```rust
use vrules_canon::{Canonicalizer, LogMask};

let a = LogMask.canon("User 42 login from 10.0.0.1");
let b = LogMask.canon("User 9999 login from 192.168.1.7");
assert_eq!(a.canonical, "User <*> login from <*>");
assert_eq!(a.id, b.id); // variants collapse to one template id
```

## LLMOps fit

Canonicalization is where LLMs can help propose labels, examples, and near-duplicate
cases without being trusted to decide production behavior. An LLM can suggest pattern
sets, explain why two messages should collapse, or generate regression cases for a PR;
`vrules-canon` keeps the actual result pure, deterministic, versioned, and safe to
use in cache keys, audits, tests, and rule debugging.

## Audit (`audit` feature)

`ExecutionRecord` is the serde-serializable log line written to the em-log-n sink
for every capability call. `ExecCanonicalizer` derives that record's cache/dedup
key from the **request + backend only** (never the origin identity), so
semantically-equivalent requests across sessions collapse to one key and share a
cache entry — expressed as a `Canonicalizer` over the core.

```rust
use vrules_canon::ExecCanonicalizer;

let c = ExecCanonicalizer;
// Whitespace-only differences collapse; tool/backend/effort diverge the key.
assert_eq!(
    c.key("web_ground", "gemini-backend", "low", "rust async   runtime").id,
    c.key("web_ground", "gemini-backend", "low", "  rust async runtime\n").id,
);
```

## License

`MIT OR Apache-2.0`.
