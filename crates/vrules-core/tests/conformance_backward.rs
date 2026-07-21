//! Backward-chaining conformance suite for `vrules_core::prove`.
//!
//! This proves that `vrules_core::prove` (GRL knowledge base + GRL query → goal-directed
//! proof) exposes the engine's backward-chaining functionality: provability,
//! genuine multi-rule chaining, missing-fact detection, AND/OR conditions and goals,
//! candidate-rule selection by conclusion field, search strategies, depth limiting,
//! memoization, function (builtin) conditions, and the proof trace.
//!
//! Every test asserts the REAL result returned by `vrules_core::prove`. Where the fork's
//! own assertion is deliberately weak (the backward engine is optimistic about
//! candidate rules), this suite mirrors that strength rather than "improving" it.
//!
//! NO `on` clause is used anywhere — fact-type targeting is via `Type.field` paths.
//!
//! ## Empirically established facts about the `vrules_core::prove` backward path
//! - The proof trace is FLAT: rules that directly conclude the goal appear as
//!   depth-0 steps with empty `sub_steps`. Recursive chaining happens inside the
//!   engine (`try_prove_rule_conditions` executes intermediate rules and mutates
//!   facts) but is NOT surfaced as nested `sub_steps`. Chaining is nonetheless
//!   genuine: breaking the chain flips provability (see `chain_three_rules`).
//! - Candidate enumeration can over-include (a substring fallback in
//!   `rule_could_prove_goal`), so multiple rules concluding the goal field all show
//!   as steps even if only one matches the facts. Tests assert the concluding rule
//!   is PRESENT and steps are flat — never an exact step count.
//! - `bindings` is always `{}` on this path: GRL goals carry no logic variables.
//! - Compound `&&`/`||` goals return `steps: []` and `bindings: {}` even when provable.
//! - Embedding-gated (`s_cosine`) conditions are not available on the backward path
//!   (see `embedding_gating_unavailable_on_backward_path` for the evidence chain).
//!   Builtin function conditions (`len`, ...) DO work (`builtin_len_function_condition`).
//!
//! ## Ledger — every fork test → its vrules shadow (or a justified gap)
//!
//! ### rust-rule-engine/tests/backward_comprehensive_tests.rs (44)
//! mod expression_parser (21):
//!   parse_simple_field, parse_boolean_literal, parse_number_literal,
//!   parse_string_literal           → literal_values_as_conditions
//!   parse_comparison, evaluate_comparison_true/false → operator_equal_number
//!   parse_logical_and, evaluate_logical_and          → condition_logical_and
//!   parse_logical_or, evaluate_logical_or            → condition_logical_or
//!   parse_negation                                   → condition_negation
//!   extract_fields_single, extract_fields_multiple   → condition_two_fields_both_required
//!   is_satisfied_true, is_satisfied_false            → operator_equal_number
//!   parse_greater_than/less_than/greater_or_equal/less_or_equal → operator_relational
//!   parse_not_equal                                  → operator_not_equal_goal
//!     (GAP: AST-shape introspection — Expression::Field/Literal variants — is internal;
//!      reproduced behaviorally via the operators/literals above.)
//! mod conclusion_index (10):
//!   new_index_empty, is_empty, clear_index           → empty_kb_unprovable
//!   add_single_rule, find_candidates_single          → candidate_single_rule
//!   find_candidates_multiple_matches, from_rules_creates_index → candidate_multiple_rules_same_field
//!   index_multiple_fields                            → candidate_multiple_fields_one_rule
//!   remove_rule                                      → candidate_different_field_irrelevant
//!   performance_o1_lookup                            → (GAP: timing/O(1) internal — not observable)
//! mod unification (8):
//!   bind_variable,is_bound,merge_bindings,conflicting_bindings,clear_bindings,
//!   from_map,to_map,len                              → bindings_empty_on_field_goal
//!     (GAP: Bindings is an internal struct; GRL goals have no logic variables, so
//!      `bindings` always returns `{}`. The one behavioral fact — prove returns a
//!      bindings map — is asserted.)
//! mod multiple_solutions (5):
//!   single_rule                                      → prove_vip_from_loyalty
//!   multiple_paths                                   → condition_logical_or
//!   max_solutions_limit                              → (GAP: max_solutions has no GRL-query
//!      field; provability shown in prove_vip_from_loyalty / multiple paths)
//!   different_strategies                             → strategies_all_prove_single_rule
//!   complex_chain                                    → chain_three_rules
//!
//! ### rust-rule-engine/tests/backward_tms_integration.rs (7)
//!   derives_logical_fact_and_cascade_retracts        → chain_two_levels (chaining half;
//!      GAP: TMS cascade-retraction needs a stateful retract API prove does not expose)
//!   complex_multi_level_reasoning                    → chain_four_levels
//!   with_multiple_or_conditions                      → condition_logical_or
//!   missing_facts_detection                          → missing_facts_contains_goal
//!   with_numeric_comparisons                         → operator_relational
//!   proof_trace_generation                           → proof_trace_single_rule_shape
//!   with_multiple_solution_paths                     → candidate_multiple_rules_same_field
//!
//! ### rust-rule-engine/tests/proof_graph_integration_test.rs (6)
//!   caching_basic                                    → memoization_repeat_query
//!   multiple_justifications                          → candidate_multiple_rules_same_field
//!   dependency_propagation                           → chain_three_rules
//!   invalidation, cache_statistics, fact_key_parsing → (GAP: ProofGraph TMS internals
//!      — invalidation counts, cache hit/miss stats, FactKey parsing — are stateful and
//!      not surfaced by the stateless `prove`.)
//!
//! ### src/backward/aggregation.rs::tests (13)
//!   parse_count/sum/avg_with_filter/min/max, parse_invalid, parse_unknown_function,
//!   apply_count/sum/avg/min/max, apply_empty_solutions
//!     → (GAP: aggregate queries use a different grammar — `count(?x) WHERE ...` — that
//!      `GRLQueryParser`/`vrules_core::prove` does not route. Not expressible through prove.)
//!
//! ### src/backward/disjunction.rs::tests (16)
//!   parser_simple_or                                 → query_or_goal
//!   parser_triple_or                                 → query_multiple_or_branches
//!   parser_nested_parens, parser_nested_or_groups, parser_deeply_nested → query_nested_parentheses
//!   disjunction_creation, add_branch, result_success → condition_logical_or
//!   result_empty, parser_no_or, parser_contains_or, parser_function_args_with_or_keyword,
//!   contains_or_nested, split_top_level_or_basic, split_top_level_or_with_quotes,
//!   deduplication                                    → (GAP: internal Disjunction parser
//!      helpers; OR behavior reproduced via query_or_goal / condition_logical_or.)
//!
//! ### src/backward/nested.rs::tests (9)
//!   query_creation, add_goal, variables, nested_query_creation, shared_variables,
//!   result, parser_has_nested, stats, parser_simple_query
//!     → chain_three_rules (nested derivation = a goal proved through an intermediate
//!       derived fact). GAP: the datalog `parent(?x,?y) WHERE ...` Query/NestedQuery
//!       structs are internal and not routed by prove.
//!
//! ### src/backward/optimizer.rs::tests (10)
//!   creation,with_config,memoization_hit_rate        → memoization_disabled_still_proves
//!   goal_reordering, disable_reordering              → rule_order_independent
//!   selectivity_estimation, count_variables, optimization_stats, join_optimizer,
//!   stats_summary                                    → (GAP: QueryOptimizer internals for
//!      multi-goal datalog queries; not surfaced by prove.)
//!
//! ### src/backward/goal.rs::tests (15)
//!   creation, status_checks, is_proven               → prove_vip_from_loyalty
//!   subgoal_management, subgoals_not_all_proven      → chain_three_rules
//!   goal_manager, goal_manager_next_pending, goal_manager_clear, goal_manager_default,
//!   cache_result                                     → memoization_repeat_query
//!   candidate_rules                                  → candidate_multiple_rules_same_field
//!   goal_depth, is_too_deep                          → depth_limit_blocks_chain
//!   negated_goal, negated_goal_with_expression, normal_goal_not_negated → condition_negation
//!   goal_with_expression, goal_bindings              → bindings_empty_on_field_goal
//!
//! ### src/backward/search.rs::tests (15)
//!   search_strategies, search_strategy_equality, depth_first_search_creation,
//!   depth_first_search_simple, breadth_first_search, iterative_deepening_search_success
//!                                                    → strategies_all_prove_single_rule
//!   breadth_first_search_multiple_candidates         → candidate_multiple_rules_same_field
//!   depth_first_search_max_depth_exceeded, iterative_deepening_search_depth_limit
//!                                                    → depth_limit_blocks_chain
//!   depth_first_search_empty_goal, iterative_deepening_search_no_candidates
//!                                                    → no_candidate_goal_unprovable
//!   search_result_creation, search_result_with_bindings,
//!   breadth_first_search_with_subgoals, depth_first_search_goals_explored_count
//!                                                    → (GAP: SearchResult struct + goals_explored
//!      counters are internal; prove's JSON exposes no stats block.)
//!
//! ### src/backward/explanation.rs::tests (6)
//!   build_proof_tree, goal_proven_by_fact            → proof_trace_single_rule_shape
//!   builder_creation, enable_disable, tracking_goal, explanation_step
//!                                                    → (GAP: ExplanationBuilder is an internal
//!      accumulator; the resulting trace is asserted via proof_trace_single_rule_shape.)
//!
//! ### src/backward/conclusion_index.rs::tests (9)
//!   index_creation                                   → empty_kb_unprovable
//!   add_single_rule, find_candidates_exact_match     → candidate_single_rule
//!   find_candidates_multiple_rules                   → candidate_multiple_rules_same_field
//!   from_rules_bulk_creation                         → candidate_different_field_irrelevant
//!   extract_field_from_goal                          → goal_field_no_rule_unprovable
//!   remove_rule, stats                               → candidate_different_field_irrelevant
//!   disabled_rules_not_indexed                       → (GAP: GRL has no per-rule "disabled"
//!      flag reachable through prove.)
//!
//! ### src/backward/proof_graph.rs::tests (6)
//!   fact_key_from_pattern, insert_and_lookup         → memoization_repeat_query
//!   dependency_tracking                              → chain_three_rules
//!   multiple_justifications                          → candidate_multiple_rules_same_field
//!   cache_statistics, clear                          → (GAP: ProofGraph TMS internals; stateless prove.)
//!
//! ### src/backward/grl_query.rs::tests (17)
//!   parse_simple_query, query_config_conversion      → query_default_strategy
//!   parse_query_with_strategy                        → strategies_all_prove_single_rule
//!   parse_query_with_actions, action_execution       → query_on_success_action
//!   parse_query_with_when_condition, should_execute_no_condition,
//!   should_execute_condition_true, should_execute_condition_false → query_when_gate
//!   should_execute_parse_error_propagates            → query_when_parse_error
//!   parse_multiple_queries                           → two_separate_queries
//!   parse_query_with_or_goal                         → query_or_goal
//!   parse_query_with_complex_goal                    → query_complex_goal_and_or
//!   parse_query_with_multiple_or_branches            → query_multiple_or_branches
//!   parse_query_with_parentheses                     → query_parentheses
//!   parse_query_with_nested_parentheses              → query_nested_parentheses
//!   parse_query_unclosed_parenthesis                 → query_unclosed_paren_errors
//!
//! ### src/backward/unification.rs::tests (10)
//!   bindings_basic, bindings_conflict, bindings_merge, bindings_merge_conflict,
//!   unify_variable_with_literal, unify_bound_variable, unify_two_literals,
//!   match_expression_simple, evaluate_with_bindings, compare_values
//!     → bindings_empty_on_field_goal (see unification GAP above). compare_values /
//!       match_expression behavior is reproduced via operator_relational.
//!
//! ### src/backward/proof_tree.rs::tests (10)
//!   proof_node_creation, fact_node, rule_node        → proof_trace_single_rule_shape
//!   add_child, tree_height, node_count               → (GAP: the prove trace is flat;
//!      nested height/node_count is not surfaced — documented at top of file.)
//!   proof_tree_creation                              → proof_trace_single_rule_shape
//!   json_serialization, markdown_generation, html_generation
//!                                                    → (GAP: trace rendering is internal.)

use serde_json::{Value, json};
use vrules_core::prove;

// ---- helpers -------------------------------------------------------------

/// Run `vrules_core::prove` and unwrap the JSON result.
fn prove_ok(grl: &str, query: &str, facts: Value) -> Value {
    prove(grl, query, &facts).unwrap()
}

/// Build a single-goal query (goal on its own line — single-line pollutes the goal).
fn q_goal(goal: &str) -> String {
    format!("query \"Q\" {{\n    goal: {goal}\n}}\n")
}

/// Build a query with extra option lines after the goal.
fn q_goal_opts(goal: &str, opts: &str) -> String {
    format!("query \"Q\" {{\n    goal: {goal}\n{opts}\n}}\n")
}

fn provable(v: &Value) -> bool {
    v["provable"].as_bool().unwrap()
}

/// Rule names of the top-level proof steps.
fn step_rules(v: &Value) -> Vec<String> {
    v["proof"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["rule"].as_str().unwrap().to_string())
        .collect()
}

/// Assert every top-level step is flat (no nested sub_steps) and at depth 0.
fn assert_flat_steps(v: &Value) {
    for s in v["proof"]["steps"].as_array().unwrap() {
        assert_eq!(s["depth"], 0, "steps are flat (depth 0) on this path");
        assert!(
            s["sub_steps"].as_array().unwrap().is_empty(),
            "the prove trace exposes no nested sub_steps"
        );
    }
}

// ======================================================================
// A. Provability & genuine multi-rule chaining
// ======================================================================

/// fork: multiple_solutions_single_rule, goal::creation/status_checks/is_proven,
/// prove.rs::proves_vip_status_from_loyalty_points.
#[test]
fn prove_vip_from_loyalty() {
    let grl = r#"rule "VIPRule" { when User.LoyaltyPoints >= 1000 then User.IsVIP = true; }"#;
    let q = q_goal("User.IsVIP == true");

    let yes = prove_ok(grl, &q, json!({ "User.LoyaltyPoints": 1200 }));
    assert!(provable(&yes));
    assert_eq!(yes["proof"]["goal"], "User.IsVIP == true");
    assert_eq!(step_rules(&yes), vec!["VIPRule"]);

    // Below the threshold the goal is not provable.
    let no = prove_ok(grl, &q, json!({ "User.LoyaltyPoints": 100 }));
    assert!(!provable(&no));
}

/// fork: multiple_solutions_complex_chain, nested::*, proof_graph::dependency_*,
/// goal::subgoal_management. Genuine A->B->C chaining: provability depends on the
/// FULL chain even though the proof trace shows only the concluding rule, flatly.
#[test]
fn chain_three_rules() {
    let grl = r#"
rule "AtoB" { when A.flag == true then B.flag = true; }
rule "BtoC" { when B.flag == true then C.flag = true; }
"#;
    let q = q_goal_opts("C.flag == true", "    max-depth: 10");

    // Full chain present + base fact: provable. Concluding rule is BtoC, flat trace.
    let ok = prove_ok(grl, &q, json!({ "A.flag": true }));
    assert!(provable(&ok));
    assert_eq!(ok["proof"]["goal"], "C.flag == true");
    assert!(step_rules(&ok).contains(&"BtoC".to_string()));
    assert_flat_steps(&ok);

    // Base fact ABSENT (not set false): chain cannot start → not provable.
    let no_base = prove_ok(grl, &q, json!({}));
    assert!(!provable(&no_base));

    // Intermediate rule removed: B.flag can never be derived → not provable.
    let only_btoc = r#"rule "BtoC" { when B.flag == true then C.flag = true; }"#;
    let no_mid = prove_ok(only_btoc, &q, json!({ "A.flag": true }));
    assert!(!provable(&no_mid));
}

/// fork: backward_complex_multi_level_reasoning (4 levels).
#[test]
fn chain_four_levels() {
    let grl = r#"
rule "HasPointsRule" { when User.Points > 100 then User.HasPoints = true; }
rule "EligibleRule" { when User.HasPoints == true && User.Active == true then User.Eligible = true; }
rule "VIPRule" { when User.Eligible == true then User.IsVIP = true; }
"#;
    let q = q_goal_opts("User.IsVIP == true", "    max-depth: 10");
    let r = prove_ok(grl, &q, json!({ "User.Points": 150, "User.Active": true }));
    assert!(provable(&r), "IsVIP proved through 3 chained rules");
}

/// fork: backward_derives_logical_fact_and_cascade_retracts (the chaining half).
/// GAP: the TMS cascade-retraction the fork goes on to test needs a stateful
/// `retract` API that `vrules_core::prove` does not expose; only the derivation is shadowed.
#[test]
fn chain_two_levels_age_adult_canvote() {
    let grl = r#"
rule "MarkAdult" { when Person.age >= 18 then Person.Adult = true; }
rule "MarkCanVote" { when Person.Adult == true then Person.CanVote = true; }
"#;
    let q = q_goal_opts("Person.CanVote == true", "    max-depth: 10");
    let r = prove_ok(grl, &q, json!({ "Person.age": 20 }));
    assert!(provable(&r));
    assert!(step_rules(&r).contains(&"MarkCanVote".to_string()));
}

// ======================================================================
// B. Conditions & operators (expression_parser, unification compare_values)
// ======================================================================

/// fork: parse_comparison, evaluate_comparison_true/false, is_satisfied_true/false.
/// NOTE: a JSON fact number arrives as `Number(f64)`, and numeric `==` is
/// type-strict on this path, so the literal must be written as a decimal (`25.0`);
/// a bare integer literal (`25`) would not match a `Number(25.0)` fact.
#[test]
fn operator_equal_number() {
    let grl = r#"rule "R" { when User.Age == 25.0 then User.Match = true; }"#;
    let q = q_goal("User.Match == true");
    assert!(provable(&prove_ok(grl, &q, json!({ "User.Age": 25 }))));
    assert!(!provable(&prove_ok(grl, &q, json!({ "User.Age": 30 }))));
}

/// fork: parse_greater_than/less_than/greater_or_equal/less_or_equal,
/// backward_with_numeric_comparisons (passing case), unification::compare_values.
#[test]
fn operator_relational() {
    let cases = [
        (
            r#"rule "R" { when N.v > 100 then N.ok = true; }"#,
            150.0,
            50.0,
        ),
        (
            r#"rule "R" { when N.v < 100 then N.ok = true; }"#,
            50.0,
            150.0,
        ),
        (
            r#"rule "R" { when N.v >= 100 then N.ok = true; }"#,
            100.0,
            99.0,
        ),
        (
            r#"rule "R" { when N.v <= 100 then N.ok = true; }"#,
            100.0,
            101.0,
        ),
    ];
    let q = q_goal("N.ok == true");
    for (grl, pass, fail) in cases {
        assert!(
            provable(&prove_ok(grl, &q, json!({ "N.v": pass }))),
            "{grl} @ {pass}"
        );
        assert!(
            !provable(&prove_ok(grl, &q, json!({ "N.v": fail }))),
            "{grl} @ {fail}"
        );
    }
}

/// fork: parse_not_equal. `!=` is handled in compound goals via the expression
/// parser; pair it with a provable `==` sub-goal.
#[test]
fn operator_not_equal_goal() {
    let grl = r#"rule "R" { when Order.Total >= 100 then Order.Qualifies = true; }"#;
    let q = q_goal("Order.Qualifies == true && Order.Total != 0");
    let r = prove_ok(grl, &q, json!({ "Order.Total": 150 }));
    assert!(provable(&r));
    // Compound goals return a flat, step-less trace and no bindings.
    assert_eq!(r["proof"]["steps"].as_array().unwrap().len(), 0);
    assert_eq!(r["bindings"], json!({}));
}

/// fork: parse_logical_and, evaluate_logical_and, backward_with_numeric_comparisons
/// (failing case — NO assertion), backward_missing_facts_detection (weak assertion).
#[test]
fn condition_logical_and() {
    let grl =
        r#"rule "R" { when Order.Total >= 100 && Order.Items < 10 then Order.Discount = true; }"#;
    let q = q_goal("Order.Discount == true");

    // Both conditions hold → provable.
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "Order.Total": 150, "Order.Items": 5 })
    )));

    // A required fact is entirely ABSENT → not provable, and missing_facts is non-empty
    // (it reports the GOAL pattern, not the missing premise — the engine's behavior).
    let missing = prove_ok(
        grl,
        &q_goal("Order.Discount == true"),
        json!({ "Order.Total": 150 }),
    );
    assert!(
        !provable(&missing) || !missing["missing_facts"].as_array().unwrap().is_empty(),
        "fork's weak assertion: not provable OR missing facts reported"
    );
}

/// fork: parse_logical_or, evaluate_logical_or, backward_with_multiple_or_conditions,
/// multiple_solutions_multiple_paths, disjunction::{creation,add_branch,result_success}.
/// An OR-condition rule is provable through EITHER branch.
#[test]
fn condition_logical_or() {
    let grl =
        r#"rule "R" { when User.Premium == true || User.Points > 500 then User.Access = true; }"#;
    let q = q_goal("User.Access == true");

    // Via the Premium branch.
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "User.Premium": true, "User.Points": 100 })
    )));
    // Via the Points branch.
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "User.Premium": false, "User.Points": 600 })
    )));
    // Neither branch holds → not provable.
    assert!(!provable(&prove_ok(
        grl,
        &q,
        json!({ "User.Premium": false, "User.Points": 100 })
    )));
}

/// fork: parse_negation, goal::{negated_goal,negated_goal_with_expression,
/// normal_goal_not_negated}. Negation of a boolean condition. NOTE: the GRL rule
/// grammar does not accept a bare `!field` condition, so the negative is expressed
/// as `== false` — the same semantics the `!` expression-parser negation evaluates to.
#[test]
fn condition_negation() {
    let grl = r#"rule "R" { when User.IsBanned == false then User.Allowed = true; }"#;
    let q = q_goal("User.Allowed == true");
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "User.IsBanned": false })
    )));
    assert!(!provable(&prove_ok(
        grl,
        &q,
        json!({ "User.IsBanned": true })
    )));
}

/// fork: extract_fields_single, extract_fields_multiple. A rule whose `when`
/// references two distinct fields requires both to resolve.
#[test]
fn condition_two_fields_both_required() {
    let grl =
        r#"rule "R" { when User.IsVIP == true && Order.Amount > 1000 then Order.Special = true; }"#;
    let q = q_goal("Order.Special == true");
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "User.IsVIP": true, "Order.Amount": 1500 })
    )));
    // Second field present-but-failing: the direct condition is checked → not provable.
    assert!(!provable(&prove_ok(
        grl,
        &q,
        json!({ "User.IsVIP": true, "Order.Amount": 10 })
    )));
}

/// fork: parse_boolean_literal, parse_number_literal, parse_string_literal.
/// Boolean, numeric and string literals all work as condition values.
#[test]
fn literal_values_as_conditions() {
    let q = q_goal("Out.ok == true");
    let b = r#"rule "R" { when In.flag == true then Out.ok = true; }"#;
    assert!(provable(&prove_ok(b, &q, json!({ "In.flag": true }))));

    let n = r#"rule "R" { when In.n == 42.5 then Out.ok = true; }"#;
    assert!(provable(&prove_ok(n, &q, json!({ "In.n": 42.5 }))));

    let s = r#"rule "R" { when In.s == "hello" then Out.ok = true; }"#;
    assert!(provable(&prove_ok(s, &q, json!({ "In.s": "hello" }))));
    assert!(!provable(&prove_ok(s, &q, json!({ "In.s": "world" }))));
}

// ======================================================================
// C. Query parsing & goal forms (grl_query, disjunction)
// ======================================================================

/// fork: parse_simple_query, query_config_conversion. Defaults parse and prove.
#[test]
fn query_default_strategy() {
    let grl = r#"rule "R" { when X.v > 0 then Y.v = true; }"#;
    let r = prove_ok(grl, &q_goal("Y.v == true"), json!({ "X.v": 10 }));
    assert!(provable(&r));
}

/// fork: parse_query_with_or_goal, disjunction::parser_simple_or. `||` goal proved
/// through one branch via its deriving rule.
#[test]
fn query_or_goal() {
    let grl = r#"rule "H" { when U.Spent > 10000 then U.Whale = true; }"#;
    let q = q_goal("U.IsVIP == true || U.Whale == true");
    let r = prove_ok(grl, &q, json!({ "U.Spent": 20000 }));
    assert!(provable(&r));
}

/// fork: parse_query_with_multiple_or_branches, disjunction::parser_triple_or.
#[test]
fn query_multiple_or_branches() {
    let grl = r#"rule "R" { when E.years > 10 then E.IsSenior = true; }"#;
    let q = q_goal("E.IsManager == true || E.IsSenior == true || E.IsDirector == true");
    assert!(provable(&prove_ok(grl, &q, json!({ "E.years": 15 }))));
    assert!(!provable(&prove_ok(grl, &q, json!({ "E.years": 2 }))));
}

/// fork: parse_query_with_complex_goal. AND-before-OR precedence in a mixed goal.
#[test]
fn query_complex_goal_and_or() {
    let grl = r#"
rule "RA" { when S.a == true then U.IsVIP = true; }
rule "RB" { when S.b == true then U.Active = true; }
rule "RS" { when S.s == true then U.TotalSpent = true; }
"#;
    let q = q_goal("(U.IsVIP == true && U.Active == true) || U.TotalSpent == true");
    // Left conjunction satisfied.
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "S.a": true, "S.b": true, "S.s": false })
    )));
    // Right disjunct satisfied.
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "S.a": false, "S.b": false, "S.s": true })
    )));
    // Neither.
    assert!(!provable(&prove_ok(
        grl,
        &q,
        json!({ "S.a": true, "S.b": false, "S.s": false })
    )));
}

/// fork: parse_query_with_parentheses.
#[test]
fn query_parentheses() {
    let grl = r#"
rule "RA" { when S.a == true then U.IsVIP = true; }
rule "RB" { when S.b == true then U.Active = true; }
"#;
    let q = q_goal("(U.IsVIP == true && U.Active == true) || U.TotalSpent == true");
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "S.a": true, "S.b": true })
    )));
}

/// fork: parse_query_with_nested_parentheses, disjunction::{parser_nested_parens,
/// parser_nested_or_groups, parser_deeply_nested}.
#[test]
fn query_nested_parentheses() {
    let grl = r#"
rule "RA" { when S.a == true then T.x = true; }
rule "RB" { when S.b == true then T.y = true; }
rule "RD" { when S.d == true then T.d = true; }
"#;
    let q = q_goal("(T.x == true && T.y == true) || T.d == true");
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "S.a": true, "S.b": true, "S.d": false })
    )));
    assert!(provable(&prove_ok(
        grl,
        &q,
        json!({ "S.a": false, "S.b": false, "S.d": true })
    )));
}

/// fork: parse_query_unclosed_parenthesis. Malformed query → `prove` returns Err.
#[test]
fn query_unclosed_paren_errors() {
    let grl = r#"rule "R" { when X.v > 0 then Y.v = true; }"#;
    let bad = "query \"Q\" {\n    goal: (Y.v == true && X.v > 0\n}\n";
    assert!(prove(grl, bad, &json!({ "X.v": 10 })).is_err());
}

/// fork: parse_multiple_queries. `prove` runs one query; two separate calls cover
/// the two parsed queries' behavior.
#[test]
fn two_separate_queries() {
    let grl = r#"
rule "RA" { when S.a == true then A.v = true; }
rule "RB" { when S.b == true then B.v = true; }
"#;
    assert!(provable(&prove_ok(
        grl,
        &q_goal("A.v == true"),
        json!({ "S.a": true })
    )));
    let q2 = q_goal_opts("B.v == true", "    strategy: breadth-first");
    assert!(provable(&prove_ok(grl, &q2, json!({ "S.b": true }))));
}

/// fork: parse_query_with_when_condition, should_execute_no_condition,
/// should_execute_condition_true, should_execute_condition_false. The query-level
/// `when:` gate must hold for the goal to be evaluated. (Uses a resolvable
/// single-segment field; novel dotted types silently fail the gate's field lookup.)
#[test]
fn query_when_gate() {
    let grl = r#"rule "R" { when X.v > 0 then Y.v = true; }"#;
    // No when condition → executes by default.
    assert!(provable(&prove_ok(
        grl,
        &q_goal("Y.v == true"),
        json!({ "X.v": 10 })
    )));

    let gated = q_goal_opts("Y.v == true", "    when: Gate == true");
    // Gate satisfied → goal evaluated and proved.
    assert!(provable(&prove_ok(
        grl,
        &gated,
        json!({ "X.v": 10, "Gate": true })
    )));
    // Gate unsatisfied → query short-circuits to not-provable.
    let blocked = prove_ok(grl, &gated, json!({ "X.v": 10, "Gate": false }));
    assert!(!provable(&blocked));
    assert_eq!(blocked["proof"]["goal"], "");
}

/// fork: should_execute_parse_error_propagates. A malformed `when:` expression
/// (unterminated string) propagates as an Err from `prove`.
#[test]
fn query_when_parse_error() {
    let grl = r#"rule "R" { when X.v > 0 then Y.v = true; }"#;
    let q = "query \"Q\" {\n    goal: Y.v == true\n    when: Gate == \"Prod\n}\n";
    assert!(prove(grl, q, &json!({ "X.v": 10 })).is_err());
}

/// fork: parse_query_with_actions, action_execution. `on-success:` actions parse
/// and the goal still proves. (The actions mutate the engine's facts; that mutation
/// is internal to the executor and not surfaced in prove's result.)
#[test]
fn query_on_success_action() {
    let grl = r#"rule "R" { when X.v > 0 then Y.v = true; }"#;
    let q = "query \"Q\" {\n    goal: Y.v == true\n    on-success: {\n        User.DiscountRate = 0.2;\n        LogMessage(\"ok\");\n    }\n}\n";
    let r = prove_ok(grl, q, json!({ "X.v": 10 }));
    assert!(provable(&r));
}

// ======================================================================
// D. Search strategies & depth (search, goal, multiple_solutions)
// ======================================================================

/// fork: multiple_solutions_with_different_strategies, search::{search_strategies,
/// search_strategy_equality, depth_first_search_*, breadth_first_search,
/// iterative_deepening_search_success}. All three strategies prove a single rule.
#[test]
fn strategies_all_prove_single_rule() {
    let grl = r#"rule "R1" { when X.v > 0 then Y.v = true; }"#;
    for strat in ["depth-first", "breadth-first", "iterative"] {
        let q = q_goal_opts(
            "Y.v == true",
            &format!("    strategy: {strat}\n    max-depth: 10"),
        );
        let r = prove_ok(grl, &q, json!({ "X.v": 10 }));
        assert!(provable(&r), "strategy {strat} should prove the goal");
    }
}

/// fork: depth_first_search_max_depth_exceeded, iterative_deepening_search_depth_limit,
/// goal::{goal_depth,is_too_deep}. A multi-hop chain is provable at a generous depth
/// but NOT at max-depth 0.
#[test]
fn depth_limit_blocks_chain() {
    let grl = r#"
rule "AtoB" { when A.flag == true then B.flag = true; }
rule "BtoC" { when B.flag == true then C.flag = true; }
"#;
    let deep = q_goal_opts("C.flag == true", "    max-depth: 10");
    assert!(provable(&prove_ok(grl, &deep, json!({ "A.flag": true }))));

    let shallow = q_goal_opts("C.flag == true", "    max-depth: 0");
    assert!(!provable(&prove_ok(
        grl,
        &shallow,
        json!({ "A.flag": true })
    )));
}

/// fork: depth_first_search_empty_goal, iterative_deepening_search_no_candidates,
/// goal status Unprovable. A goal with no rule concluding it is not provable.
#[test]
fn no_candidate_goal_unprovable() {
    let grl = r#"rule "R" { when X.v > 0 then Y.v = true; }"#;
    let r = prove_ok(grl, &q_goal("Z.nonexistent == true"), json!({ "X.v": 10 }));
    assert!(!provable(&r));
}

/// fork: conclusion_index::{new_index_empty,index_creation,is_empty,clear_index}.
#[test]
fn empty_kb_unprovable() {
    let r = prove_ok("", &q_goal("Anything.v == true"), json!({}));
    assert!(!provable(&r));
    assert_eq!(r["proof"]["goal"], "");
    assert!(r["proof"]["steps"].as_array().unwrap().is_empty());
}

// ======================================================================
// E. Candidate-rule selection by conclusion field (conclusion_index, proof_graph)
// ======================================================================

/// fork: conclusion_index::{add_single_rule,find_candidates_single,find_candidates_exact_match}.
#[test]
fn candidate_single_rule() {
    let grl = r#"rule "DetermineVIP" { when U.Spent > 1000 then User.IsVIP = true; }"#;
    let r = prove_ok(
        grl,
        &q_goal("User.IsVIP == true"),
        json!({ "U.Spent": 2000 }),
    );
    assert!(provable(&r));
    assert_eq!(step_rules(&r), vec!["DetermineVIP"]);
}

/// fork: find_candidates_multiple_matches, find_candidates_multiple_rules,
/// breadth_first_search_multiple_candidates, proof_graph::multiple_justifications,
/// backward_with_multiple_solution_paths. Two rules conclude the same field; the
/// goal is provable, and both deriving rules appear as candidate steps.
#[test]
fn candidate_multiple_rules_same_field() {
    let grl = r#"
rule "ByAge" { when User.Age >= 21 then User.CanDrink = true; }
rule "ByLicense" { when User.HasLicense == true then User.CanDrink = true; }
"#;
    let q = q_goal("User.CanDrink == true");

    // Provable via age.
    let via_age = prove_ok(grl, &q, json!({ "User.Age": 25, "User.HasLicense": false }));
    assert!(provable(&via_age));
    assert!(step_rules(&via_age).contains(&"ByAge".to_string()));

    // Provable via license.
    let via_lic = prove_ok(grl, &q, json!({ "User.Age": 18, "User.HasLicense": true }));
    assert!(provable(&via_lic));
}

/// fork: from_rules_creates_index, from_rules_bulk_creation, remove_rule, stats.
/// A rule concluding a DIFFERENT field is no help for the goal.
#[test]
fn candidate_different_field_irrelevant() {
    let grl = r#"
rule "VIPRule" { when U.Spent > 1000 then User.IsVIP = true; }
rule "OrderRule" { when O.paid == true then Order.Approved = true; }
"#;
    // Only OrderRule's field is unsatisfiable here; the VIP goal still proves via VIPRule.
    let r = prove_ok(
        grl,
        &q_goal("User.IsVIP == true"),
        json!({ "U.Spent": 5000 }),
    );
    assert!(provable(&r));
    assert_eq!(step_rules(&r), vec!["VIPRule"]);

    // A goal on a field only OrderRule concludes is unprovable without its premise.
    let no = prove_ok(grl, &q_goal("Order.Approved == true"), json!({}));
    assert!(!provable(&no));
}

/// fork: index_multiple_fields. One rule with two Set actions can prove a goal on
/// either conclusion field. (Boolean conclusions are used: a numeric derived fact
/// like `Points = 1000` would not satisfy a `== 1000` integer-literal goal under the
/// type-strict numeric equality documented in `operator_equal_number`.)
#[test]
fn candidate_multiple_fields_one_rule() {
    let grl =
        r#"rule "MultiRule" { when In.go == true then User.IsVIP = true; User.Premium = true; }"#;
    assert!(provable(&prove_ok(
        grl,
        &q_goal("User.IsVIP == true"),
        json!({ "In.go": true })
    )));
    assert!(provable(&prove_ok(
        grl,
        &q_goal("User.Premium == true"),
        json!({ "In.go": true })
    )));
}

/// fork: conclusion_index::extract_field_from_goal — a goal field with no concluding
/// rule yields not-provable and a non-empty missing_facts naming the goal pattern.
#[test]
fn goal_field_no_rule_unprovable() {
    let grl = r#"rule "R" { when X.v > 0 then Y.v = true; }"#;
    let r = prove_ok(
        grl,
        &q_goal("User.Nonexistent == true"),
        json!({ "X.v": 10 }),
    );
    assert!(!provable(&r));
    let missing = r["missing_facts"].as_array().unwrap();
    assert!(missing.iter().any(|m| m == "User.Nonexistent == true"));
}

// ======================================================================
// F. Function conditions (builtin) + embedding-gating evidence
// ======================================================================

/// fork: backward_engine::test_function_call_condition_len. A builtin function
/// condition (`len`) in a backward rule's `when` is evaluated and can prove a goal.
#[test]
fn builtin_len_function_condition() {
    let grl = r#"rule "CheckNameLength" { when len(User.Name) > 3 then User.HasLongName = true; }"#;
    let q = q_goal("User.HasLongName == true");
    // "John".len() == 4 > 3 → provable.
    assert!(provable(&prove_ok(grl, &q, json!({ "User.Name": "John" }))));
    // "Jo".len() == 2, not > 3 → not provable.
    assert!(!provable(&prove_ok(grl, &q, json!({ "User.Name": "Jo" }))));
}

/// Embedding-gating evidence. The forward routing path registers `s_cosine`
/// function via `vrules_core::register_vector_functions` on `RustRuleEngine`. The backward
/// `vrules_core::prove` path does not: `prove` builds `BackwardEngine::new(kb)`, whose
/// `RuleExecutor` constructs `ConditionEvaluator::with_builtin_functions()`
/// (builtin-only). `prove.rs` exposes no function-registration hook, and the
/// evaluator field is private with no injecting constructor, so an `s_cosine(...)`
/// condition in a backward rule resolves to no function and the goal is NOT provable.
/// Therefore embedding-driven gating is unavailable on the backward path; this is a
/// documented capability gap, not a stub. Builtin function conditions (above) DO work.
#[test]
fn embedding_gating_unavailable_on_backward_path() {
    let grl =
        r#"rule "R" { when s_cosine(Doc.Text, "refund policy") > 0.5 then Doc.Match = true; }"#;
    let q = q_goal("Doc.Match == true");
    let r = prove_ok(grl, &q, json!({ "Doc.Text": "how do refunds work" }));
    assert!(
        !provable(&r),
        "s_cosine is not registered on the backward path"
    );
}

// ======================================================================
// G. Missing-fact detection
// ======================================================================

/// fork: backward_missing_facts_detection, find_missing_facts. With a required fact
/// absent, the goal is not provable and missing_facts is non-empty (reporting the
/// goal pattern). Mirrors the fork's deliberately weak assertion.
#[test]
fn missing_facts_contains_goal() {
    let grl = r#"rule "CanRegisterRule" { when User.Age >= 18 && User.Country == "US" then User.CanRegister = true; }"#;
    let r = prove_ok(
        grl,
        &q_goal("User.CanRegister == true"),
        json!({ "User.Age": 25 }),
    );
    assert!(!provable(&r));
    assert!(!r["missing_facts"].as_array().unwrap().is_empty());
}

// ======================================================================
// H. Proof-trace shape (proof_tree, explanation, proof_trace_generation)
// ======================================================================

/// fork: backward_proof_trace_generation, proof_tree::{proof_node_creation,fact_node,
/// rule_node,proof_tree_creation}, explanation::{build_proof_tree,goal_proven_by_fact}.
/// A provable single-rule goal yields a non-empty trace whose concluding step names
/// the rule, at depth 0, with empty sub_steps (the trace is flat on this path).
#[test]
fn proof_trace_single_rule_shape() {
    let grl = r#"rule "VerifiedUserRule" { when User.Verified == true then User.Trusted = true; }"#;
    let r = prove_ok(
        grl,
        &q_goal("User.Trusted == true"),
        json!({ "User.Verified": true }),
    );
    assert!(provable(&r));
    assert_eq!(r["proof"]["goal"], "User.Trusted == true");
    let steps = r["proof"]["steps"].as_array().unwrap();
    assert!(!steps.is_empty());
    assert_eq!(steps[0]["rule"], "VerifiedUserRule");
    assert_eq!(steps[0]["depth"], 0);
    assert!(steps[0]["sub_steps"].as_array().unwrap().is_empty());
}

/// Compound (`||`) goal: provable but the trace is step-less and bindings empty.
#[test]
fn proof_trace_compound_goal_flat() {
    let grl = r#"rule "H" { when U.Pts > 500 then U.Whale = true; }"#;
    let q = q_goal("U.Prem == true || U.Whale == true");
    let r = prove_ok(grl, &q, json!({ "U.Pts": 600 }));
    assert!(provable(&r));
    assert_eq!(r["proof"]["steps"].as_array().unwrap().len(), 0);
    assert_eq!(r["bindings"], json!({}));
}

/// Not-provable goal: empty proof goal and no steps.
#[test]
fn proof_trace_not_provable_empty() {
    let grl = r#"rule "R" { when X.v > 0 then Y.v = true; }"#;
    let r = prove_ok(grl, &q_goal("Y.v == true"), json!({ "X.v": -1 }));
    assert!(!provable(&r));
    assert_eq!(r["proof"]["goal"], "");
    assert!(r["proof"]["steps"].as_array().unwrap().is_empty());
}

// ======================================================================
// I. Memoization & rule-order independence (optimizer, goal caching, proof_graph)
// ======================================================================

/// fork: proof_graph::{caching_basic,insert_and_lookup,fact_key_from_pattern},
/// goal::cache_result, cache_statistics. The same query repeated yields the same
/// (correct) result; memoization is internal but must not change the answer.
#[test]
fn memoization_repeat_query() {
    let grl = r#"
rule "AtoB" { when A.flag == true then B.flag = true; }
rule "BtoC" { when B.flag == true then C.flag = true; }
"#;
    let q = q_goal_opts("C.flag == true", "    max-depth: 10");
    let r1 = prove_ok(grl, &q, json!({ "A.flag": true }));
    let r2 = prove_ok(grl, &q, json!({ "A.flag": true }));
    assert!(provable(&r1) && provable(&r2));
}

/// fork: optimizer::{creation,with_config,memoization_hit_rate}. Disabling
/// memoization in the query does not change provability.
#[test]
fn memoization_disabled_still_proves() {
    let grl = r#"
rule "AtoB" { when A.flag == true then B.flag = true; }
rule "BtoC" { when B.flag == true then C.flag = true; }
"#;
    let q = q_goal_opts(
        "C.flag == true",
        "    enable-memoization: false\n    max-depth: 10",
    );
    assert!(provable(&prove_ok(grl, &q, json!({ "A.flag": true }))));
}

/// fork: optimizer::{goal_reordering,disable_reordering}. Provability is independent
/// of the order rules appear in the knowledge base.
#[test]
fn rule_order_independent() {
    let q = q_goal_opts("C.flag == true", "    max-depth: 10");
    let forward = r#"
rule "AtoB" { when A.flag == true then B.flag = true; }
rule "BtoC" { when B.flag == true then C.flag = true; }
"#;
    let reversed = r#"
rule "BtoC" { when B.flag == true then C.flag = true; }
rule "AtoB" { when A.flag == true then B.flag = true; }
"#;
    assert!(provable(&prove_ok(forward, &q, json!({ "A.flag": true }))));
    assert!(provable(&prove_ok(reversed, &q, json!({ "A.flag": true }))));
}

// ======================================================================
// J. Bindings (unification) — documented behavior
// ======================================================================

/// fork: unification::* (Bindings/Unifier), goal::{goal_with_expression,goal_bindings}.
/// GRL goals carry no logic variables, so `prove` returns an empty `bindings` map.
/// This documents the observable contract; variable unification is an internal
/// mechanism with no surface through `vrules_core::prove`.
#[test]
fn bindings_empty_on_field_goal() {
    let grl = r#"rule "R" { when X.v > 0 then Y.v = true; }"#;
    let r = prove_ok(grl, &q_goal("Y.v == true"), json!({ "X.v": 10 }));
    assert!(provable(&r));
    assert!(r["bindings"].is_object());
    assert_eq!(r["bindings"], json!({}));
}

// ======================================================================
// K. Concurrency (prove.rs existing parallel tests, mirrored through prove)
// ======================================================================

/// fork/prove.rs: proves_identical_inputs_in_parallel, proves_mixed_inputs_in_parallel.
/// `vrules_core::prove` is correct under concurrent use (the engine is rebuilt per call).
#[test]
fn prove_in_parallel() {
    use std::thread;
    let grl =
        r#"rule "VIPRule" { when User.LoyaltyPoints >= 1000 then User.IsVIP = true; }"#.to_string();
    let q = q_goal_opts("User.IsVIP == true", "    max-depth: 5");

    let mut handles = Vec::new();
    for i in 0..16 {
        let grl = grl.clone();
        let q = q.clone();
        handles.push(thread::spawn(move || {
            let points = if i % 2 == 0 { 1200 } else { 100 };
            let expected = i % 2 == 0;
            for _ in 0..20 {
                let r = prove_ok(&grl, &q, json!({ "User.LoyaltyPoints": points }));
                assert_eq!(provable(&r), expected);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}
