# Browser examples

**[Try them live →](https://ops-ping.github.io/vector-rules/)** — no install, no
sign-up, no server.

The `apps/examples` app is a standalone, server-free build of the capability
demonstrations. Everything runs in the browser: the rule engine (`vrules-wasm`)
and EmbeddingGemma both execute as WebAssembly, with no daemon.

Embeddings are computed by wllama, which uses WebGPU when the browser exposes a
GPU adapter and falls back to CPU otherwise — the header badge reports the active
backend. A cache-through tier serves vectors from a static, content-addressed
cache seeded for the example corpora, so every prepared demonstration runs
without downloading the model; free-form input falls back to in-browser
inference, streaming the pinned quantization from the CDN on first use.

The header states which of those is happening. Before any download it says the
prepared inputs come from the seeded cache and that editing text fetches the
model once (236 MB). During the fetch it shows a progress bar with transferred
bytes; on a return visit it reports that the model was already in the browser
rather than re-fetching it. Once loaded it names the running model, its digest,
and a running count of embeddings computed in that tab — so a claim that the real
model produced a given vector is checkable from the page itself.

The model is stored in OPFS, so the download happens once per browser.

## Verifying the examples

`npm run verify` builds the app, serves the built `dist/`, drives all five
examples in headless Chrome, and captures the screenshots below. It fails the run
on any uncaught exception, console error, failed request, missing expected
selector, or a seeded-cache miss.

It also fails if a prepared input triggers a model download. That guard is what
keeps `scripts/seed-cache.mjs` honest: the seed corpus mirrors string constants
in the example components, and drift there would silently cost every visitor a
236 MB download instead of a cache hit.

```
cd apps/examples
npm run verify        # build + drive + assert + capture
npm run screenshots   # capture against an already-running server
npm run seed          # re-seed the vector cache (needs llama-server + the GGUF)
npm run icons         # re-rasterize favicon.svg into the PNG/ICO fallbacks
```

The captured screenshots double as the fallback imagery in this document for
readers who cannot run the live site.

## Address verification

One Rust/WASM path standardizes both chat-like text and arbitrary structured
JSON, matches the result against JSON reference data for customers and products,
and applies editable organizational policy as GRL rules. Native address
functions, canonicalization, reference lexical matching, and policy compose in a
single evaluation — the address domain is never baked into the framework. The
run reports a validity score, the canonical form, exact and lexical reference
evidence, fired rules, and the parsed components.

![Address verification standardizing an order, matching reference data, and applying brand-owner bill-to policy](examples-address.png)

## Semantic rules

An editable bench for vector reasoning. The GRL rules, the asserted facts and the match string are all input: the engine parses what is typed, resolves a vector for every text the rules reference, and evaluates. Nothing runs until Run is pressed, and the execution trace, the value each rule derived, and the output facts are read back from the engine's own result rather than described alongside it — editing a rule changes what the trace reports, because the trace is built from the source the engine parsed.

The rules it starts with are a three-step forward chain over Lisp-style vector algebra (`v:add`, `v:sub`): a measurement rule writes `s_cosine(["v:add", ["v:sub", "king", "man"], "woman"], Concept.target)` into a fact, an intermediate rule thresholds that similarity to derive a category, and a decision rule fires on the derived category to grant access. Function names carry their return kind — `s_` raw scalar (a measurement, never thresholded), `c_` calibrated (thresholdable), `b_` boolean, `m_` metadata — and the engine's load-time lint rejects thresholding a raw score directly.

![Editable GRL beside an execution trace read back from the engine, forward chaining from inline vector algebra to a deterministic decision](examples-semantic.png)

A match string outside the seeded corpus takes the same path with nothing precomputed: EmbeddingGemma is downloaded once, the vector is computed in the tab, and every vector carries the tier that produced it.

![A free-form match string embedded in the browser, its computed vector driving the same rules to a granted decision](examples-semantic-dynamic.png)

## Fraud triage

A payment-request screen in the layered-fusion shape production fraud stacks use.
The embedding layer supplies named, versioned geometry — an urgency-pressure axis
with a percentile calibration window and a BEC-phrasing region fitted from
exemplars — all fitted in the browser. The symbolic rules make the decision:
vector scores are evidence beside hard checks (new payee, amount), never a
standalone gate. Every artifact carries model provenance and is validated against
the active embedder at load.

![Fraud triage holding a BEC-style request using a fitted urgency axis and phrasing region beside hard checks](examples-fraud-triage.png)

## Streaming

Real-time event stream processing through `WasmStreamProcessor` using native sliding, tumbling, and session windowing in the browser. Each stream event is evaluated against windowed GRL rules with live throughput and accuracy metrics.

![Real-time event stream processing in the browser using WasmStreamProcessor and windowing](examples-streaming.png)

## Proof

Backward-chaining `vrules::prove` runs in the browser — the same engine path the daemon runs. A goal is posed against a GRL knowledge base and the engine works backward, reporting provability, missing facts, and the mathematical proof tree. The example proves a multi-tiered e-commerce order approval chain (`Order.Approved ← Status.FundsAvailable AND Status.RiskIsLow`).

![Backward-chaining proof tree reporting provability and mathematical derivation for order approval](examples-proof.png)
