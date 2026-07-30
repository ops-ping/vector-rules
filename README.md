# vector-rules

**Reason like an AI without sacrificing determinism.**

`vector-rules` bridges meaning embedded in any media seamlessly across GitOps-managed policy requirements with mathematical precision rather than prompts.

**Turn meaning** found in documents, messages, events, and records into deterministic, governed decisions. This framework combines embeddings, which measure semantic relationships in content, with a rules engine that applies explicit organizational policy to those measurements alongside ordinary application facts.

LLMs are excellent at interpreting language, generating candidates, and summarizing complex information, **but they are not a reviewable or auditable source of production policy.** When an LLM makes an operational decision, asking it "why?" yields a hallucinated rationalization rather than a verifiable explanation. By inserting a deterministic, GitOps-managed rule layer between fuzzy inputs and operational actions, `vector-rules` ensures your AI-driven behavior can be tested, traced, audited, replayed, and rolled back.

The project is fully open source, vendor-neutral, local-first, and embeddable. The same rule semantics can govern an MCP component host, run inside a Rust application, evaluate stateful streams, and power browser-side what-if analysis through WebAssembly. Organizations keep their policy in Git and can move it across models, clients, and deployment environments without depending on a proprietary control plane.

---

## The Breakthrough: Bridging Rete & Vector Math

`vector-rules` pioneers a native bridge between semantic embeddings and deterministic Rete-style forward chaining. Instead of relying on brittle prompt engineering—hoping an LLM correctly judges whether a text is "more urgent than routine"—`vector-rules` elevates vector mathematics to a first-class citizen inside rule evaluation.

Rules reason across embeddings with mathematical precision:

* **Semantic Math over Prompts:** Mathematical functions evaluate vector contrast (`s_contrast`), axis projection (`c_project`), and region membership (`b_member`) directly inside rule conditions alongside hard scalar facts (such as payment amounts or payee status).
* **Deterministic Chaining:** Fuzzy vector measurements are captured as facts, and subsequent rules apply explicit policy thresholds to derive final operational decisions.
* **Complete Traceability:** Forward execution traces capture every input fact, fired rule, derived fact, and final action.

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

### Execution Trace

The following execution trace demonstrates how inline vector algebra `(king − man + woman) ≈ queen` and semantic reasoning are evaluated in real-time by the browser rule engine:

![Semantic vector algebra and similarity over EmbeddingGemma vectors running locally in the browser](docs/examples-semantic.png)

---

## Auditability & Mathematical Proof

When an LLM makes an operational choice or denies a request, asking it why produces a post-hoc text summary that may not reflect the actual underlying mechanics. 

Because `vector-rules` evaluates decisions through deterministic forward chaining and vector math, the engine can natively **backward-chain** to generate a mathematical proof tree of exactly how any state or status was reached:

* **Mathematical Proof vs. Hallucination:** A proof tree documents every rule that fired, the exact vector contrast scores evaluated, the intermediate facts derived, and the scalar thresholds met.
* **Missing Fact Analysis:** Backward chaining can analyze missing conditions required to prove a goal, enabling precise diagnostic tools and interactive what-if workflows.
* **Auditable State:** Every decision can be tied directly to the exact Git commit revision of the ruleset and the verified identity of the embedding model that produced the evidence.

---

## How It Works

`vector-rules` combines two complementary paradigms: probabilistic semantic measurement and deterministic declarative policy.

### 1. Policy as Code (Rete Rules Engine)

A rules engine applies **declarative policy** to facts. Conventional application code describes a fixed sequence of calls and branches: first do this, then test that, then choose a path. A rule instead states only the policy relationship: **when these facts are true, derive this fact or take this action**. The engine determines which rules apply, adds their conclusions to working facts, and continues evaluating until no further rules fire.

This forward chaining automatically connects simple, independently understandable policies into complex multi-stage decisions:

| Policy | Fact or decision produced |
|---|---|
| Language in a document expresses unusual urgency or pressure | `urgent_request` |
| A payment has a new payee and exceeds an amount threshold | `elevated_payment_risk` |
| A request is urgent and has elevated payment risk | `manual_review_required` |
| Manual review is required | Hold the payment and record the reason |

### 2. Embeddings as Evidence

An embedding model converts content into a vector that represents semantic meaning. Embeddings excel where exact keywords and rigid schemas are too brittle—for example, recognizing that `ship_to`, `delivery address`, and `destination` describe the same concept, or measuring whether a request sounds pressured.

Embeddings are measurements, not decisions. `vector-rules` treats vector measurements as governed evidence: rules combine semantic measurements with scalar context, state, and organizational policy before taking action. Model identity tracking ensures evidence is never silently mixed across incompatible vector spaces.

### 3. GitOps Governance

Policy lives in Git repositories as versioned rulesets (`shared-rules/`). Organizations use standard Git workflows—pull requests, peer reviews, automated CI testing, promotion environments, and instant rollbacks—to manage business policy.

---

## Embeddings inside deterministic rules

GRL calls vector operations as ordinary registered functions, so embedding measurements participate in the same condition evaluation as scalar facts. The browser console runs the same evaluator through `vrules-wasm` and obtains real vectors from the configured embedding component through the daemon RPC surface.

Function names carry their return kind, and the engine lints loaded rules against declared function metadata, so misuse fails at load with a legible error instead of skewing a decision at runtime:

| Prefix | Kind | May appear in `when` as |
|---|---|---|
| `s_` | raw scalar (geometry measurement) | nothing — assign to a fact in `then`, or use a `c_` form |
| `c_` | calibrated / decision-scale scalar | any comparison (`c_project(...) >= 90.0`) |
| `b_` | boolean | `== true` / `== false` / `test(...)` |
| `m_` | metadata (label, identifier) | equality and string operators |

The built-in vector surface is:

| Function | What it does | Result and rule use |
|---|---|---|
| `s_cosine(left, right)` | Embeds two texts and measures cosine similarity. | Raw similarity; assign it to a fact in `then`. |
| `s_dot(left, right)` | Embeds two texts and takes their dot product without normalizing in the function. | Raw scalar that preserves any vector-magnitude signal; assign it to a fact in `then`. |
| `s_contrast(candidate, positive, negative)` | Computes `cos(candidate, positive) - cos(candidate, negative)` so shared topic meaning cancels and the semantic contrast remains. | Raw directional evidence; assign it to a fact in `then`. |
| `s_project(text, axis)` | Projects text onto a named axis fitted from positive and negative exemplar sets. | Raw axis position; assign it to a fact in `then`. |
| `c_project(text, axis)` | Projects text onto a calibrated axis and ranks the result against its reference window. | Percentile in `[0, 100]`; compare it directly in `when`. |
| `s_depth(text, region)` | Measures graded depth in a named region fitted around an exemplar cloud (`1.0` at the fitted boundary; smaller is deeper inside). | Raw region evidence; assign it to a fact in `then`. |
| `b_member(text, region)` | Tests the same fitted region at its coverage threshold. | Boolean; test it directly in `when`. |

Pairwise functions accept fact values, literal text, or nested Lisp-style vector algebra arrays. Artifact-backed functions refer to a stable policy concept by name: an axis represents a semantic direction such as routine-to-urgent, while a region represents a cluster such as known business-email-compromise phrasing. The bridge canonicalizes every text argument before embedding it.

### Inline Vector Algebra (Lisp-Style ASTs)

In addition to string arguments, vector functions like `s_cosine` and `s_dot` accept inline Lisp-style vector algebra expressions. This allows rules to perform complex vector arithmetic (such as vector addition, subtraction, or scaling) directly inside rule conditions:

* **Operators:** `["v:add", A, B]`, `["v:sub", A, B]`, `["v:scale", A, scalar]`, `["v:mul", A, B]`.
* **Implicit Embedding:** Raw text strings inside vector operations (e.g., `"king"`, `"man"`, `"woman"`) or string fact references are implicitly converted to embedding vectors.
* **Classic Analogy Example:** Evaluate `(king − man + woman) ≈ queen` natively in GRL:

```grl
rule "EvaluateVectorAnalogy" no-loop {
    when
        Concept.target != ""
    then
        Concept.similarity = s_cosine(["v:add", ["v:sub", "king", "man"], "woman"], Concept.target);
}

rule "AnalogyMatch" no-loop {
    when
        Concept.similarity > 0.80
    then
        Decision.is_analogy_target = true;
}
```

Axes, calibration windows, and regions are **named artifacts** fitted offline (or in the browser) from exemplar sets. Each artifact records its provenance—model, dimension, task prefix, exemplar-set version—and registration validates that provenance against the active embedder, so an axis fitted against one model can never silently score vectors from another.

### Reference Embedding Component & Model Overrides

The reference embedding component loads a configured embedding-capable GGUF model directly (EmbeddingGemma by default) running single-threaded CPU or WebGPU wllama inference with SIMD128.

Use another compatible GGUF without rebuilding vector-rules:

```sh
vrules-shim --embedding-model /path/to/model.gguf
vrules-shim --embedding-model /path/to/model.gguf \
  --embedding-model-name "Organization Embeddings"
```

The host hashes the selected file, mounts its directory read-only into the embedding component, and uses the digest, model name, and output dimension as the cache identity.

### Cloud Tier Embedding Cache

The embedding cache is a two-tier system: a fast local store backed by the `cache` plugin, and an optional remote tier over HTTPS. Set `VRULES_REST_URL` to point at a cloud bucket (e.g. `https://storage.googleapis.com/bucket/v1`) and the daemon joins the distributed cache:

- **Pull-through:** local cache misses fall back to the remote endpoint before running local wllama inference.
- **Write-up:** locally computed vectors are asynchronously PUT to the remote tier so other nodes can reuse them.
- **Silent fallback:** if the remote tier errors or times out, resolution continues with local inference without interrupting the request.
- **Zero-trust host:** the native daemon holds no cloud credentials. The `auth_plugin` field in the component manifest names a WASI plugin (e.g. `vrules-gcp` in auth mode) that generates the `Authorization` header for each request, keeping all credential logic inside the guest.

---

## Agent Tools and Web Grounding

`vector-rules` exposes its capabilities to AI assistants over the Model Context Protocol (MCP). Every tool an assistant can invoke is declared in `shared-rules/proxy/tools.json` and routed by deterministic GRL rules, so the engine governs *which* tools are available, *how* they are dispatched, and *what* backend handles them—all without changing application code.

A concrete example is **web grounding**. When an assistant needs live web results, it calls the `web_ground` tool. The rule engine evaluates `shared-rules/proxy/routing.grl`, routes the request to the `ai.vrules.grounding` backend, and the `vrules-gcp` component invokes Vertex AI (Gemini) with Google Search grounding enabled. The response is cached, audited, and returned to the assistant through the MCP transport.

Because the rule engine sits between the assistant and the provider, you can wrap expensive LLM calls in deterministic policy—rate limits, context controls, required safety filters, or cost gates—without modifying application code.

---

## Reference Examples

The standalone [`apps/examples`](apps/examples) application is a suite of executable capability demonstrations running the Rust rule engine in the browser with real EmbeddingGemma vectors:

| Example | What it demonstrates |
|---|---|
| **Address verification** | Structured and unstructured input, native functions, canonicalization, embeddings, local reference indexes, and editable organizational policy in one auditable flow |
| **Semantic rules** | Similarity and contrast measurements, return-kind enforcement, and forward chaining from embedding evidence into deterministic facts |
| **Fraud triage** | Calibrated axes and fitted regions combined with hard facts such as payee status and amount |
| **Streaming** | Sequential one-event/one-result rule processing with throughput and decision output |
| **Proof** | Backward chaining with provability, missing facts, and a visible proof tree |

Check out the [browser examples gallery](docs/EXAMPLES.md) to see these interactive WebAssembly demonstrations in action.

---

## Architecture

`vector-rules` uses a single native executable (`vrules-shim`) that hosts independently replaceable WebAssembly components with Wasmtime. Rules are Git-governed, evaluated through forward chaining and backward proof, and executed by the same rule kernel in both the browser and production. See the full [architecture documentation](docs/ARCHITECTURE.md) for details.

---

## Repository Structure

### WASI Components

| Component | Responsibility |
|---|---|
| [`vrules-mcp`](components/vrules-mcp) | MCP protocol implementation, tool exposure, rule-driven tool routing, audit events, and memory tools |
| [`vrules-harness`](components/vrules-harness) | WASI harness for `vrules-core`: Git-governed rule loading, GRL parsing, forward traces, and backward proofs |
| [`vrules-storage`](components/vrules-storage) | Append-only audit, memory, and response-cache events with model-revision-aware vector search |
| [`vrules-cache`](components/vrules-cache) | Append-only, content-addressed embedding vectors, epoch invalidation, and the local store for the `vrules-rest` cache tier |
| [`vrules-admin`](components/vrules-admin) | Admin RPC, what-if, A/B evaluation, rules governance, memory inspection, and embedding diagnostics |
| [`vrules-gcp`](components/vrules-gcp) | Dual-mode component: Vertex/Gemini provider for web grounding, and an auth plugin that generates cloud-tier credentials with ADC fully encapsulated in the guest |
| [`vrules-embedding-wllama`](components/vrules-embedding-wllama) | Configurable GGUF embedding inference through pinned wllama/llama.cpp; EmbeddingGemma is the default model |

### Core Libraries and Native Host

- [`vrules-shim`](crates/vrules-shim) is the native Wasmtime host. It provides MCP stdio/WebSocket transport, admin HTTP surface, component capabilities, and model overrides.
- [`vrules-core`](crates/vrules-core) provides `Ruleset`, `RuleEvaluator`, upstream synchronous streaming types, vector/canonical functions, execution statistics, and proof.
- [`vrules-wasm`](crates/vrules-wasm) exposes the same engine to browsers.
- [`vrules-canon`](crates/vrules-canon) provides deterministic canonicalization.
- [`em-log-n`](crates/em-log-n) provides latency-first searchable storage and embedding-cache primitives.
- [`em-log-n-wasm`](crates/em-log-n-wasm) provides the browser storage and vector-index shape over IndexedDB.

### Applications

- [`apps/console`](apps/console) is the Svelte admin PWA embedded in daemon mode.
- [`apps/examples`](apps/examples) is the standalone example suite that runs in the browser with real EmbeddingGemma vectors.

### Directory Layout

```text
apps/          Svelte/Vite PWA console and browser example suite
components/    WASI component sources (Rust and C++)
crates/        Rust workspace members: core libraries and native host
shared-rules/  Git-governed GRL, schemas, and manifest
wit/           backend-neutral component interfaces
docs/          roadmap, positioning, and architecture documents
release/       component manifests, build scripts, and packaging
```

---

## Release Package & Quick Start

Release archives contain one native shim, the WASI components, an editable component manifest, a Git-initialized rules repository, and the verified default model fetcher.

```sh
tar -xf vrules-<version>-<target>.tar.zst
cd vrules-<version>-<target>
VRULES_MODEL_DIR="$PWD/models" ./fetch-model.sh
./vrules-shim
```

---

## Build from Source

Rust 1.94 and WASI targets are pinned in `rust-toolchain.toml`. Build native workspace members and Rust WASI guests with:

```sh
cargo build --workspace \
  --exclude vrules-mcp \
  --exclude vrules-storage \
  --exclude vrules-harness \
  --exclude vrules-admin \
  --exclude vrules-gcp \
  --exclude vrules-cache

cargo build --target wasm32-wasip2 \
  -p vrules-mcp \
  -p vrules-storage \
  -p vrules-harness \
  -p vrules-admin \
  -p vrules-gcp \
  -p vrules-cache
```

The wllama component additionally requires wasi-sdk 33, `wit-bindgen-cli` 0.58.0, `wasm-tools` 1.251.0, and Wasmtime's Preview 1 reactor adapter:

```sh
./release/build-components.sh
./release/build.sh
```

---

## Development Checks

```sh
./scripts/ci-check.sh
```

---

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

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
