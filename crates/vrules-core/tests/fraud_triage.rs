//! End-to-end proof of the fraud-triage reference pack: geometry artifacts
//! (axis + calibration + region) fitted offline from exemplars, registered
//! GRL vector functions, layered rules, and reason codes — all against a
//! deterministic keyword embedder so the test needs no model.

use std::sync::Arc;

use em_log_n::embed::{Embedder, ModelId};
use rust_rule_engine::Facts;
use serde_json::json;
use vrules_core::geometry::{ArtifactStore, Axis, Calibration, Provenance, Region};
use vrules_core::{RuleEvaluator, Ruleset, add_json_fact, register_vector_functions};

const TRIAGE_RULES: &str = include_str!("../../../shared-rules/fraud/triage.grl");

const DIM: usize = 8;

/// Deterministic toy embedder: counts keyword-group hits per dimension, with a
/// constant shared-topic component so every text pair has inflated cosine
/// similarity (mimicking real-model anisotropy).
struct KeywordEmbedder;

const GROUPS: [&[&str]; 6] = [
    &["urgent", "immediately", "now", "asap", "today", "deadline"],
    &[
        "confidential",
        "discreet",
        "quiet",
        "consequences",
        "penalty",
    ],
    &["ceo", "director", "executive", "boss"],
    &["wire", "transfer", "payment", "invoice", "account"],
    &["attached", "monthly", "usual", "regular", "schedule"],
    &["thanks", "hello", "hi", "regards"],
];

impl Embedder for KeywordEmbedder {
    fn dim(&self) -> usize {
        DIM
    }

    fn embed(&self, text: &str) -> em_log_n::Result<Vec<f32>> {
        let lower = text.to_lowercase();
        let mut vector = vec![0.0f32; DIM];
        vector[0] = 1.0;
        for (slot, keywords) in GROUPS.iter().enumerate() {
            vector[1 + slot] = keywords
                .iter()
                .map(|k| lower.matches(k).count() as f32)
                .sum();
        }
        // Stable per-text jitter so distinct texts never collide exactly.
        let hash = lower
            .bytes()
            .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(b)));
        vector[7] = (hash % 97) as f32 / 970.0;
        Ok(vector)
    }

    fn model_id(&self) -> ModelId {
        ModelId::from_sha256("test-kw", &"22".repeat(32), DIM).expect("valid test model id")
    }
}

const URGENT_EXEMPLARS: [&str; 6] = [
    "urgent wire transfer needed immediately or we face penalty",
    "the ceo needs this payment today, keep it confidential",
    "act now, deadline is today, wire the account immediately",
    "executive request: transfer funds asap and be discreet",
    "immediately process this urgent payment, consequences otherwise",
    "boss says wire now, strictly confidential, deadline today",
];

const CALM_EXEMPLARS: [&str; 6] = [
    "attached is the usual monthly invoice, thanks",
    "regular payment schedule attached, regards",
    "hello, the monthly account statement is attached",
    "hi, invoice attached per the usual schedule, thanks",
    "monthly transfer per the regular schedule, regards",
    "thanks, attached the invoice as usual",
];

const NEUTRAL_CORPUS: [&str; 8] = [
    "hello, following up on the quarterly report",
    "the meeting is scheduled for next week, regards",
    "attached the notes from the review, thanks",
    "hi, can you confirm the delivery address",
    "regular maintenance window this weekend",
    "monthly newsletter draft attached",
    "thanks for the update, looks good",
    "invoice received, processing per the usual schedule",
];

fn embed_all(embedder: &dyn Embedder, texts: &[&str]) -> Vec<Vec<f32>> {
    texts
        .iter()
        .map(|t| embedder.embed(t).expect("toy embedder cannot fail"))
        .collect()
}

fn provenance() -> Provenance {
    Provenance {
        model: "test-kw".into(),
        dim: DIM,
        task: None,
        exemplar_set: Some("triage-test-v1".into()),
    }
}

/// Fit the axis + calibration + region exactly the way an offline constructor
/// pipeline would, then serialize/deserialize to prove the JSON path the
/// console uses.
fn build_artifacts(embedder: &dyn Embedder) -> Arc<ArtifactStore> {
    let urgent = embed_all(embedder, &URGENT_EXEMPLARS);
    let calm = embed_all(embedder, &CALM_EXEMPLARS);

    let mut axis = Axis::from_sets("urgency_pressure_v1", provenance(), &urgent, &calm)
        .expect("axis fits from exemplar sets");
    let mut reference: Vec<f32> = embed_all(embedder, &NEUTRAL_CORPUS)
        .iter()
        .chain(calm.iter())
        .map(|v| axis.project_raw(v).expect("projection succeeds"))
        .collect();
    // Two urgent samples in the window keep the top percentiles honest.
    reference.push(axis.project_raw(&urgent[0]).unwrap());
    reference.push(axis.project_raw(&urgent[1]).unwrap());
    axis.calibrate(Calibration::from_scores(reference).expect("calibration window"));

    let region = Region::fit("bec_phrasing_v1", provenance(), &urgent, 3, 0.95)
        .expect("region fits from exemplar cloud");

    let mut store = ArtifactStore::default();
    store.insert_axis(axis);
    store.insert_region(region);

    let json = store.to_json().expect("artifacts serialize");
    Arc::new(ArtifactStore::from_json(&json).expect("artifacts round-trip"))
}

fn evaluator(artifacts: Arc<ArtifactStore>) -> RuleEvaluator {
    let ruleset = Ruleset::parse(TRIAGE_RULES).expect("triage pack parses");
    let mut registration_error = None;
    let engine = ruleset
        .build_engine_with(|engine| {
            if let Err(error) =
                register_vector_functions(engine, Arc::new(KeywordEmbedder), artifacts)
            {
                registration_error = Some(error);
            }
        })
        .expect("triage pack passes load-time validation");
    assert_eq!(registration_error, None);
    RuleEvaluator::with_engine(ruleset, engine)
}

fn triage(text: &str, new_payee: bool, amount: f64) -> vrules_core::EvalOutcome {
    let mut evaluator = evaluator(build_artifacts(&KeywordEmbedder));
    let facts = Facts::new();
    add_json_fact(
        &facts,
        "Payment",
        &json!({ "text": text, "new_payee": new_payee, "amount": amount }),
    )
    .unwrap();
    add_json_fact(
        &facts,
        "Decision",
        &json!({ "action": "approve", "reason": "routine" }),
    )
    .unwrap();
    evaluator.evaluate(&facts, false).unwrap()
}

#[test]
fn urgent_new_payee_large_amount_is_held_with_reason() {
    let out = triage(
        "urgent: the ceo needs a confidential wire transfer immediately, deadline today",
        true,
        25_000.0,
    );
    assert!(
        out.fired
            .contains(&"HoldUrgentPressureNewPayee".to_string()),
        "fired: {:?}",
        out.fired
    );
    assert_eq!(out.decision["action"], "hold");
    let reason = out.decision["reason"].as_str().unwrap();
    assert!(reason.contains("90th percentile"), "reason: {reason}");

    // The evidence layer recorded calibrated + raw geometry onto the fact.
    let urgency = out.facts["Payment"]["urgency_pct"].as_f64().unwrap();
    assert!(urgency >= 90.0, "urgency percentile {urgency}");
    let depth = out.facts["Payment"]["bec_depth"].as_f64().unwrap();
    assert!(depth.is_finite());
}

#[test]
fn known_bec_phrasing_with_new_payee_is_held_even_when_small() {
    // Below the amount threshold, so the urgency rule alone would not hold it;
    // region membership catches the known phrasing shape.
    let out = triage(
        "boss says wire now, strictly confidential, deadline today",
        true,
        900.0,
    );
    assert!(
        out.fired.contains(&"HoldKnownBecPhrasing".to_string()),
        "fired: {:?}",
        out.fired
    );
    assert_eq!(out.decision["action"], "hold");
    assert!(
        out.decision["reason"]
            .as_str()
            .unwrap()
            .contains("BEC phrasing region")
    );
}

#[test]
fn routine_invoice_to_known_payee_stays_approved() {
    let out = triage(
        "hi, attached is the usual monthly invoice, thanks and regards",
        false,
        25_000.0,
    );
    assert_eq!(out.decision["action"], "approve");
    assert_eq!(out.decision["reason"], "routine");
    let urgency = out.facts["Payment"]["urgency_pct"].as_f64().unwrap();
    assert!(urgency < 90.0, "urgency percentile {urgency}");
    let depth = out.facts["Payment"]["bec_depth"].as_f64().unwrap();
    assert!(
        depth > 1.0,
        "routine text should sit outside the region: {depth}"
    );
}

#[test]
fn artifacts_from_wrong_model_are_rejected_at_registration() {
    let artifacts = build_artifacts(&KeywordEmbedder);
    struct OtherModel;
    impl Embedder for OtherModel {
        fn dim(&self) -> usize {
            DIM
        }
        fn embed(&self, _text: &str) -> em_log_n::Result<Vec<f32>> {
            Ok(vec![0.0; DIM])
        }
        fn model_id(&self) -> ModelId {
            ModelId::from_sha256("other-model", &"33".repeat(32), DIM).unwrap()
        }
    }

    let ruleset = Ruleset::parse(TRIAGE_RULES).unwrap();
    let mut registration_error = None;
    let _ = ruleset.build_engine_with(|engine| {
        if let Err(error) = register_vector_functions(engine, Arc::new(OtherModel), artifacts) {
            registration_error = Some(error);
        }
    });
    let error = registration_error.expect("provenance mismatch must be rejected");
    assert!(error.contains("urgency_pressure_v1") || error.contains("bec_phrasing_v1"));
    assert!(error.contains("other-model"));
}
