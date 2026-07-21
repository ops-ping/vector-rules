//! Error type. Single crate-wide `Error` over `thiserror`.

use thiserror::Error;

/// Crate result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors surfaced by em-log-n.
#[derive(Debug, Error)]
pub enum Error {
    /// I/O failure (filesystem, object-store transport, etc).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Row key was the wrong length / shape for decoding.
    #[error("bad key: {0}")]
    BadKey(&'static str),

    /// Value codec failed (encode or decode).
    #[error("codec: {0}")]
    Codec(String),

    /// Embedding backend (llama.cpp etc) failed.
    #[error("embed: {0}")]
    Embed(String),

    /// Vector index (usearch) failure.
    #[error("vector index: {0}")]
    VectorIndex(String),

    /// KV backend (fjall) failure.
    #[error("kv backend: {0}")]
    KvBackend(String),

    /// Object-store / cold-tier failure.
    #[error("object store: {0}")]
    ObjectStore(String),

    /// Caller invariant violation.
    #[error("invariant: {0}")]
    Invariant(&'static str),
}
