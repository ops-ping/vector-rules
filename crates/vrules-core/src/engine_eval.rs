//! Thin host adapter around rust-rule-engine's canonical GRL and `RustRuleEngine`.

use std::collections::HashMap;
use std::sync::Arc;

use rust_rule_engine::engine::rule::ConditionExpression;
use rust_rule_engine::{
    ConditionGroup, EngineConfig, Facts, GRLParser, GruleExecutionResult, KnowledgeBase, Rule,
    RuleEngineError, RustRuleEngine, Value as RuleValue,
};
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VrulesError {
    #[error(transparent)]
    Engine(#[from] RuleEngineError),
    #[error("facts for `{fact_type}` must be a JSON object")]
    InvalidFact { fact_type: String },
}

pub type Result<T> = std::result::Result<T, VrulesError>;

/// Parsed GRL rules that can build identically configured engine instances.
#[derive(Debug, Clone)]
pub struct Ruleset {
    source: Arc<str>,
    rules: Arc<[Rule]>,
}

impl Ruleset {
    pub fn parse(grl: impl Into<String>) -> Result<Self> {
        let source: Arc<str> = Arc::from(grl.into());
        let rules: Arc<[Rule]> = GRLParser::parse_rules(&source)?.into();
        Ok(Self { source, rules })
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    #[must_use]
    pub fn uses_function(&self, function_name: &str) -> bool {
        self.rules
            .iter()
            .any(|rule| Self::condition_uses_function(&rule.conditions, function_name))
    }

    pub fn build_engine(&self) -> Result<RustRuleEngine> {
        self.build_engine_with(|_| {})
    }

    fn condition_uses_function(condition: &ConditionGroup, function_name: &str) -> bool {
        match condition {
            ConditionGroup::Single(condition) => matches!(
                &condition.expression,
                ConditionExpression::FunctionCall { name, .. }
                    | ConditionExpression::Test { name, .. }
                    if name == function_name
            ),
            ConditionGroup::Compound { left, right, .. } => {
                Self::condition_uses_function(left, function_name)
                    || Self::condition_uses_function(right, function_name)
            }
            ConditionGroup::Not(inner)
            | ConditionGroup::Exists(inner)
            | ConditionGroup::Forall(inner) => Self::condition_uses_function(inner, function_name),
            ConditionGroup::Accumulate { .. } | ConditionGroup::StreamPattern { .. } => false,
        }
    }

    pub fn build_engine_with<F>(&self, configure: F) -> Result<RustRuleEngine>
    where
        F: FnOnce(&mut RustRuleEngine),
    {
        let knowledge_base = KnowledgeBase::new("vrules");
        for rule in self.rules.iter().cloned() {
            knowledge_base.add_rule(rule)?;
        }
        let mut engine = RustRuleEngine::with_config(
            knowledge_base,
            EngineConfig {
                max_cycles: 100,
                ..EngineConfig::default()
            },
        );
        configure(&mut engine);
        // Load-time lint: every referenced function must be registered, and
        // usage must respect each function's declared return kind and cost
        // tier (raw scalars can't be thresholded, offline ops can't run live).
        engine.validate_function_usage_strict()?;
        Ok(engine)
    }
}

/// One canonical forward evaluator used by native, component, and WASM adapters.
pub struct RuleEvaluator {
    ruleset: Ruleset,
    engine: RustRuleEngine,
}

impl RuleEvaluator {
    pub fn new(ruleset: Ruleset) -> Result<Self> {
        let engine = ruleset.build_engine()?;
        Ok(Self { ruleset, engine })
    }

    pub fn with_engine(ruleset: Ruleset, engine: RustRuleEngine) -> Self {
        Self { ruleset, engine }
    }

    #[must_use]
    pub fn ruleset(&self) -> &Ruleset {
        &self.ruleset
    }

    pub fn evaluate(&mut self, facts: &Facts, want_trace: bool) -> Result<EvalOutcome> {
        self.engine.reset_no_loop_tracking();
        let mut fired = Vec::new();
        let result = self
            .engine
            .execute_with_callback(facts, |name, _| fired.push(name.to_string()))?;
        Ok(EvalOutcome::from_execution(
            fired, facts, result, want_trace,
        ))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalOutcome {
    pub fired: Vec<String>,
    pub decision: Value,
    pub facts: Value,
    pub trace: Option<Value>,
}

impl EvalOutcome {
    fn from_execution(
        fired: Vec<String>,
        facts: &Facts,
        result: GruleExecutionResult,
        want_trace: bool,
    ) -> Self {
        let all_facts = facts_to_json(facts);
        let decision = all_facts
            .get("Decision")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        let trace = want_trace.then(|| {
            json!({
                "cycles": result.cycle_count,
                "rules_evaluated": result.rules_evaluated,
                "rules_fired": result.rules_fired,
                "execution_time_ns": result.execution_time.as_nanos(),
            })
        });
        Self {
            fired,
            decision,
            facts: all_facts,
            trace,
        }
    }
}

pub fn add_json_fact(facts: &Facts, fact_type: &str, data: &Value) -> Result<()> {
    let object = data
        .as_object()
        .ok_or_else(|| VrulesError::InvalidFact {
            fact_type: fact_type.to_string(),
        })?
        .iter()
        .map(|(key, value)| (key.clone(), json_to_rule_value(value)))
        .collect();
    facts.add_value(fact_type, RuleValue::Object(object))?;
    Ok(())
}

#[must_use]
pub fn facts_to_json(facts: &Facts) -> Value {
    Value::Object(
        facts
            .get_all_facts()
            .into_iter()
            .map(|(key, value)| (key, rule_value_to_json(&value)))
            .collect(),
    )
}

#[must_use]
pub fn json_to_rule_value(value: &Value) -> RuleValue {
    match value {
        Value::Null => RuleValue::Null,
        Value::Bool(value) => RuleValue::Boolean(*value),
        Value::Number(value) => value
            .as_i64()
            .map(RuleValue::Integer)
            .unwrap_or_else(|| RuleValue::Number(value.as_f64().unwrap_or(0.0))),
        Value::String(value) => RuleValue::String(value.clone()),
        Value::Array(values) => {
            RuleValue::Array(values.iter().map(json_to_rule_value).collect::<Vec<_>>())
        }
        Value::Object(values) => RuleValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), json_to_rule_value(value)))
                .collect::<HashMap<_, _>>(),
        ),
    }
}

#[must_use]
pub fn rule_value_to_json(value: &RuleValue) -> Value {
    match value {
        RuleValue::Null => Value::Null,
        RuleValue::Boolean(value) => Value::Bool(*value),
        RuleValue::Integer(value) => Value::Number((*value).into()),
        RuleValue::Number(value) => serde_json::Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        RuleValue::String(value) | RuleValue::Expression(value) => Value::String(value.clone()),
        RuleValue::Array(values) => {
            Value::Array(values.iter().map(rule_value_to_json).collect::<Vec<_>>())
        }
        RuleValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), rule_value_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_grl_and_preserves_nested_facts() {
        let ruleset = Ruleset::parse(
            r#"
            rule "Approve" no-loop {
                when
                    Request.customer.tier == "gold"
                then
                    Decision.route = "priority";
            }
            "#,
        )
        .unwrap();
        let mut evaluator = RuleEvaluator::new(ruleset).unwrap();
        let facts = Facts::new();
        add_json_fact(
            &facts,
            "Request",
            &json!({ "customer": { "tier": "gold" } }),
        )
        .unwrap();
        add_json_fact(&facts, "Decision", &json!({})).unwrap();

        let outcome = evaluator.evaluate(&facts, true).unwrap();
        assert_eq!(outcome.fired, ["Approve"]);
        assert_eq!(outcome.decision["route"], "priority");
        assert_eq!(
            outcome.facts["Request"]["customer"]["tier"],
            Value::String("gold".to_string())
        );
    }

    #[test]
    fn detects_registered_function_requirements_from_parsed_rules() {
        let ruleset = Ruleset::parse(
            r#"
            rule "Vector" {
                when s_cosine(Input.text, "policy") == 0.7
                then Decision.match = true;
            }
            "#,
        )
        .unwrap();

        assert!(ruleset.uses_function("s_cosine"));
        assert!(!ruleset.uses_function("s_contrast"));
    }
}
