# vector-rules

**vector-rules defines an effective production role for LLMs while exposing the
power of embeddings to every layer of the technology stack.**

vector-rules turns meaning found in documents, messages, events, and records
into deterministic, governed decisions. It combines embeddings, which measure
semantic relationships in content, with a rules engine, which applies explicit
organizational policy to those measurements and ordinary application facts.

LLMs are good at interpreting language, generating candidates, and explaining
complex information. They are not, by themselves, a reviewable source of
production policy. vector-rules puts a deterministic, GitOps-managed rule layer
between fuzzy inputs and operational actions so behavior can be tested, traced,
audited, replayed, and rolled back.

The project is fully open source, vendor-neutral, local-first, and embeddable.
The same rule semantics can govern an MCP component host, run inside a Rust
application, evaluate stateful streams, and power browser-side what-if analysis
through WebAssembly. Organizations keep their policy in git and can move it
across models, clients, and deployment environments without depending on a
proprietary control plane.

## vector-rules in action

The following execution trace demonstrates how semantic meaning is turned into governed policy:

*   **Semantic evidence:** Real vector embeddings (e.g., EmbeddingGemma) measure similarity and contrast directly within a rule.
*   **Deterministic chaining:** Instead of an LLM making a black-box decision, rules capture these fuzzy measurements as facts, and subsequent rules apply explicit thresholds to derive a final decision.
*   **Complete traceability:** The engine trace shows exactly which rules fired, what intermediate facts were derived, and how the policy reached its conclusion.

![Semantic similarity over EmbeddingGemma vectors with a forward-chaining trace, running locally in the browser](docs/examples-semantic.png)

This is just one example of what the engine can do. Check out the [browser examples gallery](docs/EXAMPLES.md) to see Address Verification, Fraud Triage, Streaming, and backward-chaining Proof—all running entirely in the browser using WebAssembly.

## What is a rules engine?

A rules engine applies **declarative policy** to facts. Conventional application
code describes a fixed sequence of calls and branches: first do this, then test
that, then choose a path. A rule instead states only the policy relationship:
**when these facts are true, derive this fact or take this action**. The engine
determines which rules apply, adds their conclusions to the working facts, and
continues evaluating until no rule can derive another conclusion.

This forward chaining automatically wires simple, independently understandable
policies into complex behavior. Rules do not call one another or need to know
the complete workflow. They connect through the facts they consume and produce:

| Simple policy | Fact or decision it produces |
|---|---|
| Language in a document expresses unusual urgency or pressure | `urgent_request` |
| A payment has a new payee and exceeds an amount threshold | `elevated_payment_risk` |
| A request is urgent and has elevated payment risk | `manual_review_required` |
| Manual review is required | Hold the payment and record the reason |

Each policy remains small, but together they produce a multi-stage decision.
There is no hard-coded orchestration between the first rule and the last. The
document and its surrounding context supply the facts that connect the policy
graph, so different content naturally assembles a different decision path.
Adding a new policy can extend that graph without rewriting a central decision
tree or prompt.

This is the advantage of **policy as code**:

- **Composable:** small policies combine through shared facts instead of a
  growing web of application-specific branches.
- **Content-driven:** the applicable policy path follows the document rather
  than forcing every document through one predefined workflow.
- **Explainable:** a trace shows each input fact, fired rule, derived fact, and
  final decision.
- **Governable:** policy can be reviewed, tested, versioned, promoted, and
  rolled back in git.
- **Reusable:** the same policy and fact model can run across applications,
  agents, streams, batch jobs, and browsers.

## What are embeddings?

An embedding model converts content into a fixed-length list of numbers called
a **vector**. That vector represents patterns of meaning learned by the model.
Content with related meaning tends to produce vectors that are close together,
even when it uses different words or comes from differently named fields.

This makes embeddings useful wherever exact keywords and rigid schemas are too
brittle. For example, they can recognize that `ship_to`, `delivery address`,
and `destination` probably describe the same kind of field, or measure whether
a message sounds more urgent and pressured than routine and flexible.

Embeddings are measurements, not decisions. A similarity score does not know
whether an organization should accept an address, hold a payment, expose a
tool, or recall a memory. vector-rules turns those measurements into governed
evidence: rules combine semantic relationships with exact values, context,
state, and organizational policy before taking action.

The project uses real embedding models and tracks model identity so evidence is
not silently mixed across incompatible vector spaces. Content-addressed caches
reuse vectors for repeated content, allowing the same semantic evidence to
serve rule evaluation, memory, applications, streams, and browser analysis.

## Vision

AI-linked systems need both probabilistic interpretation and deterministic
control. vector-rules gives each one a clear responsibility:

1. **Models and embeddings interpret meaning.** They classify language, surface
   semantic relationships, retrieve relevant context, and help people work with
   policy.
2. **Rules decide production behavior.** Explicit conditions combine semantic
   evidence with request context, prior facts, organizational policy, and
   application state.
3. **Git governs change.** Rules move through normal review, testing, promotion,
   and rollback workflows.
4. **Traces and audit preserve accountability.** Decisions record the rules
   revision and evidence that produced them.

This division makes embeddings useful beyond a vector database or an LLM prompt.
Semantic similarity becomes evidence inside a deterministic rule network, where
it can gate conditions, derive facts, select tools, route requests, and control
memory recall.

### The role of LLMs

vector-rules is not anti-LLM; it gives LLMs a bounded role that plays to their
strengths. An LLM can translate policy intent into candidate rules, propose
edge-case facts, explain diffs and traces, summarize audit history, and prepare
rollout notes. Schema validation, rule parsing and evaluation, stream state,
proof, pinned rule revisions, and audit records remain the reviewable source of
runtime truth.

### Operating principles

- **Deterministic production behavior.** The canonical rule engine, rather than
  an opaque reasoning loop, makes decisions. Forward evaluation returns
  execution traces; backward chaining can prove a goal and return its proof
  tree.
- **Governed organizational memory.** Embeddings are canonicalized,
  content-addressed, cached, and reused. Recall can be controlled by policy and
  recorded with provenance instead of existing only as transient prompt
  context.
- **One policy surface across runtimes.** The same rules drive MCP tool
  exposure, request routing, embedded application behavior, sequential stream
  processing, and browser what-if analysis.
- **GitOps governance.** Every deployed decision can be tied to the exact rules
  commit that produced it, using whatever review and promotion controls an
  organization already trusts.
- **Open, composable infrastructure.** Teams can adopt the core library, browser
  package, component runtime, canonicalizer, or embedding cache independently
  and can register native Rust functions without coupling rule semantics to a
  transport or model vendor.

## From input to decision

A complete vector-rules decision path can use the following stages:

1. A host turns a request, message, event, or record into facts.
2. Canonicalization normalizes recurring variants, identifies structural
   near-duplicates, and stabilizes keys.
3. When meaning is needed, the embedding layer turns semantic relationships
   into evidence. Content-addressed caches avoid recomputing the same vectors.
4. The rule engine combines that evidence with ordinary facts and state, fires
   matching rules, and forward-chains over derived facts.
5. Rules produce an explicit decision: expose a tool, select a provider, recall
   memory, route a request, write a value, or update stream state.
6. The host records the decision, trace, active rules revision, and relevant
   model or canonicalizer identity.

This creates a cheap deterministic pre-tier for common inputs while preserving
semantic reasoning for cases that need it. Vector search is not bolted onto the
side of the engine: canonical and vector expressions participate directly in
rule evaluation.

## Reference examples

The standalone [`apps/examples`](apps/examples) application is a suite of
executable capability demonstrations. The suite runs the Rust rule engine in
the browser, and its semantic examples use real EmbeddingGemma vectors:

| Example | What it demonstrates |
|---|---|
| **Address verification** | Structured and unstructured input, native functions, canonicalization, embeddings, local reference indexes, and editable organizational policy in one auditable flow |
| **Semantic rules** | Similarity and contrast measurements, return-kind enforcement, and forward chaining from embedding evidence into deterministic facts |
| **Fraud triage** | Calibrated axes and fitted regions combined with hard facts such as payee status and amount |
| **Streaming** | Sequential one-event/one-result rule processing with throughput and decision output |
| **Proof** | Backward chaining with provability, missing facts, and a visible proof tree |

Address verification is one reference workflow in this suite, not a framework
business domain. Its PWA and native hosts consume the same structured facts and
`shared-rules/address/*.grl` policy. Address-oriented crates and
`vrules-core::address` support that example without becoming required runtime
components or core rule semantics.

## Embeddings inside deterministic rules

GRL calls vector operations as ordinary registered functions, so embedding
measurements participate in the same condition evaluation as scalar facts. The
browser console runs the same evaluator through `vrules-wasm` and obtains real
vectors from the configured embedding component through the daemon RPC surface.

Function names carry their return kind, and the engine lints loaded rules
against declared function metadata, so misuse fails at load with a legible
error instead of skewing a decision at runtime:

| Prefix | Kind | May appear in `when` as |
|---|---|---|
| `s_` | raw scalar (geometry measurement) | nothing — assign to a fact in `then`, or use a `c_` form |
| `c_` | calibrated / decision-scale scalar | any comparison (`c_project(...) >= 90.0`) |
| `b_` | boolean | `== true` / `== false` / `test(...)` |
| `m_` | metadata (label, identifier) | equality and string operators |

The prefix describes how a result may be used; the function describes the
embedding operation. The built-in vector surface is:

| Function | What it does | Result and rule use |
|---|---|---|
| `s_cosine(left, right)` | Embeds two texts and measures cosine similarity. | Raw similarity; assign it to a fact in `then`. |
| `s_dot(left, right)` | Embeds two texts and takes their dot product without normalizing in the function. | Raw scalar that preserves any vector-magnitude signal; assign it to a fact in `then`. |
| `s_contrast(candidate, positive, negative)` | Computes `cos(candidate, positive) - cos(candidate, negative)` so shared topic meaning cancels and the semantic contrast remains. | Raw directional evidence; assign it to a fact in `then`. |
| `s_project(text, axis)` | Projects text onto a named axis fitted from positive and negative exemplar sets. | Raw axis position; assign it to a fact in `then`. |
| `c_project(text, axis)` | Projects text onto a calibrated axis and ranks the result against its reference window. | Percentile in `[0, 100]`; compare it directly in `when`. |
| `s_depth(text, region)` | Measures graded depth in a named region fitted around an exemplar cloud (`1.0` at the fitted boundary; smaller is deeper inside). | Raw region evidence; assign it to a fact in `then`. |
| `b_member(text, region)` | Tests the same fitted region at its coverage threshold. | Boolean; test it directly in `when`. |

Pairwise functions accept fact values or literal text. Artifact-backed
functions refer to a stable policy concept by name: an axis represents a
semantic direction such as routine-to-urgent, while a region represents a
cluster such as known business-email-compromise phrasing. The bridge
canonicalizes every text argument before embedding it.

Raw measurements support forward chaining without making an uncalibrated model
score look like a universal policy threshold. A measurement rule records the
evidence as a fact; a later rule combines that fact with ordinary conditions.
Calibrated and boolean artifact functions can participate in a decision
directly:

```grl
rule "MeasureSemanticEvidence" no-loop {
    when
        Payment.text != ""
    then
        Payment.urgency_contrast =
            s_contrast(Payment.text, "urgent and pressured", "routine and flexible");
        Payment.bec_depth = s_depth(Payment.text, "bec_phrasing_v1");
}

rule "HoldHighRiskRequest" salience 100 no-loop {
    when
        c_project(Payment.text, "urgency_pressure_v1") >= 90.0 &&
        b_member(Payment.text, "bec_phrasing_v1") == true &&
        Payment.new_payee == true &&
        Payment.amount >= 10000.0
    then
        Decision.action = "hold";
}
```

Axes, calibration windows, and regions are **named artifacts** fitted offline
(or in the browser) from exemplar sets. Each artifact records its provenance —
model, dimension, task prefix, exemplar-set version — and registration
validates that provenance against the active embedder, so an axis fitted
against one model can never silently score vectors from another.

The reference embedding component:

- loads a configured embedding-capable GGUF model directly;
- defaults to EmbeddingGemma in the release manifest;
- runs CPU-only, single-threaded wllama inference with SIMD128;
- returns L2-normalized vectors;
- reports the model SHA-256 through the typed ABI; and
- stamps vector events so searches never mix model revisions.

Use another compatible GGUF without rebuilding vector-rules:

```sh
vrules-shim --embedding-model /path/to/model.gguf
vrules-shim --embedding-model /path/to/model.gguf \
  --embedding-model-name "Organization Embeddings"
```

The host hashes the selected file, mounts its directory read-only into the
embedding component, and uses the digest, model name, and output dimension as
the cache identity. Persistent deployments can set `model_path`, `model`,
`model_sha256`, and the `/models` preopen in `vrules-components.json`. Models
without pooling metadata can set `pooling` to `mean`, `cls`, or `last`.

## Architecture

vector-rules uses a single native executable (`vrules-shim`) that hosts
independently replaceable WebAssembly components with Wasmtime. Rules are
Git-governed, evaluated through forward chaining and backward proof, and
executed by the same rule kernel in both the browser and production. See the
full [architecture documentation](docs/ARCHITECTURE.md) for the component
layout, GitOps lifecycle, engine compatibility, and embedding cache design.

## Repository structure

### WASI components

| Component | Responsibility |
|---|---|
| [`vrules-runtime`](components/vrules-runtime) | MCP protocol, rule-driven tool exposure and routing, audit, response caching, and memory tools |
| [`vrules-rules`](components/vrules-rules) | GRL loading, canonical forward evaluation, validation, proof, Git revisions, diff, comparison, and fast-forward promotion |
| [`vrules-storage`](components/vrules-storage) | Append-only audit, memory, and response-cache events with model-revision-aware vector search |
| [`vrules-cache`](components/vrules-cache) | Append-only, content-addressed embedding vectors, epoch invalidation, and the local store for the `vrules-rest` cache tier |
| [`vrules-admin`](components/vrules-admin) | Admin RPC, what-if, A/B evaluation, rules governance, memory inspection, and embedding diagnostics |
| [`vrules-gcp`](components/vrules-gcp) | Optional Vertex/Gemini provider with guest-owned ADC and credentials |
| [`vrules-embedding-wllama`](components/vrules-embedding-wllama) | Configurable GGUF embedding inference through pinned wllama/llama.cpp; EmbeddingGemma is the default model |

`release/vrules-components.json` declares component paths, configuration,
preopens, HTTP allowlists, and the admin and cache plugins. Components can be
replaced without changing the native executable or the shared WIT contract.

### Core libraries and native host

- [`vrules-shim`](crates/vrules-shim) is the native Wasmtime host. It provides
  MCP stdio/WebSocket transport, admin HTTP surface, component capabilities,
  and model overrides.
- [`vrules-core`](crates/vrules-core) provides `Ruleset`, `RuleEvaluator`,
  upstream synchronous streaming types, vector/canonical functions, execution
  statistics, and proof.
- [`vrules-wasm`](crates/vrules-wasm) exposes the same engine to browsers.
- [`vrules-canon`](crates/vrules-canon) provides deterministic
  canonicalization.
- [`em-log-n`](crates/em-log-n) provides latency-first searchable storage and
  embedding-cache primitives.
- [`em-log-n-wasm`](crates/em-log-n-wasm) provides the browser storage and
  vector-index shape over IndexedDB.

### Applications

- [`apps/console`](apps/console) is the Svelte admin PWA embedded in daemon
  mode. Operational panels remain top-level; demonstrations use isolated routes
  under `#/examples/*`.
- [`apps/examples`](apps/examples) is the standalone example suite that runs in
  the browser with real EmbeddingGemma vectors.

## Release package

Release archives contain one native shim, the WASI components, an editable
component manifest, a Git-initialized rules repository, and the verified default
model fetcher.

```sh
tar -xf vrules-<version>-<target>.tar.zst
cd vrules-<version>-<target>
VRULES_MODEL_DIR="$PWD/models" ./fetch-model.sh
./vrules-shim
```

The optional GCP component is packaged but absent from the default manifest.
Enable it only after supplying its Google Cloud project and credential
configuration.

## Build from source

Rust 1.94 and the WASI targets are pinned in `rust-toolchain.toml`. The native
workspace and Rust WASI guests build with:

```sh
cargo build --workspace \
  --exclude vrules-runtime \
  --exclude vrules-storage \
  --exclude vrules-rules \
  --exclude vrules-admin \
  --exclude vrules-gcp \
  --exclude vrules-cache

cargo build --target wasm32-wasip2 \
  -p vrules-runtime \
  -p vrules-storage \
  -p vrules-rules \
  -p vrules-admin \
  -p vrules-gcp \
  -p vrules-cache
```

The wllama component additionally requires wasi-sdk 33, `wit-bindgen-cli`
0.58.0, `wasm-tools` 1.251.0, and Wasmtime's Preview 1 reactor adapter:

```sh
./release/build-components.sh
./release/build.sh
```

The release build produces a package for the current host. Explicit target
triples use rootless `cross`. The committed console `dist/` is rebuilt from
`apps/console` with `npm run build`.

## Development checks

```sh
./scripts/ci-check.sh
```

Semantic integration paths use vectors produced by the configured embedding
model with validated model identity and dimensions. Pure rule tests do not
substitute hash-derived, random, zero, or synthetic embeddings.

## Repository layout

```text
apps/          Svelte/Vite PWA console and browser example suite
components/    WASI component sources (Rust and C++)
crates/        Rust workspace members: core libraries and native host
shared-rules/  Git-governed GRL, schemas, and manifest
wit/           backend-neutral component interfaces
docs/          roadmap, positioning, and architecture documents
release/       component manifests, build scripts, and packaging
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Browser examples gallery](docs/EXAMPLES.md)
- [Runtime and `vrules-rest` API](crates/vrules-shim/README.md)
- [Core design](crates/vrules-core/docs/DESIGN.md)
- [Feature coverage](crates/vrules-core/docs/FEATURE-COVERAGE.md)
- [Shared rule packs](shared-rules/README.md)
- [Product roadmap](docs/ROADMAP.md)
- [GPU tensor execution roadmap](docs/GPU-TENSOR-ROADMAP.md)
- [Competitive landscape](docs/COMPETITIVE-LANDSCAPE.md)
- [Contributing](CONTRIBUTING.md)

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option.
