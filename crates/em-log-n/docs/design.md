# Design

em-log-n is the storage and retrieval substrate behind vector-rules's audit log,
organizational memory, and edge-distributed embedding cache. It can be used as a
standalone library, but its shape is optimized for the same problem vector-rules has to
solve: write now, query immediately, and reuse semantic work instead of
recomputing it.

## Problem shape

The target workload wants all of the following at once:

- **newest-first scans** with no planner in the way
- **ANN search over caller-supplied vectors**
- **sync write -> visible** semantics
- **content-addressed embedding reuse**
- **optional cold export / restore** without turning the live path into a server

That combination matters for audit logs, tool traces, UI events, and any other
domain where the next read often happens immediately after the write.

## Core architecture

| Layer | Choice | Why it exists |
|---|---|---|
| Ordered KV | `fjall` | Gives the live row store and newest-first iteration. |
| ANN | `usearch` | Gives low-latency vector search over named indexes. |
| Canonicalization | `vrules-canon` | Collapses recurring variants before cache or embed work. |
| Embedder abstraction | `Embedder`, `PooledEmbedder` | Keeps model execution outside the storage core while preserving a sync API. |
| Embedding cache | `EmbedCache`, `CachingEmbedder`, `CanonicalizingEmbedder` | Turns repeated semantic work into a content-addressed lookup. |
| Cold tier | `LocalDirStore`, `CloudStore` | Publishes immutable generations for backup, restore, and cheap colder storage. |

The vector-rules component host owns model lifecycle. em-log-n remains an
in-process library for applications that need shards, ANN indexes, and embedding
cache wrappers.

## Keys and why they look the way they do

em-log-n uses fixed-layout byte keys because key layout is the hot path.

### `InverseTimestampKey` for live rows

```text
| 8 bytes reverse_ts_be | blake3(text)[..N] | optional tiebreaker |
```

- `reverse_ts_be = (u64::MAX - ts_nanos).to_be_bytes()`
- lexicographic order is logical newest-first order
- the hash suffix keeps duplicate texts stable and compact

No varints or length prefixes are used here because they would break the ordering
guarantee that makes scans cheap.

### `ContentHashKey` for the embedding cache

```text
| 8 bytes: blake3(model_id)[..8] | 16 bytes: blake3(text)[..16] |
```

This key is about namespace safety, not ordering. The model prefix prevents stale
vectors from leaking across model revisions, and canonicalization namespaces can be
folded in when a caller wants cache keys to track canonicalizer identity as well.

## Shards

A `Shard` is the live unit of storage.

```text
caller facts / events / payloads
          |
          +--> row key
          +--> value bytes
          +--> zero or more vectors
                    |
                    v
      +---------------------------+
      |       Shard               |
      |  fjall KV + usearch ANN   |
      +---------------------------+
          |    |    |     |
          |    |    |     +--> ann_multi(...)
          |    |    +--------> ann_in_window(...)
          |    +-------------> ann(...)
          +------------------> scan(...) / get(...)
```

Key properties:

- **Sync write -> visible.** Once `put` returns, the next read sees the row.
- **Per-domain isolation.** Domains such as `ui`, `system-core`, or `vrules-audit`
  can each own their own shard layout and indexes.
- **Multiple named indexes.** A shard can hold separate text, image, or fused
  indexes with different dimensions and metrics.
- **Caller-owned vector semantics.** em-log-n stores and searches vectors; it does
  not prescribe how the caller produced them.

## Embeddings are external to the storage core

The current architecture deliberately separates storage from model execution.

### Host-supplied embedder

The caller implements `Embedder` using its model runtime. In vector-rules the
Wasmtime host obtains real vectors from the wllama component. Model identity
includes the verified GGUF revision so cache namespaces remain stable.

### `CachingEmbedder`

`CachingEmbedder` wraps any `Embedder` and turns repeated strings into
content-addressed hits. The cache value layout is:

```text
| 4 bytes BE: dim | 4 * dim bytes: f32 vector |
```

Vectors are stored as **lossless f32**, not quantized. If the stored dimension no
longer matches the current embedder, the entry degrades to a miss instead of
serving the wrong shape.

### `CanonicalizingEmbedder`

`CanonicalizingEmbedder` sits in front of another embedder and canonicalizes the
text first. This is the bridge between vrules-canon and the embedding cache: one
log template or structured payload family can collapse to one semantic key.

### `PooledEmbedder`

`PooledEmbedder` fans work out across multiple pre-constructed workers. This is the
current answer to data-parallel embedding: the caller owns worker construction and
device placement, while em-log-n owns the scheduling and ordering guarantees.

## Cold tier

The cold tier publishes immutable generations and a tiny manifest:

```text
<root>/<domain>/gen/N/kv.snapshot
<root>/<domain>/gen/N/idx.<name>.usearch
<root>/<domain>/manifest
```

Important semantics:

- publish is two-step: write blobs, then publish the manifest
- GC is two-step: rewrite the manifest, then delete old blobs
- restore re-reads the manifest to distinguish genuine corruption from a benign
  GC race

See [cold-tier.md](cold-tier.md) for the exact rules. `CloudStore` is already
shipped for S3, Azure Blob, and GCS; `LocalDirStore` remains the simplest local
backend.

## How vector-rules uses em-log-n

Within the vector-rules workspace, em-log-n supports three related jobs:

1. **Audit storage** - append-only records plus semantic search over execution
   history.
2. **Organizational memory** - searchable event and knowledge shards that rules
   can recall conditionally.
3. **Embedding reuse** - local content-addressed caches keyed by model identity.

That is why the crate is intentionally low level: the higher-level runtime policy
belongs in vrules-core and its host components, while em-log-n remains a reusable
native substrate.

## What em-log-n is not

- Not a database server
- Not a SQL engine
- Not a columnar live store
- Not a complete distributed database
- Not a model host by itself

It is an embedded library for fast write-visible storage, ANN lookup, and
semantic reuse.
