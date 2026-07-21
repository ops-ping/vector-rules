//! Embedding-free canonical matching: "do X with messages like this".
//!
//! A deterministic, in-process matcher that decides whether an incoming message
//! is "like" a registered set of examples — **without any embedding model**. It
//! is the *deterministic-before-generative* pre-tier from `docs/DESIGN.md`:
//! resolve obvious structural/near-duplicate matches cheaply, leaving the
//! embedding path (when present) for genuinely novel phrasing.
//!
//! Matching has two rungs, both backed by [`vrules_canon`]:
//! 1. **Exact template hit** — the message canonicalizes to the same form as a
//!    registered example (ids/numbers/etc. masked away), so every variant of a
//!    recurring message collapses to one template. Score `1.0`.
//! 2. **Near-duplicate** — within a [`SimHash`](vrules_canon::SimHash64) Hamming
//!    threshold of some example's canonical token stream. Score `1 - h/64`.
//!
//! Every decision returns a [`Match`] carrying the label, score, and
//! [`MatchMode`] so it is auditable (emit score + matched concept).
//!
//! [`CanonRouter`] is the runtime-registerable registry of labeled
//! [`PatternSet`]s. Rule effects are authored in GRL `then` clauses.

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use serde::Serialize;
use vrules_canon::{SimHash64, hamming_distance};

// The canon engine functions (s_canon_match/b_canon_matches/...) write the decision blackboard
// using vrules_canon only — no MCP — so they build for the wasm engine too (just
// rule-engine; the previous mcp-client gate was incidental coupling).
#[cfg(feature = "rule-engine")]
mod bridge;
#[cfg(feature = "rule-engine")]
pub use bridge::register_canon_functions;

/// Default near-duplicate Hamming threshold (out of 64 bits) for a pattern set.
pub const DEFAULT_THRESHOLD: u32 = 6;

/// Which canonicalization strategy a [`PatternSet`] applies to messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CanonKind {
    /// Free-form text masking ([`vrules_canon::LogMask`]).
    Log,
    /// Structured hybrid JSON ([`vrules_canon::JsonHybrid`]).
    Json,
    /// Auto-detect per message ([`vrules_canon::canonicalize`]).
    Auto,
}

impl CanonKind {
    /// Parse from a tool/string argument; unknown values fall back to [`Auto`].
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "log" => Self::Log,
            "json" => Self::Json,
            _ => Self::Auto,
        }
    }

    /// Canonicalize `text` under this strategy.
    #[must_use]
    pub fn apply(self, text: &str) -> vrules_canon::CanonResult {
        use vrules_canon::Canonicalizer;
        match self {
            Self::Log => vrules_canon::LogMask.canon(text),
            Self::Json => vrules_canon::JsonHybrid.canon(text),
            Self::Auto => vrules_canon::canonicalize(text),
        }
    }
}

/// How a message matched a [`PatternSet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    /// Canonical template equality (collapsed variants).
    Exact,
    /// SimHash near-duplicate within threshold.
    Near,
}

/// The outcome of classifying a message against the router.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Match {
    /// Label of the pattern set that matched.
    pub label: String,
    /// Match strength in `0.0..=1.0` (`1.0` for an exact template hit).
    pub score: f32,
    /// Whether the match was exact or near-duplicate.
    pub mode: MatchMode,
}

/// A labeled set of example messages, reduced to canonical templates plus
/// near-duplicate fingerprints.
#[derive(Debug, Clone)]
pub struct PatternSet {
    /// Human label, e.g. `"refund_request"`.
    pub label: String,
    /// Canonicalization strategy applied to both examples and queries.
    pub kind: CanonKind,
    /// Near-duplicate Hamming threshold (out of 64).
    pub threshold: u32,
    /// Canonical-template ids for O(1) exact-hit detection.
    templates: HashSet<u64>,
    /// SimHash fingerprints (over canonical tokens) for near-dup scan.
    fps: Vec<u64>,
    /// Original example messages, for audit / inspection.
    examples: Vec<String>,
}

impl PatternSet {
    /// Empty set with the given strategy and threshold.
    #[must_use]
    pub fn new(label: impl Into<String>, kind: CanonKind, threshold: u32) -> Self {
        Self {
            label: label.into(),
            kind,
            threshold,
            templates: HashSet::new(),
            fps: Vec::new(),
            examples: Vec::new(),
        }
    }

    /// Add one example message.
    pub fn add_example(&mut self, message: &str) {
        let canon = self.kind.apply(message);
        self.templates.insert(canon.id);
        self.fps
            .push(SimHash64::compute(canon.canonical.split_whitespace()).0);
        self.examples.push(message.to_owned());
    }

    /// Number of examples registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.examples.len()
    }

    /// Whether the set has no examples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.examples.is_empty()
    }

    /// Score `text` against this set: `Some((score, mode))` on match, else `None`.
    #[must_use]
    pub fn score(&self, text: &str) -> Option<(f32, MatchMode)> {
        let canon = self.kind.apply(text);
        if self.templates.contains(&canon.id) {
            return Some((1.0, MatchMode::Exact));
        }
        let h = SimHash64::compute(canon.canonical.split_whitespace()).0;
        let best = self.fps.iter().map(|&f| hamming_distance(f, h)).min()?;
        if best <= self.threshold {
            Some((1.0 - best as f32 / 64.0, MatchMode::Near))
        } else {
            None
        }
    }
}

/// Runtime-registerable registry of labeled [`PatternSet`]s.
///
/// Shared (`Arc`) between the rule-engine bridge functions and the MCP tools; an
/// `RwLock` allows many concurrent classifications with occasional registration.
#[derive(Debug, Default)]
pub struct CanonRouter {
    sets: RwLock<HashMap<String, PatternSet>>,
}

impl CanonRouter {
    /// Empty router.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a labeled pattern set built from `examples`.
    /// Returns the number of examples stored.
    pub fn register(
        &self,
        label: impl Into<String>,
        kind: CanonKind,
        threshold: u32,
        examples: &[String],
    ) -> usize {
        let label = label.into();
        let mut set = PatternSet::new(label.clone(), kind, threshold);
        for ex in examples {
            set.add_example(ex);
        }
        let n = set.len();
        self.sets
            .write()
            .expect("canon router poisoned")
            .insert(label, set);
        n
    }

    /// Score `text` against one named set.
    #[must_use]
    pub fn score_for(&self, label: &str, text: &str) -> Option<(f32, MatchMode)> {
        self.sets
            .read()
            .expect("canon router poisoned")
            .get(label)
            .and_then(|s| s.score(text))
    }

    /// Best match for `text` across all registered sets (highest score wins).
    #[must_use]
    pub fn classify(&self, text: &str) -> Option<Match> {
        let sets = self.sets.read().expect("canon router poisoned");
        let mut best: Option<Match> = None;
        for set in sets.values() {
            if let Some((score, mode)) = set.score(text)
                && best.as_ref().is_none_or(|b| score > b.score)
            {
                best = Some(Match {
                    label: set.label.clone(),
                    score,
                    mode,
                });
            }
        }
        best
    }
    /// Labels of all registered sets (sorted) — for inspection.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .sets
            .read()
            .expect("canon router poisoned")
            .keys()
            .cloned()
            .collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_template_hit_collapses_variants() {
        let mut set = PatternSet::new("login", CanonKind::Log, DEFAULT_THRESHOLD);
        set.add_example("User 1 login from 10.0.0.1");
        let (score, mode) = set.score("User 9999 login from 192.168.1.7").unwrap();
        assert_eq!(mode, MatchMode::Exact);
        assert_eq!(score, 1.0);
    }

    #[test]
    fn near_duplicate_within_threshold() {
        let mut set = PatternSet::new("payment", CanonKind::Log, 12);
        set.add_example("payment processed for order alpha beta gamma");
        let m = set.score("payment processed for order alpha beta delta");
        assert!(matches!(m, Some((_, MatchMode::Near))), "got {m:?}");
    }

    #[test]
    fn unrelated_message_does_not_match() {
        let mut set = PatternSet::new("login", CanonKind::Log, DEFAULT_THRESHOLD);
        set.add_example("User 1 login from 10.0.0.1");
        assert!(
            set.score("disk failure replication halted urgently")
                .is_none()
        );
    }

    #[test]
    fn json_kind_matches_on_shape_and_string_signal() {
        let mut set = PatternSet::new("refund", CanonKind::Json, DEFAULT_THRESHOLD);
        set.add_example(r#"{"action":"refund","amount":42}"#);
        // Same action/shape, different number → exact template hit (numbers masked).
        let (_, mode) = set.score(r#"{"action":"refund","amount":999}"#).unwrap();
        assert_eq!(mode, MatchMode::Exact);
    }

    #[test]
    fn router_classify_picks_best_label() {
        let router = CanonRouter::new();
        router.register(
            "login",
            CanonKind::Log,
            DEFAULT_THRESHOLD,
            &["User 1 login from 10.0.0.1".into()],
        );
        router.register(
            "refund",
            CanonKind::Log,
            DEFAULT_THRESHOLD,
            &["refund order 7 for user bob".into()],
        );
        // Differs from the refund example only in the masked number → exact.
        let m = router.classify("refund order 99 for user bob").unwrap();
        assert_eq!(m.label, "refund");
        assert_eq!(m.mode, MatchMode::Exact);
        assert_eq!(router.labels(), vec!["login", "refund"]);
    }

    #[test]
    fn router_score_for_specific_label() {
        let router = CanonRouter::new();
        router.register(
            "login",
            CanonKind::Log,
            DEFAULT_THRESHOLD,
            &["User 1 login".into()],
        );
        assert!(router.score_for("login", "User 5 login").is_some());
        assert!(
            router
                .score_for("login", "totally different text here")
                .is_none()
        );
        assert!(router.score_for("missing", "anything").is_none());
    }

    #[test]
    fn canon_kind_parse() {
        assert_eq!(CanonKind::parse("log"), CanonKind::Log);
        assert_eq!(CanonKind::parse("JSON"), CanonKind::Json);
        assert_eq!(CanonKind::parse("whatever"), CanonKind::Auto);
    }
}
