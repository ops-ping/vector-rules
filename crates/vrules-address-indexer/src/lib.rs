use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use em_log_n::key::{KeyBuilder, RowKey};
use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;
use vrules_core::address_index_record;

pub const SCHEMA: &str = "vrules.openaddresses.us-address-index.v1";
pub const DOMAIN: &str = "us-addresses";
pub const VECTOR_INDEX: &str = "address";
pub const VECTOR_DIM: usize = 128;
pub const VECTORIZER: &str = "vrules-address-lexical-v1";

pub type Result<T> = std::result::Result<T, IndexerError>;

#[derive(Debug, Error)]
pub enum IndexerError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("csv: {0}")]
    Csv(#[from] csv::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("em-log-n: {0}")]
    EmLogN(#[from] em_log_n::Error),
    #[error("bad artifact: {0}")]
    BadArtifact(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressIndexManifest {
    pub schema: String,
    pub kind: ArtifactKind,
    pub generation: u64,
    pub base_generation: Option<u64>,
    pub domain: String,
    pub vector_index: String,
    pub vector_dim: usize,
    pub vectorizer: String,
    pub rows_file: String,
    pub upserts: usize,
    pub deletes: usize,
    pub skipped: usize,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    FullSnapshot,
    Patch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum AddressIndexOp {
    Upsert { row: Box<AddressIndexRow> },
    Delete { id: String, key: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressIndexRow {
    pub id: String,
    pub key: String,
    pub ts_nanos: String,
    pub tiebreaker: u64,
    pub payload: AddressIndexPayload,
    pub vectors: BTreeMap<String, Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressIndexPayload {
    pub id: String,
    pub canonical: String,
    pub display: String,
    pub components: Map<String, Value>,
    pub source: Value,
    pub source_hash: String,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressIndexArtifact {
    pub manifest: AddressIndexManifest,
    pub ops: Vec<AddressIndexOp>,
}

#[derive(Debug, Clone)]
pub struct PatchRequest {
    pub base_dir: PathBuf,
    pub input_csv: PathBuf,
    pub source: String,
    pub out_dir: PathBuf,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplySummary {
    pub upserts: usize,
    pub deletes: usize,
}

pub fn build_snapshot(
    input_csv: &Path,
    source: &str,
    generation: u64,
) -> Result<AddressIndexArtifact> {
    let (rows, skipped) = read_openaddresses_csv(input_csv, source, generation)?;
    Ok(artifact(
        ArtifactKind::FullSnapshot,
        generation,
        None,
        rows,
        Vec::new(),
        skipped,
        source,
    ))
}

pub fn build_patch(request: &PatchRequest) -> Result<AddressIndexArtifact> {
    let base = read_artifact(&request.base_dir)?;
    let base_generation = Some(base.manifest.generation);
    let base_rows = upsert_rows_by_id(base.ops.iter())
        .into_iter()
        .filter(|(_, row)| row_source(row).is_some_and(|source| source == request.source))
        .collect::<BTreeMap<_, _>>();
    let (next_rows, skipped) =
        read_openaddresses_csv(&request.input_csv, &request.source, request.generation)?;
    let next_by_id: BTreeMap<_, _> = next_rows
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect();

    let mut deletes = Vec::new();
    let mut upserts = Vec::new();
    let ids: BTreeSet<_> = base_rows.keys().chain(next_by_id.keys()).cloned().collect();
    for id in ids {
        match (base_rows.get(&id), next_by_id.get(&id)) {
            (Some(old), None) => deletes.push((old.id.clone(), old.key.clone())),
            (None, Some(new)) => upserts.push(new.clone()),
            (Some(old), Some(new)) if old.payload.source_hash != new.payload.source_hash => {
                deletes.push((old.id.clone(), old.key.clone()));
                upserts.push(new.clone());
            }
            _ => {}
        }
    }
    Ok(artifact(
        ArtifactKind::Patch,
        request.generation,
        base_generation,
        upserts,
        deletes,
        skipped,
        &request.source,
    ))
}

pub fn write_artifact(artifact: &AddressIndexArtifact, out_dir: &Path) -> Result<()> {
    fs::create_dir_all(out_dir)?;
    let manifest_path = out_dir.join("manifest.json");
    let rows_path = out_dir.join(&artifact.manifest.rows_file);
    fs::write(
        manifest_path,
        serde_json::to_vec_pretty(&artifact.manifest)?,
    )?;
    let mut writer = BufWriter::new(File::create(rows_path)?);
    for op in &artifact.ops {
        serde_json::to_writer(&mut writer, op)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

pub fn read_artifact(dir: &Path) -> Result<AddressIndexArtifact> {
    let manifest: AddressIndexManifest =
        serde_json::from_slice(&fs::read(dir.join("manifest.json"))?)?;
    validate_manifest(&manifest)?;
    let file = File::open(dir.join(&manifest.rows_file))?;
    let mut ops = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if !line.trim().is_empty() {
            ops.push(serde_json::from_str(&line)?);
        }
    }
    Ok(AddressIndexArtifact { manifest, ops })
}

pub fn apply_artifact_native(artifact_dir: &Path, db_dir: &Path) -> Result<ApplySummary> {
    let artifact = read_artifact(artifact_dir)?;
    let shard = Shard::open(
        ShardSpec {
            domain: DomainId::new(DOMAIN)?,
            indexes: vec![IndexSpec {
                name: VECTOR_INDEX.to_string(),
                dim: VECTOR_DIM,
                metric: Metric::Cosine,
            }],
        },
        db_dir,
    )?;
    let mut summary = ApplySummary {
        upserts: 0,
        deletes: 0,
    };
    for op in artifact.ops {
        match op {
            AddressIndexOp::Upsert { row } => {
                let key = RowKey::from_bytes(hex_decode(&row.key)?)?;
                let value = serde_json::to_vec(&row.payload)?;
                let vector = row.vectors.get(VECTOR_INDEX).ok_or_else(|| {
                    IndexerError::BadArtifact("upsert missing address vector".to_string())
                })?;
                shard.put(&key, &value, &[(VECTOR_INDEX, vector.as_slice())])?;
                summary.upserts += 1;
            }
            AddressIndexOp::Delete { key, .. } => {
                let key = RowKey::from_bytes(hex_decode(&key)?)?;
                shard.delete(&key)?;
                summary.deletes += 1;
            }
        }
    }
    Ok(summary)
}

fn artifact(
    kind: ArtifactKind,
    generation: u64,
    base_generation: Option<u64>,
    upserts: Vec<AddressIndexRow>,
    deletes: Vec<(String, String)>,
    skipped: usize,
    source: &str,
) -> AddressIndexArtifact {
    let upsert_len = upserts.len();
    let delete_len = deletes.len();
    let mut ops = Vec::with_capacity(upsert_len + delete_len);
    ops.extend(
        deletes
            .into_iter()
            .map(|(id, key)| AddressIndexOp::Delete { id, key }),
    );
    ops.extend(
        upserts
            .into_iter()
            .map(|row| AddressIndexOp::Upsert { row: Box::new(row) }),
    );
    AddressIndexArtifact {
        manifest: AddressIndexManifest {
            schema: SCHEMA.to_string(),
            kind,
            generation,
            base_generation,
            domain: DOMAIN.to_string(),
            vector_index: VECTOR_INDEX.to_string(),
            vector_dim: VECTOR_DIM,
            vectorizer: VECTORIZER.to_string(),
            rows_file: "rows.jsonl".to_string(),
            upserts: upsert_len,
            deletes: delete_len,
            skipped,
            sources: vec![source.to_string()],
        },
        ops,
    }
}

fn read_openaddresses_csv(
    input_csv: &Path,
    source_name: &str,
    generation: u64,
) -> Result<(Vec<AddressIndexRow>, usize)> {
    let mut rdr = csv::Reader::from_path(input_csv)?;
    let headers = rdr.headers()?.clone();
    let header_names: Vec<String> = headers.iter().map(|h| h.to_ascii_uppercase()).collect();
    let mut rows = Vec::new();
    let mut skipped = 0usize;
    for (ordinal, record) in rdr.records().enumerate() {
        let record = record?;
        let source = oa_source_value(&header_names, &record, source_name);
        let index_record =
            address_index_record(stable_address_id(source_name, &source), source.clone());
        if index_record.canonical.is_empty()
            || !index_record.components.contains_key("house_number")
            || !index_record.components.contains_key("road")
        {
            skipped += 1;
            continue;
        }
        let source_hash = source_hash(&source);
        let payload = AddressIndexPayload {
            id: index_record.id.clone(),
            canonical: index_record.canonical.clone(),
            display: index_record.display,
            components: index_record.components,
            source,
            source_hash,
            generation,
        };
        rows.push(address_row(
            index_record.id,
            payload,
            generation,
            ordinal as u64,
        )?);
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((rows, skipped))
}

fn oa_source_value(headers: &[String], record: &csv::StringRecord, source_name: &str) -> Value {
    let mut source = Map::new();
    source.insert("SOURCE".into(), json!(source_name));
    for (header, value) in headers.iter().zip(record.iter()) {
        let value = value.trim();
        if !value.is_empty() {
            source.insert(header.clone(), json!(value));
        }
    }
    source.entry("COUNTRY").or_insert_with(|| json!("US"));
    Value::Object(source)
}

fn stable_address_id(source_name: &str, source: &Value) -> String {
    let preferred = ["ID", "HASH"].into_iter().find_map(|field| {
        source
            .get(field)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
    });
    match preferred {
        Some(id) => format!("oa:{source_name}:{id}"),
        None => format!("oa:{source_name}:{}", &source_hash(source)[..20]),
    }
}

fn address_row(
    id: String,
    payload: AddressIndexPayload,
    generation: u64,
    ordinal: u64,
) -> Result<AddressIndexRow> {
    let ts_nanos = generation
        .checked_mul(1_000_000_000)
        .and_then(|base| base.checked_add(ordinal))
        .ok_or_else(|| IndexerError::BadArtifact("generation timestamp overflow".to_string()))?;
    let payload_bytes = serde_json::to_vec(&payload)?;
    let key = KeyBuilder::new(ts_nanos, &payload_bytes)
        .with_tiebreaker(ordinal)
        .build();
    let mut vectors = BTreeMap::new();
    vectors.insert(
        VECTOR_INDEX.to_string(),
        lexical_address_vector(&payload.canonical),
    );
    Ok(AddressIndexRow {
        id,
        key: hex_encode(key.as_bytes()),
        ts_nanos: ts_nanos.to_string(),
        tiebreaker: ordinal,
        payload,
        vectors,
    })
}

fn upsert_rows_by_id<'a>(
    ops: impl Iterator<Item = &'a AddressIndexOp>,
) -> BTreeMap<String, AddressIndexRow> {
    let mut rows = BTreeMap::new();
    for op in ops {
        match op {
            AddressIndexOp::Upsert { row } => {
                rows.insert(row.id.clone(), row.as_ref().clone());
            }
            AddressIndexOp::Delete { id, .. } => {
                rows.remove(id);
            }
        }
    }
    rows
}

fn row_source(row: &AddressIndexRow) -> Option<&str> {
    row.payload.source.get("SOURCE").and_then(Value::as_str)
}

fn validate_manifest(manifest: &AddressIndexManifest) -> Result<()> {
    if manifest.schema != SCHEMA {
        return Err(IndexerError::BadArtifact(format!(
            "unsupported schema `{}`",
            manifest.schema
        )));
    }
    if manifest.domain != DOMAIN {
        return Err(IndexerError::BadArtifact(format!(
            "unsupported domain `{}`",
            manifest.domain
        )));
    }
    if manifest.vector_index != VECTOR_INDEX || manifest.vector_dim != VECTOR_DIM {
        return Err(IndexerError::BadArtifact(
            "unsupported vector index configuration".to_string(),
        ));
    }
    Ok(())
}

fn source_hash(source: &Value) -> String {
    let bytes = serde_json::to_vec(source).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

fn lexical_address_vector(canonical: &str) -> Vec<f32> {
    let mut out = vec![0.0f32; VECTOR_DIM];
    for token in lexical_features(canonical) {
        let hash = blake3::hash(token.as_bytes());
        let bytes = hash.as_bytes();
        let idx =
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize % VECTOR_DIM;
        let sign = if bytes[4] & 1 == 0 { 1.0 } else { -1.0 };
        out[idx] += sign;
    }
    let norm = out.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut out {
            *value /= norm;
        }
    }
    out
}

fn lexical_features(canonical: &str) -> Vec<String> {
    let normalized = canonical
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>();
    let tokens: Vec<_> = normalized.split_whitespace().collect();
    let mut features = Vec::new();
    for token in &tokens {
        features.push(format!("tok:{token}"));
        for gram in token.as_bytes().windows(3) {
            if let Ok(gram) = std::str::from_utf8(gram) {
                features.push(format!("tri:{gram}"));
            }
        }
    }
    for pair in tokens.windows(2) {
        features.push(format!("bi:{}:{}", pair[0], pair[1]));
    }
    features
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

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(IndexerError::BadArtifact(
            "hex length must be even".to_string(),
        ));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.as_bytes().chunks_exact(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(IndexerError::BadArtifact("invalid hex".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_snapshot_and_patch() {
        let temp = tempfile::tempdir().unwrap();
        let v1 = temp.path().join("v1.csv");
        let v2 = temp.path().join("v2.csv");
        fs::write(
            &v1,
            "LON,LAT,NUMBER,STREET,CITY,REGION,POSTCODE,ID,HASH\n-89,39,111,East Cola Lane,Springfield,IL,62701,a1,h1\n-89,39,5,Royal Road,Springfield,IL,62702,a2,h2\n",
        )
        .unwrap();
        fs::write(
            &v2,
            "LON,LAT,NUMBER,STREET,CITY,REGION,POSTCODE,ID,HASH\n-89,39,111,East Cola Lane,Springfield,IL,62701,a1,h1\n-89,39,6,Royal Road,Springfield,IL,62702,a2,h3\n-89,39,9,Crown Street,Springfield,IL,62703,a3,h4\n",
        )
        .unwrap();

        let snapshot = build_snapshot(&v1, "us/il/demo", 1).unwrap();
        assert_eq!(snapshot.manifest.upserts, 2);
        assert_eq!(snapshot.manifest.deletes, 0);
        let out_v1 = temp.path().join("out-v1");
        write_artifact(&snapshot, &out_v1).unwrap();

        let patch = build_patch(&PatchRequest {
            base_dir: out_v1,
            input_csv: v2,
            source: "us/il/demo".into(),
            out_dir: temp.path().join("out-v2"),
            generation: 2,
        })
        .unwrap();
        assert_eq!(patch.manifest.upserts, 2);
        assert_eq!(patch.manifest.deletes, 1);
        assert!(patch
            .ops
            .iter()
            .any(|op| matches!(op, AddressIndexOp::Delete { id, .. } if id.ends_with(":a2"))));
        assert!(patch
            .ops
            .iter()
            .any(|op| matches!(op, AddressIndexOp::Upsert { row } if row.id.ends_with(":a3"))));
    }

    #[test]
    fn patch_only_deletes_rows_for_requested_source() {
        let temp = tempfile::tempdir().unwrap();
        let il = temp.path().join("il.csv");
        let ny = temp.path().join("ny.csv");
        let il_next = temp.path().join("il-next.csv");
        fs::write(
            &il,
            "NUMBER,STREET,CITY,REGION,POSTCODE,ID,HASH\n5,Royal Road,Springfield,IL,62702,il1,h1\n",
        )
        .unwrap();
        fs::write(
            &ny,
            "NUMBER,STREET,CITY,REGION,POSTCODE,ID,HASH\n7,Crown Street,Albany,NY,12207,ny1,h2\n",
        )
        .unwrap();
        fs::write(&il_next, "NUMBER,STREET,CITY,REGION,POSTCODE,ID,HASH\n").unwrap();

        let mut combined = build_snapshot(&il, "us/il/demo", 1).unwrap();
        let ny_snapshot = build_snapshot(&ny, "us/ny/demo", 1).unwrap();
        combined.ops.extend(ny_snapshot.ops);
        combined.manifest.upserts = 2;
        combined.manifest.sources.push("us/ny/demo".into());
        let base_dir = temp.path().join("base");
        write_artifact(&combined, &base_dir).unwrap();

        let patch = build_patch(&PatchRequest {
            base_dir,
            input_csv: il_next,
            source: "us/il/demo".into(),
            out_dir: temp.path().join("patch"),
            generation: 2,
        })
        .unwrap();
        assert_eq!(patch.manifest.deletes, 1);
        assert!(patch.ops.iter().all(|op| match op {
            AddressIndexOp::Delete { id, .. } => id.contains("us/il/demo"),
            AddressIndexOp::Upsert { row } => row.id.contains("us/il/demo"),
        }));
    }

    #[test]
    fn applies_native_patch_without_leaving_deleted_vector_hit() {
        let temp = tempfile::tempdir().unwrap();
        let v1 = temp.path().join("v1.csv");
        let v2 = temp.path().join("v2.csv");
        fs::write(
            &v1,
            "NUMBER,STREET,CITY,REGION,POSTCODE,ID,HASH\n5,Royal Road,Springfield,IL,62702,a2,h2\n",
        )
        .unwrap();
        fs::write(
            &v2,
            "NUMBER,STREET,CITY,REGION,POSTCODE,ID,HASH\n6,Royal Road,Springfield,IL,62702,a2,h3\n",
        )
        .unwrap();
        let out_v1 = temp.path().join("out-v1");
        let out_v2 = temp.path().join("out-v2");
        write_artifact(&build_snapshot(&v1, "us/il/demo", 1).unwrap(), &out_v1).unwrap();
        write_artifact(
            &build_patch(&PatchRequest {
                base_dir: out_v1.clone(),
                input_csv: v2,
                source: "us/il/demo".into(),
                out_dir: out_v2.clone(),
                generation: 2,
            })
            .unwrap(),
            &out_v2,
        )
        .unwrap();
        let db = temp.path().join("db");
        apply_artifact_native(&out_v1, &db).unwrap();
        apply_artifact_native(&out_v2, &db).unwrap();

        let shard = Shard::open(
            ShardSpec {
                domain: DomainId::new(DOMAIN).unwrap(),
                indexes: vec![IndexSpec {
                    name: VECTOR_INDEX.into(),
                    dim: VECTOR_DIM,
                    metric: Metric::Cosine,
                }],
            },
            &db,
        )
        .unwrap();
        assert_eq!(shard.scan(10).unwrap().len(), 1);
        let row = shard.scan(1).unwrap().pop().unwrap();
        let payload: AddressIndexPayload = serde_json::from_slice(&row.1).unwrap();
        assert!(payload.display.starts_with("6 Royal Road"));
    }
}
