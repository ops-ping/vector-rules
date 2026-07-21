# Embedding cache tuning

em-log-n accepts a host-supplied `Embedder` and provides
`CachingEmbedder`, `CanonicalizingEmbedder`, and `PooledEmbedder`.

## Recommended layering

```text
CanonicalizingEmbedder<CachingEmbedder<ComponentEmbedder>>
```

For independent workers in the same vector space:

```text
PooledEmbedder([CanonicalizingEmbedder<CachingEmbedder<ComponentEmbedder>>, ...])
```

Each layer has one responsibility:

- the host adapter obtains real model vectors;
- `CachingEmbedder` avoids recomputing repeated strings;
- `CanonicalizingEmbedder` collapses recurring variants onto one cache key; and
- `PooledEmbedder` spreads misses across equivalent workers.

## Cache by model and canonicalizer

`EmbedCache` keys include model identity. Canonicalized and raw-text entries use
different namespaces, and each canonicalizer namespace includes its id and
version. A model or canonicalizer change therefore produces misses rather than
serving stale vectors.

## Batch true misses

`CachingEmbedder::embed_batch` checks each item, forwards true misses through the
inner embedder, and restores caller order. Batching helps ingestion, replay,
backfill, and re-indexing. A single isolated miss remains a latency-sensitive
model call.

## Pool equivalent workers

Every worker in a `PooledEmbedder` reports the same dimension and model identity.
Pooling increases throughput; it never mixes models or substitutes vectors.

## Measurements

Useful measurements are cache hit rate, canonicalized hit rate, miss latency,
batch throughput, worker saturation, and model-revision-specific cache size.
Model runtime flags and accelerator placement remain properties of the host
embedding component.
