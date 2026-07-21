# Roadmap

This document tracks product-level roadmap items for vrules. Crate-specific
implementation status stays with the owning crate docs.

## Items

### 1. [Web UI Git integration uses hosted Git, not filesystem mutation](https://github.com/ops-ping/vector-rules/issues/1)

Make the browser console interact with Git through hosted Git provider APIs and
review flows instead of reading or mutating the rules repository directly through
the local filesystem. The UI should validate, diff, propose, and promote rule
changes through explicit Git operations that preserve reviewability, provenance,
and rollback.

Local filesystem access can remain a daemon implementation detail, but the web UI
contract should be Git-on-the-web rather than file-manager semantics.

### 3. [Extend semantic function coverage across engine paths](https://github.com/ops-ping/vector-rules/issues/3)

Extend the registered vector (`s_cosine`, `s_contrast`, `c_project`, `b_member`) and canonicalization
functions across engine paths that support custom functions, including nested
boolean forms, quantifiers, and proof when the upstream backward evaluator
gains a compatible function-registration surface.

The release gate is that every condition evaluator treats vector and canonical
evidence consistently and produces replayable traces with the active model,
canonicalizer, and rule revision recorded without introducing another authored
rule syntax or evaluator.

### 4. [OpenClaw organizational memory and policy-as-code plugin](https://github.com/ops-ping/vector-rules/issues/4)

Build an OpenClaw plugin that dogfoods vector-rules as a governed organizational memory
and policy layer while preserving OpenClaw's existing personal-memory stores.

The plugin should:

- expose vector-rules organizational memory through normal OpenClaw memory/search tool
  surfaces so users do not need to migrate local personal-memory stores
- adapt existing OpenClaw memory hits as one corpus while querying the vector-rules
  organizational corpus separately
- use the component runtime for shared, content-addressed, canonicalized embedding
  lookups across the organization
- use vector-rules rules to rank and gate recall with deterministic facts,
  registered canon and vector functions, provenance, trust level, ACL, source,
  recency, and audit signals
- stamp every recall, ranking decision, promotion, and policy result with the
  active rules revision, model version, canonicalizer version, source references,
  and explanation trace
- keep organizational memory promotion behind policy-as-code review instead of
  automatic personal-memory capture

The first milestone is an adapter-only integration: OpenClaw keeps its existing
stores and calls into vector-rules for governed organizational recall, ranking, and
audit. Later milestones can add promotion workflows, browser what-if replay, and
team/org namespace management.

### 5. [GPU tensor rule engine](https://github.com/ops-ping/vector-rules/issues/5)

Add a future GPU-native execution mode for whole-ruleset batch evaluation and
large fact populations. The interactive component path stays on the canonical
per-request `RustRuleEngine` path; the tensor engine is for workloads that need
massively parallel scoring, heavy fixed-point iteration, or large joins.

See [GPU-TENSOR-ROADMAP.md](GPU-TENSOR-ROADMAP.md) for the design sketch and
prior art.

### 6. [Arrow snapshots and batch adapters](https://github.com/ops-ping/vector-rules/issues/6)

Add native Arrow IPC interchange and columnar adapters for DataFusion and
Spark-style hosts. The synchronous `StreamProcessor` API is already
host-neutral; this milestone makes event and state interchange efficient for
columnar engines and large streaming/batch workloads.

The adapter contract preserves `RustRuleEngine` and `StreamProcessor` semantics:
hosts feed facts, events, or batches without introducing another rule model or
forking behavior across component, DataFusion, and Spark surfaces.

The address-verification reference implementation is the first multimodal
conformance case for this adapter. Its PWA and the future DataFusion analysis
must consume the same fact shapes and `shared-rules/address/*.grl` policy. This
proves generic business-rule portability; it does not make address verification
part of the core framework.
