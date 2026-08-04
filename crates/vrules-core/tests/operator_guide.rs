//! Executable guidance for the vector operator vocabulary: what each function
//! measures, how to structure its arguments, and the ways each one is misused.
//!
//! These are written as lessons rather than coverage. Every geometric claim is
//! demonstrated on hand-placed vectors in four dimensions, so the numbers are
//! exact and the reason a rule behaves as it does is visible in the fixture
//! rather than buried in a model. Where a lesson was learned the hard way, the
//! test says so.

use std::sync::Arc;

use em_log_n::embed::{Embedder, ModelId};
use rust_rule_engine::Facts;
use serde_json::{Value, json};
use vrules_core::geometry::{ArtifactStore, Axis, Calibration, Provenance, Region};
use vrules_core::{RuleEvaluator, Ruleset, add_json_fact, register_vector_functions};

const DIM: usize = 4;

/// Dimensions are given meanings so the fixtures read as geometry rather than
/// magic numbers: 0 is a shared topic component, 1 is polarity, 2 is a second
/// independent topic, 3 is an unrelated one.
const TABLE: &[(&str, [f32; DIM])] = &[
    // Same direction, different magnitude — separates cosine from dot.
    ("unit", [1.0, 0.0, 0.0, 0.0]),
    ("long", [3.0, 0.0, 0.0, 0.0]),
    // One shared topic, opposite polarity — the shape s_contrast is built for.
    ("praise", [1.0, 1.0, 0.0, 0.0]),
    ("complaint", [1.0, -1.0, 0.0, 0.0]),
    ("neutral_note", [1.0, 0.0, 0.0, 0.0]),
    ("glowing", [1.0, 2.0, 0.0, 0.0]),
    // Off topic entirely.
    ("machine", [0.0, 0.0, 0.0, 1.0]),
    // Calibration populations: barely polarized, and strongly polarized.
    ("mild_a", [1.0, 0.05, 0.0, 0.0]),
    ("mild_b", [1.0, 0.10, 0.0, 0.0]),
    ("mild_c", [1.0, 0.20, 0.0, 0.0]),
    ("mild_d", [1.0, 0.30, 0.0, 0.0]),
    ("strong_a", [0.2, 3.0, 0.0, 0.0]),
    ("strong_b", [0.1, 4.0, 0.0, 0.0]),
    ("strong_c", [0.3, 5.0, 0.0, 0.0]),
    ("strong_d", [0.1, 6.0, 0.0, 0.0]),
    // A tight cloud and a point well outside it.
    ("tight_a", [1.0, 0.00, 0.00, 0.0]),
    ("tight_b", [1.0, 0.05, 0.00, 0.0]),
    ("tight_c", [1.0, 0.00, 0.05, 0.0]),
    ("tight_d", [1.0, -0.05, 0.00, 0.0]),
    ("outsider", [0.0, 1.0, 0.0, 0.0]),
    // A cloud so spread out that "inside it" stops meaning anything.
    ("diffuse_a", [1.0, 0.0, 0.0, 0.0]),
    ("diffuse_b", [0.0, 1.0, 0.0, 0.0]),
    ("diffuse_c", [0.0, 0.0, 1.0, 0.0]),
    ("diffuse_d", [-1.0, 0.0, 0.0, 0.0]),
];

/// Refuses any text it was not given. An embedder that cannot invent a vector
/// is what turns "this argument is a name, not text" from a convention into
/// something a test can prove.
struct TableEmbedder;

impl Embedder for TableEmbedder {
    fn dim(&self) -> usize {
        DIM
    }

    fn model_id(&self) -> ModelId {
        ModelId::from_sha256("operator-guide", &"33".repeat(32), DIM).expect("valid test model id")
    }

    fn embed(&self, text: &str) -> em_log_n::Result<Vec<f32>> {
        TABLE
            .iter()
            .find(|(key, _)| *key == text)
            .map(|(_, v)| v.to_vec())
            .ok_or_else(|| em_log_n::Error::Embed(format!("no fixture vector for {text:?}")))
    }
}

fn provenance() -> Provenance {
    Provenance {
        model: "operator-guide".into(),
        dim: DIM,
        task: None,
        exemplar_set: Some("operator-guide-v1".into()),
    }
}

fn vectors(texts: &[&str]) -> Vec<Vec<f32>> {
    texts
        .iter()
        .map(|t| TableEmbedder.embed(t).expect("fixture text"))
        .collect()
}

/// Evaluate `grl` over one `Concept` fact, returning every resulting fact.
fn eval(grl: &str, target: &str, artifacts: ArtifactStore) -> Result<Value, String> {
    let artifacts = Arc::new(artifacts);
    let embedder: Arc<dyn Embedder> = Arc::new(TableEmbedder);
    let ruleset = Ruleset::parse(grl).map_err(|e| e.to_string())?;
    let mut registration = Ok(());
    let engine = ruleset
        .build_engine_with(|engine| {
            registration = register_vector_functions(engine, Arc::clone(&embedder), artifacts);
        })
        .map_err(|e| e.to_string())?;
    registration.map_err(|e| e.to_string())?;

    let facts = Facts::new();
    add_json_fact(&facts, "Concept", &json!({ "target": target })).map_err(|e| e.to_string())?;
    add_json_fact(&facts, "Decision", &json!({})).map_err(|e| e.to_string())?;
    RuleEvaluator::with_engine(ruleset, engine)
        .evaluate(&facts, false)
        .map(|out| out.facts)
        .map_err(|e| e.to_string())
}

fn number(facts: &Value, path: &str) -> f64 {
    let (fact, field) = path.split_once('.').expect("Type.field");
    facts[fact][field]
        .as_f64()
        .unwrap_or_else(|| panic!("{path} is not a number in {facts}"))
}

/// A rule that writes one measurement into a fact. Measurements belong in
/// `then`; see `raw_scalar_cannot_be_thresholded_in_when` for why.
fn measure_rule(expr: &str) -> String {
    format!(
        r#"rule "Measure" no-loop {{
    when
        Concept.target != ""
    then
        Concept.score = {expr};
}}"#
    )
}

// --- s_cosine / s_dot -------------------------------------------------------

#[test]
fn s_cosine_ignores_magnitude_and_s_dot_does_not() {
    // "unit" and "long" point the same way; only their lengths differ. Reach
    // for s_dot when length carries signal (some models encode confidence or
    // frequency in it) and s_cosine when only direction should count.
    let cosine = eval(
        &measure_rule(r#"s_cosine("unit", "long")"#),
        "unit",
        ArtifactStore::default(),
    )
    .expect("cosine evaluates");
    assert!(
        (number(&cosine, "Concept.score") - 1.0).abs() < 1e-6,
        "parallel vectors are perfectly similar regardless of length"
    );

    let dot = eval(
        &measure_rule(r#"s_dot("unit", "long")"#),
        "unit",
        ArtifactStore::default(),
    )
    .expect("dot evaluates");
    assert!(
        (number(&dot, "Concept.score") - 3.0).abs() < 1e-6,
        "the dot product scales with magnitude: 1 * 3"
    );
}

// --- s_contrast -------------------------------------------------------------

#[test]
fn s_contrast_cancels_the_topic_two_poles_share() {
    // Every fixture here shares topic component 0, so a bare cosine against
    // "praise" scores even a neutral text highly — the shared topic, not the
    // polarity, is doing the work.
    let bare = eval(
        &measure_rule(r#"s_cosine(Concept.target, "praise")"#),
        "neutral_note",
        ArtifactStore::default(),
    )
    .expect("cosine evaluates");
    assert!(
        number(&bare, "Concept.score") > 0.7,
        "shared topic inflates a plain cosine: {}",
        number(&bare, "Concept.score")
    );

    // s_contrast subtracts the similarity to the opposite pole. What both poles
    // have in common cancels, leaving only the axis that separates them.
    let grl = measure_rule(r#"s_contrast(Concept.target, "praise", "complaint")"#);
    let neutral = eval(&grl, "neutral_note", ArtifactStore::default()).expect("contrast evaluates");
    assert!(
        number(&neutral, "Concept.score").abs() < 1e-6,
        "a text equidistant from both poles has no polarity: {}",
        number(&neutral, "Concept.score")
    );

    let polarized = eval(&grl, "glowing", ArtifactStore::default()).expect("contrast evaluates");
    assert!(
        number(&polarized, "Concept.score") > 1.0,
        "a text leaning to one pole keeps its contrast: {}",
        number(&polarized, "Concept.score")
    );
}

// --- axis fitting -----------------------------------------------------------

fn axis_store(name: &str, positive: &[&str], negative: &[&str], window: &[&str]) -> ArtifactStore {
    let mut axis = Axis::from_sets(name, provenance(), &vectors(positive), &vectors(negative))
        .expect("axis fits");
    if !window.is_empty() {
        let scores = vectors(window)
            .iter()
            .map(|v| axis.project_raw(v).expect("projection"))
            .collect();
        axis.calibrate(Calibration::from_scores(scores).expect("calibration window"));
    }
    let mut store = ArtifactStore::default();
    store.insert_axis(axis);
    store
}

#[test]
fn an_axis_measures_whatever_its_negative_set_contrasts_against() {
    // An axis is the direction from the negative centroid to the positive one,
    // so the negatives decide what the axis is *about*. Choose them to share
    // the positives' topic and differ only in the property being isolated.
    let grl = measure_rule(r#"s_project(Concept.target, "polarity")"#);

    // On-topic negatives: both poles are about the same subject, so the axis
    // isolates polarity. A neutral text on that subject scores ~0.
    let on_topic = eval(
        &grl,
        "neutral_note",
        axis_store("polarity", &["praise", "glowing"], &["complaint"], &[]),
    )
    .expect("axis evaluates");
    let on_topic_score = number(&on_topic, "Concept.score");
    assert!(
        on_topic_score.abs() < 0.2,
        "with on-topic negatives a neutral text is neutral: {on_topic_score}"
    );

    // Off-topic negatives: nothing cancels, so the axis mostly points at the
    // topic itself and any on-topic text scores high whatever its polarity.
    //
    // This is not hypothetical. The browser example shipped a "royalty" axis
    // whose negatives were machinery; it ranked "the royal court" above "the
    // emperor", because it was measuring monarchical topic, not royalty.
    let off_topic = eval(
        &grl,
        "neutral_note",
        axis_store("polarity", &["praise", "glowing"], &["machine"], &[]),
    )
    .expect("axis evaluates");
    let off_topic_score = number(&off_topic, "Concept.score");
    assert!(
        off_topic_score > 0.4 && off_topic_score > on_topic_score + 0.4,
        "with off-topic negatives, being on topic is enough to score high: \
         {off_topic_score} against {on_topic_score} for the same text"
    );
}

// --- s_project / c_project --------------------------------------------------

#[test]
fn c_project_is_only_as_portable_as_its_calibration_window() {
    // The raw projection is identical in both cases below — the same text on
    // the same axis. Only the reference population differs.
    let raw = eval(
        &measure_rule(r#"s_project(Concept.target, "polarity")"#),
        "glowing",
        axis_store("polarity", &["praise"], &["complaint"], &[]),
    )
    .expect("raw projection evaluates");
    let raw_score = number(&raw, "Concept.score");

    let grl = measure_rule(r#"c_project(Concept.target, "polarity")"#);
    let against_mild = eval(
        &grl,
        "glowing",
        axis_store(
            "polarity",
            &["praise"],
            &["complaint"],
            &["mild_a", "mild_b", "mild_c", "mild_d"],
        ),
    )
    .expect("calibrated projection evaluates");
    let against_strong = eval(
        &grl,
        "glowing",
        axis_store(
            "polarity",
            &["praise"],
            &["complaint"],
            &["strong_a", "strong_b", "strong_c", "strong_d"],
        ),
    )
    .expect("calibrated projection evaluates");

    assert!(
        (number(&against_mild, "Concept.score") - 100.0).abs() < 1e-6,
        "against a barely polarized population the same text tops the scale"
    );
    assert!(
        number(&against_strong, "Concept.score") < 1e-6,
        "against a strongly polarized population it sits at the bottom: {}",
        number(&against_strong, "Concept.score")
    );

    // So `c_project(...) > 75` is a claim about the calibration window, not
    // about the world. Calibrate on a population that resembles the traffic the
    // rule will actually see, and re-calibrate when that traffic changes —
    // the raw geometry did not move at all between these two runs.
    assert!(raw_score > 0.8, "raw projection unchanged: {raw_score}");
}

#[test]
fn c_project_needs_a_calibrated_axis() {
    // Percentiles cannot be invented from a bare direction, and the engine says
    // so rather than guessing.
    let error = eval(
        &measure_rule(r#"c_project(Concept.target, "polarity")"#),
        "glowing",
        axis_store("polarity", &["praise"], &["complaint"], &[]),
    )
    .expect_err("an uncalibrated axis cannot yield a percentile");
    assert!(
        error.contains("calibration"),
        "error should name the missing calibration window: {error}"
    );
}

// --- s_depth / b_member -----------------------------------------------------

fn region_store(cloud: &[&str], rank: usize, coverage: f32) -> ArtifactStore {
    let region =
        Region::fit("cluster", provenance(), &vectors(cloud), rank, coverage).expect("region fits");
    let mut store = ArtifactStore::default();
    store.insert_region(region);
    store
}

#[test]
fn b_member_answers_membership_and_s_depth_reports_the_distance() {
    // A region is a fitted cloud, not a threshold someone chose: `tau` comes
    // from the coverage requested at fit time, and s_depth reports 1.0 at that
    // boundary. Prefer b_member in `when` — it carries the threshold with the
    // artifact instead of scattering magic numbers through the rules.
    let grl = r#"rule "Classify" no-loop {
    when
        Concept.target != ""
    then
        Concept.depth = s_depth(Concept.target, "cluster");
        Concept.inside = b_member(Concept.target, "cluster");
}"#;
    let store = || region_store(&["tight_a", "tight_b", "tight_c", "tight_d"], 2, 0.95);

    let inside = eval(grl, "tight_a", store()).expect("region evaluates");
    assert!(number(&inside, "Concept.depth") <= 1.0);
    assert_eq!(inside["Concept"]["inside"], json!(true));

    let outside = eval(grl, "outsider", store()).expect("region evaluates");
    assert!(
        number(&outside, "Concept.depth") > 1.0,
        "a point far from the cloud is deeper than the boundary: {}",
        number(&outside, "Concept.depth")
    );
    assert_eq!(outside["Concept"]["inside"], json!(false));
}

#[test]
fn a_diffuse_cloud_makes_membership_meaningless() {
    // Membership is measured in units of the cloud's own spread. Fit a region
    // on exemplars that disagree with each other and the boundary swells until
    // it admits everything — including the very points it was meant to exclude.
    //
    // Learned by measurement: a royalty region fitted from 24 royal terms in
    // 768 dimensions admitted "tractor", "coffee" and "bicycle" at every rank
    // and coverage tried. Always probe a fitted region with known negatives
    // before trusting b_member.
    let grl = r#"rule "Classify" no-loop {
    when
        Concept.target != ""
    then
        Concept.inside = b_member(Concept.target, "cluster");
}"#;
    let diffuse = region_store(
        &["diffuse_a", "diffuse_b", "diffuse_c", "diffuse_d"],
        3,
        0.95,
    );
    let admitted = eval(grl, "outsider", diffuse).expect("region evaluates");
    assert_eq!(
        admitted["Concept"]["inside"],
        json!(true),
        "a cloud with no coherent shape cannot exclude anything"
    );
}

// --- structural rules -------------------------------------------------------

#[test]
fn raw_scalar_cannot_be_thresholded_in_when() {
    // The whole point of the `s_`/`c_` split. A raw score has no portable
    // meaning, so comparing one to a literal is refused at load — measure in
    // `then`, then either threshold a calibrated score or test membership.
    let grl = r#"rule "Gate" no-loop {
    when
        s_cosine(Concept.target, "praise") > 0.9
    then
        Decision.ok = true;
}"#;
    let error = eval(grl, "glowing", ArtifactStore::default())
        .expect_err("thresholding a raw scalar is rejected");
    assert!(
        error.contains("raw scalar"),
        "error should explain the kind mismatch: {error}"
    );
}

#[test]
fn calibrated_scalars_may_be_thresholded_in_when() {
    // The same shape is legal for `c_`, which is what makes the distinction
    // useful rather than merely restrictive.
    let grl = r#"rule "Gate" no-loop {
    when
        c_project(Concept.target, "polarity") > 75
    then
        Decision.ok = true;
}"#;
    let facts = eval(
        grl,
        "glowing",
        axis_store(
            "polarity",
            &["praise"],
            &["complaint"],
            &["mild_a", "mild_b", "mild_c", "mild_d"],
        ),
    )
    .expect("a calibrated comparison loads and runs");
    assert_eq!(facts["Decision"]["ok"], json!(true));
}

#[test]
fn artifact_names_are_looked_up_not_embedded() {
    // `s_project`/`c_project` take an axis name, `s_depth`/`b_member` a region
    // name, and the canon functions a label. None of those are text: the engine
    // resolves them from the artifact store.
    //
    // TableEmbedder fails on any text outside its fixture, so "polarity"
    // reaching the embedder would fail this test. Callers that pre-resolve
    // vectors by scanning rule source must skip these slots for the same
    // reason: the browser example downloaded a 236 MB model to embed the
    // string "royal_register".
    let facts = eval(
        &measure_rule(r#"s_project(Concept.target, "polarity")"#),
        "glowing",
        axis_store("polarity", &["praise"], &["complaint"], &[]),
    )
    .expect("the axis name is resolved from the store, never embedded");
    assert!(number(&facts, "Concept.score") > 0.0);
}

#[test]
fn a_missing_artifact_is_reported_by_name() {
    let error = eval(
        &measure_rule(r#"s_project(Concept.target, "absent_axis")"#),
        "glowing",
        ArtifactStore::default(),
    )
    .expect_err("an unknown axis cannot be projected onto");
    assert!(
        error.contains("absent_axis"),
        "error should name the artifact it could not find: {error}"
    );
}
