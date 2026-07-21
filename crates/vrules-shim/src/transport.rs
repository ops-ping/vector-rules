use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::StreamExt;
use include_dir::{Dir, include_dir};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::RuntimeHost;
use crate::host::{CacheError, Identity as ComponentIdentity, ResolveError};
use crate::{cache_key, cache_key::CacheKey};

static CONSOLE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../apps/console/dist");

pub type Identity = ComponentIdentity;

#[derive(Debug, Clone, Copy)]
pub struct DaemonConfig {
    pub bind: SocketAddr,
}

pub fn run_stdio(host: RuntimeHost) -> Result<()> {
    let identity = process_identity();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.context("read MCP stdin")?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match host.mcp(&line, &identity) {
            Ok(response) => response,
            Err(error) => {
                eprintln!("MCP component error: {error}");
                mcp_transport_error(&line, &error.to_string())
            }
        };
        if let Some(response) = response {
            stdout
                .write_all(response.as_bytes())
                .context("write MCP stdout")?;
            stdout.write_all(b"\n").context("terminate MCP response")?;
            stdout.flush().context("flush MCP stdout")?;
        }
    }
    Ok(())
}

pub async fn run_daemon(host: RuntimeHost, config: DaemonConfig) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("bind daemon at {}", config.bind))?;
    eprintln!("vrules-shim daemon listening on http://{}", config.bind);
    serve(listener, host).await
}

/// Serve the daemon on an already-bound listener (integration tests bind
/// `127.0.0.1:0` and read the local address before calling this).
pub async fn serve(listener: tokio::net::TcpListener, host: RuntimeHost) -> Result<()> {
    let state = AppState {
        host: Arc::new(Mutex::new(host)),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/mcp", get(mcp))
        .route(
            "/vrules-rest/v1/embeddings/{model}/{canon}/{hash}",
            get(tier_lookup).put(tier_store),
        )
        .route(
            "/vrules-rest/v1/embeddings/{model}/{canon}",
            get(tier_resolve_query).post(tier_resolve_body),
        )
        .route("/vrules-rest/v1/expire", post(tier_expire))
        .route("/vrules-rest/v1/cache/stats", get(cache_stats))
        .route("/vrules-rest/v1/storage/stats", get(storage_stats))
        .route("/vrules-rest/v1/tools/stats", get(tools_stats))
        .route("/vrules-rest/v1/log", get(log_scan))
        .route("/vrules-rest/v1/log/search", get(log_search))
        .route("/vrules-rest/v1/sessions", get(sessions_list))
        .route("/vrules-rest/v1/test/run", post(test_run))
        .route("/vrules-rest/v1/rules", get(rules_list))
        .route("/vrules-rest/v1/rules/branches", get(rules_branches))
        .route("/vrules-rest/v1/rules/diff", get(rules_diff))
        .route("/vrules-rest/v1/rules/compare", get(rules_compare))
        .route("/vrules-rest/v1/rules/validate", post(rules_validate))
        .route("/vrules-rest/v1/rules/promote", post(rules_promote))
        .route("/vrules-rest/v1/ab", post(ab_run))
        .route("/vrules-rest/v1/embedding/info", get(embedding_info))
        .route("/vrules-rest/v1/embedding", post(embedding_embed))
        .route("/vrules-rest/v1/whatif/assert", post(whatif_assert))
        .route("/vrules-rest/v1/whatif/prove", post(whatif_prove))
        .route("/vrules-rest/v1/memory/search", post(memory_search))
        .route("/vrules-rest/v1/memory/stats", get(memory_stats))
        .route("/vrules-rest/v1/memory/{id}/history", get(memory_history))
        .route("/vrules-rest/v1/memory", post(memory_write))
        .route(
            "/vrules-rest/v1/memory/{id}",
            put(memory_update).delete(memory_delete),
        )
        .fallback(static_asset)
        .with_state(state);
    axum::serve(listener, app).await.context("serve daemon")
}

#[derive(Clone)]
struct AppState {
    host: Arc<Mutex<RuntimeHost>>,
}

#[derive(Debug, Deserialize)]
struct IdentityQuery {
    session: Option<String>,
    child: Option<String>,
    proc: Option<String>,
    context: Option<String>,
}

async fn health() -> &'static str {
    "ok"
}

/// Run a blocking closure against the locked runtime host. The outer error is
/// infrastructure failure (poisoned lock, join error) and maps to HTTP 500.
async fn with_host<T, F>(state: AppState, work: F) -> Result<T, String>
where
    F: FnOnce(&RuntimeHost) -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let host = state
            .host
            .lock()
            .map_err(|_| "runtime host lock poisoned".to_string())?;
        Ok(work(&host))
    })
    .await
    .map_err(|error| format!("host task failed: {error}"))?
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn cache_error_response(error: CacheError) -> Response {
    match error {
        CacheError::Unavailable => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "no cache plugin is configured",
        ),
        CacheError::Invalid(message) => error_response(StatusCode::BAD_REQUEST, &message),
        CacheError::Failed(message) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &message),
    }
}

/// Dispatch an admin-component method and shape its outcome as a REST
/// response: the component's result JSON is the body (no wrapper), component
/// errors are 400, infrastructure failures are 500.
async fn admin_call(state: AppState, method: &'static str, params: Value) -> Response {
    let params_string = params.to_string();
    let outcome = with_host(state, move |host| {
        host.admin(method, &params_string)
            .map_err(|e| e.to_string())
    })
    .await;
    match outcome {
        Ok(Ok(payload)) => match serde_json::from_str::<Value>(&payload) {
            Ok(value) => Json(value).into_response(),
            Err(_) => Json(Value::String(payload)).into_response(),
        },
        Ok(Err(component_error)) => error_response(StatusCode::BAD_REQUEST, &component_error),
        Err(infrastructure) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &infrastructure),
    }
}

/// Build a params object from present-only entries.
fn params(entries: &[(&str, Option<Value>)]) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in entries {
        if let Some(value) = value {
            map.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(map)
}

// ---- vrules-rest embedding tier -------------------------------------------

const OCTET_STREAM: &str = "application/octet-stream";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

fn strong_etag(key: &CacheKey, generation: u32) -> String {
    format!("\"{}-{generation}\"", key.to_hex())
}

fn if_none_match_hits(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim() == etag)
}

fn vector_response(
    headers: &HeaderMap,
    key: &CacheKey,
    generation: u32,
    vector: &[f32],
    immutable: bool,
) -> Response {
    let etag = strong_etag(key, generation);
    if if_none_match_hits(headers, &etag) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }
    let mut response = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, OCTET_STREAM.to_string()),
            (header::ETAG, etag),
        ],
        cache_key::vector_to_le_bytes(vector),
    )
        .into_response();
    if immutable {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static(IMMUTABLE),
        );
    }
    response
}

/// GET `/vrules-rest/v1/embeddings/{model}/{canon}/{hash}` — immutable local
/// lookup by content hash; never computes or cascades.
async fn tier_lookup(
    State(state): State<AppState>,
    Path((model, canon, hash)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let outcome = with_host(state, move |host| host.cache_lookup(&model, &canon, &hash)).await;
    match outcome {
        Ok(Ok(Some((key, vector, generation)))) => {
            vector_response(&headers, &key, generation, &vector, true)
        }
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "embedding is not cached"),
        Ok(Err(error)) => cache_error_response(error),
        Err(infrastructure) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &infrastructure),
    }
}

/// PUT `/vrules-rest/v1/embeddings/{model}/{canon}/{hash}` — write-up receiver
/// for a downstream node's computed vector (lossless little-endian f32 body).
async fn tier_store(
    State(state): State<AppState>,
    Path((model, canon, hash)): Path<(String, String, String)>,
    body: Bytes,
) -> Response {
    let Some(vector) = cache_key::vector_from_le_bytes(&body) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "body must be a non-empty whole number of little-endian f32 values",
        );
    };
    if vector.iter().any(|value| !value.is_finite()) {
        return error_response(StatusCode::BAD_REQUEST, "vector values must be finite");
    }
    let outcome = with_host(state, move |host| {
        host.cache_store(&model, &canon, &hash, vector)
    })
    .await;
    match outcome {
        Ok(Ok(generation)) => (
            StatusCode::CREATED,
            Json(json!({ "generation": generation })),
        )
            .into_response(),
        Ok(Err(error)) => cache_error_response(error),
        Err(infrastructure) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &infrastructure),
    }
}

#[derive(Debug, Deserialize)]
struct TextQuery {
    text: String,
}

/// Compute-on-miss outcome: infrastructure error outside, resolution inside.
type ResolveOutcome = Result<Result<(CacheKey, Vec<f32>, u32), ResolveError>, String>;

fn resolve_response(headers: &HeaderMap, outcome: ResolveOutcome, immutable: bool) -> Response {
    match outcome {
        Ok(Ok((key, vector, generation))) => {
            vector_response(headers, &key, generation, &vector, immutable)
        }
        Ok(Err(ResolveError::ModelMismatch { active })) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "model does not match the active embedding model",
                "active_model": active,
            })),
        )
            .into_response(),
        Ok(Err(ResolveError::Failed(message))) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, &message)
        }
        Err(infrastructure) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &infrastructure),
    }
}

/// GET `/vrules-rest/v1/embeddings/{model}/{canon}?text=` — compute-on-miss
/// for browser and remote readers.
async fn tier_resolve_query(
    State(state): State<AppState>,
    Path((model, canon)): Path<(String, String)>,
    Query(query): Query<TextQuery>,
    headers: HeaderMap,
) -> Response {
    if query.text.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "text must not be empty");
    }
    let outcome = with_host(state, move |host| {
        host.resolve_embedding(&model, &canon, &query.text)
    })
    .await;
    resolve_response(&headers, outcome, true)
}

/// POST `/vrules-rest/v1/embeddings/{model}/{canon}` — compute-on-miss with
/// the canon text in the body.
async fn tier_resolve_body(
    State(state): State<AppState>,
    Path((model, canon)): Path<(String, String)>,
    headers: HeaderMap,
    text: String,
) -> Response {
    if text.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "text must not be empty");
    }
    let outcome = with_host(state, move |host| {
        host.resolve_embedding(&model, &canon, &text)
    })
    .await;
    resolve_response(&headers, outcome, false)
}

/// POST `/vrules-rest/v1/expire` — bump the cache epoch (append-only
/// rule-driven mass invalidation).
async fn tier_expire(State(state): State<AppState>) -> Response {
    let outcome = with_host(state, |host| host.cache_expire()).await;
    match outcome {
        Ok(Ok(epoch)) => Json(json!({ "epoch": epoch })).into_response(),
        Ok(Err(error)) => cache_error_response(error),
        Err(infrastructure) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &infrastructure),
    }
}

async fn cache_stats(State(state): State<AppState>) -> Response {
    let outcome = with_host(state, |host| host.cache_stats()).await;
    match outcome {
        Ok(Ok(payload)) => match serde_json::from_str::<Value>(&payload) {
            Ok(value) => Json(value).into_response(),
            Err(_) => Json(Value::String(payload)).into_response(),
        },
        Ok(Err(error)) => cache_error_response(error),
        Err(infrastructure) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &infrastructure),
    }
}

// ---- admin REST routes ----------------------------------------------------

#[derive(Debug, Deserialize)]
struct LogQuery {
    limit: Option<usize>,
    session: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    query: String,
    k: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RulesetQuery {
    ruleset: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PairQuery {
    a: String,
    b: String,
}

#[derive(Debug, Deserialize)]
struct MemoryStatsQuery {
    memory_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReasonQuery {
    reason: Option<String>,
}

async fn storage_stats(State(state): State<AppState>) -> Response {
    admin_call(state, "storage.stats", json!({})).await
}

async fn tools_stats(State(state): State<AppState>) -> Response {
    admin_call(state, "tools.stats", json!({})).await
}

async fn log_scan(State(state): State<AppState>, Query(query): Query<LogQuery>) -> Response {
    let entries = [
        ("limit", query.limit.map(Value::from)),
        ("session", query.session.map(Value::from)),
    ];
    admin_call(state, "log.scan", params(&entries)).await
}

async fn log_search(State(state): State<AppState>, Query(query): Query<SearchQuery>) -> Response {
    let entries = [
        ("query", Some(Value::from(query.query))),
        ("k", query.k.map(Value::from)),
    ];
    admin_call(state, "log.search", params(&entries)).await
}

async fn sessions_list(State(state): State<AppState>) -> Response {
    admin_call(state, "sessions.list", json!({})).await
}

async fn test_run(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    admin_call(state, "test.run", body).await
}

async fn rules_list(State(state): State<AppState>, Query(query): Query<RulesetQuery>) -> Response {
    let entries = [("ruleset", query.ruleset.map(Value::from))];
    admin_call(state, "rules.list", params(&entries)).await
}

async fn rules_branches(State(state): State<AppState>) -> Response {
    admin_call(state, "rules.branches", json!({})).await
}

async fn rules_diff(State(state): State<AppState>, Query(query): Query<PairQuery>) -> Response {
    admin_call(state, "rules.diff", json!({ "a": query.a, "b": query.b })).await
}

async fn rules_compare(State(state): State<AppState>, Query(query): Query<PairQuery>) -> Response {
    admin_call(
        state,
        "rules.compare",
        json!({ "a": query.a, "b": query.b }),
    )
    .await
}

async fn rules_validate(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    admin_call(state, "rules.validate", body).await
}

async fn rules_promote(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    admin_call(state, "rules.promote", body).await
}

async fn ab_run(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    admin_call(state, "ab.run", body).await
}

async fn embedding_info(State(state): State<AppState>) -> Response {
    admin_call(state, "embedding.info", json!({})).await
}

async fn embedding_embed(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    admin_call(state, "embedding.embed", body).await
}

async fn whatif_assert(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    admin_call(state, "whatif.assert", body).await
}

async fn whatif_prove(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    admin_call(state, "whatif.prove", body).await
}

async fn memory_search(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    admin_call(state, "memory.search", body).await
}

async fn memory_stats(
    State(state): State<AppState>,
    Query(query): Query<MemoryStatsQuery>,
) -> Response {
    let entries = [("memory_id", query.memory_id.map(Value::from))];
    admin_call(state, "memory.stats", params(&entries)).await
}

async fn memory_history(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    admin_call(state, "memory.history", json!({ "id": id })).await
}

async fn memory_write(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    admin_call(state, "memory.write", body).await
}

async fn memory_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Value::Object(mut fields) = body else {
        return error_response(StatusCode::BAD_REQUEST, "body must be a JSON object");
    };
    fields.insert("id".to_string(), Value::String(id));
    admin_call(state, "memory.update", Value::Object(fields)).await
}

async fn memory_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ReasonQuery>,
) -> Response {
    let entries = [
        ("id", Some(Value::String(id))),
        ("reason", query.reason.map(Value::from)),
    ];
    admin_call(state, "memory.delete", params(&entries)).await
}

async fn mcp(
    ws: WebSocketUpgrade,
    Query(query): Query<IdentityQuery>,
    State(state): State<AppState>,
) -> Response {
    let identity = Identity {
        session_id: query
            .session
            .unwrap_or_else(|| "unknown-session".to_string()),
        child_session: query.child,
        process_id: query.proc.unwrap_or_else(|| Uuid::new_v4().to_string()),
        context: query.context.filter(|value| !value.trim().is_empty()),
    };
    ws.on_upgrade(move |socket| mcp_socket(socket, state, identity))
}

async fn mcp_socket(mut socket: WebSocket, state: AppState, identity: Identity) {
    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Text(text) => {
                let state = state.clone();
                let identity = identity.clone();
                let request = text.to_string();
                let request_for_call = request.clone();
                let response = tokio::task::spawn_blocking(move || {
                    state
                        .host
                        .lock()
                        .map_err(|_| anyhow!("runtime host lock poisoned"))?
                        .mcp(&request_for_call, &identity)
                })
                .await;
                match response {
                    Ok(Ok(Some(response))) => {
                        if socket.send(Message::Text(response.into())).await.is_err() {
                            break;
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        eprintln!("daemon MCP component error: {error}");
                        if let Some(response) = mcp_transport_error(&request, &error.to_string())
                            && socket.send(Message::Text(response.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        eprintln!("daemon MCP task error: {error}");
                        if let Some(response) = mcp_transport_error(&request, "MCP task failed")
                            && socket.send(Message::Text(response.into())).await.is_err()
                        {
                            break;
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

fn mcp_transport_error(message: &str, detail: &str) -> Option<String> {
    let request = match serde_json::from_str::<Value>(message) {
        Ok(request) => request,
        Err(_) => {
            return Some(
                json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": "parse error" },
                })
                .to_string(),
            );
        }
    };
    if !request.is_object() {
        return Some(
            json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32600, "message": "invalid request" },
            })
            .to_string(),
        );
    }
    let id = request.get("id")?.clone();
    Some(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32603, "message": detail },
        })
        .to_string(),
    )
}

async fn static_asset(method: axum::http::Method, uri: Uri) -> Response {
    // The fallback exists only to serve the embedded PWA shell; answering
    // writes with HTML would mask a mistyped API route.
    if method != axum::http::Method::GET && method != axum::http::Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, "method not allowed").into_response();
    }
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let asset = CONSOLE
        .get_file(path)
        .or_else(|| CONSOLE.get_file("index.html"));
    match asset {
        Some(file) => {
            let content_type = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, content_type)],
                file.contents(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn process_identity() -> Identity {
    Identity {
        session_id: env_first(
            &["VRULES_SESSION_ID", "CLAUDE_CODE_SESSION_ID"],
            "unknown-session",
        ),
        child_session: env_first_opt(&["VRULES_CHILD_SESSION", "CLAUDE_CODE_CHILD_SESSION"]),
        process_id: Uuid::new_v4().to_string(),
        context: env_first_opt(&["VRULES_CONTEXT", "VRULES_PROFILE"]),
    }
}

fn env_first(keys: &[&str], default: &str) -> String {
    env_first_opt(keys).unwrap_or_else(|| default.to_string())
}

fn env_first_opt(keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::mcp_transport_error;

    #[test]
    fn malformed_request_gets_parse_error() {
        let response = mcp_transport_error("{", "component failed").expect("response");
        let response: Value = serde_json::from_str(&response).expect("valid JSON");
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], -32700);
    }

    #[test]
    fn request_error_preserves_id() {
        let response = mcp_transport_error(
            r#"{"jsonrpc":"2.0","id":"request-7","method":"tools/list"}"#,
            "component failed",
        )
        .expect("response");
        let response: Value = serde_json::from_str(&response).expect("valid JSON");
        assert_eq!(response["id"], "request-7");
        assert_eq!(response["error"]["code"], -32603);
    }

    #[test]
    fn notification_error_has_no_response() {
        assert!(
            mcp_transport_error(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                "component failed",
            )
            .is_none()
        );
    }
}
