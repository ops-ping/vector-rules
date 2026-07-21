//! # em-log-n
//!
//! An embedded, latency-first searchable log store for Rust. Record every
//! event your program cares about (system logs, UI events, LLM tool calls,
//! PII probes), then walk them newest-first *or* reason over them with
//! arbitrary vector arithmetic — sync, in-process, no SQL planner in the
//! way.
//!
//! See the [project README](https://github.com/ops-ping/em-log-n) and the
//! [`docs/`](https://github.com/ops-ping/em-log-n/tree/main/docs) folder
//! for the design rationale and a comparison with neighbouring engines
//! (LanceDB, Qdrant Edge, redb / usearch DIY).
//!
//! ## Quick start
//!
//! ```no_run
//! # #[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
//! # fn main() -> em_log_n::Result<()> {
//! use em_log_n::key::KeyBuilder;
//! use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};
//!
//! let spec = ShardSpec {
//!     domain: DomainId::new("ui")?,
//!     indexes: vec![IndexSpec {
//!         name: "text".into(),
//!         dim: 768,
//!         metric: Metric::Cosine,
//!     }],
//! };
//! let shard = Shard::open(spec, "/var/lib/myapp/em-log-n/ui")?;
//!
//! // sync write — immediately visible to the next read / ann / scan
//! let ts_nanos: u64 = 1_700_000_000_000_000_000;
//! let text_embedding = vec![0.0f32; 768];
//! let key = KeyBuilder::new(ts_nanos, b"user clicked button").build();
//! shard.put(&key, b"payload-bytes", &[("text", &text_embedding)])?;
//!
//! // newest-first
//! let recent = shard.scan(50)?;
//!
//! // recent + relevant
//! let hits = shard.ann_in_window("text", &text_embedding, 10, 0, u64::MAX)?;
//! # let _ = (recent, hits);
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "fjall-backend", feature = "usearch-backend")))]
//! # fn main() {}
//! ```
//!
//! ## Modules
//!
//! - [`key`] — typed row-key construction. Pluggable [`KeyFormat`](key::KeyFormat)
//!   trait with two ships-with impls: `InverseTimestampKey` (Shard) and
//!   `ContentHashKey` (EmbedCache). Callers can plug their own.
//! - [`codec`] — pluggable value serialization (`prost`, `rkyv`, third-party).
//! - [`embed`] — the host-facing embedder trait and composition wrappers.
//! - [`embed_cache`] — content-addressed embedding cache + `CachingEmbedder` wrapper.
//! - [`retention`] — pluggable compaction-time retention policy.
//! - [`shard`] — per-domain `Shard` (the live engine: fjall + usearch).
//! - [`store`] — top-level registry of `ShardSpec`s.
//! - [`coldtier`] — shale-style object-store cold tier
//!   (`publish_generation`, `restore_into`, `gc`).
//!   Local backends ship in this module; the cloud backend lives in
//!   [`coldtier_cloud`] behind feature `coldtier-cloud`.
//!
//! ## Cargo features
//!
//! | Feature | Pulls in | Enables |
//! |---|---|---|
//! | *(none, default)* | — | trait scaffolding only; build is tiny |
//! | `fjall-backend` | `fjall` | the ordered-KV backend on `Shard` |
//! | `usearch-backend` | `usearch` | the ANN backend on `Shard` |
//! | `codec-prost` | `prost` | protobuf value codec |
//! | `codec-rkyv` | `rkyv` | zero-copy value codec |
//! | `coldtier-cloud` | `object_store`, `tokio`, `futures`, `url` | `CloudStore` (S3 / Azure / GCS) |
//! | `full` | everything | the test + bench configuration |
//!
//! ## Hard constraints
//!
//! These are non-negotiable; see
//! [docs/hard-constraints.md](https://github.com/ops-ping/em-log-n/blob/main/docs/hard-constraints.md):
//!
//! - **Sync write → visible**: no microbatch, no flush wait.
//! - **No columnar format on the hot path** (rules out Arrow/Lance live).
//! - **No SQL on the hot path** (rules out a planner).
//! - **In-process store** (no IPC or subprocess dependencies on the read/write
//!   hot path). The host supplies embeddings through the [`embed::Embedder`] trait.
//! - **Pluggable codecs and retention** with sane defaults.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod codec;
pub mod coldtier;
#[cfg(feature = "coldtier-cloud")]
pub mod coldtier_cloud;
#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
pub mod detail_backfill;
#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
pub mod dual_index;
pub mod embed;
pub mod embed_cache;
pub mod error;
pub mod key;
pub mod retention;
pub mod shard;
pub mod store;

pub use codec::LogValue;
pub use error::{Error, Result};
pub use key::{KeyBuilder, KeyParts, RowKey};
