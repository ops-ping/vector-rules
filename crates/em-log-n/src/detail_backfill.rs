//! Off-hot-path population of the detail (raw) index.
//!
//! In the dual-index design (see [`crate::dual_index`]) the write hot path only
//! pays for the **canonical** embedding — almost always an
//! [`EmbedCache`](crate::embed_cache::EmbedCache) hit — and writes it to the
//! coarse index synchronously. The expensive **raw** embedding for the detail
//! index is handed to a [`DetailBackfill`] so it lands later, out of the
//! caller's latency path.
//!
//! Two implementations ship:
//! - [`SyncBackfill`] — embeds and adds inline on [`submit`](DetailBackfill::submit).
//!   Deterministic; ideal for v0, tests, and small volumes.
//! - [`AsyncBackfill`] — a bounded queue drained by a worker thread (so a slow
//!   embed never blocks the caller).
//!   Best-effort: if the queue is full a job is dropped (the detail index simply
//!   stays sparse for that row, which [`Combine::UnionMin`](crate::shard::Combine)
//!   tolerates), and the drop is counted.
//!
//! Both are interchangeable behind the [`DetailBackfill`] trait, so a deployment
//! can start synchronous and switch to async without touching call sites.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::embed::Embedder;
use crate::error::{Error, Result};
use crate::key::RowKey;
use crate::shard::Shard;

/// A unit of detail-index backfill work: embed `text` with the raw embedder and
/// add the resulting vector to `detail_index` for `row_key`.
#[derive(Debug, Clone)]
pub struct BackfillJob {
    /// Row whose detail vector is being filled in (must already exist).
    pub row_key: RowKey,
    /// Name of the detail (raw) index to populate.
    pub detail_index: String,
    /// Raw text to embed for the detail vector.
    pub text: String,
}

/// Sink for [`BackfillJob`]s. Implementations embed and add the vector either
/// inline or asynchronously.
pub trait DetailBackfill: Send + Sync {
    /// Hand off one job. Synchronous impls do the work now; async impls enqueue
    /// it. Returns `Ok` on a successful hand-off (which, for async best-effort
    /// impls, may mean the job was dropped due to backpressure — see
    /// [`AsyncBackfill`]).
    ///
    /// # Errors
    /// Implementation-specific: e.g. embed/index failures for synchronous impls,
    /// or a terminated worker for async impls.
    fn submit(&self, job: BackfillJob) -> Result<()>;
}

/// Inline backfill: embeds and writes on the calling thread.
pub struct SyncBackfill<E: Embedder> {
    shard: Arc<Shard>,
    embedder: Arc<E>,
}

impl<E: Embedder> SyncBackfill<E> {
    /// New synchronous backfill against `shard`, embedding raw text with
    /// `embedder` (the RAW embedder — NOT a canonicalizing one).
    pub fn new(shard: Arc<Shard>, embedder: Arc<E>) -> Self {
        Self { shard, embedder }
    }
}

impl<E: Embedder> DetailBackfill for SyncBackfill<E> {
    fn submit(&self, job: BackfillJob) -> Result<()> {
        let v = self.embedder.embed(&job.text)?;
        self.shard.add_vector(&job.row_key, &job.detail_index, &v)
    }
}

/// Asynchronous, bounded, best-effort backfill backed by a worker thread.
pub struct AsyncBackfill {
    tx: Option<SyncSender<BackfillJob>>,
    worker: Option<JoinHandle<()>>,
    dropped: Arc<AtomicU64>,
}

impl AsyncBackfill {
    /// Spawn a worker that drains a queue of `capacity` jobs, embedding each raw
    /// text with `embedder` and adding the vector to the shard. `embedder` is
    /// the RAW embedder for the detail index.
    pub fn new<E>(shard: Arc<Shard>, embedder: Arc<E>, capacity: usize) -> Self
    where
        E: Embedder + 'static,
    {
        let (tx, rx) = sync_channel::<BackfillJob>(capacity.max(1));
        let worker = std::thread::Builder::new()
            .name("emlogn-backfill".into())
            .spawn(move || {
                // Exits when the sender is dropped (channel disconnects).
                while let Ok(job) = rx.recv() {
                    match embedder.embed(&job.text) {
                        Ok(v) => {
                            if let Err(e) = shard.add_vector(&job.row_key, &job.detail_index, &v) {
                                eprintln!("em-log-n: detail backfill add_vector failed: {e}");
                            }
                        }
                        Err(e) => {
                            eprintln!("em-log-n: detail backfill embed failed: {e}");
                        }
                    }
                }
            })
            .expect("spawn backfill worker");
        Self {
            tx: Some(tx),
            worker: Some(worker),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Number of jobs dropped so far due to a full queue (backpressure).
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl DetailBackfill for AsyncBackfill {
    fn submit(&self, job: BackfillJob) -> Result<()> {
        let tx = self
            .tx
            .as_ref()
            .ok_or(Error::Invariant("backfill sender closed"))?;
        match tx.try_send(job) {
            Ok(()) => Ok(()),
            // Best-effort: a full queue drops the detail vector. The detail
            // index stays sparse for this row; UnionMin queries still work.
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => {
                Err(Error::Invariant("backfill worker terminated"))
            }
        }
    }
}

impl Drop for AsyncBackfill {
    fn drop(&mut self) {
        // Close the channel so the worker's recv() returns Err and it exits,
        // then join to ensure all in-flight jobs finished.
        self.tx = None;
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dual_index::DualIndex;
    use crate::key::KeyBuilder;
    use crate::shard::{DomainId, Metric, ShardSpec};

    /// Deterministic embedder: first `dim` bytes of text → normalized vector.
    struct ByteEmbedder {
        dim: usize,
    }
    impl Embedder for ByteEmbedder {
        fn dim(&self) -> usize {
            self.dim
        }
        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            let mut v = vec![0.0f32; self.dim];
            for (i, b) in text.bytes().take(self.dim).enumerate() {
                v[i] = f32::from(b) / 255.0;
            }
            let m = v.iter().fold(0.0f32, |a, &x| x.mul_add(x, a)).sqrt();
            if m > 0.0 {
                for x in &mut v {
                    *x /= m;
                }
            }
            Ok(v)
        }
    }

    fn open_dual(dim: usize, dual: &DualIndex) -> (tempfile::TempDir, Arc<Shard>) {
        let tmp = tempfile::tempdir().unwrap();
        let spec = ShardSpec {
            domain: DomainId::new("backfill-test").unwrap(),
            indexes: dual.index_specs(dim, Metric::Cosine).to_vec(),
        };
        let shard = Shard::open(spec, tmp.path()).unwrap();
        (tmp, Arc::new(shard))
    }

    #[test]
    fn sync_backfill_populates_detail_index() {
        let dim = 8;
        let dual = DualIndex::new("text");
        let (_tmp, shard) = open_dual(dim, &dual);
        let embedder = Arc::new(ByteEmbedder { dim });

        // Write row + canonical vector (canon index only).
        let key = KeyBuilder::new(100, b"row").build();
        let canon_vec = embedder.embed("User <*> login").unwrap();
        dual.put_canon(&shard, &key, b"payload", &canon_vec)
            .unwrap();

        // Detail index is empty until backfill.
        let q = embedder.embed("User 42 login").unwrap();
        assert!(shard.ann(&dual.detail, &q, 5).unwrap().is_empty());

        // Backfill the raw vector synchronously.
        let bf = SyncBackfill::new(Arc::clone(&shard), Arc::clone(&embedder));
        bf.submit(BackfillJob {
            row_key: key.clone(),
            detail_index: dual.detail.clone(),
            text: "User 42 login".into(),
        })
        .unwrap();

        // Now the detail index returns the row.
        let hits = shard.ann(&dual.detail, &q, 5).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn async_backfill_eventually_populates_then_query_unions() {
        let dim = 8;
        let dual = DualIndex::new("text");
        let (_tmp, shard) = open_dual(dim, &dual);
        let embedder = Arc::new(ByteEmbedder { dim });

        let key = KeyBuilder::new(200, b"row").build();
        let canon_vec = embedder.embed("disk <*> full").unwrap();
        dual.put_canon(&shard, &key, b"payload", &canon_vec)
            .unwrap();

        let bf = AsyncBackfill::new(Arc::clone(&shard), Arc::clone(&embedder), 16);
        bf.submit(BackfillJob {
            row_key: key.clone(),
            detail_index: dual.detail.clone(),
            text: "disk 90 full".into(),
        })
        .unwrap();
        // Dropping the backfill joins the worker → all jobs are durable.
        drop(bf);

        // Dual query: coarse + detail unioned. Both vectors find the row.
        let q_canon = embedder.embed("disk <*> full").unwrap();
        let q_detail = embedder.embed("disk 90 full").unwrap();
        let hits = dual.query(&shard, &q_canon, &q_detail, 5, 5).unwrap();
        assert_eq!(hits.len(), 1);
    }
}
