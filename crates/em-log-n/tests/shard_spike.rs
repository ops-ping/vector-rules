//! End-to-end spike for the fjall + usearch shard.
//!
//! Proves the four hard-constraint properties:
//!
//! 1. **Sync write → immediately readable.** A `put` returns Ok; the very
//!    next `get` / `scan` / `ann` sees it. No flush wait, no microbatch.
//! 2. **Newest-first scan order is free.** Insert events in any order; the
//!    forward iterator yields them newest-first.
//! 3. **ANN works as a synchronous operation** with arbitrary caller-supplied
//!    query vectors (the "king − man + woman" pattern reduces to a vector
//!    the caller composes).
//! 4. **Time-window hybrid (ANN ∩ window)** returns only hits whose ts lies
//!    in the window.

#![cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]

use em_log_n::key::KeyBuilder;
use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};

fn synthetic_vec(seed: u32, dim: usize) -> Vec<f32> {
    // Deterministic, well-distributed pseudo-vector; not a real embedding,
    // just enough to make HNSW give consistent answers in tests.
    let mut v = Vec::with_capacity(dim);
    let mut s = seed.wrapping_mul(2654435761);
    for _ in 0..dim {
        s = s.wrapping_mul(2654435761).wrapping_add(1);
        v.push(((s as i32) as f32) / (i32::MAX as f32));
    }
    // L2 normalize (cosine is sensitive to magnitude).
    let m = v.iter().fold(0.0f32, |a, &x| x.mul_add(x, a)).sqrt();
    if m > 0.0 {
        for x in &mut v {
            *x /= m;
        }
    }
    v
}

fn open_shard(tmp: &tempfile::TempDir, dim: usize) -> Shard {
    let spec = ShardSpec {
        domain: DomainId::new("test").unwrap(),
        indexes: vec![IndexSpec {
            name: "text".into(),
            dim,
            metric: Metric::Cosine,
        }],
    };
    Shard::open(spec, tmp.path()).unwrap()
}

#[test]
fn sync_write_is_immediately_visible_to_get_scan_and_ann() {
    let tmp = tempfile::tempdir().unwrap();
    let dim = 32;
    let shard = open_shard(&tmp, dim);

    let v = synthetic_vec(7, dim);
    let key = KeyBuilder::new(1_000, b"hello world").build();
    shard
        .put(&key, b"payload-bytes", &[("text", v.as_slice())])
        .unwrap();

    // (1) point get sees it
    let got = shard.get(&key).unwrap();
    assert_eq!(got.as_deref(), Some(b"payload-bytes".as_slice()));

    // (2) scan sees it
    let scanned = shard.scan(10).unwrap();
    assert_eq!(scanned.len(), 1);
    assert_eq!(scanned[0].0, key);

    // (3) ann sees it (query = the same vector → distance ≈ 0)
    let hits = shard.ann("text", &v, 5).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].row_key, key);
    assert!(
        hits[0].distance < 1e-3,
        "self-hit distance {}",
        hits[0].distance
    );
}

#[test]
fn scan_is_newest_first_regardless_of_insert_order() {
    let tmp = tempfile::tempdir().unwrap();
    let dim = 16;
    let shard = open_shard(&tmp, dim);

    // Insert in scrambled timestamp order.
    let timestamps = [300u64, 100, 500, 200, 400];
    for (i, &ts) in timestamps.iter().enumerate() {
        let v = synthetic_vec(i as u32, dim);
        let key = KeyBuilder::new(ts, format!("evt-{ts}").as_bytes()).build();
        shard
            .put(
                &key,
                format!("v={ts}").as_bytes(),
                &[("text", v.as_slice())],
            )
            .unwrap();
    }

    let scanned = shard.scan(10).unwrap();
    let mut seen_ts = Vec::new();
    for (k, _v) in &scanned {
        let parts =
            em_log_n::key::KeyParts::parse(k.as_bytes(), em_log_n::key::DEFAULT_HASH_LEN).unwrap();
        seen_ts.push(parts.ts_nanos);
    }
    assert_eq!(
        seen_ts,
        vec![500, 400, 300, 200, 100],
        "must be newest-first"
    );
}

#[test]
fn scan_window_returns_only_in_window_rows_newest_first() {
    let tmp = tempfile::tempdir().unwrap();
    let dim = 8;
    let shard = open_shard(&tmp, dim);

    for ts in [100u64, 150, 200, 250, 300, 350, 400] {
        let v = synthetic_vec(ts as u32, dim);
        let key = KeyBuilder::new(ts, format!("e{ts}").as_bytes()).build();
        shard
            .put(
                &key,
                format!("v={ts}").as_bytes(),
                &[("text", v.as_slice())],
            )
            .unwrap();
    }

    // Window [200, 350) ⇒ {200, 250, 300}, newest-first ⇒ [300, 250, 200].
    let win = shard.scan_window(200, 350, 100).unwrap();
    let ts: Vec<u64> = win
        .iter()
        .map(|(k, _)| {
            em_log_n::key::KeyParts::parse(k.as_bytes(), em_log_n::key::DEFAULT_HASH_LEN)
                .unwrap()
                .ts_nanos
        })
        .collect();
    assert_eq!(ts, vec![300, 250, 200]);
}

#[test]
fn ann_with_arbitrary_query_vector_works() {
    // The "king − man + woman" pattern: caller composes a query vector from
    // existing vectors; ANN search treats it as just another vector. We use
    // synthetic vectors but the API contract is identical.
    let tmp = tempfile::tempdir().unwrap();
    let dim = 32;
    let shard = open_shard(&tmp, dim);

    let king = synthetic_vec(1, dim);
    let man = synthetic_vec(2, dim);
    let woman = synthetic_vec(3, dim);
    let queen = {
        // Same composition the caller would do: vec arithmetic in caller-land.
        let mut q = vec![0.0f32; dim];
        for i in 0..dim {
            q[i] = king[i] - man[i] + woman[i];
        }
        // Normalize so cosine distance is meaningful.
        let m = q.iter().fold(0.0f32, |a, &x| x.mul_add(x, a)).sqrt();
        if m > 0.0 {
            for x in &mut q {
                *x /= m;
            }
        }
        q
    };

    // Seed the index with several rows including `queen`.
    let queen_key = KeyBuilder::new(50, b"queen").build();
    shard
        .put(
            &KeyBuilder::new(10, b"king").build(),
            b"k",
            &[("text", king.as_slice())],
        )
        .unwrap();
    shard
        .put(
            &KeyBuilder::new(20, b"man").build(),
            b"m",
            &[("text", man.as_slice())],
        )
        .unwrap();
    shard
        .put(
            &KeyBuilder::new(30, b"woman").build(),
            b"w",
            &[("text", woman.as_slice())],
        )
        .unwrap();
    shard
        .put(&queen_key, b"q", &[("text", queen.as_slice())])
        .unwrap();
    for i in 100..120 {
        let v = synthetic_vec(i, dim);
        let k = KeyBuilder::new(i as u64 * 1_000, format!("noise-{i}").as_bytes()).build();
        shard.put(&k, b"n", &[("text", v.as_slice())]).unwrap();
    }

    // Query with the composed vector — the closest match must be queen.
    let hits = shard.ann("text", &queen, 3).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(
        hits[0].row_key, queen_key,
        "composed query must hit the composed vector"
    );
}

#[test]
fn ann_in_window_filters_by_time() {
    let tmp = tempfile::tempdir().unwrap();
    let dim = 16;
    let shard = open_shard(&tmp, dim);

    // Put the same-ish vector at multiple timestamps.
    let target = synthetic_vec(99, dim);
    let mut keys = Vec::new();
    for ts in [100u64, 200, 300, 400, 500] {
        let k = KeyBuilder::new(ts, format!("t{ts}").as_bytes()).build();
        // Same vector for each so the ANN ranking is purely by time-filter.
        shard
            .put(
                &k,
                format!("v={ts}").as_bytes(),
                &[("text", target.as_slice())],
            )
            .unwrap();
        keys.push((ts, k));
    }

    // Window [200, 400) ⇒ matches at ts ∈ {200, 300}.
    let hits = shard.ann_in_window("text", &target, 10, 200, 400).unwrap();
    let ts_returned: std::collections::BTreeSet<u64> = hits
        .iter()
        .map(|h| {
            em_log_n::key::KeyParts::parse(h.row_key.as_bytes(), em_log_n::key::DEFAULT_HASH_LEN)
                .unwrap()
                .ts_nanos
        })
        .collect();
    assert_eq!(ts_returned, [200u64, 300].into_iter().collect());
}

#[test]
fn put_rejects_unknown_index_and_wrong_dim_before_any_write() {
    let tmp = tempfile::tempdir().unwrap();
    let dim = 8;
    let shard = open_shard(&tmp, dim);

    let key = KeyBuilder::new(1, b"x").build();
    let v_right = vec![0.0f32; dim];
    let v_wrong = vec![0.0f32; dim + 1];

    // Unknown index name.
    assert!(shard
        .put(&key, b"v", &[("nope", v_right.as_slice())])
        .is_err());
    // Wrong dim.
    assert!(shard
        .put(&key, b"v", &[("text", v_wrong.as_slice())])
        .is_err());

    // After both failures, nothing should be visible.
    assert!(shard.get(&key).unwrap().is_none());
    assert!(shard.scan(10).unwrap().is_empty());
    assert!(shard.ann("text", &v_right, 5).unwrap().is_empty());
}
