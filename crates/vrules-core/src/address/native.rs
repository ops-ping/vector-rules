use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use vrules_canon::{CanonMode, CanonResult, Canonicalizer, fnv1a_64};

use super::{AddressAnalysis, AddressComponent};

/// Stable canonicalizer for structured address values. The output is suitable
/// for embedding cache namespaces and native index keys.
#[derive(Debug, Default, Clone, Copy)]
pub struct StructuredAddressCanonicalizer;

impl Canonicalizer for StructuredAddressCanonicalizer {
    fn id(&self) -> &str {
        "structured-address"
    }

    fn version(&self) -> u32 {
        1
    }

    fn canon(&self, input: &str) -> CanonResult {
        let components = components_from_text(input);
        CanonResult::new(canonical_line(&components), Vec::new(), CanonMode::Json)
    }
}

/// Hint emitted once per distinct source field name. Hosts can embed
/// `embedding_text` through the embedding component and cache by `cache_namespace`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressFieldEmbeddingHint {
    pub field: String,
    pub embedding_text: String,
    pub cache_namespace: String,
}

/// Evidence for one source field. `embedding_score` is supplied by hosts that
/// have an embedding model available; deterministic name/content evidence is
/// always present for browser and batch operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressFieldEvidence {
    pub path: String,
    pub value: String,
    pub role: String,
    pub name_score: f32,
    pub content_score: f32,
    pub embedding_score: Option<f32>,
    pub score: f32,
}

/// Cached semantic evidence supplied by a host after embedding field names.
/// One score is keyed by source path and target role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressEmbeddingEvidence {
    pub path: String,
    pub role: String,
    pub score: f32,
}

/// Native, policy-neutral standardized address record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NativeAddressStandardization {
    pub source_kind: String,
    pub source: Value,
    pub extracted: String,
    pub canonical: String,
    pub display: String,
    pub components: Map<String, Value>,
    pub validity_score: f32,
    pub valid: bool,
    #[serde(default)]
    pub field_evidence: Vec<AddressFieldEvidence>,
    #[serde(default)]
    pub embedding_hints: Vec<AddressFieldEmbeddingHint>,
    #[serde(default)]
    pub matches: Vec<AddressIndexMatch>,
}

impl NativeAddressStandardization {
    #[must_use]
    pub fn analysis(&self) -> AddressAnalysis {
        AddressAnalysis {
            input: self.extracted.clone(),
            standardized: self.display.clone(),
            components: self
                .components
                .iter()
                .filter_map(|(label, value)| {
                    value.as_str().map(|value| AddressComponent {
                        label: label.clone(),
                        value: value.to_string(),
                    })
                })
                .collect(),
            confidence: self.validity_score,
        }
    }
}

/// One canonical address record ingested from an allowed address-data source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressIndexRecord {
    pub id: String,
    pub canonical: String,
    pub display: String,
    pub components: Map<String, Value>,
    #[serde(default)]
    pub source: Value,
}

/// Match produced by the native Rust index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AddressIndexMatch {
    pub id: String,
    pub score: f32,
    pub display: String,
    #[serde(default)]
    pub source: Value,
}

/// Dependency-free native address index. It is intentionally small and
/// deterministic; larger hosts can shard or replace this behind the same record
/// and match types.
#[derive(Debug, Clone, Default)]
pub struct AddressIndex {
    records: Vec<AddressIndexRecord>,
    token_postings: HashMap<String, Vec<usize>>,
}

impl AddressIndex {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn from_records(records: impl IntoIterator<Item = AddressIndexRecord>) -> Self {
        let mut index = Self::new();
        for record in records {
            index.insert(record);
        }
        index
    }

    pub fn insert(&mut self, record: AddressIndexRecord) {
        let idx = self.records.len();
        for token in address_tokens(&record.canonical) {
            self.token_postings.entry(token).or_default().push(idx);
        }
        self.records.push(record);
    }

    #[must_use]
    pub fn match_standardized(
        &self,
        standardized: &NativeAddressStandardization,
        limit: usize,
    ) -> Vec<AddressIndexMatch> {
        let query_tokens = address_tokens(&standardized.canonical);
        if query_tokens.is_empty() {
            return Vec::new();
        }
        let mut candidates: HashMap<usize, usize> = HashMap::new();
        for token in &query_tokens {
            if let Some(postings) = self.token_postings.get(token) {
                for idx in postings {
                    *candidates.entry(*idx).or_default() += 1;
                }
            }
        }
        let mut scored: Vec<_> = candidates
            .into_iter()
            .filter_map(|(idx, overlap)| {
                let record = self.records.get(idx)?;
                let record_tokens = address_tokens(&record.canonical);
                let union = query_tokens.len() + record_tokens.len() - overlap;
                let token_score = if union == 0 {
                    0.0
                } else {
                    overlap as f32 / union as f32
                };
                let component_score =
                    component_similarity(&standardized.components, &record.components);
                let score = (token_score * 0.55) + (component_score * 0.45);
                Some(AddressIndexMatch {
                    id: record.id.clone(),
                    score,
                    display: record.display.clone(),
                    source: record.source.clone(),
                })
            })
            .filter(|m| m.score >= 0.50)
            .collect();
        scored.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        scored.truncate(limit);
        scored
    }
}

#[must_use]
pub fn address_field_embedding_hints(input: &Value) -> Vec<AddressFieldEmbeddingHint> {
    let mut hints = Vec::new();
    collect_field_hints("$", input, &mut hints);
    hints.sort_by(|a, b| a.field.cmp(&b.field));
    hints.dedup_by(|a, b| a.field == b.field);
    hints
}

/// Standardize unstructured text into canonical address components.
#[must_use]
pub fn standardize_unstructured_address(text: &str) -> NativeAddressStandardization {
    let extracted = extract_address_span(text).unwrap_or_else(|| text.trim().to_string());
    let components = components_from_text(&extracted);
    standardization(
        "unstructured",
        Value::String(text.to_string()),
        extracted,
        components,
        Vec::new(),
    )
}

/// Standardize arbitrary JSON by scanning every scalar field for address
/// evidence and assembling the highest-confidence component set.
#[must_use]
pub fn standardize_structured_address(input: &Value) -> NativeAddressStandardization {
    let mut evidence = Vec::new();
    collect_field_evidence("$", input, &mut evidence);
    standardize_structured_from_evidence(input, evidence)
}

/// Standardize arbitrary JSON with host-supplied cached embedding evidence.
#[must_use]
pub fn standardize_structured_address_with_embeddings(
    input: &Value,
    embeddings: &[AddressEmbeddingEvidence],
) -> NativeAddressStandardization {
    let mut evidence = Vec::new();
    collect_field_evidence("$", input, &mut evidence);
    apply_embedding_evidence(&mut evidence, embeddings);
    standardize_structured_from_evidence(input, evidence)
}

fn standardize_structured_from_evidence(
    input: &Value,
    evidence: Vec<AddressFieldEvidence>,
) -> NativeAddressStandardization {
    let components = components_from_evidence(&evidence);
    let extracted = canonical_line(&components);
    let mut out = standardization("structured", input.clone(), extracted, components, evidence);
    out.embedding_hints = address_field_embedding_hints(input);
    out
}

/// Standardize and query a native index, returning the address-data match
/// evidence in the same result object used by streaming and WASM callers.
#[must_use]
pub fn standardize_structured_with_index(
    input: &Value,
    index: &AddressIndex,
    limit: usize,
) -> NativeAddressStandardization {
    let mut out = standardize_structured_address(input);
    out.matches = index.match_standardized(&out, limit);
    out.valid = out.valid || out.matches.first().is_some_and(|m| m.score >= 0.85);
    out.validity_score = out
        .validity_score
        .max(out.matches.first().map(|m| m.score).unwrap_or_default());
    out
}

fn apply_embedding_evidence(
    evidence: &mut [AddressFieldEvidence],
    embeddings: &[AddressEmbeddingEvidence],
) {
    for item in evidence {
        let semantic = embeddings
            .iter()
            .filter(|e| e.path == item.path && e.role == item.role)
            .map(|e| e.score)
            .max_by(f32::total_cmp);
        if let Some(score) = semantic {
            let score = score.clamp(0.0, 1.0);
            item.embedding_score = Some(score);
            item.score = (item.name_score * 0.30) + (item.content_score * 0.35) + (score * 0.35);
        }
    }
}

#[must_use]
pub fn address_index_record(id: impl Into<String>, source: Value) -> AddressIndexRecord {
    let standardized = if source.is_object() {
        standardize_structured_address(&source)
    } else {
        standardize_unstructured_address(source.as_str().unwrap_or_default())
    };
    AddressIndexRecord {
        id: id.into(),
        canonical: standardized.canonical,
        display: standardized.display,
        components: standardized.components,
        source,
    }
}

fn standardization(
    source_kind: &str,
    source: Value,
    extracted: String,
    components: Map<String, Value>,
    field_evidence: Vec<AddressFieldEvidence>,
) -> NativeAddressStandardization {
    let canonical = canonical_line(&components);
    let display = display_line(&components);
    let validity_score = validity_score(&components);
    NativeAddressStandardization {
        source_kind: source_kind.to_string(),
        source,
        extracted,
        canonical,
        display,
        components,
        validity_score,
        valid: validity_score >= 0.72,
        field_evidence,
        embedding_hints: Vec::new(),
        matches: Vec::new(),
    }
}

fn collect_field_hints(path: &str, value: &Value, out: &mut Vec<AddressFieldEmbeddingHint>) {
    match value {
        Value::Object(obj) => {
            for (key, value) in obj {
                let child = format!("{path}.{key}");
                out.push(AddressFieldEmbeddingHint {
                    field: child.clone(),
                    embedding_text: address_field_embedding_text(key),
                    cache_namespace: format!(
                        "{}:{}",
                        StructuredAddressCanonicalizer.id(),
                        StructuredAddressCanonicalizer.version()
                    ),
                });
                collect_field_hints(&child, value, out);
            }
        }
        Value::Array(values) => {
            for (i, value) in values.iter().enumerate() {
                collect_field_hints(&format!("{path}[{i}]"), value, out);
            }
        }
        _ => {}
    }
}

fn collect_field_evidence(path: &str, value: &Value, out: &mut Vec<AddressFieldEvidence>) {
    match value {
        Value::Object(obj) => {
            for (key, value) in obj {
                collect_field_evidence(&format!("{path}.{key}"), value, out);
            }
        }
        Value::Array(values) => {
            for (i, value) in values.iter().enumerate() {
                collect_field_evidence(&format!("{path}[{i}]"), value, out);
            }
        }
        _ => {
            let value_s = scalar_text(value);
            if value_s.trim().is_empty() {
                return;
            }
            let field = path.rsplit('.').next().unwrap_or(path);
            for role in ADDRESS_ROLES {
                let name_score = name_role_score(field, role);
                let content_score = content_role_score(&value_s, role);
                let score = (name_score * 0.45) + (content_score * 0.55);
                if score >= 0.35 {
                    out.push(AddressFieldEvidence {
                        path: path.to_string(),
                        value: value_s.clone(),
                        role: (*role).to_string(),
                        name_score,
                        content_score,
                        embedding_score: None,
                        score,
                    });
                }
            }
        }
    }
}

fn components_from_evidence(evidence: &[AddressFieldEvidence]) -> Map<String, Value> {
    let mut best: HashMap<&str, &AddressFieldEvidence> = HashMap::new();
    for ev in evidence {
        let path = normalize_field_text(&ev.path);
        if path.contains("number") && has_digit(&ev.value) {
            best.insert("house_number", ev);
        }
        if path.contains("street") && !has_digit(&ev.value) {
            best.insert("street_name", ev);
        }
        let existing = best
            .get(ev.role.as_str())
            .map(|e| e.score)
            .unwrap_or_default();
        if ev.score > existing {
            best.insert(ev.role.as_str(), ev);
        }
    }
    let mut out = Map::new();
    if let Some(ev) = best.get("customer") {
        out.insert("customer".into(), json!(ev.value));
    }
    if let Some(ev) = best.get("address_role") {
        out.insert("address_role".into(), json!(normalize_role(&ev.value)));
    }
    if let Some(ev) = best.get("address_line1") {
        let parsed = components_from_text(&ev.value);
        copy_component(&mut out, &parsed, "house_number");
        copy_component(&mut out, &parsed, "road");
        if !out.contains_key("road") {
            out.insert("road".into(), json!(title_words(&ev.value)));
        }
    }
    if let Some(ev) = best.get("house_number") {
        out.insert("house_number".into(), json!(trim_punct(&ev.value)));
    }
    if let Some(ev) = best.get("street_name") {
        out.insert("road".into(), json!(title_words(&ev.value)));
    }
    for (role, component) in [
        ("city", "city"),
        ("region", "region"),
        ("postal_code", "postal_code"),
        ("country", "country"),
    ] {
        if let Some(ev) = best.get(role) {
            let normalized = match role {
                "city" => title_words(&ev.value),
                "region" | "country" => ev.value.to_ascii_uppercase(),
                _ => ev.value.clone(),
            };
            out.insert(component.into(), json!(normalized));
        }
    }
    out
}

fn components_from_text(text: &str) -> Map<String, Value> {
    let mut out = Map::new();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if let Some(first) = tokens.first()
        && first.chars().any(|c| c.is_ascii_digit())
        && !is_zip(trim_punct(first))
    {
        out.insert("house_number".into(), json!(trim_punct(first)));
    }
    if let Some(zip) = tokens.iter().rev().find(|t| is_zip(trim_punct(t))) {
        out.insert("postal_code".into(), json!(trim_punct(zip)));
    }
    if let Some((i, state)) = tokens
        .iter()
        .enumerate()
        .rev()
        .find(|(_, t)| is_state(trim_punct(t)))
    {
        out.insert(
            "region".into(),
            json!(trim_punct(state).to_ascii_uppercase()),
        );
        if i > 0 {
            out.insert("city".into(), json!(title_words(trim_punct(tokens[i - 1]))));
        }
    }
    if !tokens.is_empty() {
        let street_end = tokens
            .iter()
            .position(|t| is_street_suffix(trim_punct(t)))
            .map(|i| i + 1)
            .unwrap_or_else(|| tokens.len().min(4));
        let street_start = if str_component(&out, "house_number").is_some() {
            1
        } else {
            0
        };
        if street_end > street_start {
            out.insert(
                "road".into(),
                json!(title_words(&tokens[street_start..street_end].join(" "))),
            );
        }
    }
    out
}

fn extract_address_span(text: &str) -> Option<String> {
    let cleaned = text.replace(['\n', '\r'], " ");
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    let start = address_span_start(&tokens)?;
    let mut end = tokens.len();
    for (i, token) in tokens.iter().enumerate().skip(start + 1) {
        let trimmed = trim_punct(token);
        if is_state(trimmed) {
            end = if tokens
                .get(i + 1)
                .is_some_and(|next| is_zip(trim_punct(next)))
            {
                (i + 2).min(tokens.len())
            } else {
                (i + 1).min(tokens.len())
            };
            break;
        }
        if is_zip(trimmed) {
            end = (i + 1).min(tokens.len());
            break;
        }
    }
    Some(tokens[start..end].join(" "))
}

fn address_span_start(tokens: &[&str]) -> Option<usize> {
    if let Some(numbered) = tokens
        .iter()
        .position(|t| has_digit(t) && !is_zip(trim_punct(t)))
    {
        return Some(numbered);
    }
    let suffix = tokens
        .iter()
        .position(|t| is_street_suffix(trim_punct(t)))?;
    let mut start = suffix;
    while start > 0 {
        let previous = trim_punct(tokens[start - 1]);
        if previous.is_empty()
            || matches!(
                previous.to_ascii_lowercase().as_str(),
                "is" | "to" | "for" | "the" | "a" | "an" | "requested" | "bill-to" | "ship-to"
            )
            || !starts_uppercase(previous)
        {
            break;
        }
        start -= 1;
    }
    (start < suffix).then_some(start)
}

fn component_similarity(left: &Map<String, Value>, right: &Map<String, Value>) -> f32 {
    let keys = [
        "house_number",
        "road",
        "city",
        "region",
        "postal_code",
        "country",
    ];
    let mut present = 0.0;
    let mut matched = 0.0;
    for key in keys {
        let l = str_component(left, key);
        let r = str_component(right, key);
        if l.is_some() || r.is_some() {
            present += 1.0;
        }
        if let (Some(l), Some(r)) = (l, r)
            && normalize_address_token(l) == normalize_address_token(r)
        {
            matched += 1.0;
        }
    }
    if present == 0.0 {
        0.0
    } else {
        matched / present
    }
}

fn validity_score(c: &Map<String, Value>) -> f32 {
    let mut score = 0.0;
    if str_component(c, "house_number").is_some() {
        score += 0.22;
    }
    if str_component(c, "road").is_some() {
        score += 0.30;
    }
    if str_component(c, "city").is_some() {
        score += 0.18;
    }
    if str_component(c, "region").is_some() {
        score += 0.15;
    }
    if str_component(c, "postal_code").is_some() {
        score += 0.15;
    }
    score
}

fn display_line(c: &Map<String, Value>) -> String {
    let mut parts = Vec::new();
    if let (Some(h), Some(r)) = (str_component(c, "house_number"), str_component(c, "road")) {
        parts.push(format!("{h} {r}"));
    } else if let Some(r) = str_component(c, "road") {
        parts.push(r.to_string());
    }
    let mut locality = Vec::new();
    if let Some(city) = str_component(c, "city") {
        locality.push(city.to_string());
    }
    if let Some(region) = str_component(c, "region") {
        locality.push(region.to_string());
    }
    if let Some(postal) = str_component(c, "postal_code") {
        locality.push(postal.to_string());
    }
    if !locality.is_empty() {
        parts.push(locality.join(" "));
    }
    if let Some(country) = str_component(c, "country") {
        parts.push(country.to_string());
    }
    parts.join(", ")
}

fn canonical_line(c: &Map<String, Value>) -> String {
    normalize_address_token(&display_line(c))
}

fn address_tokens(s: &str) -> Vec<String> {
    let mut out: Vec<_> = s
        .split_whitespace()
        .map(normalize_address_token)
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn address_field_embedding_text(field: &str) -> String {
    format!("address field meaning: {}", normalize_field_text(field))
}

fn name_role_score(field: &str, role: &str) -> f32 {
    let norm = normalize_field_text(field);
    let terms: &[&str] = match role {
        "customer" => &["customer", "client", "account", "brand", "company"],
        "address_line1" => &["address", "addr", "street", "line1", "shipaddr", "billaddr"],
        "city" => &["city", "town", "municipality", "locality"],
        "region" => &["state", "province", "region", "territory"],
        "postal_code" => &["zip", "zipcode", "postal", "postcode", "postalish"],
        "country" => &["country", "nation"],
        "address_role" => &["role", "type", "purpose", "billto", "shipto"],
        _ => &[],
    };
    if terms.iter().any(|term| norm.contains(term)) {
        1.0
    } else {
        0.0
    }
}

fn content_role_score(value: &str, role: &str) -> f32 {
    match role {
        "customer" => {
            let lower = value.to_ascii_lowercase();
            (lower.contains("cola")
                || lower.contains("company")
                || lower.contains("distributor")
                || lower.contains("inc")) as u8 as f32
        }
        "address_line1" => {
            let lower = value.to_ascii_lowercase();
            if has_digit(value) && STREET_SUFFIXES.iter().any(|s| lower.contains(s)) {
                1.0
            } else if has_digit(value) {
                0.55
            } else {
                0.0
            }
        }
        "city" => (!has_digit(value) && value.split_whitespace().count() <= 3) as u8 as f32 * 0.65,
        "region" => is_state(value.trim()) as u8 as f32,
        "postal_code" => is_zip(value.trim()) as u8 as f32,
        "country" => matches!(
            value.trim().to_ascii_uppercase().as_str(),
            "US" | "USA" | "UNITED STATES"
        ) as u8 as f32,
        "address_role" => matches!(
            normalize_role(value).as_str(),
            "bill_to" | "ship_to" | "brand_owner"
        ) as u8 as f32,
        _ => 0.0,
    }
}

fn copy_component(out: &mut Map<String, Value>, source: &Map<String, Value>, key: &str) {
    if let Some(value) = source.get(key) {
        out.insert(key.into(), value.clone());
    }
}

fn str_component<'a>(c: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    c.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn scalar_text(v: &Value) -> String {
    v.as_str()
        .map(str::to_string)
        .unwrap_or_else(|| v.to_string().trim_matches('"').to_string())
}

fn normalize_role(value: &str) -> String {
    let n = normalize_field_text(value);
    if n.contains("ship") {
        "ship_to".into()
    } else if n.contains("brand") {
        "brand_owner".into()
    } else {
        "bill_to".into()
    }
}

fn normalize_field_text(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_address_token(s: &str) -> String {
    s.split_whitespace()
        .map(|token| {
            let token = trim_punct(token).to_ascii_lowercase();
            match token.as_str() {
                "street" => "st".to_string(),
                "avenue" => "ave".to_string(),
                "road" => "rd".to_string(),
                "lane" => "ln".to_string(),
                "drive" => "dr".to_string(),
                "boulevard" => "blvd".to_string(),
                "united states" | "usa" => "us".to_string(),
                _ => token,
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_words(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let w = trim_punct(w);
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => format!(
                    "{}{}",
                    first.to_ascii_uppercase(),
                    chars.as_str().to_ascii_lowercase()
                ),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn trim_punct(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_ascii_punctuation() && c != '-')
}

fn starts_uppercase(s: &str) -> bool {
    s.chars().next().is_some_and(char::is_uppercase)
}

fn has_digit(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit())
}

fn is_zip(s: &str) -> bool {
    let s = trim_punct(s);
    s.len() == 5 && s.chars().all(|c| c.is_ascii_digit())
}

fn is_state(s: &str) -> bool {
    US_STATES.contains(&s.to_ascii_uppercase().as_str())
}

fn is_street_suffix(s: &str) -> bool {
    STREET_SUFFIXES.contains(&s.to_ascii_lowercase().as_str())
}

const ADDRESS_ROLES: &[&str] = &[
    "customer",
    "address_line1",
    "city",
    "region",
    "postal_code",
    "country",
    "address_role",
];

const STREET_SUFFIXES: &[&str] = &[
    "st",
    "street",
    "ave",
    "avenue",
    "rd",
    "road",
    "ln",
    "lane",
    "blvd",
    "boulevard",
    "drive",
    "dr",
    "way",
];

const US_STATES: &[&str] = &[
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA", "HI", "IA", "ID", "IL", "IN", "KS",
    "KY", "LA", "MA", "MD", "ME", "MI", "MN", "MO", "MS", "MT", "NC", "ND", "NE", "NH", "NJ", "NM",
    "NV", "NY", "OH", "OK", "OR", "PA", "RI", "SC", "SD", "TN", "TX", "UT", "VA", "VT", "WA", "WI",
    "WV", "WY",
];

#[must_use]
pub fn address_canonical_key(components: &Map<String, Value>) -> String {
    format!("{:016x}", fnv1a_64(canonical_line(components).as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_scan_emits_embedding_hints_and_components() {
        let out = standardize_structured_address(&json!({
            "customer_name": "Tristate Cola",
            "addr gobblygook": "111 East Cola Lane",
            "municipality": "Springfield",
            "state_province": "IL",
            "postalish": "62701",
            "purpose": "bill to"
        }));
        assert_eq!(out.components["road"], "East Cola Lane");
        assert!(
            out.embedding_hints
                .iter()
                .any(|h| h.field == "$.addr gobblygook")
        );
        assert_eq!(out.canonical, "111 east cola ln springfield il 62701");
    }

    #[test]
    fn native_index_matches_canonicalized_address() {
        let index = AddressIndex::from_records([address_index_record(
            "oa:1",
            json!({
                "NUMBER": "111",
                "STREET": "East Cola Lane",
                "CITY": "Springfield",
                "REGION": "IL",
                "POSTCODE": "62701"
            }),
        )]);
        let out = standardize_structured_with_index(
            &json!({
                "customer_name": "Tristate Cola",
                "weird": "111 East Cola Ln",
                "city": "Springfield",
                "state": "IL",
                "zip": "62701"
            }),
            &index,
            3,
        );
        assert_eq!(out.matches[0].id, "oa:1");
        assert!(out.matches[0].score > 0.80);
    }

    #[test]
    fn unstructured_standardizes_road_city_state_zip_without_house_number() {
        let out = standardize_unstructured_address(
            "anything about Queen Cola. The requested bill-to is Royal Road, Springfield IL 62701.",
        );
        assert_eq!(out.components["road"], "Royal Road");
        assert_eq!(out.components["city"], "Springfield");
        assert_eq!(out.components["region"], "IL");
        assert_eq!(out.components["postal_code"], "62701");
        assert!(out.components.get("house_number").is_none());
        assert_eq!(out.display, "Royal Road, Springfield IL 62701");
    }

    #[test]
    fn cached_embedding_evidence_can_recognize_repeated_field_names() {
        let out = standardize_structured_address_with_embeddings(
            &json!({
                "customer_name": "Tristate Cola",
                "x9": "111 East Cola Lane",
                "municipality": "Springfield",
                "state": "IL",
                "zip": "62701"
            }),
            &[AddressEmbeddingEvidence {
                path: "$.x9".into(),
                role: "address_line1".into(),
                score: 0.98,
            }],
        );
        assert_eq!(out.components["road"], "East Cola Lane");
        assert_eq!(
            out.field_evidence
                .iter()
                .find(|e| e.path == "$.x9" && e.role == "address_line1")
                .and_then(|e| e.embedding_score),
            Some(0.98)
        );
    }
}
