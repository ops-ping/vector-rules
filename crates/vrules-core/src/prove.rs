//! Backward-chaining ("prove") over a GRL knowledge base.
//!
//! This is the engine's **goal-directed** mode: given GRL rules (which *derive*
//! conclusions in their `then`) plus a goal, it works backward to decide whether
//! the goal is provable from the supplied facts, returning the proof trace and any
//! missing facts. It is distinct from the forward routing rules (which fire on
//! conditions and don't derive facts) — backward-chaining applies to derivation
//! knowledge. The result types aren't `serde`, so the proof is serialized by hand.

use serde_json::{Value, json};

use rust_rule_engine::backward::query::{ProofStep, ProofTrace};
use rust_rule_engine::backward::{BackwardEngine, GRLQueryExecutor, GRLQueryParser};
use rust_rule_engine::{Facts, GRLParser, KnowledgeBase};

use crate::engine_eval::{Result, json_to_rule_value, rule_value_to_json};

/// Prove `query` (GRL query syntax) against the `grl_rules` knowledge base under
/// `facts` (a JSON object). Returns `{ provable, bindings, missing_facts, proof }`.
pub fn prove(grl_rules: &str, query: &str, facts: &Value) -> Result<Value> {
    let kb = KnowledgeBase::new("vrules-prove");
    for rule in GRLParser::parse_rules(grl_rules)? {
        kb.add_rule(rule)?;
    }
    let q = GRLQueryParser::parse(query)?;

    let mut f = Facts::new();
    if let Some(obj) = facts.as_object() {
        for (k, v) in obj {
            f.set(k, json_to_rule_value(v));
        }
    }

    let mut bc = BackwardEngine::new(kb);
    let result = GRLQueryExecutor::execute(&q, &mut bc, &mut f)?;

    let bindings: serde_json::Map<String, Value> = result
        .bindings
        .iter()
        .map(|(k, v)| (k.clone(), rule_value_to_json(v)))
        .collect();

    Ok(json!({
        "provable": result.provable,
        "bindings": bindings,
        "missing_facts": result.missing_facts,
        "proof": proof_trace_json(&result.proof_trace),
    }))
}

/// Serialize a proof trace (the result types aren't `serde`).
fn proof_trace_json(t: &ProofTrace) -> Value {
    json!({ "goal": t.goal, "steps": t.steps.iter().map(proof_step_json).collect::<Vec<_>>() })
}

fn proof_step_json(s: &ProofStep) -> Value {
    json!({
        "rule": s.rule_name,
        "goal": s.goal,
        "depth": s.depth,
        "sub_steps": s.sub_steps.iter().map(proof_step_json).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn proves_vip_status_from_loyalty_points() {
        let grl = "rule \"VIPRule\" {\n    when\n        User.LoyaltyPoints >= 1000\n    then\n        User.IsVIP = true;\n}\n";
        let query = "query \"CheckVIP\" {\n    goal: User.IsVIP == true\n    strategy: depth-first\n    max-depth: 5\n}\n";
        // Provable when loyalty clears the bar.
        let yes = prove(grl, query, &json!({ "User.LoyaltyPoints": 1200 })).unwrap();
        assert_eq!(yes["provable"], true);
        // Not provable when it doesn't.
        let no = prove(grl, query, &json!({ "User.LoyaltyPoints": 100 })).unwrap();
        assert_eq!(no["provable"], false);
    }

    #[test]
    fn proves_identical_inputs_in_parallel() {
        let grl = "rule \"VIPRule\" {\n    when\n        User.LoyaltyPoints >= 1000\n    then\n        User.IsVIP = true;\n}\n".to_string();
        let query =
            "query \"CheckVIP\" {\n    goal: User.IsVIP == true\n    strategy: depth-first\n    max-depth: 5\n}\n"
                .to_string();

        let mut handles = Vec::new();
        for _ in 0..16 {
            let grl = grl.clone();
            let query = query.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..30 {
                    let out = prove(&grl, &query, &json!({ "User.LoyaltyPoints": 1200 })).unwrap();
                    assert_eq!(out["provable"], true);
                    assert!(out.get("proof").is_some());
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn proves_mixed_inputs_in_parallel() {
        let grl = "rule \"VIPRule\" {\n    when\n        User.LoyaltyPoints >= 1000\n    then\n        User.IsVIP = true;\n}\n".to_string();
        let query =
            "query \"CheckVIP\" {\n    goal: User.IsVIP == true\n    strategy: depth-first\n    max-depth: 5\n}\n"
                .to_string();

        let mut handles = Vec::new();
        for i in 0..20 {
            let grl = grl.clone();
            let query = query.clone();
            handles.push(thread::spawn(move || {
                let points = if i % 2 == 0 { 1200 } else { 100 };
                let expected = i % 2 == 0;
                let out = prove(&grl, &query, &json!({ "User.LoyaltyPoints": points })).unwrap();
                assert_eq!(out["provable"], expected);
                assert!(out.get("bindings").is_some());
                assert!(out.get("missing_facts").is_some());
                assert!(out.get("proof").is_some());
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }
}
