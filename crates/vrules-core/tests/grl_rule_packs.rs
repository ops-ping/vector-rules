use rust_rule_engine::Facts;
use serde_json::json;
use vrules_core::{RuleEvaluator, Ruleset, add_json_fact};

const PROXY_RULES: &str = concat!(
    include_str!("../../../shared-rules/proxy/classification.grl"),
    "\n",
    include_str!("../../../shared-rules/proxy/exposure.grl"),
    "\n",
    include_str!("../../../shared-rules/proxy/routing.grl"),
    "\n",
    include_str!("../../../shared-rules/proxy/system_metrics.grl"),
);

const ADDRESS_RULES: &str = concat!(
    include_str!("../../../shared-rules/address/selection.grl"),
    "\n",
    include_str!("../../../shared-rules/address/validation.grl"),
);

#[test]
fn shared_rule_packs_are_canonical_grl() {
    assert_eq!(Ruleset::parse(PROXY_RULES).unwrap().rule_count(), 9);
    assert_eq!(Ruleset::parse(ADDRESS_RULES).unwrap().rule_count(), 11);
}

#[test]
fn proxy_actions_write_decisions_through_rust_rule_engine() {
    let mut evaluator = RuleEvaluator::new(Ruleset::parse(PROXY_RULES).unwrap()).unwrap();
    let facts = Facts::new();
    add_json_fact(
        &facts,
        "Request",
        &json!({ "tool": "web_ground", "query_len": 250 }),
    )
    .unwrap();
    add_json_fact(&facts, "Decision", &json!({})).unwrap();

    let outcome = evaluator.evaluate(&facts, false).unwrap();

    assert_eq!(outcome.decision["backend"], "ai.vrules.grounding");
    assert_eq!(outcome.decision["tool"], "Ground");
    assert_eq!(outcome.decision["effort"], "high");
}
