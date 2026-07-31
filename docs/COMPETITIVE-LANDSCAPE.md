# Competitive Landscape & Positioning

This document explains how to position vector-rules relative to adjacent tools and
frameworks. It is intentionally practical: the goal is to describe what vector-rules is,
who it is for, where the implementation is strong, and how it differs from known
competition without drifting into platform-marketing language.

## Positioning in one sentence

**vector-rules is a semantic execution substrate: vector math is a first-class operand inside a deterministic rules engine, and AI guardrules, MCP mediation, web grounding, and organizational memory are rulesets on top of it.**

That means:

- vector-rules is **not** a passive vector database or simple RAG retrieval store; embeddings are operands it evaluates inside rule conditions, not rows it returns to an application.
- vector-rules is **not** a fuzzy, prompt-based "guardrail"; it enforces strict, mathematically provable AI *guardrules* and backward-chains a proof tree for any decision.
- vector-rules is **not** a closed hosted platform or vendor-specific orchestration surface; the kernel is WebAssembly and WASI, so the same semantics run in a browser tab, a Rust backend, and a distributed cloud cache.
- vector-rules is aimed at organizations that want these capabilities without accepting long-term platform lock-in.

### The inversion

The shortest statement of why this is a different class of tool rather than a better
instance of an existing one:

> Most AI systems make the model the judge and leave code as the tool it calls.
> vector-rules makes the model an **instrument** that supplies measurements, and leaves
> the **decision** with deterministic, Git-governed rules that can prove how they decided.

Every differentiator below is a consequence of that inversion.

## Vision

vector-rules's vision is to give organizations a rules-first control plane for AI-linked
production behavior:

1. **Deterministic control over non-deterministic models.** Models can generate,
   classify, or retrieve, but rules remain in charge of production behavior.
2. **Vendor-neutral governance.** Policy lives in git-managed rules and open
   interfaces, not in a proprietary SaaS control plane.
3. **Portable policy across runtimes.** The same semantics can govern the MCP
   component host, a browser what-if flow, and embedded application behavior.
4. **Governed organizational memory.** Embeddings and recall are treated as
   managed infrastructure rather than prompt-only convention.
5. **Explainable operation.** Rule traces, backward proof, audit records, and
   pinned rules revisions make behavior reviewable after the fact.

## Who vector-rules is for

| Organizational need | Why vector-rules fits |
|---|---|
| **Avoiding vendor lock-in** | vector-rules is fully open source, self-hostable, and built around portable rules, git workflows, MCP, WebSocket/HTTP, and WASM rather than a proprietary control plane. |
| **Governing multi-vendor AI estates** | Policy is separated from any one model, client, or application surface, which helps when an org needs to mix tools, gateways, and deployment environments. |
| **Auditable AI-linked production behavior** | Forward traces, backward proof, rule commits, and searchable audit/memory flows support review, investigation, and compliance-heavy environments. |
| **Fine-grained runtime control** | vector-rules can govern small behavior decisions, tool exposure, routing, and memory recall instead of only coarse allow/deny outcomes. |
| **Framework over platform** | Teams that want reusable building blocks they can embed, adapt, and operate themselves fit vector-rules better than teams looking for a turnkey hosted service. |

## Current state

| Area | Current state in repo | Positioning significance |
|---|---|---|
| **`vrules-core`** | Deterministic `RustRuleEngine` evaluation, canonical GRL parsing, forward traces, backward proof, and registered vector/canon functions are implemented, with exact coverage and remaining gaps documented in [`FEATURE-COVERAGE.md`](../crates/vrules-core/docs/FEATURE-COVERAGE.md). | The core rules-first runtime is implemented; roadmap work expands parity and depth without introducing a second rule model. |
| **Wasmtime component host** | `vrules-shim` provides MCP stdio plus optional HTTP/WebSocket administration while runtime behavior remains in typed WASI components. | The reference runtime is pluggable without adding per-component processes or IPC protocols. |
| **wllama + append-only storage components** | Configurable GGUF embedding inference (EmbeddingGemma by default), model-revision-aware vector events, audit, and governed memory are implemented behind the typed component ABI. | Organizational memory and semantic reuse are treated as infrastructure, not prompt convention. |
| **`vrules-wasm` + console** | Browser-side rule validation, forward what-if evaluation, backward proof, and semantic experiments run against the same core semantics. | This supports vendor-neutral policy portability and tighter authoring/runtime alignment. |
| **`shared-rules`** | Rules and schemas live in Git-managed packs that the rules component loads by revision. | Governance is aligned with policy-as-code and review/rollback practices rather than a mutable live-only editor. |
| **Named geometry artifacts** | `vrules-core/src/geometry.rs` fits axes, calibration windows, and regions offline, each carrying `Provenance` (model, dimension, task prefix, exemplar set). Ruleset load validates artifact provenance against the active embedder. | Semantic thresholds are versioned, reviewable policy objects rather than magic constants, and a swapped embedder cannot silently score vectors from a foreign space. |
| **Return-kind discipline** | `vrules-core/src/vec_bridge.rs` declares `FunctionMeta`/`ReturnKind` per function, and the `s_` / `c_` / `b_` / `m_` prefix is linted against that metadata at load. | Misuse of a raw geometry score where a calibrated decision scale belongs fails at load with a legible error instead of skewing a production decision at runtime. |
| **Inline vector algebra** | `vec_bridge` evaluates Lisp-style ASTs (`v:add`, `v:sub`, `v:scale`, `v:mul`) as function arguments, with implicit embedding of literal text and fact references. | Rules express vector arithmetic directly rather than delegating it to application code, which keeps the semantics inside the governed, auditable layer. |
| **Pluggable embedding at two levels** | The `embedding-component` world in `wit/vrules.wit` accepts any conforming WASI component in the manifest's `embedding` slot; the reference wllama component accepts any compatible GGUF via `model_path` / `--embedding-model`. | Vendor neutrality is structural rather than aspirational, and provenance checking is what makes swapping safe rather than merely possible. |
| **Cloud-tier embedding cache** | `VRULES_REST_URL` joins a distributed tier with pull-through reads, asynchronous write-up, and silent fallback to local inference. Credentials stay in a guest `auth_plugin` (`vrules-gcp` in auth mode); the native host holds none. | Semantic evidence is reusable infrastructure across nodes, and the zero-trust split keeps cloud credential logic out of the native host. |
| **Rule-routed tool mediation and grounding** | `shared-rules/proxy/tools.json` declares exposed tools and `routing.grl` selects the backend; `web_ground` routes to `ai.vrules.grounding`, where `vrules-gcp` invokes Vertex AI with Google Search grounding. | Which tools exist, how they dispatch, and which provider serves them are policy decisions in Git, not application configuration. |
| **Governed memory tools** | `vrules-mcp` exposes `memory_write`, `memory_update`, `memory_delete`, `memory_search`, `memory_history`, and `memory_stats` over append-only storage, where updates and deletes are appended events. | Recall is auditable and reconstructable after the fact, which is what separates governed memory from a convenience cache. |

## Known competition and adjacent alternatives

### 1. Policy / authorization DSLs

Representative examples: **OPA/Rego**, **Cedar**, and adjacent governance SDKs.

What they do well:

- deterministic and reviewable policy
- clear ownership and promotion workflows
- strong fit for authorization and guardrail policy

Where vector-rules differs:

- vector-rules adds **stateful deterministic rule evaluation**
- vector-rules allows **semantic predicates** inside rule logic, so a condition can test
  meaning (contrast, calibrated projection, region membership) rather than only structure
- vector-rules answers "why" by **backward chaining** to a proof tree, including analysis
  of the facts still missing to prove a goal
- vector-rules targets **MCP mediation**, organizational memory recall, and browser/runtime
  parity in addition to pure authorization decisions

### 2. Semantic routers and intent classifiers

Representative examples: **semantic-router** and similar embedding-first routers.

What they do well:

- fast meaning-based routing
- lightweight intent classification
- good fit when similarity is the entire decision

Where vector-rules differs:

- semantic similarity is one input inside a larger rules system
- vector-rules combines similarity with context, thresholds, tool exposure, prior facts,
  canon matching, and auditable side effects
- raw similarity is deliberately not comparable in a rule condition: calibrated axes
  convert geometry into a decision scale, and fitted regions carry their own coverage
  threshold, so policy is written against meaningful numbers
- vector-rules is meant for **governed production behavior**, not just top-k routing

### 3. Vector databases and RAG frameworks

Representative examples: **pgvector**, **Qdrant**, **Weaviate**, **LanceDB**, **Chroma**,
and orchestration layers such as **LlamaIndex** and **LangChain**.

What they do well:

- scalable approximate nearest-neighbour search
- mature ingestion, chunking, and retrieval pipelines
- a well-understood pattern for grounding an LLM in private content

Where vector-rules differs:

- retrieval returns candidates for a model to interpret; vector-rules **evaluates** vector
  math inside rule conditions and derives facts from it
- contrast, calibrated projection, and region membership are decision primitives, not
  post-processing over a top-k result set
- geometry is a named, versioned artifact with provenance rather than a similarity
  threshold embedded in application code
- the outcome is an auditable action with a proof tree, not a context window

### 4. LLM guardrail libraries

Representative examples: **NVIDIA NeMo Guardrails**, **Guardrails AI**, **Llama Guard**,
and similar model-side safety and validation layers.

What they do well:

- fast to adopt around an existing model call
- broad coverage of common safety and format-validation cases
- an ecosystem of ready-made checks

Where vector-rules differs:

- these evaluate a model with a model, so drift and hallucination remain in the control
  path; vector-rules keeps the control path deterministic
- policy is stateful and forward-chaining, so semantic evidence composes with scalar
  facts, prior facts, and organizational context rather than gating a single call
- an outcome is explained by backward chaining over what actually fired, not by asking a
  model to narrate its own reasoning
- policy is pinned to a Git revision, so a decision remains reproducible after the fact

### 5. Agent memory products

Representative examples: **Mem0**, **Zep**, **Letta**, and comparable hosted or
library-based agent memory layers.

What they do well:

- convenient extraction, storage, and recall of conversational context
- ready-made relevance heuristics
- quick integration with popular agent frameworks

Where vector-rules differs:

- recall is subject to the same rule layer that governs every other decision, so what is
  written and what is surfaced are policy questions
- storage is append-only, and updates and deletes are appended events, which keeps
  post-mortem reconstruction possible
- memory shares the embedding cache, provenance checking, and audit surface with the rest
  of the substrate rather than sitting beside them
- memory is one ruleset on the substrate, not the product itself

### 6. MCP gateways, agent firewalls, and AI gateways

Representative examples: **IBM ContextForge**, **Lasso MCP Gateway**,
**kgateway agentgateway**, **LangDB**, and similar gateway layers.

What they do well:

- protocol mediation
- auth, rate limits, and gateway operations
- quick centralization of agent or MCP traffic

Where vector-rules differs:

- vector-rules leads with **rule-driven mediation** rather than gateway plumbing alone
- tool exposure and routing are authored as policy, not only configured as
  gateway features
- semantic recall and searchable audit are part of the same open-source stack
- vector-rules is positioned as a **vendor-neutral framework**, not a managed gateway
  platform

### 7. Business rules and decision engines

Representative examples: **GoRules / ZEN**, **Drools**, and the broader RETE /
decision-table family.

What they do well:

- deterministic rule execution
- mature rule authoring models
- explainability and operational predictability

Where vector-rules differs:

- vector-rules is built around **LLM-, agent-, and MCP-adjacent** workloads
- embeddings and canonicalization are first-class citizens in the rule layer, including
  inline vector algebra evaluated inside rule arguments
- classic engines have no notion of a vector, so semantic judgement has to happen outside
  the rule layer and arrive as a pre-computed flag
- browser/WASM parity matters in the design, not as an afterthought

### 8. GitOps / feature-policy tooling

Representative examples: **Flipt** and other config/policy promotion tools.

What they do well:

- review, rollout, rollback, and diff workflows
- clear operational ownership
- auditable change management

Where vector-rules differs:

- vector-rules extends those disciplines into **semantic runtime behavior**
- rules can be replayed against real traffic and audit history
- the policy is not only about flags or config; it can govern tools, memory,
  routing, and application behavior

### 9. Managed or vendor-specific AI governance platforms

This category includes closed or hosted products that package guardrails,
orchestration, gateways, or AI governance into a vendor-owned platform.

What they typically do well:

- fast time-to-first-demo
- a single control plane
- polished integrated user experience

Where vector-rules differs:

- vector-rules is **not trying to be a proprietary managed platform**
- the target audience is organizations that want **vendor-neutral capabilities**
  they can run and evolve themselves
- vector-rules favors **portable infrastructure and policy** over a closed product moat

## What is genuinely distinctive about vector-rules

### The class is defined by a conjunction, and it is otherwise empty

The strongest claim available is also the most checkable one, and it is worth stating as a
conjunction rather than as a superlative. Adopters can verify it against the categories
above instead of taking it on trust:

> No other system evaluates embeddings as first-class operands inside a deterministic,
> stateful rules engine, derives governed action from them, and returns a backward-chained
> proof tied to a Git revision — on a kernel that runs unchanged in a browser and on a
> server.

Each neighbouring category satisfies part of it and fails the rest:

| Category | Semantic | Deterministic decision | Proof | Runs anywhere WASI runs |
|---|---|---|---|---|
| Policy / authorization DSLs | no | yes | partial | partial |
| Semantic routers | yes | no | no | no |
| Vector databases and RAG | yes | no | no | no |
| LLM guardrail libraries | yes | no | no | no |
| Agent memory products | yes | no | no | no |
| MCP / AI gateways | no | partial | no | no |
| Business rules engines | no | yes | partial | no |

Say "the only system that does X and Y and Z together", and cite this table. Avoid
"best-in-class", which invites the reader to ask which class and who else is in it — the
interesting answer is that the conjunction defines a class with one member, and that is a
statement a reader can falsify.

### Reach is a property of the kernel, not of a deployment

Because the execution kernel is WebAssembly and WASI, the same rule semantics run inside a
browser tab, a Rust backend, an MCP component host, and a distributed cloud cache. This is
what makes "semantic reasoning in any modern application" a structural claim rather than an
integration roadmap: there is one evaluator, not a server implementation with a client-side
approximation of it.

### Fully open source and vendor-neutral by intent

This is not just a licensing detail. The positioning matters:

- organizations own the rules
- organizations can self-host the runtime
- organizations can review, fork, and adapt the stack
- policy is not trapped inside a vendor-specific UI, DSL, or hosted memory layer

### Deterministic runtime governance instead of prompt convention

vector-rules is aimed at the gap between "prompt the model better" and "buy a whole AI
platform." It treats governance as runtime infrastructure: rules, tool exposure,
audited memory, explainability, and replay.

### One integrated open-source stack

The differentiation is not any single component in isolation. It is the
combination of:

- deterministic rule evaluation with vector math as a first-class operand
- named geometry artifacts with enforced provenance
- MCP-aware mediation and rule-routed web grounding
- governed organizational memory over append-only storage
- a shared embedding cache with an optional zero-trust cloud tier
- GitOps policy flow
- browser/runtime semantic parity

## Messaging guidance

### Lead with these claims

- **A semantic execution substrate**, with guardrules as its flagship application
- the inversion: the model is an instrument, deterministic rules make the decision
- **Fully open source, vendor-neutral framework**
- semantic predicates inside deterministic rules, with a proof tree for any decision
- pluggable embedding at both the component and model level, with provenance enforced
- rule-driven MCP mediation, web grounding, and least-privilege tool exposure
- organizational memory and semantic recall as governed infrastructure
- policy-as-code, replay, auditability, and browser/runtime parity

### Avoid these claims

- Do not present vector-rules as a managed SaaS or turnkey proprietary platform.
- Do not imply every roadmap item is already complete; use
  [`FEATURE-COVERAGE.md`](../crates/vrules-core/docs/FEATURE-COVERAGE.md) and the
  other docs to separate shipped behavior from ongoing work.
- Do not sell "Rust" as the main moat by itself.
- Do not claim models are removed from the system; the point is that they are
  governed by deterministic policy.
- Do not reach for unfalsifiable superlatives such as "best-in-class", "universal", or
  "seamless". State the conjunction that defines the category and let the reader check
  it; a claim that can be tested is stronger than one that cannot.
- Do not claim any-media or multimodal embedding. The `embedding` interface in
  [`wit/vrules.wit`](../wit/vrules.wit) is text-in, vector-out; what is pluggable is the
  embedder, not the input modality.

## Best-fit story to tell

When describing vector-rules to a prospective adopter, the cleanest story is:

1. We are fully open source.
2. We are vendor-neutral and aimed at orgs avoiding lock-in.
3. We invert the usual arrangement: the model measures, deterministic rules decide.
4. We treat agent/LLM guardrails as deterministic policy and runtime
   infrastructure, not only as prompt engineering.
5. We apply that policy at the MCP component host, in memory/recall flows, in
   the browser, and inside applications.
6. We give teams a framework they can own rather than a platform they rent.
