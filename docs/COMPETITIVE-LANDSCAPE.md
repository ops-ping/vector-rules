# Competitive Landscape & Positioning

This document explains how to position vector-rules relative to adjacent tools and
frameworks. It is intentionally practical: the goal is to describe what vector-rules is,
who it is for, where the implementation is strong, and how it differs from known
competition without drifting into platform-marketing language.

## Positioning in one sentence

**vector-rules is a fully open-source, vendor-neutral framework for deterministic
LLM/agent governance, MCP mediation, organizational memory, and policy-as-code
workflows.**

That means:

- vector-rules is **not** a closed hosted platform
- vector-rules is **not** a vendor-specific orchestration surface
- vector-rules is aimed at organizations that want these capabilities without accepting
  long-term platform lock-in

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

## Known competition and adjacent alternatives

### 1. Policy / authorization DSLs

Representative examples: **OPA/Rego**, **Cedar**, and adjacent governance SDKs.

What they do well:

- deterministic and reviewable policy
- clear ownership and promotion workflows
- strong fit for authorization and guardrail policy

Where vector-rules differs:

- vector-rules adds **stateful deterministic rule evaluation**
- vector-rules allows **semantic predicates** inside rule logic
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
- vector-rules is meant for **governed production behavior**, not just top-k routing

### 3. MCP gateways, agent firewalls, and AI gateways

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

### 4. Business rules and decision engines

Representative examples: **GoRules / ZEN**, **Drools**, and the broader RETE /
decision-table family.

What they do well:

- deterministic rule execution
- mature rule authoring models
- explainability and operational predictability

Where vector-rules differs:

- vector-rules is built around **LLM-, agent-, and MCP-adjacent** workloads
- embeddings and canonicalization are first-class citizens in the rule layer
- browser/WASM parity matters in the design, not as an afterthought

### 5. GitOps / feature-policy tooling

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

### 6. Managed or vendor-specific AI governance platforms

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

- deterministic rule evaluation
- MCP-aware mediation
- governed organizational memory
- GitOps policy flow
- browser/runtime semantic parity

## Messaging guidance

### Lead with these claims

- **Fully open source, vendor-neutral framework**
- deterministic governance for LLM- and agent-linked production flows
- rule-driven MCP mediation and least-privilege tool exposure
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

## Best-fit story to tell

When describing vector-rules to a prospective adopter, the cleanest story is:

1. We are fully open source.
2. We are vendor-neutral and aimed at orgs avoiding lock-in.
3. We treat agent/LLM guardrails as deterministic policy and runtime
   infrastructure, not only as prompt engineering.
4. We apply that policy at the MCP component host, in memory/recall flows, in
   the browser, and inside applications.
5. We give teams a framework they can own rather than a platform they rent.
