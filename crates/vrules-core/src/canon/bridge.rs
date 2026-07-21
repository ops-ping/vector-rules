//! Canonicalization functions registered directly on rust-rule-engine's GRL evaluator.

use std::sync::Arc;

use rust_rule_engine::types::{FunctionMeta, ReturnKind};
use rust_rule_engine::{
    Facts, Result as RuleResult, RuleEngineError, RustRuleEngine, Value as RuleValue,
};
use vrules_canon::SimHash64;

use super::CanonRouter;

/// Register pure canonicalization functions used by authored GRL.
///
/// Names carry the return-kind prefix (`s_` raw scalar, `m_` metadata); see
/// `vec_bridge` for the scheme. Effects belong in the GRL `then` clause; these
/// functions only compute values.
pub fn register_canon_functions(engine: &mut RustRuleEngine, router: Arc<CanonRouter>) {
    let match_router = Arc::clone(&router);
    engine.register_function_with_meta(
        "s_canon_match",
        FunctionMeta::hot(ReturnKind::RawScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            let text = arg_str(args, 0, "s_canon_match")?;
            let label = arg_str(args, 1, "s_canon_match")?;
            let score = match_router
                .score_for(label, text)
                .map_or(0.0, |(score, _)| score);
            Ok(RuleValue::Number(f64::from(score)))
        },
    );

    let matches_router = Arc::clone(&router);
    engine.register_function_with_meta(
        "b_canon_matches",
        FunctionMeta::hot(ReturnKind::Boolean),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            let text = arg_str(args, 0, "b_canon_matches")?;
            let label = arg_str(args, 1, "b_canon_matches")?;
            Ok(RuleValue::Boolean(
                matches_router.score_for(label, text).is_some(),
            ))
        },
    );

    let label_router = Arc::clone(&router);
    engine.register_function_with_meta(
        "m_canon_label",
        FunctionMeta::hot(ReturnKind::Text),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            let label = label_router
                .classify(arg_str(args, 0, "m_canon_label")?)
                .map(|matched| matched.label)
                .unwrap_or_default();
            Ok(RuleValue::String(label))
        },
    );

    engine.register_function_with_meta(
        "s_canon_near",
        FunctionMeta::hot(ReturnKind::RawScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            let left = vrules_canon::canonicalize(arg_str(args, 0, "s_canon_near")?).canonical;
            let right = vrules_canon::canonicalize(arg_str(args, 1, "s_canon_near")?).canonical;
            let distance = SimHash64::of_text(&left).distance(SimHash64::of_text(&right));
            Ok(RuleValue::Number(1.0 - f64::from(distance) / 64.0))
        },
    );

    engine.register_function_with_meta(
        "m_canon_id",
        FunctionMeta::hot(ReturnKind::Text),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            let id = vrules_canon::canonicalize(arg_str(args, 0, "m_canon_id")?).id;
            Ok(RuleValue::Integer(id as i64))
        },
    );

    engine.register_function_with_meta(
        "m_canonical",
        FunctionMeta::hot(ReturnKind::Text),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            Ok(RuleValue::String(
                vrules_canon::canonicalize(arg_str(args, 0, "m_canonical")?).canonical,
            ))
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
