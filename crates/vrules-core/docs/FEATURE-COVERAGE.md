# Feature coverage

`vrules-core` uses rust-rule-engine directly rather than maintaining separate
execution paths.

| Capability | Implementation |
|---|---|
| GRL parsing | Canonical thread-safe `GRLParser` |
| Forward evaluation | `RustRuleEngine` |
| Salience, no-loop, agenda, actions, handlers, joins, modules | `RustRuleEngine` |
| Structured facts | `Facts` and `Value::Object` |
| Vector measurements | Registered `s_cosine`, `s_dot`, `s_contrast` functions |
| Named geometry artifacts | `geometry::ArtifactStore` — axes, calibration windows, low-rank ellipsoid regions, each with model/dim provenance |
| Axis projection | `s_project` (raw) and `c_project` (calibrated percentile) over named axis artifacts |
| Region membership | `s_depth` (graded) and `b_member` (fitted threshold) over named region artifacts |
| Return-kind discipline | `register_function_with_meta` + engine load-time lint: raw scalars can't be thresholded in `when`, offline-tier ops can't run live |
| Layered composition | `then`-clause assignment dispatches registered functions (`Fact.field = s_cosine(...)`), so measurement rules feed decision rules through facts |
| Canonical match, label, near match, and ID | Registered pure canon functions (`s_canon_match`, `b_canon_matches`, `m_canon_label`, `s_canon_near`, `m_canon_id`, `m_canonical`) |
| Product/native functions | Upstream function, method, and action-handler registration |
| Backward proof | `BackwardEngine` through `prove` |
| Sequential streaming | `StreamProcessor::process_event` |
| Named stream and event-type filtering | Upstream stream-pattern evaluation |
| Windows, watermarks, late data, joins, aggregates | Upstream streaming primitives |
| In-memory and file state | Upstream `StateStore` |
| Redis state | Optional upstream `streaming-redis` feature |
| Async channel driver | Optional upstream `streaming` feature |
| Browser forward evaluation | `vrules-wasm` over `Ruleset` and `RuleEvaluator`, including in-browser artifact fitting (`fit_axis`, `fit_region`) |
| Git-governed rule loading | `vrules-rules-component` reading `.grl` files |
| Fork compatibility | Upstream public models and extension APIs; narrow engine-only fixes |

Function-name prefixes are the human-readable projection of the registered
metadata: `s_` raw scalar (measure only), `c_` calibrated/decision-scale
(thresholdable), `b_` boolean, `m_` metadata. The loader enforces the same
contract, so the prefix and the lint cannot drift apart.

Region–region operations (overlap, separation, drift sweeps) are offline-tier
by design and intentionally have no GRL surface; they belong to linting and
promotion tooling, not live rules.

Engine limitations belong upstream and are not replaced with local rule syntax
or parallel evaluators.
