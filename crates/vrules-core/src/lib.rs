//! vrules — embedding-based reasoning library.
//!
//! The core mission is **deterministic, observable, semantic reasoning** over
//! structured facts: forward-chaining RETE evaluation, canonical matching
//! (SimHash near-dup + template-based canonicalization), and ANN-backed embedding
//! search via [`em-log-n`](https://github.com/ops-ping/em-log-n).
//!
//! # Scope
//!
//! This library is the reasoning kernel — it has no `main`, no daemon, and no
//! transport. It exposes a small reasoning API: parse GRL, evaluate facts through
//! rust-rule-engine, canonical matching ([`canon`]), and
//! backward-chaining (the [`prove`](prove()) function). Custom functions (including protocol bridges
//! such as MCP) are registered onto the engine by the host application through
//! its generic extension API; the reference host is `vrules-shim`.
//!
//! # Implementation
//!
//! Intentionally **synchronous**: rule evaluation runs on ordinary threads with
//! no async runtime. Deterministic in-thread execution is the design's value-add
//! (see `docs/DESIGN.md`).

#![forbid(unsafe_code)]

pub mod canon;

pub mod address;

mod vec_expr;

pub mod geometry;

#[cfg(all(feature = "rule-engine", feature = "embeddings"))]
pub mod vec_bridge;

#[cfg(feature = "rule-engine")]
pub mod engine_eval;

#[cfg(feature = "rule-engine")]
pub mod prove;

#[cfg(all(feature = "rule-engine", feature = "embeddings"))]
pub use vec_bridge::{VECTOR_FUNCTIONS, register_vector_functions};

#[cfg(feature = "rule-engine")]
pub use engine_eval::{
    EvalOutcome, Result as EvaluationResult, RuleEvaluator, Ruleset, VrulesError, add_json_fact,
    facts_to_json, json_to_rule_value, rule_value_to_json,
};

#[cfg(feature = "rule-engine")]
pub use prove::prove;

#[cfg(feature = "rule-engine")]
pub use rust_rule_engine::streaming::{
    EventMetadata, LateDataStrategy, StateBackend, StateConfig, StateStore, StreamConfig,
    StreamEvent, StreamEventStatus, StreamJoinManager, StreamProcessingResult, StreamProcessor,
    WatermarkStrategy, WindowConfig, WindowType,
};

pub use address::{
    AddressAnalysis, AddressAnalyzer, AddressCandidate, AddressComponent, AddressEmbeddingEvidence,
    AddressFieldEmbeddingHint, AddressFieldEvidence, AddressIndex, AddressIndexMatch,
    AddressIndexRecord, AddressRole, AddressSelectionPolicy, NativeAddressStandardization,
    PolicyDecisionFact, SelectedAddress, StructuredAddressCanonicalizer, address_canonical_key,
    address_field_embedding_hints, address_index_record, address_policy_fact, select_address,
    standardize_structured_address, standardize_structured_address_with_embeddings,
    standardize_structured_with_index, standardize_unstructured_address,
};

#[cfg(feature = "rule-engine")]
pub use address::register_address_functions;
