//! Embedding functions registered on rust-rule-engine's GRL evaluator.
//!
//! Function names carry a return-kind prefix — the human-readable projection
//! of the engine's function metadata for the layer where rustc can't reach:
//!
//! - `s_` raw scalar: geometry measurements. Never thresholded in `when`;
//!   assign to a fact in `then` or use a calibrated (`c_`) form.
//! - `c_` calibrated/decision-scale scalar: safe to threshold.
//! - `b_` boolean: `test(...)`, bare, or `==`/`!=`.
//! - `m_` metadata (labels, identifiers): equality and string operators only.
//!
//! The engine enforces the same contract at ruleset load via
//! `validate_function_usage`, so the prefix and the lint can never drift apart.
//!
//! Axes and regions are *named artifacts* ([`crate::geometry::ArtifactStore`])
//! fitted offline; rules reference them by name. Registration validates every
//! artifact against the active embedder's model and dimension.

use std::sync::Arc;

use em_log_n::embed::Embedder;
use ndarray::Array1;
use rust_rule_engine::types::{FunctionMeta, ReturnKind};
use rust_rule_engine::{
    Facts, Result as RuleResult, RuleEngineError, RustRuleEngine, Value as RuleValue,
};

use crate::geometry::ArtifactStore;
use crate::vec_expr::arith::{cosine_sim, dot};

/// Every function name `register_vector_functions` registers. Hosts use this
/// to decide whether a ruleset needs an embedder at all.
pub const VECTOR_FUNCTIONS: &[&str] = &[
    "s_cosine",
    "s_dot",
    "s_contrast",
    "s_project",
    "c_project",
    "s_depth",
    "b_member",
    "v",
];

/// Register the vector functions used by authored GRL.
///
/// - `s_cosine(left, right)` — cosine similarity of two embedded texts.
/// - `s_dot(left, right)` — unnormalized dot product (magnitude carries
///   signal for some models).
/// - `s_contrast(candidate, positive, negative)` — `cos(x, pos) − cos(x, neg)`;
///   the shared-topic component cancels, isolating the polarity.
/// - `s_project(text, axis)` — raw projection onto a named axis artifact.
/// - `c_project(text, axis)` — calibrated percentile projection; the axis
///   artifact must carry a calibration window.
/// - `s_depth(text, region)` — depth in a named region artifact (`1.0` at the
///   fitted coverage boundary; smaller is deeper inside).
/// - `b_member(text, region)` — membership at the region's fitted threshold.
///
/// # Errors
/// Returns `Err` if any artifact in `artifacts` was fitted against a different
/// model or dimension than `embedder` provides.
pub fn register_vector_functions(
    engine: &mut RustRuleEngine,
    embedder: Arc<dyn Embedder>,
    artifacts: Arc<ArtifactStore>,
) -> Result<(), String> {
    let model_id = embedder.model_id();
    artifacts.validate_provenance(&model_id.name, embedder.dim())?;

    let f = Arc::clone(&embedder);
    engine.register_function_with_meta(
        "s_cosine",
        FunctionMeta::hot(ReturnKind::RawScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            require_len(args, 2, "s_cosine")?;
            let left = eval_vec_val(&*f, &args[0])?;
            let right = eval_vec_val(&*f, &args[1])?;
            Ok(RuleValue::Number(f64::from(
                cosine_sim(&left, &right).map_err(eval_err)?,
            )))
        },
    );

    let f = Arc::clone(&embedder);
    engine.register_function_with_meta(
        "s_dot",
        FunctionMeta::hot(ReturnKind::RawScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            require_len(args, 2, "s_dot")?;
            let left = eval_vec_val(&*f, &args[0])?;
            let right = eval_vec_val(&*f, &args[1])?;
            Ok(RuleValue::Number(f64::from(
                dot(&left, &right).map_err(eval_err)?,
            )))
        },
    );

    let f = Arc::clone(&embedder);
    engine.register_function_with_meta(
        "s_contrast",
        FunctionMeta::hot(ReturnKind::RawScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            require_len(args, 3, "s_contrast")?;
            let candidate = eval_vec_val(&*f, &args[0])?;
            let positive = eval_vec_val(&*f, &args[1])?;
            let negative = eval_vec_val(&*f, &args[2])?;
            let toward = cosine_sim(&candidate, &positive).map_err(eval_err)?;
            let away = cosine_sim(&candidate, &negative).map_err(eval_err)?;
            Ok(RuleValue::Number(f64::from(toward - away)))
        },
    );

    let f = Arc::clone(&embedder);
    let store = Arc::clone(&artifacts);
    engine.register_function_with_meta(
        "s_project",
        FunctionMeta::hot(ReturnKind::RawScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            require_len(args, 2, "s_project")?;
            let vector = eval_vec_val(&*f, &args[0])?;
            let axis = axis(&store, arg_str(args, 1, "s_project")?)?;
            Ok(RuleValue::Number(f64::from(
                axis.project_raw(&vector).map_err(eval_err)?,
            )))
        },
    );

    let f = Arc::clone(&embedder);
    let store = Arc::clone(&artifacts);
    engine.register_function_with_meta(
        "c_project",
        FunctionMeta::hot(ReturnKind::CalibratedScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            require_len(args, 2, "c_project")?;
            let vector = eval_vec_val(&*f, &args[0])?;
            let axis = axis(&store, arg_str(args, 1, "c_project")?)?;
            Ok(RuleValue::Number(f64::from(
                axis.project_percentile(&vector).map_err(eval_err)?,
            )))
        },
    );

    let f = Arc::clone(&embedder);
    let store = Arc::clone(&artifacts);
    engine.register_function_with_meta(
        "s_depth",
        FunctionMeta::hot(ReturnKind::RawScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            require_len(args, 2, "s_depth")?;
            let vector = eval_vec_val(&*f, &args[0])?;
            let region = region(&store, arg_str(args, 1, "s_depth")?)?;
            Ok(RuleValue::Number(f64::from(
                region.depth(&vector).map_err(eval_err)?,
            )))
        },
    );

    let f = Arc::clone(&embedder);
    let store = Arc::clone(&artifacts);
    engine.register_function_with_meta(
        "b_member",
        FunctionMeta::hot(ReturnKind::Boolean),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            require_len(args, 2, "b_member")?;
            let vector = eval_vec_val(&*f, &args[0])?;
            let region = region(&store, arg_str(args, 1, "b_member")?)?;
            Ok(RuleValue::Boolean(
                region.member(&vector).map_err(eval_err)?,
            ))
        },
    );

    let f = Arc::clone(&embedder);
    engine.register_function_with_meta(
        "v",
        FunctionMeta::hot(ReturnKind::RawScalar),
        move |args: &[RuleValue], _facts: &Facts| -> RuleResult<RuleValue> {
            require_len(args, 1, "v")?;
            let vector = eval_vec_val(&*f, &args[0])?;
            Ok(RuleValue::Array(
                vector
                    .into_iter()
                    .map(|n| RuleValue::Number(f64::from(n)))
                    .collect(),
            ))
        },
    );

    Ok(())
}

fn axis<'a>(store: &'a ArtifactStore, name: &str) -> RuleResult<&'a crate::geometry::Axis> {
    store
        .axis(name)
        .ok_or_else(|| eval_err(format!("unknown axis artifact `{name}`")))
}

fn region<'a>(store: &'a ArtifactStore, name: &str) -> RuleResult<&'a crate::geometry::Region> {
    store
        .region(name)
        .ok_or_else(|| eval_err(format!("unknown region artifact `{name}`")))
}

fn eval_vec_val(embedder: &dyn Embedder, val: &RuleValue) -> RuleResult<Vec<f32>> {
    match val {
        RuleValue::String(text) => embed(embedder, text),
        RuleValue::Expression(text) => embed(embedder, text),
        RuleValue::Array(arr) => {
            if arr.is_empty() {
                return Err(eval_err("vector array cannot be empty"));
            }

            let is_raw_floats = arr
                .iter()
                .all(|item| matches!(item, RuleValue::Number(_) | RuleValue::Integer(_)));

            if is_raw_floats {
                let vec: Vec<f32> = arr
                    .iter()
                    .map(|item| match item {
                        RuleValue::Number(f) => *f as f32,
                        RuleValue::Integer(i) => *i as f32,
                        _ => 0.0,
                    })
                    .collect();
                return Ok(vec);
            }

            let op_str = match &arr[0] {
                RuleValue::String(s) => s.as_str(),
                _ => {
                    return Err(eval_err(format!(
                        "vector operation first element must be an operator string, got {:?}",
                        arr[0]
                    )));
                }
            };

            let op_norm = op_str.trim().to_lowercase();
            match op_norm.as_str() {
                "v:" => {
                    if arr.len() < 2 {
                        return Err(eval_err("v: requires an operand"));
                    }
                    eval_vec_val(embedder, &arr[1])
                }
                "v:add" | "add" => {
                    if arr.len() < 3 {
                        return Err(eval_err("v:add requires at least two operands"));
                    }
                    let mut acc = Array1::from(eval_vec_val(embedder, &arr[1])?);
                    for item in &arr[2..] {
                        let next = Array1::from(eval_vec_val(embedder, item)?);
                        if acc.len() != next.len() {
                            return Err(eval_err(format!(
                                "dimension mismatch in v:add: {} vs {}",
                                acc.len(),
                                next.len()
                            )));
                        }
                        acc = acc + next;
                    }
                    Ok(acc.to_vec())
                }
                "v:sub" | "sub" => {
                    if arr.len() < 3 {
                        return Err(eval_err("v:sub requires at least two operands"));
                    }
                    let mut acc = Array1::from(eval_vec_val(embedder, &arr[1])?);
                    for item in &arr[2..] {
                        let next = Array1::from(eval_vec_val(embedder, item)?);
                        if acc.len() != next.len() {
                            return Err(eval_err(format!(
                                "dimension mismatch in v:sub: {} vs {}",
                                acc.len(),
                                next.len()
                            )));
                        }
                        acc = acc - next;
                    }
                    Ok(acc.to_vec())
                }
                "v:scale" | "scale" | "v:mul" | "mul" => {
                    if arr.len() < 3 {
                        return Err(eval_err("v:scale requires two operands"));
                    }
                    let op1 = &arr[1];
                    let op2 = &arr[2];

                    if let Some(scalar) = extract_scalar(op2) {
                        let vec = Array1::from(eval_vec_val(embedder, op1)?);
                        Ok((vec * scalar).to_vec())
                    } else if let Some(scalar) = extract_scalar(op1) {
                        let vec = Array1::from(eval_vec_val(embedder, op2)?);
                        Ok((vec * scalar).to_vec())
                    } else {
                        let vec1 = Array1::from(eval_vec_val(embedder, op1)?);
                        let vec2 = Array1::from(eval_vec_val(embedder, op2)?);
                        if vec1.len() != vec2.len() {
                            return Err(eval_err(format!(
                                "dimension mismatch in v:mul: {} vs {}",
                                vec1.len(),
                                vec2.len()
                            )));
                        }
                        Ok((vec1 * vec2).to_vec())
                    }
                }
                _ => Err(eval_err(format!("unknown vector operator `{op_str}`"))),
            }
        }
        other => Err(eval_err(format!(
            "expected string or vector array argument, got {other:?}"
        ))),
    }
}

fn extract_scalar(val: &RuleValue) -> Option<f32> {
    match val {
        RuleValue::Number(f) => Some(*f as f32),
        RuleValue::Integer(i) => Some(*i as f32),
        _ => None,
    }
}

// Raw embedding — `s_dot` needs magnitude, and the geometry artifacts
// normalize internally.
fn embed(embedder: &dyn Embedder, text: &str) -> RuleResult<Vec<f32>> {
    let canonical = vrules_canon::canonicalize(text).canonical;
    embedder
        .embed(&canonical)
        .map_err(|error| eval_err(format!("embedding `{canonical}` failed: {error}")))
}

fn require_len(args: &[RuleValue], expected: usize, function: &str) -> RuleResult<()> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(eval_err(format!(
            "{function} requires {expected} arguments, got {}",
            args.len()
        )))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use em_log_n::Error;
    use em_log_n::embed::ModelId;

    struct MockEmbedder;
    impl Embedder for MockEmbedder {
        fn model_id(&self) -> ModelId {
            ModelId::unspecified(3)
        }
        fn dim(&self) -> usize {
            3
        }
        fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
            match text {
                "king" => Ok(vec![10.0, 10.0, 0.0]),
                "man" => Ok(vec![5.0, 0.0, 0.0]),
                "woman" => Ok(vec![0.0, 5.0, 0.0]),
                "queen" => Ok(vec![5.0, 15.0, 0.0]), // king (10,10) - man (5,0) + woman (0,5) = (5,15)
                _ => Ok(vec![1.0, 1.0, 1.0]),
            }
        }
    }

    #[test]
    fn test_vector_lisp_ast_analogy() {
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
        let store = Arc::new(ArtifactStore::default());
        let mut engine = RustRuleEngine::new(rust_rule_engine::KnowledgeBase::new("test"));
        register_vector_functions(&mut engine, Arc::clone(&embedder), Arc::clone(&store)).unwrap();

        let grl = r#"
        rule "Analogy" no-loop {
            when
                s_cosine(["v:add", ["v:sub", "king", "man"], "woman"], "queen") > 0.99
            then
                Result.matched = true;
        }
        "#;

        let rules = rust_rule_engine::GRLParser::parse_rules(grl).unwrap();
        let kb = rust_rule_engine::KnowledgeBase::new("analogy_kb");
        for r in rules {
            kb.add_rule(r).unwrap();
        }

        let mut engine = RustRuleEngine::new(kb);
        register_vector_functions(&mut engine, embedder, store).unwrap();

        let facts = Facts::new();
        let res = engine.execute(&facts).unwrap();
        assert_eq!(res.rules_fired, 1);
    }
}
