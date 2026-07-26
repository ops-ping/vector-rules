#![deny(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Map, Value, json};

#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "runtime-component",
    });
}

use bindings::ai::vrules::types::{EmbeddingInfo, Identity, PluginDescriptor};
use bindings::exports::ai::vrules::runtime::Guest;

struct McpRuntime;

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

struct State {
    rules_plugin: String,
    storage_plugin: String,
    cache_ttl_ns: u64,
    tools: Vec<Value>,
    tool_stats: HashMap<String, ToolStat>,
    embedding: EmbeddingInfo,
}

#[derive(Default)]
struct ToolStat {
    calls: u64,
    errors: u64,
    last_called_ns: u64,
}

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default = "default_rules_plugin")]
    rules_plugin: String,
    #[serde(default = "default_storage_plugin")]
    storage_plugin: String,
    #[serde(default = "default_cache_ttl_secs")]
    cache_ttl_secs: u64,
}

impl Guest for McpRuntime {
    fn initialize(
        config: String,
        plugins: Vec<PluginDescriptor>,
        embedding: EmbeddingInfo,
    ) -> Result<(), String> {
        let config: Config =
            serde_json::from_str(&config).map_err(|e| format!("invalid runtime config: {e}"))?;
        require_plugin(&plugins, &config.rules_plugin)?;
        require_plugin(&plugins, &config.storage_plugin)?;
        let tools_payload = invoke(&config.rules_plugin, "tools", "{}")?;
        let mut tools = serde_json::from_str::<Value>(&tools_payload)
            .map_err(|e| format!("decode rules tools: {e}"))?
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        tools.extend(platform_tools());
        STATE
            .set(Mutex::new(State {
                rules_plugin: config.rules_plugin,
                storage_plugin: config.storage_plugin,
                cache_ttl_ns: config.cache_ttl_secs.saturating_mul(1_000_000_000),
                tools,
                tool_stats: HashMap::new(),
                embedding,
            }))
            .map_err(|_| "runtime component is already initialized".to_string())
    }

    fn mcp(message: String, identity: Identity) -> Result<Option<String>, String> {
        let request: Value = match serde_json::from_str(&message) {
            Ok(request) => request,
            Err(error) => {
                return Ok(Some(
                    rpc_error(Value::Null, -32700, &format!("parse error: {error}")).to_string(),
                ));
            }
        };
        if !request.is_object() {
            return Ok(Some(
                rpc_error(Value::Null, -32600, "invalid request").to_string(),
            ));
        }
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let Some(id) = request.get("id").cloned() else {
            return Ok(None);
        };
        let response = match method {
            "initialize" => Ok(rpc_ok(
                id.clone(),
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "vrules",
                        "version": env!("CARGO_PKG_VERSION"),
                    }
                }),
            )),
            "tools/list" => (|| {
                let state = state()?.lock().map_err(|_| "runtime lock poisoned")?;
                let tools = exposed_tools(&state, &identity)?;
                Ok(rpc_ok(id.clone(), json!({ "tools": tools })))
            })(),
            "tools/call" => handle_tool_call(id.clone(), &request, &identity),
            other => Ok(rpc_error(
                id.clone(),
                -32601,
                &format!("method not found: {other}"),
            )),
        }
        .unwrap_or_else(|message| rpc_error(id, -32603, &message));
        Ok(Some(response.to_string()))
    }
}

fn handle_tool_call(id: Value, request: &Value, identity: &Identity) -> Result<Value, String> {
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let started = Instant::now();
    let result = call_tool(&name, &args, identity);
    {
        let mut state = state()?.lock().map_err(|_| "runtime lock poisoned")?;
        let stat = state.tool_stats.entry(name.clone()).or_default();
        stat.calls = stat.calls.saturating_add(1);
        stat.errors = stat.errors.saturating_add(u64::from(result.is_err()));
        stat.last_called_ns = now_ns();
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let activity_error = record_activity(&name, identity, result.is_ok(), elapsed_ms).err();
    if let Some(error) = &activity_error {
        eprintln!("failed to persist tool activity for `{name}`: {error}");
    }
    let metadata = match activity_error {
        Some(error) => json!({ "duration_ms": elapsed_ms, "activity_error": error }),
        None => json!({ "duration_ms": elapsed_ms }),
    };
    Ok(match result {
        Ok(value) => rpc_ok(
            id,
            json!({
                "content": [{ "type": "text", "text": value_text(&value) }],
                "structuredContent": value,
                "isError": false,
                "_meta": metadata,
            }),
        ),
        Err(message) => rpc_ok(
            id,
            json!({
                "content": [{ "type": "text", "text": message }],
                "isError": true,
                "_meta": metadata,
            }),
        ),
    })
}

fn record_activity(
    tool: &str,
    identity: &Identity,
    ok: bool,
    duration_ms: u64,
) -> Result<(), String> {
    let storage = state()?
        .lock()
        .map_err(|_| "runtime lock poisoned")?
        .storage_plugin
        .clone();
    invoke_json(
        &storage,
        "append",
        &json!({
            "stream": "activity",
            "kind": "tool-call",
            "payload": {
                "tool": tool,
                "session_id": identity.session_id,
                "process_id": identity.process_id,
                "ok": ok,
                "duration_ms": duration_ms,
            },
        }),
    )?;
    Ok(())
}

fn call_tool(name: &str, args: &Value, identity: &Identity) -> Result<Value, String> {
    let (rules_plugin, storage_plugin) = {
        let state = state()?.lock().map_err(|_| "runtime lock poisoned")?;
        if !tool_exposed(&state, identity, name)? {
            return Err(format!("tool `{name}` is not exposed in this context"));
        }
        (state.rules_plugin.clone(), state.storage_plugin.clone())
    };
    match name {
        "rules_validate" => invoke_json(
            &rules_plugin,
            "validate",
            &json!({ "grl": args.get("grl").and_then(Value::as_str).unwrap_or("") }),
        ),
        "rules_list" => invoke_json(&rules_plugin, "list", &json!({})),
        "whatif_assert" => {
            let fact_type = args
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("Request");
            invoke_json(
                &rules_plugin,
                "evaluate",
                &json!({
                    "facts": [{
                        "type": fact_type,
                        "data": args.get("facts").cloned().unwrap_or_else(|| json!({})),
                    }],
                    "trace": args.get("trace").and_then(Value::as_bool).unwrap_or(true),
                    "grl": args.get("grl").and_then(Value::as_str),
                }),
            )
        }
        "whatif_prove" => invoke_json(
            &rules_plugin,
            "prove",
            &json!({
                "grl": args.get("grl").and_then(Value::as_str).unwrap_or(""),
                "query": args.get("query").and_then(Value::as_str).unwrap_or(""),
                "facts": args.get("facts").cloned().unwrap_or_else(|| json!({})),
            }),
        ),
        "memory_write" => memory_write(&storage_plugin, args, identity),
        "memory_update" => memory_update(&storage_plugin, args, identity),
        "memory_delete" => memory_delete(&storage_plugin, args, identity),
        "memory_search" => memory_search(&storage_plugin, args, identity),
        "memory_history" => memory_history(&storage_plugin, args, identity),
        "memory_stats" => invoke_json(&storage_plugin, "stats", &json!({})),
        capability => capability_call(&rules_plugin, &storage_plugin, capability, args, identity),
    }
}

fn capability_call(
    rules_plugin: &str,
    storage_plugin: &str,
    tool: &str,
    args: &Value,
    identity: &Identity,
) -> Result<Value, String> {
    let text = args
        .get("query")
        .or_else(|| args.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let effort = args.get("effort").and_then(Value::as_str).unwrap_or("low");
    let evaluation = invoke_json(
        rules_plugin,
        "evaluate",
        &json!({
            "facts": [{
                "type": "Request",
                "data": {
                    "tool": tool,
                    "query": args.get("query").cloned().unwrap_or(Value::Null),
                    "content": args.get("content").cloned().unwrap_or(Value::Null),
                    "query_len": args.get("query").and_then(Value::as_str).map(str::len).unwrap_or(0),
                    "content_len": args.get("content").and_then(Value::as_str).map(str::len).unwrap_or(0),
                    "effort": effort,
                },
            }],
            "trace": true,
        }),
    )?;
    let decision = evaluation
        .get("decision")
        .and_then(Value::as_object)
        .ok_or("rules evaluation did not return a decision")?;
    let backend = decision
        .get("backend")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("no rule routed tool `{tool}` to a backend"))?;
    let downstream = decision.get("tool").and_then(Value::as_str).unwrap_or(tool);
    let routed_effort = decision
        .get("effort")
        .and_then(Value::as_str)
        .unwrap_or(effort);
    let mut downstream_args = args.clone();
    if let Some(object) = downstream_args.as_object_mut() {
        object.insert(
            "effort".to_string(),
            Value::String(routed_effort.to_string()),
        );
    }
    let cache_id = cache_id(
        tool,
        backend,
        downstream,
        routed_effort,
        text,
        &downstream_args,
    );
    if let Some(value) = cache_get(storage_plugin, &cache_id)? {
        audit(
            storage_plugin,
            identity,
            tool,
            args,
            &value,
            backend,
            downstream,
            routed_effort,
            "hit",
            &evaluation,
        )?;
        return Ok(value);
    }

    let provider = invoke_json(backend, downstream, &downstream_args)?;
    cache_put(storage_plugin, &cache_id, &provider)?;
    audit(
        storage_plugin,
        identity,
        tool,
        args,
        &provider,
        backend,
        downstream,
        routed_effort,
        "miss",
        &evaluation,
    )?;
    Ok(provider)
}

#[allow(clippy::too_many_arguments)]
fn audit(
    storage_plugin: &str,
    identity: &Identity,
    tool: &str,
    args: &Value,
    result: &Value,
    backend: &str,
    downstream: &str,
    effort: &str,
    cache: &str,
    evaluation: &Value,
) -> Result<(), String> {
    let semantic_text = args
        .get("query")
        .or_else(|| args.get("content"))
        .and_then(Value::as_str)
        .unwrap_or(tool);
    let vector = embed(semantic_text)?;
    invoke_json(
        storage_plugin,
        "append",
        &json!({
            "stream": "audit",
            "kind": "execution",
            "payload": {
                "session_id": identity.session_id,
                "child_session": identity.child_session,
                "process_id": identity.process_id,
                "context": identity.context,
                "tool": tool,
                "args": args,
                "result": result,
                "backend": backend,
                "downstream": downstream,
                "effort": effort,
                "cache": cache,
                "fired": evaluation.get("fired").cloned().unwrap_or(Value::Null),
                "trace": evaluation.get("trace").cloned().unwrap_or(Value::Null),
            },
            "vector": vector,
            "embedding_model": embedding_revision()?,
        }),
    )?;
    Ok(())
}

fn cache_get(storage_plugin: &str, id: &str) -> Result<Option<Value>, String> {
    let ttl = state()?
        .lock()
        .map_err(|_| "runtime lock poisoned")?
        .cache_ttl_ns;
    let response = invoke_json(
        storage_plugin,
        "scan",
        &json!({ "stream": "cache", "limit": 10_000 }),
    )?;
    let now = now_ns();
    Ok(response
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|event| event["payload"]["cache_id"].as_str() == Some(id))
        .filter(|event| {
            event["timestamp_ns"]
                .as_u64()
                .is_some_and(|timestamp| now.saturating_sub(timestamp) <= ttl)
        })
        .and_then(|event| event["payload"].get("value").cloned()))
}

fn cache_put(storage_plugin: &str, id: &str, value: &Value) -> Result<(), String> {
    invoke_json(
        storage_plugin,
        "append",
        &json!({
            "stream": "cache",
            "kind": "value",
            "payload": { "cache_id": id, "value": value },
        }),
    )?;
    Ok(())
}

fn memory_write(storage_plugin: &str, args: &Value, identity: &Identity) -> Result<Value, String> {
    let fact = required_string(args, "fact")?;
    let vector = embed(fact)?;
    let event = invoke_json(
        storage_plugin,
        "append",
        &json!({
            "stream": "memory",
            "kind": "write",
            "payload": {
                "fact": fact,
                "tags": string_array(args.get("tags")),
                "source": format!("agent:{}", identity.session_id),
            },
            "vector": vector,
            "embedding_model": embedding_revision()?,
        }),
    )?;
    Ok(json!({ "id": event["id"] }))
}

fn memory_update(storage_plugin: &str, args: &Value, identity: &Identity) -> Result<Value, String> {
    let id = required_string(args, "id")?;
    let fact = required_string(args, "fact")?;
    let vector = embed(fact)?;
    let event = invoke_json(
        storage_plugin,
        "append",
        &json!({
            "stream": "memory",
            "kind": "update",
            "payload": {
                "fact": fact,
                "tags": string_array(args.get("tags")),
                "source": format!("agent:{}", identity.session_id),
            },
            "vector": vector,
            "embedding_model": embedding_revision()?,
            "supersedes": id,
        }),
    )?;
    Ok(json!({ "id": event["id"], "supersedes": id }))
}

fn memory_delete(storage_plugin: &str, args: &Value, identity: &Identity) -> Result<Value, String> {
    let id = required_string(args, "id")?;
    let event = invoke_json(
        storage_plugin,
        "append",
        &json!({
            "stream": "memory",
            "kind": "delete",
            "payload": {
                "reason": args.get("reason").cloned().unwrap_or(Value::Null),
                "source": format!("agent:{}", identity.session_id),
            },
            "supersedes": id,
            "tombstone": true,
        }),
    )?;
    Ok(json!({ "deleted": id, "event_id": event["id"] }))
}

fn memory_search(
    storage_plugin: &str,
    args: &Value,
    _identity: &Identity,
) -> Result<Value, String> {
    let query = required_string(args, "query")?;
    let vector = embed(query)?;
    let response = invoke_json(
        storage_plugin,
        "search",
        &json!({
            "stream": "memory",
            "query": vector,
            "embedding_model": embedding_revision()?,
            "k": args.get("k").and_then(Value::as_u64).unwrap_or(10),
            "include_superseded": args.get("include_superseded").and_then(Value::as_bool).unwrap_or(false),
            "include_tombstones": args.get("include_deleted").and_then(Value::as_bool).unwrap_or(false),
        }),
    )?;
    let tags = string_array(args.get("tags"));
    let mut results = response
        .get("hits")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !tags.is_empty() {
        results.retain(|hit| {
            let present = string_array(hit["event"]["payload"].get("tags"));
            tags.iter().all(|tag| present.contains(tag))
        });
    }
    Ok(json!({ "results": results }))
}

fn memory_history(
    storage_plugin: &str,
    args: &Value,
    _identity: &Identity,
) -> Result<Value, String> {
    invoke_json(
        storage_plugin,
        "history",
        &json!({ "id": required_string(args, "id")? }),
    )
}

fn exposed_tools(state: &State, identity: &Identity) -> Result<Vec<Value>, String> {
    state
        .tools
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?;
            match tool_exposed(state, identity, name) {
                Ok(true) => Some(Ok(tool.clone())),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .collect()
}

fn tool_exposed(state: &State, identity: &Identity, tool: &str) -> Result<bool, String> {
    let Some(context) = identity.context.as_deref() else {
        return Ok(true);
    };
    let response = invoke_json(
        &state.rules_plugin,
        "exposed",
        &json!({
            "context": context,
            "tool": tool,
            "tool_group": tool_group(tool),
        }),
    )?;
    Ok(response
        .get("exposed")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

fn invoke(plugin: &str, operation: &str, payload: &str) -> Result<String, String> {
    bindings::ai::vrules::host::invoke(plugin, operation, payload)
}

fn invoke_json(plugin: &str, operation: &str, payload: &Value) -> Result<Value, String> {
    let response = invoke(plugin, operation, &payload.to_string())?;
    serde_json::from_str(&response).map_err(|e| {
        format!("plugin `{plugin}` operation `{operation}` returned invalid JSON: {e}")
    })
}

fn embed(text: &str) -> Result<Vec<f32>, String> {
    let expected = state()?
        .lock()
        .map_err(|_| "runtime lock poisoned")?
        .embedding
        .dimensions as usize;
    let vector = bindings::ai::vrules::host::embed(text)?;
    if vector.len() != expected {
        return Err(format!(
            "embedding dimension mismatch: provider declared {expected}, returned {}",
            vector.len()
        ));
    }
    Ok(vector)
}

fn embedding_revision() -> Result<String, String> {
    state()?
        .lock()
        .map_err(|_| "runtime lock poisoned".to_string())
        .map(|state| state.embedding.revision.clone())
}

fn require_plugin(plugins: &[PluginDescriptor], id: &str) -> Result<(), String> {
    plugins
        .iter()
        .any(|plugin| plugin.id == id)
        .then_some(())
        .ok_or_else(|| format!("required plugin `{id}` is not configured"))
}

fn state() -> Result<&'static Mutex<State>, String> {
    STATE
        .get()
        .ok_or_else(|| "runtime component is not initialized".to_string())
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing `{key}`"))
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn value_text(value: &Value) -> String {
    value
        .get("text")
        .or_else(|| value.get("answer"))
        .or_else(|| value.get("summary"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn cache_id(
    tool: &str,
    backend: &str,
    downstream: &str,
    effort: &str,
    text: &str,
    args: &Value,
) -> String {
    let mut hasher = blake3::Hasher::new();
    for value in [tool, backend, downstream, effort, text] {
        hasher.update(value.as_bytes());
        hasher.update(&[0]);
    }
    hasher.update(args.to_string().as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn tool_group(tool: &str) -> &'static str {
    if tool.starts_with("memory_") {
        "memory"
    } else if tool.starts_with("rules_") {
        "rules"
    } else if tool.starts_with("whatif_") {
        "whatif"
    } else {
        "capability"
    }
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn platform_tools() -> Vec<Value> {
    vec![
        tool(
            "rules_validate",
            "Validate GRL with the active rules engine.",
            json!({
                "type": "object",
                "properties": { "grl": { "type": "string" } },
                "required": ["grl"],
            }),
        ),
        tool(
            "rules_list",
            "List the active rule definitions.",
            json!({ "type": "object" }),
        ),
        tool(
            "whatif_assert",
            "Evaluate facts against the active rules or explicit GRL.",
            json!({
                "type": "object",
                "properties": {
                    "type": { "type": "string" },
                    "facts": { "type": "object" },
                    "grl": { "type": "string" },
                    "trace": { "type": "boolean" },
                },
                "required": ["facts"],
            }),
        ),
        tool(
            "whatif_prove",
            "Prove a backward-chaining goal against GRL rules.",
            json!({
                "type": "object",
                "properties": {
                    "grl": { "type": "string" },
                    "query": { "type": "string" },
                    "facts": { "type": "object" },
                },
                "required": ["grl", "query"],
            }),
        ),
        memory_tool("memory_write", "Append a durable memory.", &["fact"]),
        memory_tool(
            "memory_update",
            "Append a corrected memory that supersedes an earlier event.",
            &["id", "fact"],
        ),
        memory_tool(
            "memory_delete",
            "Append a tombstone without removing history.",
            &["id"],
        ),
        memory_tool(
            "memory_search",
            "Search durable memory by embedding similarity.",
            &["query"],
        ),
        memory_tool(
            "memory_history",
            "Read a memory's append-only event lineage.",
            &["id"],
        ),
        memory_tool("memory_stats", "Read append-only storage statistics.", &[]),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

fn memory_tool(name: &str, description: &str, required: &[&str]) -> Value {
    let mut properties = Map::new();
    properties.insert("id".to_string(), json!({ "type": "string" }));
    properties.insert("fact".to_string(), json!({ "type": "string" }));
    properties.insert("query".to_string(), json!({ "type": "string" }));
    properties.insert(
        "tags".to_string(),
        json!({ "type": "array", "items": { "type": "string" } }),
    );
    properties.insert("reason".to_string(), json!({ "type": "string" }));
    properties.insert("k".to_string(), json!({ "type": "integer" }));
    tool(
        name,
        description,
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
        }),
    )
}

fn default_rules_plugin() -> String {
    "rules".to_string()
}

fn default_storage_plugin() -> String {
    "storage".to_string()
}

fn default_cache_ttl_secs() -> u64 {
    3_600
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default()
}

#[allow(unsafe_code)]
mod component_export {
    use super::McpRuntime;
    use crate::bindings;

    crate::bindings::export!(McpRuntime with_types_in bindings);
}
