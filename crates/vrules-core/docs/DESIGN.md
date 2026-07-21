# Design

`vrules-core` preserves rust-rule-engine semantics across native, WASI, browser,
and application hosts. It adds model-backed vector functions and deterministic
canonicalization through the engine's existing extension APIs.

## Invariants

1. GRL is the only authored rule format.
2. `RustRuleEngine` is the forward evaluator.
3. `BackwardEngine` remains the explicit proof evaluator.
4. Effects execute from GRL `then` actions, never from condition functions.
5. JSON objects remain structured engine objects.
6. Embeddings come from a real host-supplied model with stable model identity.
7. Streaming is synchronous one-input/one-output core processing; async drivers
   are optional transport adapters.
8. State, windows, watermarks, joins, action handlers, agenda behavior, and
   other engine capabilities remain upstream types and semantics.
9. The rust-rule-engine fork stays consistent with the originating project
   wherever possible; deviations are narrow, explicit, and upstreamable.

## Forward path

```text
GRL
  -> canonical thread-safe parser
  -> Rule / KnowledgeBase
  -> RustRuleEngine
       + vector functions
       + canonicalization functions
       + host functions and actions
  -> final Facts + fired rules + execution statistics
```

`Ruleset` stores parsed upstream `Rule` values so hosts can build identically
configured evaluators. `RuleEvaluator` is only a result-shaping adapter; it does
not implement conditions, actions, conflict resolution, or working memory.

## Fork consistency

rust-rule-engine is the engine of record. The fork exists to carry generally
useful fixes and runtime capabilities while they are prepared for the
originating project; it is not a place to create a separate vrules rule engine.
Maintaining that relationship is necessary so parser fixes, engine corrections,
and new upstream releases can roll forward without a translation layer or a
second set of semantics.

Fork changes preserve upstream public types, GRL syntax, evaluator behavior,
and feature boundaries whenever possible. Parser correctness, error
propagation, numeric behavior, and runtime-neutral streaming belong in the fork
because they are engine concerns. Vector, canonicalization, address, MCP, and
other product behavior stays in vector-rules and uses upstream function or
action registration APIs.

A change that cannot remain origin-compatible must be:

1. required by an engine-level correctness or portability constraint;
2. smaller than an equivalent vrules-only abstraction;
3. isolated from authored rule syntax and product policy;
4. documented with the exact semantic difference; and
5. covered by a compatibility test that makes subsequent upstream reconciliation
   explicit.

No host may compensate for an engine limitation by introducing another rule
format, evaluator, working-memory model, or streaming session abstraction.

## Streaming path

```text
StreamEvent
  -> StreamProcessor
       -> watermark and late-data handling
       -> stream/event binding
       -> windows, joins, aggregates, and StateStore
       -> RustRuleEngine
  -> StreamProcessingResult
```

The caller invokes `process_event` sequentially. The optional upstream Tokio
driver delegates to this processor. The optional Redis backend implements the
same `StateStore` contract.

## Semantic extensions

GRL calls ordinary registered functions such as
`c_project(Request.text, "urgency_v1")` and
`b_canon_matches(Request.text, "known-pattern")`. Vector geometry is exposed as
named functions over named artifacts rather than a second JSON expression tree,
and every function declares its return kind (`s_` raw scalar, `c_` calibrated,
`b_` boolean, `m_` metadata) as engine metadata that the loader lints rules
against — raw scalars cannot be thresholded in `when`; they are assigned to
facts in `then` and thresholded downstream. Canonicalization functions are
pure. GRL assignments (including registered-function right-hand sides) and
registered action handlers own effects.

## Host boundaries

`vrules-core` does not know about MCP, WebSocket, stdio, repositories, browser
streams, DataFusion, or Spark. Those hosts translate their input into upstream
`Facts` or `StreamEvent` values and consume the same result types.
