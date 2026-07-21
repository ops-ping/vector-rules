# em-log-n

**An embedded, latency-first searchable log and memory store for Rust.** Record
every event your program cares about (system logs, UI events, LLM tool calls,
PII probes), then walk them newest-first *or* reason over them with arbitrary
vector arithmetic - sync, in-process, no SQL planner in the way.

Inside vector-rules, `em-log-n` supplies storage and cache primitives used by
native libraries. The WASI runtime uses its append-only storage component for
audited organizational memory.

> **Status:** v0.0.1 — early. The on-disk format and core API are
> stabilizing but may change before 1.0. Production use is not advised
> yet; experimentation, feedback, and contribution very much are.

License: [Apache-2.0](LICENSE).

## Why this exists

A typical program needs both of these, fast:

- **Scan recent events** — "what just happened in this domain in the
  last N seconds?" — and
- **Reason over events with embeddings** — "find me the rows most
  similar to this composed query vector", where the caller controls the
  vector (so `king − man + woman` is just a vector you computed).

Existing engines do one well but not the other:

| Engine | Newest-first native? | ANN with arbitrary vectors? | Sync write→visible? |
|---|---|---|---|
| LanceDB / Lance MemWAL | via SQL `ORDER BY`, planner-driven | ✅ | ⚠️ columnar fragment flush |
| Qdrant Edge | ❌ no ordered range scan | ✅ | ✅ |
| fjall / redb alone | ✅ | ❌ | ✅ |
| usearch alone | n/a | ✅ | ✅ |
| **em-log-n** | ✅ native LSM order over reverse-ts keys | ✅ | ✅ |

em-log-n combines fjall (pure-Rust LSM) + usearch (HNSW ANN) + a
shale-style object-store cold tier into one library aimed at the
searchable-log shape. See [docs/design.md](docs/design.md) for the
rationale and alternatives considered.

## LLMOps fit

LLMs are strongest when they can read context, summarize history, compare examples,
and propose fixes. em-log-n gives that work a local evidence store: newest-first
scans for recent behavior, ANN search for semantically related records, and immediate
write visibility for verification loops. In vector-rules, an LLM can use this substrate to
debug rule failures, review audit history, generate regression cases, and prepare
management notes while deterministic rules remain the production control path.

## Quick start

```toml
# Cargo.toml
[dependencies]
em-log-n = { version = "0.0.1", features = ["fjall-backend", "usearch-backend", "codec-prost"] }
```

```rust
use em_log_n::key::KeyBuilder;
use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};

let spec = ShardSpec {
    domain: DomainId::new("ui").unwrap(),
    indexes: vec![IndexSpec {
        name: "text".into(),
        dim: 768,
        metric: Metric::Cosine,
    }],
};
let shard = Shard::open(spec, "/var/lib/myapp/em-log-n/ui").unwrap();

// sync write — immediately visible to the next read / ANN / scan
let key = KeyBuilder::new(ts_nanos, event_text.as_bytes()).build();
shard.put(&key, value_bytes, &[("text", &text_embedding)]).unwrap();

// newest-first walk
let recent = shard.scan(50).unwrap();

// recent + relevant (ANN restricted to a time window)
let hits = shard.ann_in_window("text", &qvec, 10, t_lo, t_hi).unwrap();
```

## Concepts

- **Shard** — one fjall keyspace + N usearch indexes. One shard per
  *domain* (e.g. `system-core`, `ui`, `llm-traces`).
- **Row key** — `reverse_ts_be (8B) || blake3(text)[..16] || optional
  tiebreaker`. Lex order = logical newest-first. Property-tested.
- **Index** — a named usearch index inside a shard, with its own dim
  and metric (built-in or caller-supplied). A shard can host text +
  image + geo + fused multi-modal at once.
- **Cold tier** — periodic atomic publish of immutable generation
  blobs + a tiny manifest, shale-style, to any `ObjectStore` impl
  (in-memory + local FS via `LocalDirStore`; **S3, Azure Blob, GCS
  via `CloudStore`** behind the `coldtier-cloud` feature, built on
  Apache Arrow's `object_store` crate).
- **Embedding cache** — content-addressed cache (`blake3(model_id) ||
  blake3(text)`) backed by a dedicated fjall keyspace. Vectors stored
  as lossless f32, never quantized. A hit is ~1 µs; misses fall through to the
  inner embedder. Wrap any `Embedder` with
  [`CachingEmbedder`](src/embed_cache.rs).
- **Codec** — pluggable value serialization. Defaults: `prost` for
  durable storage, `rkyv` for zero-copy hot-path decode.
- **Retention policy** — pluggable, compaction-time only. Default keeps
  everything forever.

## API tour

```rust
use em_log_n::shard::{Combine, Shard};

// arbitrary query vectors (the "king − man + woman = queen" pattern)
let q = compose_query_vector(king, man, woman);
let hits = shard.ann("text", &q, 10)?;

// multi-modal intersection — "similar images AND similar text AND nearby"
let hits = shard.ann_multi(
    &[("text", &q_text, 50), ("image", &q_image, 50), ("geo", &q_geo, 100)],
    Combine::IntersectSum,
    10,
)?;

// caller-supplied distance — usearch's "JIT-UDF" pattern, but as a regular Rust closure
shard.register_custom_metric("fused", |a: &[f32], b: &[f32]| -> f32 {
    // your composite metric here
    0.0
})?;

// cold-tier publish + restore — works against local FS or any cloud
use em_log_n::coldtier::{publish_generation, restore_into, gc, LocalDirStore, CloudStore};
let store = LocalDirStore::open("/var/lib/myapp/em-log-n-coldtier")?;
publish_generation(&shard, &store, "ui")?;

// Or publish to S3 (Azure / GCS / R2 / LocalStack all similar):
let cloud = CloudStore::s3("my-bucket", "us-east-1", "em-log-n/prod")?;
publish_generation(&shard, &cloud, "ui")?;
gc(&shard, &cloud, "ui", 3)?;          // keep the most recent 3 generations
restore_into(&fresh_shard, &cloud, "ui")?;

// embedding cache — repeated strings bypass the model runtime
use em_log_n::embed_cache::{EmbedCache, CachingEmbedder};
let cache = EmbedCache::open("/var/lib/myapp/em-log-n-cache", "my-model-revision", 768)?;
let cached = CachingEmbedder::new(my_embedder, cache)?;
let v = cached.embed("user clicked button")?;   // miss → embed + cache
let v = cached.embed("user clicked button")?;   // hit → storage lookup
```

## Performance

See [docs/benchmarks.md](docs/benchmarks.md) for full numbers and
methodology. Headline (5k rows, dim 768, container CPU):

| op | p50 |
|---|---:|
| `put` (sync, with vector add + fsync) | 555 µs |
| `scan(10)` | 2 µs |
| `ann(k=10)` | 487 µs |
| `ann_in_window(k=10, 20% window)` | 516 µs |
| **embed-cache hit** | **1 µs** |

End-to-end model latency is intentionally excluded because it depends on the host
component, configured model, hardware, batching, and placement. The useful
storage-level result is that repeated strings become local cache reads instead of
new model evaluations.

## Documentation

- [docs/design.md](docs/design.md) — architecture and substrate choices.
- [docs/hard-constraints.md](docs/hard-constraints.md) — non-negotiable
  design rules.
- [docs/cold-tier.md](docs/cold-tier.md) — object-store cold-tier model.
- [docs/embedding-tuning.md](docs/embedding-tuning.md) — cache, canonicalization,
  batching, and host-worker composition.
- [docs/benchmarks.md](docs/benchmarks.md) — numbers + methodology.
- [docs/roadmap.md](docs/roadmap.md) — what's next.
- [CHANGELOG.md](CHANGELOG.md) — release history.
- [CONTRIBUTING.md](CONTRIBUTING.md) — how to help.

## Cargo features

| Feature | Pulls in | Enables |
|---|---|---|
| (none, default) | — | trait scaffolding only; build is tiny |
| `fjall-backend` | fjall | the ordered-KV backend on `Shard` |
| `usearch-backend` | usearch | the ANN backend on `Shard` |
| `codec-prost` | prost | protobuf value codec |
| `codec-rkyv` | rkyv | zero-copy value codec |
| `coldtier-cloud` | object_store, tokio, futures, url | `CloudStore` (S3 / Azure / GCS cold-tier backend) |
| `full` | all of the above | the configuration tests use |

## Build, test, bench

```sh
# Light build — trait scaffolding only, no C++/CMake (default features = none)
cargo build --release

# Tests build with `full` automatically (via a self dev-dependency) — no flags needed
cargo test

# Ship the full backend stack explicitly
cargo build --release --features full

# storage microbench
cargo run --release --features fjall-backend,usearch-backend --example storage_bench
```

MSRV: 1.90 (driven by fjall 3.1.5).

Every native dep is pinned to an exact version (see `Cargo.toml`). The default
build stays light; enabling `full` pulls in the complete storage stack. Model
execution remains external to this crate and arrives through the `Embedder` trait.

## Acknowledgements

- [fjall](https://github.com/fjall-rs/fjall) — pure-Rust LSM storage.
- [usearch](https://github.com/unum-cloud/usearch) — HNSW + flexible
  metrics + the JIT-UDF pattern that inspired the custom-metric API.
- [llama.cpp](https://github.com/ggml-org/llama.cpp) — the GGUF runtime used by
  vector-rules's separate wllama embedding component.
- [EmbeddingGemma](https://huggingface.co/google/embeddinggemma-300M) —
  the default vector-rules model; other GGUF embedding models can use the same
  `Embedder` and cache contracts.
- [blake3](https://github.com/BLAKE3-team/BLAKE3) — content hash in the
  row key.
- [Apache Arrow `object_store`](https://crates.io/crates/object_store)
  crate — the recommended substrate for the upcoming in-process cloud
  cold-tier backend.

The cold-tier model is derived from a prior internal prototype (shale);
the per-segment ANN sub-index idea on the roadmap is from the
[NEXT paper (SIGMOD 2025)](https://github.com/JC-Shi/NEXT-A-New-Secondary-Index-Framework-for-LSM-based-Data-Storage).
