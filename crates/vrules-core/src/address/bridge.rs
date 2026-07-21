//! Address-analysis functions registered on rust-rule-engine's GRL evaluator.

use std::sync::Arc;

use rust_rule_engine::types::{FunctionMeta, ReturnKind};
use rust_rule_engine::{
    Facts, Result as RuleResult, RuleEngineError, RustRuleEngine, Value as RuleValue,
};

use super::AddressAnalyzer;

/// Register pure address-analysis functions backed by a host-supplied analyzer.
///
/// Effects belong in GRL actions; functions only return analysis values.
pub fn register_address_functions(engine: &mut RustRuleEngine, analyzer: Arc<dyn AddressAnalyzer>) {
    let confidence_analyzer = Arc::clone(&analyzer);
    engine.register_function_with_meta(
        "c_addr_confidence",
        FunctionMeta::hot(ReturnKind::CalibratedScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            let analysis = confidence_analyzer
                .analyze(arg_str(args, 0, "c_addr_confidence")?)
                .map_err(|error| eval_err(format!("c_addr_confidence: {error}")))?;
            Ok(RuleValue::Number(f64::from(analysis.confidence)))
        },
    );

    let standardize_analyzer = Arc::clone(&analyzer);
    engine.register_function_with_meta(
        "m_addr_standardize",
        FunctionMeta::hot(ReturnKind::Text),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            let analysis = standardize_analyzer
                .analyze(arg_str(args, 0, "m_addr_standardize")?)
                .map_err(|error| eval_err(format!("m_addr_standardize: {error}")))?;
            Ok(RuleValue::String(analysis.standardized))
        },
    );

    let component_analyzer = Arc::clone(&analyzer);
    engine.register_function_with_meta(
        "m_addr_component",
        FunctionMeta::hot(ReturnKind::Text),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            let analysis = component_analyzer
                .analyze(arg_str(args, 0, "m_addr_component")?)
                .map_err(|error| eval_err(format!("m_addr_component: {error}")))?;
            Ok(RuleValue::String(
                analysis
                    .component(arg_str(args, 1, "m_addr_component")?)
                    .unwrap_or_default()
                    .to_string(),
            ))
        },
    );

    let has_component_analyzer = Arc::clone(&analyzer);
    engine.register_function_with_meta(
        "b_addr_has_component",
        FunctionMeta::hot(ReturnKind::Boolean),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            let analysis = has_component_analyzer
                .analyze(arg_str(args, 0, "b_addr_has_component")?)
                .map_err(|error| eval_err(format!("b_addr_has_component: {error}")))?;
            Ok(RuleValue::Boolean(
                analysis
                    .component(arg_str(args, 1, "b_addr_has_component")?)
                    .is_some(),
            ))
        },
    );

    engine.register_action_handler(
        "standardize_address",
        move |params, facts: &Facts| -> RuleResult<()> {
            let text = param_str(params, "0", "standardize_address")?;
            let target = param_str(params, "1", "standardize_address")?;
            let standardized = analyzer
                .analyze(text)
                .map_err(|error| eval_err(format!("standardize_address: {error}")))?
                .standardized;
            facts.set_nested(target, RuleValue::String(standardized))
        },
    );
}

fn arg_str<'a>(args: &'a [RuleValue], index: usize, function: &str) -> RuleResult<&'a str> {
    match args.get(index) {
        Some(RuleValue::String(value)) => Ok(value),
        Some(value) => Err(eval_err(format!(
            "{function} argument {index} must be a string, got {value:?}"
        ))),
        None => Err(eval_err(format!("{function} argument {index} is missing"))),
    }
}

fn eval_err(message: impl Into<String>) -> RuleEngineError {
    RuleEngineError::EvaluationError {
        message: message.into(),
    }
}

fn param_str<'a>(
    params: &'a std::collections::HashMap<String, RuleValue>,
    key: &str,
    action: &str,
) -> RuleResult<&'a str> {
    match params.get(key) {
        Some(RuleValue::String(value)) => Ok(value),
        Some(value) => Err(eval_err(format!(
            "{action} parameter {key} must be a string, got {value:?}"
        ))),
        None => Err(eval_err(format!("{action} parameter {key} is missing"))),
    }
}
