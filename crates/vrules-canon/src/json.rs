//! Structured-JSON canonicalizers (feature `json`).
//!
//! Two strategies with very different goals:
//!
//! - [`JsonHybrid`] — **what you embed.** Keeps string values (in log JSON the
//!   strings ARE the signal: `level`, `msg`, `error`), masks only the variable
//!   parts (numbers, and id-like strings: UUIDs, IPs, timestamps, hex), and
//!   sorts object keys for determinism. Maximizes cache hits without discarding
//!   the semantic payload.
//! - [`SchemaFingerprint`] — **dedup detection only.** Strips ALL values to type
//!   tokens (`<str>`/`<num>`/`<bool>`/`<null>`). Two payloads with the same shape
//!   collapse regardless of content. NEVER embed this — it throws away meaning.
//!
//! Determinism: object keys are emitted in sorted order (serde_json's default
//! `Map` is a `BTreeMap`), and the transform is a pure function of the input.

use serde_json::Value;

use crate::mask::{is_variable, MASK};
use crate::{CanonMode, CanonResult, Canonicalizer};

/// Hybrid JSON canonicalizer: keep string signal, mask variable scalars, sort
/// keys. See the [module docs](self).
#[derive(Debug, Default, Clone, Copy)]
pub struct JsonHybrid;

impl JsonHybrid {
    /// Canonicalize, returning `None` if `input` is not valid JSON (so callers
    /// can fall back to text masking).
    #[must_use]
    pub fn try_canon(&self, input: &str) -> Option<CanonResult> {
        let value: Value = serde_json::from_str(input).ok()?;
        let mut vars: Vec<String> = Vec::new();
        let canon = transform_hybrid(&value, &mut vars);
        let canonical = canon.to_string();
        Some(CanonResult::new(canonical, vars, CanonMode::Json))
    }
}

impl Canonicalizer for JsonHybrid {
    fn id(&self) -> &str {
        "json-hybrid"
    }

    fn version(&self) -> u32 {
        1
    }

    /// Canonicalize as JSON; on parse failure fall back to [`LogMask`] over the
    /// raw text so the call always yields a usable canonical form.
    fn canon(&self, input: &str) -> CanonResult {
        self.try_canon(input)
            .unwrap_or_else(|| crate::LogMask.canon(input))
    }
}

/// Recursively rewrite a value: numbers → [`MASK`], id-like strings → [`MASK`],
/// other strings kept, bool/null kept, objects key-sorted (via `BTreeMap`),
/// arrays order-preserved. Masked originals are pushed to `vars`.
fn transform_hybrid(v: &Value, vars: &mut Vec<String>) -> Value {
    match v {
        Value::Number(n) => {
            vars.push(n.to_string());
            Value::String(MASK.to_owned())
        }
        Value::String(s) => {
            if is_variable(s) {
                vars.push(s.clone());
                Value::String(MASK.to_owned())
            } else {
                Value::String(s.clone())
            }
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|x| transform_hybrid(x, vars)).collect())
        }
        Value::Object(map) => {
            // serde_json default Map = BTreeMap → iteration & re-serialization
            // are key-sorted, giving a deterministic canonical string.
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                out.insert(k.clone(), transform_hybrid(val, vars));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Pure schema fingerprint: every value → its type token. For dedup/shape
/// detection only — do not embed the result.
#[derive(Debug, Default, Clone, Copy)]
pub struct SchemaFingerprint;

impl SchemaFingerprint {
    /// Fingerprint, returning `None` if `input` is not valid JSON.
    #[must_use]
    pub fn try_canon(&self, input: &str) -> Option<CanonResult> {
        let value: Value = serde_json::from_str(input).ok()?;
        let canonical = transform_schema(&value).to_string();
        Some(CanonResult::new(canonical, Vec::new(), CanonMode::Schema))
    }
}

impl Canonicalizer for SchemaFingerprint {
    fn id(&self) -> &str {
        "json-schema"
    }

    fn version(&self) -> u32 {
        1
    }

    fn canon(&self, input: &str) -> CanonResult {
        self.try_canon(input)
            .unwrap_or_else(|| CanonResult::new(input.to_owned(), Vec::new(), CanonMode::Schema))
    }
}

fn transform_schema(v: &Value) -> Value {
    match v {
        Value::Null => Value::String("<null>".to_owned()),
        Value::Bool(_) => Value::String("<bool>".to_owned()),
        Value::Number(_) => Value::String("<num>".to_owned()),
        Value::String(_) => Value::String("<str>".to_owned()),
        Value::Array(items) => Value::Array(items.iter().map(transform_schema).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                out.insert(k.clone(), transform_schema(val));
            }
            Value::Object(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_keeps_strings_masks_numbers() {
        let r = JsonHybrid.canon(r#"{"level":"error","code":500,"msg":"disk full"}"#);
        assert_eq!(r.mode, CanonMode::Json);
        // Keys sorted; strings kept; number masked.
        assert_eq!(
            r.canonical,
            r#"{"code":"<*>","level":"error","msg":"disk full"}"#
        );
        assert_eq!(r.vars, vec!["500"]);
    }

    #[test]
    fn hybrid_preserves_recall_signal_across_value_change() {
        // Different string values must NOT collapse — strings are the signal.
        let a = JsonHybrid.canon(r#"{"level":"error","msg":"disk full"}"#);
        let b = JsonHybrid.canon(r#"{"level":"info","msg":"ok"}"#);
        assert_ne!(a.canonical, b.canonical);
    }

    #[test]
    fn hybrid_collapses_pure_numeric_variation() {
        let a = JsonHybrid.canon(r#"{"user":42,"action":"login"}"#);
        let b = JsonHybrid.canon(r#"{"user":99999,"action":"login"}"#);
        assert_eq!(a.canonical, b.canonical);
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn hybrid_masks_id_like_strings() {
        let r =
            JsonHybrid.canon(r#"{"ip":"10.0.0.1","id":"550e8400-e29b-41d4-a716-446655440000"}"#);
        assert_eq!(r.canonical, r#"{"id":"<*>","ip":"<*>"}"#);
        assert_eq!(r.vars.len(), 2);
    }

    #[test]
    fn hybrid_key_order_independent() {
        let a = JsonHybrid.canon(r#"{"b":"x","a":"y"}"#);
        let b = JsonHybrid.canon(r#"{"a":"y","b":"x"}"#);
        assert_eq!(a.canonical, b.canonical);
    }

    #[test]
    fn hybrid_nested_and_arrays() {
        let r = JsonHybrid.canon(r#"{"meta":{"n":1,"tags":["a",2]}}"#);
        assert_eq!(r.canonical, r#"{"meta":{"n":"<*>","tags":["a","<*>"]}}"#);
    }

    #[test]
    fn invalid_json_falls_back_to_log_mask() {
        let r = JsonHybrid.canon("not json 42");
        assert_eq!(r.mode, CanonMode::Log);
        assert_eq!(r.canonical, "not json <*>");
    }

    #[test]
    fn schema_strips_all_values() {
        let r = SchemaFingerprint.canon(r#"{"level":"error","code":500,"ok":true}"#);
        assert_eq!(r.mode, CanonMode::Schema);
        assert_eq!(
            r.canonical,
            r#"{"code":"<num>","level":"<str>","ok":"<bool>"}"#
        );
    }

    #[test]
    fn schema_collapses_same_shape() {
        let a = SchemaFingerprint.canon(r#"{"level":"error","msg":"disk full"}"#);
        let b = SchemaFingerprint.canon(r#"{"level":"info","msg":"ok"}"#);
        assert_eq!(a.canonical, b.canonical, "same shape → same fingerprint");
    }
}
