#![deny(unsafe_code)]

//! Content-addressed embedding cache store for the vrules-rest tier.
//!
//! The host derives 32-byte cache keys and orchestrates the miss path; this
//! component is pure storage. It must NEVER call back into `ai:vrules/host`
//! (`embed` or `invoke`): the host invokes it from inside `Services::embed`
//! while the calling component's mutex is held, so a host call from here
//! deadlocks the shim.
//!
//! Expiry is an epoch bump appended to the segment, never a physical deletion:
//! an entry is live while `generation >= epoch`. The full history stays
//! reconstructable from the append-only `cache-*.jsonl` segments.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub cache_dir: PathBuf,
    #[serde(default)]
    pub instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Record {
    Entry {
        key: String,
        created_ts: u64,
        generation: u32,
        vector: Vec<f32>,
    },
    Epoch {
        epoch: u32,
        created_ts: u64,
    },
}

#[derive(Debug, Clone)]
struct Entry {
    created_ts: u64,
    generation: u32,
    vector: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct KeyRequest {
    key: String,
}

#[derive(Debug, Deserialize)]
struct PutRequest {
    key: String,
    vector: Vec<f32>,
}

#[derive(Debug)]
pub struct CacheStore {
    root: PathBuf,
    segment: PathBuf,
    entries: HashMap<String, Entry>,
    epoch: u32,
    append_failure: Option<String>,
}

impl CacheStore {
    pub fn open(root: PathBuf, instance_id: &str) -> Result<Self, String> {
        validate_instance_id(instance_id)?;
        fs::create_dir_all(&root).map_err(|e| format!("create {}: {e}", root.display()))?;
        let segment = root.join(format!("cache-{instance_id}.jsonl"));
        recover_segment(&segment)?;
        let mut entries = HashMap::new();
        let mut epoch = 0;
        for record in read_records(&root)? {
            match record {
                Record::Entry {
                    key,
                    created_ts,
                    generation,
                    vector,
                } => {
                    entries.insert(
                        key,
                        Entry {
                            created_ts,
                            generation,
                            vector,
                        },
                    );
                }
                Record::Epoch { epoch: seen, .. } => epoch = epoch.max(seen),
            }
        }
        Ok(Self {
            root,
            segment,
            entries,
            epoch,
            append_failure: None,
        })
    }

    pub fn get(&self, payload: &str) -> Result<String, String> {
        let request: KeyRequest =
            serde_json::from_str(payload).map_err(|e| format!("invalid get request: {e}"))?;
        validate_key(&request.key)?;
        let hit = self
            .entries
            .get(&request.key)
            .filter(|entry| entry.generation >= self.epoch);
        let result = match hit {
            Some(entry) => json!({
                "found": true,
                "vector": entry.vector,
                "generation": entry.generation,
                "created_ts": entry.created_ts,
            }),
            None => json!({ "found": false, "epoch": self.epoch }),
        };
        serde_json::to_string(&result).map_err(|e| format!("encode get result: {e}"))
    }

    pub fn put(&mut self, payload: &str) -> Result<String, String> {
        let request: PutRequest =
            serde_json::from_str(payload).map_err(|e| format!("invalid put request: {e}"))?;
        validate_key(&request.key)?;
        validate_vector(&request.vector)?;
        self.check_append_failure()?;
        let entry = Entry {
            created_ts: now_ns(),
            generation: self.epoch,
            vector: request.vector,
        };
        let record = Record::Entry {
            key: request.key.clone(),
            created_ts: entry.created_ts,
            generation: entry.generation,
            vector: entry.vector.clone(),
        };
        if let Err(error) = append_record(&self.segment, &record) {
            self.append_failure = Some(error.clone());
            return Err(error);
        }
        let generation = entry.generation;
        self.entries.insert(request.key, entry);
        serde_json::to_string(&json!({ "generation": generation }))
            .map_err(|e| format!("encode put result: {e}"))
    }

    pub fn expire(&mut self) -> Result<String, String> {
        self.check_append_failure()?;
        let epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| "cache epoch exhausted".to_string())?;
        let record = Record::Epoch {
            epoch,
            created_ts: now_ns(),
        };
        if let Err(error) = append_record(&self.segment, &record) {
            self.append_failure = Some(error.clone());
            return Err(error);
        }
        self.epoch = epoch;
        serde_json::to_string(&json!({ "epoch": epoch }))
            .map_err(|e| format!("encode expire result: {e}"))
    }

    pub fn epoch(&self) -> Result<String, String> {
        serde_json::to_string(&json!({ "epoch": self.epoch }))
            .map_err(|e| format!("encode epoch result: {e}"))
    }

    pub fn stats(&self) -> Result<String, String> {
        let live = self
            .entries
            .values()
            .filter(|entry| entry.generation >= self.epoch)
            .count();
        let segments = segment_paths(&self.root)?;
        let bytes = segments
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum::<u64>();
        serde_json::to_string(&json!({
            "entries": live,
            "epoch": self.epoch,
            "segments": segments.len(),
            "bytes": bytes,
        }))
        .map_err(|e| format!("encode stats result: {e}"))
    }

    pub fn invoke(&mut self, operation: &str, payload: &str) -> Result<String, String> {
        match operation {
            "get" => self.get(payload),
            "put" => self.put(payload),
            "expire" => self.expire(),
            "epoch" => self.epoch(),
            "stats" => self.stats(),
            other => Err(format!("unsupported cache operation `{other}`")),
        }
    }

    fn check_append_failure(&self) -> Result<(), String> {
        match &self.append_failure {
            Some(failure) => Err(format!(
                "cache segment is unavailable after an append failure: {failure}"
            )),
            None => Ok(()),
        }
    }
}

fn append_record(path: &Path, record: &Record) -> Result<(), String> {
    let mut line = serde_json::to_vec(record).map_err(|e| format!("encode cache record: {e}"))?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open cache segment {}: {e}", path.display()))?;
    let original_len = file
        .metadata()
        .map_err(|e| format!("read cache segment metadata {}: {e}", path.display()))?
        .len();
    if let Err(error) = file.write_all(&line) {
        return Err(rollback_append(&file, original_len, "append", error));
    }
    if let Err(error) = file.flush() {
        return Err(rollback_append(&file, original_len, "flush", error));
    }
    Ok(())
}

fn rollback_append(
    file: &File,
    original_len: u64,
    operation: &str,
    error: std::io::Error,
) -> String {
    match file.set_len(original_len) {
        Ok(()) => format!("{operation} cache record: {error}"),
        Err(rollback) => {
            format!(
                "{operation} cache record: {error}; rollback to {original_len} bytes failed: {rollback}"
            )
        }
    }
}

fn read_records(root: &Path) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    for path in segment_paths(root)? {
        let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let complete = &bytes[..complete_prefix_len(&bytes)];
        for (index, line) in complete.split(|byte| *byte == b'\n').enumerate() {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let record = serde_json::from_slice(line).map_err(|e| {
                format!(
                    "decode {} line {}: {e}",
                    path.display(),
                    index.saturating_add(1)
                )
            })?;
            records.push(record);
        }
    }
    Ok(records)
}

fn segment_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = fs::read_dir(root)
        .map_err(|e| format!("read cache directory {}: {e}", root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("cache-") && name.ends_with(".jsonl"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn recover_segment(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let complete_len = complete_prefix_len(&bytes);
    if complete_len == bytes.len() {
        return Ok(());
    }
    File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_len(complete_len as u64))
        .map_err(|e| format!("recover incomplete record in {}: {e}", path.display()))
}

fn complete_prefix_len(bytes: &[u8]) -> usize {
    if bytes.last() == Some(&b'\n') {
        bytes.len()
    } else {
        bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1)
    }
}

fn validate_instance_id(instance_id: &str) -> Result<(), String> {
    if instance_id.is_empty()
        || !instance_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("instance_id must contain only ASCII letters, digits, `-`, or `_`".to_string());
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err("cache key must be exactly 64 lowercase hex characters".to_string())
    }
}

fn validate_vector(vector: &[f32]) -> Result<(), String> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        // serde_json writes non-finite floats as `null`, which would corrupt
        // the segment on the next rebuild.
        Err("vectors must be non-empty and finite".to_string())
    } else {
        Ok(())
    }
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "plugin-component",
    });
}

#[cfg(target_arch = "wasm32")]
mod component {
    use std::sync::{Mutex, OnceLock};

    use super::{CacheStore, Config};
    use crate::bindings::ai::vrules::types::{PluginDescriptor, PluginKind};
    use crate::bindings::exports::ai::vrules::plugin::Guest;

    struct CacheComponent;

    static STATE: OnceLock<Mutex<CacheStore>> = OnceLock::new();

    impl Guest for CacheComponent {
        fn initialize(config: String) -> Result<PluginDescriptor, String> {
            let config: Config =
                serde_json::from_str(&config).map_err(|e| format!("invalid cache config: {e}"))?;
            let instance_id = config
                .instance_id
                .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
            let store = CacheStore::open(config.cache_dir, &instance_id)?;
            STATE
                .set(Mutex::new(store))
                .map_err(|_| "cache component is already initialized".to_string())?;
            Ok(PluginDescriptor {
                id: "cache".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                kind: PluginKind::Storage,
                operations: vec![
                    "get".to_string(),
                    "put".to_string(),
                    "expire".to_string(),
                    "epoch".to_string(),
                    "stats".to_string(),
                ],
            })
        }

        fn invoke(operation: String, payload: String) -> Result<String, String> {
            STATE
                .get()
                .ok_or_else(|| "cache component is not initialized".to_string())?
                .lock()
                .map_err(|_| "cache lock poisoned".to_string())?
                .invoke(&operation, &payload)
        }
    }

    #[allow(unsafe_code)]
    mod component_export {
        use super::CacheComponent;
        use crate::bindings;

        crate::bindings::export!(CacheComponent with_types_in bindings);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::CacheStore;

    const KEY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const KEY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn open(dir: &std::path::Path) -> CacheStore {
        CacheStore::open(dir.to_path_buf(), "test").expect("open store")
    }

    fn get_vector(store: &CacheStore, key: &str) -> Option<Vec<f32>> {
        let result: Value =
            serde_json::from_str(&store.get(&json!({ "key": key }).to_string()).expect("get"))
                .expect("decode get");
        if !result["found"].as_bool().expect("found flag") {
            return None;
        }
        Some(
            result["vector"]
                .as_array()
                .expect("vector array")
                .iter()
                .map(|value| value.as_f64().expect("vector element") as f32)
                .collect(),
        )
    }

    fn put_vector(store: &mut CacheStore, key: &str, vector: &[f32]) -> u32 {
        let result: Value = serde_json::from_str(
            &store
                .put(&json!({ "key": key, "vector": vector }).to_string())
                .expect("put"),
        )
        .expect("decode put");
        result["generation"].as_u64().expect("generation") as u32
    }

    #[test]
    fn put_get_round_trip_is_bit_exact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open(dir.path());
        let vector = vec![
            0.0f32,
            -0.0,
            1.5,
            -1.0e-40, // subnormal
            f32::MIN_POSITIVE,
            core::f32::consts::PI,
            -3.402_823_5e38,
        ];
        put_vector(&mut store, KEY_A, &vector);
        let bits = |values: &[f32]| values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        let hit = get_vector(&store, KEY_A).expect("hit");
        assert_eq!(bits(&hit), bits(&vector));

        // Bit-exact across a rebuild from the segment, too.
        let reopened = open(dir.path());
        let hit = get_vector(&reopened, KEY_A).expect("hit after reopen");
        assert_eq!(bits(&hit), bits(&vector));
    }

    #[test]
    fn epoch_expiry_cycle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open(dir.path());
        assert_eq!(put_vector(&mut store, KEY_A, &[1.0, 2.0]), 0);
        assert!(get_vector(&store, KEY_A).is_some());

        let expired: Value =
            serde_json::from_str(&store.expire().expect("expire")).expect("decode expire");
        assert_eq!(expired["epoch"], 1);
        assert!(get_vector(&store, KEY_A).is_none(), "expired entry served");

        assert_eq!(put_vector(&mut store, KEY_A, &[3.0, 4.0]), 1);
        assert_eq!(get_vector(&store, KEY_A).expect("re-put hit"), [3.0, 4.0]);

        // Epoch survives a rebuild; the pre-expiry entry stays dead.
        let reopened = open(dir.path());
        let epoch: Value =
            serde_json::from_str(&reopened.epoch().expect("epoch")).expect("decode epoch");
        assert_eq!(epoch["epoch"], 1);
        assert_eq!(get_vector(&reopened, KEY_A).expect("hit"), [3.0, 4.0]);
    }

    #[test]
    fn corrupt_tail_is_recovered() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open(dir.path());
        put_vector(&mut store, KEY_A, &[1.0]);
        drop(store);

        let segment = dir.path().join("cache-test.jsonl");
        let mut bytes = std::fs::read(&segment).expect("read segment");
        bytes.extend_from_slice(b"{\"kind\":\"entry\",\"key\":\"tru");
        std::fs::write(&segment, &bytes).expect("write truncated tail");

        let mut store = open(dir.path());
        assert_eq!(get_vector(&store, KEY_A).expect("survivor"), [1.0]);
        put_vector(&mut store, KEY_B, &[2.0]);
        let reopened = open(dir.path());
        assert_eq!(
            get_vector(&reopened, KEY_B).expect("post-recovery put"),
            [2.0]
        );
    }

    #[test]
    fn multi_segment_rebuild_is_last_writer_wins() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut first = CacheStore::open(dir.path().to_path_buf(), "a").expect("open a");
        put_vector(&mut first, KEY_A, &[1.0]);
        drop(first);
        let mut second = CacheStore::open(dir.path().to_path_buf(), "b").expect("open b");
        assert_eq!(
            get_vector(&second, KEY_A).expect("cross-segment hit"),
            [1.0]
        );
        put_vector(&mut second, KEY_A, &[9.0]);
        drop(second);

        let merged = CacheStore::open(dir.path().to_path_buf(), "c").expect("open c");
        assert_eq!(get_vector(&merged, KEY_A).expect("merged hit"), [9.0]);
        let stats: Value =
            serde_json::from_str(&merged.stats().expect("stats")).expect("decode stats");
        assert_eq!(stats["entries"], 1);
        assert_eq!(stats["segments"], 2);
    }

    #[test]
    fn rejects_invalid_keys_and_vectors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open(dir.path());
        for key in ["", "abc", &KEY_A.to_uppercase(), &format!("{KEY_A}aa")] {
            assert!(store.get(&json!({ "key": key }).to_string()).is_err());
        }
        assert!(
            store
                .put(&json!({ "key": KEY_A, "vector": [] }).to_string())
                .is_err()
        );
        assert!(
            store
                .put(&format!(r#"{{"key":"{KEY_A}","vector":[1.0,null]}}"#))
                .is_err()
        );
    }
}
