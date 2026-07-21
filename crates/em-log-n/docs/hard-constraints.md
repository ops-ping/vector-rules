# Hard constraints

Public statement of design constraints that em-log-n is built around. These
are not preferences — they are the assumptions every feature, refactor, and
performance proposal must respect. If your idea is in tension with one of
these, please open an issue first so we can discuss before code is written.

## Latency is the product

The primary use cases require **sub-millisecond write→visible** and
**sub-millisecond ANN read**. Examples that drove this:

- **In-stream PII detection** — a write happens during a model's inference
  pass; the next read happens on the next token. There is no batch window
  to amortize anything.
- **Tool-use traces** in an LLM turn — each tool call appends to the log
  and may immediately be searched by the next call.

This forbids any design that introduces a flush boundary between writing
and reading.

## No columnar format on the write path

Columnar formats (Arrow, Parquet, Lance) intrinsically imply micro-batching
to amortize encoding cost. That is incompatible with the latency
constraint. Columnar is acceptable only as an **offline analytics export**
of cold segments — never as the live store.

## No SQL on the hot path

SQL adds a planner, query compilation, and a type system between the
caller and the index. That is the wrong shape for code that runs every
turn of an LLM loop. The public API is a small set of typed Rust calls
(`put`, `scan`, `scan_window`, `ann`, `ann_in_window`, `ann_multi`).

## In-process always

em-log-n is a library that lives in the caller's address space. There is
no server, no IPC, no broker.

- No async runtime is imposed on callers. Implementations may use
  internal parallelism but every public entry point is sync.
- The cloud cold-tier backend is already a pure-Rust in-process client
  (`CloudStore` over Apache Arrow's `object_store` crate). Future backends
  must keep that property. External subprocess dependencies (rclone, aws CLI)
  are explicitly rejected.
- Callers provide an `Embedder` implementation. That does not relax the storage
  constraint: storage, scan, cache, and ANN stay in-process.

## Pluggable codecs (typed, not stringly-typed)

Two different problems with two different answers:

- **Keys are composable, not serialized.** Row key is a fixed-layout
  big-endian byte string `reverse_ts_be (8B) || blake3(text)[..N] (NB) ||
  optional tiebreaker`. Lexicographic byte order MUST equal logical order
  — that rules out varint or length-prefixed encodings (no protobuf, no
  bincode, no postcard for keys). Callers compose via `KeyBuilder`; the
  crate encodes.
- **Values use a `Codec` trait.** Default impl is `prost` (protobuf)
  for stable, language-portable wire format with schema evolution. A
  second built-in is `rkyv` for zero-copy hot-path decode. Callers may
  plug a third impl.

## Pluggable retention

A `RetentionPolicy` trait runs at **compaction time only** (never on the
write or read hot path). The default impl keeps everything forever; users
can plug TTL-by-domain, point-delete for GDPR, etc., without touching the
core engine.

## Multiple named usearch indexes per domain are first-class

A domain shard hosts N usearch indexes (e.g. text, image, geo, fused
multi-modal), each with its own dimension and metric (built-in or a
caller-supplied function pointer). Queries can intersect or union across
indexes. This is what makes the store *executable* (drives behaviour in
the moment), not merely *analytical*.

## Stability of the on-disk format

A migration story exists from day one: the protobuf wire format is
defined by tag numbers, not field names, and tag numbers are never reused
after deprecation. Schema additions are always safe. The key layout is
versioned at the crate boundary; if it ever changes, a major version bump
ships with a migrator.
