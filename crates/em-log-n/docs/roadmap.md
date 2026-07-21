# Roadmap

This is the current public roadmap for em-log-n inside the vector-rules workspace.
Completed capabilities live in the README and design docs; this page focuses on
what still looks important from here.

## [Performance - tighter-window ANN](https://github.com/ops-ping/vector-rules/issues/7)

`ann_in_window` is correct today, but it still oversamples globally and filters by
timestamp afterward. For very tight windows that wastes work.

The main performance roadmap item is still **per-segment ANN sub-indexes**:

- build smaller ANN structures alongside storage segments
- map time windows to the relevant segment set
- search only those segments for `ann_in_window`

This keeps the live API the same while improving the cost profile for
window-constrained search.

## [Cold tier - partial reads when they matter](https://github.com/ops-ping/vector-rules/issues/8)

The current cold tier is intentionally simple: immutable blobs plus an atomic
manifest. That matches the correctness model well, but whole-blob reads may stop
being attractive once generations become large enough.

The follow-on work is **range GET / partial hydration**:

- fetch only the needed byte ranges from object storage
- load just the indexes or snapshots a query actually needs
- preserve the same manifest-first correctness rules

This stays a measured optimization, not a default complexity increase.

## [Repair and operator tooling](https://github.com/ops-ping/vector-rules/issues/10)

The storage primitives are intentionally low level. Useful follow-on tooling still
includes:

- index / cache inspection helpers
- integrity and repair flows for persisted index state
- retention-policy dry runs
- generation inspection for cold-tier debugging

These do not change the core design, but they make production use easier.

## [More reference examples](https://github.com/ops-ping/vector-rules/issues/11)

The crate already has the building blocks for audit, organizational memory, and
distributed embedding reuse. More examples are still valuable, especially around:

- host adapters composed with `CachingEmbedder`
- canonicalization-aware cache namespaces
- model-revision-aware cache and audit patterns
- audit-style append + semantic search patterns

## [API stability](https://github.com/ops-ping/vector-rules/issues/12)

Until 1.0, expect API churn at minor-version boundaries. The on-disk format is
treated as more stable than the API surface; if that format ever changes, it
should ship with an explicit migration story.
