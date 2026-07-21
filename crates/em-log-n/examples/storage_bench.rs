//! Hot-path microbench (no embedding cost — that's `embed_bench.rs`).
//!
//! Measures the **storage layer alone** so we can quantify what's left after
//! embeddings are out of the picture:
//!
//! - put throughput (sync write→visible, no embed)
//! - scan latency (newest-first prefix iteration)
//! - ann latency (HNSW search)
//! - ann_in_window latency
//!
//! Run: `cargo run --release --features fjall-backend,usearch-backend --example storage_bench`.

use em_log_n::key::KeyBuilder;
use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};
use std::time::Instant;

fn norm(mut v: Vec<f32>) -> Vec<f32> {
    let m = v.iter().fold(0.0f32, |a, &x| x.mul_add(x, a)).sqrt();
    if m > 0.0 {
        for x in &mut v {
            *x /= m;
        }
    }
    v
}
fn synthetic_vec(seed: u64, dim: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(dim);
    let mut s = seed.wrapping_mul(2654435761);
    for _ in 0..dim {
        s = s.wrapping_mul(2654435761).wrapping_add(1);
        v.push(((s as i64) as f32) / (i64::MAX as f32));
    }
    norm(v)
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p / 100.0).round() as usize;
    sorted[idx]
}

fn report(label: &str, samples_us: &mut [u128]) {
    samples_us.sort_unstable();
    let mean = samples_us.iter().sum::<u128>() / samples_us.len() as u128;
    let p50 = percentile(samples_us, 50.0);
    let p95 = percentile(samples_us, 95.0);
    let p99 = percentile(samples_us, 99.0);
    let max = *samples_us.last().unwrap();
    let throughput = if mean > 0 { 1_000_000 / mean } else { 0 };
    println!(
        "{label:>22}  n={:>6}  mean={mean:>6}µs  p50={p50:>6}µs  p95={p95:>6}µs  p99={p99:>6}µs  max={max:>6}µs  ≈{throughput} ops/sec",
        samples_us.len()
    );
}

fn main() {
    let tmp = tempfile::tempdir().unwrap();
    let dim = 768; // EmbeddingGemma dim
    let n_seed = 5_000usize;
    let n_query = 500usize;

    let spec = ShardSpec {
        domain: DomainId::new("bench").unwrap(),
        indexes: vec![IndexSpec {
            name: "text".into(),
            dim,
            metric: Metric::Cosine,
        }],
    };
    let shard = Shard::open(spec, tmp.path()).unwrap();

    eprintln!("seeding {n_seed} rows (dim={dim})...");
    let mut put_us = Vec::with_capacity(n_seed);
    for i in 0..n_seed {
        let ts = (i as u64 + 1) * 1_000_000;
        let key = KeyBuilder::new(ts, format!("evt-{i}").as_bytes()).build();
        let v = synthetic_vec(i as u64, dim);
        let value = format!("payload-{i}-and-some-padding-to-make-it-realistic").into_bytes();
        let t0 = Instant::now();
        shard.put(&key, &value, &[("text", v.as_slice())]).unwrap();
        put_us.push(t0.elapsed().as_micros());
    }
    report("put (sync write+ann add)", &mut put_us);

    // scan top-10 newest
    let mut scan_us = Vec::with_capacity(n_query);
    for _ in 0..n_query {
        let t0 = Instant::now();
        let r = shard.scan(10).unwrap();
        scan_us.push(t0.elapsed().as_micros());
        assert_eq!(r.len(), 10);
    }
    report("scan(10)", &mut scan_us);

    // ANN
    let mut ann_us = Vec::with_capacity(n_query);
    for i in 0..n_query {
        let q = synthetic_vec((n_seed + i) as u64, dim);
        let t0 = Instant::now();
        let r = shard.ann("text", &q, 10).unwrap();
        ann_us.push(t0.elapsed().as_micros());
        assert!(!r.is_empty());
    }
    report("ann(k=10)", &mut ann_us);

    // ANN-in-window (middle 20% of seeded ts range).
    let t_lo = (n_seed as u64 * 1_000_000) * 4 / 10;
    let t_hi = (n_seed as u64 * 1_000_000) * 6 / 10;
    let mut win_us = Vec::with_capacity(n_query);
    for i in 0..n_query {
        let q = synthetic_vec((n_seed + i) as u64, dim);
        let t0 = Instant::now();
        let _ = shard.ann_in_window("text", &q, 10, t_lo, t_hi).unwrap();
        win_us.push(t0.elapsed().as_micros());
    }
    report("ann_in_window(k=10,20%)", &mut win_us);

    // scan_window (middle 20%).
    let mut sw_us = Vec::with_capacity(n_query);
    for _ in 0..n_query {
        let t0 = Instant::now();
        let _ = shard.scan_window(t_lo, t_hi, 10).unwrap();
        sw_us.push(t0.elapsed().as_micros());
    }
    report("scan_window(k=10,20%)", &mut sw_us);

    // Embedding cache hit latency (separate fjall keyspace; vector is bf16).
    bench_embed_cache(dim, n_query);
}

fn bench_embed_cache(dim: usize, n_query: usize) {
    use em_log_n::embed_cache::EmbedCache;

    let tmp = tempfile::tempdir().unwrap();
    let cache = EmbedCache::open(tmp.path(), "bench-model", dim).unwrap();

    // Seed with a small population of cached vectors.
    let n_seed = 1_000;
    for i in 0..n_seed {
        let v = synthetic_vec(i as u64, dim);
        cache.put(&format!("seed-{i}"), &v).unwrap();
    }

    // Measure HITs.
    let mut hit_us = Vec::with_capacity(n_query);
    for i in 0..n_query {
        let key = format!("seed-{}", i % n_seed);
        let t0 = Instant::now();
        let v = cache.get(&key).unwrap().unwrap();
        hit_us.push(t0.elapsed().as_micros());
        assert_eq!(v.len(), dim);
    }
    report("embed_cache_hit", &mut hit_us);

    // Measure MISSes (key not present).
    let mut miss_us = Vec::with_capacity(n_query);
    for i in 0..n_query {
        let key = format!("not-cached-{i}");
        let t0 = Instant::now();
        let v = cache.get(&key).unwrap();
        miss_us.push(t0.elapsed().as_micros());
        assert!(v.is_none());
    }
    report("embed_cache_miss", &mut miss_us);

    // Measure PUTs (warm; existing keys, overwrite).
    let mut put_us = Vec::with_capacity(n_query);
    for i in 0..n_query {
        let key = format!("seed-{}", i % n_seed);
        let v = synthetic_vec((i + n_seed) as u64, dim);
        let t0 = Instant::now();
        cache.put(&key, &v).unwrap();
        put_us.push(t0.elapsed().as_micros());
    }
    report("embed_cache_put", &mut put_us);
}
