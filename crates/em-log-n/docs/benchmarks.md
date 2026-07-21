# Benchmarks

The benchmark that currently ships with em-log-n is `storage_bench`. It measures
the storage layer and the embedding cache directly, with **no external embedding
service in the loop**.

End-to-end embedding latency belongs to the caller's model component, hardware,
batching, and placement. Those numbers are measured at the deployment boundary,
not as a storage-crate property.

## Storage layer (no embedding service)

Run:

```sh
cargo run --release --features fjall-backend,usearch-backend --example storage_bench
```

Current benchmark shape:

- 5,000 rows pre-seeded
- dim = 768
- sync write with `fjall::PersistMode::SyncAll`
- 500 query samples per measurement

| Op | n | mean | p50 | p95 | p99 | max | throughput |
|---|---:|---:|---:|---:|---:|---:|---:|
| put (sync write + ann add + fsync) | 5000 | 571 us | 555 us | 1136 us | 1419 us | 4841 us | ~1.75k puts/sec |
| `scan(10)` newest-first | 500 | 2 us | 2 us | 2 us | 2 us | 152 us | ~500k ops/sec |
| `scan_window(10, 20%)` | 500 | 3 us | 2 us | 2 us | 8 us | 321 us | ~333k ops/sec |
| `ann(k=10)` | 500 | 511 us | 487 us | 707 us | 853 us | 1046 us | ~2k ops/sec |
| `ann_in_window(k=10, 20%)` | 500 | 531 us | 516 us | 664 us | 784 us | 980 us | ~1.9k ops/sec |
| **embed-cache hit** (768-d f32 vec) | 500 | **1 us** | **1 us** | **1 us** | **1 us** | 4 us | ~1M ops/sec |
| embed-cache miss (key absent) | 500 | <1 us | <1 us | <1 us | <1 us | 2 us | very fast |
| embed-cache put (overwrite) | 500 | 5 us | 5 us | 10 us | 19 us | 89 us | ~200k ops/sec |

## Takeaways

- **Newest-first is effectively native.** The reverse-timestamp key layout plus
  fjall iteration keep scans in the low-microsecond range.
- **ANN is hot-path viable.** At this scale the storage-side vector search stays
  around the half-millisecond mark.
- **Windowed ANN is correct but not yet segment-aware.** The modest overhead for
  a 20% window is why per-segment ANN remains on the roadmap.
- **The embedding cache is the dominant latency lever.** A repeated string is a
  microsecond-scale cache hit rather than a service call.

## What this doc does not benchmark

This doc intentionally does **not** publish a single "embedding latency" number
for the crate, because that is no longer a crate-local property. In current
deployments it depends on:

- the host embedding component
- model choice and quantization
- loopback vs remote network path
- batching
- cache hit, pull-through hit, or true miss
- canonicalized vs raw text inputs

For vector-rules deployments, measure the embedding path at the wllama component
boundary.

## Methodology notes

- Storage benchmarks use deterministic synthetic vectors, not a captured
  production embedding distribution.
- `SyncAll` durability is part of the benchmark; skipping fsync would change the
  put profile materially.
- These numbers are best used to understand **ratios** and **bottlenecks**, not
  as a universal product benchmark.
