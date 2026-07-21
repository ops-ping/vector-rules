# Changelog

All notable changes to em-log-n are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html);
until 1.0 the API may evolve at any minor-version bump and the on-disk
format may evolve at any major-version bump.

## [Unreleased]

### Added

- **Persistent-context embedder pool.** `LlamaEmbedder` now loads the
  GGUF model once (shared `Arc<LlamaModel>`) and runs a bounded pool of
  worker threads, each owning a reused `LlamaContext` behind a blocking
  work queue — eliminating the per-call ~61 MB context allocation. New
  config-first API `LlamaEmbedder::open(path, LlamaEmbedderConfig)` with
  `pool_size`, `n_threads`, `n_threads_batch`, `n_ctx`, `pooling`,
  `output_dims`, and `device`. A process-global backend `OnceLock` fixes
  a latent "second embedder fails backend init" bug. **(Replaces the old
  `LlamaEmbedder::load(path, n_ctx, dim)` — no back-compat shim.)**
- **Batched embedding.** `Embedder::embed_batch(&[&str])` added with a
  default loop impl; `LlamaEmbedder` overrides it with true
  multi-sequence packing (one decode per chunk). `CachingEmbedder::embed_batch`
  is **cache-aware**: it looks up every item, forwards only the misses
  to one packed `embed_batch`, writes them back, and reassembles in
  caller order.
- **Matryoshka output dimensions.** `output_dims = Some(128/256/512)`
  truncates EmbeddingGemma's 768-d output and L2-renormalizes; `dim()`
  reports the effective dim so `Shard` / `EmbedCache` stay consistent.
- **GPU offload plumbing (wire-complete, pending hardware validation).**
  `Device` enum (`Auto`/`Cpu`/`Gpu { n_layers }`) with runtime detection
  and graceful CPU fallback (a GPU init failure logs and retries on CPU,
  never failing an embed). Opt-in cargo features `embed-gpu-vulkan`,
  `embed-gpu-cuda`, `embed-gpu-metal` (NOT in `full`). The GPU *compute*
  build has not yet been validated on hardware — the development machine
  could not build the Vulkan shaders — so CPU remains the validated
  default; GPU validation is deferred to a GPU-capable machine.
- **`docs/embedding-tuning.md`** — operator guide: the two caches, every
  config knob, pool-vs-threads oversubscription, Matryoshka tradeoffs,
  batching guidance, GPU build prerequisites, and measured sweeps.
- Guarded `tests/embed_llama.rs` (skips when no model present):
  determinism under context reuse, batch≈individual, Matryoshka dim,
  concurrent embeds across a multi-worker pool. Cache-aware-batch +
  `NullEmbedder` default-batch tests added (model-free).

- **Cloud cold-tier backend** (`em_log_n::coldtier::CloudStore`) behind
  new feature `coldtier-cloud`. Built on Apache Arrow's pure-Rust
  [`object_store`](https://crates.io/crates/object_store) crate
  (v0.13.2). Constructors for AWS S3 (`CloudStore::s3`), Azure Blob
  (`azure`), Google Cloud Storage (`gcs`), generic URL parsing
  (`from_url`), and bring-your-own (`from_object_store`) for
  LocalStack / MinIO / R2 / custom endpoints. Bridges Arrow's async
  API to em-log-n's sync `ObjectStore` trait via a small owned
  current-thread tokio runtime, so callers stay sync. The
  `object_store::Error::NotFound` → `Ok(None)` mapping satisfies the
  absent-vs-error contract documented in `docs/cold-tier.md`.
- `tests/coldtier_cloud.rs` cross-validates the publish / restore /
  gc / GC-race scenarios from `tests/coldtier.rs` against `CloudStore`
  over `object_store::memory::InMemory`. Proves the cloud bridge is
  semantics-equivalent to em-log-n's native `InMemoryStore`. 12 new
  tests in total (7 unit + 5 integration); 78 tests passing overall.
- Pinned native deps: object_store=0.13.2, tokio=1.48.0, futures=0.3.31,
  url=2.5.4.
- docs/cold-tier.md: new "Cloud backend" section with provider
  one-liners and rationale.
- docs/roadmap.md: in-process cloud backend marked done.
- README: cloud bullet under Concepts + cloud snippet in API tour +
  new feature row.

- **Pluggable key format** via the new
  [`em_log_n::key::KeyFormat`] trait. Two ships-with implementations:
  `InverseTimestampKey` (the `Shard` row-key encoder) and
  `ContentHashKey` (the `EmbedCache` key encoder). Callers can plug
  their own row layout — the trait uses a GAT so impls can take
  borrowed inputs without allocations.
- **Embedding cache** (`em_log_n::embed_cache`). Content-addressed
  cache backed by a dedicated fjall keyspace. Key =
  `blake3(model_id)[..8] || blake3(text)[..16]`; value =
  `dim(4B) || bf16 bytes`. `EmbedCache::{open,get,put}` for raw
  access; `CachingEmbedder<E>` is a drop-in wrapper that consults the
  cache, falls through to the inner embedder on miss, and writes the
  result best-effort. Identical-text concurrent writes are idempotent.
  Pluggable retention via the existing `RetentionPolicy` trait;
  default `KeepAll`.
- `Store::open_embed_cache(base_dir, model_id, dim)` helper that
  manages the cache lifecycle for a `Store`.
- `storage_bench` example now reports `embed_cache_{hit,miss,put}`
  latencies. Measured ~1 µs per hit on the container bench.

### Changed

- `EmbedCache` now derives its keys via a stored `ContentHashKey`
  instead of an inlined helper. Byte layout is unchanged (regression-
  tested).

## [0.0.1] — initial private release

The first release. Establishes the core API and on-disk format.

### Added

- **Typed row-key encoding** (`em_log_n::key`). Fixed-layout
  `reverse_ts_be (8B) || blake3(text)[..16] || optional tiebreaker`.
  Lexicographic byte order = logical newest-first; proptest-verified.
- **Pluggable `Codec` trait** (`em_log_n::codec`) with two built-in
  impls behind cargo features:
  - `codec-prost` — protobuf via `prost`. Default for cold-tier storage.
    Stable wire format, language-portable, schema-evolving.
  - `codec-rkyv` — zero-copy archive via `rkyv`. Hot-path-friendly.
- **`Embedder` trait** with a `llama-cpp-2`-backed implementation
  (`em_log_n::embed::LlamaEmbedder`) behind feature `embed-llama`.
- **`RetentionPolicy` trait** (`em_log_n::retention`) with `KeepAll`
  default; applied at compaction time only.
- **Functional `Shard`** (`em_log_n::shard`) behind features
  `fjall-backend` + `usearch-backend`. Provides:
  - Sync `put` → immediately visible to subsequent `get` / `scan` /
    `ann`. `fjall::PersistMode::SyncAll` durability on return.
  - `scan(limit)` and `scan_window(t_lo, t_hi, limit)` — native
    newest-first via fjall's `DoubleEndedIterator`.
  - `ann(index_name, qvec, k)` and `ann_in_window` over usearch HNSW.
  - **Multiple named usearch indexes per shard** (e.g. text + image +
    geo + fused) with their own dim / metric.
  - `ann_multi(queries, Combine::IntersectSum | UnionMin, final_k)`
    for cross-index combination (the marketplace pattern).
  - `register_custom_metric(index_name, |a, b| f32)` — caller-supplied
    distance function via usearch's metric-function-pointer hook.
- **`Store`** (`em_log_n::store`) registry over per-domain `ShardSpec`s
  with `open_shard()`.
- **Shale-style object-store cold tier** (`em_log_n::coldtier`):
  - `ObjectStore` trait + `InMemoryStore` + `LocalDirStore`
    (write-tempfile-then-rename atomic PUT).
  - `Manifest` parse / encode (comma-separated u64 generations).
  - `publish_generation(shard, store, domain)` — two-step publish
    (segments first, then atomic manifest).
  - `restore_into(shard, store, domain)` — GC-race-tolerant restore.
  - `gc(shard, store, domain, keep)` — two-step GC (manifest first,
    then segment deletes).
- **48 tests** across 6 integration test files + unit tests, all
  passing on the release pinned dep set.
- **Two benchmark examples**: `storage_bench` and `embed_bench`.
- **Pinned native deps** for reproducible builds: fjall 3.1.5,
  usearch 2.25.3, prost 0.13.5, rkyv 0.8.16, llama-cpp-2 0.1.150,
  blake3 1.8.5, thiserror 2.0.18.
- **MSRV** 1.90.
- Project documentation: README, design, hard-constraints, cold-tier,
  benchmarks, roadmap, contributing.

### Known limitations

- Cold-tier backends are local-only today; in-process S3/Azure/GCS via
  Apache Arrow's `object_store` crate is the v0.1 roadmap item.
- `LlamaEmbedder` allocates a fresh llama.cpp context per call; a
  persistent-context refactor is queued for a ~2–4× warm-short
  throughput gain.
- `ann_in_window` uses oversample + post-filter. For tight windows the
  NEXT-style per-segment usearch sub-index work on the roadmap is what
  removes the overhead.

[Unreleased]: https://github.com/ops-ping/em-log-n/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/ops-ping/em-log-n/releases/tag/v0.0.1
