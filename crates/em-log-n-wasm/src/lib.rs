//! Browser-ready em-log-n distribution.
//!
//! Native em-log-n shards use fjall for ordered KV and usearch for ANN. The
//! pinned Rust bindings for those backends are native-oriented, while browsers
//! provide durable local storage through IndexedDB. This crate keeps the same
//! row-key ordering and vector-search semantics in WASM and pairs with
//! `web/em_log_n_browser.js` for durable IndexedDB persistence.

#![deny(unsafe_code)]

use std::collections::{BTreeMap, HashMap};

use em_log_n::key::{KeyBuilder, KeyParts, RowKey, DEFAULT_HASH_LEN};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// In-browser em-log-n shard.
#[wasm_bindgen]
pub struct EmLogN {
    domain: String,
    indexes: HashMap<String, BrowserIndexSpec>,
    rows: BTreeMap<String, BrowserStoredRow>,
}

#[wasm_bindgen]
impl EmLogN {
    /// Create a shard from JSON:
    /// `{ "domain": "ui", "indexes": [{ "name": "text", "dim": 768, "metric": "cosine" }] }`.
    #[wasm_bindgen(constructor)]
    pub fn new(spec_json: &str) -> Result<EmLogN, JsValue> {
        let spec: BrowserShardSpec = serde_json::from_str(spec_json)
            .map_err(|e| JsValue::from_str(&format!("invalid shard spec JSON: {e}")))?;
        validate_domain(&spec.domain)?;
        let mut indexes = HashMap::with_capacity(spec.indexes.len());
        for index in spec.indexes {
            if index.name.is_empty() {
                return Err(JsValue::from_str("index name must be non-empty"));
            }
            if index.dim == 0 {
                return Err(JsValue::from_str("index dim must be > 0"));
            }
            if indexes.insert(index.name.clone(), index).is_some() {
                return Err(JsValue::from_str("duplicate index name"));
            }
        }
        Ok(Self {
            domain: spec.domain,
            indexes,
            rows: BTreeMap::new(),
        })
    }

    /// Domain id for this shard.
    #[wasm_bindgen(getter)]
    pub fn domain(&self) -> String {
        self.domain.clone()
    }

    /// Number of persisted rows loaded into the live search index.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns true when no rows are loaded.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Remove all loaded rows. The JS IndexedDB wrapper owns durable deletion.
    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// Delete one loaded row by hex row key. The JS IndexedDB wrapper owns
    /// durable deletion.
    pub fn delete_key(&mut self, key: &str) -> bool {
        self.rows.remove(key).is_some()
    }

    /// Insert or replace a row. Input JSON:
    /// `{ "ts_nanos": "1700000000000000000", "payload": {...}, "vectors": { "text": [0.1] } }`.
    /// Returns the stored row, including its deterministic row key.
    pub fn put_json(&mut self, row_json: &str) -> Result<JsValue, JsValue> {
        let input: BrowserPutRow = serde_json::from_str(row_json)
            .map_err(|e| JsValue::from_str(&format!("invalid row JSON: {e}")))?;
        let stored = self.put_inner(input)?;
        to_js(&stored)
    }

    /// Load rows previously returned by `put_json` or by the IndexedDB wrapper.
    /// Existing loaded rows are retained and matching keys are replaced.
    pub fn load_rows_json(&mut self, rows_json: &str) -> Result<usize, JsValue> {
        let rows: Vec<BrowserStoredRow> = serde_json::from_str(rows_json)
            .map_err(|e| JsValue::from_str(&format!("invalid rows JSON: {e}")))?;
        let mut loaded = 0usize;
        for row in rows {
            self.validate_stored_row(&row)?;
            self.rows.insert(row.key.clone(), row);
            loaded += 1;
        }
        Ok(loaded)
    }

    /// Export all loaded rows as JSON for simple host-controlled snapshots.
    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.rows.values().collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".into())
    }

    /// Newest-first scan using em-log-n inverse-timestamp row-key ordering.
    pub fn scan(&self, limit: usize) -> Result<JsValue, JsValue> {
        let rows = self.rows.values().take(limit).cloned().collect::<Vec<_>>();
        to_js(&rows)
    }

    /// Vector search over a named index. Query JSON is a numeric vector array.
    pub fn ann(&self, index_name: &str, query_json: &str, k: usize) -> Result<JsValue, JsValue> {
        let query = parse_query(query_json)?;
        let hits = self.ann_inner(index_name, &query, k, None)?;
        to_js(&hits)
    }

    /// Vector search restricted to `[t_lo, t_hi)` nanoseconds.
    pub fn ann_in_window(
        &self,
        index_name: &str,
        query_json: &str,
        k: usize,
        t_lo: String,
        t_hi: String,
    ) -> Result<JsValue, JsValue> {
        let query = parse_query(query_json)?;
        let t_lo = parse_u64(&t_lo, "t_lo")?;
        let t_hi = parse_u64(&t_hi, "t_hi")?;
        let hits = self.ann_inner(index_name, &query, k, Some((t_lo, t_hi)))?;
        to_js(&hits)
    }
}

impl EmLogN {
    fn put_inner(&mut self, input: BrowserPutRow) -> Result<BrowserStoredRow, JsValue> {
        let stored = self.prepare_row(input)?;
        self.rows.insert(stored.key.clone(), stored.clone());
        Ok(stored)
    }

    fn prepare_row(&self, input: BrowserPutRow) -> Result<BrowserStoredRow, JsValue> {
        let ts_nanos = parse_u64(&input.ts_nanos, "ts_nanos")?;
        self.validate_vectors(&input.vectors)?;
        let payload_bytes = serde_json::to_vec(&input.payload)
            .map_err(|e| JsValue::from_str(&format!("payload encode: {e}")))?;
        let key = KeyBuilder::new(ts_nanos, &payload_bytes)
            .with_tiebreaker(input.tiebreaker.unwrap_or(0))
            .build();
        Ok(BrowserStoredRow {
            key: hex_encode(key.as_bytes()),
            ts_nanos: ts_nanos.to_string(),
            payload: input.payload,
            vectors: input.vectors,
        })
    }

    fn validate_stored_row(&self, row: &BrowserStoredRow) -> Result<(), JsValue> {
        let key_bytes = hex_decode(&row.key)?;
        RowKey::from_bytes(key_bytes.clone())
            .map_err(|e| JsValue::from_str(&format!("bad row key: {e}")))?;
        let parts = KeyParts::parse(&key_bytes, DEFAULT_HASH_LEN)
            .map_err(|e| JsValue::from_str(&format!("bad row-key parts: {e}")))?;
        let ts_nanos = parse_u64(&row.ts_nanos, "ts_nanos")?;
        if parts.ts_nanos != ts_nanos {
            return Err(JsValue::from_str(
                "row key timestamp does not match ts_nanos",
            ));
        }
        self.validate_vectors(&row.vectors)
    }

    fn validate_vectors(&self, vectors: &HashMap<String, Vec<f32>>) -> Result<(), JsValue> {
        for (name, vector) in vectors {
            let spec = self
                .indexes
                .get(name)
                .ok_or_else(|| JsValue::from_str(&format!("unknown index `{name}`")))?;
            if vector.len() != spec.dim {
                return Err(JsValue::from_str(&format!(
                    "vector `{name}` dim mismatch: expected {}, got {}",
                    spec.dim,
                    vector.len()
                )));
            }
        }
        Ok(())
    }

    fn ann_inner(
        &self,
        index_name: &str,
        query: &[f32],
        k: usize,
        window: Option<(u64, u64)>,
    ) -> Result<Vec<BrowserAnnHit>, JsValue> {
        let spec = self
            .indexes
            .get(index_name)
            .ok_or_else(|| JsValue::from_str(&format!("unknown index `{index_name}`")))?;
        if query.len() != spec.dim {
            return Err(JsValue::from_str(&format!(
                "query dim mismatch: expected {}, got {}",
                spec.dim,
                query.len()
            )));
        }
        let mut hits = Vec::new();
        for row in self.rows.values() {
            let ts_nanos = parse_u64(&row.ts_nanos, "ts_nanos")?;
            if let Some((lo, hi)) = window {
                if ts_nanos < lo || ts_nanos >= hi {
                    continue;
                }
            }
            let Some(vector) = row.vectors.get(index_name) else {
                continue;
            };
            hits.push(BrowserAnnHit {
                key: row.key.clone(),
                ts_nanos: row.ts_nanos.clone(),
                distance: distance(spec.metric, query, vector)?,
                payload: row.payload.clone(),
            });
        }
        hits.sort_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(k);
        Ok(hits)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct BrowserShardSpec {
    domain: String,
    indexes: Vec<BrowserIndexSpec>,
}

#[derive(Debug, Clone, Deserialize)]
struct BrowserIndexSpec {
    name: String,
    dim: usize,
    #[serde(default)]
    metric: BrowserMetric,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum BrowserMetric {
    #[default]
    Cosine,
    L2sq,
    Ip,
    Haversine,
}

#[derive(Debug, Deserialize)]
struct BrowserPutRow {
    ts_nanos: String,
    #[serde(default)]
    tiebreaker: Option<u64>,
    payload: Value,
    #[serde(default)]
    vectors: HashMap<String, Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserStoredRow {
    key: String,
    ts_nanos: String,
    payload: Value,
    vectors: HashMap<String, Vec<f32>>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserAnnHit {
    key: String,
    ts_nanos: String,
    distance: f32,
    payload: Value,
}

fn distance(metric: BrowserMetric, a: &[f32], b: &[f32]) -> Result<f32, JsValue> {
    Ok(match metric {
        BrowserMetric::Cosine => cosine_distance(a, b),
        BrowserMetric::L2sq => a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum(),
        BrowserMetric::Ip => -a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>(),
        BrowserMetric::Haversine => {
            if a.len() != 2 || b.len() != 2 {
                return Err(JsValue::from_str("haversine metric requires dim=2"));
            }
            haversine_km(a[0], a[1], b[0], b[1])
        }
    })
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut an = 0.0f32;
    let mut bn = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        an += x * x;
        bn += y * y;
    }
    if an == 0.0 || bn == 0.0 {
        return f32::INFINITY;
    }
    1.0 - dot / (an.sqrt() * bn.sqrt())
}

fn haversine_km(lat_a: f32, lon_a: f32, lat_b: f32, lon_b: f32) -> f32 {
    let r = 6_371.0f32;
    let dlat = (lat_b - lat_a).to_radians();
    let dlon = (lon_b - lon_a).to_radians();
    let lat_a = lat_a.to_radians();
    let lat_b = lat_b.to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat_a.cos() * lat_b.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * r * h.sqrt().asin()
}

fn parse_query(query_json: &str) -> Result<Vec<f32>, JsValue> {
    serde_json::from_str(query_json)
        .map_err(|e| JsValue::from_str(&format!("invalid query vector JSON: {e}")))
}

fn parse_u64(s: &str, field: &str) -> Result<u64, JsValue> {
    s.parse::<u64>()
        .map_err(|e| JsValue::from_str(&format!("invalid {field}: {e}")))
}

fn validate_domain(domain: &str) -> Result<(), JsValue> {
    em_log_n::shard::DomainId::new(domain)
        .map(|_| ())
        .map_err(|e| JsValue::from_str(&format!("invalid domain: {e}")))
}

fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(value).map_err(|e| JsValue::from_str(&e.to_string()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(s: &str) -> Result<Vec<u8>, JsValue> {
    if !s.len().is_multiple_of(2) {
        return Err(JsValue::from_str("hex key length must be even"));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.as_bytes().chunks_exact(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, JsValue> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(JsValue::from_str("invalid hex key")),
    }
}

/// Self-test helper used by Node/browser smoke tests.
#[wasm_bindgen]
pub fn smoke_test() -> Result<JsValue, JsValue> {
    let mut shard =
        EmLogN::new(r#"{"domain":"ui","indexes":[{"name":"text","dim":3,"metric":"cosine"}]}"#)?;
    shard.put_json(
        r#"{"ts_nanos":"1700000000000000000","payload":{"msg":"alpha"},"vectors":{"text":[1,0,0]}}"#,
    )?;
    shard.put_json(
        r#"{"ts_nanos":"1700000000000000001","payload":{"msg":"beta"},"vectors":{"text":[0,1,0]}}"#,
    )?;
    let hits = shard.ann_inner("text", &[1.0, 0.0, 0.0], 1, None)?;
    to_js(&json!({
        "len": shard.len(),
        "top": hits.first().map(|h| h.payload.clone()),
        "scan": serde_json::from_str::<Value>(&shard.snapshot_json()).unwrap_or(Value::Null),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_newest_first_and_searches_vectors() {
        let mut shard =
            EmLogN::new(r#"{"domain":"ui","indexes":[{"name":"text","dim":2,"metric":"cosine"}]}"#)
                .unwrap();
        shard
            .put_inner(BrowserPutRow {
                ts_nanos: "100".into(),
                tiebreaker: None,
                payload: json!({"msg":"old"}),
                vectors: [("text".to_string(), vec![1.0, 0.0])].into(),
            })
            .unwrap();
        shard
            .put_inner(BrowserPutRow {
                ts_nanos: "200".into(),
                tiebreaker: None,
                payload: json!({"msg":"new"}),
                vectors: [("text".to_string(), vec![0.0, 1.0])].into(),
            })
            .unwrap();

        let scanned: Vec<BrowserStoredRow> = shard.rows.values().take(2).cloned().collect();
        assert_eq!(scanned[0].payload["msg"], "new");

        let hits = shard.ann_inner("text", &[1.0, 0.0], 1, None).unwrap();
        assert_eq!(hits[0].payload["msg"], "old");
    }
}
