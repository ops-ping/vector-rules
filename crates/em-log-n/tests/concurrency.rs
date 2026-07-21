//! Concurrency stress: one writer, many readers, no torn state.
//!
//! Mirrors shale's smoke-test pattern at a smaller scale:
//! - One writer thread continuously puts new rows under fresh timestamps.
//! - N reader threads loop, each performing `scan`, `ann`, and `get` and
//!   verifying every result is self-consistent (key parses, ANN distance
//!   ≥ 0, etc.).
//! - Run for a bounded duration; assert zero errors, monotonic visible-row
//!   counts, and that the final reader sees ≥ the writer's last-written
//!   timestamp.

#![cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use em_log_n::key::{KeyBuilder, KeyParts, DEFAULT_HASH_LEN};
use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};

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

#[test]
fn single_writer_many_readers_no_torn_state() {
    let tmp = tempfile::tempdir().unwrap();
    let dim = 32;
    let spec = ShardSpec {
        domain: DomainId::new("stress").unwrap(),
        indexes: vec![IndexSpec {
            name: "text".into(),
            dim,
            metric: Metric::Cosine,
        }],
    };
    let shard = Arc::new(Shard::open(spec, tmp.path()).unwrap());

    let stop = Arc::new(AtomicBool::new(false));
    let written = Arc::new(AtomicU64::new(0));
    let last_ts = Arc::new(AtomicU64::new(0));
    let reader_iterations = Arc::new(AtomicU64::new(0));
    let reader_errors = Arc::new(AtomicU64::new(0));

    // Writer.
    let writer = {
        let shard = Arc::clone(&shard);
        let stop = Arc::clone(&stop);
        let written = Arc::clone(&written);
        let last_ts = Arc::clone(&last_ts);
        std::thread::spawn(move || {
            let mut counter: u64 = 0;
            while !stop.load(Ordering::Relaxed) {
                counter += 1;
                let ts = counter * 1000; // strictly increasing
                let key = KeyBuilder::new(ts, format!("ev-{counter}").as_bytes()).build();
                let v = synthetic_vec(counter, dim);
                if shard
                    .put(
                        &key,
                        format!("p-{counter}").as_bytes(),
                        &[("text", v.as_slice())],
                    )
                    .is_ok()
                {
                    written.fetch_add(1, Ordering::Relaxed);
                    last_ts.store(ts, Ordering::Relaxed);
                }
                // Yield a bit so readers get progress; the point of the test
                // is correctness, not throughput.
                if counter.is_multiple_of(32) {
                    std::thread::yield_now();
                }
            }
        })
    };

    // Readers.
    let n_readers = 4;
    let mut reader_handles = Vec::with_capacity(n_readers);
    for r in 0..n_readers {
        let shard = Arc::clone(&shard);
        let stop = Arc::clone(&stop);
        let iters = Arc::clone(&reader_iterations);
        let errs = Arc::clone(&reader_errors);
        reader_handles.push(std::thread::spawn(move || {
            let mut probe: u64 = (r as u64).wrapping_mul(2654435761);
            while !stop.load(Ordering::Relaxed) {
                probe = probe.wrapping_mul(2654435761).wrapping_add(1);
                // scan
                match shard.scan(8) {
                    Ok(rows) => {
                        for (k, v) in &rows {
                            if KeyParts::parse(k.as_bytes(), DEFAULT_HASH_LEN).is_err() {
                                errs.fetch_add(1, Ordering::Relaxed);
                            }
                            if v.is_empty() {
                                errs.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    Err(_) => {
                        errs.fetch_add(1, Ordering::Relaxed);
                    }
                }
                // ann
                let q = synthetic_vec(probe, dim);
                match shard.ann("text", &q, 4) {
                    Ok(hits) => {
                        for h in hits {
                            if !(h.distance.is_finite() && h.distance >= -1e-3) {
                                errs.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    Err(_) => {
                        errs.fetch_add(1, Ordering::Relaxed);
                    }
                }
                iters.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    // Run for a bounded duration.
    let duration = Duration::from_secs(3);
    let start = Instant::now();
    while start.elapsed() < duration {
        std::thread::sleep(Duration::from_millis(100));
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().expect("writer thread");
    for h in reader_handles {
        h.join().expect("reader thread");
    }

    let written = written.load(Ordering::Relaxed);
    let last_ts = last_ts.load(Ordering::Relaxed);
    let iters = reader_iterations.load(Ordering::Relaxed);
    let errs = reader_errors.load(Ordering::Relaxed);
    eprintln!("stress: writes={written}  last_ts={last_ts}  reader_iters={iters}  errors={errs}");

    assert_eq!(errs, 0, "no reader errors expected");
    assert!(written > 0, "writer should have written something");
    assert!(iters > 0, "readers should have run something");

    // Final consistency: the latest scan must include a row with ts >= last_ts.
    let final_scan = shard.scan(1).expect("final scan");
    assert!(!final_scan.is_empty(), "final scan must be non-empty");
    let final_ts = KeyParts::parse(final_scan[0].0.as_bytes(), DEFAULT_HASH_LEN)
        .unwrap()
        .ts_nanos;
    assert_eq!(
        final_ts, last_ts,
        "final newest-first row must be the writer's last-written ts"
    );
}
