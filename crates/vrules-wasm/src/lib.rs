//! Browser bindings for the canonical GRL evaluator from `vrules-core`.
//!
//! Authored rules are GRL strings. Evaluation uses the same [`Ruleset`] and
//! [`RuleEvaluator`] path as native hosts. Vector functions consume only real
//! host-supplied embeddings accompanied by a validated model name, SHA-256
//! revision, and output dimension.

use std::sync::Arc;

use serde_json::{json, Value};

use em_log_n::embed::{Embedder, ModelId};
use rust_rule_engine::types::{FunctionMeta, ReturnKind};
use rust_rule_engine::{Facts, RuleEngineError, RustRuleEngine, Value as RuleValue};
use vrules_core::canon::{register_canon_functions, CanonKind, CanonRouter};
use vrules_core::geometry::{ArtifactStore, Axis, Calibration, Provenance, Region};
use vrules_core::{
    add_json_fact, address_index_record, address_policy_fact, register_vector_functions,
    standardize_structured_address, standardize_structured_with_index,
    standardize_unstructured_address, AddressIndex, EvalOutcome, RuleEvaluator, Ruleset,
};

use wasm_bindgen::prelude::*;

/// Validate GRL through the same parser used for evaluation.
#[wasm_bindgen]
pub fn validate_rule(grl: &str) -> JsValue {
    let result = match Ruleset::parse(grl) {
        Ok(_) => json!({ "ok": true, "errors": [] }),
        Err(error) => {
            json!({ "ok": false, "errors": [{ "path": "", "message": error.to_string() }] })
        }
    };
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Backward-chaining proof in the browser: prove a GRL `query` goal against the
/// `grl_rules` knowledge base under `facts` (a JSON object), returning
/// `{ provable, bindings, missing_facts, proof }`. This is the same `vrules_core::prove`
/// the native engine runs — goal-directed backward chaining with no server round-trip.
#[wasm_bindgen]
pub fn prove(grl_rules: &str, query: &str, facts_json: &str) -> Result<JsValue, JsValue> {
    let facts: Value = serde_json::from_str(facts_json)
        .map_err(|e| JsValue::from_str(&format!("invalid facts JSON: {e}")))?;
    let out = vrules_core::prove(grl_rules, query, &facts).map_err(|e| js_error(e.to_string()))?;
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Run the repository's address reference workflow with address-data and
/// reference indexes registered as browser-callable rule functions:
/// - `c_addr_index_score(text)` -> best match score in 0..1000
/// - `b_addr_index_match(text)` -> whether the best match is >= 0.85
/// - `m_addr_index_match_id(text)` -> best matching index id or `""`
#[wasm_bindgen]
pub fn verify_address(
    mode: &str,
    input_json: &str,
    grl: &str,
    index_json: &str,
    reference_json: &str,
) -> Result<JsValue, JsValue> {
    let input: Value = serde_json::from_str(input_json)
        .map_err(|e| JsValue::from_str(&format!("invalid input JSON: {e}")))?;
    let index = build_address_index(index_json)?;
    let reference_index = ReferenceIndex::from_json(reference_json)?;
    let native = standardize_with_optional_index(mode, &input, Some(&index))?;
    let mut policy_fact = address_policy_fact(&input, &native);
    let reference_matches = reference_index.search(&policy_fact.source_text, None);
    let customer_matches = reference_index.search(&policy_fact.source_text, Some("customer"));
    if let Some(best) = customer_matches.first() {
        policy_fact.customer = best.name.clone();
        policy_fact.reference_status = "matched".into();
        policy_fact.reference_name = best.name.clone();
    }

    let fact = serde_json::to_value(&policy_fact)
        .map_err(|e| JsValue::from_str(&format!("policy fact serialize: {e}")))?;
    let engine_out = eval_core_configured(
        grl,
        "AddressDecision",
        &fact,
        true,
        None,
        Arc::new(CanonRouter::new()),
        Arc::new(ArtifactStore::default()),
        |engine| {
            register_address_index_functions(engine, Arc::new(index));
            register_reference_index_functions(engine, Arc::new(reference_index));
        },
    )
    .map_err(|e| JsValue::from_str(&e))?;
    policy_fact = serde_json::from_value(engine_out.facts["AddressDecision"].clone())
        .map_err(|e| JsValue::from_str(&format!("AddressDecision result decode: {e}")))?;

    let result = json!({
        "native": native,
        "policy_fact": policy_fact,
        "reference_matches": reference_matches,
        "grl": grl,
        "engine": engine_out,
        "function_evidence": {
            "addr_index_score": native.matches.first().map(|m| (m.score * 1000.0) as i64).unwrap_or(0),
            "addr_index_match": native.matches.first().is_some_and(|m| m.score >= 0.85),
            "addr_index_match_id": native.matches.first().map(|m| m.id.clone()).unwrap_or_default(),
            "ref_match_name": policy_fact.reference_name,
            "ref_match_count": reference_matches.len(),
            "ref_lexical_hits": reference_matches.iter().filter(|m| m.lexical_score >= 0.45).count(),
        }
    });
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Policy-neutral native address standardization. `index_json` accepts an array
/// of objects shaped as `{ "id": "...", "source": {...} }` or raw address
/// records; matches are returned when an index is supplied.
#[wasm_bindgen]
pub fn address_standardize(
    mode: &str,
    input_json: &str,
    index_json: &str,
) -> Result<JsValue, JsValue> {
    let input: Value = serde_json::from_str(input_json)
        .map_err(|e| JsValue::from_str(&format!("invalid input JSON: {e}")))?;
    let index = build_address_index(index_json)?;
    let out = match (mode, Some(&index).filter(|_| !index_json.trim().is_empty())) {
        ("structured", Some(index)) => standardize_structured_with_index(&input, index, 5),
        ("structured", None) => standardize_structured_address(&input),
        ("unstructured", Some(index)) => {
            let mut out = standardize_unstructured_address(input.as_str().unwrap_or_default());
            out.matches = index.match_standardized(&out, 5);
            out.valid = out.valid || out.matches.first().is_some_and(|m| m.score >= 0.85);
            out.validity_score = out
                .validity_score
                .max(out.matches.first().map(|m| m.score).unwrap_or_default());
            out
        }
        ("unstructured", None) => {
            standardize_unstructured_address(input.as_str().unwrap_or_default())
        }
        (other, _) => {
            return Err(JsValue::from_str(&format!(
                "unknown address mode `{other}`"
            )));
        }
    };
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsValue::from_str(&e.to_string()))
}

fn build_address_index(index_json: &str) -> Result<AddressIndex, JsValue> {
    let records: Vec<Value> = if index_json.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(index_json)
            .map_err(|e| JsValue::from_str(&format!("invalid index JSON: {e}")))?
    };
    Ok(AddressIndex::from_records(
        records.into_iter().enumerate().map(|(i, record)| {
            let id = record
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("record:{i}"));
            let source = record.get("source").cloned().unwrap_or(record);
            address_index_record(id, source)
        }),
    ))
}

fn standardize_with_optional_index(
    mode: &str,
    input: &Value,
    index: Option<&AddressIndex>,
) -> Result<vrules_core::NativeAddressStandardization, JsValue> {
    match (mode, index) {
        ("structured", Some(index)) => Ok(standardize_structured_with_index(input, index, 5)),
        ("structured", None) => Ok(standardize_structured_address(input)),
        ("unstructured", Some(index)) => {
            let mut out = standardize_unstructured_address(input.as_str().unwrap_or_default());
            out.matches = index.match_standardized(&out, 5);
            out.valid = out.valid || out.matches.first().is_some_and(|m| m.score >= 0.85);
            out.validity_score = out
                .validity_score
                .max(out.matches.first().map(|m| m.score).unwrap_or_default());
            Ok(out)
        }
        ("unstructured", None) => Ok(standardize_unstructured_address(
            input.as_str().unwrap_or_default(),
        )),
        (other, _) => Err(JsValue::from_str(&format!(
            "unknown address mode `{other}`"
        ))),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct ReferenceMatch {
    id: String,
    kind: String,
    name: String,
    matched_text: String,
    score: f32,
    exact_score: f32,
    lexical_score: f32,
    data: Value,
}

#[derive(Debug, Clone)]
struct ReferenceRecord {
    id: String,
    kind: String,
    name: String,
    aliases: Vec<String>,
    data: Value,
}

#[derive(Debug, Clone, Default)]
struct ReferenceIndex {
    records: Vec<ReferenceRecord>,
}

impl ReferenceIndex {
    fn from_json(reference_json: &str) -> Result<Self, JsValue> {
        if reference_json.trim().is_empty() {
            return Ok(Self::default());
        }
        let values: Vec<Value> = serde_json::from_str(reference_json)
            .map_err(|e| JsValue::from_str(&format!("invalid reference JSON: {e}")))?;
        let records = values
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                let kind = value
                    .get("kind")
                    .or_else(|| value.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("reference")
                    .to_string();
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| value.get("customer_name").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string();
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{kind}:{i}"));
                let aliases = value
                    .get("aliases")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .chain(std::iter::once(name.clone()))
                    .filter(|s| !s.trim().is_empty())
                    .collect();
                ReferenceRecord {
                    id,
                    kind,
                    name,
                    aliases,
                    data: value,
                }
            })
            .filter(|record| !record.name.is_empty())
            .collect();
        Ok(Self { records })
    }

    fn search(&self, text: &str, kind: Option<&str>) -> Vec<ReferenceMatch> {
        let normalized = normalize_reference_text(text);
        let mut out = Vec::new();
        for record in &self.records {
            if kind.is_some_and(|k| k != record.kind) {
                continue;
            }
            let best = record
                .aliases
                .iter()
                .filter_map(|alias| score_reference_alias(&normalized, alias))
                .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.2.total_cmp(&b.2)));
            let Some((alias, exact_score, lexical_score)) = best else {
                continue;
            };
            let score = ((exact_score * 0.65) + (lexical_score * 0.35)).clamp(0.0, 1.0);
            if score < 0.42 {
                continue;
            }
            out.push(ReferenceMatch {
                id: record.id.clone(),
                kind: record.kind.clone(),
                name: record.name.clone(),
                matched_text: alias,
                score,
                exact_score,
                lexical_score,
                data: record.data.clone(),
            });
        }
        out.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.kind.cmp(&b.kind))
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }
}

fn score_reference_alias(normalized_text: &str, alias: &str) -> Option<(String, f32, f32)> {
    let phrase = normalize_reference_text(alias);
    if phrase.is_empty() {
        return None;
    }
    let exact = phrase_score(normalized_text, &phrase);
    let lexical = lexical_similarity_score(normalized_text, &phrase);
    (exact > 0.0 || lexical >= 0.45).then_some((alias.to_string(), exact, lexical))
}

fn phrase_score(normalized_text: &str, phrase: &str) -> f32 {
    let text_tokens: Vec<_> = normalized_text.split_whitespace().collect();
    let phrase_tokens: Vec<_> = phrase.split_whitespace().collect();
    if phrase_tokens.is_empty() || text_tokens.len() < phrase_tokens.len() {
        return 0.0;
    }
    if text_tokens
        .windows(phrase_tokens.len())
        .any(|window| window == phrase_tokens.as_slice())
    {
        return 1.0;
    }
    let compact_text = normalized_text.replace(' ', "");
    let compact_phrase = phrase.replace(' ', "");
    if compact_text.contains(&compact_phrase) {
        return 0.92;
    }
    0.0
}

fn lexical_similarity_score(text: &str, phrase: &str) -> f32 {
    let text_features = reference_features(text);
    let phrase_features = reference_features(phrase);
    if text_features.is_empty() || phrase_features.is_empty() {
        return 0.0;
    }
    let overlap = phrase_features
        .iter()
        .filter(|feature| text_features.contains(*feature))
        .count();
    overlap as f32 / phrase_features.len() as f32
}

fn reference_features(text: &str) -> std::collections::BTreeSet<String> {
    let tokens: Vec<_> = text.split_whitespace().collect();
    let mut features = std::collections::BTreeSet::new();
    for token in &tokens {
        features.insert(format!("tok:{token}"));
        for gram in token.as_bytes().windows(3) {
            if let Ok(gram) = std::str::from_utf8(gram) {
                features.insert(format!("tri:{gram}"));
            }
        }
    }
    for pair in tokens.windows(2) {
        features.insert(format!("bi:{}:{}", pair[0], pair[1]));
    }
    features
}

fn normalize_reference_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Browser-side GRL evaluator.
#[wasm_bindgen]
pub struct RuleEngine {
    grl: String,
    rule_count: usize,
    prefetched: std::collections::HashMap<String, Vec<f32>>,
    model_id: Option<ModelId>,
    canon_router: Arc<CanonRouter>,
    artifacts: Arc<ArtifactStore>,
}

#[wasm_bindgen]
impl RuleEngine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> RuleEngine {
        console_error_panic_hook::set_once();
        RuleEngine {
            grl: String::new(),
            rule_count: 0,
            prefetched: std::collections::HashMap::new(),
            model_id: None,
            canon_router: Arc::new(CanonRouter::new()),
            artifacts: Arc::new(ArtifactStore::default()),
        }
    }

    /// Parse and append one GRL rule or source string.
    pub fn register_rule(&mut self, grl_source: &str) -> Result<(), JsValue> {
        let combined = if self.grl.trim().is_empty() {
            grl_source.to_string()
        } else {
            format!("{}\n{}", self.grl, grl_source)
        };
        let ruleset =
            Ruleset::parse(combined.clone()).map_err(|error| js_error(error.to_string()))?;
        self.grl = combined;
        self.rule_count = ruleset.rule_count();
        Ok(())
    }

    /// Evaluate one JSON object through the registered GRL source.
    pub fn evaluate(
        &self,
        fact_type: &str,
        fact_data_json: &str,
        want_trace: bool,
    ) -> Result<JsValue, JsValue> {
        let data: Value = serde_json::from_str(fact_data_json)
            .map_err(|e| JsValue::from_str(&format!("invalid data JSON: {e}")))?;
        let embedder = self.prefetched_embedder().map_err(js_error)?;
        let out = eval_core_configured(
            &self.grl,
            fact_type,
            &data,
            want_trace,
            embedder,
            Arc::clone(&self.canon_router),
            Arc::clone(&self.artifacts),
            |_| {},
        )
        .map_err(js_error)?;
        serde_wasm_bindgen::to_value(&out).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Number of registered rules.
    pub fn rule_count(&self) -> usize {
        self.rule_count
    }

    /// Register examples for `s_canon_match` and `m_canon_label`.
    pub fn register_canon_pattern(
        &self,
        label: &str,
        kind: &str,
        threshold: u32,
        examples_json: &str,
    ) -> Result<usize, JsValue> {
        if label.trim().is_empty() {
            return Err(js_error("canonical pattern label must not be empty"));
        }
        if threshold > 64 {
            return Err(js_error(
                "canonical pattern threshold must be between 0 and 64",
            ));
        }
        let kind = match kind.trim().to_ascii_lowercase().as_str() {
            "auto" => CanonKind::Auto,
            "log" => CanonKind::Log,
            "json" => CanonKind::Json,
            other => {
                return Err(js_error(format!(
                    "unknown canonical pattern kind `{other}`; expected auto, log, or json"
                )));
            }
        };
        let examples: Vec<String> = serde_json::from_str(examples_json).map_err(|error| {
            js_error(format!(
                "canonical pattern examples must be a JSON string array: {error}"
            ))
        })?;
        Ok(self
            .canon_router
            .register(label, kind, threshold, &examples))
    }

    /// Inject one real host-produced vector and its model identity.
    pub fn set_embedding(
        &mut self,
        text: &str,
        vector: Vec<f32>,
        model_name: &str,
        model_sha256: &str,
        dimensions: usize,
    ) -> Result<(), JsValue> {
        let model_id = validate_model_id(model_name, model_sha256, dimensions).map_err(js_error)?;
        validate_vector(&vector, dimensions).map_err(js_error)?;
        if self
            .model_id
            .as_ref()
            .is_some_and(|current| current != &model_id)
        {
            return Err(js_error(
                "all prefetched embeddings must use the same model identity",
            ));
        }
        let canonical = CanonKind::Auto.apply(text).canonical;
        self.model_id = Some(model_id);
        self.prefetched.insert(canonical, vector);
        Ok(())
    }

    /// Drop all injected vectors and their model identity.
    pub fn clear_embeddings(&mut self) {
        self.prefetched.clear();
        self.model_id = None;
    }

    /// Load named geometry artifacts (axes/regions) from their JSON form,
    /// replacing any previously loaded set.
    pub fn load_artifacts(&mut self, artifacts_json: &str) -> Result<(), JsValue> {
        self.artifacts = Arc::new(ArtifactStore::from_json(artifacts_json).map_err(js_error)?);
        Ok(())
    }

    /// Constructor tier: fit a named axis from positive/negative exemplar
    /// texts (JSON string arrays) whose vectors were injected via
    /// `set_embedding`, calibrating a percentile window from
    /// `calibration_json` texts.
    pub fn fit_axis(
        &mut self,
        name: &str,
        positive_json: &str,
        negative_json: &str,
        calibration_json: &str,
    ) -> Result<(), JsValue> {
        let positive = self.exemplar_vectors(positive_json)?;
        let negative = self.exemplar_vectors(negative_json)?;
        let mut axis =
            Axis::from_sets(name, self.provenance()?, &positive, &negative).map_err(js_error)?;
        let reference = self
            .exemplar_vectors(calibration_json)?
            .iter()
            .map(|v| axis.project_raw(v))
            .collect::<Result<Vec<f32>, String>>()
            .map_err(js_error)?;
        axis.calibrate(Calibration::from_scores(reference).map_err(js_error)?);
        let mut store = (*self.artifacts).clone();
        store.insert_axis(axis);
        self.artifacts = Arc::new(store);
        Ok(())
    }

    /// Constructor tier: fit a named region from exemplar texts (JSON string
    /// array) whose vectors were injected via `set_embedding`.
    pub fn fit_region(
        &mut self,
        name: &str,
        exemplars_json: &str,
        rank: usize,
        coverage: f32,
    ) -> Result<(), JsValue> {
        let cloud = self.exemplar_vectors(exemplars_json)?;
        let region =
            Region::fit(name, self.provenance()?, &cloud, rank, coverage).map_err(js_error)?;
        let mut store = (*self.artifacts).clone();
        store.insert_region(region);
        self.artifacts = Arc::new(store);
        Ok(())
    }

    /// The loaded/fitted artifact set in its JSON form (for display and
    /// persistence).
    pub fn artifacts_json(&self) -> Result<String, JsValue> {
        self.artifacts.to_json().map_err(js_error)
    }
}

impl RuleEngine {
    fn provenance(&self) -> Result<Provenance, JsValue> {
        let model_id = self
            .model_id
            .as_ref()
            .ok_or_else(|| js_error("inject embeddings before fitting artifacts"))?;
        Ok(Provenance {
            model: model_id.name.clone(),
            dim: model_id.dim,
            task: None,
            exemplar_set: None,
        })
    }

    /// Resolve exemplar texts (a JSON string array) to their injected vectors.
    fn exemplar_vectors(&self, texts_json: &str) -> Result<Vec<Vec<f32>>, JsValue> {
        let texts: Vec<String> = serde_json::from_str(texts_json)
            .map_err(|e| js_error(format!("exemplars must be a JSON string array: {e}")))?;
        texts
            .iter()
            .map(|text| {
                let canonical = CanonKind::Auto.apply(text).canonical;
                self.prefetched
                    .get(&canonical)
                    .cloned()
                    .ok_or_else(|| js_error(format!("no injected embedding for exemplar {text:?}")))
            })
            .collect()
    }

    fn prefetched_embedder(&self) -> Result<Option<Arc<dyn Embedder>>, String> {
        match (&self.model_id, self.prefetched.is_empty()) {
            (None, true) => Ok(None),
            (None, false) => {
                Err("prefetched embeddings are missing validated model metadata".into())
            }
            (Some(_), true) => Err("embedding model metadata has no prefetched vectors".into()),
            (Some(model_id), false) => Ok(Some(Arc::new(PrefetchedEmbedder::new(
                self.prefetched.clone(),
                model_id.clone(),
            )?))),
        }
    }
}

impl Default for RuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluate GRL with the canonical `Ruleset`/`RuleEvaluator` path.
pub fn eval_core(
    grl: &str,
    fact_type: &str,
    data: &Value,
    want_trace: bool,
    embedder: Option<Arc<dyn Embedder>>,
) -> Result<EvalOutcome, String> {
    eval_core_configured(
        grl,
        fact_type,
        data,
        want_trace,
        embedder,
        Arc::new(CanonRouter::new()),
        Arc::new(ArtifactStore::default()),
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)] // internal seam: every host wiring knob is explicit
fn eval_core_configured<F>(
    grl: &str,
    fact_type: &str,
    data: &Value,
    want_trace: bool,
    embedder: Option<Arc<dyn Embedder>>,
    canon_router: Arc<CanonRouter>,
    artifacts: Arc<ArtifactStore>,
    configure: F,
) -> Result<EvalOutcome, String>
where
    F: FnOnce(&mut RustRuleEngine),
{
    let ruleset = Ruleset::parse(grl).map_err(|error| error.to_string())?;
    let mut registration_error = None;
    let engine = ruleset
        .build_engine_with(|engine| {
            register_canon_functions(engine, canon_router);
            if let Some(embedder) = embedder {
                if let Err(error) = register_vector_functions(engine, embedder, artifacts) {
                    registration_error = Some(error);
                }
            }
            configure(engine);
        })
        .map_err(|error| error.to_string())?;
    if let Some(error) = registration_error {
        return Err(error);
    }
    let facts = Facts::new();
    add_json_fact(&facts, fact_type, data).map_err(|error| error.to_string())?;
    if fact_type != "Decision" {
        add_json_fact(&facts, "Decision", &json!({})).map_err(|error| error.to_string())?;
    }
    RuleEvaluator::with_engine(ruleset, engine)
        .evaluate(&facts, want_trace)
        .map_err(|error| error.to_string())
}

fn js_error(message: impl AsRef<str>) -> JsValue {
    JsValue::from_str(message.as_ref())
}

fn validate_model_id(name: &str, sha256: &str, dimensions: usize) -> Result<ModelId, String> {
    let name = name.trim();
    if name.is_empty() || matches!(name, "unspecified" | "prefetched") {
        return Err("embedding model name must identify the real model".into());
    }
    if dimensions == 0 {
        return Err("embedding dimensions must be greater than zero".into());
    }
    let model_id =
        ModelId::from_sha256(name, sha256, dimensions).map_err(|error| error.to_string())?;
    if model_id.digest == [0; 32] {
        return Err("embedding model SHA-256 digest must not be all zeroes".into());
    }
    Ok(model_id)
}

fn validate_vector(vector: &[f32], dimensions: usize) -> Result<(), String> {
    if vector.len() != dimensions {
        return Err(format!(
            "embedding dimension mismatch: model declares {dimensions}, vector has {}",
            vector.len()
        ));
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err("embedding vector contains a non-finite value".into());
    }
    if vector.iter().all(|value| *value == 0.0) {
        return Err("embedding vector must not be an all-zero placeholder".into());
    }
    Ok(())
}

fn register_address_index_functions(engine: &mut RustRuleEngine, index: Arc<AddressIndex>) {
    let score_index = Arc::clone(&index);
    engine.register_function_with_meta(
        "c_addr_index_score",
        FunctionMeta::hot(ReturnKind::CalibratedScalar),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "c_addr_index_score")?;
            let standardized = standardize_unstructured_address(&text);
            let score = score_index
                .match_standardized(&standardized, 1)
                .first()
                .map(|m| (m.score * 1000.0) as i64)
                .unwrap_or(0);
            Ok(RuleValue::Integer(score))
        },
    );

    let match_index = Arc::clone(&index);
    engine.register_function_with_meta(
        "b_addr_index_match",
        FunctionMeta::hot(ReturnKind::Boolean),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "b_addr_index_match")?;
            let standardized = standardize_unstructured_address(&text);
            let matched = match_index
                .match_standardized(&standardized, 1)
                .first()
                .is_some_and(|m| m.score >= 0.85);
            Ok(RuleValue::Boolean(matched))
        },
    );

    engine.register_function_with_meta(
        "m_addr_index_match_id",
        FunctionMeta::hot(ReturnKind::Text),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "m_addr_index_match_id")?;
            let standardized = standardize_unstructured_address(&text);
            let id = index
                .match_standardized(&standardized, 1)
                .first()
                .map(|m| m.id.clone())
                .unwrap_or_default();
            Ok(RuleValue::String(id))
        },
    );
}

fn register_reference_index_functions(engine: &mut RustRuleEngine, index: Arc<ReferenceIndex>) {
    let count_index = Arc::clone(&index);
    engine.register_function_with_meta(
        "c_ref_match_count",
        FunctionMeta::hot(ReturnKind::CalibratedScalar),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "c_ref_match_count")?;
            let kind = fact_arg_string(args, 1, "c_ref_match_count")?;
            Ok(RuleValue::Integer(
                count_index.search(&text, Some(&kind)).len() as i64,
            ))
        },
    );

    let name_index = Arc::clone(&index);
    engine.register_function_with_meta(
        "m_ref_match_name",
        FunctionMeta::hot(ReturnKind::Text),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "m_ref_match_name")?;
            let kind = fact_arg_string(args, 1, "m_ref_match_name")?;
            let name = name_index
                .search(&text, Some(&kind))
                .first()
                .map(|m| m.name.clone())
                .unwrap_or_default();
            Ok(RuleValue::String(name))
        },
    );

    let id_index = Arc::clone(&index);
    engine.register_function_with_meta(
        "m_ref_match_id",
        FunctionMeta::hot(ReturnKind::Text),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "m_ref_match_id")?;
            let kind = fact_arg_string(args, 1, "m_ref_match_id")?;
            let id = id_index
                .search(&text, Some(&kind))
                .first()
                .map(|m| m.id.clone())
                .unwrap_or_default();
            Ok(RuleValue::String(id))
        },
    );

    let exact_index = Arc::clone(&index);
    engine.register_function_with_meta(
        "c_ref_exact_score",
        FunctionMeta::hot(ReturnKind::CalibratedScalar),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "c_ref_exact_score")?;
            let kind = fact_arg_string(args, 1, "c_ref_exact_score")?;
            let score = exact_index
                .search(&text, Some(&kind))
                .first()
                .map(|m| (m.exact_score * 1000.0) as i64)
                .unwrap_or(0);
            Ok(RuleValue::Integer(score))
        },
    );

    let lexical_index = Arc::clone(&index);
    engine.register_function_with_meta(
        "c_ref_lexical_score",
        FunctionMeta::hot(ReturnKind::CalibratedScalar),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "c_ref_lexical_score")?;
            let kind = fact_arg_string(args, 1, "c_ref_lexical_score")?;
            let score = lexical_index
                .search(&text, Some(&kind))
                .first()
                .map(|m| (m.lexical_score * 1000.0) as i64)
                .unwrap_or(0);
            Ok(RuleValue::Integer(score))
        },
    );

    let score_index = Arc::clone(&index);
    engine.register_function_with_meta(
        "c_ref_match_score",
        FunctionMeta::hot(ReturnKind::CalibratedScalar),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "c_ref_match_score")?;
            let kind = fact_arg_string(args, 1, "c_ref_match_score")?;
            let score = score_index
                .search(&text, Some(&kind))
                .first()
                .map(|m| (m.score * 1000.0) as i64)
                .unwrap_or(0);
            Ok(RuleValue::Integer(score))
        },
    );

    engine.register_function_with_meta(
        "b_ref_match",
        FunctionMeta::hot(ReturnKind::Boolean),
        move |args: &[RuleValue], _facts: &Facts| {
            let text = fact_arg_string(args, 0, "b_ref_match")?;
            let kind = fact_arg_string(args, 1, "b_ref_match")?;
            let expected_name = fact_arg_string(args, 2, "b_ref_match")?;
            let matched = index
                .search(&text, Some(&kind))
                .iter()
                .any(|m| m.name == expected_name);
            Ok(RuleValue::Boolean(matched))
        },
    );
}

fn fact_arg_string(
    args: &[RuleValue],
    index: usize,
    function: &str,
) -> rust_rule_engine::Result<String> {
    let value = args
        .get(index)
        .ok_or_else(|| RuleEngineError::EvaluationError {
            message: format!("{function} argument {index} is missing"),
        })?;
    let RuleValue::String(value) = value else {
        return Err(RuleEngineError::EvaluationError {
            message: format!("{function} argument {index} must be a string, got {value:?}"),
        });
    };
    Ok(value.clone())
}

/// Real host-produced vectors keyed by their model identity.
#[derive(Debug)]
struct PrefetchedEmbedder {
    map: std::collections::HashMap<String, Vec<f32>>,
    model_id: ModelId,
}

impl PrefetchedEmbedder {
    fn new(
        map: std::collections::HashMap<String, Vec<f32>>,
        model_id: ModelId,
    ) -> Result<Self, String> {
        if model_id.name.trim().is_empty()
            || matches!(model_id.name.as_str(), "unspecified" | "prefetched")
            || model_id.digest == [0; 32]
            || model_id.dim == 0
        {
            return Err("embedding model metadata must identify a real model".into());
        }
        if map.is_empty() {
            return Err("at least one prefetched embedding is required".into());
        }
        for vector in map.values() {
            validate_vector(vector, model_id.dim)?;
        }
        Ok(Self { map, model_id })
    }
}

impl Embedder for PrefetchedEmbedder {
    fn dim(&self) -> usize {
        self.model_id.dim
    }

    fn model_id(&self) -> ModelId {
        self.model_id.clone()
    }

    fn embed(&self, text: &str) -> em_log_n::Result<Vec<f32>> {
        self.map
            .get(text)
            .cloned()
            .ok_or_else(|| em_log_n::Error::Embed(format!("no prefetched embedding for {text:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_rule_appends_one_combined_grl_source() {
        let mut engine = RuleEngine::new();
        engine
            .register_rule(
                r#"rule "First" no-loop {
                    when Request.score > 0
                    then Decision.first = true;
                }"#,
            )
            .unwrap();
        engine
            .register_rule(
                r#"rule "Second" no-loop {
                    when Request.score > 1
                    then Decision.second = true;
                }"#,
            )
            .unwrap();

        assert_eq!(engine.rule_count(), 2);
        assert_eq!(Ruleset::parse(engine.grl.clone()).unwrap().rule_count(), 2);
    }

    #[test]
    fn registered_canon_pattern_drives_grl_functions() {
        let engine = RuleEngine::new();
        let registered = engine
            .register_canon_pattern("login", "log", 6, r#"["User 1 login from 10.0.0.1"]"#)
            .unwrap();
        assert_eq!(registered, 1);

        let grl = r#"
            rule "CanonMatch" no-loop {
                when
                    b_canon_matches(Request.message, "login") == true
                then
                    Decision.canon_match = true;
            }

            rule "CanonLabel" no-loop {
                when
                    m_canon_label(Request.message) == "login"
                then
                    Decision.canon_label = true;
            }
        "#;
        let out = eval_core_configured(
            grl,
            "Request",
            &json!({ "message": "User 999 login from 192.168.1.7" }),
            false,
            None,
            Arc::clone(&engine.canon_router),
            Arc::new(ArtifactStore::default()),
            |_| {},
        )
        .unwrap();

        assert!(out.fired.contains(&"CanonMatch".to_string()));
        assert!(out.fired.contains(&"CanonLabel".to_string()));
        assert_eq!(out.decision["canon_match"], true);
        assert_eq!(out.decision["canon_label"], true);
    }

    #[test]
    fn nested_objects_remain_structured_and_actions_supply_decision() {
        let grl = r#"
            rule "RouteGold" no-loop {
                when
                    Request.customer.tier == "gold"
                then
                    Decision.route = "priority";
            }
        "#;
        let out = eval_core(
            grl,
            "Request",
            &json!({ "customer": { "tier": "gold", "profile": { "active": true } } }),
            true,
            None,
        )
        .unwrap();
        assert_eq!(out.fired, ["RouteGold"]);
        assert_eq!(out.decision["route"], "priority");
        assert_eq!(out.facts["Request"]["customer"]["tier"], "gold");
        assert_eq!(out.facts["Request"]["customer"]["profile"]["active"], true);
        assert!(out.trace.is_some());
    }

    #[test]
    fn upstream_engine_handles_salience_and_then_actions() {
        let grl = r#"
            rule "LowSalience" salience 10 no-loop {
                when
                    Request.score > 0
                then
                    Decision.order = "low";
            }

            rule "HighSalience" salience 100 no-loop {
                when
                    Request.score > 0
                then
                    Decision.order = "high";
            }
        "#;
        let out = eval_core(grl, "Request", &json!({ "score": 1 }), false, None).unwrap();
        assert_eq!(out.fired, ["HighSalience", "LowSalience"]);
        assert_eq!(out.decision["order"], "low");
    }

    #[test]
    fn address_and_reference_indexes_are_engine_functions() {
        let grl = r#"
            rule "KnownCustomerAddress" no-loop {
                when
                    m_ref_match_name(AddressDecision.source_text, "customer") == "King Cola" &&
                    c_addr_index_score(AddressDecision.standardized) >= 850
                then
                    AddressDecision.policy_status = "accepted";
                    AddressDecision.policy_reason = "Matched both indexes.";
            }
        "#;
        let address_index = AddressIndex::from_records(vec![address_index_record(
            "known-address",
            json!({
                "NUMBER": "500",
                "STREET": "Royal Road",
                "CITY": "Springfield",
                "REGION": "IL",
                "POSTCODE": "62701"
            }),
        )]);
        let reference_index = ReferenceIndex::from_json(
            r#"[{
                "id": "customer:king-cola",
                "kind": "customer",
                "name": "King Cola",
                "aliases": ["King Cola"]
            }]"#,
        )
        .unwrap();
        let fact = json!({
            "customer": "",
            "role": "bill_to",
            "address_valid": true,
            "standardized": "500 Royal Road, Springfield, IL, 62701",
            "source_text": "Bill King Cola at 500 Royal Road",
            "reference_status": "matched",
            "reference_name": "King Cola",
            "policy_status": "pending",
            "policy_reason": ""
        });

        let out = eval_core_configured(
            grl,
            "AddressDecision",
            &fact,
            false,
            None,
            Arc::new(CanonRouter::new()),
            Arc::new(ArtifactStore::default()),
            |engine| {
                register_address_index_functions(engine, Arc::new(address_index));
                register_reference_index_functions(engine, Arc::new(reference_index));
            },
        )
        .unwrap();

        assert_eq!(out.fired, ["KnownCustomerAddress"]);
        assert_eq!(out.facts["AddressDecision"]["policy_status"], "accepted");
        assert_eq!(
            out.facts["AddressDecision"]["policy_reason"],
            "Matched both indexes."
        );
    }

    #[test]
    fn index_functions_reject_missing_and_non_string_arguments() {
        let missing = eval_core_configured(
            r#"
                rule "MissingAddressArg" no-loop {
                    when c_addr_index_score() >= 0
                    then Decision.matched = true;
                }
            "#,
            "AddressDecision",
            &json!({}),
            false,
            None,
            Arc::new(CanonRouter::new()),
            Arc::new(ArtifactStore::default()),
            |engine| {
                register_address_index_functions(
                    engine,
                    Arc::new(AddressIndex::from_records(Vec::new())),
                );
            },
        )
        .unwrap_err();
        assert!(missing.contains("c_addr_index_score argument 0 is missing"));

        let wrong_type = eval_core_configured(
            r#"
                rule "WrongReferenceArg" no-loop {
                    when c_ref_match_count(AddressDecision.enabled, "customer") >= 0
                    then Decision.matched = true;
                }
            "#,
            "AddressDecision",
            &json!({ "enabled": true }),
            false,
            None,
            Arc::new(CanonRouter::new()),
            Arc::new(ArtifactStore::default()),
            |engine| {
                register_reference_index_functions(engine, Arc::new(ReferenceIndex::default()));
            },
        )
        .unwrap_err();
        assert!(wrong_type.contains("c_ref_match_count argument 0 must be a string"));
    }

    #[test]
    fn prefetched_vectors_require_real_model_metadata() {
        let mut map = std::collections::HashMap::new();
        map.insert("king".into(), vec![1.0, 0.0]);
        let error = PrefetchedEmbedder::new(map, ModelId::unspecified(2)).unwrap_err();
        assert_eq!(error, "embedding model metadata must identify a real model");
        assert!(validate_model_id("EmbeddingGemma", &"00".repeat(32), 2).is_err());
    }

    #[test]
    fn missing_vector_function_and_vectors_error_explicitly() {
        // With no embedder, s_cosine is never registered: load-time lint.
        let unregistered = r#"
            rule "AboutRoyalty" no-loop {
                when
                    s_cosine(Concept.word, "royalty") == 1.0
                then
                    Decision.about_royalty = true;
            }
        "#;
        let metadata_error = eval_core(
            unregistered,
            "Concept",
            &json!({ "word": "king" }),
            false,
            None,
        )
        .unwrap_err();
        assert!(
            metadata_error.contains("s_cosine") && metadata_error.contains("not registered"),
            "unexpected missing-function error: {metadata_error}"
        );

        // With an embedder but a missing prefetched vector, the measurement
        // rule fails at execution when it embeds the anchor text.
        let layered = r#"
            rule "MeasureRoyalty" no-loop {
                when
                    Concept.word == "king"
                then
                    Concept.royal_sim = s_cosine(Concept.word, "royalty");
            }
        "#;
        let model_id = validate_model_id("EmbeddingGemma-300M", &"11".repeat(32), 2).unwrap();
        let mut map = std::collections::HashMap::new();
        map.insert("king".into(), vec![1.0, 0.0]);
        let embedder: Arc<dyn Embedder> = Arc::new(PrefetchedEmbedder::new(map, model_id).unwrap());
        let vector_error = eval_core(
            layered,
            "Concept",
            &json!({ "word": "king" }),
            false,
            Some(embedder),
        )
        .unwrap_err();
        assert!(vector_error.contains("no prefetched embedding for \"royalty\""));
    }

    #[test]
    fn raw_scalar_threshold_is_rejected_at_load() {
        let grl = r#"
            rule "AboutRoyalty" no-loop {
                when
                    s_cosine(Concept.word, "royalty") > 0.99
                then
                    Decision.about_royalty = true;
            }
        "#;
        let model_id = validate_model_id("EmbeddingGemma-300M", &"11".repeat(32), 2).unwrap();
        let mut map = std::collections::HashMap::new();
        map.insert("king".into(), vec![1.0, 0.0]);
        let embedder: Arc<dyn Embedder> = Arc::new(PrefetchedEmbedder::new(map, model_id).unwrap());
        let error = eval_core(
            grl,
            "Concept",
            &json!({ "word": "king" }),
            false,
            Some(embedder),
        )
        .unwrap_err();
        assert!(
            error.contains("raw scalar"),
            "expected the raw-scalar lint, got: {error}"
        );
    }

    #[test]
    fn vector_function_text_without_a_call_needs_no_embedder() {
        let grl = r#"
            rule "Documents s_cosine" no-loop {
                when
                    Request.enabled == true
                then
                    Decision.allowed = true;
            }
        "#;
        let out = eval_core(grl, "Request", &json!({ "enabled": true }), false, None).unwrap();
        assert_eq!(out.fired, ["Documents s_cosine"]);
        assert_eq!(out.decision["allowed"], true);
    }

    #[test]
    fn vector_rules_reuse_host_vectors_and_write_decisions() {
        // Layered idiom: a measurement rule assigns the raw contrast score to
        // a fact; a decision rule thresholds the fact.
        let grl = r#"
            rule "MeasurePolarity" no-loop {
                when
                    Concept.word == "queen"
                then
                    Concept.polarity = s_contrast(Concept.word, "king", "man");
            }

            rule "GrantRoyalAccess" no-loop {
                when
                    Concept.polarity > 0.5
                then
                    Decision.access_granted = true;
            }
        "#;
        let model_id = validate_model_id("EmbeddingGemma-300M", &"11".repeat(32), 2).unwrap();
        let map = [
            ("king".into(), vec![1.0, 1.0]),
            ("man".into(), vec![0.0, 1.0]),
            ("queen".into(), vec![1.0, -1.0]),
        ]
        .into_iter()
        .collect();
        let embedder: Arc<dyn Embedder> =
            Arc::new(PrefetchedEmbedder::new(map, model_id.clone()).unwrap());
        assert_eq!(embedder.model_id(), model_id);

        let out = eval_core(
            grl,
            "Concept",
            &json!({ "word": "queen" }),
            true,
            Some(embedder),
        );
        let out = out.unwrap();
        assert_eq!(out.fired, ["MeasurePolarity", "GrantRoyalAccess"]);
        // cos(queen, king) = 0, cos(queen, man) < 0 → contrast is positive.
        let polarity = out.facts["Concept"]["polarity"].as_f64().unwrap();
        assert!(polarity > 0.5, "polarity {polarity}");
        assert_eq!(out.decision["access_granted"], true);
    }

    #[test]
    fn browser_fit_and_triage_path_works_end_to_end() {
        let sha = "44".repeat(32);
        let mut engine = RuleEngine::new();
        // Toy 3-dim space: dim1 = urgency signal, dim2 = routine signal.
        fn inject(engine: &mut RuleEngine, sha: &str, text: &str, v: [f32; 3]) {
            engine
                .set_embedding(text, v.to_vec(), "TestModel", sha, 3)
                .unwrap();
        }
        let urgent = [
            ("wire funds immediately, ceo order", [1.0, 2.1, 0.0]),
            ("urgent confidential transfer today", [1.0, 1.9, 0.1]),
            ("act now, penalty deadline", [1.0, 2.0, 0.2]),
        ];
        let calm = [
            ("attached the usual monthly invoice", [1.0, 0.1, 2.0]),
            ("regular payment schedule attached", [1.0, 0.0, 1.9]),
            ("thanks, invoice as usual", [1.0, 0.2, 2.1]),
        ];
        for (text, v) in urgent.iter().chain(calm.iter()) {
            inject(&mut engine, &sha, text, *v);
        }
        inject(
            &mut engine,
            &sha,
            "quarterly report attached",
            [1.0, 0.3, 1.5],
        );
        inject(&mut engine, &sha, "meeting notes, regards", [1.0, 0.2, 1.2]);

        let urgent_texts =
            serde_json::to_string(&urgent.iter().map(|(t, _)| *t).collect::<Vec<_>>()).unwrap();
        let calm_texts =
            serde_json::to_string(&calm.iter().map(|(t, _)| *t).collect::<Vec<_>>()).unwrap();
        let calibration = serde_json::to_string(&[
            "attached the usual monthly invoice",
            "regular payment schedule attached",
            "thanks, invoice as usual",
            "quarterly report attached",
            "meeting notes, regards",
        ])
        .unwrap();

        engine
            .fit_axis(
                "urgency_pressure_v1",
                &urgent_texts,
                &calm_texts,
                &calibration,
            )
            .unwrap();
        engine
            .fit_region("bec_phrasing_v1", &urgent_texts, 2, 0.95)
            .unwrap();

        // The fitted set is exportable and reloadable (the persistence path).
        let artifacts_json = engine.artifacts_json().unwrap();
        assert!(artifacts_json.contains("urgency_pressure_v1"));
        assert!(artifacts_json.contains("bec_phrasing_v1"));
        engine.load_artifacts(&artifacts_json).unwrap();

        let grl = r#"rule "HoldUrgent" no-loop {
            when
                c_project(Payment.text, "urgency_pressure_v1") >= 90.0 &&
                Payment.new_payee == true
            then
                Decision.action = "hold";
        }"#;

        inject(
            &mut engine,
            &sha,
            "urgent: wire immediately per the ceo",
            [1.0, 2.2, 0.1],
        );
        inject(
            &mut engine,
            &sha,
            "the usual invoice attached, thanks",
            [1.0, 0.1, 2.0],
        );
        let embedder = engine.prefetched_embedder().unwrap();
        let held = eval_core_configured(
            grl,
            "Payment",
            &json!({ "text": "urgent: wire immediately per the ceo", "new_payee": true }),
            false,
            embedder.clone(),
            Arc::new(CanonRouter::new()),
            Arc::clone(&engine.artifacts),
            |_| {},
        )
        .unwrap();
        assert_eq!(held.fired, ["HoldUrgent"]);
        assert_eq!(held.decision["action"], "hold");

        let routine = eval_core_configured(
            grl,
            "Payment",
            &json!({ "text": "the usual invoice attached, thanks", "new_payee": true }),
            false,
            embedder,
            Arc::new(CanonRouter::new()),
            Arc::clone(&engine.artifacts),
            |_| {},
        )
        .unwrap();
        assert!(routine.fired.is_empty(), "fired: {:?}", routine.fired);
    }
}
