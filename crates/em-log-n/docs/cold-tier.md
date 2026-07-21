# Cold-tier design

em-log-n's cold tier publishes immutable per-generation segment blobs and a
single atomic manifest blob to an `ObjectStore`. This document captures the
semantics every backend must respect, especially the corner cases that
weren't obvious until they bit a prototype.

## Model

Each domain owns a key namespace under `<root>/<domain>/`:

```
<root>/<domain>/gen/00000000/kv.snapshot
<root>/<domain>/gen/00000000/idx.text.usearch
<root>/<domain>/gen/00000001/kv.snapshot
<root>/<domain>/gen/00000001/idx.text.usearch
...
<root>/<domain>/manifest                       # the single source of truth
```

- A **generation** is one atomic publish. Each segment blob inside it is
  immutable.
- The **manifest** is a tiny ASCII blob listing live generation numbers
  in monotonic order, e.g. `40,41,42`.
- The manifest is the single source of truth. A segment that fails to
  load is only a real error if it is *still listed* in the freshly
  re-read manifest.

## Two-step publish

```
PUT <root>/<domain>/gen/N/kv.snapshot
PUT <root>/<domain>/gen/N/idx.<name1>.usearch
PUT <root>/<domain>/gen/N/idx.<name2>.usearch
...
PUT <root>/<domain>/manifest         # contains generation N for the first time
```

The manifest PUT is the publication barrier. A reader who observes the
manifest is guaranteed to find every named segment present.

## Two-step GC

```
read manifest -> [40,41,42,43,44]
keep K=3 -> drop {40, 41}
PUT <root>/<domain>/manifest    # now reads [42,43,44]
DELETE the segments for gens 40 and 41
```

Manifest PUT happens first so the generations become unreachable from
correct readers before any segment blob is deleted. If the process dies
between manifest PUT and segment delete, the next GC will retry.

## Absent-vs-error mapping

A GET on a segment blob can fail in two semantically-distinct ways:

- **Absent** — the blob has been GC'd or was never published. This is
  benign IF the gen has already left the manifest by the time we re-read
  it.
- **Error** — transport failure, auth failure, body corruption.

Every backend's `ObjectStore::get` MUST translate provider-specific
absence into `Ok(None)`. The list of strings/codes that signal "absent"
(culled from real shale experience):

| Provider | Signal |
|---|---|
| Azure Blob | `BlobNotFound`, `does not exist`, HTTP 404 |
| Amazon S3 | `NoSuchKey`, `Not Found`, HTTP 404 |
| Google Cloud Storage | `notFound`, HTTP 404 |
| Local FS | `ENOENT` / `std::io::ErrorKind::NotFound` |

A non-Ok-None error is treated as a hard failure unless the gen has
already left the manifest.

## Validate-after-GET + retry transient

Object stores can return a `200 OK` with a **truncated body** under
concurrent load (this was observed in shale against both Azurite and a
real S3 backend). Trusting the 200 and loading it as if it were complete
will fail downstream (usearch refuses to load a zero-byte or partially-
written index).

The required reader pattern:

1. GET the segment.
2. **Validate** by attempting to load + perform a trivial operation
   against it (parse the index, run a search).
3. If validation fails, retry the GET with a short delay (3–4 attempts
   total).
4. If still failing after retries, re-read the manifest. If the gen has
   left the manifest, the failure is benign; if it is still listed, it is
   a hard error.

em-log-n's `restore_into` implements steps 1–4 against the in-memory and
local-dir backends. The pattern carries over verbatim to future
in-process cloud backends.

## GC-race semantics — covered scenarios

`tests/coldtier.rs` covers the in-memory case:

- Publish 3 gens, simulate gen 2's KV blob being deleted while gen 2 is
  no longer in the manifest → restore must succeed against the latest
  live gen.
- Publish 1 gen, delete its KV blob while it is **still** listed in the
  manifest → restore must error (hard failure).
- Publish N gens, GC keep=K → manifest reflects the most-recent K, and
  the dropped generations' segments are gone.

When the real cloud backend lands, its tests must additionally cover:

- Eventual-consistency surprises (delete-then-GET races) where
  applicable per provider.
- Partial bodies under emulator load (Azurite is the worst offender,
  per shale).
- Transport-level retries (the underlying SDK's retry should be
  configurable; em-log-n stays out of its way).

## Reading list (origin of design)

- Shale prototype's `src/bin/smoke.rs` — original GET-validate-retry
  pattern for the rclone backend.
- Shale's `src/object_store.rs` — first version of the absent-vs-error
  mapping; logic salvaged here, code not reused.
- LanceDB's `object_store` integration (Apache Arrow `object_store`
  crate) — the substrate em-log-n's `CloudStore` is built on.

## Cloud backend

`em_log_n::coldtier::CloudStore` is the in-process backend for the
public clouds, built on the Apache Arrow
[`object_store`](https://docs.rs/object_store/) crate (pure Rust,
production-grade, used by Lance, DataFusion, InfluxDB IOx, crates.io).
It implements em-log-n's sync [`ObjectStore`] trait, so
`publish_generation` / `restore_into` / `gc` work unchanged against
S3, Azure Blob, or GCS.

Construct one of these:

```rust,no_run
use em_log_n::coldtier::CloudStore;

// AWS S3 with default credential resolution (env, IMDS, etc.):
let s3 = CloudStore::s3("my-bucket", "us-east-1", "em-log-n/prod")?;

// Azure Blob Storage:
let az = CloudStore::azure("myaccount", "mycontainer", "prefix")?;

// Google Cloud Storage:
let gs = CloudStore::gcs("my-bucket", "prefix")?;

// Any provider URL recognised by object_store::parse_url:
let any = CloudStore::from_url("s3://my-bucket/em-log-n/prod")?;

// Bring-your-own object_store impl (LocalStack, MinIO, R2, custom endpoints):
let custom = CloudStore::from_object_store(arrow_object_store, "prefix")?;
# Ok::<_, em_log_n::Error>(())
```

The bridge owns a small `tokio` current-thread runtime internally to
drive `object_store`'s async API; callers stay sync. The
absent-vs-error contract above is satisfied (Arrow returns a typed
`Error::NotFound` which we map to `Ok(None)`).
