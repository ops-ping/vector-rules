//! Multi-index shard tests. Proves the marketplace pattern:
//! independent named usearch indexes per domain, combined at query time via
//! intersection or union.

#![cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]

use em_log_n::key::KeyBuilder;
use em_log_n::shard::{Combine, DomainId, IndexSpec, Metric, Shard, ShardSpec};

fn norm(mut v: Vec<f32>) -> Vec<f32> {
    let m = v.iter().fold(0.0f32, |a, &x| x.mul_add(x, a)).sqrt();
    if m > 0.0 {
        for x in &mut v {
            *x /= m;
        }
    }
    v
}

fn open() -> (tempfile::TempDir, Shard) {
    let tmp = tempfile::tempdir().unwrap();
    let spec = ShardSpec {
        domain: DomainId::new("marketplace").unwrap(),
        indexes: vec![
            IndexSpec {
                name: "image".into(),
                dim: 4,
                metric: Metric::Cosine,
            },
            IndexSpec {
                name: "text".into(),
                dim: 4,
                metric: Metric::Cosine,
            },
            IndexSpec {
                name: "geo".into(),
                dim: 2,
                metric: Metric::Haversine,
            },
        ],
    };
    let s = Shard::open(spec, tmp.path()).unwrap();
    (tmp, s)
}

/// Three "listings" with image, text, and geo vectors each. Query for one
/// that matches all three concepts.
#[test]
fn intersect_returns_rows_present_in_all_index_results() {
    let (_tmp, shard) = open();

    // Listing A — close in image space, close in text space, close in geo.
    let a_key = KeyBuilder::new(100, b"A").build();
    shard
        .put(
            &a_key,
            b"A",
            &[
                ("image", norm(vec![1.0, 0.0, 0.0, 0.0]).as_slice()),
                ("text", norm(vec![1.0, 0.0, 0.0, 0.0]).as_slice()),
                ("geo", [37.0f32, -122.0].as_slice()),
            ],
        )
        .unwrap();

    // Listing B — close in image and text but far in geo.
    let b_key = KeyBuilder::new(200, b"B").build();
    shard
        .put(
            &b_key,
            b"B",
            &[
                ("image", norm(vec![0.9, 0.1, 0.0, 0.0]).as_slice()),
                ("text", norm(vec![0.9, 0.1, 0.0, 0.0]).as_slice()),
                ("geo", [-37.0f32, 122.0].as_slice()),
            ],
        )
        .unwrap();

    // Listing C — far in image, far in text, close in geo.
    let c_key = KeyBuilder::new(300, b"C").build();
    shard
        .put(
            &c_key,
            b"C",
            &[
                ("image", norm(vec![0.0, 0.0, 0.0, 1.0]).as_slice()),
                ("text", norm(vec![0.0, 0.0, 0.0, 1.0]).as_slice()),
                ("geo", [37.0f32, -122.0].as_slice()),
            ],
        )
        .unwrap();

    let q_image = norm(vec![1.0, 0.0, 0.0, 0.0]);
    let q_text = norm(vec![1.0, 0.0, 0.0, 0.0]);
    let q_geo = [37.0f32, -122.0];

    // Intersection over image ∩ text ∩ geo with per-query k=2:
    //   image top-2 = {A, B}
    //   text  top-2 = {A, B}
    //   geo   top-2 = {A, C}
    //   ∩          = {A}
    let hits = shard
        .ann_multi(
            &[
                ("image", &q_image, 2),
                ("text", &q_text, 2),
                ("geo", q_geo.as_slice(), 2),
            ],
            Combine::IntersectSum,
            10,
        )
        .unwrap();

    let returned: Vec<_> = hits.iter().map(|h| h.row_key.clone()).collect();
    assert_eq!(returned, vec![a_key.clone()], "only A matches all three");
}

#[test]
fn union_returns_any_index_hit_and_orders_by_min_distance() {
    let (_tmp, shard) = open();

    let a_key = KeyBuilder::new(100, b"A").build();
    shard
        .put(
            &a_key,
            b"A",
            &[
                ("image", norm(vec![1.0, 0.0, 0.0, 0.0]).as_slice()),
                ("text", norm(vec![0.0, 0.0, 0.0, 1.0]).as_slice()),
                ("geo", [10.0f32, 10.0].as_slice()),
            ],
        )
        .unwrap();
    let b_key = KeyBuilder::new(200, b"B").build();
    shard
        .put(
            &b_key,
            b"B",
            &[
                ("image", norm(vec![0.0, 0.0, 0.0, 1.0]).as_slice()),
                ("text", norm(vec![1.0, 0.0, 0.0, 0.0]).as_slice()),
                ("geo", [10.0f32, 10.0].as_slice()),
            ],
        )
        .unwrap();

    let q_image = norm(vec![1.0, 0.0, 0.0, 0.0]);
    let q_text = norm(vec![1.0, 0.0, 0.0, 0.0]);

    let hits = shard
        .ann_multi(
            &[("image", &q_image, 5), ("text", &q_text, 5)],
            Combine::UnionMin,
            10,
        )
        .unwrap();

    let returned: std::collections::BTreeSet<_> = hits.iter().map(|h| h.row_key.clone()).collect();
    let expected: std::collections::BTreeSet<_> = [a_key, b_key].into_iter().collect();
    assert_eq!(returned, expected, "union should contain both rows");
}

#[test]
fn empty_queries_returns_empty() {
    let (_tmp, shard) = open();
    let hits = shard.ann_multi(&[], Combine::IntersectSum, 5).unwrap();
    assert!(hits.is_empty());
}

#[test]
fn unknown_index_in_multi_query_errors() {
    let (_tmp, shard) = open();
    let q = vec![0.0f32; 4];
    let err = shard.ann_multi(&[("missing", &q, 1)], Combine::UnionMin, 5);
    assert!(err.is_err());
}
