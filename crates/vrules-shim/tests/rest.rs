//! End-to-end tests for the daemon's `/vrules-rest/v1` surface: cache
//! cascade, epoch expiry, write-up, upstream tiering, and the admin routes.
//!
//! The suite boots the real component runtime — wllama with a real
//! EmbeddingGemma GGUF, never a stub — so it is gated on an environment
//! variable and skips (passing) when the prerequisites are absent:
//!
//! ```sh
//! release/build-components.sh                       # target/vrules-components
//! VRULES_TEST_COMPONENTS=target/vrules-components \
//!   cargo test -p vrules-shim --test rest -- --nocapture
//! ```
//!
//! `VRULES_TEST_MODEL` overrides the GGUF path (default:
//! `~/.local/share/vrules/models/embeddinggemma-300M-F32.gguf`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vrules_shim::{CacheKey, ComponentManifest, ComponentOutput, ManifestPath, RuntimeHost, serve};

const CANON: &str = "default/v1";

struct TestDaemon {
    base: String,
    runtime: tokio::runtime::Runtime,
    _root: tempfile::TempDir,
}

fn components_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("VRULES_TEST_COMPONENTS")?);
    Some(dir.canonicalize().expect("resolve VRULES_TEST_COMPONENTS"))
}

fn model_path() -> PathBuf {
    std::env::var_os("VRULES_TEST_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").expect("HOME is set");
            PathBuf::from(home).join(".local/share/vrules/models/embeddinggemma-300M-F32.gguf")
        })
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create copy target");
    for entry in std::fs::read_dir(from).expect("read copy source") {
        let entry = entry.expect("read dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("entry type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// Boot a full component daemon on an ephemeral port. `None` when the
/// environment lacks the built components or the real model.
fn boot(components: &Path, upstream: Option<String>) -> TestDaemon {
    let model = model_path();
    let root = tempfile::tempdir().expect("create daemon root");
    for dir in ["data", "cache"] {
        std::fs::create_dir(root.path().join(dir)).expect("create preopen dir");
    }
    let rules = root.path().join("rules");
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    copy_dir(&repo_root.join("shared-rules"), &rules.join("shared-rules"));
    git(&rules, &["init", "-q", "-b", "main"]);
    git(&rules, &["config", "user.name", "vrules-test"]);
    git(&rules, &["config", "user.email", "test@vrules.invalid"]);
    git(&rules, &["add", "shared-rules"]);
    git(&rules, &["commit", "-q", "-m", "rules baseline"]);

    let wasm = |name: &str| components.join(name);
    let manifest_json = json!({
        "runtime": {
            "path": wasm("vrules-runtime.wasm"),
            "config": { "rules_plugin": "rules", "storage_plugin": "storage", "cache_ttl_secs": 300 }
        },
        "embedding": {
            "path": wasm("vrules-embedding-wllama.wasm"),
            "config": { "context_size": 2048 }
        },
        "admin_plugin": "admin",
        "cache_plugin": "cache",
        "plugins": [
            {
                "id": "rules",
                "path": wasm("vrules-rules.wasm"),
                "config": {
                    "rules_dir": "/rules/shared-rules",
                    "directories": ["proxy"],
                    "tools_file": "proxy/tools.json",
                    "repository_dir": "/rules",
                    "repository_rules_path": "shared-rules"
                },
                "preopens": [{ "host": rules, "guest": "/rules", "read_only": false }]
            },
            {
                "id": "storage",
                "path": wasm("vrules-storage.wasm"),
                "config": { "data_dir": "/data" },
                "preopens": [{ "host": root.path().join("data"), "guest": "/data", "read_only": false }]
            },
            {
                "id": "admin",
                "path": wasm("vrules-admin.wasm"),
                "config": { "rules_plugin": "rules", "storage_plugin": "storage" }
            },
            {
                "id": "cache",
                "path": wasm("vrules-cache.wasm"),
                "config": { "cache_dir": "/cache" },
                "preopens": [{ "host": root.path().join("cache"), "guest": "/cache", "read_only": false }]
            }
        ]
    });
    let manifest_path = root.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest_json).expect("encode manifest"),
    )
    .expect("write manifest");

    let path = ManifestPath::resolve(Some(manifest_path)).expect("resolve manifest path");
    let mut manifest = ComponentManifest::load(&path).expect("load manifest");
    manifest
        .use_embedding_model(&model, None)
        .expect("mount embedding model");
    let host =
        RuntimeHost::load(manifest, ComponentOutput::Daemon, upstream).expect("load runtime host");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind daemon listener");
    let address = listener.local_addr().expect("daemon local address");
    runtime.spawn(async move {
        if let Err(error) = serve(listener, host).await {
            eprintln!("test daemon exited: {error}");
        }
    });
    TestDaemon {
        base: format!("http://{address}"),
        runtime,
        _root: root,
    }
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("build test client")
}

fn etag_of(response: &reqwest::blocking::Response) -> String {
    response
        .headers()
        .get(reqwest::header::ETAG)
        .expect("ETag header")
        .to_str()
        .expect("ETag is ASCII")
        .to_string()
}

/// `"{key64}-{generation}"` → (key hex, text-hash segment, generation).
fn parse_etag(etag: &str) -> (String, String, u32) {
    let inner = etag.trim_matches('"');
    let (key, generation) = inner.split_at(64);
    (
        key.to_string(),
        key[32..64].to_string(),
        generation
            .trim_start_matches('-')
            .parse()
            .expect("generation in ETag"),
    )
}

type UpstreamStore = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// A minimal parent tier: GET serves stored vectors, PUT records write-ups.
fn mock_upstream(runtime: &tokio::runtime::Runtime, store: UpstreamStore) -> SocketAddr {
    use axum::body::Bytes;
    use axum::extract::{Path as AxumPath, State};
    use axum::http::StatusCode;
    use axum::routing::get;

    async fn lookup(
        State(store): State<UpstreamStore>,
        AxumPath((model, canon, hash)): AxumPath<(String, String, String)>,
    ) -> (StatusCode, Vec<u8>) {
        let store = store.lock().expect("upstream store lock");
        match store.get(&format!("{model}/{canon}/{hash}")) {
            Some(bytes) => (StatusCode::OK, bytes.clone()),
            None => (StatusCode::NOT_FOUND, Vec::new()),
        }
    }

    async fn write_up(
        State(store): State<UpstreamStore>,
        AxumPath((model, canon, hash)): AxumPath<(String, String, String)>,
        body: Bytes,
    ) -> StatusCode {
        store
            .lock()
            .expect("upstream store lock")
            .insert(format!("upload:{model}/{canon}/{hash}"), body.to_vec());
        StatusCode::CREATED
    }

    let app = axum::Router::new()
        .route(
            "/vrules-rest/v1/embeddings/{model}/{canon}/{hash}",
            get(lookup).put(write_up),
        )
        .with_state(store);
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind mock upstream");
    let address = listener.local_addr().expect("mock upstream address");
    runtime.spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    address
}

#[test]
fn vrules_rest_surface() {
    let Some(components) = components_dir() else {
        eprintln!("skipping: VRULES_TEST_COMPONENTS is not set");
        return;
    };
    if !model_path().is_file() {
        eprintln!(
            "skipping: embedding model {} absent",
            model_path().display()
        );
        return;
    }

    cascade_expire_and_admin(&components);
    upstream_tier(&components);
}

/// Cascade (miss → embed → write → hit), epoch expiry, write-up, model
/// mismatch, and the admin REST routes — one daemon boot.
fn cascade_expire_and_admin(components: &Path) {
    let daemon = boot(components, None);
    let client = client();
    let base = &daemon.base;
    let canon = vrules_shim::cache_key::encode_path_segment(CANON);
    let canon = canon.as_str();

    // Liveness + the removed /rpc endpoint stays removed (the GET/HEAD-only
    // PWA fallback answers writes with 405, never JSON-RPC).
    assert!(
        client
            .get(format!("{base}/health"))
            .send()
            .expect("health")
            .status()
            .is_success()
    );
    assert_eq!(
        client
            .post(format!("{base}/rpc"))
            .json(&json!({ "method": "storage.stats", "params": {} }))
            .send()
            .expect("rpc probe")
            .status(),
        reqwest::StatusCode::METHOD_NOT_ALLOWED
    );

    // Active model identity drives the tier's {model} segment.
    let info: Value = client
        .get(format!("{base}/vrules-rest/v1/embedding/info"))
        .send()
        .expect("embedding info")
        .json()
        .expect("embedding info JSON");
    let revision = info["revision"].as_str().expect("revision").to_string();
    let dims = info["dimensions"].as_u64().expect("dimensions") as usize;

    // Cascade: compute-on-miss, then bit-identical immutable lookup by hash.
    let text = "the cache cascade test phrase";
    let resolve = client
        .post(format!(
            "{base}/vrules-rest/v1/embeddings/{revision}/{canon}"
        ))
        .body(text)
        .send()
        .expect("resolve");
    assert_eq!(resolve.status(), reqwest::StatusCode::OK);
    let etag = etag_of(&resolve);
    let (_, hash, generation) = parse_etag(&etag);
    assert_eq!(generation, 0);
    let resolved_bytes = resolve.bytes().expect("resolve body").to_vec();
    assert_eq!(resolved_bytes.len(), dims * 4);

    let lookup_url = format!("{base}/vrules-rest/v1/embeddings/{revision}/{canon}/{hash}");
    let lookup = client.get(&lookup_url).send().expect("lookup");
    assert_eq!(lookup.status(), reqwest::StatusCode::OK);
    assert_eq!(etag_of(&lookup), etag);
    assert_eq!(
        lookup.bytes().expect("lookup body").to_vec(),
        resolved_bytes
    );

    // Conditional revalidation.
    let revalidated = client
        .get(&lookup_url)
        .header(reqwest::header::IF_NONE_MATCH, &etag)
        .send()
        .expect("revalidate");
    assert_eq!(revalidated.status(), reqwest::StatusCode::NOT_MODIFIED);

    // Epoch expiry: the entry goes dark, re-resolution stamps generation 1,
    // the stale ETag stops matching.
    let expired: Value = client
        .post(format!("{base}/vrules-rest/v1/expire"))
        .send()
        .expect("expire")
        .json()
        .expect("expire JSON");
    assert_eq!(expired["epoch"], 1);
    assert_eq!(
        client
            .get(&lookup_url)
            .send()
            .expect("post-expire lookup")
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let refreshed = client
        .get(format!(
            "{base}/vrules-rest/v1/embeddings/{revision}/{canon}?text={}",
            vrules_shim::cache_key::encode_path_segment(text)
        ))
        .send()
        .expect("re-resolve");
    assert_eq!(refreshed.status(), reqwest::StatusCode::OK);
    let fresh_etag = etag_of(&refreshed);
    let (_, fresh_hash, fresh_generation) = parse_etag(&fresh_etag);
    assert_eq!(fresh_hash, hash, "same text, same content hash");
    assert_eq!(fresh_generation, 1);
    let stale = client
        .get(&lookup_url)
        .header(reqwest::header::IF_NONE_MATCH, &etag)
        .send()
        .expect("stale revalidate");
    assert_eq!(stale.status(), reqwest::StatusCode::OK);

    // Write-up receiver stores a foreign vector retrievable by hash.
    let put_hash = "ab".repeat(16);
    let put_vector: Vec<f32> = (0..dims).map(|i| i as f32 / dims as f32).collect();
    let put_bytes: Vec<u8> = put_vector.iter().flat_map(|v| v.to_le_bytes()).collect();
    let put_url = format!("{base}/vrules-rest/v1/embeddings/{revision}/{canon}/{put_hash}");
    let put = client
        .put(&put_url)
        .body(put_bytes.clone())
        .send()
        .expect("write-up");
    assert_eq!(put.status(), reqwest::StatusCode::CREATED);
    let echoed = client.get(&put_url).send().expect("write-up lookup");
    assert_eq!(echoed.status(), reqwest::StatusCode::OK);
    assert_eq!(echoed.bytes().expect("write-up body").to_vec(), put_bytes);

    // Malformed hash and foreign-model compute-on-miss.
    assert_eq!(
        client
            .get(format!(
                "{base}/vrules-rest/v1/embeddings/{revision}/{canon}/nothex"
            ))
            .send()
            .expect("bad hash")
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let mismatch = client
        .post(format!(
            "{base}/vrules-rest/v1/embeddings/not-the-active-model/{canon}"
        ))
        .body("text")
        .send()
        .expect("model mismatch");
    assert_eq!(mismatch.status(), reqwest::StatusCode::CONFLICT);
    let mismatch: Value = mismatch.json().expect("mismatch JSON");
    assert_eq!(mismatch["active_model"], revision.as_str());

    // The internal embed path shares the same cache: embedding.embed via the
    // admin route must land entries in cache stats.
    let embed: Value = client
        .post(format!("{base}/vrules-rest/v1/embedding"))
        .json(&json!({ "text": "internal path phrase" }))
        .send()
        .expect("admin embed")
        .json()
        .expect("admin embed JSON");
    assert_eq!(embed["info"]["revision"], revision.as_str());
    assert_eq!(embed["vector"].as_array().expect("vector").len(), dims);
    let stats: Value = client
        .get(format!("{base}/vrules-rest/v1/cache/stats"))
        .send()
        .expect("cache stats")
        .json()
        .expect("cache stats JSON");
    assert!(stats["entries"].as_u64().expect("entries") >= 3);
    assert_eq!(stats["epoch"], 1);

    // Admin smoke across GET and POST dispatch, plus a component-level 400.
    let storage: Value = client
        .get(format!("{base}/vrules-rest/v1/storage/stats"))
        .send()
        .expect("storage stats")
        .json()
        .expect("storage stats JSON");
    assert!(storage["events"].is_u64());
    let branches: Value = client
        .get(format!("{base}/vrules-rest/v1/rules/branches"))
        .send()
        .expect("rules branches")
        .json()
        .expect("rules branches JSON");
    assert!(branches.to_string().contains("main"));
    assert_eq!(
        client
            .post(format!("{base}/vrules-rest/v1/embedding"))
            .json(&json!({}))
            .send()
            .expect("embed without text")
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    drop(client);
    daemon.runtime.shutdown_background();
}

/// Tier-up: pull-through serves the parent's vector without local inference;
/// local computes write up; a dead parent falls back to local inference.
fn upstream_tier(components: &Path) {
    let store: UpstreamStore = Arc::new(Mutex::new(HashMap::new()));
    let bootstrap = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build mock runtime");
    let upstream_address = mock_upstream(&bootstrap, Arc::clone(&store));

    let daemon = boot(components, Some(upstream_address.to_string()));
    let client = client();
    let base = &daemon.base;
    let canon = vrules_shim::cache_key::encode_path_segment(CANON);
    let canon = canon.as_str();

    let info: Value = client
        .get(format!("{base}/vrules-rest/v1/embedding/info"))
        .send()
        .expect("embedding info")
        .json()
        .expect("embedding info JSON");
    let revision = info["revision"].as_str().expect("revision").to_string();
    let dims = info["dimensions"].as_u64().expect("dimensions") as usize;

    // Preload the parent with a recognizable vector for "pull me": the child
    // must serve those exact bytes (upstream hit, no local inference).
    let pull_text = "pull me";
    let pull_key = CacheKey::new(&revision, CANON, pull_text);
    let parent_vector: Vec<f32> = (0..dims).map(|i| (i % 7) as f32).collect();
    let parent_bytes: Vec<u8> = parent_vector.iter().flat_map(|v| v.to_le_bytes()).collect();
    // The daemon percent-encodes the canon segment; axum decodes it back, so
    // the mock's key is the decoded form.
    store.lock().expect("seed upstream").insert(
        format!("{revision}/{CANON}/{}", pull_key.text_hash_hex()),
        parent_bytes.clone(),
    );

    let pulled = client
        .post(format!(
            "{base}/vrules-rest/v1/embeddings/{revision}/{canon}"
        ))
        .body(pull_text)
        .send()
        .expect("pull-through resolve");
    assert_eq!(pulled.status(), reqwest::StatusCode::OK);
    assert_eq!(
        pulled.bytes().expect("pull-through body").to_vec(),
        parent_bytes,
        "vector must come from the parent tier, not local inference"
    );

    // A locally computed vector is written up to the parent.
    let local_text = "computed locally and written up";
    let local_key = CacheKey::new(&revision, CANON, local_text);
    let computed = client
        .post(format!(
            "{base}/vrules-rest/v1/embeddings/{revision}/{canon}"
        ))
        .body(local_text)
        .send()
        .expect("local resolve");
    assert_eq!(computed.status(), reqwest::StatusCode::OK);
    let computed_bytes = computed.bytes().expect("local body").to_vec();
    let uploaded = store
        .lock()
        .expect("read upstream")
        .get(&format!(
            "upload:{revision}/{CANON}/{}",
            local_key.text_hash_hex()
        ))
        .cloned()
        .expect("write-up recorded by the parent tier");
    assert_eq!(uploaded, computed_bytes);

    // Parent down: resolution still succeeds via local inference.
    bootstrap.shutdown_background();
    let fallback = client
        .post(format!(
            "{base}/vrules-rest/v1/embeddings/{revision}/{canon}"
        ))
        .body("parent is unreachable now")
        .send()
        .expect("fallback resolve");
    assert_eq!(fallback.status(), reqwest::StatusCode::OK);
    assert_eq!(fallback.bytes().expect("fallback body").len(), dims * 4);

    drop(client);
    daemon.runtime.shutdown_background();
}
