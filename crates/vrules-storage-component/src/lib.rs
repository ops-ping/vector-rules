#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "plugin-component",
    });
}

use bindings::ai::vrules::types::{PluginDescriptor, PluginKind};
use bindings::exports::ai::vrules::plugin::Guest;

struct StorageComponent;

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

#[derive(Debug)]
struct State {
    root: PathBuf,
    segment: PathBuf,
    instance_id: String,
    next_sequence: u64,
    append_failure: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Config {
    data_dir: PathBuf,
    #[serde(default)]
    instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Event {
    id: String,
    stream: String,
    kind: String,
    timestamp_ns: u64,
    instance_id: String,
    sequence: u64,
    payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vector: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supersedes: Option<String>,
    #[serde(default)]
    tombstone: bool,
}

#[derive(Debug, Deserialize)]
struct AppendRequest {
    stream: String,
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    timestamp_ns: Option<u64>,
    #[serde(default)]
    payload: Value,
    #[serde(default)]
    vector: Option<Vec<f32>>,
    #[serde(default)]
    embedding_model: Option<String>,
    #[serde(default)]
    supersedes: Option<String>,
    #[serde(default)]
    tombstone: bool,
}

#[derive(Debug, Deserialize)]
struct ScanRequest {
    stream: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    include_superseded: bool,
    #[serde(default)]
    include_tombstones: bool,
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    stream: String,
    query: Vec<f32>,
    embedding_model: String,
    #[serde(default = "default_limit")]
    k: usize,
    #[serde(default)]
    include_superseded: bool,
    #[serde(default)]
    include_tombstones: bool,
}

#[derive(Debug, Serialize)]
struct SearchHit {
    event: Event,
    distance: f32,
}

#[derive(Debug, Deserialize)]
struct HistoryRequest {
    id: String,
}

fn default_kind() -> String {
    "record".to_string()
}

fn default_limit() -> usize {
    50
}

impl Guest for StorageComponent {
    fn initialize(config: String) -> Result<PluginDescriptor, String> {
        let config: Config =
            serde_json::from_str(&config).map_err(|e| format!("invalid storage config: {e}"))?;
        let instance_id = config
            .instance_id
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
        validate_instance_id(&instance_id)?;
        fs::create_dir_all(&config.data_dir)
            .map_err(|e| format!("create {}: {e}", config.data_dir.display()))?;
        let segment = config.data_dir.join(format!("events-{instance_id}.jsonl"));
        recover_segment(&segment)?;
        let next_sequence = segment_sequence(&segment)?;
        STATE
            .set(Mutex::new(State {
                root: config.data_dir,
                segment,
                instance_id,
                next_sequence,
                append_failure: None,
            }))
            .map_err(|_| "storage component is already initialized".to_string())?;
        Ok(descriptor())
    }

    fn invoke(operation: String, payload: String) -> Result<String, String> {
        match operation.as_str() {
            "append" => append(&payload),
            "scan" => scan(&payload),
            "search" => search(&payload),
            "history" => history(&payload),
            "stats" => stats(),
            other => Err(format!("unsupported storage operation `{other}`")),
        }
    }
}

fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        id: "storage".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        kind: PluginKind::Storage,
        operations: vec![
            "append".to_string(),
            "scan".to_string(),
            "search".to_string(),
            "history".to_string(),
            "stats".to_string(),
        ],
    }
}

fn append(payload: &str) -> Result<String, String> {
    let request: AppendRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid append request: {e}"))?;
    validate_stream(&request.stream)?;
    validate_vector(request.vector.as_deref())?;
    match (&request.vector, &request.embedding_model) {
        (Some(_), Some(model)) if !model.trim().is_empty() => {}
        (Some(_), _) => return Err("vector events require embedding_model".to_string()),
        (None, Some(_)) => return Err("embedding_model requires a vector".to_string()),
        (None, None) => {}
    }

    let mut state = state()?.lock().map_err(|_| "storage lock poisoned")?;
    if let Some(failure) = &state.append_failure {
        return Err(format!(
            "storage segment is unavailable after an append failure: {failure}"
        ));
    }
    let timestamp_ns = request.timestamp_ns.unwrap_or_else(now_ns);
    let sequence = state.next_sequence;
    let next_sequence = sequence
        .checked_add(1)
        .ok_or_else(|| "event sequence exhausted".to_string())?;
    let id = event_id(
        &state.instance_id,
        sequence,
        timestamp_ns,
        &request.stream,
        &request.payload,
    )?;
    let event = Event {
        id,
        stream: request.stream,
        kind: request.kind,
        timestamp_ns,
        instance_id: state.instance_id.clone(),
        sequence,
        payload: request.payload,
        vector: request.vector,
        embedding_model: request.embedding_model,
        supersedes: request.supersedes,
        tombstone: request.tombstone,
    };
    if let Err(error) = append_event(&state.segment, &event) {
        state.append_failure = Some(error.clone());
        return Err(error);
    }
    state.next_sequence = next_sequence;
    serde_json::to_string(&event).map_err(|e| format!("encode appended event: {e}"))
}

fn scan(payload: &str) -> Result<String, String> {
    let request: ScanRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid scan request: {e}"))?;
    validate_stream(&request.stream)?;
    let root = state()?
        .lock()
        .map_err(|_| "storage lock poisoned")?
        .root
        .clone();
    let mut events = live_events(
        read_events(&root)?,
        &request.stream,
        request.include_superseded,
        request.include_tombstones,
    );
    newest_first(&mut events);
    events.truncate(request.limit);
    serde_json::to_string(&json!({ "events": events }))
        .map_err(|e| format!("encode scan result: {e}"))
}

fn search(payload: &str) -> Result<String, String> {
    let request: SearchRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid search request: {e}"))?;
    validate_stream(&request.stream)?;
    validate_vector(Some(&request.query))?;
    if request.embedding_model.trim().is_empty() {
        return Err("search embedding_model must not be empty".to_string());
    }
    let root = state()?
        .lock()
        .map_err(|_| "storage lock poisoned")?
        .root
        .clone();
    let events = live_events(
        read_events(&root)?,
        &request.stream,
        request.include_superseded,
        request.include_tombstones,
    );
    let mut hits = Vec::new();
    for event in events {
        if event.embedding_model.as_deref() != Some(request.embedding_model.as_str()) {
            continue;
        }
        let Some(vector) = event.vector.as_deref() else {
            continue;
        };
        if vector.len() != request.query.len() {
            continue;
        }
        let distance = cosine_distance(&request.query, vector);
        let mut event = event;
        event.vector = None;
        hits.push(SearchHit { distance, event });
    }
    hits.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.event.timestamp_ns.cmp(&a.event.timestamp_ns))
    });
    hits.truncate(request.k);
    serde_json::to_string(&json!({ "hits": hits }))
        .map_err(|e| format!("encode search result: {e}"))
}

fn history(payload: &str) -> Result<String, String> {
    let request: HistoryRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid history request: {e}"))?;
    if request.id.trim().is_empty() {
        return Err("history id must not be empty".to_string());
    }
    let root = state()?
        .lock()
        .map_err(|_| "storage lock poisoned")?
        .root
        .clone();
    let events = read_events(&root)?;
    let mut by_id: HashMap<String, Event> = events
        .iter()
        .cloned()
        .map(|event| (event.id.clone(), event))
        .collect();
    let mut next = request.id;
    let mut chain = Vec::new();
    let mut visited = HashSet::new();
    while visited.insert(next.clone()) {
        let Some(event) = by_id.remove(&next) else {
            break;
        };
        next = event.supersedes.clone().unwrap_or_default();
        chain.push(event);
        if next.is_empty() {
            break;
        }
    }
    chain.reverse();
    serde_json::to_string(&json!({ "events": chain }))
        .map_err(|e| format!("encode history result: {e}"))
}

fn stats() -> Result<String, String> {
    let state = state()?.lock().map_err(|_| "storage lock poisoned")?;
    let events = read_events(&state.root)?;
    let bytes = segment_paths(&state.root)?
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
    let mut streams = HashMap::<String, usize>::new();
    for event in &events {
        *streams.entry(event.stream.clone()).or_default() += 1;
    }
    serde_json::to_string(&json!({
        "events": events.len(),
        "segments": segment_paths(&state.root)?.len(),
        "bytes": bytes,
        "streams": streams,
    }))
    .map_err(|e| format!("encode stats result: {e}"))
}

fn state() -> Result<&'static Mutex<State>, String> {
    STATE
        .get()
        .ok_or_else(|| "storage component is not initialized".to_string())
}

fn append_event(path: &Path, event: &Event) -> Result<(), String> {
    let mut record = serde_json::to_vec(event).map_err(|e| format!("encode event: {e}"))?;
    record.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open append segment {}: {e}", path.display()))?;
    let original_len = file
        .metadata()
        .map_err(|e| format!("read append segment metadata {}: {e}", path.display()))?
        .len();
    if let Err(error) = file.write_all(&record) {
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
        Ok(()) => format!("{operation} event: {error}"),
        Err(rollback) => {
            format!(
                "{operation} event: {error}; rollback to {original_len} bytes failed: {rollback}"
            )
        }
    }
}

fn read_events(root: &Path) -> Result<Vec<Event>, String> {
    let mut events = Vec::new();
    for path in segment_paths(root)? {
        let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let complete = &bytes[..complete_prefix_len(&bytes)];
        for (index, line) in complete.split(|byte| *byte == b'\n').enumerate() {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let event = serde_json::from_slice(line).map_err(|e| {
                format!(
                    "decode {} line {}: {e}",
                    path.display(),
                    index.saturating_add(1)
                )
            })?;
            events.push(event);
        }
    }
    Ok(events)
}

fn segment_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = fs::read_dir(root)
        .map_err(|e| format!("read storage directory {}: {e}", root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("events-") && name.ends_with(".jsonl"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn live_events(
    events: Vec<Event>,
    stream: &str,
    include_superseded: bool,
    include_tombstones: bool,
) -> Vec<Event> {
    let superseded = events
        .iter()
        .filter_map(|event| event.supersedes.clone())
        .collect::<HashSet<_>>();
    events
        .into_iter()
        .filter(|event| event.stream == stream)
        .filter(|event| include_superseded || !superseded.contains(&event.id))
        .filter(|event| include_tombstones || !event.tombstone)
        .collect()
}

fn newest_first(events: &mut [Event]) {
    events.sort_by(|a, b| {
        b.timestamp_ns
            .cmp(&a.timestamp_ns)
            .then_with(|| b.instance_id.cmp(&a.instance_id))
            .then_with(|| b.sequence.cmp(&a.sequence))
    });
}

fn segment_sequence(path: &Path) -> Result<u64, String> {
    if !path.exists() {
        return Ok(0);
    }
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(bytes[..complete_prefix_len(&bytes)]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
        .count() as u64)
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
        .map_err(|e| format!("recover incomplete event in {}: {e}", path.display()))
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

fn event_id(
    instance_id: &str,
    sequence: u64,
    timestamp_ns: u64,
    stream: &str,
    payload: &Value,
) -> Result<String, String> {
    let payload = serde_json::to_vec(payload).map_err(|e| format!("encode event payload: {e}"))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(instance_id.as_bytes());
    hasher.update(&sequence.to_le_bytes());
    hasher.update(&timestamp_ns.to_le_bytes());
    hasher.update(stream.as_bytes());
    hasher.update(&payload);
    Ok(hasher.finalize().to_hex().to_string())
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

fn validate_stream(stream: &str) -> Result<(), String> {
    if stream.trim().is_empty() {
        Err("stream must not be empty".to_string())
    } else {
        Ok(())
    }
}

fn validate_vector(vector: Option<&[f32]>) -> Result<(), String> {
    if vector.is_some_and(|values| values.is_empty() || values.iter().any(|v| !v.is_finite())) {
        Err("vectors must be non-empty and finite".to_string())
    } else {
        Ok(())
    }
}

fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        f32::INFINITY
    } else {
        1.0 - dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default()
}

#[allow(unsafe_code)]
mod component_export {
    use super::StorageComponent;
    use crate::bindings;

    crate::bindings::export!(StorageComponent with_types_in bindings);
}
