# vrules-core

`vrules-core` is a thin, synchronous integration layer over
[`rust-rule-engine`](https://github.com/ops-ping/rust-rule-engine). It keeps the
engine's GRL, `Rule`, `KnowledgeBase`, `Facts`, `RustRuleEngine`,
`BackwardEngine`, and streaming APIs intact and adds host-registered vector,
canonicalization, and reference-workflow functions.

## Forward evaluation

GRL is the only authored rule format. JSON is used for facts and results.

```rust
use rust_rule_engine::Facts;
use serde_json::json;
use vrules_core::{RuleEvaluator, Ruleset, add_json_fact};

let rules = Ruleset::parse(r#"
    rule "RouteGrounding" no-loop {
        when Request.tool == "web_ground"
        then Decision.backend = "ai.vrules.grounding";
    }
"#)?;

let facts = Facts::new();
add_json_fact(&facts, "Request", &json!({ "tool": "web_ground" }))?;
add_json_fact(&facts, "Decision", &json!({}))?;

let mut evaluator = RuleEvaluator::new(rules)?;
let outcome = evaluator.evaluate(&facts, true)?;
assert_eq!(outcome.decision["backend"], "ai.vrules.grounding");
# Ok::<(), vrules_core::VrulesError>(())
```

`Ruleset` parses GRL with the canonical thread-safe parser and builds ordinary
`RustRuleEngine` instances. `RuleEvaluator` resets per-evaluation no-loop
tracking, executes the engine, and returns fired rules, final structured facts,
the `Decision` fact, and optional execution statistics.

Nested JSON objects remain engine `Value::Object` values. The shared conversion
helpers are:

- `add_json_fact`
- `json_to_rule_value`
- `rule_value_to_json`
- `facts_to_json`

## Extension functions

Extensions use `RustRuleEngine::register_function_with_meta` and
`RustRuleEngine::register_action_handler`; they do not add rule syntax.

Function names carry their return kind — `s_` raw scalar, `c_` calibrated
(thresholdable), `b_` boolean, `m_` metadata — and each registration declares
the same kind as engine metadata, which the loader lints rules against: a raw
scalar compared in `when` is a load-time error ("calibrate it or assign it to
a fact in `then`"), and offline-tier operations cannot appear in live rules.

Vector functions require a real host-supplied `Embedder` and, for the
artifact-backed operations, a `geometry::ArtifactStore` whose provenance
(model, dimension) is validated at registration:

- `s_cosine(left, right)` / `s_dot(left, right)`
- `s_contrast(candidate, positive, negative)` — `cos(x,pos) − cos(x,neg)`
- `s_project(text, axis)` / `c_project(text, axis)` — named axis artifact
- `s_depth(text, region)` / `b_member(text, region)` — named region artifact

Axes, calibration windows, and regions are fitted offline via
`geometry::{Axis, Calibration, Region}` and serialized as JSON artifacts;
rules reference them by name only.

Canonicalization functions are deterministic and side-effect free:

- `s_canon_match(text, label)` / `b_canon_matches(text, label)`
- `m_canon_label(text)`
- `s_canon_near(left, right)`
- `m_canon_id(text)`
- `m_canonical(text)`

Effects are GRL actions in `then`. A registered function call may be the whole
right-hand side of a `then` assignment (`Fact.score = s_cosine(...)`), which is
how measurement rules feed decision rules through facts. Product-specific
native functions and action handlers follow the same upstream extension APIs.

## Synchronous streaming

The re-exported `StreamProcessor` is rust-rule-engine's runtime-neutral
one-event-in/one-result-out stream core:

```text
StreamEvent
  -> watermark and late-data policy
  -> named-stream and event-type binding
  -> windows, joins, aggregates, and state
  -> RustRuleEngine
  -> StreamProcessingResult
```

Processing is synchronous and deterministic on the calling thread. The
underlying engine also provides an optional Tokio driver and an optional Redis
state backend; neither changes rule semantics.

## Backward proof

`prove(grl, query, facts)` delegates to rust-rule-engine's `BackwardEngine` and
returns provability, bindings, missing facts, and proof steps. Forward
evaluation and backward proof consume the same GRL rule model.

## Features

| Feature | Purpose |
|---|---|
| `rule-engine` | GRL forward evaluation, backward proof, and synchronous streaming |
| `embeddings` | Host-supplied real embedding support for vector functions |

The crate contains no transport, async runtime, model lifecycle, rule
repository, or persistence policy.
