#![deny(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{Value, json};

#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "plugin-component",
    });
}

use bindings::ai::vrules::types::{PluginDescriptor, PluginKind};
use bindings::exports::ai::vrules::plugin::Guest;

struct AdminComponent;

static CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
struct Config {
    #[serde(default = "default_rules_plugin")]
    rules_plugin: String,
    #[serde(default = "default_storage_plugin")]
    storage_plugin: String,
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    method: String,
    params: String,
}

impl Guest for AdminComponent {
    fn initialize(config: String) -> Result<PluginDescriptor, String> {
        let config: Config =
            serde_json::from_str(&config).map_err(|e| format!("invalid admin config: {e}"))?;
        CONFIG
            .set(config)
            .map_err(|_| "admin component is already initialized".to_string())?;
        Ok(PluginDescriptor {
            id: "admin".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            kind: PluginKind::Admin,
            operations: vec!["rpc".to_string(), "tick".to_string()],
        })
    }

    fn invoke(operation: String, payload: String) -> Result<String, String> {
        match operation.as_str() {
            "rpc" => rpc(&payload),
            "tick" => Ok("{}".to_string()),
            other => Err(format!("unsupported admin operation `{other}`")),
        }
    }
}

fn rpc(payload: &str) -> Result<String, String> {
    let request: RpcRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid admin request: {e}"))?;
    let params: Value =
        serde_json::from_str(&request.params).map_err(|e| format!("invalid admin params: {e}"))?;
    let config = config()?;
    let result = match request.method.as_str() {
        "storage.stats" => storage_stats(config)?,
        "tools.stats" => tool_stats(config)?,
        "log.scan" => log_scan(config, &params)?,
        "log.search" => semantic_search(config, "audit", &params)?,
        "sessions.list" => sessions(config)?,
        "test.run" => test_run(config, &params)?,
        "rules.validate" => call_json(
            &config.rules_plugin,
            "validate",
            &json!({ "grl": params.get("grl").and_then(Value::as_str).unwrap_or("") }),
        )?,
        "rules.list" => call_json(
            &config.rules_plugin,
            "list",
            &json!({ "revision": params.get("ruleset").cloned() }),
        )?,
        "rules.branches" => call_json(&config.rules_plugin, "branches", &json!({}))?,
        "rules.diff" => call_json(&config.rules_plugin, "diff", &params)?,
        "rules.compare" => call_json(&config.rules_plugin, "compare", &params)?,
        "rules.promote" => call_json(&config.rules_plugin, "promote", &params)?,
        "ab.run" => ab_run(config, &params)?,
        "embedding.info" => {
            let info = bindings::ai::vrules::host::get_embedding_info()?;
            json!({
                "id": info.id,
                "version": info.version,
                "model": info.model,
                "revision": info.revision,
                "dimensions": info.dimensions,
            })
        }
        "embedding.embed" => {
            let text = required_string(&params, "text")?;
            let info = bindings::ai::vrules::host::get_embedding_info()?;
            let vector = bindings::ai::vrules::host::embed(text)?;
            json!({
                "info": {
                    "id": info.id,
                    "version": info.version,
                    "model": info.model,
                    "revision": info.revision,
                    "dimensions": info.dimensions,
                },
                "vector": vector,
            })
        }
        "whatif.assert" => evaluate(config, &params)?,
        "whatif.prove" => call_json(
            &config.rules_plugin,
            "prove",
            &json!({
                "grl": params.get("grl").and_then(Value::as_str).unwrap_or(""),
                "query": params.get("query").and_then(Value::as_str).unwrap_or(""),
                "facts": params.get("facts").cloned().unwrap_or_else(|| json!({})),
            }),
        )?,
        "memory.search" => memory_search(config, &params)?,
        "memory.history" => call_json(
            &config.storage_plugin,
            "history",
            &json!({ "id": required_string(&params, "id")? }),
        )?,
        "memory.stats" => storage_stats(config)?,
        "memory.write" => memory_write(config, &params)?,
        "memory.update" => memory_update(config, &params)?,
        "memory.delete" => memory_delete(config, &params)?,
        other => return Err(format!("unknown admin method `{other}`")),
    };
    serde_json::to_string(&result).map_err(|e| format!("encode admin response: {e}"))
}

fn evaluate(config: &Config, params: &Value) -> Result<Value, String> {
    call_json(
        &config.rules_plugin,
        "evaluate",
        &json!({
            "facts": [{
                "type": params.get("type").and_then(Value::as_str).unwrap_or("Request"),
                "data": params.get("facts").cloned().unwrap_or_else(|| json!({})),
            }],
            "trace": true,
            "grl": params.get("grl").and_then(Value::as_str),
            "revision": params.get("ruleset").cloned(),
        }),
    )
}

fn ab_run(config: &Config, params: &Value) -> Result<Value, String> {
    let a = required_string(params, "a")?;
    let b = required_string(params, "b")?;
    let evaluate = |revision: &str| {
        call_json(
            &config.rules_plugin,
            "evaluate",
            &json!({
                "facts": [{
                    "type": params.get("type").and_then(Value::as_str).unwrap_or("Request"),
                    "data": params.get("facts").cloned().unwrap_or_else(|| json!({})),
                }],
                "trace": true,
                "revision": revision,
            }),
        )
    };
    let result_a = evaluate(a)?;
    let result_b = evaluate(b)?;
    Ok(json!({
        "a": { "ref": a, "result": result_a },
        "b": { "ref": b, "result": result_b },
        "diff": {
            "fired_only_a": fired_only(&result_a, &result_b),
            "fired_only_b": fired_only(&result_b, &result_a),
            "decision_changed": result_a["decision"] != result_b["decision"],
        },
    }))
}

fn fired_only<'a>(left: &'a Value, right: &Value) -> Vec<&'a str> {
    let right = right["fired"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    left["fired"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|rule| !right.contains(rule))
        .collect()
}

fn test_run(config: &Config, params: &Value) -> Result<Value, String> {
    let tool = params
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or("web_ground");
    let args = params.get("args").cloned().unwrap_or_else(|| json!({}));
    call_json(
        &config.rules_plugin,
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
                    "effort": args.get("effort").and_then(Value::as_str).unwrap_or("low"),
                },
            }],
            "trace": true,
        }),
    )
}

fn memory_write(config: &Config, params: &Value) -> Result<Value, String> {
    let fact = required_string(params, "fact")?;
    let event = append_memory(config, "write", fact, params, None, false)?;
    Ok(json!({ "id": event["id"] }))
}

fn memory_update(config: &Config, params: &Value) -> Result<Value, String> {
    let id = required_string(params, "id")?;
    let fact = required_string(params, "fact")?;
    let event = append_memory(config, "update", fact, params, Some(id), false)?;
    Ok(json!({ "id": event["id"], "supersedes": id }))
}

fn memory_delete(config: &Config, params: &Value) -> Result<Value, String> {
    let id = required_string(params, "id")?;
    let event = call_json(
        &config.storage_plugin,
        "append",
        &json!({
            "stream": "memory",
            "kind": "delete",
            "payload": {
                "reason": params.get("reason").cloned().unwrap_or(Value::Null),
                "source": "admin",
            },
            "supersedes": id,
            "tombstone": true,
        }),
    )?;
    Ok(json!({ "deleted": id, "event_id": event["id"] }))
}

fn append_memory(
    config: &Config,
    kind: &str,
    fact: &str,
    params: &Value,
    supersedes: Option<&str>,
    tombstone: bool,
) -> Result<Value, String> {
    let vector = bindings::ai::vrules::host::embed(fact)?;
    let embedding_model = embedding_revision()?;
    call_json(
        &config.storage_plugin,
        "append",
        &json!({
            "stream": "memory",
            "kind": kind,
            "payload": {
                "fact": fact,
                "tags": string_array(params.get("tags")),
                "source": "admin",
            },
            "vector": vector,
            "embedding_model": embedding_model,
            "supersedes": supersedes,
            "tombstone": tombstone,
        }),
    )
}

fn memory_search(config: &Config, params: &Value) -> Result<Value, String> {
    let limit = params.get("k").and_then(Value::as_u64).unwrap_or(10);
    let include_superseded = params
        .get("include_superseded")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let include_tombstones = params
        .get("include_deleted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let response = if params.get("mode").and_then(Value::as_str) != Some("recent")
        && let Some(query) = params
            .get("query")
            .and_then(Value::as_str)
            .filter(|query| !query.trim().is_empty())
    {
        let vector = bindings::ai::vrules::host::embed(query)?;
        call_json(
            &config.storage_plugin,
            "search",
            &json!({
                "stream": "memory",
                "query": vector,
                "embedding_model": embedding_revision()?,
                "k": limit,
                "include_superseded": include_superseded,
                "include_tombstones": include_tombstones,
            }),
        )?
    } else {
        call_json(
            &config.storage_plugin,
            "scan",
            &json!({
                "stream": "memory",
                "limit": limit,
                "include_superseded": include_superseded,
                "include_tombstones": include_tombstones,
            }),
        )?
    };
    Ok(filter_memory_tags(
        response,
        string_array(params.get("tags")),
    ))
}

fn semantic_search(config: &Config, stream: &str, params: &Value) -> Result<Value, String> {
    let query = required_string(params, "query")?;
    let vector = bindings::ai::vrules::host::embed(query)?;
    let embedding_model = embedding_revision()?;
    call_json(
        &config.storage_plugin,
        "search",
        &json!({
            "stream": stream,
            "query": vector,
            "embedding_model": embedding_model,
            "k": params.get("k").and_then(Value::as_u64).unwrap_or(10),
        }),
    )
}

fn scan(config: &Config, stream: &str, limit: u64) -> Result<Value, String> {
    call_json(
        &config.storage_plugin,
        "scan",
        &json!({ "stream": stream, "limit": limit }),
    )
}

fn log_scan(config: &Config, params: &Value) -> Result<Value, String> {
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(50);
    let session = params
        .get("session")
        .and_then(Value::as_str)
        .filter(|session| !session.trim().is_empty());
    let mut response = scan(
        config,
        "audit",
        if session.is_some() { 10_000 } else { limit },
    )?;
    if let Some(events) = response.get_mut("events").and_then(Value::as_array_mut) {
        if let Some(session) = session {
            events.retain(|event| event["payload"]["session_id"].as_str() == Some(session));
        }
        events.truncate(limit as usize);
    }
    Ok(response)
}

fn filter_memory_tags(mut response: Value, tags: Vec<String>) -> Value {
    if tags.is_empty() {
        return response;
    }
    if let Some(hits) = response.get_mut("hits").and_then(Value::as_array_mut) {
        hits.retain(|hit| {
            let present = string_array(hit["event"]["payload"].get("tags"));
            tags.iter().all(|tag| present.contains(tag))
        });
    }
    if let Some(events) = response.get_mut("events").and_then(Value::as_array_mut) {
        events.retain(|event| {
            let present = string_array(event["payload"].get("tags"));
            tags.iter().all(|tag| present.contains(tag))
        });
    }
    response
}

fn sessions(config: &Config) -> Result<Value, String> {
    let response = scan(config, "audit", 10_000)?;
    let sessions = response
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| event["payload"]["session_id"].as_str())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    Ok(json!(sessions))
}

fn tool_stats(config: &Config) -> Result<Value, String> {
    let response = scan(config, "activity", 100_000)?;
    let mut stats = BTreeMap::<String, (u64, u64, u64)>::new();
    for event in response
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(tool) = event["payload"]["tool"].as_str() else {
            continue;
        };
        let entry = stats.entry(tool.to_string()).or_default();
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry
            .1
            .saturating_add(u64::from(event["payload"]["ok"].as_bool() == Some(false)));
        entry.2 = entry
            .2
            .max(event["timestamp_ns"].as_u64().unwrap_or_default());
    }
    Ok(json!({
        "tools": stats.into_iter().map(|(name, (calls, errors, last_called_ns))| {
            json!({
                "name": name,
                "calls": calls,
                "errors": errors,
                "last_called_ns": last_called_ns,
            })
        }).collect::<Vec<_>>()
    }))
}

fn storage_stats(config: &Config) -> Result<Value, String> {
    call_json(&config.storage_plugin, "stats", &json!({}))
}

fn embedding_revision() -> Result<String, String> {
    bindings::ai::vrules::host::get_embedding_info().map(|info| info.revision)
}

fn call_json(plugin: &str, operation: &str, payload: &Value) -> Result<Value, String> {
    let response = bindings::ai::vrules::host::invoke(plugin, operation, &payload.to_string())?;
    serde_json::from_str(&response).map_err(|e| {
        format!("plugin `{plugin}` operation `{operation}` returned invalid JSON: {e}")
    })
}

fn config() -> Result<&'static Config, String> {
    CONFIG
        .get()
        .ok_or_else(|| "admin component is not initialized".to_string())
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

fn default_rules_plugin() -> String {
    "rules".to_string()
}

fn default_storage_plugin() -> String {
    "storage".to_string()
}

#[allow(unsafe_code)]
mod component_export {
    use super::AdminComponent;
    use crate::bindings;

    crate::bindings::export!(AdminComponent with_types_in bindings);
}
