#![deny(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use em_log_n::embed::{Embedder, ModelId};
use rust_rule_engine::Facts;
use serde::Deserialize;
use serde_json::{Value, json};
use vrules_core::canon::{CanonKind, CanonRouter, DEFAULT_THRESHOLD, register_canon_functions};
use vrules_core::geometry::ArtifactStore;
use vrules_core::{
    RuleEvaluator, Ruleset, VECTOR_FUNCTIONS, add_json_fact, register_vector_functions,
};

mod repository;

#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "plugin-component",
    });
}

use bindings::ai::vrules::types::{PluginDescriptor, PluginKind};
use bindings::exports::ai::vrules::plugin::Guest;

struct RulesComponent;

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

struct State {
    grl: String,
    tools: Vec<Value>,
    ruleset: Ruleset,
    canon_router: Arc<CanonRouter>,
    repository: Option<repository::RulesRepository>,
}

#[derive(Debug, Deserialize)]
struct Config {
    rules_dir: PathBuf,
    #[serde(default = "default_rule_directories")]
    directories: Vec<String>,
    #[serde(default = "default_tools_file")]
    tools_file: PathBuf,
    #[serde(default)]
    repository_dir: Option<PathBuf>,
    #[serde(default)]
    repository_rules_path: PathBuf,
    #[serde(default)]
    canon_patterns: Vec<CanonPatternConfig>,
}

#[derive(Debug, Deserialize)]
struct CanonPatternConfig {
    label: String,
    #[serde(default)]
    kind: String,
    #[serde(default = "default_canon_threshold")]
    threshold: u32,
    examples: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EvaluateRequest {
    facts: Vec<FactInput>,
    #[serde(default)]
    trace: bool,
    #[serde(default)]
    grl: Option<String>,
    #[serde(default)]
    revision: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FactInput {
    #[serde(rename = "type")]
    fact_type: String,
    #[serde(default)]
    data: Value,
}

#[derive(Debug, Deserialize)]
struct ValidateRequest {
    grl: String,
}

#[derive(Debug, Deserialize)]
struct ProveRequest {
    grl: String,
    query: String,
    #[serde(default)]
    facts: Value,
}

#[derive(Debug, Deserialize)]
struct ExposureRequest {
    context: String,
    tool: String,
    tool_group: String,
}

#[derive(Debug, Deserialize)]
struct RevisionRequest {
    revision: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiffRequest {
    a: String,
    b: String,
}

#[derive(Debug, Deserialize)]
struct PromoteRequest {
    from: String,
    #[serde(default = "default_main_branch")]
    to: String,
    #[serde(default)]
    sign_off: bool,
}

impl Guest for RulesComponent {
    fn initialize(config: String) -> Result<PluginDescriptor, String> {
        let config: Config =
            serde_json::from_str(&config).map_err(|e| format!("invalid rules config: {e}"))?;
        let grl = load_rules(&config)?;
        let tools = load_json_array(&config.rules_dir.join(&config.tools_file))?;
        let ruleset = Ruleset::parse(grl.clone()).map_err(|error| error.to_string())?;
        let canon_router = Arc::new(CanonRouter::new());
        for pattern in &config.canon_patterns {
            canon_router.register(
                pattern.label.clone(),
                CanonKind::parse(&pattern.kind),
                pattern.threshold,
                &pattern.examples,
            );
        }
        let repository = config
            .repository_dir
            .as_deref()
            .map(|directory| {
                repository::RulesRepository::open(
                    directory,
                    config.repository_rules_path,
                    config.directories,
                )
            })
            .transpose()?;
        STATE
            .set(Mutex::new(State {
                grl,
                tools,
                ruleset,
                canon_router,
                repository,
            }))
            .map_err(|_| "rules component is already initialized".to_string())?;
        Ok(descriptor())
    }

    fn invoke(operation: String, payload: String) -> Result<String, String> {
        match operation.as_str() {
            "evaluate" => evaluate(&payload),
            "branches" => repository_call(|repository| repository.branches()),
            "compare" => compare(&payload),
            "diff" => diff(&payload),
            "exposed" => exposed(&payload),
            "head" => repository_call(|repository| repository.head()),
            "list" => list(&payload),
            "promote" => promote(&payload),
            "prove" => prove(&payload),
            "tools" => tools(),
            "validate" => validate(&payload),
            other => Err(format!("unsupported rules operation `{other}`")),
        }
    }
}

fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        id: "rules".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        kind: PluginKind::Rules,
        operations: vec![
            "evaluate".to_string(),
            "branches".to_string(),
            "compare".to_string(),
            "diff".to_string(),
            "exposed".to_string(),
            "head".to_string(),
            "list".to_string(),
            "promote".to_string(),
            "prove".to_string(),
            "tools".to_string(),
            "validate".to_string(),
        ],
    }
}

fn evaluate(payload: &str) -> Result<String, String> {
    let request: EvaluateRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid evaluate request: {e}"))?;
    if request.facts.is_empty() {
        return Err("evaluate requires at least one fact".to_string());
    }
    if request.grl.is_some() && request.revision.is_some() {
        return Err("evaluate accepts either grl or revision, not both".to_string());
    }
    let state = state()?.lock().map_err(|_| "rules lock poisoned")?;
    let temporary;
    let ruleset = if let Some(grl) = request.grl {
        temporary = Ruleset::parse(grl).map_err(|error| error.to_string())?;
        &temporary
    } else if let Some(revision) = request.revision {
        let grl = require_repository(&state)?.load_at(&revision)?;
        temporary = Ruleset::parse(grl).map_err(|error| error.to_string())?;
        &temporary
    } else {
        &state.ruleset
    };
    let outcome = evaluate_ruleset(
        ruleset,
        Arc::clone(&state.canon_router),
        &request.facts,
        request.trace,
    )?;
    serde_json::to_string(&outcome).map_err(|e| format!("encode evaluation: {e}"))
}

fn evaluate_ruleset(
    ruleset: &Ruleset,
    canon_router: Arc<CanonRouter>,
    facts: &[FactInput],
    trace: bool,
) -> Result<Value, String> {
    let embedding = if VECTOR_FUNCTIONS
        .iter()
        .any(|function| ruleset.uses_function(function))
    {
        Some(Arc::new(HostEmbedder::new()?))
    } else {
        None
    };
    let mut registration_error = None;
    let engine = ruleset
        .build_engine_with(|engine| {
            register_canon_functions(engine, Arc::clone(&canon_router));
            if let Some(embedding) = embedding
                && let Err(error) =
                    register_vector_functions(engine, embedding, Arc::new(ArtifactStore::default()))
            {
                registration_error = Some(error);
            }
        })
        .map_err(|error| error.to_string())?;
    if let Some(error) = registration_error {
        return Err(error);
    }
    let working_memory = Facts::new();
    for input in facts {
        add_json_fact(&working_memory, &input.fact_type, &input.data)
            .map_err(|error| error.to_string())?;
    }
    add_json_fact(&working_memory, "Decision", &json!({})).map_err(|error| error.to_string())?;
    let mut evaluator = RuleEvaluator::with_engine(ruleset.clone(), engine);
    serde_json::to_value(
        evaluator
            .evaluate(&working_memory, trace)
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("encode evaluation: {error}"))
}

fn exposed(payload: &str) -> Result<String, String> {
    let request: ExposureRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid exposure request: {e}"))?;
    let state = state()?.lock().map_err(|_| "rules lock poisoned")?;
    let active = rule_mentions_exposure(&state.grl);
    if !active {
        return Ok(json!({ "active": false, "exposed": true }).to_string());
    }
    let facts = [FactInput {
        fact_type: "Exposure".to_string(),
        data: json!({
            "context": request.context,
            "tool": request.tool,
            "tool_group": request.tool_group,
        }),
    }];
    let outcome = evaluate_ruleset(
        &state.ruleset,
        Arc::clone(&state.canon_router),
        &facts,
        false,
    )?;
    let exposed = outcome["fired"]
        .as_array()
        .is_some_and(|fired| !fired.is_empty());
    Ok(json!({ "active": true, "exposed": exposed }).to_string())
}

fn list(payload: &str) -> Result<String, String> {
    let request: RevisionRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid list request: {e}"))?;
    let state = state()?.lock().map_err(|_| "rules lock poisoned")?;
    let grl = if let Some(revision) = request.revision {
        require_repository(&state)?.load_at(&revision)?
    } else {
        state.grl.clone()
    };
    serde_json::to_string(&json!({
        "count": Ruleset::parse(grl.clone()).map_err(|error| error.to_string())?.rule_count(),
        "grl": grl,
    }))
    .map_err(|e| format!("encode rules: {e}"))
}

fn diff(payload: &str) -> Result<String, String> {
    let request: DiffRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid diff request: {e}"))?;
    repository_call(|repository| repository.diff(&request.a, &request.b))
}

fn compare(payload: &str) -> Result<String, String> {
    let request: DiffRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid compare request: {e}"))?;
    repository_call(|repository| repository.compare(&request.a, &request.b))
}

fn promote(payload: &str) -> Result<String, String> {
    let request: PromoteRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid promote request: {e}"))?;
    if !request.sign_off {
        return Err("rules promotion requires sign_off=true".to_string());
    }
    repository_call(|repository| repository.promote(&request.from, &request.to))
}

fn repository_call(
    call: impl FnOnce(&repository::RulesRepository) -> Result<Value, String>,
) -> Result<String, String> {
    let state = state()?.lock().map_err(|_| "rules lock poisoned")?;
    let result = call(require_repository(&state)?)?;
    serde_json::to_string(&result).map_err(|e| format!("encode repository result: {e}"))
}

fn require_repository(state: &State) -> Result<&repository::RulesRepository, String> {
    state
        .repository
        .as_ref()
        .ok_or_else(|| "rules repository is not configured".to_string())
}

fn tools() -> Result<String, String> {
    let state = state()?.lock().map_err(|_| "rules lock poisoned")?;
    serde_json::to_string(&json!({ "tools": state.tools }))
        .map_err(|e| format!("encode tools: {e}"))
}

fn validate(payload: &str) -> Result<String, String> {
    let request: ValidateRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid validate request: {e}"))?;
    match Ruleset::parse(request.grl) {
        Ok(_) => Ok(json!({ "ok": true, "errors": [] }).to_string()),
        Err(error) => Ok(json!({
            "ok": false,
            "errors": [{ "path": "", "message": error.to_string() }],
        })
        .to_string()),
    }
}

fn prove(payload: &str) -> Result<String, String> {
    let request: ProveRequest =
        serde_json::from_str(payload).map_err(|e| format!("invalid prove request: {e}"))?;
    let result = vrules_core::prove(&request.grl, &request.query, &request.facts)
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&result).map_err(|e| format!("encode proof: {e}"))
}

fn state() -> Result<&'static Mutex<State>, String> {
    STATE
        .get()
        .ok_or_else(|| "rules component is not initialized".to_string())
}

fn load_rules(config: &Config) -> Result<String, String> {
    let mut rules = String::new();
    for directory in &config.directories {
        let path = config.rules_dir.join(directory);
        let mut files = fs::read_dir(&path)
            .map_err(|e| format!("read rules directory {}: {e}", path.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "grl"))
            .collect::<Vec<_>>();
        files.sort();
        for file in files {
            let source =
                fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
            rules.push_str(&source);
            rules.push('\n');
        }
    }
    Ok(rules)
}

fn load_json_array(path: &Path) -> Result<Vec<Value>, String> {
    match load_json(path)? {
        Value::Array(values) => Ok(values),
        _ => Err(format!("{} is not a JSON array", path.display())),
    }
}

fn load_json(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))
}

fn rule_mentions_exposure(grl: &str) -> bool {
    grl.contains("Exposure.")
}

fn default_rule_directories() -> Vec<String> {
    vec!["proxy".to_string(), "shared".to_string()]
}

fn default_tools_file() -> PathBuf {
    PathBuf::from("proxy/tools.json")
}

fn default_main_branch() -> String {
    "main".to_string()
}

fn default_canon_threshold() -> u32 {
    DEFAULT_THRESHOLD
}

struct HostEmbedder {
    model_id: ModelId,
}

impl HostEmbedder {
    fn new() -> Result<Self, String> {
        let info = bindings::ai::vrules::host::get_embedding_info()?;
        Ok(Self {
            model_id: ModelId::from_sha256(info.model, &info.revision, info.dimensions as usize)
                .map_err(|error| error.to_string())?,
        })
    }
}

impl Embedder for HostEmbedder {
    fn dim(&self) -> usize {
        self.model_id.dim
    }

    fn model_id(&self) -> ModelId {
        self.model_id.clone()
    }

    fn embed(&self, text: &str) -> em_log_n::Result<Vec<f32>> {
        bindings::ai::vrules::host::embed(text)
            .map_err(|message| em_log_n::Error::Embed(message.to_string()))
    }
}

#[allow(unsafe_code)]
mod component_export {
    use super::RulesComponent;
    use crate::bindings;

    crate::bindings::export!(RulesComponent with_types_in bindings);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_grl_does_not_require_embedding_host() {
        let ruleset = Ruleset::parse(
            r#"
            rule "Route" no-loop {
                when Request.tool == "web_ground"
                then Decision.backend = "grounding";
            }
            "#,
        )
        .unwrap();
        let outcome = evaluate_ruleset(
            &ruleset,
            Arc::new(CanonRouter::new()),
            &[FactInput {
                fact_type: "Request".to_string(),
                data: json!({ "tool": "web_ground" }),
            }],
            true,
        )
        .unwrap();

        assert_eq!(outcome["decision"]["backend"], "grounding");
        assert_eq!(outcome["fired"], json!(["Route"]));
    }
}
