//! Shared audit record + canonicalization for vrules capability-MCP executions.
//!
//! Two responsibilities:
//! 1. [`ExecutionRecord`] — the structured, serde-serializable log line written
//!    to em-log-n for every capability call (`web_ground` / `summarize` / …).
//! 2. [`ExecCanonicalizer`] — derives the **cache/dedup key** from the
//!    *request + backend only* (NOT the origin identity), so that
//!    semantically-equivalent requests across different sessions collapse to the
//!    same key and share an em-log-n cache entry. Origin ids ride on
//!    the record (and em-log-n tags) for per-session querying without
//!    fragmenting the cache.

use std::collections::BTreeMap;

use crate::{CanonMode, CanonResult, Canonicalizer};
use serde::{Deserialize, Serialize};

/// Whether a capability call was served from cache or from the live backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheState {
    Hit,
    Miss,
}

/// One audited capability-MCP execution. Written to em-log-n as the value; the
/// `canonical_id` is the row/cache key and the origin session id rides in tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    /// Upstream session/context id used as the primary correlation key.
    ///
    /// Serialized key remains `session_id` for wire and storage compatibility.
    #[serde(rename = "session_id", alias = "origin_session_id")]
    pub origin_session_id: String,
    /// Optional nested context id (subagent/child scope, etc).
    ///
    /// Serialized key remains `child_session` for wire and storage compatibility.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "child_session",
        alias = "origin_subsession_id"
    )]
    pub origin_subsession_id: Option<String>,
    /// Source process/runtime instance id.
    ///
    /// Serialized key remains `process_uuid` for wire and storage compatibility.
    #[serde(rename = "process_uuid", alias = "origin_process_id")]
    pub origin_process_id: String,

    /// Capability tool invoked, e.g. `"web_ground"` / `"summarize"`.
    pub tool: String,
    /// Backend the router selected, e.g. `"gemini-backend"`.
    pub backend: String,
    /// The request arguments (may be redaction-masked in a later pass).
    pub request: serde_json::Value,
    /// The answer returned to the caller.
    pub answer: String,

    /// Cache hit or live backend call.
    pub cache: CacheState,
    /// Total wall-clock latency in milliseconds.
    pub latency_ms: u64,
    /// Per-stage timings in milliseconds (e.g. `canonicalize`, `rules_eval`,
    /// `trace_serialize`, `cache_lookup`, `backend`), so the rules-engine overhead
    /// is reportable **separately** from backend latency. Empty when not measured.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stage_latencies: BTreeMap<String, u64>,
    /// Routing rules that fired for this request.
    #[serde(default)]
    pub fired: Vec<String>,
    /// Optional EXPLAIN-ANALYZE engine trace (stats + justifications).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<serde_json::Value>,

    /// Git commit SHA of the rules repo the engine evaluated against — so this
    /// logged decision replays against the exact ruleset that produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ruleset_sha: Option<String>,
    /// Git branch of the rules repo (the live/proposal branch in effect).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ruleset_branch: Option<String>,

    /// Unix nanoseconds at execution time (em-log-n key ordering).
    pub ts_nanos: u64,
    /// FNV-1a cache key from [`ExecCanonicalizer`] (request + backend only).
    pub canonical_id: u64,

    /// Rule-driven action applied to this call by a metric/policy rule, e.g.
    /// `"shed"`, `"route:other-backend"`, `"effort:low"`. `None` when the call
    /// proceeded normally. Additive + append-only-safe: old records omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

impl ExecutionRecord {
    /// The semantic text to embed for similarity search: the request's
    /// query/content plus the answer, so a search matches both the intent and
    /// the result. Falls back to the whole request JSON if no known text field.
    #[must_use]
    pub fn search_text(&self) -> String {
        let req = self
            .request
            .get("query")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                self.request
                    .get("content")
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::to_string)
            .unwrap_or_else(|| self.request.to_string());
        format!("{req}\n{}", self.answer)
    }
}

/// Canonicalization strategy for execution cache keys.
///
/// Conservative on purpose: it joins the dedup-relevant request fields and
/// normalizes only whitespace (the semantic payload — the query/content text —
/// is preserved, not number-masked, so meaning-bearing digits aren't lost).
/// **Excludes** origin identity so the key is shared across sessions.
#[derive(Debug, Default, Clone, Copy)]
pub struct ExecCanonicalizer;

impl ExecCanonicalizer {
    /// Build the cache key from the dedup-relevant request fields.
    /// `effort` is the capability-level knob (e.g. `"low"`/`"high"`); pass `""`
    /// when not applicable. `text` is the query (web_ground) or content
    /// (summarize). Origin ids are intentionally NOT included.
    #[must_use]
    pub fn key(&self, tool: &str, backend: &str, effort: &str, text: &str) -> CanonResult {
        // Unit separator (US, 0x1f) can't appear in normal text → unambiguous join.
        let joined = format!(
            "{tool}\u{1f}{backend}\u{1f}{effort}\u{1f}{}",
            normalize_ws(text)
        );
        self.canon(&joined)
    }
}

impl Canonicalizer for ExecCanonicalizer {
    fn id(&self) -> &str {
        "vrules-exec"
    }

    fn version(&self) -> u32 {
        1
    }

    fn canon(&self, input: &str) -> CanonResult {
        // The field-join + whitespace normalization is the canonicalization;
        // `CanonResult::new` computes the stable FNV-1a id from the canonical.
        CanonResult::new(normalize_ws(input), Vec::new(), CanonMode::Identity)
    }
}

/// Collapse runs of ASCII whitespace to a single space and trim the ends.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_requests_share_a_key_across_whitespace() {
        let c = ExecCanonicalizer;
        let a = c.key(
            "web_ground",
            "gemini-backend",
            "low",
            "rust async   runtime",
        );
        let b = c.key(
            "web_ground",
            "gemini-backend",
            "low",
            "  rust async runtime\n",
        );
        assert_eq!(a.id, b.id, "whitespace-only differences must collapse");
    }

    #[test]
    fn different_backend_or_effort_or_tool_diverges() {
        let c = ExecCanonicalizer;
        let base = c.key("web_ground", "gemini-backend", "low", "q").id;
        assert_ne!(base, c.key("web_ground", "gemini-backend", "high", "q").id);
        assert_ne!(base, c.key("web_ground", "other-backend", "low", "q").id);
        assert_ne!(base, c.key("summarize", "gemini-backend", "low", "q").id);
    }

    #[test]
    fn key_is_independent_of_session() {
        // The key fn takes no session input at all — equal inputs, equal key,
        // regardless of which session produced them.
        let c = ExecCanonicalizer;
        assert_eq!(
            c.key("summarize", "gemini-backend", "", "hello").id,
            c.key("summarize", "gemini-backend", "", "hello").id,
        );
    }

    #[test]
    fn record_round_trips_through_serde() {
        let rec = ExecutionRecord {
            origin_session_id: "sess-1".into(),
            origin_subsession_id: None,
            origin_process_id: "uuid-1".into(),
            tool: "web_ground".into(),
            backend: "gemini-backend".into(),
            request: serde_json::json!({"query": "q", "effort": "low"}),
            answer: "a".into(),
            cache: CacheState::Miss,
            latency_ms: 42,
            stage_latencies: BTreeMap::from([("rules_eval".to_string(), 1)]),
            fired: vec!["RouteToFlash".into()],
            trace: None,
            ruleset_sha: Some("abc123".into()),
            ruleset_branch: Some("live".into()),
            ts_nanos: 1,
            canonical_id: ExecCanonicalizer
                .key("web_ground", "gemini-backend", "low", "q")
                .id,
            action: None,
        };
        let s = serde_json::to_string(&rec).unwrap();
        let back: ExecutionRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(back.canonical_id, rec.canonical_id);
        assert_eq!(back.cache, CacheState::Miss);
        assert_eq!(back.ruleset_sha.as_deref(), Some("abc123"));
        assert_eq!(back.stage_latencies.get("rules_eval"), Some(&1));
    }
}
