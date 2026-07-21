//! Address-domain adapters for the repository's reference business workflow.
//!
//! Address verification is not part of the generic framework semantics. These
//! adapters make the reference implementation executable across libpostal-backed
//! native functions, browser/WASM analysis, and future batch/DataFusion hosts.
//! Policy stays in shared rules: analyzers report normalized facts, and authored
//! rules decide whether to save, prefer, or reject them.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "rule-engine")]
mod bridge;
#[cfg(feature = "rule-engine")]
pub use bridge::register_address_functions;
mod native;
pub use native::{
    AddressEmbeddingEvidence, AddressFieldEmbeddingHint, AddressFieldEvidence, AddressIndex,
    AddressIndexMatch, AddressIndexRecord, NativeAddressStandardization,
    StructuredAddressCanonicalizer, address_canonical_key, address_field_embedding_hints,
    address_index_record, standardize_structured_address,
    standardize_structured_address_with_embeddings, standardize_structured_with_index,
    standardize_unstructured_address,
};
mod verification;
pub use verification::{PolicyDecisionFact, address_policy_fact};

/// One parsed component from an address analyzer such as libpostal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressComponent {
    pub label: String,
    pub value: String,
}

/// Analyzer output for one address-like string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressAnalysis {
    pub input: String,
    pub standardized: String,
    #[serde(default)]
    pub components: Vec<AddressComponent>,
    /// Rule-friendly confidence in `0.0..=1.0`. The analyzer supplies evidence;
    /// rules decide thresholds.
    pub confidence: f32,
}

impl AddressAnalysis {
    #[must_use]
    pub fn component(&self, label: &str) -> Option<&str> {
        self.components
            .iter()
            .find(|c| c.label == label)
            .map(|c| c.value.as_str())
    }

    #[must_use]
    pub fn has_minimum_shape(&self) -> bool {
        self.component("road").is_some()
            && (self.component("house_number").is_some()
                || self.component("postcode").is_some()
                || self.component("city").is_some())
    }
}

/// Pluggable analyzer used by native, browser/WASM, and batch hosts.
pub trait AddressAnalyzer: Send + Sync {
    /// Analyze one address-like string.
    fn analyze(&self, text: &str) -> Result<AddressAnalysis, String>;
}

/// Business role of an address candidate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AddressRole {
    BrandOwner,
    BillTo,
    ShipTo,
    Distributor,
    Retailer,
    Other(String),
}

impl AddressRole {
    #[must_use]
    pub fn as_key(&self) -> &str {
        match self {
            Self::BrandOwner => "brand_owner",
            Self::BillTo => "bill_to",
            Self::ShipTo => "ship_to",
            Self::Distributor => "distributor",
            Self::Retailer => "retailer",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// One selectable customer address after analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressCandidate {
    pub id: String,
    pub role: AddressRole,
    pub analysis: AddressAnalysis,
    #[serde(default)]
    pub hierarchy_depth: i64,
    #[serde(default)]
    pub attributes: Value,
}

/// Rule-authored address preference knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressSelectionPolicy {
    /// Role weights, e.g. `{ "bill_to": 100, "brand_owner": -10 }`.
    #[serde(default)]
    pub role_weights: HashMap<String, i64>,
    /// Attribute equality bonuses, keyed as `"key=value"`.
    #[serde(default)]
    pub attribute_weights: HashMap<String, i64>,
    /// Multiplier applied to analyzer confidence.
    #[serde(default = "default_confidence_weight")]
    pub confidence_weight: i64,
    /// Penalty per hierarchy level when lower-level entities should win.
    #[serde(default)]
    pub depth_penalty: i64,
}

fn default_confidence_weight() -> i64 {
    100
}

impl Default for AddressSelectionPolicy {
    fn default() -> Self {
        Self {
            role_weights: HashMap::new(),
            attribute_weights: HashMap::new(),
            confidence_weight: default_confidence_weight(),
            depth_penalty: 0,
        }
    }
}

/// Selected address plus the rule-visible score.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectedAddress {
    pub candidate: AddressCandidate,
    pub score: i64,
}

/// Select the highest-scoring address according to an authored policy.
#[must_use]
pub fn select_address(
    candidates: &[AddressCandidate],
    policy: &AddressSelectionPolicy,
) -> Option<SelectedAddress> {
    candidates
        .iter()
        .cloned()
        .map(|candidate| {
            let mut score = policy
                .role_weights
                .get(candidate.role.as_key())
                .copied()
                .unwrap_or_default();
            score += (candidate.analysis.confidence * policy.confidence_weight as f32) as i64;
            score -= candidate.hierarchy_depth * policy.depth_penalty;
            if let Some(attrs) = candidate.attributes.as_object() {
                for (key, value) in attrs {
                    let value_key = value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string());
                    let lookup = format!("{key}={value_key}");
                    score += policy
                        .attribute_weights
                        .get(&lookup)
                        .copied()
                        .unwrap_or_default();
                }
            }
            SelectedAddress { candidate, score }
        })
        .max_by_key(|selected| selected.score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn candidate(id: &str, role: AddressRole, confidence: f32, depth: i64) -> AddressCandidate {
        AddressCandidate {
            id: id.to_string(),
            role,
            analysis: AddressAnalysis {
                input: "1 Main St".into(),
                standardized: "1 main st".into(),
                components: vec![
                    AddressComponent {
                        label: "house_number".into(),
                        value: "1".into(),
                    },
                    AddressComponent {
                        label: "road".into(),
                        value: "main st".into(),
                    },
                ],
                confidence,
            },
            hierarchy_depth: depth,
            attributes: json!({}),
        }
    }

    #[test]
    fn selection_uses_authored_role_weights() {
        let candidates = vec![
            candidate("brand", AddressRole::BrandOwner, 1.0, 0),
            candidate("bill", AddressRole::BillTo, 0.8, 3),
        ];
        let policy = AddressSelectionPolicy {
            role_weights: HashMap::from([("bill_to".into(), 100), ("brand_owner".into(), -20)]),
            confidence_weight: 10,
            depth_penalty: 0,
            attribute_weights: HashMap::new(),
        };
        let selected = select_address(&candidates, &policy).unwrap();
        assert_eq!(selected.candidate.id, "bill");
    }

    #[test]
    fn minimum_shape_requires_road_plus_anchor() {
        let analysis = candidate("x", AddressRole::BillTo, 1.0, 0).analysis;
        assert!(analysis.has_minimum_shape());
        let weak = AddressAnalysis {
            input: "main".into(),
            standardized: "main".into(),
            components: vec![AddressComponent {
                label: "road".into(),
                value: "main".into(),
            }],
            confidence: 0.2,
        };
        assert!(!weak.has_minimum_shape());
    }
}
