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
            let left = embed(&*f, arg_str(args, 0, "s_cosine")?)?;
            let right = embed(&*f, arg_str(args, 1, "s_cosine")?)?;
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
            let left = embed(&*f, arg_str(args, 0, "s_dot")?)?;
            let right = embed(&*f, arg_str(args, 1, "s_dot")?)?;
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
            let candidate = embed(&*f, arg_str(args, 0, "s_contrast")?)?;
            let positive = embed(&*f, arg_str(args, 1, "s_contrast")?)?;
            let negative = embed(&*f, arg_str(args, 2, "s_contrast")?)?;
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
            let vector = embed(&*f, arg_str(args, 0, "s_project")?)?;
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
            let vector = embed(&*f, arg_str(args, 0, "c_project")?)?;
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
            let vector = embed(&*f, arg_str(args, 0, "s_depth")?)?;
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
            let vector = embed(&*f, arg_str(args, 0, "b_member")?)?;
            let region = region(&store, arg_str(args, 1, "b_member")?)?;
            Ok(RuleValue::Boolean(
                region.member(&vector).map_err(eval_err)?,
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
